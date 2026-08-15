//! Request-header reconstruction over `request/header` session events.

use serde_json::Value;

use crate::SessionEvent;
use crate::message::{EpochHeader, LlmCallConfig, LlmCallConfigAdapterDefaults};

/// Normalize a header so empty system text and an empty or null tool list are absent.
///
/// A string tools token is kept. `adapter_defaults` is kept only when
/// `reasoning_effort` or `max_tokens` is `Some(true)`.
///
/// # Parameters
///
/// * `header` - snapshot to normalize; not mutated.
///
/// # Returns
///
/// The canonical header used for logging, folding, and comparison.
#[must_use]
pub fn canonical_header(header: &EpochHeader) -> EpochHeader {
    EpochHeader {
        config: header.config.clone(),
        adapter_defaults: keep_adapter_defaults(header.adapter_defaults.as_ref()),
        system: nonempty_system(header.system.as_deref()),
        tools: keep_tools(header.tools.as_ref()),
    }
}

/// Field-wise equality over headers after treating a missing tools list and `[]` as equal.
///
/// Config compares `provider`, `model`, `reasoning_effort`, `temperature`, `max_tokens`,
/// and `stop` element-wise. Adapter-default markers compare independently of the
/// surrounding `adapter_defaults` object.
///
/// # Parameters
///
/// * `a` - one header.
/// * `b` - the other header.
///
/// # Returns
///
/// Whether config, adapter-default markers, system, and tools all match.
#[must_use]
pub fn header_equals(a: &EpochHeader, b: &EpochHeader) -> bool {
    call_config_equals(&a.config, &b.config)
        && adapter_marker(a.adapter_defaults.as_ref(), |defaults| {
            defaults.reasoning_effort
        }) == adapter_marker(b.adapter_defaults.as_ref(), |defaults| {
            defaults.reasoning_effort
        })
        && adapter_marker(a.adapter_defaults.as_ref(), |defaults| defaults.max_tokens)
            == adapter_marker(b.adapter_defaults.as_ref(), |defaults| defaults.max_tokens)
        && a.system == b.system
        && tools_equal(a.tools.as_ref(), b.tools.as_ref())
}

/// Fold `request/header` events into the header in force after the last snapshot.
///
/// Non-header events are skipped. `from` is the baseline when no snapshot follows.
/// Each observed snapshot replaces the state with its canonical form.
///
/// # Parameters
///
/// * `events` - session events in log order.
/// * `from` - previously folded state to continue from.
///
/// # Returns
///
/// The latest canonical header, or `None` when none exists yet.
#[must_use]
pub fn fold_request_header(
    events: &[SessionEvent],
    from: Option<EpochHeader>,
) -> Option<EpochHeader> {
    fold_request_header_iter(events.iter(), from)
}

pub(crate) fn fold_request_header_iter<'a, I>(
    events: I,
    from: Option<EpochHeader>,
) -> Option<EpochHeader>
where
    I: IntoIterator<Item = &'a SessionEvent>,
{
    let mut state = from;
    for event in events {
        if let SessionEvent::RequestHeader { data, .. } = event {
            state = Some(canonical_header(&data.header));
        }
    }
    state
}

fn keep_adapter_defaults(
    defaults: Option<&LlmCallConfigAdapterDefaults>,
) -> Option<LlmCallConfigAdapterDefaults> {
    let defaults = defaults?;
    if defaults.reasoning_effort == Some(true) || defaults.max_tokens == Some(true) {
        Some(defaults.clone())
    } else {
        None
    }
}

fn nonempty_system(system: Option<&str>) -> Option<String> {
    system.filter(|text| !text.is_empty()).map(str::to_owned)
}

fn keep_tools(tools: Option<&Value>) -> Option<Value> {
    match tools {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) if items.is_empty() => None,
        Some(value) => Some(value.clone()),
    }
}

fn call_config_equals(a: &LlmCallConfig, b: &LlmCallConfig) -> bool {
    a.provider == b.provider
        && a.model == b.model
        && a.reasoning_effort == b.reasoning_effort
        && a.temperature == b.temperature
        && a.max_tokens == b.max_tokens
        && stop_equals(a.stop.as_deref(), b.stop.as_deref())
}

fn stop_equals(a: Option<&[String]>, b: Option<&[String]>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn adapter_marker(
    defaults: Option<&LlmCallConfigAdapterDefaults>,
    field: impl Fn(&LlmCallConfigAdapterDefaults) -> Option<bool>,
) -> Option<bool> {
    defaults.and_then(field)
}

fn tools_equal(a: Option<&Value>, b: Option<&Value>) -> bool {
    canonical_tools(a) == canonical_tools(b)
}

fn canonical_tools(tools: Option<&Value>) -> Value {
    match tools {
        None => Value::Array(Vec::new()),
        Some(Value::Array(items)) if items.is_empty() => Value::Array(Vec::new()),
        Some(value) => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_header, fold_request_header, header_equals};
    use crate::SessionEvent;
    use crate::message::{
        EpochHeader, LlmCallConfig, RequestHeaderData, RequestHeaderReason, TurnStartData,
    };
    use serde_json::json;

    fn config() -> LlmCallConfig {
        LlmCallConfig {
            provider: "mock".into(),
            model: "m".into(),
            reasoning_effort: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        }
    }

    #[test]
    fn canonical_drops_empty_system_and_empty_tools_array() {
        let header = EpochHeader {
            config: config(),
            adapter_defaults: Some(crate::LlmCallConfigAdapterDefaults::default()),
            system: Some(String::new()),
            tools: Some(json!([])),
        };
        let canonical = canonical_header(&header);
        assert!(canonical.system.is_none());
        assert!(canonical.tools.is_none());
        assert!(canonical.adapter_defaults.is_none());
    }

    #[test]
    fn fold_takes_latest_snapshot() {
        let events = vec![
            SessionEvent::TurnStart {
                seq: 0,
                time: 1,
                data: TurnStartData { turn: 1 },
                ignorable: None,
            },
            SessionEvent::RequestHeader {
                seq: 1,
                time: 2,
                data: RequestHeaderData {
                    header: EpochHeader {
                        config: config(),
                        adapter_defaults: None,
                        system: Some("first".into()),
                        tools: None,
                    },
                    reason: RequestHeaderReason::Initial,
                },
                ignorable: None,
            },
            SessionEvent::RequestHeader {
                seq: 2,
                time: 3,
                data: RequestHeaderData {
                    header: EpochHeader {
                        config: config(),
                        adapter_defaults: None,
                        system: Some("second".into()),
                        tools: None,
                    },
                    reason: RequestHeaderReason::Change,
                },
                ignorable: None,
            },
        ];
        let folded = fold_request_header(&events, None).expect("header");
        assert_eq!(folded.system.as_deref(), Some("second"));
        assert!(header_equals(&folded, &folded));
    }
}
