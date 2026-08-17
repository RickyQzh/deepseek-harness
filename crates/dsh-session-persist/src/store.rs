//! Live uncompressed JSONL session store.

use std::path::{Path, PathBuf};

use dsh_session::{Session, SessionId};

use crate::PersistError;
use crate::jsonl::encode_session_log;

/// Directory-backed JSONL store. One file per session, rewritten on flush.
#[derive(Debug)]
pub struct JsonlSessionStore {
    root: PathBuf,
}

impl JsonlSessionStore {
    /// `DSH_SESSION_ROOT` if set and non-empty, else `{DSH_HOME}/sessions`.
    ///
    /// # Errors
    ///
    /// [`PersistError::Io`] when neither variable is set.
    pub fn from_env() -> Result<Self, PersistError> {
        if let Ok(root) = std::env::var("DSH_SESSION_ROOT") {
            if !root.is_empty() {
                return Ok(Self::with_root(root));
            }
        }
        match std::env::var("DSH_HOME") {
            Ok(home) if !home.is_empty() => {
                Ok(Self::with_root(PathBuf::from(home).join("sessions")))
            }
            _ => Err(PersistError::Io(
                "DSH_SESSION_ROOT or DSH_HOME must be set for JSONL persistence".into(),
            )),
        }
    }

    /// Use `root` as the sessions directory.
    #[must_use]
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Sessions directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `{root}/{id}/session.jsonl`.
    #[must_use]
    pub fn path_for(&self, id: &SessionId) -> PathBuf {
        self.root.join(id.as_str()).join("session.jsonl")
    }

    /// Encode the live session as uncompressed JSONL and replace the file.
    ///
    /// # Errors
    ///
    /// [`PersistError::Corrupt`] on encode failure; [`PersistError::Io`] on create/write.
    pub fn flush(&self, session: &Session) -> Result<(), PersistError> {
        let path = self.path_for(session.id());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| PersistError::Io(error.to_string()))?;
        }
        let text = encode_session_log(session.header(), session.events(), false)?;
        std::fs::write(&path, text).map_err(|error| PersistError::Io(error.to_string()))
    }

    /// Session ids that have a `{root}/{id}/session.jsonl` file.
    ///
    /// Missing `root` is an empty list. Directory names without that file are skipped.
    ///
    /// # Errors
    ///
    /// [`PersistError::Io`] when `root` exists but cannot be read.
    pub fn list_ids(&self) -> Result<Vec<SessionId>, PersistError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(PersistError::Io(error.to_string())),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| PersistError::Io(error.to_string()))?;
            let name = entry.file_name();
            let jsonl = self.root.join(&name).join("session.jsonl");
            if !jsonl.is_file() {
                continue;
            }
            let Some(id) = name.to_str() else {
                continue;
            };
            ids.push(SessionId::new(id.to_string()));
        }
        Ok(ids)
    }

    /// Read uncompressed JSONL from [`Self::path_for`] and rebuild the session surface.
    ///
    /// # Errors
    ///
    /// [`PersistError::Io`] when the file cannot be read.
    /// [`PersistError::Corrupt`], [`PersistError::Format`], or [`PersistError::Session`]
    /// from [`decode_session_log`] / [`Session::from_events`].
    pub fn load(&self, id: &SessionId) -> Result<Session, PersistError> {
        let path = self.path_for(id);
        let text =
            std::fs::read_to_string(&path).map_err(|error| PersistError::Io(error.to_string()))?;
        let (header, events) = crate::decode_session_log(&text)?;
        Ok(Session::from_events(header, events)?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::JsonlSessionStore;
    use crate::decode_session_log;
    use dsh_session::{
        ContentBlock, Message, MessageId, MessageRole, MessageSource, SESSION_FORMAT_VERSION,
        Session, SessionEvent, SessionHeader, SessionId, SurfaceOp,
    };

    // Process-global env; Cargo's default harness runs tests in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_temp_dir(prefix: &str) -> std::path::PathBuf {
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

    fn header(id: &str) -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new(id),
            created_at: 1,
            cwd: Some("/work".into()),
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    #[test]
    fn flush_writes_uncompressed_jsonl_under_session_id_dir() {
        let root = test_temp_dir("dsh-store");
        let store = JsonlSessionStore::with_root(&root);
        let mut session = Session::new(header("sdk-snapshot-text"));
        session
            .append(SessionEvent::UserMessage {
                seq: 0,
                time: 0,
                data: Message {
                    id: MessageId::new("m0"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text { text: "hi".into() }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            })
            .unwrap();
        store.flush(&session).unwrap();
        let path = store.path_for(&SessionId::new("sdk-snapshot-text"));
        assert_eq!(path, root.join("sdk-snapshot-text").join("session.jsonl"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.starts_with('\u{28}'), "must not be zstd");
        assert!(
            text.contains("\"type\":\"session\"")
                || text.contains("\"type\": \"session\"")
                || text.contains("session")
        );
        let (decoded_header, events) = decode_session_log(&text).unwrap();
        assert_eq!(decoded_header.id.as_str(), "sdk-snapshot-text");
        assert_eq!(events.len(), 1);
        store.flush(&session).unwrap();
        let text2 = std::fs::read_to_string(&path).unwrap();
        let (_, events2) = decode_session_log(&text2).unwrap();
        assert_eq!(events2.len(), 1);
    }

    fn session_with_id(id: &str) -> Session {
        let mut session = Session::new(header(id));
        session
            .append(SessionEvent::UserMessage {
                seq: 0,
                time: 0,
                data: Message {
                    id: MessageId::new("m0"),
                    role: MessageRole::User,
                    content: vec![ContentBlock::Text { text: "hi".into() }],
                    source: MessageSource::User,
                },
                surface_op: Some(SurfaceOp::Append),
                source_event_seqs: None,
                ignorable: None,
            })
            .unwrap();
        session
    }

    #[test]
    fn list_ids_scans_session_jsonl_directories() {
        let dir = test_temp_dir("list-ids");
        let store = JsonlSessionStore::with_root(&dir);
        store.flush(&session_with_id("alpha")).unwrap();
        store.flush(&session_with_id("beta")).unwrap();
        std::fs::create_dir_all(dir.join("ghost")).unwrap();
        std::fs::write(dir.join("not-a-session"), b"x").unwrap();
        let mut ids: Vec<String> = store
            .list_ids()
            .unwrap()
            .into_iter()
            .map(|id| id.into_inner())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);
        let missing = JsonlSessionStore::with_root(dir.join("no-such-root"));
        assert!(missing.list_ids().unwrap().is_empty());
    }

    #[test]
    fn load_round_trips_uncompressed_jsonl() {
        let dir = test_temp_dir("load");
        let store = JsonlSessionStore::with_root(&dir);
        let session = session_with_id("workspace-context-resume");
        store.flush(&session).unwrap();
        let loaded = store
            .load(&SessionId::new("workspace-context-resume"))
            .unwrap();
        assert_eq!(loaded.events().len(), session.events().len());
    }

    #[test]
    fn from_env_prefers_dsh_session_root() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let root = test_temp_dir("dsh-store-env");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_home = std::env::var("DSH_HOME").ok();
        unsafe {
            std::env::set_var("DSH_SESSION_ROOT", root.as_os_str());
            std::env::set_var("DSH_HOME", "/tmp/should-not-use");
        }
        let store = JsonlSessionStore::from_env().unwrap();
        assert_eq!(store.root(), root.as_path());
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
    }

    #[test]
    fn from_env_uses_dsh_home_sessions_when_root_unset() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let home = test_temp_dir("dsh-home");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_home = std::env::var("DSH_HOME").ok();
        unsafe {
            std::env::remove_var("DSH_SESSION_ROOT");
            std::env::set_var("DSH_HOME", home.as_os_str());
        }
        let store = JsonlSessionStore::from_env().unwrap();
        assert_eq!(store.root(), home.join("sessions").as_path());
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
    }

    #[test]
    fn from_env_fails_when_neither_root_nor_home_is_set() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous_root = std::env::var("DSH_SESSION_ROOT").ok();
        let previous_home = std::env::var("DSH_HOME").ok();
        unsafe {
            std::env::remove_var("DSH_SESSION_ROOT");
            std::env::remove_var("DSH_HOME");
        }
        let err = JsonlSessionStore::from_env().expect_err("missing");
        assert!(err.to_string().contains("DSH_SESSION_ROOT or DSH_HOME"));
        match previous_root {
            Some(value) => unsafe { std::env::set_var("DSH_SESSION_ROOT", value) },
            None => unsafe { std::env::remove_var("DSH_SESSION_ROOT") },
        }
        match previous_home {
            Some(value) => unsafe { std::env::set_var("DSH_HOME", value) },
            None => unsafe { std::env::remove_var("DSH_HOME") },
        }
    }
}
