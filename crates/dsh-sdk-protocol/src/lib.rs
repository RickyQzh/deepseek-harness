//! SDK JSON-RPC 2.0 wire types and NDJSON transport.

mod types;

pub use types::{
    InitializeParams, InitializeResult, SDK_SERVER_NAME, SDK_SERVER_VERSION, SdkRunStatus,
    ServerInfo, SessionEventNotification, SessionPromptParams, SessionPromptResult, SessionStatus,
    SessionStatusNotification, ShutdownResult, SubagentFinishedNotification,
    SubagentStartedNotification, SubagentStopReason,
};
