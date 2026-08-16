//! Process-local [`LocalJobRegistry`]: in-memory records, per-kind ids, owner fence, first-wins settlement.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dsh_jobs::{
    JobError, JobHooks, JobId, JobKind, JobOutcome, JobRead, JobRegistry, JobSnapshot, JobStart,
    JobStatus, KillResult,
};
use dsh_session::SessionId;
use tokio::sync::oneshot;

/// Default maximum `running` plus `stopping` jobs per owner (or the shared unowned bucket).
pub const DEFAULT_MAX_CONCURRENT_PER_OWNER: u32 = 10;

type DoneListener = Arc<dyn Fn(JobSnapshot) + Send + Sync>;

struct TrackedJob {
    id: JobId,
    kind: JobKind,
    label: String,
    owner_session: Option<SessionId>,
    cancel: Box<dyn Fn(Option<String>) + Send>,
    read_output: Option<Box<dyn Fn() -> String + Send>>,
    status: JobStatus,
    detail: Option<String>,
    output: Option<String>,
    started_at: i64,
    finished_at: Option<i64>,
    reported: bool,
    waiters: u32,
    wait_resolvers: Vec<oneshot::Sender<()>>,
}

struct Inner {
    store: HashMap<JobId, TrackedJob>,
    order: Vec<JobId>,
    counters: HashMap<JobKind, u32>,
    listeners: Vec<DoneListener>,
}

/// In-memory `jobs` registry. `inject::<LocalJobRegistry>()` yields `Arc<Self>`, so methods take `&self`.
pub struct LocalJobRegistry {
    inner: Arc<Mutex<Inner>>,
    max_concurrent_per_owner: u32,
}

impl LocalJobRegistry {
    /// Empty registry. `max_concurrent_per_owner` is the active-job cap per owner bucket (plugin default 10).
    #[must_use]
    pub fn new(max_concurrent_per_owner: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                store: HashMap::new(),
                order: Vec::new(),
                counters: HashMap::new(),
                listeners: Vec::new(),
            })),
            max_concurrent_per_owner,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Bounded wait for settlement. Timeout returns the current snapshot and leaves a live job running.
    ///
    /// Dropping the future uncounts the waiter so a cancelled wait cannot suppress
    /// [`Self::on_job_done`].
    ///
    /// # Errors
    ///
    /// [`JobError`] when `timeout_ms` is 0, the id is unknown, or the caller is fenced out.
    pub async fn wait(
        &self,
        id: &JobId,
        timeout_ms: u64,
        caller: Option<&SessionId>,
    ) -> Result<JobSnapshot, JobError> {
        if timeout_ms == 0 {
            return Err(JobError::new(
                "invalid wait timeout: expected a positive number of milliseconds, got 0",
            ));
        }
        let rx = {
            let mut inner = self.lock();
            let job = expect_job(&inner, id)?;
            assert_access(job, caller)?;
            if job.status.is_terminal() {
                let job = inner
                    .store
                    .get_mut(id)
                    .ok_or_else(|| JobError::new(format!("unknown job {id}")))?;
                job.reported = true;
                return Ok(snapshot(job));
            }
            let (tx, rx) = oneshot::channel();
            let job = inner
                .store
                .get_mut(id)
                .ok_or_else(|| JobError::new(format!("unknown job {id}")))?;
            job.waiters = job.waiters.saturating_add(1);
            job.wait_resolvers.push(tx);
            rx
        };
        let _guard = WaiterGuard {
            inner: Arc::clone(&self.inner),
            id: id.clone(),
        };
        tokio::select! {
            _ = rx => {}
            () = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)) => {}
        }
        let mut inner = self.lock();
        let job = inner
            .store
            .get_mut(id)
            .ok_or_else(|| JobError::new(format!("unknown job {id}")))?;
        if job.status.is_terminal() {
            job.reported = true;
        }
        Ok(snapshot(job))
    }

    /// Register a completion listener. It runs after the record is terminal; panics are contained.
    pub fn on_job_done(&self, listener: impl Fn(JobSnapshot) + Send + Sync + 'static) {
        self.lock().listeners.push(Arc::new(listener));
    }

    /// Cancel every live job. Idempotent. Used by [`Drop`] and kernel dispose.
    pub fn cancel_live(&self, reason: &str) {
        let mut inner = self.lock();
        let ids: Vec<JobId> = inner.order.clone();
        let mut failed = Vec::new();
        for id in ids {
            let Some(job) = inner.store.get(&id) else {
                continue;
            };
            if job.status.is_terminal() {
                continue;
            }
            let cancel_result = panic_cancel(&job.cancel, Some(reason.to_string()));
            if let Err(message) = cancel_result {
                failed.push((
                    id,
                    JobOutcome {
                        status: JobStatus::Failed,
                        detail: Some(format!(
                            "cancel threw during teardown; work may be orphaned: {message}"
                        )),
                        output: None,
                    },
                ));
                continue;
            }
            if let Some(job) = inner.store.get_mut(&id) {
                job.reported = true;
                job.status = JobStatus::Stopping;
            }
        }
        drop(inner);
        for (id, outcome) in failed {
            settle_on(&self.inner, &id, outcome);
        }
    }
}

impl Default for LocalJobRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENT_PER_OWNER)
    }
}

impl Drop for LocalJobRegistry {
    fn drop(&mut self) {
        self.cancel_live("jobs service disposed");
    }
}

impl JobRegistry for LocalJobRegistry {
    fn start(&self, spec: JobStart) -> Result<JobId, JobError> {
        LocalJobRegistry::start(self, spec)
    }

    fn list(&self, caller: Option<&SessionId>) -> Vec<JobSnapshot> {
        LocalJobRegistry::list(self, caller)
    }

    fn get(&self, id: &JobId, caller: Option<&SessionId>) -> Result<JobSnapshot, JobError> {
        LocalJobRegistry::get(self, id, caller)
    }

    fn read(&self, id: &JobId, caller: Option<&SessionId>) -> Result<JobRead, JobError> {
        LocalJobRegistry::read(self, id, caller)
    }

    fn kill(
        &self,
        id: &JobId,
        caller: Option<&SessionId>,
        reason: Option<String>,
    ) -> Result<KillResult, JobError> {
        LocalJobRegistry::kill(self, id, caller, reason)
    }
}

impl LocalJobRegistry {
    /// Admit, invoke `run`, issue `<kind>-N`, and spawn settlement.
    ///
    /// Holds the registry mutex through the active-count check, `run()`, and insert
    /// so concurrent `start`s cannot exceed `max_concurrent_per_owner`. `run` must
    /// not call back into this registry.
    ///
    /// # Errors
    ///
    /// Empty label or per-owner active limit. `run` is called only after admission.
    pub fn start(&self, spec: JobStart) -> Result<JobId, JobError> {
        if spec.label.is_empty() {
            return Err(JobError::new(
                "invalid job label: expected a non-empty string",
            ));
        }
        let mut inner = self.lock();
        let active = active_count(&inner, spec.owner_session.as_ref());
        if active >= self.max_concurrent_per_owner {
            return Err(JobError::new(format!(
                "background job limit reached for this owner (limit: {}); use job_kill to stop an unneeded job, wait for it to finish, then retry",
                self.max_concurrent_per_owner
            )));
        }
        let kind = spec.kind;
        let label = spec.label;
        let owner_session = spec.owner_session;
        let hooks: JobHooks = (spec.run)();
        let count = inner.counters.entry(kind).or_insert(0);
        *count = count.saturating_add(1);
        let n = *count;
        let id = JobId::new(format!("{}-{n}", kind.as_str()));
        let started_at = now_ms();
        inner.store.insert(
            id.clone(),
            TrackedJob {
                id: id.clone(),
                kind,
                label,
                owner_session,
                cancel: hooks.cancel,
                read_output: hooks.read_output,
                status: JobStatus::Running,
                detail: None,
                output: None,
                started_at,
                finished_at: None,
                reported: false,
                waiters: 0,
                wait_resolvers: Vec::new(),
            },
        );
        inner.order.push(id.clone());
        drop(inner);
        let inner = Arc::clone(&self.inner);
        let settle_id = id.clone();
        tokio::spawn(async move {
            let join = tokio::spawn(hooks.done);
            let outcome = match join.await {
                Ok(outcome) => normalize_outcome(outcome),
                Err(error) => JobOutcome {
                    status: JobStatus::Failed,
                    detail: Some(format!("job done future panicked: {error}")),
                    output: None,
                },
            };
            settle_on(&inner, &settle_id, outcome);
        });
        Ok(id)
    }

    /// Caller-owned and unowned jobs in registration order.
    #[must_use]
    pub fn list(&self, caller: Option<&SessionId>) -> Vec<JobSnapshot> {
        let inner = self.lock();
        inner
            .order
            .iter()
            .filter_map(|id| inner.store.get(id))
            .filter(|job| list_visible(job, caller))
            .map(snapshot)
            .collect()
    }

    /// Non-consuming snapshot.
    ///
    /// # Errors
    ///
    /// Unknown id or foreign owner.
    pub fn get(&self, id: &JobId, caller: Option<&SessionId>) -> Result<JobSnapshot, JobError> {
        let inner = self.lock();
        let job = expect_job(&inner, id)?;
        assert_access(job, caller)?;
        Ok(snapshot(job))
    }

    /// Consuming stream read, or idempotent terminal output.
    ///
    /// # Errors
    ///
    /// Unknown id or foreign owner.
    pub fn read(&self, id: &JobId, caller: Option<&SessionId>) -> Result<JobRead, JobError> {
        let mut inner = self.lock();
        let job = inner
            .store
            .get_mut(id)
            .ok_or_else(|| JobError::new(format!("unknown job {id}")))?;
        assert_access(job, caller)?;
        let text = if let Some(read_output) = &job.read_output {
            read_output()
        } else if job.status.is_terminal() {
            job.output.clone().unwrap_or_default()
        } else {
            String::new()
        };
        if job.status.is_terminal() {
            job.reported = true;
        }
        Ok(JobRead {
            text,
            snapshot: snapshot(job),
        })
    }

    /// Cancel a live job or acknowledge an already-terminal one.
    ///
    /// # Errors
    ///
    /// Unknown id, foreign owner, or panicking producer `cancel`.
    pub fn kill(
        &self,
        id: &JobId,
        caller: Option<&SessionId>,
        reason: Option<String>,
    ) -> Result<KillResult, JobError> {
        let mut inner = self.lock();
        let job = inner
            .store
            .get_mut(id)
            .ok_or_else(|| JobError::new(format!("unknown job {id}")))?;
        assert_access(job, caller)?;
        if job.status.is_terminal() {
            job.reported = true;
            return Ok(KillResult::AlreadyFinished);
        }
        if let Err(message) = panic_cancel(&job.cancel, reason) {
            return Err(JobError::new(message));
        }
        job.status = JobStatus::Stopping;
        job.reported = true;
        Ok(KillResult::Requested)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn same_owner(left: Option<&SessionId>, right: Option<&SessionId>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left.as_str() == right.as_str(),
        _ => false,
    }
}

fn is_active(status: JobStatus) -> bool {
    matches!(status, JobStatus::Running | JobStatus::Stopping)
}

fn active_count(inner: &Inner, owner: Option<&SessionId>) -> u32 {
    let mut count = 0_u32;
    for job in inner.store.values() {
        if same_owner(job.owner_session.as_ref(), owner) && is_active(job.status) {
            count = count.saturating_add(1);
        }
    }
    count
}

fn list_visible(job: &TrackedJob, caller: Option<&SessionId>) -> bool {
    match job.owner_session.as_ref() {
        None => true,
        Some(owner) => caller.is_some_and(|caller| caller.as_str() == owner.as_str()),
    }
}

fn expect_job<'a>(inner: &'a Inner, id: &JobId) -> Result<&'a TrackedJob, JobError> {
    inner
        .store
        .get(id)
        .ok_or_else(|| JobError::new(format!("unknown job {id}")))
}

fn assert_access(job: &TrackedJob, caller: Option<&SessionId>) -> Result<(), JobError> {
    if let Some(owner) = job.owner_session.as_ref() {
        let allowed = caller.is_some_and(|caller| caller.as_str() == owner.as_str());
        if !allowed {
            return Err(JobError::new(format!(
                "job {} belongs to another session",
                job.id
            )));
        }
    }
    Ok(())
}

fn snapshot(job: &TrackedJob) -> JobSnapshot {
    JobSnapshot::new(
        job.id.clone(),
        job.kind,
        job.label.clone(),
        job.status,
        job.detail.clone(),
        job.started_at,
        job.finished_at,
        job.reported,
        job.owner_session.clone(),
    )
}

fn normalize_outcome(outcome: JobOutcome) -> JobOutcome {
    if outcome.status.is_terminal() {
        outcome
    } else {
        JobOutcome {
            status: JobStatus::Failed,
            detail: outcome.detail.or_else(|| {
                Some(format!(
                    "producer settled with non-terminal status {}",
                    outcome.status
                ))
            }),
            output: outcome.output,
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "job cancel panicked".into()
}

fn panic_cancel(
    cancel: &(dyn Fn(Option<String>) + Send),
    reason: Option<String>,
) -> Result<(), String> {
    catch_unwind(AssertUnwindSafe(|| cancel(reason)))
        .map_err(|payload| panic_message(payload.as_ref()))
}

/// Decrements `waiters` and drops a closed resolver when a `wait` future is
/// cancelled, times out, or returns.
struct WaiterGuard {
    inner: Arc<Mutex<Inner>>,
    id: JobId,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(job) = inner.store.get_mut(&self.id) else {
            return;
        };
        job.waiters = job.waiters.saturating_sub(1);
        job.wait_resolvers.retain(|tx| !tx.is_closed());
    }
}

fn settle_on(inner: &Arc<Mutex<Inner>>, id: &JobId, outcome: JobOutcome) {
    let mut guard = inner.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(job) = guard.store.get_mut(id) else {
        return;
    };
    if job.status.is_terminal() {
        return;
    }
    job.status = outcome.status;
    job.detail = outcome.detail;
    job.output = outcome.output;
    job.finished_at = Some(now_ms());
    if job.waiters > 0 {
        job.reported = true;
    }
    let snap = snapshot(job);
    let resolvers = std::mem::take(&mut job.wait_resolvers);
    let listeners = guard.listeners.clone();
    drop(guard);
    for tx in resolvers {
        let _ = tx.send(());
    }
    for listener in listeners {
        let snap = snap.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || listener(snap)));
    }
}

pub mod plugin;

pub use plugin::register;

#[cfg(test)]
mod tests {
    use super::LocalJobRegistry;
    use dsh_jobs::{JobHooks, JobKind, JobOutcome, JobStart, JobStatus, KillResult};
    use dsh_session::SessionId;
    use std::sync::{Arc, Mutex};

    fn immediate_job(kind: JobKind, label: &str, owner: Option<SessionId>) -> JobStart {
        JobStart {
            kind,
            label: label.to_string(),
            owner_session: owner,
            run: Box::new(|| JobHooks {
                cancel: Box::new(|_| {}),
                done: Box::pin(async {
                    JobOutcome {
                        status: JobStatus::Completed,
                        detail: None,
                        output: None,
                    }
                }),
                read_output: None,
            }),
        }
    }

    fn hanging_job() -> JobStart {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = std::sync::Mutex::new(Some(tx));
        JobStart {
            kind: JobKind::Bash,
            label: "hang".into(),
            owner_session: None,
            run: Box::new(move || JobHooks {
                cancel: Box::new(move |_| {
                    if let Some(sender) = tx
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        let _ = sender.send(());
                    }
                }),
                done: Box::pin(async move {
                    let _ = rx.await;
                    JobOutcome {
                        status: JobStatus::Killed,
                        detail: None,
                        output: None,
                    }
                }),
                read_output: None,
            }),
        }
    }

    #[tokio::test]
    async fn start_assigns_kind_n_and_list_is_owner_fenced() {
        let jobs = LocalJobRegistry::new(10);
        let id = jobs
            .start(immediate_job(
                JobKind::Bash,
                "echo",
                Some(SessionId::new("a")),
            ))
            .unwrap();
        assert_eq!(id.as_str(), "bash-1");
        assert!(jobs.list(Some(&SessionId::new("b"))).is_empty());
        assert_eq!(jobs.list(Some(&SessionId::new("a"))).len(), 1);
    }

    #[tokio::test]
    async fn job_kill_marks_killed() {
        let jobs = LocalJobRegistry::new(10);
        let id = jobs.start(hanging_job()).unwrap();
        jobs.kill(&id, None, Some("stop".into())).unwrap();
        assert!(matches!(
            jobs.get(&id, None).unwrap().status(),
            JobStatus::Killed | JobStatus::Stopping
        ));
    }

    #[tokio::test]
    async fn subagent_counter_is_independent_of_bash() {
        let jobs = LocalJobRegistry::new(10);
        jobs.start(immediate_job(JobKind::Subagent, "research", None))
            .unwrap();
        let bash = jobs
            .start(immediate_job(JobKind::Bash, "echo", None))
            .unwrap();
        assert_eq!(bash.as_str(), "bash-1");
    }

    #[tokio::test]
    async fn foreign_get_is_an_error() {
        let jobs = LocalJobRegistry::new(10);
        let id = jobs
            .start(immediate_job(
                JobKind::Bash,
                "echo",
                Some(SessionId::new("a")),
            ))
            .unwrap();
        let err = jobs.get(&id, Some(&SessionId::new("b"))).unwrap_err();
        assert!(err.to_string().contains("belongs to another session"));
    }

    #[tokio::test]
    async fn unknown_id_mentions_unknown_job() {
        let jobs = LocalJobRegistry::new(10);
        let err = jobs
            .get(&dsh_jobs::JobId::new("bash-99"), None)
            .unwrap_err();
        assert!(err.to_string().contains("unknown job"));
    }

    #[tokio::test]
    async fn wait_timeout_leaves_a_hanging_job_running() {
        let jobs = LocalJobRegistry::new(10);
        let id = jobs.start(hanging_job()).unwrap();
        let snap = jobs.wait(&id, 20, None).await.unwrap();
        assert_eq!(snap.status(), JobStatus::Running);
        assert_eq!(jobs.get(&id, None).unwrap().status(), JobStatus::Running);
    }

    #[tokio::test]
    async fn drop_cancels_live_jobs() {
        let jobs = LocalJobRegistry::new(10);
        let id = jobs.start(hanging_job()).unwrap();
        jobs.cancel_live("test drop");
        let status = jobs.get(&id, None).unwrap().status();
        assert!(matches!(status, JobStatus::Stopping | JobStatus::Killed));
    }

    #[tokio::test]
    async fn on_job_done_sees_a_terminal_snapshot() {
        let jobs = LocalJobRegistry::new(10);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        jobs.on_job_done(move |snap| {
            if let Some(sender) = tx
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _ = sender.send(snap);
            }
        });
        let id = jobs
            .start(immediate_job(JobKind::Bash, "echo", None))
            .unwrap();
        let snap = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .expect("on_job_done")
            .expect("listener");
        assert_eq!(snap.id(), &id);
        assert!(snap.status().is_terminal());
        assert!(snap.finished_at().is_some());
    }

    #[tokio::test]
    async fn kill_after_completion_is_already_finished() {
        let jobs = LocalJobRegistry::new(10);
        let id = jobs
            .start(immediate_job(JobKind::Bash, "echo", None))
            .unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if jobs.get(&id, None).unwrap().status().is_terminal() {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("job did not settle");
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(
            jobs.kill(&id, None, Some("late".into())).unwrap(),
            KillResult::AlreadyFinished
        );
        assert_eq!(jobs.get(&id, None).unwrap().status(), JobStatus::Completed);
    }

    #[tokio::test]
    async fn first_terminal_settlement_wins_against_later_producer() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let jobs = LocalJobRegistry::new(10);
        let id = jobs
            .start(JobStart {
                kind: JobKind::Bash,
                label: "panic-cancel".into(),
                owner_session: None,
                run: Box::new(move || JobHooks {
                    cancel: Box::new(|_| panic!("cancel boom")),
                    done: Box::pin(async move {
                        let _ = rx.await;
                        JobOutcome {
                            status: JobStatus::Completed,
                            detail: None,
                            output: None,
                        }
                    }),
                    read_output: None,
                }),
            })
            .unwrap();
        jobs.cancel_live("teardown");
        assert_eq!(jobs.get(&id, None).unwrap().status(), JobStatus::Failed);
        let _ = tx.send(());
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(jobs.get(&id, None).unwrap().status(), JobStatus::Failed);
    }

    #[tokio::test]
    async fn aborted_wait_does_not_suppress_on_job_done() {
        let jobs = Arc::new(LocalJobRegistry::new(10));
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        jobs.on_job_done(move |snap| {
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snap);
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let owner = SessionId::new("a");
        let id = jobs
            .start(JobStart {
                kind: JobKind::Bash,
                label: "hang".into(),
                owner_session: Some(owner.clone()),
                run: Box::new(move || JobHooks {
                    cancel: Box::new(|_| {}),
                    done: Box::pin(async move {
                        let _ = rx.await;
                        JobOutcome {
                            status: JobStatus::Completed,
                            detail: None,
                            output: None,
                        }
                    }),
                    read_output: None,
                }),
            })
            .unwrap();
        let wait_jobs = Arc::clone(&jobs);
        let wait_id = id.clone();
        let wait_owner = owner.clone();
        let wait =
            tokio::spawn(async move { wait_jobs.wait(&wait_id, 60_000, Some(&wait_owner)).await });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        wait.abort();
        let _ = wait.await;
        tx.send(()).expect("settle");
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(snap) = seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                assert!(
                    !snap.reported(),
                    "aborted wait must not mark the job reported"
                );
                assert_eq!(snap.status(), JobStatus::Completed);
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("on_job_done did not fire after aborted wait");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(!jobs.get(&id, Some(&owner)).unwrap().reported());
    }

    #[tokio::test]
    async fn second_start_hits_per_owner_cap() {
        let jobs = LocalJobRegistry::new(1);
        jobs.start(hanging_job()).unwrap();
        let err = jobs.start(hanging_job()).unwrap_err();
        assert!(err.to_string().contains("limit: 1"));
        assert_eq!(jobs.list(None).len(), 1);
    }

    #[tokio::test]
    async fn concurrent_start_respects_per_owner_cap() {
        let jobs = Arc::new(LocalJobRegistry::new(1));
        let first = Arc::clone(&jobs);
        let second = Arc::clone(&jobs);
        let t1 = tokio::task::spawn_blocking(move || first.start(delayed_hanging_job()));
        let t2 = tokio::task::spawn_blocking(move || second.start(delayed_hanging_job()));
        let r1 = t1.await.expect("t1");
        let r2 = t2.await.expect("t2");
        let ok = r1.is_ok() as u8 + r2.is_ok() as u8;
        let err = r1.is_err() as u8 + r2.is_err() as u8;
        assert_eq!(ok, 1, "exactly one start must admit: {r1:?} {r2:?}");
        assert_eq!(err, 1, "the other start must hit the cap: {r1:?} {r2:?}");
        assert_eq!(jobs.list(None).len(), 1);
        let err_text = r1.err().or(r2.err()).expect("limit error");
        assert!(err_text.to_string().contains("limit: 1"));
    }

    fn delayed_hanging_job() -> JobStart {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = std::sync::Mutex::new(Some(tx));
        JobStart {
            kind: JobKind::Bash,
            label: "hang".into(),
            owner_session: None,
            run: Box::new(move || {
                std::thread::sleep(std::time::Duration::from_millis(80));
                JobHooks {
                    cancel: Box::new(move |_| {
                        if let Some(sender) = tx
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take()
                        {
                            let _ = sender.send(());
                        }
                    }),
                    done: Box::pin(async move {
                        let _ = rx.await;
                        JobOutcome {
                            status: JobStatus::Killed,
                            detail: None,
                            output: None,
                        }
                    }),
                    read_output: None,
                }
            }),
        }
    }
}
