//! Dotted `session.*` GUI methods.

use std::collections::HashSet;
use std::sync::Arc;

use dsh_agent::CreateAgentOptions;
use dsh_rpc::{RpcError, RpcErrorCode, RpcId, RpcResult};
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
    SessionId, SessionTitleData,
};
use dsh_workspace::{WorkspaceId, WorkspaceRegistry};
use serde_json::{Value, json};

use crate::dispatch::RpcHandler;
use crate::lookup::{AgentLookup, DEFAULT_MODEL, DEFAULT_PROVIDER, LookupError};

/// Answers `session.*` and `host.describe`.
#[derive(Clone)]
pub struct SessionHandler {
    lookup: AgentLookup,
    workspaces: Option<Arc<WorkspaceRegistry>>,
}

impl SessionHandler {
    /// `workspaces` is used only for `session.create` attach; `None` ignores `workspaceId`.
    #[must_use]
    pub fn new(lookup: AgentLookup, workspaces: Option<Arc<WorkspaceRegistry>>) -> Self {
        Self { lookup, workspaces }
    }
}

impl RpcHandler for SessionHandler {
    fn accepts_dotted(&self, method: &str) -> bool {
        matches!(
            method,
            "host.describe"
                | "session.list"
                | "session.create"
                | "session.history"
                | "session.prompt"
                | "session.cancel"
                | "session.models"
                | "session.selectModel"
                | "session.rename"
                | "session.updateQueue"
                | "session.fork"
                | "session.search"
                | "session.attachment"
        )
    }

    fn handle_dotted(
        &self,
        method: &str,
        _rpc_id: &RpcId,
        payload: Value,
    ) -> impl std::future::Future<Output = RpcResult> + Send {
        let lookup = self.lookup.clone();
        let workspaces = self.workspaces.clone();
        let method = method.to_string();
        async move { dispatch_session(&lookup, workspaces.as_deref(), &method, payload).await }
    }
}

async fn dispatch_session(
    lookup: &AgentLookup,
    workspaces: Option<&WorkspaceRegistry>,
    method: &str,
    payload: Value,
) -> RpcResult {
    match method {
        "host.describe" => RpcResult::ok(host_describe_from_lookup(lookup)),
        "session.list" => session_list(lookup),
        "session.create" => session_create(lookup, workspaces, &payload),
        "session.history" => session_history(lookup, &payload).await,
        "session.prompt" => session_prompt(lookup, &payload).await,
        "session.cancel" => session_cancel(lookup, &payload).await,
        "session.models" => session_models(lookup, &payload).await,
        "session.selectModel" => session_select_model(lookup, &payload).await,
        "session.rename" => session_rename(lookup, &payload).await,
        "session.updateQueue" => RpcResult::err(RpcError::with_code(
            RpcErrorCode::QueueItemNotFound,
            "queue item not found",
            json!({}),
        )),
        "session.fork" => RpcResult::err(RpcError::with_code(
            RpcErrorCode::ForkUnavailable,
            "fork-unavailable",
            json!({}),
        )),
        "session.search" => RpcResult::err(RpcError::internal("not implemented in Phase 7")),
        "session.attachment" => RpcResult::err(RpcError::with_code(
            RpcErrorCode::AttachmentError,
            "attachment-error",
            json!({}),
        )),
        _ => RpcResult::err(RpcError::internal("uninstalled dotted method")),
    }
}

fn host_describe_from_lookup(lookup: &AgentLookup) -> Value {
    let attached = lookup.registry().list().len();
    let providers = lookup.registry().list_providers();
    let provider = match providers.first() {
        Some(id) => id.as_str(),
        None => DEFAULT_PROVIDER,
    };
    crate::dispatch::describe_host(attached, Some(provider), Some(DEFAULT_MODEL))
}

fn lookup_rpc(error: LookupError) -> RpcResult {
    match error {
        LookupError::AgentBusy { session_id } => RpcResult::err(RpcError::with_code(
            RpcErrorCode::AgentBusy,
            format!("session \"{session_id}\" is owned by subagent routing"),
            json!({ "reason": "use subagent delivery for this child session" }),
        )),
        LookupError::SessionNotFound { session_id } => RpcResult::err(RpcError::with_code(
            RpcErrorCode::SessionNotFound,
            format!("session \"{session_id}\" not found"),
            json!({ "sessionId": session_id }),
        )),
        LookupError::Internal(message) => RpcResult::err(RpcError::internal(message)),
    }
}

fn bad_request(message: &str) -> RpcResult {
    RpcResult::err(RpcError::with_code(
        RpcErrorCode::BadRequest,
        message,
        json!({}),
    ))
}

fn session_id_of(payload: &Value) -> Result<SessionId, RpcResult> {
    match payload.get("sessionId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => Ok(SessionId::new(id)),
        _ => Err(bad_request("sessionId is required")),
    }
}

fn session_list(lookup: &AgentLookup) -> RpcResult {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    match lookup.store().list_ids() {
        Ok(persisted) => {
            for id in persisted {
                if seen.insert(id.as_str().to_string()) {
                    ids.push(id);
                }
            }
        }
        Err(error) => return RpcResult::err(RpcError::internal(error.to_string())),
    }
    for handle in lookup.registry().list() {
        if seen.insert(handle.id().as_str().to_string()) {
            ids.push(handle.id().clone());
        }
    }
    let mut items = Vec::new();
    for id in ids {
        if let Some(handle) = lookup.registry().get(id.as_str()) {
            let item = {
                let agent = handle.lock();
                list_item(&agent.session)
            };
            items.push(item);
            continue;
        }
        if let Ok(session) = lookup.store().load(&id) {
            items.push(list_item(&session));
        }
    }
    RpcResult::ok(json!({ "items": items }))
}

fn list_item(session: &Session) -> Value {
    let header = session.header();
    let mut title = String::new();
    let mut blank = true;
    let mut last_prompt_at = header.created_at;
    for event in session.events() {
        let LogEvent::Known(known) = event else {
            continue;
        };
        match known {
            SessionEvent::SessionTitle { data, .. } => title = data.title.clone(),
            SessionEvent::TurnStart { .. } => blank = false,
            SessionEvent::UserMessage { time, .. } => last_prompt_at = *time,
            _ => {}
        }
    }
    let mut item = json!({
        "sessionId": header.id.as_str(),
        "title": title,
        "blank": blank,
        "lastPromptAt": last_prompt_at,
    });
    if let Some(cwd) = &header.cwd {
        item["cwd"] = json!(cwd);
    }
    item
}

fn session_create(
    lookup: &AgentLookup,
    workspaces: Option<&WorkspaceRegistry>,
    payload: &Value,
) -> RpcResult {
    let session_id = match payload.get("sessionId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => SessionId::new(id),
        _ => mint_session_id(),
    };
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    let handle = match lookup.registry().create(CreateAgentOptions {
        session_id,
        cwd,
        provider: DEFAULT_PROVIDER.into(),
        model: DEFAULT_MODEL.into(),
        max_tokens: None,
    }) {
        Ok(handle) => handle,
        Err(error) => return RpcResult::err(RpcError::internal(error.to_string())),
    };
    if let Some(ws_id) = payload.get("workspaceId").and_then(Value::as_str) {
        if let Some(workspaces) = workspaces {
            if let Err(error) = workspaces.attach_session(&WorkspaceId::new(ws_id), handle.id()) {
                return RpcResult::err(RpcError::with_code(
                    error.rpc_code(),
                    error.to_string(),
                    json!({}),
                ));
            }
        }
    }
    RpcResult::ok(json!({ "sessionId": handle.id().as_str() }))
}

async fn session_history(lookup: &AgentLookup, payload: &Value) -> RpcResult {
    let session_id = match session_id_of(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let handle = match lookup.agent_for(&session_id).await {
        Ok(handle) => handle,
        Err(error) => return lookup_rpc(error),
    };
    let before = payload.get("beforeSeq").and_then(Value::as_u64);
    let max_messages = payload.get("maxMessages").and_then(Value::as_u64);
    let (events, has_more) = {
        let agent = handle.lock();
        history_page(agent.session.events(), before, max_messages)
    };
    RpcResult::ok(json!({ "events": events, "hasMore": has_more }))
}

fn history_page(
    events: &[LogEvent],
    before: Option<u64>,
    max_messages: Option<u64>,
) -> (Vec<Value>, bool) {
    let mut selected: Vec<&LogEvent> = events
        .iter()
        .filter(|event| match before {
            Some(before) => event.seq() < before,
            None => true,
        })
        .collect();
    let mut has_more = false;
    if let Some(max) = max_messages {
        let max = max as usize;
        if selected.len() > max {
            selected = selected[selected.len() - max..].to_vec();
            has_more = true;
        }
    }
    let json_events = selected
        .into_iter()
        .filter_map(|event| serde_json::to_value(event).ok())
        .collect();
    (json_events, has_more)
}

async fn session_prompt(lookup: &AgentLookup, payload: &Value) -> RpcResult {
    let session_id = match session_id_of(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let text = match prompt_text(payload) {
        Ok(text) => text,
        Err(error) => return error,
    };
    let handle = match lookup.agent_for(&session_id).await {
        Ok(handle) => handle,
        Err(error) => return lookup_rpc(error),
    };
    let message = Message {
        id: MessageId::new(mint_message_id()),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text }],
        source: MessageSource::User,
    };
    let mode = payload.get("mode").and_then(Value::as_str).unwrap_or("");
    let queued = match mode {
        "queue" => handle.followup(message).await,
        "steer" => handle.steer(message).await,
        _ => return bad_request("session.prompt mode must be queue or steer"),
    };
    if let Err(error) = queued {
        return RpcResult::err(RpcError::internal(error.to_string()));
    }
    let driver = handle.clone();
    tokio::spawn(async move {
        let _ = driver.run_until_idle().await;
    });
    RpcResult::ok(json!({ "accepted": true }))
}

fn prompt_text(payload: &Value) -> Result<String, RpcResult> {
    let Some(parts) = payload.get("content").and_then(Value::as_array) else {
        return Err(bad_request("session.prompt requires content"));
    };
    let mut text = String::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(chunk) = part.get("text").and_then(Value::as_str) {
                    text.push_str(chunk);
                }
            }
            _ => {
                return Err(RpcResult::err(RpcError::with_code(
                    RpcErrorCode::AttachmentError,
                    "image and non-text prompt parts are not supported",
                    json!({}),
                )));
            }
        }
    }
    Ok(text)
}

async fn session_cancel(lookup: &AgentLookup, payload: &Value) -> RpcResult {
    let session_id = match session_id_of(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let handle = match lookup.agent_for(&session_id).await {
        Ok(handle) => handle,
        Err(error) => return lookup_rpc(error),
    };
    if let Err(error) = handle.cancel().await {
        return RpcResult::err(RpcError::internal(error.to_string()));
    }
    RpcResult::ok(json!({ "accepted": true }))
}

async fn session_models(lookup: &AgentLookup, payload: &Value) -> RpcResult {
    let session_id = match session_id_of(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let handle = match lookup.agent_for(&session_id).await {
        Ok(handle) => handle,
        Err(error) => return lookup_rpc(error),
    };
    let current = {
        let agent = handle.lock();
        json!({
            "provider": agent.options.provider,
            "model": agent.options.model,
        })
    };
    let available = !lookup.registry().list_providers().is_empty();
    RpcResult::ok(json!({
        "current": current,
        "available": available,
        "groups": [],
        "failures": [],
    }))
}

async fn session_select_model(lookup: &AgentLookup, payload: &Value) -> RpcResult {
    let session_id = match session_id_of(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let Some(provider) = payload.get("provider").and_then(Value::as_str) else {
        return bad_request("provider is required");
    };
    let Some(model) = payload.get("model").and_then(Value::as_str) else {
        return bad_request("model is required");
    };
    if provider.is_empty() || model.is_empty() {
        return bad_request("provider and model must be non-empty");
    }
    let handle = match lookup.agent_for(&session_id).await {
        Ok(handle) => handle,
        Err(error) => return lookup_rpc(error),
    };
    {
        let mut agent = handle.lock();
        agent.options.provider = provider.to_string();
        agent.options.model = model.to_string();
    }
    RpcResult::ok(json!({
        "current": { "provider": provider, "model": model }
    }))
}

async fn session_rename(lookup: &AgentLookup, payload: &Value) -> RpcResult {
    let session_id = match session_id_of(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let Some(title) = payload.get("title").and_then(Value::as_str) else {
        return bad_request("title is required");
    };
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return RpcResult::err(RpcError::with_code(
            RpcErrorCode::TitleInvalid,
            "title is empty",
            json!({}),
        ));
    }
    let title = trimmed.to_string();
    let handle = match lookup.agent_for(&session_id).await {
        Ok(handle) => handle,
        Err(error) => return lookup_rpc(error),
    };
    let seq = {
        let mut agent = handle.lock();
        let seq = agent.session.events().len() as u64;
        match agent.session.append(SessionEvent::SessionTitle {
            seq,
            time: seq as i64,
            data: SessionTitleData {
                title: title.clone(),
                message_seqs: Vec::new(),
                source: json!({ "kind": "user" }),
            },
            ignorable: None,
        }) {
            Ok(_) => seq,
            Err(error) => return RpcResult::err(RpcError::internal(error.to_string())),
        }
    };
    RpcResult::ok(json!({ "title": title, "seq": seq }))
}

fn mint_session_id() -> SessionId {
    SessionId::new(format!("sess-{}-{}", std::process::id(), unix_nanos()))
}

fn mint_message_id() -> String {
    format!("msg-{}-{}", std::process::id(), unix_nanos())
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::SessionHandler;
    use crate::dispatch::RpcHandler;
    use crate::lookup::{AgentLookup, hanging_registry, mock_registry, test_temp_dir};
    use dsh_rpc::RpcId;
    use dsh_session::ContentBlock;
    use dsh_session_persist::JsonlSessionStore;
    use serde_json::json;
    use std::sync::Arc;

    fn handler() -> (SessionHandler, Arc<dsh_agent::AgentRegistry>) {
        let registry = Arc::new(mock_registry());
        let store = Arc::new(JsonlSessionStore::with_root(test_temp_dir("session-rpc")));
        let lookup = AgentLookup::new(Arc::clone(&registry), store);
        (SessionHandler::new(lookup, None), registry)
    }

    #[tokio::test]
    async fn session_create_then_list_contains_id() {
        let (handler, _) = handler();
        let created = handler
            .handle_dotted(
                "session.create",
                &RpcId::new("r-create"),
                json!({"sessionId": "sess-create-1", "cwd": "/work"}),
            )
            .await;
        let value = created.as_ok().expect("create ok");
        assert_eq!(value["sessionId"], "sess-create-1");
        let listed = handler
            .handle_dotted("session.list", &RpcId::new("r-list"), json!({}))
            .await;
        let items = listed.as_ok().expect("list ok")["items"]
            .as_array()
            .expect("items");
        assert!(
            items
                .iter()
                .any(|item| item["sessionId"] == "sess-create-1"),
            "{items:?}"
        );
    }

    #[tokio::test]
    async fn session_prompt_text_followup_accepted() {
        let registry = Arc::new(hanging_registry());
        let store = Arc::new(JsonlSessionStore::with_root(test_temp_dir(
            "session-prompt",
        )));
        let lookup = AgentLookup::new(Arc::clone(&registry), store);
        let handler = SessionHandler::new(lookup, None);
        handler
            .handle_dotted(
                "session.create",
                &RpcId::new("r-create"),
                json!({"sessionId": "sess-prompt-1", "cwd": "/work"}),
            )
            .await
            .as_ok()
            .expect("create");
        let prompted = handler
            .handle_dotted(
                "session.prompt",
                &RpcId::new("r-prompt"),
                json!({
                    "sessionId": "sess-prompt-1",
                    "mode": "queue",
                    "content": [{"type": "text", "text": "hello-followup"}]
                }),
            )
            .await;
        let value = prompted.as_ok().expect("prompt ok");
        assert_eq!(value["accepted"], true);
        let handle = registry.get("sess-prompt-1").expect("live");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let queued = {
                let agent = handle.lock();
                let in_inbox = agent
                    .inbox
                    .next_turn()
                    .iter()
                    .chain(agent.inbox.next_step().iter())
                    .any(|message| {
                        message.content.iter().any(|block| match block {
                            ContentBlock::Text { text } => text == "hello-followup",
                            _ => false,
                        })
                    });
                let in_events = format!("{:?}", agent.session.events()).contains("hello-followup");
                in_inbox || in_events
            };
            if queued {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("text followup was not queued");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let _ = handle.cancel().await;
    }
}
