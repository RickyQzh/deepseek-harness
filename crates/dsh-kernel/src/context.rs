//! Context and plugin mount.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::KernelError;
use crate::fiber::{
    FiberHandle, FiberId, FiberRec, FiberState, PluginKind, Runtime, alloc_fiber, dispose_fiber,
    push_effect,
};

/// Cloneable handle to one fiber in a shared runtime.
#[derive(Clone)]
pub struct Context {
    pub(crate) rt: Arc<Runtime>,
    pub(crate) fiber_id: FiberId,
    pub(crate) isolate: Arc<HashMap<String, u64>>,
}

impl Context {
    /// Create a root context whose fiber is `Active`.
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let (ready, _) = watch::channel(FiberState::Active);
        let mut fibers = HashMap::new();
        fibers.insert(
            FiberId(0),
            FiberRec {
                state: FiberState::Active,
                kind: PluginKind::Host,
                parent: None,
                inject: Vec::new(),
                isolate: Arc::new(HashMap::new()),
                effects: Vec::new(),
                next_effect: 1,
                error: None,
                ready,
            },
        );
        let rt = Arc::new(Runtime {
            next_fiber: AtomicU64::new(1),
            fibers: Mutex::new(fibers),
            services: Mutex::new(HashMap::new()),
            service_notify: tokio::sync::Notify::new(),
        });
        Context {
            rt,
            fiber_id: FiberId(0),
            isolate: Arc::new(HashMap::new()),
        }
    }

    /// Id of the fiber this handle views.
    #[must_use]
    pub fn fiber_id(&self) -> FiberId {
        self.fiber_id
    }

    /// Current lifecycle state of this handle's fiber.
    #[must_use]
    pub fn fiber_state(&self) -> FiberState {
        self.rt
            .fibers
            .lock()
            .expect("fiber table lock")
            .get(&self.fiber_id)
            .map(|rec| rec.state)
            .unwrap_or(FiberState::Disposed)
    }

    /// Mount a host plugin. `setup` runs on a child fiber.
    pub fn plugin<F, Fut>(&self, setup: F) -> FiberHandle
    where
        F: FnOnce(Context) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), KernelError>> + Send + 'static,
    {
        self.spawn_plugin(PluginKind::Host, Vec::new(), setup)
    }

    /// Mount a host plugin that stays `Pending` until every `inject` name is present.
    pub fn plugin_injecting<F, Fut>(&self, inject: &[&str], setup: F) -> FiberHandle
    where
        F: FnOnce(Context) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), KernelError>> + Send + 'static,
    {
        self.spawn_plugin(
            PluginKind::Host,
            inject.iter().map(|name| (*name).to_string()).collect(),
            setup,
        )
    }

    pub(crate) fn realm_u64(&self, service: &str) -> u64 {
        self.isolate.get(service).copied().unwrap_or(0)
    }

    /// Publish `value` under `name` in this context's realm for that name.
    ///
    /// # Errors
    ///
    /// `ServiceAlreadyProvided` when the realm already has `name`; `InactiveEffect` when this fiber cannot register effects.
    pub fn provide<T: Send + Sync + 'static>(
        &self,
        name: impl Into<String>,
        value: T,
    ) -> Result<crate::Disposer, KernelError> {
        let name = name.into();
        let realm = self.realm_u64(&name);
        {
            let mut services = self.rt.services.lock().expect("service table lock");
            let key = (realm, name.clone());
            if services.contains_key(&key) {
                return Err(KernelError::ServiceAlreadyProvided { name });
            }
            services.insert(
                key,
                crate::fiber::ServiceSlot {
                    value: Arc::new(value),
                },
            );
        }
        self.rt.service_notify.notify_waiters();
        let rt = Arc::clone(&self.rt);
        let removed = name.clone();
        self.effect(move || async move {
            rt.services
                .lock()
                .expect("service table lock")
                .remove(&(realm, removed));
            rt.service_notify.notify_waiters();
        })
    }

    /// Return a service if it is present in this context's realm. Does not wait.
    #[must_use]
    pub fn get<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        let realm = self.realm_u64(name);
        let services = self.rt.services.lock().expect("service table lock");
        let slot = services.get(&(realm, name.to_string()))?;
        Arc::clone(&slot.value).downcast::<T>().ok()
    }

    /// Wait until `name` is present in this context's realm or this fiber cannot wait.
    ///
    /// # Errors
    ///
    /// `InjectWaitDisposed` when this fiber is `Failed`, `Unloading`, or `Disposed` before the service appears; `ServiceTypeMismatch` when the stored value is not `T`.
    pub async fn inject<T: Send + Sync + 'static>(
        &self,
        name: &str,
    ) -> Result<Arc<T>, KernelError> {
        loop {
            match self.fiber_state() {
                FiberState::Failed | FiberState::Unloading | FiberState::Disposed => {
                    return Err(KernelError::InjectWaitDisposed {
                        name: name.to_string(),
                    });
                }
                FiberState::Pending | FiberState::Loading | FiberState::Active => {}
            }
            if let Some(slot) = {
                let services = self.rt.services.lock().expect("service table lock");
                services
                    .get(&(self.realm_u64(name), name.to_string()))
                    .cloned()
            } {
                return Arc::clone(&slot.value).downcast::<T>().map_err(|_| {
                    KernelError::ServiceTypeMismatch {
                        name: name.to_string(),
                    }
                });
            }
            self.rt.service_notify.notified().await;
        }
    }

    /// Register an async cleanup on this fiber. Runs in reverse order on unload.
    ///
    /// # Errors
    ///
    /// `InactiveEffect` when the fiber is `Failed`, `Unloading`, or `Disposed`.
    pub fn effect<F, Fut>(&self, cleanup: F) -> Result<crate::Disposer, KernelError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut fibers = self.rt.fibers.lock().expect("fiber table lock");
        let rec = fibers
            .get_mut(&self.fiber_id)
            .ok_or(KernelError::InactiveEffect)?;
        match rec.state {
            FiberState::Pending | FiberState::Loading | FiberState::Active => {}
            FiberState::Failed | FiberState::Unloading | FiberState::Disposed => {
                return Err(KernelError::InactiveEffect);
            }
        }
        let effect_id = push_effect(rec, Box::new(move || Box::pin(cleanup())));
        Ok(crate::Disposer {
            rt: Arc::clone(&self.rt),
            fiber_id: self.fiber_id,
            effect_id,
        })
    }

    pub(crate) fn spawn_plugin<F, Fut>(
        &self,
        kind: PluginKind,
        inject: Vec<String>,
        setup: F,
    ) -> FiberHandle
    where
        F: FnOnce(Context) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), KernelError>> + Send + 'static,
    {
        let child_id = alloc_fiber(
            &self.rt,
            Some(self.fiber_id),
            kind,
            inject,
            Arc::clone(&self.isolate),
        );
        let child = Context {
            rt: Arc::clone(&self.rt),
            fiber_id: child_id,
            isolate: Arc::clone(&self.isolate),
        };
        let rt_child = Arc::clone(&self.rt);
        match self.effect(move || async move {
            dispose_fiber(&rt_child, child_id).await;
        }) {
            Ok(_) => {
                tokio::spawn(async move {
                    run_setup(child, setup).await;
                });
            }
            Err(_) => {
                self.rt.set_state(child_id, FiberState::Disposed);
            }
        }
        FiberHandle {
            rt: Arc::clone(&self.rt),
            fiber_id: child_id,
        }
    }

    /// Dispose the fiber this handle views and wait until `Disposed`.
    pub async fn dispose(&self) {
        dispose_fiber(&self.rt, self.fiber_id).await;
    }
}

async fn run_setup<F, Fut>(ctx: Context, setup: F)
where
    F: FnOnce(Context) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), KernelError>> + Send + 'static,
{
    let inject = ctx
        .rt
        .fibers
        .lock()
        .expect("fiber table lock")
        .get(&ctx.fiber_id)
        .map(|rec| rec.inject.clone())
        .unwrap_or_default();
    while !inject.is_empty() {
        match ctx.fiber_state() {
            FiberState::Unloading | FiberState::Disposed => return,
            FiberState::Failed => return,
            FiberState::Pending | FiberState::Loading | FiberState::Active => {}
        }
        let ready = inject.iter().all(|name| {
            ctx.rt
                .services
                .lock()
                .expect("service table lock")
                .contains_key(&(ctx.realm_u64(name), name.clone()))
        });
        if ready {
            break;
        }
        ctx.rt.service_notify.notified().await;
    }
    ctx.rt.set_state(ctx.fiber_id, FiberState::Loading);
    match setup(ctx.clone()).await {
        Ok(()) => ctx.rt.set_state(ctx.fiber_id, FiberState::Active),
        Err(error) => {
            let slots = {
                let mut fibers = ctx.rt.fibers.lock().expect("fiber table lock");
                if let Some(rec) = fibers.get_mut(&ctx.fiber_id) {
                    rec.error = Some(error);
                    std::mem::take(&mut rec.effects)
                } else {
                    Vec::new()
                }
            };
            for slot in slots.into_iter().rev() {
                if let Some(cleanup) = slot.cleanup {
                    cleanup().await;
                }
            }
            ctx.rt.set_state(ctx.fiber_id, FiberState::Failed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Context;
    use crate::{FiberId, FiberState, KernelError};

    #[tokio::test]
    async fn root_fiber_is_active_with_id_zero() {
        let root = Context::new();
        assert_eq!(root.fiber_id(), FiberId(0));
        assert_eq!(root.fiber_state(), FiberState::Active);
    }

    #[tokio::test]
    async fn plugin_reaches_active_after_setup() {
        let root = Context::new();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = root.plugin(move |ctx| async move {
            tx.send(ctx.fiber_state()).ok();
            Ok(())
        });
        let during = rx.await.unwrap();
        assert_eq!(during, FiberState::Loading);
        handle.await_ready().await.unwrap();
        assert_eq!(handle.state(), FiberState::Active);
        assert_ne!(handle.context().fiber_id(), FiberId(0));
    }

    #[tokio::test]
    async fn failed_setup_settles_failed() {
        let root = Context::new();
        let handle = root.plugin(|_ctx| async { Err(KernelError::SetupFailed("boom".into())) });
        let err = handle.await_ready().await.unwrap_err();
        assert_eq!(err, KernelError::SetupFailed("boom".into()));
        assert_eq!(handle.state(), FiberState::Failed);
    }

    #[tokio::test]
    async fn dispose_reaches_disposed() {
        let root = Context::new();
        let handle = root.plugin(|_ctx| async { Ok(()) });
        handle.await_ready().await.unwrap();
        handle.dispose().await;
        assert_eq!(handle.state(), FiberState::Disposed);
    }

    #[tokio::test]
    async fn dispose_runs_effects_in_reverse_order() {
        use std::sync::{Arc, Mutex};
        let root = Context::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        let handle = root.plugin({
            let order = Arc::clone(&order);
            move |ctx| async move {
                let first = Arc::clone(&order);
                ctx.effect(move || async move {
                    first.lock().expect("order").push("first");
                })
                .unwrap();
                let second = Arc::clone(&order);
                ctx.effect(move || async move {
                    second.lock().expect("order").push("second");
                })
                .unwrap();
                Ok(())
            }
        });
        handle.await_ready().await.unwrap();
        handle.dispose().await;
        assert_eq!(*order.lock().expect("order"), vec!["second", "first"]);
        assert_eq!(handle.state(), FiberState::Disposed);
    }

    #[tokio::test]
    async fn failed_setup_rolls_back_collected_cleanup() {
        use std::sync::{Arc, Mutex};
        let root = Context::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        let handle = root.plugin({
            let order = Arc::clone(&order);
            move |ctx| async move {
                let first = Arc::clone(&order);
                ctx.effect(move || async move {
                    first.lock().expect("order").push("first");
                })
                .unwrap();
                let second = Arc::clone(&order);
                ctx.effect(move || async move {
                    second.lock().expect("order").push("second");
                })
                .unwrap();
                Err(KernelError::SetupFailed("boom".into()))
            }
        });
        let err = handle.await_ready().await.unwrap_err();
        assert_eq!(err, KernelError::SetupFailed("boom".into()));
        assert_eq!(handle.state(), FiberState::Failed);
        assert_eq!(*order.lock().expect("order"), vec!["second", "first"]);
    }

    #[tokio::test]
    async fn effect_rejected_while_unloading() {
        let root = Context::new();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = root.plugin(move |ctx| async move {
            ctx.effect(move || async move {
                let _ = entered_tx.send(());
                let _ = release_rx.await;
            })
            .unwrap();
            Ok(())
        });
        handle.await_ready().await.unwrap();
        let child = handle.context();
        let dispose_task = tokio::spawn({
            let handle = handle.clone();
            async move { handle.dispose().await }
        });
        entered_rx.await.unwrap();
        assert_eq!(handle.state(), FiberState::Unloading);
        let err = child.effect(|| async {}).map(|_| ()).unwrap_err();
        assert_eq!(err, KernelError::InactiveEffect);
        release_tx.send(()).unwrap();
        dispose_task.await.unwrap();
        assert_eq!(handle.state(), FiberState::Disposed);
    }

    #[tokio::test]
    async fn inject_waits_until_provider_is_present() {
        let root = Context::new();
        let waiter = root.plugin_injecting(&["tools"], |ctx| async move {
            let value = ctx
                .get::<String>("tools")
                .expect("visible after inject-wait");
            assert_eq!(value.as_str(), "ok");
            Ok(())
        });
        tokio::task::yield_now().await;
        assert_eq!(waiter.state(), FiberState::Pending);
        let provider = root.plugin(|ctx| async move {
            ctx.provide("tools", String::from("ok"))?;
            Ok(())
        });
        provider.await_ready().await.unwrap();
        waiter.await_ready().await.unwrap();
        assert_eq!(waiter.state(), FiberState::Active);
    }

    #[tokio::test]
    async fn inject_method_waits_and_downcasts() {
        let root = Context::new();
        let waiter = tokio::spawn({
            let root = root.clone();
            async move { root.inject::<u32>("n").await }
        });
        tokio::task::yield_now().await;
        root.provide("n", 7_u32).unwrap();
        assert_eq!(*waiter.await.unwrap().unwrap(), 7);
    }

    #[tokio::test]
    async fn inject_unblocks_when_waiting_fiber_is_disposed() {
        let root = Context::new();
        let waiter = root.plugin_injecting(&["never"], |_ctx| async { Ok(()) });
        tokio::task::yield_now().await;
        assert_eq!(waiter.state(), FiberState::Pending);
        waiter.dispose().await;
        let err = waiter.await_ready().await.unwrap_err();
        assert_eq!(err, KernelError::InactiveEffect);
    }

    #[tokio::test]
    async fn provide_duplicate_in_same_realm_fails() {
        let root = Context::new();
        root.provide("dup", 1_u8).unwrap();
        let err = root.provide("dup", 2_u8).unwrap_err();
        assert_eq!(
            err,
            KernelError::ServiceAlreadyProvided { name: "dup".into() }
        );
    }
}
