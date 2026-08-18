//! Durable JSON workspace registry.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use dsh_session::SessionId;
use serde::{Deserialize, Serialize};

use crate::error::WorkspaceError;
use crate::ids::WorkspaceId;

/// Snapshot of one workspace record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceView {
    workspace_id: WorkspaceId,
    path: String,
    title: String,
    session_ids: Vec<SessionId>,
    created_at: String,
    updated_at: String,
}

impl WorkspaceView {
    /// Stable workspace id.
    #[must_use]
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    /// Canonical directory path stored at create.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Sessions accounted under this workspace, in manual order.
    #[must_use]
    pub fn session_ids(&self) -> &[SessionId] {
        &self.session_ids
    }

    /// ISO-8601 creation instant.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }

    /// ISO-8601 last-mutation instant.
    #[must_use]
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }
}

/// `list` snapshot: display order plus the registry-global archive set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListResult {
    items: Vec<WorkspaceView>,
    archived_session_ids: Vec<SessionId>,
}

impl ListResult {
    /// Workspaces in durable display order.
    #[must_use]
    pub fn items(&self) -> &[WorkspaceView] {
        &self.items
    }

    /// Registry-global archived session ids, in archive order.
    #[must_use]
    pub fn archived_session_ids(&self) -> &[SessionId] {
        &self.archived_session_ids
    }
}

/// File-backed workspace registry. Mutations take an in-process [`Mutex`] and write JSON atomically.
pub struct WorkspaceRegistry {
    path: PathBuf,
    lock: Mutex<()>,
}

impl std::fmt::Debug for WorkspaceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceRegistry")
            .field("path", &self.path)
            .finish()
    }
}

impl WorkspaceRegistry {
    /// Persist at `path`. Missing file is an empty registry; the parent is created on write.
    #[must_use]
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    /// Workspaces in display order plus the archive set.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::Corrupt`] when the persist file is not valid registry JSON.
    /// [`WorkspaceError::Io`] when the file cannot be read.
    pub fn list(&self) -> Result<ListResult, WorkspaceError> {
        let _guard = self.lock.lock().expect("workspace registry lock");
        let state = load_state(&self.path)?;
        Ok(list_from_state(&state))
    }

    /// Create or reuse a workspace for an existing directory. Same canonical path is idempotent.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::InvalidPath`] when `path` is missing, not a directory, or cannot be
    /// canonicalized. Persist read/write failures.
    pub fn create(&self, path: &Path) -> Result<(WorkspaceView, bool), WorkspaceError> {
        let canonical = canonical_dir(path)?;
        let _guard = self.lock.lock().expect("workspace registry lock");
        let mut state = load_state(&self.path)?;
        if let Some(existing) = state.find_by_path(&canonical) {
            return Ok((existing, false));
        }
        let id = mint_workspace_id(&state.workspaces);
        let now = utc_now_iso();
        let title = default_title(&canonical);
        let record = WorkspaceRecord {
            path: canonical,
            title,
            session_ids: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        let view = record.to_view(&id);
        state.workspace_ids.insert(0, id);
        state
            .workspaces
            .insert(view.workspace_id.as_str().to_string(), record);
        save_state(&self.path, &state)?;
        Ok((view, true))
    }

    /// Replace a workspace display title. Trimmed empty titles fail; a same-title rename is a no-op.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::NotFound`], [`WorkspaceError::TitleInvalid`],
    /// [`WorkspaceError::NameConflict`], or persist failures.
    pub fn rename(&self, id: &WorkspaceId, title: &str) -> Result<WorkspaceView, WorkspaceError> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(WorkspaceError::TitleInvalid);
        }
        self.mutate(|state| {
            let Some(record) = state.workspaces.get(id.as_str()) else {
                return Err(WorkspaceError::NotFound);
            };
            if record.title == trimmed {
                return Ok(Mutate::Unchanged(record.to_view(id)));
            }
            let conflict = state.workspaces.iter().any(|(other_id, other)| {
                other_id.as_str() != id.as_str() && other.title == trimmed
            });
            if conflict {
                return Err(WorkspaceError::NameConflict);
            }
            let record = state
                .workspaces
                .get_mut(id.as_str())
                .expect("workspace present after lookup");
            record.title = trimmed.to_string();
            record.updated_at = utc_now_iso();
            Ok(Mutate::Changed(record.to_view(id)))
        })
    }

    /// Unregister a workspace. The directory and session logs stay.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::NotFound`] when `id` is unknown. Persist failures.
    pub fn delete(&self, id: &WorkspaceId) -> Result<(), WorkspaceError> {
        self.mutate(|state| {
            if state.workspaces.remove(id.as_str()).is_none() {
                return Err(WorkspaceError::NotFound);
            }
            state
                .workspace_ids
                .retain(|item| item.as_str() != id.as_str());
            Ok(Mutate::Changed(()))
        })
    }

    /// Move `id` in display order (DOM `insertBefore`). Omitted `before` appends.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::NotFound`] when `id` or `before` is unknown. Persist failures.
    pub fn insert_before(
        &self,
        id: &WorkspaceId,
        before: Option<&WorkspaceId>,
    ) -> Result<Vec<WorkspaceId>, WorkspaceError> {
        self.mutate(|state| {
            let next =
                insert_before_ids(&state.workspace_ids, id, before, WorkspaceError::NotFound)?;
            if ids_equal(&next, &state.workspace_ids) {
                return Ok(Mutate::Unchanged(next));
            }
            state.workspace_ids = next.clone();
            Ok(Mutate::Changed(next))
        })
    }

    /// Move an accounted session in that workspace (DOM `insertBefore`). Omitted `before` appends.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::NotFound`] when the workspace is unknown.
    /// [`WorkspaceError::MoveInvalid`] when `session_id` or `before` is not on that workspace.
    /// Persist failures.
    pub fn insert_session_before(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        before: Option<&SessionId>,
    ) -> Result<WorkspaceView, WorkspaceError> {
        self.mutate(|state| {
            let Some(record) = state.workspaces.get(workspace_id.as_str()) else {
                return Err(WorkspaceError::NotFound);
            };
            let next = insert_before_sessions(
                &record.session_ids,
                session_id,
                before,
                WorkspaceError::MoveInvalid,
            )?;
            if sessions_equal(&next, &record.session_ids) {
                return Ok(Mutate::Unchanged(record.to_view(workspace_id)));
            }
            let record = state
                .workspaces
                .get_mut(workspace_id.as_str())
                .expect("workspace present after lookup");
            record.session_ids = next;
            record.updated_at = utc_now_iso();
            Ok(Mutate::Changed(record.to_view(workspace_id)))
        })
    }

    /// Append `session_id` to the registry-global archive set. Already archived is a no-op.
    ///
    /// # Errors
    ///
    /// Persist read/write failures.
    pub fn archive_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<SessionId>, WorkspaceError> {
        self.mutate(|state| {
            if state
                .archived_session_ids
                .iter()
                .any(|item| item.as_str() == session_id.as_str())
            {
                return Ok(Mutate::Unchanged(state.archived_session_ids.clone()));
            }
            state.archived_session_ids.push(session_id.clone());
            Ok(Mutate::Changed(state.archived_session_ids.clone()))
        })
    }

    /// Prepend `session_id` to a workspace account. Already present is a no-op.
    ///
    /// # Errors
    ///
    /// [`WorkspaceError::NotFound`] when `workspace_id` is unknown. Persist failures.
    pub fn attach_session(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
    ) -> Result<WorkspaceView, WorkspaceError> {
        self.mutate(|state| {
            let Some(record) = state.workspaces.get(workspace_id.as_str()) else {
                return Err(WorkspaceError::NotFound);
            };
            if record
                .session_ids
                .iter()
                .any(|item| item.as_str() == session_id.as_str())
            {
                return Ok(Mutate::Unchanged(record.to_view(workspace_id)));
            }
            let record = state
                .workspaces
                .get_mut(workspace_id.as_str())
                .expect("workspace present after lookup");
            record.session_ids.insert(0, session_id.clone());
            record.updated_at = utc_now_iso();
            Ok(Mutate::Changed(record.to_view(workspace_id)))
        })
    }

    fn mutate<T>(
        &self,
        op: impl FnOnce(&mut RegistryState) -> Result<Mutate<T>, WorkspaceError>,
    ) -> Result<T, WorkspaceError> {
        let _guard = self.lock.lock().expect("workspace registry lock");
        let mut state = load_state(&self.path)?;
        match op(&mut state)? {
            Mutate::Unchanged(value) => Ok(value),
            Mutate::Changed(value) => {
                save_state(&self.path, &state)?;
                Ok(value)
            }
        }
    }
}

enum Mutate<T> {
    Unchanged(T),
    Changed(T),
}

/// `{dsh_home}/workspaces.json` when `dsh_home` is non-empty, else `{dsh_session_root}/workspaces.json`.
///
/// # Errors
///
/// [`WorkspaceError::MissingPersistPath`] when both values are missing or empty.
pub fn default_persist_path(
    dsh_home: Option<&str>,
    dsh_session_root: Option<&str>,
) -> Result<PathBuf, WorkspaceError> {
    if let Some(home) = dsh_home {
        if !home.is_empty() {
            return Ok(PathBuf::from(home).join("workspaces.json"));
        }
    }
    if let Some(root) = dsh_session_root {
        if !root.is_empty() {
            return Ok(PathBuf::from(root).join("workspaces.json"));
        }
    }
    Err(WorkspaceError::MissingPersistPath)
}

/// Create the persist file's parent directory.
pub(crate) fn ensure_persist_parent(path: &Path) -> Result<(), WorkspaceError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|error| WorkspaceError::Io(error.to_string()))
}

#[derive(Clone, Debug)]
struct RegistryState {
    workspace_ids: Vec<WorkspaceId>,
    archived_session_ids: Vec<SessionId>,
    workspaces: BTreeMap<String, WorkspaceRecord>,
}

impl RegistryState {
    fn empty() -> Self {
        Self {
            workspace_ids: Vec::new(),
            archived_session_ids: Vec::new(),
            workspaces: BTreeMap::new(),
        }
    }

    fn find_by_path(&self, path: &str) -> Option<WorkspaceView> {
        for id in &self.workspace_ids {
            if let Some(record) = self.workspaces.get(id.as_str()) {
                if record.path == path {
                    return Some(record.to_view(id));
                }
            }
        }
        None
    }
}

#[derive(Clone, Debug)]
struct WorkspaceRecord {
    path: String,
    title: String,
    session_ids: Vec<SessionId>,
    created_at: String,
    updated_at: String,
}

impl WorkspaceRecord {
    fn to_view(&self, id: &WorkspaceId) -> WorkspaceView {
        WorkspaceView {
            workspace_id: id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            session_ids: self.session_ids.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistFile {
    #[serde(default)]
    workspace_ids: Vec<String>,
    #[serde(default)]
    archived_session_ids: Vec<String>,
    #[serde(default)]
    workspaces: BTreeMap<String, PersistWorkspace>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistWorkspace {
    path: String,
    title: String,
    #[serde(default)]
    session_ids: Vec<String>,
    created_at: String,
    updated_at: String,
}

fn list_from_state(state: &RegistryState) -> ListResult {
    let items = state
        .workspace_ids
        .iter()
        .map(|id| {
            state
                .workspaces
                .get(id.as_str())
                .expect("order id present in map")
                .to_view(id)
        })
        .collect();
    ListResult {
        items,
        archived_session_ids: state.archived_session_ids.clone(),
    }
}

fn load_state(path: &Path) -> Result<RegistryState, WorkspaceError> {
    match fs::read_to_string(path) {
        Ok(text) => parse_state(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RegistryState::empty()),
        Err(error) => Err(WorkspaceError::Io(error.to_string())),
    }
}

fn parse_state(text: &str) -> Result<RegistryState, WorkspaceError> {
    let file: PersistFile =
        serde_json::from_str(text).map_err(|error| WorkspaceError::Corrupt(error.to_string()))?;
    let mut workspaces = BTreeMap::new();
    for (id, record) in file.workspaces {
        workspaces.insert(
            id,
            WorkspaceRecord {
                path: record.path,
                title: record.title,
                session_ids: record.session_ids.into_iter().map(SessionId::new).collect(),
                created_at: record.created_at,
                updated_at: record.updated_at,
            },
        );
    }
    let workspace_ids: Vec<WorkspaceId> = file
        .workspace_ids
        .into_iter()
        .map(WorkspaceId::new)
        .collect();
    validate_state(&workspace_ids, &workspaces)?;
    Ok(RegistryState {
        workspace_ids,
        archived_session_ids: file
            .archived_session_ids
            .into_iter()
            .map(SessionId::new)
            .collect(),
        workspaces,
    })
}

fn validate_state(
    workspace_ids: &[WorkspaceId],
    workspaces: &BTreeMap<String, WorkspaceRecord>,
) -> Result<(), WorkspaceError> {
    let mut seen = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for id in workspace_ids {
        if seen.insert(id.as_str().to_string(), ()).is_some() {
            return Err(WorkspaceError::Corrupt(format!(
                "registry order repeats workspace '{}'",
                id.as_str()
            )));
        }
        let Some(record) = workspaces.get(id.as_str()) else {
            return Err(WorkspaceError::Corrupt(format!(
                "registry order references missing workspace '{}'",
                id.as_str()
            )));
        };
        if let Some(holder) = paths.insert(record.path.clone(), id.as_str().to_string()) {
            return Err(WorkspaceError::Corrupt(format!(
                "path '{}' is claimed by both workspace '{}' and workspace '{}'",
                record.path,
                holder,
                id.as_str()
            )));
        }
    }
    for id in workspaces.keys() {
        if !seen.contains_key(id) {
            return Err(WorkspaceError::Corrupt(format!(
                "workspace '{id}' is absent from registry order"
            )));
        }
    }
    Ok(())
}

fn save_state(path: &Path, state: &RegistryState) -> Result<(), WorkspaceError> {
    let file = PersistFile {
        workspace_ids: state
            .workspace_ids
            .iter()
            .map(|id| id.as_str().to_string())
            .collect(),
        archived_session_ids: state
            .archived_session_ids
            .iter()
            .map(|id| id.as_str().to_string())
            .collect(),
        workspaces: state
            .workspaces
            .iter()
            .map(|(id, record)| {
                (
                    id.clone(),
                    PersistWorkspace {
                        path: record.path.clone(),
                        title: record.title.clone(),
                        session_ids: record
                            .session_ids
                            .iter()
                            .map(|id| id.as_str().to_string())
                            .collect(),
                        created_at: record.created_at.clone(),
                        updated_at: record.updated_at.clone(),
                    },
                )
            })
            .collect(),
    };
    let bytes =
        serde_json::to_vec(&file).map_err(|error| WorkspaceError::Corrupt(error.to_string()))?;
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    ensure_persist_parent(path)?;
    let tmp = tmp_path(path)?;
    if let Err(error) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(WorkspaceError::Io(error.to_string()));
    }
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(WorkspaceError::Io(error.to_string()));
    }
    Ok(())
}

fn tmp_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    let Some(name) = path.file_name() else {
        return Err(WorkspaceError::Io(
            "workspace persist path has no file name".into(),
        ));
    };
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.join(tmp_name)),
        _ => Ok(PathBuf::from(tmp_name)),
    }
}

fn canonical_dir(path: &Path) -> Result<String, WorkspaceError> {
    let canonical = fs::canonicalize(path).map_err(|_| WorkspaceError::InvalidPath)?;
    let meta = fs::metadata(&canonical).map_err(|_| WorkspaceError::InvalidPath)?;
    if !meta.is_dir() {
        return Err(WorkspaceError::InvalidPath);
    }
    match canonical.to_str() {
        Some(text) => Ok(text.to_string()),
        None => Err(WorkspaceError::InvalidPath),
    }
}

fn default_title(canonical: &str) -> String {
    Path::new(canonical)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| canonical.to_string())
}

fn mint_workspace_id(used: &BTreeMap<String, WorkspaceRecord>) -> WorkspaceId {
    let pid = std::process::id();
    let mut nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    loop {
        let raw = format!("wk-{pid}-{nanos}");
        if !used.contains_key(&raw) {
            return WorkspaceId::new(raw);
        }
        nanos += 1;
    }
}

fn utc_now_iso() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_iso8601_millis(duration.as_secs(), duration.subsec_millis())
}

fn format_iso8601_millis(secs: u64, millis: u32) -> String {
    let (year, month, day, hour, minute, second) = civil_from_unix(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_unix(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;
    let second = (rem % 60) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, minute, second)
}

fn civil_from_days(mut days: u64) -> (i32, u32, u32) {
    let mut year = 1970i32;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days {
            break;
        }
        days -= year_days;
        year += 1;
    }
    let mut month = 1u32;
    loop {
        let mut month_days = DAYS_IN_MONTH[(month - 1) as usize];
        if month == 2 && is_leap(year) {
            month_days = 29;
        }
        if days < month_days {
            break;
        }
        days -= month_days;
        month += 1;
    }
    (year, month, (days + 1) as u32)
}

const DAYS_IN_MONTH: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn insert_before_ids(
    order: &[WorkspaceId],
    id: &WorkspaceId,
    before: Option<&WorkspaceId>,
    missing: WorkspaceError,
) -> Result<Vec<WorkspaceId>, WorkspaceError> {
    if !order.iter().any(|item| item.as_str() == id.as_str()) {
        return Err(missing);
    }
    if let Some(anchor) = before {
        if !order.iter().any(|item| item.as_str() == anchor.as_str()) {
            return Err(WorkspaceError::NotFound);
        }
        if anchor.as_str() == id.as_str() {
            return Ok(order.to_vec());
        }
    }
    let mut next: Vec<WorkspaceId> = order
        .iter()
        .filter(|item| item.as_str() != id.as_str())
        .cloned()
        .collect();
    match before {
        None => next.push(id.clone()),
        Some(anchor) => {
            let at = next
                .iter()
                .position(|item| item.as_str() == anchor.as_str())
                .expect("anchor present after membership check");
            next.insert(at, id.clone());
        }
    }
    Ok(next)
}

fn insert_before_sessions(
    order: &[SessionId],
    id: &SessionId,
    before: Option<&SessionId>,
    missing: WorkspaceError,
) -> Result<Vec<SessionId>, WorkspaceError> {
    if !order.iter().any(|item| item.as_str() == id.as_str()) {
        return Err(missing);
    }
    if let Some(anchor) = before {
        if !order.iter().any(|item| item.as_str() == anchor.as_str()) {
            return Err(WorkspaceError::MoveInvalid);
        }
        if anchor.as_str() == id.as_str() {
            return Ok(order.to_vec());
        }
    }
    let mut next: Vec<SessionId> = order
        .iter()
        .filter(|item| item.as_str() != id.as_str())
        .cloned()
        .collect();
    match before {
        None => next.push(id.clone()),
        Some(anchor) => {
            let at = next
                .iter()
                .position(|item| item.as_str() == anchor.as_str())
                .expect("anchor present after membership check");
            next.insert(at, id.clone());
        }
    }
    Ok(next)
}

fn ids_equal(left: &[WorkspaceId], right: &[WorkspaceId]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.as_str() == b.as_str())
}

fn sessions_equal(left: &[SessionId], right: &[SessionId]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.as_str() == b.as_str())
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceRegistry, default_persist_path, format_iso8601_millis, utc_now_iso};
    use crate::WorkspaceError;
    use dsh_session::SessionId;

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

    #[test]
    fn create_requires_existing_directory() {
        let dir = test_temp_dir("ws");
        let reg = WorkspaceRegistry::with_path(dir.join("workspaces.json"));
        let missing = dir.join("nope");
        let err = reg.create(&missing).unwrap_err();
        assert_eq!(err.code(), "workspace-invalid-path");
    }

    #[test]
    fn create_is_idempotent_on_realpath() {
        let dir = test_temp_dir("ws2");
        let cwd = dir.join("proj");
        std::fs::create_dir(&cwd).unwrap();
        let reg = WorkspaceRegistry::with_path(dir.join("workspaces.json"));
        let (a, created_a) = reg.create(&cwd).unwrap();
        let (b, created_b) = reg.create(&cwd).unwrap();
        assert!(created_a);
        assert!(!created_b);
        assert_eq!(a.workspace_id().as_str(), b.workspace_id().as_str());
    }

    #[test]
    fn archive_keeps_session_in_account() {
        let dir = test_temp_dir("ws3");
        let cwd = dir.join("proj");
        std::fs::create_dir(&cwd).unwrap();
        let reg = WorkspaceRegistry::with_path(dir.join("workspaces.json"));
        let (ws, _) = reg.create(&cwd).unwrap();
        let sid = dsh_session::SessionId::new("s1");
        reg.attach_session(ws.workspace_id(), &sid).unwrap();
        let archived = reg.archive_session(&sid).unwrap();
        assert_eq!(
            archived.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            vec!["s1"]
        );
        let list = reg.list().unwrap();
        assert!(
            list.items()[0]
                .session_ids()
                .iter()
                .any(|s| s.as_str() == "s1")
        );
        assert!(
            list.archived_session_ids()
                .iter()
                .any(|s| s.as_str() == "s1")
        );
    }

    #[test]
    fn default_persist_path_prefers_dsh_home() {
        let home = default_persist_path(Some("/home/dsh"), Some("/sessions")).unwrap();
        assert_eq!(home, std::path::PathBuf::from("/home/dsh/workspaces.json"));
        let session = default_persist_path(None, Some("/sessions")).unwrap();
        assert_eq!(
            session,
            std::path::PathBuf::from("/sessions/workspaces.json")
        );
        let skipped_empty = default_persist_path(Some(""), Some("/sessions")).unwrap();
        assert_eq!(
            skipped_empty,
            std::path::PathBuf::from("/sessions/workspaces.json")
        );
        let err = default_persist_path(None, None).unwrap_err();
        assert_eq!(err.code(), "internal");
        assert!(matches!(err, WorkspaceError::MissingPersistPath));
    }

    #[test]
    fn create_prepends_and_persists_camel_case_json() {
        let dir = test_temp_dir("ws-order");
        let first = dir.join("alpha");
        let second = dir.join("beta");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let persist = dir.join("workspaces.json");
        let reg = WorkspaceRegistry::with_path(persist.clone());
        let (a, _) = reg.create(&first).unwrap();
        let (b, _) = reg.create(&second).unwrap();
        let list = reg.list().unwrap();
        assert_eq!(
            list.items()[0].workspace_id().as_str(),
            b.workspace_id().as_str()
        );
        assert_eq!(
            list.items()[1].workspace_id().as_str(),
            a.workspace_id().as_str()
        );
        assert_eq!(list.items()[0].title(), "beta");
        let text = std::fs::read_to_string(&persist).unwrap();
        assert!(text.contains("workspaceIds"));
        assert!(text.contains("archivedSessionIds"));
        assert!(text.contains("sessionIds"));
        assert!(text.contains("createdAt"));
        let reopened = WorkspaceRegistry::with_path(persist);
        let again = reopened.list().unwrap();
        assert_eq!(
            again.items()[0].workspace_id().as_str(),
            b.workspace_id().as_str()
        );
    }

    #[test]
    fn rename_delete_and_order_errors() {
        let dir = test_temp_dir("ws-mut");
        let one = dir.join("one");
        let two = dir.join("two");
        std::fs::create_dir(&one).unwrap();
        std::fs::create_dir(&two).unwrap();
        let reg = WorkspaceRegistry::with_path(dir.join("workspaces.json"));
        let (a, _) = reg.create(&one).unwrap();
        let (b, _) = reg.create(&two).unwrap();
        let empty = reg.rename(a.workspace_id(), "  ").unwrap_err();
        assert_eq!(empty.code(), "title-invalid");
        let conflict = reg.rename(a.workspace_id(), b.title()).unwrap_err();
        assert_eq!(conflict.code(), "workspace-name-conflict");
        let same = reg.rename(a.workspace_id(), a.title()).unwrap();
        assert_eq!(same.updated_at(), a.updated_at());
        let renamed = reg.rename(a.workspace_id(), "  gamma  ").unwrap();
        assert_eq!(renamed.title(), "gamma");
        let unknown = WorkspaceError::NotFound;
        let missing = crate::WorkspaceId::new("wk-missing");
        assert_eq!(reg.delete(&missing).unwrap_err().code(), unknown.code());
        let appended = reg.insert_before(b.workspace_id(), None).unwrap();
        assert_eq!(appended.last().unwrap().as_str(), b.workspace_id().as_str());
        let missing_anchor = reg
            .insert_before(a.workspace_id(), Some(&missing))
            .unwrap_err();
        assert_eq!(missing_anchor.code(), "workspace-not-found");
        reg.delete(a.workspace_id()).unwrap();
        assert_eq!(reg.list().unwrap().items().len(), 1);
        assert_eq!(
            reg.delete(&missing).unwrap_err().code(),
            "workspace-not-found"
        );
    }

    #[test]
    fn insert_session_before_and_attach_idempotent() {
        let dir = test_temp_dir("ws-sess");
        let cwd = dir.join("proj");
        std::fs::create_dir(&cwd).unwrap();
        let reg = WorkspaceRegistry::with_path(dir.join("workspaces.json"));
        let (ws, _) = reg.create(&cwd).unwrap();
        let s1 = SessionId::new("s1");
        let s2 = SessionId::new("s2");
        reg.attach_session(ws.workspace_id(), &s1).unwrap();
        reg.attach_session(ws.workspace_id(), &s2).unwrap();
        let again = reg.attach_session(ws.workspace_id(), &s2).unwrap();
        assert_eq!(
            again
                .session_ids()
                .iter()
                .map(SessionId::as_str)
                .collect::<Vec<_>>(),
            vec!["s2", "s1"]
        );
        let moved = reg
            .insert_session_before(ws.workspace_id(), &s2, None)
            .unwrap();
        assert_eq!(
            moved
                .session_ids()
                .iter()
                .map(SessionId::as_str)
                .collect::<Vec<_>>(),
            vec!["s1", "s2"]
        );
        let unknown_ws = crate::WorkspaceId::new("wk-missing");
        assert_eq!(
            reg.insert_session_before(&unknown_ws, &s1, None)
                .unwrap_err()
                .code(),
            "workspace-not-found"
        );
        let ghost = SessionId::new("ghost");
        assert_eq!(
            reg.insert_session_before(ws.workspace_id(), &ghost, None)
                .unwrap_err()
                .code(),
            "workspace-move-invalid"
        );
        let archived = reg.archive_session(&s1).unwrap();
        let again_archive = reg.archive_session(&s1).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(again_archive.len(), 1);
    }

    #[test]
    fn corrupt_json_fails_list() {
        let dir = test_temp_dir("ws-bad");
        let persist = dir.join("workspaces.json");
        std::fs::write(&persist, b"not-json").unwrap();
        let reg = WorkspaceRegistry::with_path(persist);
        let err = reg.list().unwrap_err();
        assert_eq!(err.code(), "internal");
        assert!(matches!(err, WorkspaceError::Corrupt(_)));
    }

    #[test]
    fn iso8601_epoch_zero() {
        assert_eq!(format_iso8601_millis(0, 0), "1970-01-01T00:00:00.000Z");
        let now = utc_now_iso();
        assert!(now.ends_with('Z'));
        assert_eq!(now.len(), "1970-01-01T00:00:00.000Z".len());
    }
}
