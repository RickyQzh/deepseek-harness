//! `ApprovalService`, policy fold, and the `approval/request` waterfall.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

use dsh_kernel::Context;
use dsh_session::{
    ApprovalPolicyData, CallId, LogEvent, Session, SessionError, SessionEvent, SessionId,
};
use dsh_tools::{ApprovalOutcome, Approver, ToolExecution};

/// Kernel waterfall name. Payload is [`ApprovalQuestion`]; default outcome is [`ApprovalOutcome::Unavailable`].
pub const EVENT_APPROVAL_REQUEST: &str = "approval/request";

static NEXT_APPROVAL_ID: AtomicU64 = AtomicU64::new(1);

/// Per-session policy applied before waterfall answerers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalPolicy {
    /// Delegate to composed answerers; absence fails closed as [`ApprovalOutcome::Unavailable`].
    Ask,
    /// Reject every ask without consulting answerers.
    Never,
}

impl ApprovalPolicy {
    /// Session-log and YAML string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(Self::Ask),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// Failures from [`ApprovalService::request`].
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    /// No open `turn/start` encloses the audit pair.
    #[error(
        "approval.request() outside an open turn: the approval/asked + approval/decided audit pair must be turn-enclosed"
    )]
    Idle,
    /// Session rejected an audit append.
    #[error(transparent)]
    Session(#[from] SessionError),
}

/// Readonly permission question for one tool call.
pub struct ApprovalRequest {
    tool_name: String,
    call_id: Option<CallId>,
    reason: Option<String>,
}

impl ApprovalRequest {
    /// Ask about `tool_name` with no call id or reason.
    #[must_use]
    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            call_id: None,
            reason: None,
        }
    }

    /// Attach the tool-call id already presented to the model.
    #[must_use]
    pub fn with_call_id(mut self, id: CallId) -> Self {
        self.call_id = Some(id);
        self
    }

    /// Attach the asker's human-readable explanation.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Tool the question is about.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Exact tool call being decided, when the asker had one.
    #[must_use]
    pub fn call_id(&self) -> Option<&CallId> {
        self.call_id.as_ref()
    }

    /// Asker's explanation, when provided.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// Waterfall payload for [`EVENT_APPROVAL_REQUEST`]: session identity plus the current outcome.
#[derive(Clone)]
pub struct ApprovalQuestion {
    session_id: SessionId,
    call_id: Option<CallId>,
    tool_name: String,
    outcome: ApprovalOutcome,
}

impl ApprovalQuestion {
    /// Start a question with outcome [`ApprovalOutcome::Unavailable`].
    #[must_use]
    pub fn new(session_id: SessionId, tool_name: impl Into<String>) -> Self {
        Self {
            session_id,
            call_id: None,
            tool_name: tool_name.into(),
            outcome: ApprovalOutcome::Unavailable,
        }
    }

    /// Attach the tool-call id already presented to the model.
    #[must_use]
    pub fn with_call_id(mut self, id: CallId) -> Self {
        self.call_id = Some(id);
        self
    }

    /// Set the answerer outcome carried through the waterfall.
    #[must_use]
    pub fn with_outcome(mut self, outcome: ApprovalOutcome) -> Self {
        self.outcome = outcome;
        self
    }

    /// Session this question belongs to.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Exact tool call being decided, when the asker had one.
    #[must_use]
    pub fn call_id(&self) -> Option<&CallId> {
        self.call_id.as_ref()
    }

    /// Tool the question is about.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Current waterfall outcome; default is [`ApprovalOutcome::Unavailable`].
    #[must_use]
    pub fn outcome(&self) -> ApprovalOutcome {
        self.outcome
    }
}

/// Applies session policy, dispatches [`EVENT_APPROVAL_REQUEST`], and logs the audit pair.
#[derive(Clone)]
pub struct ApprovalService {
    default_policy: ApprovalPolicy,
    ctx: Context,
}

impl ApprovalService {
    /// Store `ctx` for waterfall dispatch and `default_policy` when the log has no override.
    #[must_use]
    pub fn new(ctx: Context, default_policy: ApprovalPolicy) -> Self {
        Self {
            default_policy,
            ctx,
        }
    }

    /// Last `approval/policy` event, or the plugin default.
    #[must_use]
    pub fn effective_policy(&self, events: &[LogEvent]) -> ApprovalPolicy {
        effective_approval_policy(events).unwrap_or(self.default_policy)
    }

    /// Append `approval/asked`, resolve one outcome, append `approval/decided`.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::Idle`] when `events` has no open turn (no appends).
    /// [`ApprovalError::Session`] when either audit append is rejected.
    pub async fn request(
        &self,
        session: &mut Session,
        req: ApprovalRequest,
    ) -> Result<ApprovalOutcome, ApprovalError> {
        if !has_open_turn(session.events()) {
            return Err(ApprovalError::Idle);
        }
        let id = next_approval_id();
        let seq = session.events().len() as u64;
        session.append(SessionEvent::ApprovalAsked {
            seq,
            time: seq as i64,
            data: asked_data(&id, &req),
            ignorable: None,
        })?;
        let outcome = if self.effective_policy(session.events()) == ApprovalPolicy::Never {
            ApprovalOutcome::Rejected
        } else {
            let mut question = ApprovalQuestion::new(session.id().clone(), req.tool_name());
            if let Some(call_id) = req.call_id() {
                question = question.with_call_id(call_id.clone());
            }
            self.ctx
                .waterfall(EVENT_APPROVAL_REQUEST, question)
                .await
                .outcome()
        };
        let seq = session.events().len() as u64;
        session.append(SessionEvent::ApprovalDecided {
            seq,
            time: seq as i64,
            data: serde_json::json!({
                "id": id,
                "outcome": outcome.as_str(),
            }),
            ignorable: None,
        })?;
        Ok(outcome)
    }
}

impl Approver for ApprovalService {
    fn decide<'a>(
        &'a self,
        session: &'a mut Session,
        exec: &'a ToolExecution,
        reason: Option<String>,
    ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send + 'a>> {
        let mut req = ApprovalRequest::new(exec.name.clone()).with_call_id(exec.call_id.clone());
        if let Some(reason) = reason {
            req = req.with_reason(reason);
        }
        Box::pin(async move {
            match self.request(session, req).await {
                Ok(outcome) => outcome,
                Err(ApprovalError::Idle | ApprovalError::Session(_)) => {
                    ApprovalOutcome::Unavailable
                }
            }
        })
    }
}

/// Append `approval/policy` with `policy`. Invalid values cannot be constructed.
///
/// # Errors
///
/// [`SessionError`] when the session rejects the append.
pub fn set_approval_policy(
    session: &mut Session,
    policy: ApprovalPolicy,
) -> Result<(), SessionError> {
    let seq = session.events().len() as u64;
    session.append(SessionEvent::ApprovalPolicy {
        seq,
        time: seq as i64,
        data: ApprovalPolicyData {
            policy: policy.as_str().to_string(),
            source: None,
        },
        ignorable: None,
    })?;
    Ok(())
}

/// Last `approval/policy` event, if it names `ask` or `never`.
#[must_use]
pub fn effective_approval_policy(events: &[LogEvent]) -> Option<ApprovalPolicy> {
    for event in events.iter().rev() {
        if let LogEvent::Known(SessionEvent::ApprovalPolicy { data, .. }) = event {
            return ApprovalPolicy::parse(&data.policy);
        }
    }
    None
}

/// Whether a `turn/start` is still unmatched by a later `turn/end`.
#[must_use]
pub fn has_open_turn(events: &[LogEvent]) -> bool {
    for event in events.iter().rev() {
        match event {
            LogEvent::Known(SessionEvent::TurnStart { .. }) => return true,
            LogEvent::Known(SessionEvent::TurnEnd { .. }) => return false,
            _ => {}
        }
    }
    false
}

fn next_approval_id() -> String {
    format!(
        "approval-{}",
        NEXT_APPROVAL_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn asked_data(id: &str, req: &ApprovalRequest) -> serde_json::Value {
    let mut data = serde_json::Map::new();
    data.insert("id".into(), serde_json::Value::String(id.to_string()));
    data.insert(
        "toolName".into(),
        serde_json::Value::String(req.tool_name.clone()),
    );
    if let Some(call_id) = &req.call_id {
        data.insert(
            "callId".into(),
            serde_json::Value::String(call_id.as_str().to_string()),
        );
    }
    if let Some(reason) = &req.reason {
        data.insert("reason".into(), serde_json::Value::String(reason.clone()));
    }
    serde_json::Value::Object(data)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{
        ApprovalError, ApprovalOutcome, ApprovalPolicy, ApprovalQuestion, ApprovalRequest,
        ApprovalService, EVENT_APPROVAL_REQUEST, set_approval_policy,
    };
    use dsh_kernel::Context;
    use dsh_session::{
        CallId, SESSION_FORMAT_VERSION, Session, SessionEvent, SessionHeader, SessionId,
        TurnStartData,
    };

    fn empty_header() -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("approval-test"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    fn open_turn_session() -> Session {
        let mut session = Session::new(empty_header());
        session
            .append(SessionEvent::TurnStart {
                seq: 0,
                time: 0,
                data: TurnStartData { turn: 1 },
                ignorable: None,
            })
            .unwrap();
        session
    }

    fn count_type(session: &Session, ty: &str) -> usize {
        session
            .events()
            .iter()
            .filter(|event| event.event_type() == ty)
            .count()
    }

    #[tokio::test]
    async fn request_without_answerer_is_unavailable_and_logs_the_pair() {
        let ctx = Context::new();
        let svc = ApprovalService::new(ctx, ApprovalPolicy::Ask);
        let mut session = open_turn_session();
        let out = svc
            .request(&mut session, ApprovalRequest::new("bash"))
            .await
            .unwrap();
        assert!(matches!(out, ApprovalOutcome::Unavailable));
        assert_eq!(count_type(&session, "approval/asked"), 1);
        assert_eq!(count_type(&session, "approval/decided"), 1);
    }

    #[tokio::test]
    async fn never_policy_rejects_before_answerer() {
        let ctx = Context::new();
        ctx.on_waterfall::<ApprovalQuestion, _, _>(EVENT_APPROVAL_REQUEST, |question, _n| async {
            question.with_outcome(ApprovalOutcome::AllowedOnce)
        })
        .unwrap();
        let svc = ApprovalService::new(ctx, ApprovalPolicy::Never);
        let mut session = open_turn_session();
        set_approval_policy(&mut session, ApprovalPolicy::Never).unwrap();
        let out = svc
            .request(&mut session, ApprovalRequest::new("bash"))
            .await
            .unwrap();
        assert!(matches!(out, ApprovalOutcome::Rejected));
    }

    #[tokio::test]
    async fn idle_request_does_not_append_audit() {
        let ctx = Context::new();
        let svc = ApprovalService::new(ctx, ApprovalPolicy::Ask);
        let mut session = Session::new(empty_header());
        let err = svc
            .request(&mut session, ApprovalRequest::new("bash"))
            .await
            .unwrap_err();
        assert!(matches!(err, ApprovalError::Idle));
        assert_eq!(session.events().len(), 0);
    }

    #[tokio::test]
    async fn auto_approve_grants_allowed_once() {
        let ctx = Context::new();
        ctx.on_waterfall::<ApprovalQuestion, _, _>(EVENT_APPROVAL_REQUEST, |question, _n| async {
            question.with_outcome(ApprovalOutcome::AllowedOnce)
        })
        .unwrap();
        let svc = ApprovalService::new(ctx, ApprovalPolicy::Ask);
        let mut session = open_turn_session();
        let out = svc
            .request(&mut session, ApprovalRequest::new("bash"))
            .await
            .unwrap();
        assert!(matches!(out, ApprovalOutcome::AllowedOnce));
    }

    #[tokio::test]
    async fn waterfall_question_carries_session_and_call_id() {
        let ctx = Context::new();
        let seen = Arc::new(Mutex::new(None::<(SessionId, Option<CallId>)>));
        let seen_listener = Arc::clone(&seen);
        ctx.on_waterfall::<ApprovalQuestion, _, _>(
            EVENT_APPROVAL_REQUEST,
            move |question, _next| {
                let seen_listener = Arc::clone(&seen_listener);
                async move {
                    *seen_listener.lock().expect("seen") =
                        Some((question.session_id().clone(), question.call_id().cloned()));
                    question.with_outcome(ApprovalOutcome::AllowedOnce)
                }
            },
        )
        .unwrap();
        let svc = ApprovalService::new(ctx, ApprovalPolicy::Ask);
        let mut session = open_turn_session();
        let out = svc
            .request(
                &mut session,
                ApprovalRequest::new("bash").with_call_id(CallId::new("call-9")),
            )
            .await
            .unwrap();
        let (session_id, call_id) = seen.lock().expect("seen").clone().expect("listener ran");
        assert_eq!(&session_id, session.id());
        assert_eq!(call_id, Some(CallId::new("call-9")));
        assert!(matches!(out, ApprovalOutcome::AllowedOnce));
    }
}
