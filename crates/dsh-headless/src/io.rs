//! Process-facing IO and launcher services.

use std::io::Write;
use std::sync::{Arc, Mutex};

/// Snapshot of inner argv after launcher flags (`--profile`, `--patch`).
pub struct CmdlineArgs {
    args: Vec<String>,
}

impl CmdlineArgs {
    /// Inner argv words.
    #[must_use]
    pub fn new(args: Vec<String>) -> Self {
        Self { args }
    }

    /// The snapshot.
    #[must_use]
    pub fn get(&self) -> &[String] {
        &self.args
    }
}

/// Task text published by `headless-startup`.
pub struct HeadlessStartup {
    task: String,
    resume_session_id: Option<String>,
}

impl HeadlessStartup {
    /// Non-empty task text, with no session resume.
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            resume_session_id: None,
        }
    }

    /// Record optional YAML `resumeSessionId`. Empty strings are treated as absent.
    #[must_use]
    pub fn with_resume_session_id(mut self, id: Option<String>) -> Self {
        self.resume_session_id = id.filter(|value| !value.is_empty());
        self
    }

    /// The one-shot prompt.
    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }

    /// Session id to load and resume, when YAML `resumeSessionId` was set.
    #[must_use]
    pub fn resume_session_id(&self) -> Option<&str> {
        self.resume_session_id.as_deref()
    }
}

/// One-shot process exit request. The second `exit` is ignored.
pub struct AppExit {
    tx: Mutex<Option<tokio::sync::oneshot::Sender<i32>>>,
}

impl AppExit {
    /// Sender stored as the `appExit` service; receiver owned by the launcher.
    #[must_use]
    pub fn pair() -> (Self, tokio::sync::oneshot::Receiver<i32>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (
            Self {
                tx: Mutex::new(Some(tx)),
            },
            rx,
        )
    }

    /// Request process exit with `code` after the tree can dispose.
    pub fn exit(&self, code: i32) {
        if let Some(tx) = self.tx.lock().expect("appExit").take() {
            let _ = tx.send(code);
        }
    }
}

#[derive(Clone)]
enum IoTarget {
    Stdout,
    Stderr,
    Buffer(Arc<Mutex<String>>),
}

/// stdout/stderr used by the runner. Tests install a capture via `headlessIo`.
#[derive(Clone)]
pub struct HeadlessIo {
    stdout: IoTarget,
    stderr: IoTarget,
}

impl HeadlessIo {
    /// Real process streams.
    #[must_use]
    pub fn stdio() -> Self {
        Self {
            stdout: IoTarget::Stdout,
            stderr: IoTarget::Stderr,
        }
    }

    /// In-memory capture.
    #[must_use]
    pub fn capture() -> Self {
        Self {
            stdout: IoTarget::Buffer(Arc::new(Mutex::new(String::new()))),
            stderr: IoTarget::Buffer(Arc::new(Mutex::new(String::new()))),
        }
    }

    /// Write to the configured stdout.
    pub fn write_stdout(&self, chunk: &str) {
        write_target(&self.stdout, chunk);
    }

    /// Write to the configured stderr.
    pub fn write_stderr(&self, chunk: &str) {
        write_target(&self.stderr, chunk);
    }

    /// Captured stdout, or empty for stdio.
    #[must_use]
    pub fn stdout_text(&self) -> String {
        read_target(&self.stdout)
    }

    /// Captured stderr, or empty for stdio.
    #[must_use]
    pub fn stderr_text(&self) -> String {
        read_target(&self.stderr)
    }
}

fn write_target(target: &IoTarget, chunk: &str) {
    match target {
        IoTarget::Stdout => {
            let mut out = std::io::stdout();
            let _ = out.write_all(chunk.as_bytes());
            let _ = out.flush();
        }
        IoTarget::Stderr => {
            let mut err = std::io::stderr();
            let _ = err.write_all(chunk.as_bytes());
            let _ = err.flush();
        }
        IoTarget::Buffer(buf) => buf.lock().expect("headlessIo").push_str(chunk),
    }
}

fn read_target(target: &IoTarget) -> String {
    match target {
        IoTarget::Stdout | IoTarget::Stderr => String::new(),
        IoTarget::Buffer(buf) => buf.lock().expect("headlessIo").clone(),
    }
}
