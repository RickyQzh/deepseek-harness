//! Event bus stored on the kernel runtime.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::KernelError;
use crate::context::Context;
use crate::fiber::FiberId;

/// Type-erased emit/serial/parallel payload.
#[derive(Clone)]
pub struct Payload(Arc<dyn Any + Send + Sync>);

impl Payload {
    /// Wrap a value.
    #[must_use]
    pub fn new<T: Send + Sync + 'static>(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// Borrow the value when the stored type is `T`.
    #[must_use]
    pub fn downcast_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

/// Listener placement and scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OnOptions {
    /// Insert before existing listeners for this name.
    pub prepend: bool,
    /// Receive events from every fiber in the runtime.
    pub global: bool,
}

impl Default for OnOptions {
    fn default() -> Self {
        Self {
            prepend: false,
            global: true,
        }
    }
}

/// Continuation passed to a waterfall listener. Call `next(value).await` to delegate.
pub type Next<T> = Box<dyn FnOnce(T) -> Pin<Box<dyn Future<Output = T> + Send>> + Send>;

type AsyncListener = Arc<dyn Fn(Payload) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

pub(crate) struct ListenerEntry {
    id: u64,
    fiber_id: FiberId,
    global: bool,
    cb: AsyncListener,
}

type TypedWaterfall<T> =
    Arc<dyn Fn(T, Next<T>) -> Pin<Box<dyn Future<Output = T> + Send>> + Send + Sync>;

pub(crate) struct WaterfallEntry {
    id: u64,
    type_id: TypeId,
    /// `TypedWaterfall<T>` boxed as `Any` for the `T` used at registration.
    cb: Box<dyn Any + Send + Sync>,
}

/// Named listener tables for one runtime.
#[derive(Default)]
pub(crate) struct EventBus {
    pub(crate) listeners: HashMap<String, Vec<ListenerEntry>>,
    pub(crate) waterfalls: HashMap<String, Vec<WaterfallEntry>>,
}

fn ancestor_snapshot(
    parents: &std::collections::HashMap<FiberId, Option<FiberId>>,
    emitter: FiberId,
    hook: FiberId,
) -> bool {
    let mut current = Some(emitter);
    while let Some(id) = current {
        if id == hook {
            return true;
        }
        current = parents.get(&id).copied().flatten();
    }
    false
}

impl Context {
    /// Register an async listener (`global: true`, append).
    ///
    /// # Errors
    ///
    /// `InactiveEffect` when this fiber cannot register effects.
    pub fn on<F, Fut>(&self, name: &str, listener: F) -> Result<crate::Disposer, KernelError>
    where
        F: Fn(Payload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.on_with(name, OnOptions::default(), listener)
    }

    /// Register an async listener with placement and scope options.
    ///
    /// # Errors
    ///
    /// `InactiveEffect` when this fiber cannot register effects.
    pub fn on_with<F, Fut>(
        &self,
        name: &str,
        options: OnOptions,
        listener: F,
    ) -> Result<crate::Disposer, KernelError>
    where
        F: Fn(Payload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self
            .rt
            .next_listener
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name_owned = name.to_string();
        let cb: AsyncListener = Arc::new(move |payload| Box::pin(listener(payload)));
        {
            let mut bus = self.rt.events.lock().expect("event bus lock");
            let list = bus.listeners.entry(name_owned.clone()).or_default();
            let entry = ListenerEntry {
                id,
                fiber_id: self.fiber_id,
                global: options.global,
                cb,
            };
            if options.prepend {
                list.insert(0, entry);
            } else {
                list.push(entry);
            }
        }
        let rt = Arc::clone(&self.rt);
        let removed = name_owned.clone();
        match self.effect(move || async move {
            let mut bus = rt.events.lock().expect("event bus lock");
            if let Some(list) = bus.listeners.get_mut(&name_owned) {
                list.retain(|entry| entry.id != id);
            }
        }) {
            Ok(disposer) => Ok(disposer),
            Err(error) => {
                let mut bus = self.rt.events.lock().expect("event bus lock");
                if let Some(list) = bus.listeners.get_mut(&removed) {
                    list.retain(|entry| entry.id != id);
                }
                Err(error)
            }
        }
    }

    fn snapshot_listeners(&self, name: &str) -> Vec<AsyncListener> {
        let entries: Vec<(FiberId, bool, AsyncListener)> = {
            let bus = self.rt.events.lock().expect("event bus lock");
            bus.listeners
                .get(name)
                .into_iter()
                .flatten()
                .map(|entry| (entry.fiber_id, entry.global, Arc::clone(&entry.cb)))
                .collect()
        };
        let parents: std::collections::HashMap<FiberId, Option<FiberId>> = {
            let fibers = self.rt.fibers.lock().expect("fiber table lock");
            fibers.iter().map(|(id, rec)| (*id, rec.parent)).collect()
        };
        entries
            .into_iter()
            .filter(|(hook, global, _)| {
                *global || ancestor_snapshot(&parents, self.fiber_id, *hook)
            })
            .map(|(_, _, cb)| cb)
            .collect()
    }

    /// Fire listeners without awaiting them. A panicked listener does not skip the others.
    pub fn emit(&self, name: &str, payload: Payload) {
        for cb in self.snapshot_listeners(name) {
            let payload = payload.clone();
            tokio::spawn(async move {
                cb(payload).await;
            });
        }
    }

    /// Await listeners in registration order. A panicked listener is contained.
    pub async fn serial(&self, name: &str, payload: Payload) {
        for cb in self.snapshot_listeners(name) {
            let payload = payload.clone();
            let _ = tokio::spawn(async move { cb(payload).await }).await;
        }
    }

    /// Await all listeners together. A panicked listener is contained.
    pub async fn parallel(&self, name: &str, payload: Payload) {
        let mut joins = Vec::new();
        for cb in self.snapshot_listeners(name) {
            let payload = payload.clone();
            joins.push(tokio::spawn(async move { cb(payload).await }));
        }
        for join in joins {
            let _ = join.await;
        }
    }

    /// Register a waterfall listener. First registration is outermost.
    ///
    /// # Errors
    ///
    /// `InactiveEffect` when this fiber cannot register effects.
    pub fn on_waterfall<T, F, Fut>(
        &self,
        name: &str,
        listener: F,
    ) -> Result<crate::Disposer, KernelError>
    where
        T: Send + 'static,
        F: Fn(T, Next<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let id = self
            .rt
            .next_listener
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name_owned = name.to_string();
        let typed: TypedWaterfall<T> = Arc::new(move |value, next| Box::pin(listener(value, next)));
        {
            let mut bus = self.rt.events.lock().expect("event bus lock");
            bus.waterfalls
                .entry(name_owned.clone())
                .or_default()
                .push(WaterfallEntry {
                    id,
                    type_id: TypeId::of::<T>(),
                    cb: Box::new(typed),
                });
        }
        let rt = Arc::clone(&self.rt);
        let removed = name_owned.clone();
        match self.effect(move || async move {
            let mut bus = rt.events.lock().expect("event bus lock");
            if let Some(list) = bus.waterfalls.get_mut(&name_owned) {
                list.retain(|entry| entry.id != id);
            }
        }) {
            Ok(disposer) => Ok(disposer),
            Err(error) => {
                let mut bus = self.rt.events.lock().expect("event bus lock");
                if let Some(list) = bus.waterfalls.get_mut(&removed) {
                    list.retain(|entry| entry.id != id);
                }
                Err(error)
            }
        }
    }

    /// Run the waterfall. Listeners that return without `next` short-circuit.
    pub async fn waterfall<T: Send + 'static>(&self, name: &str, value: T) -> T {
        let listeners: Vec<TypedWaterfall<T>> = {
            let bus = self.rt.events.lock().expect("event bus lock");
            bus.waterfalls
                .get(name)
                .into_iter()
                .flatten()
                .filter(|entry| entry.type_id == TypeId::of::<T>())
                .map(|entry| {
                    entry
                        .cb
                        .downcast_ref::<TypedWaterfall<T>>()
                        .expect("waterfall type id matches stored callback")
                        .clone()
                })
                .collect()
        };
        fn invoke<T: Send + 'static>(
            mut rest: std::vec::IntoIter<TypedWaterfall<T>>,
            value: T,
        ) -> Pin<Box<dyn Future<Output = T> + Send>> {
            Box::pin(async move {
                match rest.next() {
                    None => value,
                    Some(listener) => {
                        let next: Next<T> = Box::new(move |v| invoke(rest, v));
                        listener(value, next).await
                    }
                }
            })
        }
        invoke(listeners.into_iter(), value).await
    }
}

#[cfg(test)]
mod tests {
    use super::{OnOptions, Payload};
    use crate::{Context, KernelError};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn waterfall_short_circuit_skips_downstream() {
        let root = Context::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_a = Arc::clone(&seen);
        root.on_waterfall::<i32, _, _>("policy", move |value, _next| {
            let seen_a = Arc::clone(&seen_a);
            async move {
                seen_a.lock().expect("seen").push(("a", value));
                99
            }
        })
        .unwrap();
        let seen_b = Arc::clone(&seen);
        root.on_waterfall::<i32, _, _>("policy", move |value, next| {
            let seen_b = Arc::clone(&seen_b);
            async move {
                seen_b.lock().expect("seen").push(("b", value));
                next(value + 1).await
            }
        })
        .unwrap();
        let out = root.waterfall("policy", 0).await;
        assert_eq!(out, 99);
        assert_eq!(*seen.lock().expect("seen"), vec![("a", 0)]);
    }

    #[tokio::test]
    async fn waterfall_next_reaches_terminal() {
        let root = Context::new();
        root.on_waterfall::<i32, _, _>("sum", |value, next| async move { next(value + 1).await })
            .unwrap();
        root.on_waterfall::<i32, _, _>("sum", |value, next| async move { next(value + 1).await })
            .unwrap();
        assert_eq!(root.waterfall("sum", 0).await, 2);
    }

    #[tokio::test]
    async fn emit_contains_listener_panic() {
        let root = Context::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        root.on("tick", |_payload| async {
            panic!("boom");
        })
        .unwrap();
        root.on("tick", move |_payload| {
            let tx = tx.clone();
            async move {
                tx.send("second").unwrap();
            }
        })
        .unwrap();
        root.emit("tick", Payload::new(()));
        let got = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("second listener ran")
            .expect("value");
        assert_eq!(got, "second");
    }

    #[tokio::test]
    async fn serial_runs_in_registration_order() {
        let root = Context::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        let a = Arc::clone(&order);
        root.on("s", move |_| {
            let a = Arc::clone(&a);
            async move {
                a.lock().expect("order").push("a");
            }
        })
        .unwrap();
        let b = Arc::clone(&order);
        root.on("s", move |_| {
            let b = Arc::clone(&b);
            async move {
                b.lock().expect("order").push("b");
            }
        })
        .unwrap();
        root.serial("s", Payload::new(())).await;
        assert_eq!(*order.lock().expect("order"), vec!["a", "b"]);
    }

    #[tokio::test]
    async fn parallel_runs_every_listener() {
        let root = Context::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        root.on("p", {
            let tx = tx.clone();
            move |_| {
                let tx = tx.clone();
                async move {
                    tx.send("a").unwrap();
                }
            }
        })
        .unwrap();
        root.on("p", move |_| {
            let tx = tx.clone();
            async move {
                tx.send("b").unwrap();
            }
        })
        .unwrap();
        root.parallel("p", Payload::new(())).await;
        let mut got = vec![rx.recv().await.unwrap(), rx.recv().await.unwrap()];
        got.sort();
        assert_eq!(got, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn scoped_listener_skips_events_from_ancestor() {
        let root = Context::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let child = root.plugin(|ctx| async move {
            ctx.on_with(
                "x",
                OnOptions {
                    prepend: false,
                    global: false,
                },
                move |_| {
                    let tx = tx.clone();
                    async move {
                        tx.send("child").unwrap();
                    }
                },
            )
            .unwrap();
            Ok(())
        });
        child.await_ready().await.unwrap();
        root.emit("x", Payload::new(()));
        let timed = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(
            timed.is_err(),
            "child-scoped listener must not see root emit"
        );
    }

    #[tokio::test]
    async fn on_on_disposed_fiber_does_not_orphan_listener() {
        let root = Context::new();
        let handle = root.plugin(|_ctx| async { Ok(()) });
        handle.await_ready().await.unwrap();
        handle.dispose().await;
        let ran = Arc::new(Mutex::new(false));
        let ran_cb = Arc::clone(&ran);
        let err = handle
            .context()
            .on("ghost", move |_| {
                let ran_cb = Arc::clone(&ran_cb);
                async move {
                    *ran_cb.lock().expect("ran") = true;
                }
            })
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err, KernelError::InactiveEffect);
        root.serial("ghost", Payload::new(())).await;
        assert!(!*ran.lock().expect("ran"));
    }

    #[tokio::test]
    async fn on_waterfall_on_disposed_fiber_does_not_orphan_listener() {
        let root = Context::new();
        let handle = root.plugin(|_ctx| async { Ok(()) });
        handle.await_ready().await.unwrap();
        handle.dispose().await;
        let ran = Arc::new(Mutex::new(false));
        let ran_cb = Arc::clone(&ran);
        let err = handle
            .context()
            .on_waterfall::<i32, _, _>("ghost", move |_value, _next| {
                let ran_cb = Arc::clone(&ran_cb);
                async move {
                    *ran_cb.lock().expect("ran") = true;
                    99
                }
            })
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err, KernelError::InactiveEffect);
        assert_eq!(root.waterfall("ghost", 0).await, 0);
        assert!(!*ran.lock().expect("ran"));
    }
}
