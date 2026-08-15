//! Exclusive/parallel tool-call scheduler and abort drain.

use std::future::Future;
use std::pin::Pin;

use dsh_session::{
    CallId, ContentBlock, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
    SurfaceOp, ToolCallData, ToolResultData, ToolResultError,
};
use dsh_tools::{
    AbortFlag, TOOL_ABORTED, TOOL_ABORTED_BEFORE_DISPATCH, ToolError, ToolErrorInfo, ToolExecution,
    ToolExecutionInput, ToolExecutionMode, ToolExecutionResult, ToolExecutionToken, ToolFailure,
    ToolRuntime, freeze_args_from_raw,
};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde_json::Value;

use crate::error::LoopError;

type BodyFuture = Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send>>;
type InFlightFuture = Pin<Box<dyn Future<Output = (usize, Result<Value, ToolError>)> + Send>>;

/// Whether the scheduled calls ended the turn or stopped on abort.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToolCallsOutcome {
    /// True when any committed result set `concludes_turn`.
    pub concluded: bool,
    /// True when abort stopped dispatch; not-started calls received synthetic results.
    pub aborted: bool,
}

struct PlannedCall {
    id: CallId,
    name: String,
    raw_arguments: String,
    arguments: Value,
}

struct GroupOutcome {
    consumed: usize,
    aborted: bool,
    concluded: bool,
}

/// Schedule one assistant step's tool-call blocks in model order.
///
/// Exclusive calls are a barrier of one and run the full tool pipeline.
/// Parallel-safe calls share a pool of `max_parallel` in-flight bodies; a later
/// sibling is reclassified before start, and an exclusive reclassification
/// stops replenishing the pool. Abort stops new starts, drains in-flight
/// bodies, and appends synthetic `ABORTED_BEFORE_DISPATCH` results for
/// not-started calls. Committed results stay in model order; additional
/// contexts are handed to `accept_context`.
///
/// # Errors
///
/// [`LoopError::Session`] when a `tool/call` or `tool/result` append is rejected.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_calls(
    session: &mut Session,
    tools: &mut ToolRuntime,
    turn: u64,
    step: u64,
    tool_calls: &[ContentBlock],
    signal: &AbortFlag,
    max_parallel: usize,
    accept_context: &mut dyn FnMut(Message),
) -> Result<ToolCallsOutcome, LoopError> {
    let planned: Vec<PlannedCall> = tool_calls.iter().filter_map(plan_block).collect();
    let mut next = 0;
    let mut concluded = false;
    while next < planned.len() {
        let input = planned_input(&planned[next], signal);
        let mode = tools.execution_mode(&input);
        let group_end = if mode == ToolExecutionMode::Parallel {
            planned.len()
        } else {
            next + 1
        };
        let outcome = run_group(
            session,
            tools,
            turn,
            step,
            &planned[next..group_end],
            mode,
            signal,
            max_parallel,
            accept_context,
        )
        .await?;
        next += outcome.consumed;
        concluded |= outcome.concluded;
        if outcome.aborted {
            for call in &planned[next..] {
                append_skipped_tool_call(session, turn, step, call)?;
            }
            return Ok(ToolCallsOutcome {
                concluded,
                aborted: true,
            });
        }
    }
    Ok(ToolCallsOutcome {
        concluded,
        aborted: false,
    })
}

fn plan_block(block: &ContentBlock) -> Option<PlannedCall> {
    let ContentBlock::ToolCall {
        id,
        name,
        arguments,
    } = block
    else {
        return None;
    };
    let frozen =
        freeze_args_from_raw(arguments).unwrap_or_else(|_| Value::String(arguments.clone()));
    Some(PlannedCall {
        id: id.clone(),
        name: name.clone(),
        raw_arguments: arguments.clone(),
        arguments: frozen,
    })
}

fn planned_input(call: &PlannedCall, signal: &AbortFlag) -> ToolExecutionInput {
    ToolExecutionInput {
        call_id: call.id.clone(),
        root_call_id: None,
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        parent: None,
        signal: signal.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_group(
    session: &mut Session,
    tools: &mut ToolRuntime,
    turn: u64,
    step: u64,
    group: &[PlannedCall],
    mode: ToolExecutionMode,
    signal: &AbortFlag,
    max_parallel: usize,
    accept_context: &mut dyn FnMut(Message),
) -> Result<GroupOutcome, LoopError> {
    let pool_limit = if mode == ToolExecutionMode::Parallel {
        max_parallel.max(1)
    } else {
        1
    };
    let mut slots: Vec<Option<ToolExecutionResult>> = (0..group.len()).map(|_| None).collect();
    let mut call_seqs = vec![0_u64; group.len()];
    let mut next_to_start = 0;
    let mut committed = 0;
    let mut started = 0;
    let mut aborted = signal.is_aborted();
    let mut concluded = false;
    let mut in_flight: FuturesUnordered<InFlightFuture> = FuturesUnordered::new();

    loop {
        while !aborted && next_to_start < group.len() && in_flight.len() < pool_limit {
            if next_to_start > 0 && mode == ToolExecutionMode::Parallel {
                let input = planned_input(&group[next_to_start], signal);
                if tools.execution_mode(&input) != ToolExecutionMode::Parallel {
                    break;
                }
            }
            let index = next_to_start;
            call_seqs[index] = append_tool_call(session, turn, step, &group[index])?;
            started += 1;
            next_to_start += 1;
            if mode == ToolExecutionMode::Parallel {
                if let Some(body) = parallel_body(tools, &group[index], signal) {
                    in_flight.push(Box::pin(async move { (index, body.await) }));
                } else {
                    slots[index] = Some(tools.execute(planned_input(&group[index], signal)).await);
                }
            } else {
                slots[index] = Some(tools.execute(planned_input(&group[index], signal)).await);
            }
            if signal.is_aborted() {
                aborted = true;
            }
            commit_ready(
                session,
                turn,
                step,
                group,
                &mut slots,
                &call_seqs,
                &mut committed,
                &mut concluded,
                accept_context,
            )?;
        }

        if in_flight.is_empty() {
            break;
        }
        let Some((index, body)) = in_flight.next().await else {
            break;
        };
        slots[index] = Some(materialize(tools, &group[index], body, signal));
        if signal.is_aborted() {
            aborted = true;
        }
        commit_ready(
            session,
            turn,
            step,
            group,
            &mut slots,
            &call_seqs,
            &mut committed,
            &mut concluded,
            accept_context,
        )?;
    }

    if aborted {
        for call in &group[started..] {
            append_skipped_tool_call(session, turn, step, call)?;
        }
        return Ok(GroupOutcome {
            consumed: group.len(),
            aborted: true,
            concluded,
        });
    }
    Ok(GroupOutcome {
        consumed: started,
        aborted: false,
        concluded,
    })
}

fn parallel_body(
    tools: &ToolRuntime,
    call: &PlannedCall,
    signal: &AbortFlag,
) -> Option<BodyFuture> {
    let def = tools.get(&call.name)?;
    let exec = ToolExecution {
        token: ToolExecutionToken(0),
        call_id: call.id.clone(),
        root_call_id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        parent: None,
        signal: signal.clone(),
    };
    Some((def.execute)(call.arguments.clone(), exec))
}

fn materialize(
    tools: &ToolRuntime,
    call: &PlannedCall,
    body: Result<Value, ToolError>,
    signal: &AbortFlag,
) -> ToolExecutionResult {
    match body {
        Ok(value) => {
            if signal.is_aborted() {
                return aborted_after_body();
            }
            let content = tools
                .get(&call.name)
                .map(|tool| (tool.render)(&call.arguments, &value))
                .unwrap_or_default();
            ToolExecutionResult::Success {
                value,
                content,
                meta: None,
                additional_contexts: vec![],
                concludes_turn: false,
            }
        }
        Err(error) => result_from_tool_error(&error),
    }
}

#[allow(clippy::too_many_arguments)]
fn commit_ready(
    session: &mut Session,
    turn: u64,
    step: u64,
    group: &[PlannedCall],
    slots: &mut [Option<ToolExecutionResult>],
    call_seqs: &[u64],
    committed: &mut usize,
    concluded: &mut bool,
    accept_context: &mut dyn FnMut(Message),
) -> Result<(), LoopError> {
    while *committed < group.len() {
        let Some(result) = slots[*committed].take() else {
            break;
        };
        let call = &group[*committed];
        append_tool_result(
            session,
            turn,
            step,
            call,
            &result,
            call_seqs[*committed],
            accept_context,
        )?;
        *concluded |= matches!(
            result,
            ToolExecutionResult::Success {
                concludes_turn: true,
                ..
            }
        );
        *committed += 1;
    }
    Ok(())
}

fn append_skipped_tool_call(
    session: &mut Session,
    turn: u64,
    step: u64,
    call: &PlannedCall,
) -> Result<(), LoopError> {
    let call_seq = append_tool_call(session, turn, step, call)?;
    append_tool_result(
        session,
        turn,
        step,
        call,
        &aborted_before_dispatch(),
        call_seq,
        &mut |_| {},
    )
}

fn append_tool_call(
    session: &mut Session,
    turn: u64,
    step: u64,
    call: &PlannedCall,
) -> Result<u64, LoopError> {
    let seq = session.events().len() as u64;
    session.append(SessionEvent::ToolCall {
        seq,
        time: seq as i64,
        data: ToolCallData {
            turn,
            step,
            call_id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.raw_arguments.clone(),
        },
        ignorable: None,
    })?;
    Ok(seq)
}

fn append_tool_result(
    session: &mut Session,
    turn: u64,
    step: u64,
    call: &PlannedCall,
    result: &ToolExecutionResult,
    call_seq: u64,
    accept_context: &mut dyn FnMut(Message),
) -> Result<(), LoopError> {
    let seq = session.events().len() as u64;
    let is_error = result.is_error();
    session.append(SessionEvent::ToolResult {
        seq,
        time: seq as i64,
        data: ToolResultData {
            turn,
            step,
            message: Message {
                id: MessageId::new(format!("tool-result-{}-{seq}", call.id.as_str())),
                role: MessageRole::User,
                content: vec![ContentBlock::ToolResult {
                    tool_call_id: call.id.clone(),
                    content: result.content().to_vec(),
                    is_error: Some(is_error),
                }],
                source: MessageSource::Tool {
                    call_id: call.id.clone(),
                },
            },
            error: tool_result_error(result),
            meta: tool_meta(result),
        },
        surface_op: Some(SurfaceOp::Append),
        source_event_seqs: Some(vec![call_seq]),
        ignorable: None,
    })?;
    for context in additional_contexts(result) {
        accept_context(context.clone());
    }
    Ok(())
}

fn tool_result_error(result: &ToolExecutionResult) -> Option<ToolResultError> {
    match result {
        ToolExecutionResult::Failure { error, .. } => {
            error.info.as_ref().map(|info| ToolResultError {
                name: info.name.clone(),
                code: info.code.clone(),
            })
        }
        ToolExecutionResult::Success { .. } => None,
    }
}

fn tool_meta(result: &ToolExecutionResult) -> Option<Value> {
    match result {
        ToolExecutionResult::Success { meta, .. } | ToolExecutionResult::Failure { meta, .. } => {
            meta.clone()
        }
    }
}

fn additional_contexts(result: &ToolExecutionResult) -> &[Message] {
    match result {
        ToolExecutionResult::Success {
            additional_contexts,
            ..
        }
        | ToolExecutionResult::Failure {
            additional_contexts,
            ..
        } => additional_contexts,
    }
}

fn aborted_before_dispatch() -> ToolExecutionResult {
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: "tool call aborted before dispatch".into(),
            info: Some(ToolErrorInfo {
                name: "AbortError".into(),
                code: TOOL_ABORTED_BEFORE_DISPATCH.into(),
            }),
        },
        content: vec![ContentBlock::Text {
            text: "Error: tool call aborted before dispatch".into(),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

fn aborted_after_body() -> ToolExecutionResult {
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: "tool call aborted".into(),
            info: Some(ToolErrorInfo {
                name: "AbortError".into(),
                code: TOOL_ABORTED.into(),
            }),
        },
        content: vec![ContentBlock::Text {
            text: "Error: tool call aborted".into(),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

fn result_from_tool_error(error: &ToolError) -> ToolExecutionResult {
    let message = error.to_string();
    let info = match error {
        ToolError::UnknownTool(_) | ToolError::UnknownToolHint { .. } => Some(ToolErrorInfo {
            name: "ToolNotFoundError".into(),
            code: "UNKNOWN_TOOL".into(),
        }),
        ToolError::ArgsNotJson | ToolError::Other(_) => None,
    };
    ToolExecutionResult::Failure {
        error: ToolFailure {
            message: message.clone(),
            info,
        },
        content: vec![ContentBlock::Text {
            text: format!("Error: {message}"),
        }],
        meta: None,
        additional_contexts: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolCallsOutcome, execute_tool_calls};
    use crate::test_header;
    use dsh_session::{CallId, ContentBlock, LogEvent, Session, SessionEvent};
    use dsh_tools::{
        AbortFlag, TOOL_ABORTED_BEFORE_DISPATCH, ToolDefinition, ToolPresentationMode, ToolRuntime,
    };
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tool_call(id: &str, name: &str, args: serde_json::Value) -> ContentBlock {
        ContentBlock::ToolCall {
            id: CallId::new(id),
            name: name.into(),
            arguments: args.to_string(),
        }
    }

    fn gated_tool(
        name: &str,
        parallel: bool,
        live: Arc<AtomicUsize>,
        max_live: Arc<AtomicUsize>,
    ) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: "test".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(move |args, _exec| {
                let live = live.clone();
                let max_live = max_live.clone();
                Box::pin(async move {
                    let n = live.fetch_add(1, Ordering::SeqCst) + 1;
                    max_live.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    live.fetch_sub(1, Ordering::SeqCst);
                    Ok(args)
                })
            }),
            render: Box::new(|_, v| {
                vec![ContentBlock::Text {
                    text: v.to_string(),
                }]
            }),
            is_concurrency_safe: if parallel {
                Some(Box::new(|_| true))
            } else {
                None
            },
        }
    }

    #[tokio::test]
    async fn aborted_signal_skips_unstarted_calls() {
        let mut session = Session::new(test_header("skip"));
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let live = Arc::new(AtomicUsize::new(0));
        tools.register(gated_tool(
            "slow",
            false,
            live.clone(),
            Arc::new(AtomicUsize::new(0)),
        ));
        let signal = AbortFlag::new();
        signal.abort();
        let blocks = vec![
            tool_call("c1", "slow", json!({"id": "a"})),
            tool_call("c2", "slow", json!({"id": "b"})),
        ];
        let mut extra = Vec::new();
        let outcome = execute_tool_calls(
            &mut session,
            &mut tools,
            1,
            1,
            &blocks,
            &signal,
            10,
            &mut |message| extra.push(message),
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            ToolCallsOutcome {
                concluded: false,
                aborted: true
            }
        );
        assert_eq!(live.load(Ordering::SeqCst), 0);
        let errors: Vec<_> = session
            .events()
            .iter()
            .filter_map(|event| match event {
                LogEvent::Known(SessionEvent::ToolResult { data, .. }) => data.error.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|error| {
            error.code == TOOL_ABORTED_BEFORE_DISPATCH && error.name == "AbortError"
        }));
    }

    #[tokio::test]
    async fn exclusive_calls_do_not_overlap() {
        let mut session = Session::new(test_header("excl"));
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let live = Arc::new(AtomicUsize::new(0));
        let max_live = Arc::new(AtomicUsize::new(0));
        tools.register(gated_tool("slow", false, live, max_live.clone()));
        let blocks = vec![
            tool_call("c1", "slow", json!({"id": "a"})),
            tool_call("c2", "slow", json!({"id": "b"})),
        ];
        let mut extra = Vec::new();
        let outcome = execute_tool_calls(
            &mut session,
            &mut tools,
            1,
            1,
            &blocks,
            &AbortFlag::new(),
            10,
            &mut |message| extra.push(message),
        )
        .await
        .unwrap();
        assert!(!outcome.aborted);
        assert_eq!(max_live.load(Ordering::SeqCst), 1);
    }
}
