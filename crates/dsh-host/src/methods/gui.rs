//! Combined dotted GUI handler: session methods plus the rest of the Phase 7 map.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dsh_commands::CommandRegistry;
use dsh_credentials::{CredentialError, LayeredCredentials};
use dsh_rpc::{RpcError, RpcErrorCode, RpcId, RpcResult};
use dsh_session::SessionId;
use dsh_settings::{MutateOp, NamespaceView, SettingsError, SettingsService};
use dsh_skill::SkillRegistry;
use dsh_workspace::{WorkspaceError, WorkspaceId, WorkspaceRegistry, WorkspaceView};
use serde_json::{Map, Value, json};

use super::session::SessionHandler;
use crate::dispatch::RpcHandler;
use crate::lookup::AgentLookup;

const LIST_DIRECTORY_CAP: usize = 500;

/// Optional in-process services mounted beside [`SessionHandler`].
#[derive(Default)]
pub struct GuiServices {
    workspaces: Option<Arc<WorkspaceRegistry>>,
    settings: Option<Arc<SettingsService>>,
    credentials: Option<Arc<LayeredCredentials>>,
    commands: Option<Arc<CommandRegistry>>,
    skills: Option<Arc<SkillRegistry>>,
}

impl GuiServices {
    /// All services absent: dotted methods that need one answer empty or `internal`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Durable workspace registry for `workspace.*` and `session.create` attach.
    #[must_use]
    pub fn workspaces(mut self, value: Arc<WorkspaceRegistry>) -> Self {
        self.workspaces = Some(value);
        self
    }

    /// File-backed settings for `settings.*`.
    #[must_use]
    pub fn settings(mut self, value: Arc<SettingsService>) -> Self {
        self.settings = Some(value);
        self
    }

    /// Layered credential store for `credentials.*`.
    #[must_use]
    pub fn credentials(mut self, value: Arc<LayeredCredentials>) -> Self {
        self.credentials = Some(value);
        self
    }

    /// Slash-command registry for `commands/list` and `commands/execute`.
    #[must_use]
    pub fn commands(mut self, value: Arc<CommandRegistry>) -> Self {
        self.commands = Some(value);
        self
    }

    /// Skill catalog for `skill.list`.
    #[must_use]
    pub fn skills(mut self, value: Arc<SkillRegistry>) -> Self {
        self.skills = Some(value);
        self
    }
}

/// Answers session.* plus the remaining Phase 7 dotted methods and slash remotes.
pub struct GuiHandler {
    sessions: SessionHandler,
    lookup: AgentLookup,
    workspaces: Option<Arc<WorkspaceRegistry>>,
    settings: Option<Arc<SettingsService>>,
    credentials: Option<Arc<LayeredCredentials>>,
    commands: Option<Arc<CommandRegistry>>,
    skills: Option<Arc<SkillRegistry>>,
}

impl GuiHandler {
    /// `lookup` is shared with [`SessionHandler`]. Missing services stay honest empties or errors.
    #[must_use]
    pub fn new(lookup: AgentLookup, services: GuiServices) -> Self {
        Self {
            sessions: SessionHandler::new(lookup.clone(), services.workspaces.clone()),
            lookup,
            workspaces: services.workspaces,
            settings: services.settings,
            credentials: services.credentials,
            commands: services.commands,
            skills: services.skills,
        }
    }
}

impl RpcHandler for GuiHandler {
    fn accepts_dotted(&self, method: &str) -> bool {
        if self.sessions.accepts_dotted(method) || method.starts_with("goal.") {
            return true;
        }
        matches!(
            method,
            "host.listDirectory"
                | "host.createDirectory"
                | "host.pickDirectory"
                | "host.openPath"
                | "workspace.list"
                | "workspace.create"
                | "workspace.rename"
                | "workspace.delete"
                | "workspace.insertBefore"
                | "workspace.insertSessionBefore"
                | "workspace.archiveSession"
                | "skill.list"
                | "agentPreset.list"
                | "agentPreset.select"
                | "agentPreset.read"
                | "agentPreset.copy"
                | "agentPreset.openDocument"
                | "agentPreset.remove"
                | "llm.providers"
                | "llm.models"
                | "llm.discoverModels"
                | "settings.describe"
                | "settings.openDocument"
                | "settings.update"
                | "settings.replace"
                | "settings.mutate"
                | "credentials.describe"
                | "credentials.set"
                | "credentials.unset"
                | "subagent.list"
        )
    }

    fn handle_dotted(
        &self,
        method: &str,
        rpc_id: &RpcId,
        payload: Value,
    ) -> impl std::future::Future<Output = RpcResult> + Send {
        let sessions = self.sessions.clone();
        let lookup = self.lookup.clone();
        let workspaces = self.workspaces.clone();
        let settings = self.settings.clone();
        let credentials = self.credentials.clone();
        let skills = self.skills.clone();
        let method = method.to_string();
        let rpc_id = rpc_id.clone();
        async move {
            if sessions.accepts_dotted(&method) {
                return sessions.handle_dotted(&method, &rpc_id, payload).await;
            }
            dispatch_gui(
                &lookup,
                workspaces.as_deref(),
                settings.as_deref(),
                credentials.as_deref(),
                skills.as_deref(),
                &method,
                payload,
            )
        }
    }

    fn handle_slash(
        &self,
        method: &str,
        _rpc_id: &RpcId,
        payload: Value,
    ) -> impl std::future::Future<Output = Option<RpcResult>> + Send {
        let commands = self.commands.clone();
        let method = method.to_string();
        async move { crate::slash::handle_slash(commands.as_deref(), &method, &payload).await }
    }
}

fn dispatch_gui(
    lookup: &AgentLookup,
    workspaces: Option<&WorkspaceRegistry>,
    settings: Option<&SettingsService>,
    credentials: Option<&LayeredCredentials>,
    skills: Option<&SkillRegistry>,
    method: &str,
    payload: Value,
) -> RpcResult {
    if method.starts_with("goal.") {
        return RpcResult::err(RpcError::internal("goals are not implemented in Phase 7"));
    }
    match method {
        "host.listDirectory" => list_directory(&payload),
        "host.createDirectory" => create_directory(&payload),
        "host.pickDirectory" => RpcResult::err(RpcError::with_code(
            RpcErrorCode::DirectoryPickerUnavailable,
            "directory-picker-unavailable",
            json!({}),
        )),
        "host.openPath" => RpcResult::err(RpcError::internal("openPath is unavailable")),
        "workspace.list" => workspace_list(workspaces),
        "workspace.create" => workspace_create(workspaces, &payload),
        "workspace.rename" => workspace_rename(workspaces, &payload),
        "workspace.delete" => workspace_delete(workspaces, &payload),
        "workspace.insertBefore" => workspace_insert_before(workspaces, &payload),
        "workspace.insertSessionBefore" => workspace_insert_session_before(workspaces, &payload),
        "workspace.archiveSession" => workspace_archive_session(workspaces, &payload),
        "skill.list" => skill_list(lookup, skills, &payload),
        "agentPreset.list" => agent_preset_list(),
        "agentPreset.select" => agent_preset_select(&payload),
        "agentPreset.read"
        | "agentPreset.copy"
        | "agentPreset.openDocument"
        | "agentPreset.remove" => RpcResult::err(RpcError::with_code(
            RpcErrorCode::AgentPresetReadOnly,
            "agent-preset-read-only",
            json!({}),
        )),
        "llm.providers" => llm_providers(lookup),
        "llm.models" => llm_models(lookup),
        "llm.discoverModels" => RpcResult::err(RpcError::with_code(
            RpcErrorCode::ModelDiscoveryFailed,
            "model-discovery-failed",
            json!({}),
        )),
        "settings.describe" => settings_describe(settings),
        "settings.openDocument" => RpcResult::err(RpcError::internal("not implemented in Phase 7")),
        "settings.update" => settings_update(settings, &payload),
        "settings.replace" => settings_replace(settings, &payload),
        "settings.mutate" => settings_mutate(settings, &payload),
        "credentials.describe" => credentials_describe(credentials, &payload),
        "credentials.set" => credentials_set(credentials, &payload),
        "credentials.unset" => credentials_unset(credentials, &payload),
        "subagent.list" => RpcResult::ok(json!({ "items": [] })),
        _ => RpcResult::err(RpcError::internal("uninstalled dotted method")),
    }
}

fn bad_request(message: &str) -> RpcResult {
    RpcResult::err(RpcError::with_code(
        RpcErrorCode::BadRequest,
        message,
        json!({}),
    ))
}

fn missing_workspaces() -> RpcResult {
    RpcResult::err(RpcError::internal("workspace registry is not mounted"))
}

fn missing_settings() -> RpcResult {
    RpcResult::err(RpcError::with_code(
        RpcErrorCode::SettingsNotExposed,
        "settings are not mounted",
        json!({}),
    ))
}

fn missing_credentials() -> RpcResult {
    RpcResult::err(RpcError::internal("credentials are not mounted"))
}

fn workspace_rpc(error: WorkspaceError) -> RpcResult {
    RpcResult::err(RpcError::with_code(
        error.rpc_code(),
        error.to_string(),
        json!({}),
    ))
}

fn settings_rpc(error: SettingsError) -> RpcResult {
    RpcResult::err(RpcError::with_code(
        error.rpc_code(),
        error.to_string(),
        error.details(),
    ))
}

fn credentials_rpc(error: CredentialError) -> RpcResult {
    RpcResult::err(RpcError::with_code(
        error.rpc_code(),
        error.to_string(),
        json!({}),
    ))
}

fn workspace_view_json(view: &WorkspaceView) -> Value {
    json!({
        "workspaceId": view.workspace_id().as_str(),
        "path": view.path(),
        "title": view.title(),
        "sessionIds": view.session_ids().iter().map(SessionId::as_str).collect::<Vec<_>>(),
        "createdAt": view.created_at(),
        "updatedAt": view.updated_at(),
    })
}

fn namespace_view_json(view: &NamespaceView) -> Value {
    json!({
        "ns": view.ns(),
        "schema": view.schema(),
        "value": view.value(),
        "applies": view.applies(),
        "secrets": view.secrets(),
        "revision": view.revision(),
    })
}

fn require_workspace_id(payload: &Value) -> Result<WorkspaceId, RpcResult> {
    match payload.get("workspaceId").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => Ok(WorkspaceId::new(id)),
        _ => Err(bad_request("workspaceId is required")),
    }
}

fn require_session_id_field(payload: &Value, field: &str) -> Result<SessionId, RpcResult> {
    match payload.get(field).and_then(Value::as_str) {
        Some(id) if !id.is_empty() => Ok(SessionId::new(id)),
        _ => Err(bad_request(&format!("{field} is required"))),
    }
}

fn home_directory() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    if let Ok(cwd) = std::env::var("DSH_CWD") {
        if !cwd.is_empty() {
            return PathBuf::from(cwd);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn directory_unreadable(path: &str) -> RpcResult {
    RpcResult::err(RpcError::with_code(
        RpcErrorCode::DirectoryUnreadable,
        "directory-unreadable",
        json!({ "path": path }),
    ))
}

fn ancestry_crumbs(path: &Path) -> Vec<Value> {
    let mut parts = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        let name = match current.parent() {
            Some(_) => current
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| current.to_string_lossy().into_owned()),
            None => current.to_string_lossy().into_owned(),
        };
        parts.push(json!({
            "name": name,
            "path": current.to_string_lossy(),
            "hidden": false,
        }));
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }
    parts.reverse();
    parts
}

fn list_directory(payload: &Value) -> RpcResult {
    let home = home_directory();
    let home_str = home.to_string_lossy().into_owned();
    let explicit = payload.get("path").and_then(Value::as_str);
    let target = match explicit {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => home.clone(),
    };
    let target_str = target.to_string_lossy().into_owned();
    if let Some(path) = explicit {
        if !path.is_empty() && !target.is_absolute() {
            return directory_unreadable(&target_str);
        }
    }
    let read = match std::fs::read_dir(&target) {
        Ok(read) => read,
        Err(_) => return directory_unreadable(&target_str),
    };
    let mut entries = Vec::new();
    for entry in read {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let hidden = name.starts_with('.');
        entries.push(json!({
            "name": name,
            "path": path.to_string_lossy(),
            "hidden": hidden,
        }));
    }
    entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    let truncated = entries.len() > LIST_DIRECTORY_CAP;
    entries.truncate(LIST_DIRECTORY_CAP);
    RpcResult::ok(json!({
        "path": target_str,
        "home": home_str,
        "crumbs": ancestry_crumbs(&target),
        "entries": entries,
        "truncated": truncated,
    }))
}

fn is_plain_segment(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return false;
    }
    !trimmed.contains('/') && !trimmed.contains('\\')
}

fn create_directory(payload: &Value) -> RpcResult {
    let Some(parent) = payload.get("path").and_then(Value::as_str) else {
        return bad_request("path is required");
    };
    let Some(name) = payload.get("name").and_then(Value::as_str) else {
        return bad_request("name is required");
    };
    if !is_plain_segment(name) {
        return bad_request("name must be a single non-blank path segment");
    }
    let parent_path = Path::new(parent);
    if !parent_path.is_absolute() {
        return RpcResult::err(RpcError::with_code(
            RpcErrorCode::DirectoryCreateFailed,
            "directory-create-failed",
            json!({ "path": parent }),
        ));
    }
    let child = parent_path.join(name);
    let child_str = child.to_string_lossy().into_owned();
    if child.exists() {
        return RpcResult::err(RpcError::with_code(
            RpcErrorCode::DirectoryExists,
            "directory-exists",
            json!({ "path": child_str }),
        ));
    }
    match std::fs::create_dir(&child) {
        Ok(()) => RpcResult::ok(json!({ "path": child_str })),
        Err(_) => RpcResult::err(RpcError::with_code(
            RpcErrorCode::DirectoryCreateFailed,
            "directory-create-failed",
            json!({ "path": child_str }),
        )),
    }
}

fn workspace_list(workspaces: Option<&WorkspaceRegistry>) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    match registry.list() {
        Ok(listed) => RpcResult::ok(json!({
            "items": listed.items().iter().map(workspace_view_json).collect::<Vec<_>>(),
            "archivedSessionIds": listed
                .archived_session_ids()
                .iter()
                .map(SessionId::as_str)
                .collect::<Vec<_>>(),
        })),
        Err(error) => workspace_rpc(error),
    }
}

fn workspace_create(workspaces: Option<&WorkspaceRegistry>, payload: &Value) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    let Some(path) = payload.get("path").and_then(Value::as_str) else {
        return bad_request("path is required");
    };
    match registry.create(Path::new(path)) {
        Ok((view, created)) => RpcResult::ok(json!({
            "workspace": workspace_view_json(&view),
            "created": created,
        })),
        Err(error) => workspace_rpc(error),
    }
}

fn workspace_rename(workspaces: Option<&WorkspaceRegistry>, payload: &Value) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    let id = match require_workspace_id(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let Some(title) = payload.get("title").and_then(Value::as_str) else {
        return bad_request("title is required");
    };
    match registry.rename(&id, title) {
        Ok(view) => RpcResult::ok(json!({ "workspace": workspace_view_json(&view) })),
        Err(error) => workspace_rpc(error),
    }
}

fn workspace_delete(workspaces: Option<&WorkspaceRegistry>, payload: &Value) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    let id = match require_workspace_id(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    match registry.delete(&id) {
        Ok(()) => RpcResult::ok(json!({ "deleted": true })),
        Err(error) => workspace_rpc(error),
    }
}

fn workspace_insert_before(workspaces: Option<&WorkspaceRegistry>, payload: &Value) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    let id = match require_workspace_id(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let before = match payload.get("beforeWorkspaceId").and_then(Value::as_str) {
        Some(before) if !before.is_empty() => Some(WorkspaceId::new(before)),
        _ => None,
    };
    match registry.insert_before(&id, before.as_ref()) {
        Ok(ids) => RpcResult::ok(json!({
            "workspaceIds": ids.iter().map(WorkspaceId::as_str).collect::<Vec<_>>(),
        })),
        Err(error) => workspace_rpc(error),
    }
}

fn workspace_insert_session_before(
    workspaces: Option<&WorkspaceRegistry>,
    payload: &Value,
) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    let workspace_id = match require_workspace_id(payload) {
        Ok(id) => id,
        Err(error) => return error,
    };
    let session_id = match require_session_id_field(payload, "sessionId") {
        Ok(id) => id,
        Err(error) => return error,
    };
    let before = match payload.get("beforeSessionId").and_then(Value::as_str) {
        Some(before) if !before.is_empty() => Some(SessionId::new(before)),
        _ => None,
    };
    match registry.insert_session_before(&workspace_id, &session_id, before.as_ref()) {
        Ok(view) => RpcResult::ok(json!({ "workspace": workspace_view_json(&view) })),
        Err(error) => workspace_rpc(error),
    }
}

fn workspace_archive_session(workspaces: Option<&WorkspaceRegistry>, payload: &Value) -> RpcResult {
    let Some(registry) = workspaces else {
        return missing_workspaces();
    };
    let session_id = match require_session_id_field(payload, "sessionId") {
        Ok(id) => id,
        Err(error) => return error,
    };
    match registry.archive_session(&session_id) {
        Ok(ids) => RpcResult::ok(json!({
            "archivedSessionIds": ids.iter().map(SessionId::as_str).collect::<Vec<_>>(),
        })),
        Err(error) => workspace_rpc(error),
    }
}

fn session_cwd(lookup: &AgentLookup, session_id: &str) -> Option<String> {
    if let Some(handle) = lookup.registry().get(session_id) {
        return handle.lock().session.header().cwd.clone();
    }
    lookup
        .store()
        .load(&SessionId::new(session_id))
        .ok()
        .and_then(|session| session.header().cwd.clone())
}

fn skill_list(lookup: &AgentLookup, skills: Option<&SkillRegistry>, payload: &Value) -> RpcResult {
    let cwd = payload
        .get("sessionId")
        .and_then(Value::as_str)
        .and_then(|id| session_cwd(lookup, id));
    let listed = match skills {
        Some(registry) => registry.list(cwd.as_deref()),
        None => Vec::new(),
    };
    let skills: Vec<Value> = listed
        .into_iter()
        .map(|skill| {
            json!({
                "name": skill.name,
                "description": skill.description,
            })
        })
        .collect();
    RpcResult::ok(json!({ "skills": skills }))
}

fn agent_preset_list() -> RpcResult {
    RpcResult::ok(json!({
        "presets": [{
            "id": "standard",
            "name": "standard",
            "readOnly": true,
        }],
        "authorable": false,
        "hasDocument": false,
    }))
}

fn agent_preset_select(payload: &Value) -> RpcResult {
    let preset = payload
        .get("agentPreset")
        .and_then(Value::as_str)
        .unwrap_or("");
    if preset.is_empty() || preset == "standard" {
        return RpcResult::ok(json!({ "agentPreset": "standard" }));
    }
    RpcResult::err(RpcError::with_code(
        RpcErrorCode::AgentPresetNotFound,
        "agent-preset-not-found",
        json!({ "available": ["standard"] }),
    ))
}

fn llm_providers(lookup: &AgentLookup) -> RpcResult {
    let providers: Vec<Value> = lookup
        .registry()
        .list_providers()
        .into_iter()
        .map(|id| {
            json!({
                "provider": id,
                "displayName": id,
                "settingsNs": "",
                "settingsPath": [],
                "active": true,
            })
        })
        .collect();
    RpcResult::ok(json!({ "providers": providers }))
}

fn llm_models(lookup: &AgentLookup) -> RpcResult {
    let available = !lookup.registry().list_providers().is_empty();
    RpcResult::ok(json!({
        "groups": [],
        "failures": [],
        "available": available,
    }))
}

fn settings_describe(settings: Option<&SettingsService>) -> RpcResult {
    let Some(service) = settings else {
        return missing_settings();
    };
    match service.describe_all() {
        Ok(all) => RpcResult::ok(json!({
            "writable": true,
            "hasDocument": true,
            "namespaces": all.namespaces().iter().map(namespace_view_json).collect::<Vec<_>>(),
        })),
        Err(error) => settings_rpc(error),
    }
}

fn expected_revision(payload: &Value) -> Option<u64> {
    payload.get("expectedRevision").and_then(Value::as_u64)
}

fn require_ns(payload: &Value) -> Result<&str, RpcResult> {
    match payload.get("ns").and_then(Value::as_str) {
        Some(ns) if !ns.is_empty() => Ok(ns),
        _ => Err(bad_request("ns is required")),
    }
}

fn settings_update(settings: Option<&SettingsService>, payload: &Value) -> RpcResult {
    let Some(service) = settings else {
        return missing_settings();
    };
    let ns = match require_ns(payload) {
        Ok(ns) => ns,
        Err(error) => return error,
    };
    let Some(patch) = payload.get("patch").cloned() else {
        return bad_request("patch is required");
    };
    match service.update(ns, patch, expected_revision(payload)) {
        Ok(view) => RpcResult::ok(namespace_view_json(&view)),
        Err(error) => settings_rpc(error),
    }
}

fn settings_replace(settings: Option<&SettingsService>, payload: &Value) -> RpcResult {
    let Some(service) = settings else {
        return missing_settings();
    };
    let ns = match require_ns(payload) {
        Ok(ns) => ns,
        Err(error) => return error,
    };
    let Some(section) = payload.get("section").cloned() else {
        return bad_request("section is required");
    };
    match service.replace(ns, section, expected_revision(payload)) {
        Ok(view) => RpcResult::ok(namespace_view_json(&view)),
        Err(error) => settings_rpc(error),
    }
}

fn parse_ops(value: &Value) -> Result<Vec<MutateOp>, RpcResult> {
    let Some(ops) = value.as_array() else {
        return Err(bad_request("ops must be an array"));
    };
    let mut out = Vec::new();
    for op in ops {
        let path = match op.get("path").and_then(Value::as_array) {
            Some(parts) => {
                let mut path = Vec::new();
                for part in parts {
                    match part.as_str() {
                        Some(text) => path.push(text.to_string()),
                        None => return Err(bad_request("mutate op path must be strings")),
                    }
                }
                path
            }
            None => return Err(bad_request("mutate op path must be an array")),
        };
        match op.get("op").and_then(Value::as_str) {
            Some("set") => {
                let value = op.get("value").cloned().unwrap_or(Value::Null);
                out.push(MutateOp::set(path, value));
            }
            Some("unset") => out.push(MutateOp::unset(path)),
            _ => return Err(bad_request("mutate op must be set or unset")),
        }
    }
    Ok(out)
}

fn settings_mutate(settings: Option<&SettingsService>, payload: &Value) -> RpcResult {
    let Some(service) = settings else {
        return missing_settings();
    };
    let ns = match require_ns(payload) {
        Ok(ns) => ns,
        Err(error) => return error,
    };
    let Some(ops_value) = payload.get("ops") else {
        return bad_request("ops is required");
    };
    let ops = match parse_ops(ops_value) {
        Ok(ops) => ops,
        Err(error) => return error,
    };
    match service.mutate(ns, ops, expected_revision(payload)) {
        Ok(view) => RpcResult::ok(namespace_view_json(&view)),
        Err(error) => settings_rpc(error),
    }
}

fn credentials_describe(credentials: Option<&LayeredCredentials>, payload: &Value) -> RpcResult {
    let Some(store) = credentials else {
        return missing_credentials();
    };
    let Some(refs) = payload.get("refs").and_then(Value::as_array) else {
        return bad_request("refs is required");
    };
    let mut names = Vec::new();
    for item in refs {
        match item.as_str() {
            Some(name) => names.push(name.to_string()),
            None => return bad_request("refs must be strings"),
        }
    }
    match store.describe_refs(&names) {
        Ok(map) => {
            let mut object = Map::new();
            for (name, view) in map {
                let mut entry = json!({
                    "configured": view.configured(),
                    "writable": view.writable(),
                });
                if let Some(source) = view.source() {
                    entry["source"] = json!(source);
                }
                object.insert(name, entry);
            }
            RpcResult::ok(json!({ "credentials": object }))
        }
        Err(error) => credentials_rpc(error),
    }
}

fn credentials_set(credentials: Option<&LayeredCredentials>, payload: &Value) -> RpcResult {
    let Some(store) = credentials else {
        return missing_credentials();
    };
    let Some(r#ref) = payload.get("ref").and_then(Value::as_str) else {
        return bad_request("ref is required");
    };
    let Some(value) = payload.get("value").and_then(Value::as_str) else {
        return bad_request("value is required");
    };
    match store.set_ref(r#ref, value) {
        Ok(()) => RpcResult::ok(json!({})),
        Err(error) => credentials_rpc(error),
    }
}

fn credentials_unset(credentials: Option<&LayeredCredentials>, payload: &Value) -> RpcResult {
    let Some(store) = credentials else {
        return missing_credentials();
    };
    let Some(r#ref) = payload.get("ref").and_then(Value::as_str) else {
        return bad_request("ref is required");
    };
    match store.unset_ref(r#ref) {
        Ok(()) => RpcResult::ok(json!({})),
        Err(error) => credentials_rpc(error),
    }
}
