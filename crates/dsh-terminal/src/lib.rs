//! Owner-scoped persistent PTY registry and in-memory snapshot backend.

mod id;
mod plugin;
mod service;
mod snapshot;

#[cfg(test)]
mod phase8_pty_exit;

pub use id::{TerminalSessionId, TerminalSessionIdTag};
pub use plugin::{register, register_snapshot_backend, register_terminal_plugins};
pub use service::{
    TerminalBackend, TerminalBackendSession, TerminalBackendSpawnFuture, TerminalBackendSpawnSpec,
    TerminalError, TerminalErrorCode, TerminalReadRequest, TerminalReadResult,
    TerminalSendOperation, TerminalSendRead, TerminalSendRequest, TerminalSendResult,
    TerminalSendShared, TerminalSessionService, TerminalSessionSnapshot, TerminalSessionStatus,
    TerminalSignal, TerminalSignalResult, TerminalSpawnRequest, TerminalSpawnResult,
    TerminalWaitReason,
};
pub use snapshot::SnapshotBackend;
