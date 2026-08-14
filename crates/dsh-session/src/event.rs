//! Closed first-party session event enum.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::{
    ApprovalPolicyData, AssistantChunkData, AssistantMessageData, InboxSplicedData, Message,
    PermissionPresetData, RequestContext, RequestHeaderData, SandboxModeData, SessionTitleData,
    StepBoundaryData, ToolCallData, ToolResultData, TurnEndData, TurnStartData,
};

/// How a surface-eligible event entered the ordered surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceOp {
    /// Appended to the tail (`"append"`).
    Append,
    /// Replaced an inclusive surface range (`{"op":"replace","start","end"}`).
    Replace {
        /// Inclusive start seq currently on the surface.
        start: u64,
        /// Inclusive end seq currently on the surface.
        end: u64,
    },
}

impl Serialize for SurfaceOp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Append => serializer.serialize_str("append"),
            Self::Replace { start, end } => {
                #[derive(Serialize)]
                struct ReplaceWire {
                    op: &'static str,
                    start: u64,
                    end: u64,
                }
                ReplaceWire {
                    op: "replace",
                    start: *start,
                    end: *end,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for SurfaceOp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if value.as_str() == Some("append") {
            return Ok(Self::Append);
        }
        let op = value.get("op").and_then(Value::as_str);
        let start = value.get("start").and_then(Value::as_u64);
        let end = value.get("end").and_then(Value::as_u64);
        if op == Some("replace") {
            if let (Some(start), Some(end)) = (start, end) {
                return Ok(Self::Replace { start, end });
            }
        }
        Err(serde::de::Error::custom("invalid surfaceOp"))
    }
}

/// One immutable first-party log entry. Unknown types are [`LogEvent::Leftover`], not variants here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionEvent {
    /// `agent-preset/selected`
    #[serde(rename = "agent-preset/selected")]
    AgentPresetSelected {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker for unknown-type readers of *this* event in a newer vocabulary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `agent/inbox/spliced`
    #[serde(rename = "agent/inbox/spliced")]
    AgentInboxSpliced {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Inbox splice.
        data: InboxSplicedData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `approval/asked`
    #[serde(rename = "approval/asked")]
    ApprovalAsked {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `approval/decided`
    #[serde(rename = "approval/decided")]
    ApprovalDecided {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `approval/policy`
    #[serde(rename = "approval/policy")]
    ApprovalPolicy {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Policy snapshot.
        data: ApprovalPolicyData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `assistant/chunk`
    #[serde(rename = "assistant/chunk")]
    AssistantChunk {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Stream chunk wrapper.
        data: AssistantChunkData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `assistant/message`
    #[serde(rename = "assistant/message")]
    AssistantMessage {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Assembled assistant message.
        data: AssistantMessageData,
        /// Surface placement.
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "surfaceOp")]
        surface_op: Option<SurfaceOp>,
        /// Cited source seqs.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceEventSeqs"
        )]
        source_event_seqs: Option<Vec<u64>>,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `command/done`
    #[serde(rename = "command/done")]
    CommandDone {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `command/run`
    #[serde(rename = "command/run")]
    CommandRun {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `compaction/end`
    #[serde(rename = "compaction/end")]
    CompactionEnd {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `compaction/prune`
    #[serde(rename = "compaction/prune")]
    CompactionPrune {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `compaction/start`
    #[serde(rename = "compaction/start")]
    CompactionStart {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `compaction/summary`
    #[serde(rename = "compaction/summary")]
    CompactionSummary {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `feedback/record`
    #[serde(rename = "feedback/record")]
    FeedbackRecord {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `goal/change`
    #[serde(rename = "goal/change")]
    GoalChange {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `hook/invoked`
    #[serde(rename = "hook/invoked")]
    HookInvoked {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `hook/result`
    #[serde(rename = "hook/result")]
    HookResult {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `llm/retry`
    #[serde(rename = "llm/retry")]
    LlmRetry {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `llm/retry-started`
    #[serde(rename = "llm/retry-started")]
    LlmRetryStarted {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `permission/preset`
    #[serde(rename = "permission/preset")]
    PermissionPreset {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Preset snapshot.
        data: PermissionPresetData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `plan/mode`
    #[serde(rename = "plan/mode")]
    PlanMode {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `request/context`
    #[serde(rename = "request/context")]
    RequestContextEvent {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Route metadata.
        data: RequestContext,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `request/header`
    #[serde(rename = "request/header")]
    RequestHeader {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Header snapshot.
        data: RequestHeaderData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `sandbox/mode`
    #[serde(rename = "sandbox/mode")]
    SandboxMode {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Mode snapshot.
        data: SandboxModeData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `schedule/change`
    #[serde(rename = "schedule/change")]
    ScheduleChange {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `session/end-seed`
    #[serde(rename = "session/end-seed")]
    SessionEndSeed {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Empty object.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `session/title`
    #[serde(rename = "session/title")]
    SessionTitle {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Title snapshot.
        data: SessionTitleData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `session/title-llm-request`
    #[serde(rename = "session/title-llm-request")]
    SessionTitleLlmRequest {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Auxiliary request record (fixture-shaped JSON).
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `step/end`
    #[serde(rename = "step/end")]
    StepEnd {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Step boundary.
        data: StepBoundaryData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `step/start`
    #[serde(rename = "step/start")]
    StepStart {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Step boundary.
        data: StepBoundaryData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `subagent/descriptor`
    #[serde(rename = "subagent/descriptor")]
    SubagentDescriptor {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `todo/write`
    #[serde(rename = "todo/write")]
    TodoWrite {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool-workflow/agent-end`
    #[serde(rename = "tool-workflow/agent-end")]
    ToolWorkflowAgentEnd {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool-workflow/agent-start`
    #[serde(rename = "tool-workflow/agent-start")]
    ToolWorkflowAgentStart {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool-workflow/run-end`
    #[serde(rename = "tool-workflow/run-end")]
    ToolWorkflowRunEnd {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool-workflow/run-start`
    #[serde(rename = "tool-workflow/run-start")]
    ToolWorkflowRunStart {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool/call`
    #[serde(rename = "tool/call")]
    ToolCall {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Tool request.
        data: ToolCallData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool/code-dispatch`
    #[serde(rename = "tool/code-dispatch")]
    ToolCodeDispatch {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool/code-dispatch-start`
    #[serde(rename = "tool/code-dispatch-start")]
    ToolCodeDispatchStart {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `tool/result`
    #[serde(rename = "tool/result")]
    ToolResult {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Tool result.
        data: ToolResultData,
        /// Surface placement.
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "surfaceOp")]
        surface_op: Option<SurfaceOp>,
        /// Cited source seqs.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceEventSeqs"
        )]
        source_event_seqs: Option<Vec<u64>>,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `turn/end`
    #[serde(rename = "turn/end")]
    TurnEnd {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Turn close.
        data: TurnEndData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `turn/start`
    #[serde(rename = "turn/start")]
    TurnStart {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Turn open.
        data: TurnStartData,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `user/message`
    #[serde(rename = "user/message")]
    UserMessage {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// User-role message (the event `data` *is* the message).
        data: Message,
        /// Surface placement.
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "surfaceOp")]
        surface_op: Option<SurfaceOp>,
        /// Cited source seqs.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "sourceEventSeqs"
        )]
        source_event_seqs: Option<Vec<u64>>,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
    /// `web/deepseek-search-llm-request`
    #[serde(rename = "web/deepseek-search-llm-request")]
    WebDeepseekSearchLlmRequest {
        /// Sequence number.
        seq: u64,
        /// Epoch milliseconds.
        time: i64,
        /// Log-only payload.
        data: Value,
        /// Skip marker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ignorable: Option<bool>,
    },
}

impl SessionEvent {
    /// Wire `type` string.
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::AgentPresetSelected { .. } => "agent-preset/selected",
            Self::AgentInboxSpliced { .. } => "agent/inbox/spliced",
            Self::ApprovalAsked { .. } => "approval/asked",
            Self::ApprovalDecided { .. } => "approval/decided",
            Self::ApprovalPolicy { .. } => "approval/policy",
            Self::AssistantChunk { .. } => "assistant/chunk",
            Self::AssistantMessage { .. } => "assistant/message",
            Self::CommandDone { .. } => "command/done",
            Self::CommandRun { .. } => "command/run",
            Self::CompactionEnd { .. } => "compaction/end",
            Self::CompactionPrune { .. } => "compaction/prune",
            Self::CompactionStart { .. } => "compaction/start",
            Self::CompactionSummary { .. } => "compaction/summary",
            Self::FeedbackRecord { .. } => "feedback/record",
            Self::GoalChange { .. } => "goal/change",
            Self::HookInvoked { .. } => "hook/invoked",
            Self::HookResult { .. } => "hook/result",
            Self::LlmRetry { .. } => "llm/retry",
            Self::LlmRetryStarted { .. } => "llm/retry-started",
            Self::PermissionPreset { .. } => "permission/preset",
            Self::PlanMode { .. } => "plan/mode",
            Self::RequestContextEvent { .. } => "request/context",
            Self::RequestHeader { .. } => "request/header",
            Self::SandboxMode { .. } => "sandbox/mode",
            Self::ScheduleChange { .. } => "schedule/change",
            Self::SessionEndSeed { .. } => "session/end-seed",
            Self::SessionTitle { .. } => "session/title",
            Self::SessionTitleLlmRequest { .. } => "session/title-llm-request",
            Self::StepEnd { .. } => "step/end",
            Self::StepStart { .. } => "step/start",
            Self::SubagentDescriptor { .. } => "subagent/descriptor",
            Self::TodoWrite { .. } => "todo/write",
            Self::ToolWorkflowAgentEnd { .. } => "tool-workflow/agent-end",
            Self::ToolWorkflowAgentStart { .. } => "tool-workflow/agent-start",
            Self::ToolWorkflowRunEnd { .. } => "tool-workflow/run-end",
            Self::ToolWorkflowRunStart { .. } => "tool-workflow/run-start",
            Self::ToolCall { .. } => "tool/call",
            Self::ToolCodeDispatch { .. } => "tool/code-dispatch",
            Self::ToolCodeDispatchStart { .. } => "tool/code-dispatch-start",
            Self::ToolResult { .. } => "tool/result",
            Self::TurnEnd { .. } => "turn/end",
            Self::TurnStart { .. } => "turn/start",
            Self::UserMessage { .. } => "user/message",
            Self::WebDeepseekSearchLlmRequest { .. } => "web/deepseek-search-llm-request",
        }
    }

    /// Monotonic sequence number.
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            Self::AgentPresetSelected { seq, .. }
            | Self::AgentInboxSpliced { seq, .. }
            | Self::ApprovalAsked { seq, .. }
            | Self::ApprovalDecided { seq, .. }
            | Self::ApprovalPolicy { seq, .. }
            | Self::AssistantChunk { seq, .. }
            | Self::AssistantMessage { seq, .. }
            | Self::CommandDone { seq, .. }
            | Self::CommandRun { seq, .. }
            | Self::CompactionEnd { seq, .. }
            | Self::CompactionPrune { seq, .. }
            | Self::CompactionStart { seq, .. }
            | Self::CompactionSummary { seq, .. }
            | Self::FeedbackRecord { seq, .. }
            | Self::GoalChange { seq, .. }
            | Self::HookInvoked { seq, .. }
            | Self::HookResult { seq, .. }
            | Self::LlmRetry { seq, .. }
            | Self::LlmRetryStarted { seq, .. }
            | Self::PermissionPreset { seq, .. }
            | Self::PlanMode { seq, .. }
            | Self::RequestContextEvent { seq, .. }
            | Self::RequestHeader { seq, .. }
            | Self::SandboxMode { seq, .. }
            | Self::ScheduleChange { seq, .. }
            | Self::SessionEndSeed { seq, .. }
            | Self::SessionTitle { seq, .. }
            | Self::SessionTitleLlmRequest { seq, .. }
            | Self::StepEnd { seq, .. }
            | Self::StepStart { seq, .. }
            | Self::SubagentDescriptor { seq, .. }
            | Self::TodoWrite { seq, .. }
            | Self::ToolWorkflowAgentEnd { seq, .. }
            | Self::ToolWorkflowAgentStart { seq, .. }
            | Self::ToolWorkflowRunEnd { seq, .. }
            | Self::ToolWorkflowRunStart { seq, .. }
            | Self::ToolCall { seq, .. }
            | Self::ToolCodeDispatch { seq, .. }
            | Self::ToolCodeDispatchStart { seq, .. }
            | Self::ToolResult { seq, .. }
            | Self::TurnEnd { seq, .. }
            | Self::TurnStart { seq, .. }
            | Self::UserMessage { seq, .. }
            | Self::WebDeepseekSearchLlmRequest { seq, .. } => *seq,
        }
    }

    /// Epoch milliseconds.
    #[must_use]
    pub fn time(&self) -> i64 {
        match self {
            Self::AgentPresetSelected { time, .. }
            | Self::AgentInboxSpliced { time, .. }
            | Self::ApprovalAsked { time, .. }
            | Self::ApprovalDecided { time, .. }
            | Self::ApprovalPolicy { time, .. }
            | Self::AssistantChunk { time, .. }
            | Self::AssistantMessage { time, .. }
            | Self::CommandDone { time, .. }
            | Self::CommandRun { time, .. }
            | Self::CompactionEnd { time, .. }
            | Self::CompactionPrune { time, .. }
            | Self::CompactionStart { time, .. }
            | Self::CompactionSummary { time, .. }
            | Self::FeedbackRecord { time, .. }
            | Self::GoalChange { time, .. }
            | Self::HookInvoked { time, .. }
            | Self::HookResult { time, .. }
            | Self::LlmRetry { time, .. }
            | Self::LlmRetryStarted { time, .. }
            | Self::PermissionPreset { time, .. }
            | Self::PlanMode { time, .. }
            | Self::RequestContextEvent { time, .. }
            | Self::RequestHeader { time, .. }
            | Self::SandboxMode { time, .. }
            | Self::ScheduleChange { time, .. }
            | Self::SessionEndSeed { time, .. }
            | Self::SessionTitle { time, .. }
            | Self::SessionTitleLlmRequest { time, .. }
            | Self::StepEnd { time, .. }
            | Self::StepStart { time, .. }
            | Self::SubagentDescriptor { time, .. }
            | Self::TodoWrite { time, .. }
            | Self::ToolWorkflowAgentEnd { time, .. }
            | Self::ToolWorkflowAgentStart { time, .. }
            | Self::ToolWorkflowRunEnd { time, .. }
            | Self::ToolWorkflowRunStart { time, .. }
            | Self::ToolCall { time, .. }
            | Self::ToolCodeDispatch { time, .. }
            | Self::ToolCodeDispatchStart { time, .. }
            | Self::ToolResult { time, .. }
            | Self::TurnEnd { time, .. }
            | Self::TurnStart { time, .. }
            | Self::UserMessage { time, .. }
            | Self::WebDeepseekSearchLlmRequest { time, .. } => *time,
        }
    }

    /// Surface operation when this event is surface-eligible and marked.
    #[must_use]
    pub fn surface_op(&self) -> Option<&SurfaceOp> {
        match self {
            Self::UserMessage { surface_op, .. }
            | Self::AssistantMessage { surface_op, .. }
            | Self::ToolResult { surface_op, .. } => surface_op.as_ref(),
            _ => None,
        }
    }

    /// Cited source-event seqs when present.
    #[must_use]
    pub fn source_event_seqs(&self) -> Option<&[u64]> {
        match self {
            Self::UserMessage {
                source_event_seqs, ..
            }
            | Self::AssistantMessage {
                source_event_seqs, ..
            }
            | Self::ToolResult {
                source_event_seqs, ..
            } => source_event_seqs.as_deref(),
            _ => None,
        }
    }

    /// Whether the envelope carries `ignorable: true`.
    #[must_use]
    pub fn ignorable(&self) -> bool {
        match self {
            Self::AgentPresetSelected { ignorable, .. }
            | Self::AgentInboxSpliced { ignorable, .. }
            | Self::ApprovalAsked { ignorable, .. }
            | Self::ApprovalDecided { ignorable, .. }
            | Self::ApprovalPolicy { ignorable, .. }
            | Self::AssistantChunk { ignorable, .. }
            | Self::AssistantMessage { ignorable, .. }
            | Self::CommandDone { ignorable, .. }
            | Self::CommandRun { ignorable, .. }
            | Self::CompactionEnd { ignorable, .. }
            | Self::CompactionPrune { ignorable, .. }
            | Self::CompactionStart { ignorable, .. }
            | Self::CompactionSummary { ignorable, .. }
            | Self::FeedbackRecord { ignorable, .. }
            | Self::GoalChange { ignorable, .. }
            | Self::HookInvoked { ignorable, .. }
            | Self::HookResult { ignorable, .. }
            | Self::LlmRetry { ignorable, .. }
            | Self::LlmRetryStarted { ignorable, .. }
            | Self::PermissionPreset { ignorable, .. }
            | Self::PlanMode { ignorable, .. }
            | Self::RequestContextEvent { ignorable, .. }
            | Self::RequestHeader { ignorable, .. }
            | Self::SandboxMode { ignorable, .. }
            | Self::ScheduleChange { ignorable, .. }
            | Self::SessionEndSeed { ignorable, .. }
            | Self::SessionTitle { ignorable, .. }
            | Self::SessionTitleLlmRequest { ignorable, .. }
            | Self::StepEnd { ignorable, .. }
            | Self::StepStart { ignorable, .. }
            | Self::SubagentDescriptor { ignorable, .. }
            | Self::TodoWrite { ignorable, .. }
            | Self::ToolWorkflowAgentEnd { ignorable, .. }
            | Self::ToolWorkflowAgentStart { ignorable, .. }
            | Self::ToolWorkflowRunEnd { ignorable, .. }
            | Self::ToolWorkflowRunStart { ignorable, .. }
            | Self::ToolCall { ignorable, .. }
            | Self::ToolCodeDispatch { ignorable, .. }
            | Self::ToolCodeDispatchStart { ignorable, .. }
            | Self::ToolResult { ignorable, .. }
            | Self::TurnEnd { ignorable, .. }
            | Self::TurnStart { ignorable, .. }
            | Self::UserMessage { ignorable, .. }
            | Self::WebDeepseekSearchLlmRequest { ignorable, .. } => *ignorable == Some(true),
        }
    }

    /// Whether this type may carry `surfaceOp`.
    #[must_use]
    pub fn is_surface_eligible_type(type_name: &str) -> bool {
        matches!(
            type_name,
            "user/message" | "assistant/message" | "tool/result"
        )
    }
}

/// Unknown leftover accepted only when `ignorable: true`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeftoverEvent {
    /// Unrecognized type string.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Sequence number.
    pub seq: u64,
    /// Epoch milliseconds.
    pub time: i64,
    /// Opaque payload.
    pub data: Value,
    /// Must be `true` for a reader to keep this event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignorable: Option<bool>,
    /// Ignored on leftovers; retained for byte-faithful re-encode.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "surfaceOp")]
    pub surface_op: Option<Value>,
    /// Ignored on leftovers; retained for byte-faithful re-encode.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "sourceEventSeqs"
    )]
    pub source_event_seqs: Option<Vec<u64>>,
}

/// One decoded log row: a first-party event or an ignorable leftover.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LogEvent {
    /// Closed first-party event.
    Known(SessionEvent),
    /// Unknown type marked ignorable.
    Leftover(LeftoverEvent),
}

impl LogEvent {
    /// Wire type string.
    #[must_use]
    pub fn event_type(&self) -> &str {
        match self {
            Self::Known(event) => event.event_type(),
            Self::Leftover(event) => event.type_name.as_str(),
        }
    }

    /// Sequence number.
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            Self::Known(event) => event.seq(),
            Self::Leftover(event) => event.seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionEvent, SurfaceOp};
    use crate::catalog::KNOWN_SESSION_EVENT_TYPES;
    use serde_json::json;

    #[test]
    fn every_catalog_type_is_a_serde_variant() {
        for tag in KNOWN_SESSION_EVENT_TYPES {
            let value = json!({ "type": tag, "seq": 0, "time": 0, "data": {} });
            match serde_json::from_value::<SessionEvent>(value) {
                Ok(_) => {}
                Err(error) => {
                    let message = error.to_string();
                    assert!(
                        !message.contains("unknown variant"),
                        "{tag} is missing from SessionEvent: {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn turn_start_round_trips() {
        let value = json!({"type":"turn/start","seq":4,"time":0,"data":{"turn":1}});
        let event: SessionEvent = serde_json::from_value(value.clone()).expect("decode");
        assert_eq!(event.event_type(), "turn/start");
        assert_eq!(event.seq(), 4);
        assert_eq!(serde_json::to_value(&event).expect("encode"), value);
    }

    #[test]
    fn user_message_carries_surface_append() {
        let value = json!({
            "type": "user/message",
            "seq": 7,
            "time": 0,
            "data": {
                "content": [{"type":"text","text":"hi"}],
                "source": {"kind":"user"},
                "role": "user",
                "id": "sess-1"
            },
            "surfaceOp": "append"
        });
        let event: SessionEvent = serde_json::from_value(value.clone()).expect("decode");
        assert_eq!(event.surface_op(), Some(&SurfaceOp::Append));
        assert_eq!(serde_json::to_value(&event).expect("encode"), value);
    }

    #[test]
    fn request_header_accepts_string_tools_token() {
        let value = json!({
            "type": "request/header",
            "seq": 10,
            "time": 0,
            "data": {
                "header": {
                    "config": {"provider":"cli-mock","model":"cli-mock","reasoningEffort":"high"},
                    "adapterDefaults": {"reasoningEffort": true},
                    "system": "{{system}}",
                    "tools": "{{tools}}"
                },
                "reason": "initial"
            }
        });
        let event: SessionEvent = serde_json::from_value(value.clone()).expect("decode");
        assert_eq!(event.event_type(), "request/header");
        assert_eq!(serde_json::to_value(&event).expect("encode"), value);
    }

    #[test]
    fn assistant_chunk_tool_call_delta_round_trips() {
        let value = json!({
            "type": "assistant/chunk",
            "seq": 14,
            "time": 0,
            "data": {
                "turn": 1,
                "step": 1,
                "chunk": {
                    "type": "tool-call-delta",
                    "index": 0,
                    "id": "cli-smoke-call",
                    "name": "bash",
                    "argumentsDelta": "{\"command\":\"printf x\"}"
                }
            }
        });
        let event: SessionEvent = serde_json::from_value(value.clone()).expect("decode");
        assert_eq!(serde_json::to_value(&event).expect("encode"), value);
    }
}
