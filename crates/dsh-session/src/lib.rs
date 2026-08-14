//! Append-only session log types for the Rust host.

mod catalog;
mod decode;
mod error;
mod event;
mod header;
mod ids;
mod message;

pub use catalog::{KNOWN_SESSION_EVENT_TYPES, is_known_session_event_type};
pub use decode::{decode_log_event, decode_log_event_for_session, refuse_foreign_format_version};
pub use error::{
    SessionError, SessionFormatError, session_format_version_refusal,
    unknown_required_event_refusal,
};
pub use event::{LeftoverEvent, LogEvent, SessionEvent, SurfaceOp};
pub use header::{SESSION_FORMAT_VERSION, SessionHeader, SessionOrigin};
pub use ids::{CallId, CallIdTag, MessageId, MessageIdTag, SessionId, SessionIdTag};
pub use message::{
    ApprovalPolicyData, AssistantChunkData, AssistantMessageData, ContentBlock, EpochHeader,
    FinishReason, InboxSplicedData, InboxTarget, LlmCallConfig, LlmCallConfigAdapterDefaults,
    LlmFailure, Message, MessageRole, MessageSource, PermissionPresetData, RequestContext,
    RequestHeaderData, RequestHeaderReason, SandboxModeData, SessionTitleData, StepBoundaryData,
    StreamChunk, TokenUsage, ToolCallData, ToolResultData, TurnEndData, TurnEndReason,
    TurnStartData,
};
