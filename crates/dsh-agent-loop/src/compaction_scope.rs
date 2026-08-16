//! Task-local live-session access for compaction listeners.

use std::future::Future;
use std::sync::Mutex;

use dsh_session::Session;
use dsh_tools::AbortFlag;

use crate::{LoopAgent, LoopOptions};

enum SessionAccess {
    Exclusive(usize),
    Shared(usize),
}

/// Live session, abort flag, and loop options for one in-flight request-error or pre-step waterfall.
///
/// Exclusive and Shared drivers enter this scope around those waterfalls and do not
/// use the [`LoopAgent`] mutex (Shared) or `&mut LoopAgent` (Exclusive) until `run`
/// returns. Listeners must call [`Self::with_session`] only for synchronous work and
/// must not `.await` inside the callback.
pub struct CompactionScope {
    access: SessionAccess,
    abort: AbortFlag,
    options: LoopOptions,
}

// SAFETY: Exclusive access is a pointer into the `LoopAgent` the driver parks for the
// waterfall; Shared access is a pointer to that agent's `Mutex`. The driver does not
// alias either until `run` returns, and callbacks do not hold the Shared mutex across
// `.await`.
unsafe impl Send for CompactionScope {}
unsafe impl Sync for CompactionScope {}

tokio::task_local! {
    static COMPACTION_SCOPE: CompactionScope;
}

impl CompactionScope {
    /// Exclusive driver: `session` is parked for the waterfall and not aliased until `run` returns.
    pub(crate) fn exclusive(session: &mut Session, abort: AbortFlag, options: LoopOptions) -> Self {
        Self {
            access: SessionAccess::Exclusive(std::ptr::from_mut(session) as usize),
            abort,
            options,
        }
    }

    /// Shared driver: `state` stays locked only inside [`Self::with_session`], never across `.await`.
    pub(crate) fn shared(state: &Mutex<LoopAgent>, abort: AbortFlag, options: LoopOptions) -> Self {
        Self {
            access: SessionAccess::Shared(std::ptr::from_ref(state) as usize),
            abort,
            options,
        }
    }

    /// Run `fut` with this scope as the task-local compaction context.
    pub async fn run<F, R>(self, fut: F) -> R
    where
        F: Future<Output = R>,
    {
        COMPACTION_SCOPE.scope(self, fut).await
    }

    /// Apply `f` to the task-local compaction scope when one is entered.
    pub fn try_current<F, R>(f: F) -> Option<R>
    where
        F: FnOnce(&Self) -> R,
    {
        COMPACTION_SCOPE.try_with(f).ok()
    }

    /// Borrow the live session for a synchronous callback.
    ///
    /// Shared drivers lock `Mutex<LoopAgent>` only for `f`. Exclusive drivers use the
    /// parked session pointer. Do not `.await` inside `f`.
    pub fn with_session<R>(&self, f: impl FnOnce(&mut Session) -> R) -> R {
        match self.access {
            SessionAccess::Exclusive(ptr) => {
                // SAFETY: `run` keeps Exclusive's `LoopAgent` unused until this scope ends.
                let session = unsafe { &mut *(ptr as *mut Session) };
                f(session)
            }
            SessionAccess::Shared(ptr) => {
                // SAFETY: `run` keeps the `Mutex<LoopAgent>` alive for the waterfall.
                let state = unsafe { &*(ptr as *const Mutex<LoopAgent>) };
                let mut agent = state.lock().expect("loop agent state");
                f(&mut agent.session)
            }
        }
    }

    /// Cancellation flag for the in-flight reservation.
    #[must_use]
    pub fn abort(&self) -> &AbortFlag {
        &self.abort
    }

    /// Loop route used to summarize when the request header omits a target.
    #[must_use]
    pub fn options(&self) -> &LoopOptions {
        &self.options
    }
}
