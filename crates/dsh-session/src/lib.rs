//! Append-only session log types for the Rust host.

mod catalog;
mod chunk_rows;
mod decode;
mod error;
mod event;
mod header;
mod ids;
mod message;
mod repair;
mod request_header;
mod session;
mod surface;

pub use catalog::{KNOWN_SESSION_EVENT_TYPES, is_known_session_event_type};
pub use chunk_rows::{
    ChunkRow, MIN_CHUNK_RUN, StorageRecord, TextRunData, ToolCallRunData, decode_storage_record,
    pack_chunk_runs,
};
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
    RequestHeaderData, RequestHeaderReason, SandboxModeData, SessionTitleData, SkillCatalogEntry,
    StepBoundaryData, StreamChunk, TokenUsage, ToolCallData, ToolResultData, ToolResultError,
    TurnEndData, TurnEndReason, TurnStartData,
};
pub use repair::{TOOL_NOT_STARTED, TOOL_OUTCOME_UNKNOWN, interrupted_turn_closers};
pub use request_header::{canonical_header, fold_request_header, header_equals};
pub use session::{AppendSink, Session};
pub use surface::{SurfaceFoldReplacement, SurfaceFoldResult, derive_event_message, fold_surface};
