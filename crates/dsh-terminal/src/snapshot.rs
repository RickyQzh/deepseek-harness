//! In-memory PTY backend for named ACP `pty-tools` snapshots.

use std::future;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::{
    TerminalBackend, TerminalBackendSession, TerminalBackendSpawnFuture, TerminalBackendSpawnSpec,
    TerminalError, TerminalReadRequest, TerminalReadResult, TerminalSendOperation,
    TerminalSendRead, TerminalSendRequest, TerminalSendResult, TerminalSessionStatus,
    TerminalSignal, TerminalSignalResult, TerminalWaitReason,
};

const MOTD: &str = "dsh> ";

/// Deterministic `shell` backend: MOTD `dsh> `, send `{text}\nPTY_OK\ndsh> `.
pub struct SnapshotBackend;

struct SnapshotSession {
    status: Mutex<TerminalSessionStatus>,
    scrollback: Mutex<String>,
}

impl SnapshotSession {
    fn new() -> Self {
        Self {
            status: Mutex::new(TerminalSessionStatus::Running),
            scrollback: Mutex::new(MOTD.to_string()),
        }
    }

    fn lock_status(&self) -> std::sync::MutexGuard<'_, TerminalSessionStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_scrollback(&self) -> std::sync::MutexGuard<'_, String> {
        self.scrollback
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl TerminalBackend for SnapshotBackend {
    fn spawn(&self, _spec: TerminalBackendSpawnSpec) -> TerminalBackendSpawnFuture {
        Box::pin(std::future::ready(Ok(
            Box::new(SnapshotSession::new()) as Box<dyn TerminalBackendSession>
        )))
    }
}

impl TerminalBackendSession for SnapshotSession {
    fn motd(&self) -> &str {
        MOTD
    }

    fn pid(&self) -> Option<i32> {
        None
    }

    fn start_send(&self, request: TerminalSendRequest) -> TerminalSendOperation {
        let viewport = format!("{}\nPTY_OK\ndsh> ", request.text());
        self.lock_scrollback().push_str(&viewport);
        let status = self.lock_status().clone();
        let result = TerminalSendResult::new(
            viewport.clone(),
            TerminalWaitReason::StdinRead,
            status,
            false,
        );
        let consumed = Arc::new(AtomicBool::new(false));
        let delta = viewport;
        TerminalSendOperation::new(
            Box::pin(future::ready(result)),
            Box::new(move || {
                if consumed.swap(true, Ordering::SeqCst) {
                    TerminalSendRead::new(String::new(), false)
                } else {
                    TerminalSendRead::new(delta.clone(), false)
                }
            }),
            Box::new(|| false),
        )
    }

    fn read(&self, request: TerminalReadRequest) -> TerminalReadResult {
        let scrollback = self.lock_scrollback();
        let lines: Vec<&str> = scrollback.split('\n').collect();
        let offset = request.offset().unwrap_or(0);
        let count = request.count().unwrap_or(500);
        let end = lines.len().saturating_sub(offset);
        let start = end.saturating_sub(count);
        let text = lines[start..end].join("\n");
        let line_end = offset.saturating_add(text.split('\n').count());
        TerminalReadResult::new(text, lines.len(), offset, line_end, false)
    }

    fn signal(
        &self,
        _signal: TerminalSignal,
    ) -> Pin<Box<dyn Future<Output = TerminalSignalResult> + Send>> {
        Box::pin(std::future::ready(TerminalSignalResult::new(true, 1)))
    }

    fn status(&self) -> TerminalSessionStatus {
        self.lock_status().clone()
    }

    fn close(
        &self,
        _reason: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TerminalError>> + Send>> {
        *self.lock_status() = TerminalSessionStatus::Exited {
            exit_code: Some(0),
            signal: None,
        };
        Box::pin(std::future::ready(Ok(())))
    }
}
