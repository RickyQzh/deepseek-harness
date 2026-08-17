//! Live reuse and cold resume of GUI sessions.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dsh_agent::{AgentHandle, AgentRegistry, CreateAgentOptions};
use dsh_session::{SessionHeader, SessionId, SessionOrigin};
use dsh_session_persist::JsonlSessionStore;
use futures::FutureExt;
use futures::future::Shared;
use tokio::sync::Mutex;

/// Default provider route for GUI create and cold resume.
pub const DEFAULT_PROVIDER: &str = "deepseek-official";
/// Default model id for GUI create and cold resume.
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

type SharedResume = Shared<Pin<Box<dyn Future<Output = Result<AgentHandle, LookupError>> + Send>>>;

/// Failure looking up a live or persisted session for GUI RPC.
#[derive(Clone, Debug, thiserror::Error)]
pub enum LookupError {
    /// Header `origin` is subagent or `parent_session` is set.
    #[error("session \"{session_id}\" is owned by subagent routing")]
    AgentBusy {
        /// Rejected session id.
        session_id: String,
    },
    /// No live handle and no loadable JSONL with a cwd.
    #[error("session \"{session_id}\" not found")]
    SessionNotFound {
        /// Missing session id.
        session_id: String,
    },
    /// Resume or persist failure that is not a missing session.
    #[error("{0}")]
    Internal(String),
}

impl LookupError {
    fn agent_busy(session_id: impl Into<String>) -> Self {
        Self::AgentBusy {
            session_id: session_id.into(),
        }
    }

    fn session_not_found(session_id: impl Into<String>) -> Self {
        Self::SessionNotFound {
            session_id: session_id.into(),
        }
    }
}

/// Resolves a session id to a live [`AgentHandle`], resuming from JSONL when needed.
#[derive(Clone)]
pub struct AgentLookup {
    registry: Arc<AgentRegistry>,
    store: Arc<JsonlSessionStore>,
    inflight: Arc<Mutex<HashMap<String, SharedResume>>>,
}

impl AgentLookup {
    /// Share `registry` and `store` across dotted session methods.
    #[must_use]
    pub fn new(registry: Arc<AgentRegistry>, store: Arc<JsonlSessionStore>) -> Self {
        Self {
            registry,
            store,
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Live agent registry used by create, list, and models.
    #[must_use]
    pub fn registry(&self) -> &Arc<AgentRegistry> {
        &self.registry
    }

    /// JSONL store used by list and cold resume.
    #[must_use]
    pub fn store(&self) -> &Arc<JsonlSessionStore> {
        &self.store
    }

    /// Reuse a live handle or single-flight a cold resume.
    ///
    /// Subagent-owned identities (`origin == subagent` or `parent_session` set) are
    /// [`LookupError::AgentBusy`]. Missing JSONL or a loaded header with no cwd is
    /// [`LookupError::SessionNotFound`].
    ///
    /// # Errors
    ///
    /// [`LookupError`] as described above; persist or resume failures become
    /// [`LookupError::Internal`].
    pub async fn agent_for(&self, session_id: &SessionId) -> Result<AgentHandle, LookupError> {
        if let Some(handle) = self.fenced_live(session_id)? {
            return Ok(handle);
        }
        let key = session_id.as_str().to_string();
        let shared = {
            let mut map = self.inflight.lock().await;
            if let Some(existing) = map.get(&key) {
                existing.clone()
            } else {
                let registry = Arc::clone(&self.registry);
                let store = Arc::clone(&self.store);
                let inflight = Arc::clone(&self.inflight);
                let sid = session_id.clone();
                let key_clear = key.clone();
                let fut = async move {
                    let result = resume_cold(&registry, &store, &sid);
                    inflight.lock().await.remove(&key_clear);
                    result
                }
                .boxed()
                .shared();
                map.insert(key, fut.clone());
                fut
            }
        };
        let resumed = shared.await;
        match self.fenced_live(session_id) {
            Ok(Some(handle)) => Ok(handle),
            Ok(None) => resumed,
            Err(error) => Err(error),
        }
    }

    fn fenced_live(&self, session_id: &SessionId) -> Result<Option<AgentHandle>, LookupError> {
        let Some(handle) = self.registry.get(session_id.as_str()) else {
            return Ok(None);
        };
        let owned = {
            let agent = handle.lock();
            is_subagent_owned(agent.session.header())
        };
        if owned {
            return Err(LookupError::agent_busy(session_id.as_str()));
        }
        Ok(Some(handle))
    }
}

fn is_subagent_owned(header: &SessionHeader) -> bool {
    matches!(header.origin, Some(SessionOrigin::Subagent)) || header.parent_session.is_some()
}

fn resume_cold(
    registry: &AgentRegistry,
    store: &JsonlSessionStore,
    session_id: &SessionId,
) -> Result<AgentHandle, LookupError> {
    if !store.path_for(session_id).is_file() {
        return Err(LookupError::session_not_found(session_id.as_str()));
    }
    let session = store
        .load(session_id)
        .map_err(|error| LookupError::Internal(error.to_string()))?;
    if is_subagent_owned(session.header()) {
        return Err(LookupError::agent_busy(session_id.as_str()));
    }
    if session.header().cwd.is_none() {
        return Err(LookupError::session_not_found(session_id.as_str()));
    }
    let options = CreateAgentOptions {
        session_id: session.id().clone(),
        cwd: session.header().cwd.clone(),
        provider: DEFAULT_PROVIDER.into(),
        model: DEFAULT_MODEL.into(),
        max_tokens: None,
    };
    registry
        .resume(session, options)
        .map_err(|error| LookupError::Internal(error.to_string()))
}

#[cfg(test)]
pub(crate) fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[cfg(test)]
pub(crate) fn mock_registry() -> AgentRegistry {
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript, text_response};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
        "ok",
    ))]));
    let mut llm = LlmRuntime::new();
    llm.register_adapter(DEFAULT_PROVIDER, adapter);
    AgentRegistry::new(
        llm,
        ToolRuntime::new(ToolPresentationMode::Native),
        SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
    )
}

#[cfg(test)]
pub(crate) fn hanging_registry() -> AgentRegistry {
    use dsh_llm::{LlmRuntime, MockAdapter, MockScript};
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_tools::{ToolPresentationMode, ToolRuntime};
    let adapter = Arc::new(MockAdapter::new(vec![MockScript::Hang]));
    let mut llm = LlmRuntime::new();
    llm.register_adapter(DEFAULT_PROVIDER, adapter);
    AgentRegistry::new(
        llm,
        ToolRuntime::new(ToolPresentationMode::Native),
        SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
    )
}

#[cfg(test)]
fn create_opts(id: &str, cwd: Option<&str>) -> CreateAgentOptions {
    CreateAgentOptions {
        session_id: SessionId::new(id),
        cwd: cwd.map(str::to_string),
        provider: DEFAULT_PROVIDER.into(),
        model: DEFAULT_MODEL.into(),
        max_tokens: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentLookup, LookupError, create_opts, mock_registry, test_temp_dir};
    use dsh_agent::AgentHandle;
    use dsh_session::{SESSION_FORMAT_VERSION, Session, SessionHeader, SessionId, SessionOrigin};
    use dsh_session_persist::JsonlSessionStore;
    use std::sync::Arc;

    fn lookup_pair() -> (AgentLookup, Arc<dsh_agent::AgentRegistry>) {
        let registry = Arc::new(mock_registry());
        let store = Arc::new(JsonlSessionStore::with_root(test_temp_dir("lookup")));
        (AgentLookup::new(Arc::clone(&registry), store), registry)
    }

    fn header(id: &str, origin: Option<SessionOrigin>, parent: Option<&str>) -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new(id),
            created_at: 1,
            cwd: Some("/work".into()),
            parent_session: parent.map(SessionId::new),
            seed_length: None,
            origin,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    #[tokio::test]
    async fn agent_for_reuses_live_handle() {
        let (lookup, registry) = lookup_pair();
        let created = registry
            .create(create_opts("sess-live", Some("/work")))
            .unwrap();
        let found = lookup
            .agent_for(&SessionId::new("sess-live"))
            .await
            .expect("live");
        assert_eq!(found.id().as_str(), created.id().as_str());
        assert!(AgentHandle::ptr_eq(&found, &created));
    }

    #[tokio::test]
    async fn agent_for_single_flight_cold_resume() {
        let store = Arc::new(JsonlSessionStore::with_root(test_temp_dir("lookup-cold")));
        store
            .flush(&Session::new(header("sess-cold", None, None)))
            .unwrap();
        let registry = Arc::new(mock_registry());
        let lookup = AgentLookup::new(Arc::clone(&registry), store);
        let id = SessionId::new("sess-cold");
        let (a, b) = tokio::join!(lookup.agent_for(&id), lookup.agent_for(&id));
        let a = a.expect("first");
        let b = b.expect("second");
        assert_eq!(a.id().as_str(), "sess-cold");
        assert_eq!(b.id().as_str(), "sess-cold");
        assert!(AgentHandle::ptr_eq(&a, &b));
        assert_eq!(registry.list().len(), 1);
    }

    #[tokio::test]
    async fn agent_for_rejects_subagent_origin() {
        let store = Arc::new(JsonlSessionStore::with_root(test_temp_dir("lookup-sub")));
        store
            .flush(&Session::new(header(
                "sess-child",
                Some(SessionOrigin::Subagent),
                None,
            )))
            .unwrap();
        let lookup = AgentLookup::new(Arc::new(mock_registry()), store);
        let err = match lookup.agent_for(&SessionId::new("sess-child")).await {
            Err(err) => err,
            Ok(_) => panic!("expected agent-busy"),
        };
        assert!(matches!(err, LookupError::AgentBusy { .. }), "{err:?}");
    }
}
