//! Preset table, pin-at-create, and knob folds.

use dsh_sandbox::SandboxMode;
use dsh_session::{
    LogEvent, PermissionPresetData, SandboxModeData, Session, SessionError, SessionEvent,
};
use dsh_user_approval::{ApprovalPolicy, effective_approval_policy, set_approval_policy};
use serde_json::Value;

/// Returned when effective knobs match no table entry. Never a table key or event payload.
pub const CUSTOM_PRESET: &str = "custom";

/// One preset's sandbox/approval bundle and optional client labels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetSpec {
    sandbox: SandboxMode,
    approval: ApprovalPolicy,
    name: Option<String>,
    description: Option<String>,
}

impl PresetSpec {
    /// Bundle `sandbox` with `approval`.
    #[must_use]
    pub fn new(sandbox: SandboxMode, approval: ApprovalPolicy) -> Self {
        Self {
            sandbox,
            approval,
            name: None,
            description: None,
        }
    }
}

/// YAML plugin config: optional preset table and optional new-session default.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PermissionPresetConfig {
    /// Ordered table. `None` uses the two-row built-in table (no `read-only`).
    pub presets: Option<Vec<(String, PresetSpec)>>,
    /// New-session default. `None` infers the unique table row matching composed knobs.
    pub default_preset: Option<String>,
}

impl PermissionPresetConfig {
    /// Three-row dsh-base table: `read-only`, `workspace-write`, `danger-full-access`.
    #[must_use]
    pub fn dsh_base() -> Self {
        Self {
            presets: Some(vec![
                (
                    "read-only".into(),
                    PresetSpec::new(SandboxMode::ReadOnly, ApprovalPolicy::Ask),
                ),
                (
                    "workspace-write".into(),
                    PresetSpec::new(SandboxMode::WorkspaceWrite, ApprovalPolicy::Ask),
                ),
                (
                    "danger-full-access".into(),
                    PresetSpec::new(SandboxMode::DangerFullAccess, ApprovalPolicy::Never),
                ),
            ]),
            default_preset: None,
        }
    }

    /// Parse interpolated YAML `{ presets?, defaultPreset? }`.
    ///
    /// # Errors
    ///
    /// [`PermissionError::InvalidConfig`] when the value is not a mapping, `presets` is not a
    /// mapping of objects, a row omits `sandbox`/`approval` or uses an unknown spelling, or
    /// `defaultPreset` is not a string.
    pub fn from_value(value: &Value) -> Result<Self, PermissionError> {
        match value {
            Value::Null => Ok(Self::default()),
            Value::Object(map) => {
                let presets = match map.get("presets") {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(rows)) => {
                        let mut presets = Vec::new();
                        for (name, spec) in rows {
                            presets.push((name.clone(), parse_spec(spec)?));
                        }
                        Some(presets)
                    }
                    Some(other) => {
                        return Err(PermissionError::InvalidConfig(format!(
                            "presets must be a mapping, got {other}"
                        )));
                    }
                };
                let default_preset = match map.get("defaultPreset") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(name)) => Some(name.clone()),
                    Some(other) => {
                        return Err(PermissionError::InvalidConfig(format!(
                            "defaultPreset must be a string, got {other}"
                        )));
                    }
                };
                Ok(Self {
                    presets,
                    default_preset,
                })
            }
            other => Err(PermissionError::InvalidConfig(format!(
                "permission config must be a mapping, got {other}"
            ))),
        }
    }
}

/// Failures from loading the preset table or pinning knobs into a session.
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    /// `defaultPreset` is not a table key.
    #[error("unknown preset \"{name}\" (known: {known})")]
    UnknownPreset {
        /// Requested preset name.
        name: String,
        /// Table keys in declaration order.
        known: String,
    },
    /// Omitted `defaultPreset` and no table row matches composed sandbox+approval.
    #[error(
        "composed sandbox and approval defaults match no preset; configure defaultPreset explicitly"
    )]
    NoMatchingDefault,
    /// Table key [`CUSTOM_PRESET`] is reserved.
    #[error(
        "\"{CUSTOM_PRESET}\" is reserved for the derived not-a-preset state and cannot name a table entry"
    )]
    CustomReserved,
    /// YAML types or enum spellings are invalid.
    #[error("{0}")]
    InvalidConfig(String),
    /// Session rejected a knob append.
    #[error(transparent)]
    Session(#[from] SessionError),
}

/// Deployment preset table plus the composed sandbox/approval used to fill missing knobs.
#[derive(Clone, Debug)]
pub struct PermissionPresetService {
    presets: Vec<(String, PresetSpec)>,
    default_preset: String,
    composed_sandbox: SandboxMode,
    composed_approval: ApprovalPolicy,
}

impl PermissionPresetService {
    /// Load the table and resolve `defaultPreset`.
    ///
    /// # Errors
    ///
    /// [`PermissionError::CustomReserved`] when a table key is [`CUSTOM_PRESET`].
    /// [`PermissionError::UnknownPreset`] when `defaultPreset` is not a table key.
    /// [`PermissionError::NoMatchingDefault`] when `defaultPreset` is omitted and no row matches
    /// `composed_sandbox` + `composed_approval`.
    pub fn from_config(
        config: PermissionPresetConfig,
        composed_sandbox: SandboxMode,
        composed_approval: ApprovalPolicy,
    ) -> Result<Self, PermissionError> {
        let presets = match config.presets {
            Some(presets) => presets,
            None => builtin_two(),
        };
        for (name, _) in &presets {
            if name == CUSTOM_PRESET {
                return Err(PermissionError::CustomReserved);
            }
        }
        let default_preset = match config.default_preset {
            Some(name) => {
                if !presets.iter().any(|(key, _)| key == &name) {
                    return Err(PermissionError::UnknownPreset {
                        name,
                        known: known_names(&presets),
                    });
                }
                name
            }
            None => match first_match(&presets, composed_sandbox, composed_approval) {
                Some(name) => name,
                None => return Err(PermissionError::NoMatchingDefault),
            },
        };
        Ok(Self {
            presets,
            default_preset,
            composed_sandbox,
            composed_approval,
        })
    }

    /// Pin missing permission knobs. Fresh unseeded sessions receive `defaultPreset` and its bundle.
    ///
    /// Seeded or partial logs keep present knobs and receive only the missing ones. Composed
    /// sandbox/approval fill missing mechanism knobs; a missing preset event is written only when
    /// the derived name is not [`CUSTOM_PRESET`].
    ///
    /// # Errors
    ///
    /// [`PermissionError::Session`] when an append is rejected.
    pub fn pin_initial(&self, session: &mut Session) -> Result<(), PermissionError> {
        let selected = effective_permission_preset(session.events());
        let sandbox = effective_sandbox_mode(session.events());
        let approval = effective_approval_policy(session.events());
        let seeded = is_seeded(session);
        if selected.is_none() && sandbox.is_none() && approval.is_none() && !seeded {
            let spec = self.spec(&self.default_preset);
            append_preset(session, &self.default_preset)?;
            set_sandbox_mode(session, spec.sandbox)?;
            set_approval_policy(session, spec.approval)?;
            return Ok(());
        }
        let effective = self.derive(selected.as_deref(), sandbox, approval);
        if selected.is_none() && effective != CUSTOM_PRESET {
            append_preset(session, &effective)?;
        }
        if sandbox.is_none() {
            set_sandbox_mode(session, self.composed_sandbox)?;
        }
        if approval.is_none() {
            set_approval_policy(session, self.composed_approval)?;
        }
        Ok(())
    }

    /// Preset matching the effective knobs, or [`CUSTOM_PRESET`] when none match.
    #[must_use]
    pub fn current(&self, events: &[LogEvent]) -> String {
        self.derive(
            effective_permission_preset(events).as_deref(),
            effective_sandbox_mode(events),
            effective_approval_policy(events),
        )
    }

    /// Table keys in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.presets.iter().map(|(name, _)| name.clone()).collect()
    }

    fn spec(&self, name: &str) -> &PresetSpec {
        for (key, spec) in &self.presets {
            if key == name {
                return spec;
            }
        }
        unreachable!("defaultPreset is a table key")
    }

    fn derive(
        &self,
        preset: Option<&str>,
        sandbox: Option<SandboxMode>,
        approval: Option<ApprovalPolicy>,
    ) -> String {
        let sandbox = sandbox.unwrap_or(self.composed_sandbox);
        let approval = approval.unwrap_or(self.composed_approval);
        if let Some(name) = preset {
            if let Some(spec) = self
                .presets
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, s)| s)
            {
                if spec.sandbox == sandbox && spec.approval == approval {
                    return name.to_string();
                }
            }
        }
        match first_match(&self.presets, sandbox, approval) {
            Some(name) => name,
            None => CUSTOM_PRESET.to_string(),
        }
    }
}

/// Last `permission/preset` payload, if any.
#[must_use]
pub fn effective_permission_preset(events: &[LogEvent]) -> Option<String> {
    for event in events.iter().rev() {
        if let LogEvent::Known(SessionEvent::PermissionPreset { data, .. }) = event {
            return Some(data.preset.clone());
        }
    }
    None
}

/// Last `sandbox/mode` payload that names a known mode.
#[must_use]
pub fn effective_sandbox_mode(events: &[LogEvent]) -> Option<SandboxMode> {
    for event in events.iter().rev() {
        if let LogEvent::Known(SessionEvent::SandboxMode { data, .. }) = event {
            return SandboxMode::parse(&data.mode);
        }
    }
    None
}

/// Append `sandbox/mode` with `mode` and `source: None`. Time equals `seq`.
///
/// # Errors
///
/// [`SessionError`] when the session rejects the append.
pub fn set_sandbox_mode(session: &mut Session, mode: SandboxMode) -> Result<(), SessionError> {
    let seq = session.events().len() as u64;
    session.append(SessionEvent::SandboxMode {
        seq,
        time: seq as i64,
        data: SandboxModeData {
            mode: mode.as_str().into(),
            source: None,
        },
        ignorable: None,
    })?;
    Ok(())
}

fn builtin_two() -> Vec<(String, PresetSpec)> {
    vec![
        (
            "workspace-write".into(),
            PresetSpec::new(SandboxMode::WorkspaceWrite, ApprovalPolicy::Ask),
        ),
        (
            "danger-full-access".into(),
            PresetSpec::new(SandboxMode::DangerFullAccess, ApprovalPolicy::Never),
        ),
    ]
}

fn known_names(presets: &[(String, PresetSpec)]) -> String {
    presets
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn first_match(
    presets: &[(String, PresetSpec)],
    sandbox: SandboxMode,
    approval: ApprovalPolicy,
) -> Option<String> {
    for (name, spec) in presets {
        if spec.sandbox == sandbox && spec.approval == approval {
            return Some(name.clone());
        }
    }
    None
}

fn is_seeded(session: &Session) -> bool {
    if session.header().seed_length.is_some() {
        return true;
    }
    for event in session.events() {
        if let LogEvent::Known(SessionEvent::SessionEndSeed { .. }) = event {
            return true;
        }
    }
    false
}

fn append_preset(session: &mut Session, preset: &str) -> Result<(), SessionError> {
    let seq = session.events().len() as u64;
    session.append(SessionEvent::PermissionPreset {
        seq,
        time: seq as i64,
        data: PermissionPresetData {
            preset: preset.to_string(),
        },
        ignorable: None,
    })?;
    Ok(())
}

fn parse_spec(value: &Value) -> Result<PresetSpec, PermissionError> {
    let Some(object) = value.as_object() else {
        return Err(PermissionError::InvalidConfig(format!(
            "preset spec must be a mapping, got {value}"
        )));
    };
    let sandbox = match object.get("sandbox").and_then(Value::as_str) {
        Some(raw) => match SandboxMode::parse(raw) {
            Some(mode) => mode,
            None => {
                return Err(PermissionError::InvalidConfig(format!(
                    "unknown sandbox mode \"{raw}\""
                )));
            }
        },
        None => {
            return Err(PermissionError::InvalidConfig(
                "preset spec requires sandbox".into(),
            ));
        }
    };
    let approval = match object.get("approval").and_then(Value::as_str) {
        Some("ask") => ApprovalPolicy::Ask,
        Some("never") => ApprovalPolicy::Never,
        Some(raw) => {
            return Err(PermissionError::InvalidConfig(format!(
                "approval policy must be \"ask\" or \"never\", got \"{raw}\""
            )));
        }
        None => {
            return Err(PermissionError::InvalidConfig(
                "preset spec requires approval".into(),
            ));
        }
    };
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(PresetSpec {
        sandbox,
        approval,
        name,
        description,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CUSTOM_PRESET, PermissionPresetConfig, PermissionPresetService, PresetSpec,
        effective_permission_preset, effective_sandbox_mode, set_sandbox_mode,
    };
    use dsh_sandbox::SandboxMode;
    use dsh_session::{SESSION_FORMAT_VERSION, Session, SessionEvent, SessionHeader, SessionId};
    use dsh_user_approval::{ApprovalPolicy, effective_approval_policy};
    use serde_json::json;

    fn empty_header() -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new("permission-test"),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    fn default_two() -> Option<Vec<(String, PresetSpec)>> {
        Some(vec![
            (
                "workspace-write".into(),
                PresetSpec::new(SandboxMode::WorkspaceWrite, ApprovalPolicy::Ask),
            ),
            (
                "danger-full-access".into(),
                PresetSpec::new(SandboxMode::DangerFullAccess, ApprovalPolicy::Never),
            ),
        ])
    }

    fn count_type(session: &Session, ty: &str) -> usize {
        session
            .events()
            .iter()
            .filter(|event| event.event_type() == ty)
            .count()
    }

    #[test]
    fn pin_fresh_session_writes_three_knob_events() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::dsh_base(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        let mut session = Session::new(empty_header());
        svc.pin_initial(&mut session).unwrap();
        assert_eq!(
            effective_permission_preset(session.events()).as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            effective_sandbox_mode(session.events()),
            Some(SandboxMode::WorkspaceWrite)
        );
        assert_eq!(
            effective_approval_policy(session.events()),
            Some(ApprovalPolicy::Ask)
        );
    }

    #[test]
    fn pin_is_idempotent_when_knobs_already_match() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::dsh_base(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        let mut session = Session::new(empty_header());
        svc.pin_initial(&mut session).unwrap();
        let n = session.events().len();
        svc.pin_initial(&mut session).unwrap();
        assert_eq!(session.events().len(), n);
    }

    #[test]
    fn unknown_default_preset_fails_load() {
        let err = PermissionPresetService::from_config(
            PermissionPresetConfig {
                presets: default_two(),
                default_preset: Some("not-a-preset".into()),
            },
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown preset"));
    }

    #[test]
    fn omitted_presets_table_has_no_read_only() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::default(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        assert_eq!(
            svc.names(),
            vec![
                "workspace-write".to_string(),
                "danger-full-access".to_string()
            ]
        );
    }

    #[test]
    fn dsh_base_table_includes_read_only() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::dsh_base(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        assert_eq!(
            svc.names(),
            vec![
                "read-only".to_string(),
                "workspace-write".to_string(),
                "danger-full-access".to_string()
            ]
        );
    }

    #[test]
    fn omitted_default_preset_fails_when_composed_knobs_match_no_row() {
        let err = PermissionPresetService::from_config(
            PermissionPresetConfig {
                presets: default_two(),
                default_preset: None,
            },
            SandboxMode::ReadOnly,
            ApprovalPolicy::Ask,
        )
        .unwrap_err();
        assert!(err.to_string().contains("defaultPreset"));
        assert!(!err.to_string().contains(CUSTOM_PRESET));
    }

    #[test]
    fn custom_table_key_fails_load() {
        let err = PermissionPresetService::from_config(
            PermissionPresetConfig {
                presets: Some(vec![(
                    CUSTOM_PRESET.into(),
                    PresetSpec::new(SandboxMode::ReadOnly, ApprovalPolicy::Ask),
                )]),
                default_preset: Some("custom".into()),
            },
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap_err();
        assert!(err.to_string().contains("reserved"));
    }

    #[test]
    fn current_returns_custom_when_knobs_match_no_preset() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::default(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        let mut session = Session::new(empty_header());
        set_sandbox_mode(&mut session, SandboxMode::ReadOnly).unwrap();
        assert_eq!(svc.current(session.events()), CUSTOM_PRESET);
    }

    #[test]
    fn pin_partial_log_fills_only_missing_knobs() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::dsh_base(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        let mut session = Session::new(empty_header());
        set_sandbox_mode(&mut session, SandboxMode::WorkspaceWrite).unwrap();
        svc.pin_initial(&mut session).unwrap();
        assert_eq!(count_type(&session, "sandbox/mode"), 1);
        assert_eq!(
            effective_permission_preset(session.events()).as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            effective_approval_policy(session.events()),
            Some(ApprovalPolicy::Ask)
        );
    }

    #[test]
    fn pin_seeded_session_uses_composed_knobs_not_named_default() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig {
                presets: PermissionPresetConfig::dsh_base().presets,
                default_preset: Some("danger-full-access".into()),
            },
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        let mut header = empty_header();
        header.seed_length = Some(0);
        let mut session = Session::new(header);
        session
            .append(SessionEvent::SessionEndSeed {
                seq: 0,
                time: 0,
                data: json!({}),
                ignorable: None,
            })
            .unwrap();
        svc.pin_initial(&mut session).unwrap();
        assert_eq!(
            effective_permission_preset(session.events()).as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            effective_sandbox_mode(session.events()),
            Some(SandboxMode::WorkspaceWrite)
        );
        assert_eq!(
            effective_approval_policy(session.events()),
            Some(ApprovalPolicy::Ask)
        );
    }

    #[test]
    fn pin_unmatched_partial_does_not_write_custom_preset_event() {
        let svc = PermissionPresetService::from_config(
            PermissionPresetConfig::default(),
            SandboxMode::WorkspaceWrite,
            ApprovalPolicy::Ask,
        )
        .unwrap();
        let mut session = Session::new(empty_header());
        set_sandbox_mode(&mut session, SandboxMode::ReadOnly).unwrap();
        svc.pin_initial(&mut session).unwrap();
        assert_eq!(effective_permission_preset(session.events()), None);
        assert_eq!(svc.current(session.events()), CUSTOM_PRESET);
        assert_eq!(
            effective_approval_policy(session.events()),
            Some(ApprovalPolicy::Ask)
        );
    }

    #[test]
    fn effective_permission_preset_folds_to_the_last_event() {
        let mut session = Session::new(empty_header());
        assert_eq!(effective_permission_preset(session.events()), None);
        super::append_preset(&mut session, "danger-full-access").unwrap();
        super::append_preset(&mut session, "workspace-write").unwrap();
        set_sandbox_mode(&mut session, SandboxMode::ReadOnly).unwrap();
        assert_eq!(
            effective_permission_preset(session.events()).as_deref(),
            Some("workspace-write")
        );
    }

    #[test]
    fn from_value_omits_presets_when_absent() {
        let config = PermissionPresetConfig::from_value(&json!({})).unwrap();
        assert!(config.presets.is_none());
        assert!(config.default_preset.is_none());
        let parsed = PermissionPresetConfig::from_value(&json!({
            "presets": {
                "read-only": { "sandbox": "read-only", "approval": "ask" },
                "workspace-write": { "sandbox": "workspace-write", "approval": "ask" },
                "danger-full-access": { "sandbox": "danger-full-access", "approval": "never" }
            }
        }))
        .unwrap();
        let names: Vec<&str> = parsed
            .presets
            .as_ref()
            .unwrap()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"read-only"));
        assert!(names.contains(&"workspace-write"));
        assert!(names.contains(&"danger-full-access"));
    }
}
