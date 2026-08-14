//! Context and plugin mount.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::KernelError;
use crate::fiber::{
    FiberHandle, FiberId, FiberRec, FiberState, PluginKind, Runtime, alloc_fiber, dispose_fiber,
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
        let rt = Arc::clone(&self.rt);
        tokio::spawn(async move {
            run_setup(child, setup).await;
            let _ = rt;
        });
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
    ctx.rt.set_state(ctx.fiber_id, FiberState::Loading);
    match setup(ctx.clone()).await {
        Ok(()) => ctx.rt.set_state(ctx.fiber_id, FiberState::Active),
        Err(error) => {
            {
                let mut fibers = ctx.rt.fibers.lock().expect("fiber table lock");
                if let Some(rec) = fibers.get_mut(&ctx.fiber_id) {
                    rec.error = Some(error);
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
}
