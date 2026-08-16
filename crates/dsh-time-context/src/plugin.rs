//! Kernel plugin `@deepseek-ai/dsh-time-context`.

use std::sync::Arc;

use dsh_agent_loop::{CompactionScope, EVENT_AGENT_PRE_STEP, PreStepDecision};
use dsh_boot::{PLUGIN_TIME_CONTEXT, PluginRegistry, PluginSetup};
use dsh_kernel::{Context, KernelError};
use dsh_session::{
    ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
};
use serde_json::Value;

/// Plugin name stamped on injected [`MessageSource::Plugin`] messages.
pub(crate) const PLUGIN_SOURCE: &str = "time-context";

/// Load-time configuration failure for time-context.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

const CONFIG_KEYS: &[&str] = &["timeZone", "refreshIntervalMs"];

/// Validated plugin configuration used by the pre-step listener.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimeContextConfig {
    /// Optional display-zone label. This phase always formats wall time as UTC.
    pub time_zone: Option<String>,
    /// Minimum milliseconds between durable injections in one session.
    ///
    /// Omit or `0` to inject at every eligible entering pre-step. A positive
    /// value injects only when the session has no earlier time-context
    /// injection or elapsed time is at least this many milliseconds.
    pub refresh_interval_ms: Option<u64>,
}

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Resolve plugin JSON into a validated [`TimeContextConfig`].
///
/// # Errors
///
/// Unknown keys, a non-object config, a non-string `timeZone`, or a
/// `refreshIntervalMs` that is not a non-negative integer.
pub fn resolve_config(value: &Value) -> Result<TimeContextConfig, ConfigError> {
    match value {
        Value::Null => Ok(TimeContextConfig::default()),
        Value::Object(map) => {
            for key in map.keys() {
                if !CONFIG_KEYS.contains(&key.as_str()) {
                    return Err(ConfigError::new(format!(
                        "TimeContextConfig: unknown key \"{key}\""
                    )));
                }
            }
            Ok(TimeContextConfig {
                time_zone: optional_string(map.get("timeZone"))?,
                refresh_interval_ms: optional_refresh_interval(map.get("refreshIntervalMs"))?,
            })
        }
        _ => Err(ConfigError::new(
            "TimeContextConfig: config must be an object",
        )),
    }
}

fn optional_string(value: Option<&Value>) -> Result<Option<String>, ConfigError> {
    match value {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ConfigError::new(
            "TimeContextConfig.timeZone must be a string",
        )),
    }
}

fn optional_refresh_interval(value: Option<&Value>) -> Result<Option<u64>, ConfigError> {
    match value {
        None => Ok(None),
        Some(item) => Ok(Some(parse_refresh_interval(item)?)),
    }
}

fn parse_refresh_interval(value: &Value) -> Result<u64, ConfigError> {
    let invalid =
        || ConfigError::new("time-context: refreshIntervalMs must be a non-negative safe integer");
    if let Some(number) = value.as_u64() {
        return Ok(number);
    }
    if let Some(number) = value.as_i64() {
        if number < 0 {
            return Err(invalid());
        }
        return u64::try_from(number).map_err(|_| invalid());
    }
    let Some(number) = value.as_f64() else {
        return Err(invalid());
    };
    if number.is_finite() && number >= 0.0 && number.fract() == 0.0 && number <= u64::MAX as f64 {
        Ok(number as u64)
    } else {
        Err(invalid())
    }
}

/// Register the pre-step listener. Unknown keys and an invalid refresh interval fail load.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let resolved = resolve_config(&config).map_err(|error| setup_err(error.to_string()))?;
            register_listener(&ctx, resolved).map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_TIME_CONTEXT, setup);
}

/// Register the pre-step listener on `ctx` with an already-validated config.
pub fn install_time_context(ctx: &Context, config: TimeContextConfig) {
    register_listener(ctx, config).expect("time-context pre-step listener");
}

fn register_listener(ctx: &Context, config: TimeContextConfig) -> Result<(), KernelError> {
    ctx.on_waterfall::<PreStepDecision, _, _>(EVENT_AGENT_PRE_STEP, move |decision, next| {
        let config = config.clone();
        async move {
            let decision = inject_if_due(decision, &config, unix_now_ms());
            next(decision).await
        }
    })
    .map(|_| ())
}

fn inject_if_due(
    decision: PreStepDecision,
    config: &TimeContextConfig,
    now_ms: u64,
) -> PreStepDecision {
    let Some(view) = CompactionScope::try_current(|scope| {
        if scope.abort().is_aborted() {
            None
        } else {
            Some(scope.with_session(|session| injection_view(session)))
        }
    })
    .flatten() else {
        return decision;
    };
    apply_injection(decision, &view, now_ms, config)
}

/// Turn, next step, and elapsed-time baselines read from the live session.
pub(crate) struct InjectionView {
    turn: u64,
    step: u64,
    last_injection: Option<i64>,
    previous: Option<i64>,
}

fn injection_view(session: &Session) -> InjectionView {
    let (turn, step) = next_turn_step(session);
    let last_injection = latest_injection_time(session);
    let previous = if step == 1 {
        preceding_message_time(session)
    } else {
        preceding_step_context_time(session, turn)
    };
    InjectionView {
        turn,
        step,
        last_injection,
        previous,
    }
}

fn next_turn_step(session: &Session) -> (u64, u64) {
    let mut turn = 0u64;
    let mut step = 0u64;
    for event in session.events() {
        match event {
            LogEvent::Known(SessionEvent::TurnStart { data, .. }) => {
                turn = data.turn;
                step = 0;
            }
            LogEvent::Known(SessionEvent::StepStart { data, .. }) if data.turn == turn => {
                step = data.step;
            }
            _ => {}
        }
    }
    (turn, step + 1)
}

fn latest_injection_time(session: &Session) -> Option<i64> {
    for event in session.events().iter().rev() {
        if let LogEvent::Known(SessionEvent::UserMessage { time, data, .. }) = event {
            if is_time_context_source(&data.source) {
                return Some(*time);
            }
        }
    }
    None
}

fn preceding_message_time(session: &Session) -> Option<i64> {
    for event in session.events().iter().rev() {
        match event {
            LogEvent::Known(SessionEvent::UserMessage { time, .. })
            | LogEvent::Known(SessionEvent::AssistantMessage { time, .. })
            | LogEvent::Known(SessionEvent::ToolResult { time, .. }) => return Some(*time),
            _ => {}
        }
    }
    None
}

fn preceding_step_context_time(session: &Session, turn: u64) -> Option<i64> {
    for event in session.events().iter().rev() {
        match event {
            LogEvent::Known(SessionEvent::TurnStart { data, .. }) if data.turn == turn => {
                return None;
            }
            LogEvent::Known(SessionEvent::UserMessage { time, data, .. }) => {
                if is_time_context_source(&data.source) {
                    return Some(*time);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_time_context_source(source: &MessageSource) -> bool {
    matches!(source, MessageSource::Plugin { plugin, .. } if plugin == PLUGIN_SOURCE)
}

fn should_skip_interval(now_ms: u64, last: Option<i64>, interval: Option<u64>) -> bool {
    let Some(interval) = interval else {
        return false;
    };
    if interval == 0 {
        return false;
    }
    let Some(last) = last else {
        return false;
    };
    if last < 0 {
        return false;
    }
    let last = last as u64;
    now_ms >= last && now_ms.saturating_sub(last) < interval
}

/// Prepend a time-context user message onto a non-empty [`PreStepDecision::Enter`].
pub(crate) fn apply_injection(
    decision: PreStepDecision,
    view: &InjectionView,
    now_ms: u64,
    config: &TimeContextConfig,
) -> PreStepDecision {
    let PreStepDecision::Enter { mut messages } = decision else {
        return decision;
    };
    if messages.is_empty() {
        return PreStepDecision::Enter { messages };
    }
    if should_skip_interval(now_ms, view.last_injection, config.refresh_interval_ms) {
        return PreStepDecision::Enter { messages };
    }
    let text = render_text(now_ms, view.turn, view.step, view.previous);
    messages.insert(0, time_context_message(text));
    PreStepDecision::Enter { messages }
}

/// Build the view used by [`apply_injection`] from a session log.
#[cfg(test)]
pub(crate) fn view_from_session(session: &Session) -> InjectionView {
    injection_view(session)
}

fn render_text(now_ms: u64, turn: u64, step: u64, previous: Option<i64>) -> String {
    let elapsed = match previous {
        None => "unavailable".to_string(),
        Some(previous) => format_duration(elapsed_ms(now_ms, previous)),
    };
    let baseline = if step == 1 {
        "model-visible message"
    } else {
        "step context"
    };
    format!(
        "Time sampled while preparing turn {turn}, step {step}: {}\nElapsed since the preceding {baseline}: {elapsed}.",
        format_utc_timestamp(now_ms)
    )
}

fn elapsed_ms(now_ms: u64, previous: i64) -> u64 {
    if previous < 0 {
        return 0;
    }
    now_ms.saturating_sub(previous as u64)
}

fn time_context_message(text: String) -> Message {
    Message {
        id: mint_message_id(),
        role: MessageRole::User,
        content: vec![ContentBlock::Text { text }],
        source: MessageSource::Plugin {
            plugin: PLUGIN_SOURCE.into(),
            form: None,
            sections: Vec::new(),
            summary: None,
            compaction_id: None,
            source_command_id: None,
        },
    }
}

fn mint_message_id() -> MessageId {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    MessageId::new(format!("time-context-{}-{}", std::process::id(), nanos))
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Format a non-negative elapsed millisecond count as compact whole-second units.
pub(crate) fn format_duration(elapsed_ms: u64) -> String {
    let mut seconds = elapsed_ms / 1000;
    let days = seconds / 86_400;
    seconds %= 86_400;
    let hours = seconds / 3600;
    seconds %= 3600;
    let minutes = seconds / 60;
    seconds %= 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    parts.push(format!("{seconds}s"));
    parts.join(" ")
}

/// Format epoch milliseconds as `YYYY-MM-DD HH:MM:SS UTC`.
pub(crate) fn format_utc_timestamp(ms: u64) -> String {
    let secs = ms / 1000;
    let (year, month, day, hour, minute, second) = civil_from_unix(secs);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
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
