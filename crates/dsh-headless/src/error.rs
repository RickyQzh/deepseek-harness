//! Headless runner failures.

/// Failure while driving the one-shot task.
#[derive(Debug, thiserror::Error)]
pub enum HeadlessError {
    /// Launcher did not provide `appExit`.
    #[error("headless-runner: the launcher must provide ctx.appExit before the tree mounts")]
    MissingAppExit,
    /// `headlessStartup.task` is missing or whitespace.
    #[error("error: a task is required, for example: dsh --profile headless \"run the tests\"")]
    MissingTask,
    /// Registry or loop failure.
    #[error(transparent)]
    Agent(#[from] dsh_agent::AgentError),
    /// Persist flush failure.
    #[error(transparent)]
    Persist(#[from] dsh_session_persist::PersistError),
    /// Kernel provide/inject failure.
    #[error(transparent)]
    Kernel(#[from] dsh_kernel::KernelError),
}
