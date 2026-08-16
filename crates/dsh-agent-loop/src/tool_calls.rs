//! Exclusive/parallel tool-call scheduler and abort drain.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll, Wake, Waker};
use std::thread::{self, Thread};

use dsh_session::{
    CallId, ContentBlock, Message, MessageId, MessageRole, MessageSource, Session, SessionEvent,
    SurfaceOp, ToolCallData, ToolResultData, ToolResultError,
};
use dsh_tools::{
    AbortFlag, ScheduledToolDispatch, ScheduledToolPreparation, TOOL_ABORTED_BEFORE_DISPATCH,
    ToolErrorInfo, ToolExecution, ToolExecutionInput, ToolExecutionMode, ToolExecutionResult,
    ToolFailure, ToolRuntime, freeze_args_from_raw,
};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde_json::Value;

use crate::error::LoopError;

type DispatchFuture = Pin<Box<dyn Future<Output = ScheduledToolDispatch> + Send>>;
type InFlightFuture =
    Pin<Box<dyn Future<Output = (usize, ToolExecution, ScheduledToolDispatch)> + Send>>;

/// Session and tool-runtime access for one scheduled tool-call group.
pub(crate) trait ToolCallHost {
    /// Run `f` with exclusive access to the session log.
    fn with_session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R;
    /// Shared tool runtime. Callers lock around `prepare` / `finalize` and drop
    /// the guard across `dispatch` `.await`.
    fn tools(&self) -> &Mutex<ToolRuntime>;
}

/// Direct session and tools mutex, used by in-process [`crate::LoopAgent`] tests.
pub(crate) struct DirectHost<'a> {
    /// Session log that receives `tool/call` and `tool/result`.
    pub session: &'a mut Session,
    /// Shared tool runtime.
    pub tools: &'a Mutex<ToolRuntime>,
}

impl ToolCallHost for DirectHost<'_> {
    fn with_session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R {
        f(self.session)
    }

    fn tools(&self) -> &Mutex<ToolRuntime> {
        self.tools
    }
}

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
/// Parallel-safe calls run [`ToolRuntime::prepare`] serially while the tools
/// mutex is held, overlap [`ToolRuntime::dispatch`] bodies without that mutex,
/// then [`ToolRuntime::finalize`]. A later sibling
/// is reclassified before start, and an exclusive reclassification stops
/// replenishing the pool. Abort stops new starts, drains in-flight bodies, and
/// appends synthetic `ABORTED_BEFORE_DISPATCH` results for not-started calls.
/// Committed results stay in model order; additional contexts are handed to
/// `accept_context`.
///
/// # Errors
///
/// [`LoopError::Session`] when a `tool/call` or `tool/result` append is rejected.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_calls<H: ToolCallHost>(
    host: &mut H,
    turn: u64,
    step: u64,
    tool_calls: &[ContentBlock],
    signal: &AbortFlag,
    max_parallel: usize,
    accept_context: &mut (dyn FnMut(Message) + Send),
) -> Result<ToolCallsOutcome, LoopError> {
    let planned: Vec<PlannedCall> = tool_calls.iter().filter_map(plan_block).collect();
    let mut next = 0;
    let mut concluded = false;
    while next < planned.len() {
        let input = planned_input(&planned[next], signal);
        let mode = host.tools().lock().expect("tools").execution_mode(&input);
        let group_end = if mode == ToolExecutionMode::Parallel {
            planned.len()
        } else {
            next + 1
        };
        let outcome = run_group(
            host,
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
                append_skipped_tool_call(host, turn, step, call)?;
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
async fn run_group<H: ToolCallHost>(
    host: &mut H,
    turn: u64,
    step: u64,
    group: &[PlannedCall],
    mode: ToolExecutionMode,
    signal: &AbortFlag,
    max_parallel: usize,
    accept_context: &mut (dyn FnMut(Message) + Send),
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
                if host.tools().lock().expect("tools").execution_mode(&input)
                    != ToolExecutionMode::Parallel
                {
                    break;
                }
            }
            let index = next_to_start;
            call_seqs[index] = match host
                .with_session(|session| append_tool_call(session, turn, step, &group[index]))
            {
                Ok(seq) => seq,
                Err(error) => {
                    drain_in_flight(&mut in_flight).await;
                    return Err(error);
                }
            };
            started += 1;
            next_to_start += 1;
            if mode == ToolExecutionMode::Parallel {
                match start_parallel(host.tools(), &group[index], signal) {
                    StartKind::InFlight { exec, body } => {
                        in_flight.push(Box::pin(async move { (index, exec, body.await) }));
                    }
                    StartKind::Ready(result) => slots[index] = Some(result),
                }
            } else {
                slots[index] =
                    Some(execute_locked(host.tools(), planned_input(&group[index], signal)).await);
            }
            if signal.is_aborted() {
                aborted = true;
            }
            if let Err(error) = commit_ready(
                host,
                turn,
                step,
                group,
                &mut slots,
                &call_seqs,
                &mut committed,
                &mut concluded,
                accept_context,
            ) {
                drain_in_flight(&mut in_flight).await;
                return Err(error);
            }
        }

        if in_flight.is_empty() {
            break;
        }
        let Some((index, exec, dispatched)) = in_flight.next().await else {
            break;
        };
        slots[index] = Some(settle_dispatch(host.tools(), &exec, dispatched));
        if signal.is_aborted() {
            aborted = true;
        }
        if let Err(error) = commit_ready(
            host,
            turn,
            step,
            group,
            &mut slots,
            &call_seqs,
            &mut committed,
            &mut concluded,
            accept_context,
        ) {
            drain_in_flight(&mut in_flight).await;
            return Err(error);
        }
    }

    if aborted {
        for call in &group[started..] {
            append_skipped_tool_call(host, turn, step, call)?;
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

enum StartKind {
    InFlight {
        exec: ToolExecution,
        body: DispatchFuture,
    },
    Ready(ToolExecutionResult),
}

fn start_parallel(tools: &Mutex<ToolRuntime>, call: &PlannedCall, signal: &AbortFlag) -> StartKind {
    let prepared = prepare_locked(tools, planned_input(call, signal));
    match prepared {
        ScheduledToolPreparation::Dispatch { exec } => {
            let body = dispatch_locked(tools, &exec);
            StartKind::InFlight { exec, body }
        }
        ScheduledToolPreparation::PostResult { exec, result } => StartKind::Ready(settle_dispatch(
            tools,
            &exec,
            ScheduledToolDispatch::PostResult { result },
        )),
        ScheduledToolPreparation::FinalResult { result, .. } => StartKind::Ready(result),
    }
}

async fn execute_locked(
    tools: &Mutex<ToolRuntime>,
    input: ToolExecutionInput,
) -> ToolExecutionResult {
    let prepared = prepare_locked(tools, input);
    match prepared {
        ScheduledToolPreparation::Dispatch { exec } => {
            let body = dispatch_locked(tools, &exec);
            let dispatched = body.await;
            settle_dispatch(tools, &exec, dispatched)
        }
        ScheduledToolPreparation::PostResult { exec, result } => {
            settle_dispatch(tools, &exec, ScheduledToolDispatch::PostResult { result })
        }
        ScheduledToolPreparation::FinalResult { result, .. } => result,
    }
}

fn settle_dispatch(
    tools: &Mutex<ToolRuntime>,
    exec: &ToolExecution,
    dispatched: ScheduledToolDispatch,
) -> ToolExecutionResult {
    match dispatched {
        ScheduledToolDispatch::PostResult { result } => finalize_locked(tools, exec, result),
        ScheduledToolDispatch::FinalResult { result } => result,
    }
}

/// Lock `tools` for [`ToolRuntime::prepare`] and drop the guard before returning.
///
/// `std::sync::MutexGuard` is `!Send`, so the future is driven to completion
/// here rather than `.await`ed in a `Send` task.
fn prepare_locked(
    tools: &Mutex<ToolRuntime>,
    input: ToolExecutionInput,
) -> ScheduledToolPreparation {
    let mut tools = tools.lock().expect("tools");
    block_on(tools.prepare(input))
}

fn dispatch_locked(tools: &Mutex<ToolRuntime>, exec: &ToolExecution) -> DispatchFuture {
    let tools = tools.lock().expect("tools");
    tools.dispatch(exec)
}

/// Lock `tools` for [`ToolRuntime::finalize`] and drop the guard before returning.
fn finalize_locked(
    tools: &Mutex<ToolRuntime>,
    exec: &ToolExecution,
    result: ToolExecutionResult,
) -> ToolExecutionResult {
    let tools = tools.lock().expect("tools");
    block_on(tools.finalize(exec, result))
}

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut cx = TaskContext::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

async fn drain_in_flight(in_flight: &mut FuturesUnordered<InFlightFuture>) {
    while in_flight.next().await.is_some() {}
}

#[allow(clippy::too_many_arguments)]
fn commit_ready<H: ToolCallHost>(
    host: &mut H,
    turn: u64,
    step: u64,
    group: &[PlannedCall],
    slots: &mut [Option<ToolExecutionResult>],
    call_seqs: &[u64],
    committed: &mut usize,
    concluded: &mut bool,
    accept_context: &mut (dyn FnMut(Message) + Send),
) -> Result<(), LoopError> {
    while *committed < group.len() {
        let Some(result) = slots[*committed].take() else {
            break;
        };
        let call = &group[*committed];
        host.with_session(|session| {
            append_tool_result(
                session,
                turn,
                step,
                call,
                &result,
                call_seqs[*committed],
                accept_context,
            )
        })?;
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

fn append_skipped_tool_call<H: ToolCallHost>(
    host: &mut H,
    turn: u64,
    step: u64,
    call: &PlannedCall,
) -> Result<(), LoopError> {
    let call_seq = host.with_session(|session| append_tool_call(session, turn, step, call))?;
    host.with_session(|session| {
        append_tool_result(
            session,
            turn,
            step,
            call,
            &aborted_before_dispatch(),
            call_seq,
            &mut |_| {},
        )
    })
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
    accept_context: &mut (dyn FnMut(Message) + Send),
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

#[cfg(test)]
mod tests {
    use super::{DirectHost, ToolCallsOutcome, execute_tool_calls};
    use crate::test_header;
    use dsh_session::{CallId, ContentBlock, LogEvent, Session, SessionEvent};
    use dsh_tools::{
        AbortFlag, PreToolDecision, TOOL_ABORTED_BEFORE_DISPATCH, ToolDefinition,
        ToolPresentationMode, ToolRuntime,
    };
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;
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
        let tools = Mutex::new(tools);
        let mut extra = Vec::new();
        let outcome = execute_tool_calls(
            &mut DirectHost {
                session: &mut session,
                tools: &tools,
            },
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
        let tools = Mutex::new(tools);
        let mut extra = Vec::new();
        let outcome = execute_tool_calls(
            &mut DirectHost {
                session: &mut session,
                tools: &tools,
            },
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

    #[tokio::test]
    async fn parallel_pre_deny_skips_the_body() {
        let mut session = Session::new(test_header("deny"));
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_body = ran.clone();
        let seen_token = Arc::new(AtomicUsize::new(0));
        let token_slot = seen_token.clone();
        tools.register(ToolDefinition {
            name: "par".into(),
            description: "parallel".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(move |args, _exec| {
                ran_body.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move { Ok(args) })
            }),
            render: Box::new(|_, v| {
                vec![ContentBlock::Text {
                    text: v.to_string(),
                }]
            }),
            is_concurrency_safe: Some(Box::new(|_| true)),
        });
        tools.on_pre(move |exec, _next| {
            token_slot.store(exec.token.0 as usize, Ordering::SeqCst);
            Box::pin(async {
                PreToolDecision::Deny {
                    reason: "denied by policy".into(),
                }
            })
        });
        let blocks = vec![
            tool_call("c1", "par", json!({"id": "a"})),
            tool_call("c2", "par", json!({"id": "b"})),
        ];
        let tools = Mutex::new(tools);
        let mut extra = Vec::new();
        let outcome = execute_tool_calls(
            &mut DirectHost {
                session: &mut session,
                tools: &tools,
            },
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
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        assert!(seen_token.load(Ordering::SeqCst) >= 1);
        let texts: Vec<_> = session
            .events()
            .iter()
            .filter_map(|event| match event {
                LogEvent::Known(SessionEvent::ToolResult { data, .. }) => {
                    data.message.content.iter().find_map(|block| match block {
                        ContentBlock::ToolResult { content, .. } => {
                            content.iter().find_map(|inner| match inner {
                                ContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2);
        assert!(texts.iter().all(|text| text == "Error: denied by policy"));
    }
}
