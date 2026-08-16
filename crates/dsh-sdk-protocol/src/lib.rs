//! SDK JSON-RPC 2.0 wire types and NDJSON transport.

mod transport;
mod types;

pub use transport::{
    DecodedFrame, ERR_INTERNAL, ERR_METHOD_NOT_FOUND, JSONRPC_VERSION, JsonRpcId,
    JsonRpcLineTransport, JsonRpcResponseError, NotificationHandler, RequestHandler, decode_line,
    encode_error, encode_notification, encode_request, encode_result,
};
pub use types::{
    InitializeParams, InitializeResult, SDK_SERVER_NAME, SDK_SERVER_VERSION, SdkRunStatus,
    ServerInfo, SessionEventNotification, SessionPromptParams, SessionPromptResult, SessionStatus,
    SessionStatusNotification, ShutdownResult, SubagentFinishedNotification,
    SubagentStartedNotification, SubagentStopReason,
};
