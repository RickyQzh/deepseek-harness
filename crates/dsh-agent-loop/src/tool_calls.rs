//! Exclusive/parallel tool-call scheduler and abort drain.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use dsh_session::{
    CallId, ContentBlock, LogEvent, Message, MessageId, MessageRole, MessageSource, Session,
    SessionEvent, SessionId, SurfaceOp, ToolCallData, ToolResultData, ToolResultError,
};
use dsh_tools::{
    AbortFlag, PrepareSnapshot, ScheduledToolDispatch, ScheduledToolPreparation,
    TOOL_ABORTED_BEFORE_DISPATCH, ToolErrorInfo, ToolExecution, ToolExecutionInput,
    ToolExecutionMode, ToolExecutionResult, ToolFailure, ToolRuntime, freeze_args_from_raw,
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
    /// Shared tool runtime. Snapshot `prepare` under this mutex without `.await`;
    /// `dispatch` clones the body future under a brief lock then drops the guard
    /// before polling. `finalize` may still use the blocking pool.
    fn tools(&self) -> &Arc<Mutex<ToolRuntime>>;
    /// Direct `&mut Session` for [`PrepareSnapshot::finish`]. Shared hosts return
    /// `None` and merge approval events after a detached clone.
    fn session_mut(&mut self) -> Option<&mut Session> {
        None
    }
}

/// Direct session and tools mutex, used by in-process [`crate::LoopAgent`] tests.
pub(crate) struct DirectHost<'a> {
    /// Session log that receives `tool/call` and `tool/result`.
    pub session: &'a mut Session,
    /// Shared tool runtime.
    pub tools: &'a Arc<Mutex<ToolRuntime>>,
}

impl ToolCallHost for DirectHost<'_> {
    fn with_session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R {
        f(self.session)
    }

    fn tools(&self) -> &Arc<Mutex<ToolRuntime>> {
        self.tools
    }

    fn session_mut(&mut self) -> Option<&mut Session> {
        Some(self.session)
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
/// Parallel-safe calls run [`ToolRuntime::begin_prepare`] under a brief mutex
/// (no `.await`), overlap [`ToolRuntime::dispatch`] bodies without that mutex,
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
        let input = planned_input(host, &planned[next], signal);
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

fn planned_input<H: ToolCallHost>(
    host: &mut H,
    call: &PlannedCall,
    signal: &AbortFlag,
) -> ToolExecutionInput {
    let session_id: Option<SessionId> = Some(host.with_session(|session| session.id().clone()));
    ToolExecutionInput {
        call_id: call.id.clone(),
        root_call_id: None,
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        parent: None,
        session_id,
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
                let input = planned_input(host, &group[next_to_start], signal);
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
                match start_parallel(host, &group[index], signal).await {
                    StartKind::InFlight { exec, body } => {
                        in_flight.push(Box::pin(async move { (index, exec, body.await) }));
                    }
                    StartKind::Ready(result) => slots[index] = Some(result),
                }
            } else {
                let input = planned_input(host, &group[index], signal);
                slots[index] = Some(execute_locked(host, input).await);
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
        slots[index] = Some(settle_dispatch(host.tools(), &exec, dispatched).await);
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

async fn start_parallel<H: ToolCallHost>(
    host: &mut H,
    call: &PlannedCall,
    signal: &AbortFlag,
) -> StartKind {
    let input = planned_input(host, call, signal);
    let prepared = prepare_on_host(host, input).await;
    match prepared {
        ScheduledToolPreparation::Dispatch { exec } => {
            let body = dispatch_locked(host.tools(), &exec);
            StartKind::InFlight { exec, body }
        }
        ScheduledToolPreparation::PostResult { exec, result } => StartKind::Ready(
            settle_dispatch(
                host.tools(),
                &exec,
                ScheduledToolDispatch::PostResult { result },
            )
            .await,
        ),
        ScheduledToolPreparation::FinalResult { result, .. } => StartKind::Ready(result),
    }
}

async fn execute_locked<H: ToolCallHost>(
    host: &mut H,
    input: ToolExecutionInput,
) -> ToolExecutionResult {
    let prepared = prepare_on_host(host, input).await;
    match prepared {
        ScheduledToolPreparation::Dispatch { exec } => {
            let body = dispatch_locked(host.tools(), &exec);
            let dispatched = body.await;
            settle_dispatch(host.tools(), &exec, dispatched).await
        }
        ScheduledToolPreparation::PostResult { exec, result } => {
            settle_dispatch(
                host.tools(),
                &exec,
                ScheduledToolDispatch::PostResult { result },
            )
            .await
        }
        ScheduledToolPreparation::FinalResult { result, .. } => result,
    }
}

async fn settle_dispatch(
    tools: &Arc<Mutex<ToolRuntime>>,
    exec: &ToolExecution,
    dispatched: ScheduledToolDispatch,
) -> ToolExecutionResult {
    match dispatched {
        ScheduledToolDispatch::PostResult { result } => finalize_locked(tools, exec, result).await,
        ScheduledToolDispatch::FinalResult { result } => result,
    }
}

/// Snapshot freeze/collapse under the tools mutex, then finish on this worker
/// with `&mut Session` so an [`dsh_tools::Approver`] can append audit events.
async fn prepare_on_host<H: ToolCallHost>(
    host: &mut H,
    input: ToolExecutionInput,
) -> ScheduledToolPreparation {
    let snapshot = {
        let mut tools = host.tools().lock().expect("tools");
        tools.begin_prepare(input)
    };
    finish_prepare(host, snapshot).await
}

async fn finish_prepare<H: ToolCallHost>(
    host: &mut H,
    snapshot: PrepareSnapshot,
) -> ScheduledToolPreparation {
    if let Some(session) = host.session_mut() {
        return snapshot.finish(Some(session)).await;
    }
    let (mut session, origin_len) = host.with_session(|live| {
        let mut cloned = live.clone();
        cloned.set_append_sink(None);
        let origin_len = cloned.events().len();
        (cloned, origin_len)
    });
    let prepared = snapshot.finish(Some(&mut session)).await;
    host.with_session(|live| {
        merge_approval_audit(live, &session, origin_len);
    });
    prepared
}

fn merge_approval_audit(live: &mut Session, clone: &Session, origin_len: usize) {
    for event in clone.events().iter().skip(origin_len) {
        let seq = live.events().len() as u64;
        let time = seq as i64;
        let known = match event {
            LogEvent::Known(SessionEvent::ApprovalAsked {
                data, ignorable, ..
            }) => SessionEvent::ApprovalAsked {
                seq,
                time,
                data: data.clone(),
                ignorable: *ignorable,
            },
            LogEvent::Known(SessionEvent::ApprovalDecided {
                data, ignorable, ..
            }) => SessionEvent::ApprovalDecided {
                seq,
                time,
                data: data.clone(),
                ignorable: *ignorable,
            },
            _ => continue,
        };
        live.append(known).expect("approval audit merge");
    }
}

fn dispatch_locked(tools: &Arc<Mutex<ToolRuntime>>, exec: &ToolExecution) -> DispatchFuture {
    let tools = tools.lock().expect("tools");
    tools.dispatch(exec)
}

/// Drive [`ToolRuntime::finalize`] on the blocking pool with the tools mutex held
/// there so the Tokio worker stays free.
async fn finalize_locked(
    tools: &Arc<Mutex<ToolRuntime>>,
    exec: &ToolExecution,
    result: ToolExecutionResult,
) -> ToolExecutionResult {
    let tools = Arc::clone(tools);
    let exec = exec.clone();
    tokio::task::spawn_blocking(move || {
        let guard = tools.lock().expect("tools");
        tokio::runtime::Handle::current().block_on(guard.finalize(&exec, result))
    })
    .await
    .expect("finalize join")
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
    use super::{DirectHost, ToolCallHost, ToolCallsOutcome, execute_tool_calls};
    use crate::test_header;
    use dsh_session::{
        CallId, ContentBlock, InboxSplicedData, InboxTarget, LogEvent, Session, SessionEvent,
    };
    use dsh_tools::{
        AbortFlag, ApprovalOutcome, Approver, PreToolDecision, TOOL_ABORTED_BEFORE_DISPATCH,
        ToolDefinition, ToolExecution, ToolPresentationMode, ToolRuntime,
    };
    use serde_json::json;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
        let tools = Arc::new(Mutex::new(tools));
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
    async fn execute_copies_host_session_id_onto_tool_execution() {
        let mut session = Session::new(test_header("sess-a"));
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        tools.register(ToolDefinition {
            name: "probe".into(),
            description: "probe".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(move |args, exec| {
                *slot.lock().expect("seen") = exec.session_id.clone();
                Box::pin(async move { Ok(args) })
            }),
            render: Box::new(|_, v| {
                vec![ContentBlock::Text {
                    text: v.to_string(),
                }]
            }),
            is_concurrency_safe: None,
        });
        let tools = Arc::new(Mutex::new(tools));
        let outcome = execute_tool_calls(
            &mut DirectHost {
                session: &mut session,
                tools: &tools,
            },
            1,
            1,
            &[tool_call("c1", "probe", json!({}))],
            &AbortFlag::new(),
            10,
            &mut |_| {},
        )
        .await
        .unwrap();
        assert!(!outcome.aborted);
        let id = seen.lock().expect("seen");
        assert_eq!(id.as_ref().map(|sid| sid.as_str()), Some("sess-a"));
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
        let tools = Arc::new(Mutex::new(tools));
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
        let tools = Arc::new(Mutex::new(tools));
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

    /// Current-thread `#[tokio::test]` parks the only worker if `prepare` uses
    /// `thread::park` `block_on`; a sibling task that opens the pre-hook gate
    /// would never run. A std-thread watchdog fails the process if that happens.
    #[tokio::test(flavor = "current_thread")]
    async fn yielding_prepare_does_not_park_the_runtime_worker() {
        let finished = Arc::new(AtomicBool::new(false));
        let watchdog_flag = Arc::clone(&finished);
        std::thread::spawn(move || {
            for _ in 0..40 {
                if watchdog_flag.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if !watchdog_flag.load(Ordering::SeqCst) {
                eprintln!(
                    "yielding_prepare_does_not_park_the_runtime_worker hung: prepare parked the Tokio worker"
                );
                std::process::exit(101);
            }
        });
        struct StopWatchdog(Arc<AtomicBool>);
        impl Drop for StopWatchdog {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let _stop = StopWatchdog(finished);

        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let gate_rx = Arc::new(Mutex::new(Some(gate_rx)));
        let mut session = Session::new(test_header("yield-prep"));
        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_body = ran.clone();
        tools.register(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
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
            is_concurrency_safe: None,
        });
        tools.on_pre(move |_exec, next| {
            let rx = gate_rx.lock().expect("gate").take().expect("one prepare");
            Box::pin(async move {
                rx.await.expect("sibling released prepare");
                next().await
            })
        });
        tokio::spawn(async move {
            gate_tx.send(()).expect("gate");
        });
        let tools = Arc::new(Mutex::new(tools));
        let mut extra = Vec::new();
        let outcome = execute_tool_calls(
            &mut DirectHost {
                session: &mut session,
                tools: &tools,
            },
            1,
            1,
            &[tool_call("c1", "echo", json!({"id": "a"}))],
            &AbortFlag::new(),
            10,
            &mut |message| extra.push(message),
        )
        .await
        .unwrap();
        assert!(!outcome.aborted);
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    /// SharedHost `session_mut` is `None`. A waiter that runs after the first
    /// length-preserving `with_session` is the two-lock `origin_len` → clone gap.
    #[tokio::test]
    async fn followup_during_shared_prepare_does_not_duplicate_inbox() {
        struct AuditApprover;
        impl Approver for AuditApprover {
            fn decide<'a>(
                &'a self,
                session: &'a mut Session,
                exec: &'a ToolExecution,
                _reason: Option<String>,
            ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send + 'a>> {
                Box::pin(async move {
                    let seq = session.events().len() as u64;
                    session
                        .append(SessionEvent::ApprovalAsked {
                            seq,
                            time: seq as i64,
                            data: json!({
                                "id": "a1",
                                "toolName": exec.name,
                                "callId": exec.call_id.as_str(),
                            }),
                            ignorable: None,
                        })
                        .expect("asked");
                    let seq = session.events().len() as u64;
                    session
                        .append(SessionEvent::ApprovalDecided {
                            seq,
                            time: seq as i64,
                            data: json!({
                                "id": "a1",
                                "outcome": "allowed-once",
                            }),
                            ignorable: None,
                        })
                        .expect("decided");
                    ApprovalOutcome::AllowedOnce
                })
            }
        }

        struct SharedRaceHost {
            session: Session,
            tools: Arc<Mutex<ToolRuntime>>,
            spliced: bool,
        }

        impl ToolCallHost for SharedRaceHost {
            fn with_session<R>(&mut self, f: impl FnOnce(&mut Session) -> R) -> R {
                let before = self.session.events().len();
                let result = f(&mut self.session);
                if before == self.session.events().len() && !self.spliced {
                    self.spliced = true;
                    let seq = self.session.events().len() as u64;
                    self.session
                        .append(SessionEvent::AgentInboxSpliced {
                            seq,
                            time: seq as i64,
                            data: InboxSplicedData {
                                target: InboxTarget::NextTurn,
                                start: 0,
                                removed_count: None,
                                inserted: vec![crate::user_text("followup", "followup")],
                                outcome: None,
                            },
                            ignorable: None,
                        })
                        .expect("followup splice");
                }
                result
            }

            fn tools(&self) -> &Arc<Mutex<ToolRuntime>> {
                &self.tools
            }
        }

        let mut tools = ToolRuntime::new(ToolPresentationMode::Native);
        tools.register(ToolDefinition {
            name: "echo".into(),
            description: "echo".into(),
            parameters: json!({"type": "object"}),
            execute: Box::new(|args, _exec| Box::pin(async move { Ok(args) })),
            render: Box::new(|_, v| {
                vec![ContentBlock::Text {
                    text: v.to_string(),
                }]
            }),
            is_concurrency_safe: None,
        });
        tools.set_approver(Some(Arc::new(AuditApprover)));
        tools.on_pre(|_exec, _next| Box::pin(async { PreToolDecision::Ask { reason: None } }));

        let mut host = SharedRaceHost {
            session: Session::new(test_header("shared-prep")),
            tools: Arc::new(Mutex::new(tools)),
            spliced: false,
        };
        let outcome = execute_tool_calls(
            &mut host,
            1,
            1,
            &[tool_call("c1", "echo", json!({"id": "a"}))],
            &AbortFlag::new(),
            10,
            &mut |_| {},
        )
        .await
        .unwrap();
        assert!(!outcome.aborted);
        let types: Vec<_> = host
            .session
            .events()
            .iter()
            .map(|event| event.event_type().to_string())
            .collect();
        let splices = types
            .iter()
            .filter(|ty| ty.as_str() == "agent/inbox/spliced")
            .count();
        let asked = types
            .iter()
            .filter(|ty| ty.as_str() == "approval/asked")
            .count();
        let decided = types
            .iter()
            .filter(|ty| ty.as_str() == "approval/decided")
            .count();
        assert_eq!(splices, 1, "{types:?}");
        assert_eq!(asked, 1, "{types:?}");
        assert_eq!(decided, 1, "{types:?}");
    }
}
