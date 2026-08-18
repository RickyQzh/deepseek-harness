//! Fiber identifiers, states, handles, and the shared runtime table.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::sync::watch;

use crate::KernelError;
use crate::context::Context;

/// Opaque fiber id. The root fiber is `FiberId(0)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct FiberId(pub u64);

/// Explicit isolate realm. `RealmKey::root()` is the host realm (`0`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RealmKey(pub u64);

impl RealmKey {
    /// The host / root realm.
    #[must_use]
    pub const fn root() -> Self {
        Self(0)
    }
}

/// Lifecycle state for one plugin fiber.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FiberState {
    /// Waiting for required services.
    Pending,
    /// `setup` is running.
    Loading,
    /// Loaded and providing.
    Active,
    /// `setup` returned `Err`.
    Failed,
    /// Disposers are running.
    Unloading,
    /// Removed; cannot accept new effects.
    Disposed,
}

/// Distinguishes host composition rows from preset-owned rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginKind {
    /// A row in the host composition (root realm is allowed).
    Host,
    /// A preset-owned row; `provide` into the root realm is a load failure.
    Preset {
        /// Preset row id used in the load-failure message.
        row_id: String,
    },
}

/// Cloneable view of one child fiber.
#[derive(Clone)]
pub struct FiberHandle {
    pub(crate) rt: Arc<Runtime>,
    pub(crate) fiber_id: FiberId,
}

impl FiberHandle {
    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> FiberState {
        self.rt
            .fibers
            .lock()
            .expect("fiber table lock")
            .get(&self.fiber_id)
            .map(|rec| rec.state)
            .unwrap_or(FiberState::Disposed)
    }

    /// Context bound to this fiber.
    #[must_use]
    pub fn context(&self) -> Context {
        Context {
            rt: Arc::clone(&self.rt),
            fiber_id: self.fiber_id,
            isolate: Arc::clone(
                &self
                    .rt
                    .fibers
                    .lock()
                    .expect("fiber table lock")
                    .get(&self.fiber_id)
                    .map(|rec| Arc::clone(&rec.isolate))
                    .unwrap_or_else(|| Arc::new(HashMap::new())),
            ),
        }
    }

    /// Wait until the fiber is `Active`, `Failed`, or `Disposed`.
    ///
    /// # Errors
    ///
    /// `SetupFailed` when the fiber settled `Failed`; `InactiveEffect` when it settled `Disposed` before becoming `Active`.
    pub async fn await_ready(&self) -> Result<(), KernelError> {
        let mut rx = self
            .rt
            .fibers
            .lock()
            .expect("fiber table lock")
            .get(&self.fiber_id)
            .expect("fiber exists")
            .ready
            .subscribe();
        loop {
            let state = *rx.borrow();
            match state {
                FiberState::Active => return Ok(()),
                FiberState::Failed => {
                    return Err(self
                        .rt
                        .fibers
                        .lock()
                        .expect("fiber table lock")
                        .get(&self.fiber_id)
                        .and_then(|rec| rec.error.clone())
                        .unwrap_or_else(|| KernelError::SetupFailed("unknown".into())));
                }
                FiberState::Disposed => return Err(KernelError::InactiveEffect),
                FiberState::Pending | FiberState::Loading | FiberState::Unloading => {
                    if rx.changed().await.is_err() {
                        return Err(KernelError::InactiveEffect);
                    }
                }
            }
        }
    }

    /// Unload this fiber and wait until `Disposed`.
    pub async fn dispose(&self) {
        dispose_fiber(&self.rt, self.fiber_id).await;
    }
}

/// Single-shot async disposer returned by `effect` / `provide` / `on`.
pub struct Disposer {
    pub(crate) rt: Arc<Runtime>,
    pub(crate) fiber_id: FiberId,
    pub(crate) effect_id: u64,
}

impl std::fmt::Debug for Disposer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disposer")
            .field("fiber_id", &self.fiber_id)
            .field("effect_id", &self.effect_id)
            .finish()
    }
}

impl Disposer {
    /// Run this disposer. A second call is a no-op once the slot is gone.
    pub async fn dispose(self) {
        let cleanup = {
            let mut fibers = self.rt.fibers.lock().expect("fiber table lock");
            let Some(rec) = fibers.get_mut(&self.fiber_id) else {
                return;
            };
            let Some(pos) = rec
                .effects
                .iter()
                .position(|slot| slot.id == self.effect_id)
            else {
                return;
            };
            rec.effects.remove(pos).cleanup
        };
        if let Some(cleanup) = cleanup {
            cleanup().await;
        }
    }
}

/// Type-erased async cleanup stored on a fiber.
pub(crate) type Cleanup =
    Box<dyn FnOnce() -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

pub(crate) struct EffectSlot {
    pub(crate) id: u64,
    pub(crate) cleanup: Option<Cleanup>,
}

/// Push a cleanup onto `rec` and return the new slot id.
pub(crate) fn push_effect(rec: &mut FiberRec, cleanup: Cleanup) -> u64 {
    let effect_id = rec.next_effect;
    rec.next_effect += 1;
    rec.effects.push(EffectSlot {
        id: effect_id,
        cleanup: Some(cleanup),
    });
    effect_id
}

/// Per-fiber record. `parent` is walked by scoped event delivery.
pub(crate) struct FiberRec {
    pub(crate) state: FiberState,
    pub(crate) kind: PluginKind,
    pub(crate) parent: Option<FiberId>,
    pub(crate) inject: Vec<String>,
    pub(crate) isolate: Arc<HashMap<String, u64>>,
    pub(crate) effects: Vec<EffectSlot>,
    pub(crate) next_effect: u64,
    pub(crate) error: Option<KernelError>,
    pub(crate) ready: watch::Sender<FiberState>,
}

#[derive(Clone)]
pub(crate) struct ServiceSlot {
    pub(crate) value: Arc<dyn Any + Send + Sync>,
}

pub(crate) struct Runtime {
    pub(crate) next_fiber: AtomicU64,
    pub(crate) next_realm: AtomicU64,
    pub(crate) next_listener: AtomicU64,
    pub(crate) fibers: Mutex<HashMap<FiberId, FiberRec>>,
    pub(crate) services: Mutex<HashMap<(u64, String), ServiceSlot>>,
    pub(crate) service_notify: Notify,
    pub(crate) events: Mutex<crate::events::EventBus>,
}

impl Runtime {
    pub(crate) fn set_state(&self, id: FiberId, state: FiberState) {
        {
            let mut fibers = self.fibers.lock().expect("fiber table lock");
            if let Some(rec) = fibers.get_mut(&id) {
                rec.state = state;
                rec.ready.send_replace(state);
            }
        }
        if matches!(
            state,
            FiberState::Failed | FiberState::Unloading | FiberState::Disposed
        ) {
            self.service_notify.notify_waiters();
        }
    }
}

pub(crate) async fn dispose_fiber(rt: &Arc<Runtime>, id: FiberId) {
    let already = {
        let mut fibers = rt.fibers.lock().expect("fiber table lock");
        let Some(rec) = fibers.get_mut(&id) else {
            return;
        };
        match rec.state {
            FiberState::Disposed => true,
            FiberState::Unloading => false,
            _ => {
                rec.state = FiberState::Unloading;
                rec.ready.send_replace(FiberState::Unloading);
                false
            }
        }
    };
    if already {
        return;
    }
    rt.service_notify.notify_waiters();
    let slots = {
        let mut fibers = rt.fibers.lock().expect("fiber table lock");
        fibers
            .get_mut(&id)
            .map(|rec| std::mem::take(&mut rec.effects))
            .unwrap_or_default()
    };
    for slot in slots.into_iter().rev() {
        if let Some(cleanup) = slot.cleanup {
            cleanup().await;
        }
    }
    rt.set_state(id, FiberState::Disposed);
}

pub(crate) fn alloc_fiber(
    rt: &Arc<Runtime>,
    parent: Option<FiberId>,
    kind: PluginKind,
    inject: Vec<String>,
    isolate: Arc<HashMap<String, u64>>,
) -> FiberId {
    let id = FiberId(rt.next_fiber.fetch_add(1, Ordering::Relaxed));
    let (ready, _) = watch::channel(FiberState::Pending);
    rt.fibers.lock().expect("fiber table lock").insert(
        id,
        FiberRec {
            state: FiberState::Pending,
            kind,
            parent,
            inject,
            isolate,
            effects: Vec::new(),
            next_effect: 1,
            error: None,
            ready,
        },
    );
    id
}
