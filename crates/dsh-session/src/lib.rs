//! Append-only session log types for the Rust host.

mod catalog;
mod event;
mod header;
mod ids;
mod message;

pub use catalog::{KNOWN_SESSION_EVENT_TYPES, is_known_session_event_type};
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
