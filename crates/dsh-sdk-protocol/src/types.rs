//! Named JSON-RPC request and notification payloads.

use serde::{Deserialize, Serialize};

use dsh_session::{ContentBlock, SessionEvent};

/// Wire-stable `serverInfo.name`.
pub const SDK_SERVER_NAME: &str = "deepseek-harness-sdk-runtime";
/// Wire-stable `serverInfo.version` for this host.
pub const SDK_SERVER_VERSION: &str = "0.0.1";

/// JSON-RPC `initialize` parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InitializeParams {
    cwd: String,
    provider: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "maxTokens")]
    max_tokens: Option<u64>,
}

impl InitializeParams {
    /// Handshake parameters without an output-token cap.
    #[must_use]
    pub fn new(
        cwd: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            cwd: cwd.into(),
            provider: provider.into(),
            model: model.into(),
            max_tokens: None,
        }
    }

    /// Set the optional output-token cap.
    ///
    /// The server (Task 55) must require a positive integer; this constructor does not validate.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u64) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Working directory recorded on SDK-created session headers.
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Provider route.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Model id.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Optional output-token cap.
    #[must_use]
    pub fn max_tokens(&self) -> Option<u64> {
        self.max_tokens
    }
}

/// Handshake `serverInfo` object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    name: String,
    version: String,
}

impl ServerInfo {
    /// Wire-stable harness SDK identity.
    #[must_use]
    pub fn harness_runtime() -> Self {
        Self {
            name: SDK_SERVER_NAME.into(),
            version: SDK_SERVER_VERSION.into(),
        }
    }

    /// `serverInfo.name`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `serverInfo.version`.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// JSON-RPC `initialize` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
}

impl InitializeResult {
    /// Handshake result for this host.
    #[must_use]
    pub fn harness_runtime() -> Self {
        Self {
            server_info: ServerInfo::harness_runtime(),
        }
    }

    /// Server identity object.
    #[must_use]
    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }
}

/// JSON-RPC `session/prompt` parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionPromptParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "contentBlocks")]
    content_blocks: Vec<ContentBlock>,
}

impl SessionPromptParams {
    /// One user turn on one SDK session.
    #[must_use]
    pub fn new(session_id: impl Into<String>, content_blocks: Vec<ContentBlock>) -> Self {
        Self {
            session_id: session_id.into(),
            content_blocks,
        }
    }

    /// SDK session id; an unknown id lazily creates the agent.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Prompt content blocks, sent verbatim as the user message.
    #[must_use]
    pub fn content_blocks(&self) -> &[ContentBlock] {
        &self.content_blocks
    }
}

/// JSON-RPC `session/prompt` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionPromptResult {
    #[serde(rename = "messageId")]
    message_id: String,
}

impl SessionPromptResult {
    /// Durable enqueue receipt.
    #[must_use]
    pub fn new(message_id: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
        }
    }

    /// Identity of the queued user message.
    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }
}

/// Empty JSON-RPC `shutdown` result (`{}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShutdownResult {}

/// Notification payload for `session.event`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEventNotification {
    #[serde(rename = "sessionId")]
    session_id: String,
    event: SessionEvent,
}

impl SessionEventNotification {
    /// One session-log event, streamed as it is recorded.
    #[must_use]
    pub fn new(session_id: impl Into<String>, event: SessionEvent) -> Self {
        Self {
            session_id: session_id.into(),
            event,
        }
    }

    /// Session the event belongs to.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Full session-log event envelope.
    #[must_use]
    pub fn event(&self) -> &SessionEvent {
        &self.event
    }
}

/// Whole-agent lifecycle state for one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    /// No running turn driver.
    Idle,
    /// A turn driver holds the agent.
    Running,
}

/// Notification payload for `session.status`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionStatusNotification {
    #[serde(rename = "sessionId")]
    session_id: String,
    status: SessionStatus,
}

impl SessionStatusNotification {
    /// Status transition for one session.
    #[must_use]
    pub fn new(session_id: impl Into<String>, status: SessionStatus) -> Self {
        Self {
            session_id: session_id.into(),
            status,
        }
    }

    /// Session whose live agent changed status.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Whole-agent state after the transition.
    #[must_use]
    pub fn status(&self) -> SessionStatus {
        self.status
    }
}

/// Notification payload for `subagent.started`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentStartedNotification {
    #[serde(rename = "parentSessionId")]
    parent_session_id: String,
    #[serde(rename = "childSessionId")]
    child_session_id: String,
}

impl SubagentStartedNotification {
    /// An in-runtime child session was created.
    #[must_use]
    pub fn new(parent_session_id: impl Into<String>, child_session_id: impl Into<String>) -> Self {
        Self {
            parent_session_id: parent_session_id.into(),
            child_session_id: child_session_id.into(),
        }
    }

    /// Delegating session.
    #[must_use]
    pub fn parent_session_id(&self) -> &str {
        &self.parent_session_id
    }

    /// New child session.
    #[must_use]
    pub fn child_session_id(&self) -> &str {
        &self.child_session_id
    }
}

/// Deployment-mapped SDK outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdkRunStatus {
    /// Accepted result.
    Ok,
    /// Infrastructure or model failure.
    Error,
}

/// Provider-reported subagent stop reason. Phase 5 servers may never emit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubagentStopReason {
    /// Child finished normally.
    Completed,
    /// Cancelled or disposed.
    Aborted,
    /// Model or transport failure.
    Error,
    /// Token ceiling.
    #[serde(rename = "max-tokens")]
    MaxTokens,
    /// Child declined the task.
    Refusal,
}

/// Notification payload for `subagent.finished`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentFinishedNotification {
    provider: String,
    #[serde(rename = "agentId")]
    agent_id: String,
    #[serde(rename = "parentSessionId")]
    parent_session_id: String,
    #[serde(rename = "childSessionId")]
    child_session_id: String,
    status: SdkRunStatus,
    #[serde(rename = "stopReason")]
    stop_reason: SubagentStopReason,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "lastAssistantMessage"
    )]
    last_assistant_message: Option<Vec<ContentBlock>>,
}

impl SubagentFinishedNotification {
    /// In-process subagent run ended. Phase 5 may never construct this value on the wire.
    #[must_use]
    pub fn new(
        provider: impl Into<String>,
        agent_id: impl Into<String>,
        parent_session_id: impl Into<String>,
        child_session_id: impl Into<String>,
        status: SdkRunStatus,
        stop_reason: SubagentStopReason,
    ) -> Self {
        Self {
            provider: provider.into(),
            agent_id: agent_id.into(),
            parent_session_id: parent_session_id.into(),
            child_session_id: child_session_id.into(),
            status,
            stop_reason,
            last_assistant_message: None,
        }
    }

    /// Attach the child's selected assistant output.
    #[must_use]
    pub fn with_last_assistant_message(mut self, blocks: Vec<ContentBlock>) -> Self {
        self.last_assistant_message = Some(blocks);
        self
    }

    /// Subagent provider name.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Child agent id.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Delegating session.
    #[must_use]
    pub fn parent_session_id(&self) -> &str {
        &self.parent_session_id
    }

    /// Child session.
    #[must_use]
    pub fn child_session_id(&self) -> &str {
        &self.child_session_id
    }

    /// Deployment-mapped run outcome.
    #[must_use]
    pub fn status(&self) -> SdkRunStatus {
        self.status
    }

    /// Provider-reported stop reason.
    #[must_use]
    pub fn stop_reason(&self) -> SubagentStopReason {
        self.stop_reason
    }

    /// Child assistant output, when present.
    #[must_use]
    pub fn last_assistant_message(&self) -> Option<&[ContentBlock]> {
        self.last_assistant_message.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InitializeParams, InitializeResult, SDK_SERVER_NAME, SDK_SERVER_VERSION, SdkRunStatus,
        ServerInfo, SessionEventNotification, SessionPromptParams, SessionPromptResult,
        SessionStatus, SessionStatusNotification, ShutdownResult, SubagentFinishedNotification,
        SubagentStartedNotification, SubagentStopReason,
    };
    use dsh_session::{
        ContentBlock, SESSION_FORMAT_VERSION, SessionEvent, SessionHeader, SessionId, TurnStartData,
    };

    #[test]
    fn server_info_name_is_the_wire_stable_runtime_id() {
        let result = InitializeResult::harness_runtime();
        assert_eq!(result.server_info().name(), SDK_SERVER_NAME);
        assert_eq!(result.server_info().name(), "deepseek-harness-sdk-runtime");
        assert_eq!(result.server_info().version(), SDK_SERVER_VERSION);
        assert_eq!(result.server_info().version(), "0.0.1");
        let value = serde_json::to_value(&result).expect("serialize");
        assert_eq!(
            value["serverInfo"]["name"],
            serde_json::json!("deepseek-harness-sdk-runtime")
        );
        let back: InitializeResult = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.server_info().name(), "deepseek-harness-sdk-runtime");
    }

    #[test]
    fn initialize_params_round_trip_optional_max_tokens() {
        let params = InitializeParams::new("/work", "deepseek-official", "deepseek-v4-flash")
            .with_max_tokens(4096);
        let value = serde_json::to_value(&params).expect("serialize");
        assert_eq!(
            value,
            serde_json::json!({
                "cwd": "/work",
                "provider": "deepseek-official",
                "model": "deepseek-v4-flash",
                "maxTokens": 4096
            })
        );
        let back: InitializeParams = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.cwd(), "/work");
        assert_eq!(back.provider(), "deepseek-official");
        assert_eq!(back.model(), "deepseek-v4-flash");
        assert_eq!(back.max_tokens(), Some(4096));
        let omitted = InitializeParams::new("/work", "mock", "mock");
        let value = serde_json::to_value(&omitted).expect("serialize");
        assert!(value.get("maxTokens").is_none());
    }

    #[test]
    fn session_prompt_round_trips_content_blocks() {
        let params = SessionPromptParams::new(
            "sdk-snapshot-text",
            vec![ContentBlock::Text {
                text: "hello".into(),
            }],
        );
        let value = serde_json::to_value(&params).expect("serialize");
        assert_eq!(value["sessionId"], serde_json::json!("sdk-snapshot-text"));
        assert_eq!(value["contentBlocks"][0]["type"], serde_json::json!("text"));
        let back: SessionPromptParams = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.session_id(), "sdk-snapshot-text");
        assert_eq!(back.content_blocks().len(), 1);
        let result = SessionPromptResult::new("msg-1");
        let value = serde_json::to_value(&result).expect("serialize");
        assert_eq!(value, serde_json::json!({"messageId": "msg-1"}));
    }

    #[test]
    fn shutdown_result_is_empty_object() {
        let value = serde_json::to_value(&ShutdownResult {}).expect("serialize");
        assert_eq!(value, serde_json::json!({}));
    }

    #[test]
    fn session_event_notification_embeds_a_full_envelope() {
        let event = SessionEvent::TurnStart {
            seq: 1,
            time: 0,
            data: TurnStartData { turn: 1 },
            ignorable: None,
        };
        let n = SessionEventNotification::new("s1", event.clone());
        let value = serde_json::to_value(&n).expect("serialize");
        assert_eq!(value["sessionId"], serde_json::json!("s1"));
        assert_eq!(value["event"]["type"], serde_json::json!("turn/start"));
        assert_eq!(value["event"]["seq"], serde_json::json!(1));
        let back: SessionEventNotification = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.session_id(), "s1");
        assert_eq!(back.event(), &event);
        let _header = SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("s1"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        };
    }

    #[test]
    fn session_status_round_trips_idle_and_running() {
        for status in [SessionStatus::Idle, SessionStatus::Running] {
            let n = SessionStatusNotification::new("s1", status);
            let value = serde_json::to_value(&n).expect("serialize");
            let expected = match status {
                SessionStatus::Idle => "idle",
                SessionStatus::Running => "running",
            };
            assert_eq!(value["status"], serde_json::json!(expected));
            let back: SessionStatusNotification =
                serde_json::from_value(value).expect("deserialize");
            assert_eq!(back.status(), status);
        }
    }

    #[test]
    fn subagent_notifications_round_trip() {
        let started = SubagentStartedNotification::new("parent", "child");
        let value = serde_json::to_value(&started).expect("serialize");
        assert_eq!(value["parentSessionId"], serde_json::json!("parent"));
        assert_eq!(value["childSessionId"], serde_json::json!("child"));
        let finished = SubagentFinishedNotification::new(
            "in-process",
            "child",
            "parent",
            "child",
            SdkRunStatus::Ok,
            SubagentStopReason::Completed,
        )
        .with_last_assistant_message(vec![ContentBlock::Text {
            text: "done".into(),
        }]);
        let value = serde_json::to_value(&finished).expect("serialize");
        assert_eq!(value["stopReason"], serde_json::json!("completed"));
        assert_eq!(value["status"], serde_json::json!("ok"));
        assert_eq!(
            value["lastAssistantMessage"][0]["text"],
            serde_json::json!("done")
        );
        let back: SubagentFinishedNotification =
            serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.stop_reason(), SubagentStopReason::Completed);
        assert_eq!(back.status(), SdkRunStatus::Ok);
        assert!(back.last_assistant_message().is_some());
    }

    #[test]
    fn server_info_struct_round_trips() {
        let info = ServerInfo::harness_runtime();
        let value = serde_json::to_value(&info).expect("serialize");
        assert_eq!(
            value["name"],
            serde_json::json!("deepseek-harness-sdk-runtime")
        );
        let back: ServerInfo = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.name(), SDK_SERVER_NAME);
    }
}
