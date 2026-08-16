//! Basic compaction backend: lock, retain, summarize, replace, overflow retry.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dsh_agent_loop::{
    CompactionScope, EVENT_AGENT_PRE_STEP, EVENT_AGENT_REQUEST_ERROR, LoopOptions, PreStepDecision,
    RequestErrorAction,
};
use dsh_compaction::{
    CompactionEngine, CompactionError, CompactionId, CompactionResult, CompactionTrigger,
    ManualCompactionErrorCode, ShadowedRange, compact_checkpoint_source,
    tool_pairing_balanced_after, tool_pairing_balanced_before,
};
use dsh_kernel::{Context, Next};
use dsh_llm::retry::RetryScope;
use dsh_llm::{CONTEXT_WINDOW_EXCEEDED_CODE, LlmRuntime};
use dsh_session::{
    LogEvent, Message, MessageId, MessageRole, Session, SessionEvent, SessionId, SurfaceOp,
    derive_event_message,
};
use dsh_token_meter::{TokenMeasurement, TokenMeter};
use dsh_tools::AbortFlag;
use serde_json::{Value, json};

use crate::config::{BasicCompactionConfig, resolve_compact_spec, resolve_target_policy};
use crate::pruner::ToolResultPruner;
use crate::summarize::{
    SummarizationInput, SummaryResult, frame_summary, summarize_with_llm, tools_from_header,
};

/// Replay-aware compaction backend using `tokenMeter` pressure and `llm.stream` summarization.
pub struct BasicCompactionEngine {
    llm: Arc<Mutex<LlmRuntime>>,
    meter: Arc<TokenMeter>,
    config: BasicCompactionConfig,
    overflow: Mutex<HashMap<String, OverflowState>>,
    kernel: Option<Context>,
}

struct OverflowState {
    count: u32,
    last_inc_seq: u64,
}

struct PreparedCompaction {
    compaction_id: CompactionId,
    turn: Option<u64>,
    start: u64,
    end: u64,
    shadowed_seqs: Vec<u64>,
    shadowed_token_count: u64,
    start_seq: u64,
    input: SummarizationInput,
    node_fingerprint: Vec<(u64, u64)>,
    session_id: SessionId,
}

struct EntryState {
    open_turn: Option<u64>,
    unmatched_start_seq: Option<u64>,
    latest_end_seed_seq: Option<u64>,
}

impl BasicCompactionEngine {
    /// Bind the LLM runtime, token meter, and validated configuration.
    #[must_use]
    pub fn new(
        llm: Arc<Mutex<LlmRuntime>>,
        meter: Arc<TokenMeter>,
        config: BasicCompactionConfig,
    ) -> Self {
        Self {
            llm,
            meter,
            config,
            overflow: Mutex::new(HashMap::new()),
            kernel: None,
        }
    }

    /// Bind the kernel context used to look up an optional `toolResultPruner`.
    #[must_use]
    pub fn with_kernel(mut self, kernel: Context) -> Self {
        self.kernel = Some(kernel);
        self
    }

    /// Validated configuration used by this backend.
    #[must_use]
    pub fn config(&self) -> &BasicCompactionConfig {
        &self.config
    }

    /// Recover a canonical context overflow through [`CompactionScope`].
    pub(crate) async fn recover_overflow(
        &self,
        action: RequestErrorAction,
        next: Next<RequestErrorAction>,
    ) -> RequestErrorAction {
        let Some(failure) = RetryScope::try_with(|scope| scope.failure().clone()) else {
            return next(action).await;
        };
        if failure.code != CONTEXT_WINDOW_EXCEEDED_CODE {
            return next(action).await;
        }
        let aborted =
            CompactionScope::try_current(|scope| scope.abort().is_aborted()).unwrap_or(true);
        if aborted {
            return next(action).await;
        }
        let Some(snapshot) = CompactionScope::try_current(|scope| {
            scope.with_session(|session| session.replace_generation())
        }) else {
            return next(action).await;
        };
        let retries = CompactionScope::try_current(|scope| {
            scope.with_session(|session| self.overflow_retries(session))
        })
        .unwrap_or(u32::MAX);
        let cap = CompactionScope::try_current(|scope| {
            scope.with_session(|session| self.overflow_cap(session))
        })
        .unwrap_or(0);
        if retries >= cap {
            return next(action).await;
        }
        let _ = self
            .compact_if_needed_scoped(CompactionTrigger::ContextOverflow)
            .await;
        if CompactionScope::try_current(|scope| scope.abort().is_aborted()).unwrap_or(true) {
            return next(action).await;
        }
        let now = CompactionScope::try_current(|scope| {
            scope.with_session(|session| session.replace_generation())
        })
        .unwrap_or(snapshot);
        if now > snapshot {
            CompactionScope::try_current(|scope| {
                scope.with_session(|session| self.bump_overflow(session));
            });
            return RequestErrorAction::Retry;
        }
        next(action).await
    }

    /// Automatic pressure or overflow compaction using the task-local [`CompactionScope`].
    pub(crate) async fn compact_if_needed_scoped(
        &self,
        trigger: CompactionTrigger,
    ) -> Result<Option<CompactionResult>, CompactionError> {
        let Some(options) = CompactionScope::try_current(|scope| scope.options().clone()) else {
            return Ok(None);
        };
        let abort = CompactionScope::try_current(|scope| scope.abort().clone())
            .unwrap_or_else(AbortFlag::new);
        match trigger {
            CompactionTrigger::Pressure => self.compact_pressure_scoped(&options, &abort).await,
            CompactionTrigger::ContextOverflow => {
                self.compact_overflow_scoped(&options, &abort).await
            }
        }
    }

    async fn compact_pressure_scoped(
        &self,
        options: &LoopOptions,
        abort: &AbortFlag,
    ) -> Result<Option<CompactionResult>, CompactionError> {
        let Some(header) = with_session(|session| session.request_header()).flatten() else {
            return Ok(None);
        };
        if header.config.provider.is_empty() || header.config.model.is_empty() {
            return Ok(None);
        }
        if let Err(error) = with_session(|session| {
            assert_no_active_compaction(session, "automatic pressure compaction")
        })
        .unwrap_or(Ok(()))
        {
            return Err(error);
        }
        let policy =
            resolve_target_policy(&self.config, &header.config.provider, &header.config.model);
        let context = self
            .llm
            .lock()
            .expect("llm")
            .prepare_call(&header.config)
            .ok()
            .and_then(|prepared| prepared.context);
        let Some(context) = context else {
            return Err(CompactionError::new(
                ManualCompactionErrorCode::Summary,
                format!(
                    "compaction-basic: no context capacity for {}/{}; \
                     configure contextWindow on that adapter model",
                    header.config.provider, header.config.model
                ),
            ));
        };
        let spec = resolve_compact_spec(
            &policy,
            &header.config.provider,
            &header.config.model,
            context.context_window,
        )
        .map_err(|error| {
            CompactionError::new(ManualCompactionErrorCode::Summary, error.to_string())
        })?;
        let Some(measurement) = with_session(|session| self.meter.measure(session, None)) else {
            return Ok(None);
        };
        if measurement.total_tokens() < spec.threshold_tokens {
            return Ok(None);
        }
        with_session(|session| self.maybe_prune(session));
        let Some(mut measurement) = with_session(|session| self.meter.measure(session, None))
        else {
            return Ok(None);
        };
        if measurement.total_tokens() < spec.threshold_tokens {
            return Ok(None);
        }
        let mut result = None;
        for _attempt in 0..=spec.compaction_retries {
            let range = with_session(|session| {
                select_compactable_range(session, &measurement, spec.retain_tokens)
            })
            .flatten();
            let Some((start, end)) = range else {
                return Ok(result);
            };
            result = Some(
                self.compact_region_scoped(start, end, options, abort)
                    .await?,
            );
            measurement = match with_session(|session| self.meter.measure(session, None)) {
                Some(next) => next,
                None => return Ok(result),
            };
            if measurement.total_tokens() < spec.threshold_tokens {
                return Ok(result);
            }
        }
        Err(CompactionError::new(
            ManualCompactionErrorCode::Summary,
            format!(
                "compaction still above threshold after {} compaction attempts \
                 ({} estimated tokens >= threshold {})",
                spec.compaction_retries + 1,
                measurement.total_tokens(),
                spec.threshold_tokens
            ),
        ))
    }

    async fn compact_overflow_scoped(
        &self,
        options: &LoopOptions,
        abort: &AbortFlag,
    ) -> Result<Option<CompactionResult>, CompactionError> {
        let Some(header) = with_session(|session| session.request_header()).flatten() else {
            return Ok(None);
        };
        if header.config.provider.is_empty() || header.config.model.is_empty() {
            return Ok(None);
        }
        with_session(|session| self.maybe_prune(session));
        let range = with_session(|session| {
            let measurement = self.meter.measure(session, None);
            select_compactable_range(session, &measurement, 0)
        })
        .flatten();
        let Some((start, end)) = range else {
            return Ok(None);
        };
        self.compact_region_scoped(start, end, options, abort)
            .await
            .map(Some)
    }

    async fn compact_region_scoped(
        &self,
        start: u64,
        end: u64,
        options: &LoopOptions,
        abort: &AbortFlag,
    ) -> Result<CompactionResult, CompactionError> {
        let mut prepared = match with_session(|session| self.prepare_region(session, start, end)) {
            Some(prepared) => prepared?,
            None => {
                return Err(CompactionError::new(
                    ManualCompactionErrorCode::Cancelled,
                    "compaction scope missing",
                ));
            }
        };
        let summary = match self.summarize_prepared(&prepared, options, abort).await {
            Ok(summary) => summary,
            Err(error) => {
                let _ = with_session(|session| self.close_error(session, &prepared, &error));
                return Err(error);
            }
        };
        if abort.is_aborted() {
            let error = CompactionError::new(
                ManualCompactionErrorCode::Cancelled,
                "compaction was cancelled",
            );
            let _ = with_session(|session| self.close_error(session, &prepared, &error));
            return Err(error);
        }
        with_session(|session| {
            if let Err(error) = assert_whole_surface_unchanged(session, &self.meter, &prepared) {
                let _ = self.close_error(session, &prepared, &error);
                return Err(error);
            }
            if let Err(error) = self.assert_shrink(&prepared, &summary) {
                let _ = self.close_error(session, &prepared, &error);
                return Err(error);
            }
            self.commit_region(session, &mut prepared, summary)
        })
        .unwrap_or_else(|| {
            Err(CompactionError::new(
                ManualCompactionErrorCode::Cancelled,
                "compaction scope missing",
            ))
        })
    }

    async fn compact_region_inner(
        &self,
        start: u64,
        end: u64,
        session: &mut Session,
        options: &LoopOptions,
        abort: &AbortFlag,
    ) -> Result<CompactionResult, CompactionError> {
        let mut prepared = self.prepare_region(session, start, end)?;
        let summary = match self.summarize_prepared(&prepared, options, abort).await {
            Ok(summary) => summary,
            Err(error) => {
                let _ = self.close_error(session, &prepared, &error);
                return Err(error);
            }
        };
        if abort.is_aborted() {
            let error = CompactionError::new(
                ManualCompactionErrorCode::Cancelled,
                "compaction was cancelled",
            );
            let _ = self.close_error(session, &prepared, &error);
            return Err(error);
        }
        if let Err(error) = assert_whole_surface_unchanged(session, &self.meter, &prepared) {
            let _ = self.close_error(session, &prepared, &error);
            return Err(error);
        }
        if let Err(error) = self.assert_shrink(&prepared, &summary) {
            let _ = self.close_error(session, &prepared, &error);
            return Err(error);
        }
        self.commit_region(session, &mut prepared, summary)
    }

    async fn compact_if_needed_inner(
        &self,
        session: &mut Session,
        options: &LoopOptions,
        trigger: CompactionTrigger,
        abort: &AbortFlag,
    ) -> Result<Option<CompactionResult>, CompactionError> {
        let Some(header) = session.request_header() else {
            return Ok(None);
        };
        if header.config.provider.is_empty() || header.config.model.is_empty() {
            return Ok(None);
        }
        match trigger {
            CompactionTrigger::ContextOverflow => {
                self.maybe_prune(session);
                let measurement = self.meter.measure(session, None);
                let Some((start, end)) = select_compactable_range(session, &measurement, 0) else {
                    return Ok(None);
                };
                self.compact_region_inner(start, end, session, options, abort)
                    .await
                    .map(Some)
            }
            CompactionTrigger::Pressure => {
                assert_no_active_compaction(session, "automatic pressure compaction")?;
                let policy = resolve_target_policy(
                    &self.config,
                    &header.config.provider,
                    &header.config.model,
                );
                let context = self
                    .llm
                    .lock()
                    .expect("llm")
                    .prepare_call(&header.config)
                    .ok()
                    .and_then(|prepared| prepared.context);
                let Some(context) = context else {
                    return Err(CompactionError::new(
                        ManualCompactionErrorCode::Summary,
                        format!(
                            "compaction-basic: no context capacity for {}/{}",
                            header.config.provider, header.config.model
                        ),
                    ));
                };
                let spec = resolve_compact_spec(
                    &policy,
                    &header.config.provider,
                    &header.config.model,
                    context.context_window,
                )
                .map_err(|error| {
                    CompactionError::new(ManualCompactionErrorCode::Summary, error.to_string())
                })?;
                let mut measurement = self.meter.measure(session, None);
                if measurement.total_tokens() < spec.threshold_tokens {
                    return Ok(None);
                }
                self.maybe_prune(session);
                measurement = self.meter.measure(session, None);
                if measurement.total_tokens() < spec.threshold_tokens {
                    return Ok(None);
                }
                let mut result = None;
                for _attempt in 0..=spec.compaction_retries {
                    let Some((start, end)) =
                        select_compactable_range(session, &measurement, spec.retain_tokens)
                    else {
                        return Ok(result);
                    };
                    result = Some(
                        self.compact_region_inner(start, end, session, options, abort)
                            .await?,
                    );
                    measurement = self.meter.measure(session, None);
                    if measurement.total_tokens() < spec.threshold_tokens {
                        return Ok(result);
                    }
                }
                Err(CompactionError::new(
                    ManualCompactionErrorCode::Summary,
                    format!(
                        "compaction still above threshold after {} compaction attempts",
                        spec.compaction_retries + 1
                    ),
                ))
            }
        }
    }

    fn prepare_region(
        &self,
        session: &mut Session,
        start: u64,
        end: u64,
    ) -> Result<PreparedCompaction, CompactionError> {
        let selection = validate_surface_region(session, start, end)?;
        let entry = inspect_compaction_entry_state(session);
        assert_compaction_inactive(
            entry.unmatched_start_seq,
            entry.latest_end_seed_seq,
            "compaction",
        )?;
        let turn = match entry.open_turn {
            Some(turn) => Some(turn),
            None => {
                return Err(CompactionError::new(
                    ManualCompactionErrorCode::Busy,
                    "compactRegion: no open turn — automatic compaction events must be enclosed in a turn",
                ));
            }
        };
        let measurement = self.meter.measure(session, None);
        let start_idx = session
            .surface_nodes()
            .iter()
            .position(|seq| *seq == start)
            .ok_or_else(|| {
                CompactionError::new(
                    ManualCompactionErrorCode::Changed,
                    format!("compactRegion: start seq {start} not found in surface"),
                )
            })?;
        let end_idx = session
            .surface_nodes()
            .iter()
            .position(|seq| *seq == end)
            .ok_or_else(|| {
                CompactionError::new(
                    ManualCompactionErrorCode::Changed,
                    format!("compactRegion: end seq {end} not found in surface"),
                )
            })?;
        let selected = &measurement.nodes()[start_idx..=end_idx];
        let shadowed_token_count = selected.iter().map(|node| node.tokens()).sum();
        let node_fingerprint = measurement
            .nodes()
            .iter()
            .map(|node| (node.seq(), node.tokens()))
            .collect();
        let input = build_summarization_input(session, &selection.shadowed_seqs);
        let compaction_id = next_compaction_id();
        let start_seq = session.events().len() as u64;
        let mut data = json!({
            "compactionId": compaction_id.as_str(),
        });
        if let Some(turn) = turn {
            data["turn"] = json!(turn);
        }
        session
            .append(SessionEvent::CompactionStart {
                seq: start_seq,
                time: start_seq as i64,
                data,
                ignorable: None,
            })
            .map_err(|error| {
                CompactionError::new(ManualCompactionErrorCode::Commit, error.to_string())
            })?;
        Ok(PreparedCompaction {
            compaction_id,
            turn,
            start: selection.start,
            end: selection.end,
            shadowed_seqs: selection.shadowed_seqs,
            shadowed_token_count,
            start_seq,
            input,
            node_fingerprint,
            session_id: session.id().clone(),
        })
    }

    async fn summarize_prepared(
        &self,
        prepared: &PreparedCompaction,
        options: &LoopOptions,
        abort: &AbortFlag,
    ) -> Result<SummaryResult, CompactionError> {
        summarize_with_llm(
            &self.llm,
            &self.config,
            &prepared.input,
            prepared.session_id.clone(),
            &options.provider,
            &options.model,
            abort,
        )
        .await
        .map_err(|message| CompactionError::new(ManualCompactionErrorCode::Summary, message))
    }

    fn assert_shrink(
        &self,
        prepared: &PreparedCompaction,
        summary: &SummaryResult,
    ) -> Result<(), CompactionError> {
        let checkpoint = Message {
            id: MessageId::new("shrink-check"),
            role: MessageRole::User,
            content: frame_summary(&summary.summary),
            source: compact_checkpoint_source(&prepared.compaction_id),
        };
        let framed = self.meter.estimate_message(&checkpoint);
        if framed >= prepared.shadowed_token_count {
            return Err(CompactionError::new(
                ManualCompactionErrorCode::Summary,
                format!(
                    "summary is not smaller than the shadowed content ({framed} estimated framed tokens >= {})",
                    prepared.shadowed_token_count
                ),
            ));
        }
        Ok(())
    }

    fn commit_region(
        &self,
        session: &mut Session,
        prepared: &mut PreparedCompaction,
        summary: SummaryResult,
    ) -> Result<CompactionResult, CompactionError> {
        let summary_seq = session.events().len() as u64;
        let mut data = json!({
            "compactionId": prepared.compaction_id.as_str(),
            "summary": summary.summary,
            "rawOutput": summary.raw_output,
            "llmStreamCall": true,
            "shadowedRange": { "start": prepared.start, "end": prepared.end },
            "shadowedSeqs": prepared.shadowed_seqs,
            "shadowedTokenCount": prepared.shadowed_token_count,
            "provider": summary.provider,
            "model": summary.model,
            "maxTokens": summary.max_tokens,
        });
        if let Some(usage) = &summary.usage {
            data["usage"] = serde_json::to_value(usage).unwrap_or(Value::Null);
        }
        session
            .append(SessionEvent::CompactionSummary {
                seq: summary_seq,
                time: summary_seq as i64,
                data,
                ignorable: None,
            })
            .map_err(|error| {
                CompactionError::new(ManualCompactionErrorCode::Commit, error.to_string())
            })?;
        let mut source_event_seqs = vec![prepared.start_seq, summary_seq];
        source_event_seqs.extend(prepared.shadowed_seqs.iter().copied());
        let replace_seq = session.events().len() as u64;
        let checkpoint = Message {
            id: MessageId::new(format!("compact-{}", prepared.compaction_id.as_str())),
            role: MessageRole::User,
            content: frame_summary(&summary.summary),
            source: compact_checkpoint_source(&prepared.compaction_id),
        };
        session
            .append(SessionEvent::UserMessage {
                seq: replace_seq,
                time: replace_seq as i64,
                data: checkpoint,
                surface_op: Some(SurfaceOp::Replace {
                    start: prepared.start,
                    end: prepared.end,
                }),
                source_event_seqs: Some(source_event_seqs),
                ignorable: None,
            })
            .map_err(|error| {
                CompactionError::new(ManualCompactionErrorCode::Commit, error.to_string())
            })?;
        let end_seq = session.events().len() as u64;
        let mut end_data = json!({
            "compactionId": prepared.compaction_id.as_str(),
        });
        if let Some(turn) = prepared.turn {
            end_data["turn"] = json!(turn);
        }
        session
            .append(SessionEvent::CompactionEnd {
                seq: end_seq,
                time: end_seq as i64,
                data: end_data,
                ignorable: None,
            })
            .map_err(|error| {
                CompactionError::new(ManualCompactionErrorCode::Commit, error.to_string())
            })?;
        Ok(CompactionResult::new(
            prepared.compaction_id.clone(),
            prepared.start_seq,
            summary_seq,
            end_seq,
            summary.summary,
            ShadowedRange {
                start: prepared.start,
                end: prepared.end,
            },
            prepared.shadowed_seqs.clone(),
            prepared.shadowed_token_count,
        ))
    }

    fn close_error(
        &self,
        session: &mut Session,
        prepared: &PreparedCompaction,
        error: &CompactionError,
    ) -> Result<(), CompactionError> {
        let seq = session.events().len() as u64;
        let mut data = json!({
            "compactionId": prepared.compaction_id.as_str(),
            "error": error.to_string(),
        });
        if let Some(turn) = prepared.turn {
            data["turn"] = json!(turn);
        }
        session
            .append(SessionEvent::CompactionEnd {
                seq,
                time: seq as i64,
                data,
                ignorable: None,
            })
            .map(|_| ())
            .map_err(|error| {
                CompactionError::new(ManualCompactionErrorCode::Commit, error.to_string())
            })
    }

    fn maybe_prune(&self, session: &mut Session) {
        let Some(kernel) = &self.kernel else {
            return;
        };
        if let Some(pruner) = kernel.get::<ToolResultPruner>("toolResultPruner") {
            pruner.prune_session(session);
        }
    }

    fn overflow_cap(&self, session: &Session) -> u32 {
        match session.request_header() {
            Some(header) => {
                resolve_target_policy(&self.config, &header.config.provider, &header.config.model)
                    .max_overflow_retries
            }
            None => self.config.max_overflow_retries,
        }
    }

    fn overflow_retries(&self, session: &Session) -> u32 {
        let key = session.id().as_str().to_string();
        let last_assistant = last_assistant_seq(session);
        let map = self.overflow.lock().expect("overflow");
        match map.get(&key) {
            Some(state) => match last_assistant {
                Some(seq) if seq > state.last_inc_seq => 0,
                Some(_) | None => state.count,
            },
            None => 0,
        }
    }

    fn bump_overflow(&self, session: &Session) {
        let key = session.id().as_str().to_string();
        let last_seq = session.events().len() as u64;
        let mut map = self.overflow.lock().expect("overflow");
        let next_count = match map.get(&key) {
            Some(state) => match last_assistant_seq(session) {
                Some(seq) if seq > state.last_inc_seq => 1,
                Some(_) | None => state.count + 1,
            },
            None => 1,
        };
        map.insert(
            key,
            OverflowState {
                count: next_count,
                last_inc_seq: last_seq,
            },
        );
    }
}

impl CompactionEngine for BasicCompactionEngine {
    fn compact_if_needed<'a>(
        &'a self,
        session: &'a mut Session,
        options: &'a LoopOptions,
        trigger: CompactionTrigger,
        signal: &'a AbortFlag,
    ) -> Pin<Box<dyn Future<Output = Result<Option<CompactionResult>, CompactionError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.compact_if_needed_inner(session, options, trigger, signal)
                .await
        })
    }

    fn compact_region<'a>(
        &'a self,
        start: u64,
        end: u64,
        session: &'a mut Session,
        options: &'a LoopOptions,
        signal: &'a AbortFlag,
    ) -> Pin<Box<dyn Future<Output = Result<CompactionResult, CompactionError>> + Send + 'a>> {
        Box::pin(async move {
            self.compact_region_inner(start, end, session, options, signal)
                .await
        })
    }
}

struct SurfaceSelection {
    start: u64,
    end: u64,
    shadowed_seqs: Vec<u64>,
}

fn with_session<R>(f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    CompactionScope::try_current(|scope| scope.with_session(f))
}

fn next_compaction_id() -> CompactionId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    CompactionId::new(format!("00000000-0000-4000-8000-{n:012x}"))
}

fn select_compactable_range(
    session: &Session,
    measurement: &TokenMeasurement,
    retain_tokens: u64,
) -> Option<(u64, u64)> {
    let priced_nodes = measurement.nodes();
    if priced_nodes.is_empty() {
        return None;
    }
    let surface_nodes = session.surface_nodes();
    if surface_nodes.len() != priced_nodes.len()
        || surface_nodes
            .iter()
            .zip(priced_nodes.iter())
            .any(|(seq, node)| *seq != node.seq())
    {
        panic!("compaction: token-meter surface does not match the current session surface");
    }
    let mut accumulated = 0_u64;
    let mut keep_from_idx = priced_nodes.len();
    for index in (0..priced_nodes.len()).rev() {
        accumulated = accumulated.saturating_add(priced_nodes[index].tokens());
        keep_from_idx = index;
        if accumulated >= retain_tokens {
            break;
        }
    }
    if keep_from_idx == 0 {
        return None;
    }
    while keep_from_idx > 0 {
        let before_ok = tool_pairing_balanced_before(session, surface_nodes[keep_from_idx]);
        let after_ok = tool_pairing_balanced_after(session, surface_nodes[keep_from_idx - 1]);
        if before_ok && after_ok {
            break;
        }
        keep_from_idx -= 1;
    }
    if keep_from_idx == 0 {
        return None;
    }
    let first = surface_nodes[0];
    let cutoff = surface_nodes[keep_from_idx - 1];
    Some((first, cutoff))
}

fn validate_surface_region(
    session: &Session,
    start: u64,
    end: u64,
) -> Result<SurfaceSelection, CompactionError> {
    let nodes = session.surface_nodes();
    let start_idx = nodes.iter().position(|seq| *seq == start).ok_or_else(|| {
        CompactionError::new(
            ManualCompactionErrorCode::Changed,
            format!("compactRegion: start seq {start} not found in surface"),
        )
    })?;
    let end_idx = nodes.iter().position(|seq| *seq == end).ok_or_else(|| {
        CompactionError::new(
            ManualCompactionErrorCode::Changed,
            format!("compactRegion: end seq {end} not found in surface"),
        )
    })?;
    if start_idx > end_idx {
        return Err(CompactionError::new(
            ManualCompactionErrorCode::Changed,
            format!(
                "compactRegion: start seq {start} (position {start_idx}) is after end seq {end} (position {end_idx}) on the surface"
            ),
        ));
    }
    if !tool_pairing_balanced_before(session, nodes[start_idx]) {
        return Err(CompactionError::new(
            ManualCompactionErrorCode::Changed,
            format!(
                "compactRegion: start seq {start} is not a balanced boundary (would split a step's tool-call/result pair)"
            ),
        ));
    }
    if !tool_pairing_balanced_after(session, nodes[end_idx]) {
        return Err(CompactionError::new(
            ManualCompactionErrorCode::Changed,
            format!(
                "compactRegion: end seq {end} is not a balanced boundary (would split a step, or the step is still open)"
            ),
        ));
    }
    Ok(SurfaceSelection {
        start,
        end,
        shadowed_seqs: nodes[start_idx..=end_idx].to_vec(),
    })
}

fn inspect_compaction_entry_state(session: &Session) -> EntryState {
    let mut open_turn = None;
    let mut open_known = false;
    let mut unmatched_start_seq = None;
    let mut compaction_known = false;
    let mut latest_end_seed_seq = None;
    for event in session.events().iter().rev() {
        let LogEvent::Known(event) = event else {
            continue;
        };
        if latest_end_seed_seq.is_none() && event.event_type() == "session/end-seed" {
            latest_end_seed_seq = Some(event.seq());
        }
        if !compaction_known {
            match event.event_type() {
                "compaction/start" => {
                    unmatched_start_seq = Some(event.seq());
                    compaction_known = true;
                }
                "compaction/end" => compaction_known = true,
                _ => {}
            }
        }
        if !open_known {
            match event.event_type() {
                "turn/start" => {
                    if let SessionEvent::TurnStart { data, .. } = event {
                        open_turn = Some(data.turn);
                    }
                    open_known = true;
                }
                "turn/end" => open_known = true,
                _ => {}
            }
        }
        if open_known && compaction_known && latest_end_seed_seq.is_some() {
            break;
        }
    }
    EntryState {
        open_turn,
        unmatched_start_seq,
        latest_end_seed_seq,
    }
}

fn assert_compaction_inactive(
    unmatched_start: Option<u64>,
    latest_end_seed: Option<u64>,
    stage: &str,
) -> Result<(), CompactionError> {
    match (unmatched_start, latest_end_seed) {
        (None, _) => Ok(()),
        (Some(start), Some(seed)) if seed > start => Ok(()),
        (Some(_), _) => Err(CompactionError::new(
            ManualCompactionErrorCode::Busy,
            format!(
                "{stage}: compaction already in progress; the session compaction lock is already active"
            ),
        )),
    }
}

fn assert_no_active_compaction(session: &Session, stage: &str) -> Result<(), CompactionError> {
    let entry = inspect_compaction_entry_state(session);
    assert_compaction_inactive(entry.unmatched_start_seq, entry.latest_end_seed_seq, stage)
}

fn assert_whole_surface_unchanged(
    session: &Session,
    meter: &TokenMeter,
    prepared: &PreparedCompaction,
) -> Result<(), CompactionError> {
    let current = meter.measure(session, None);
    let fingerprint: Vec<(u64, u64)> = current
        .nodes()
        .iter()
        .map(|node| (node.seq(), node.tokens()))
        .collect();
    if fingerprint != prepared.node_fingerprint {
        return Err(CompactionError::new(
            ManualCompactionErrorCode::Changed,
            "compaction: session surface changed during summarization",
        ));
    }
    Ok(())
}

fn build_summarization_input(session: &Session, shadowed_seqs: &[u64]) -> SummarizationInput {
    let header = session.request_header();
    let messages = shadowed_seqs
        .iter()
        .filter_map(|seq| match session.events().get(*seq as usize) {
            Some(LogEvent::Known(event)) => derive_event_message(event),
            Some(LogEvent::Leftover(_)) | None => None,
        })
        .collect();
    SummarizationInput {
        system: header.as_ref().and_then(|header| header.system.clone()),
        tools: tools_from_header(header.as_ref().and_then(|header| header.tools.as_ref())),
        messages,
        provider: header.as_ref().map(|header| header.config.provider.clone()),
        model: header.as_ref().map(|header| header.config.model.clone()),
    }
}

fn last_assistant_seq(session: &Session) -> Option<u64> {
    session.events().iter().rev().find_map(|event| match event {
        LogEvent::Known(SessionEvent::AssistantMessage { seq, .. }) => Some(*seq),
        _ => None,
    })
}

/// Register automatic pre-step pressure and overflow-recovery listeners.
pub(crate) fn register_automatic_listeners(ctx: &Context) {
    let pre_ctx = ctx.clone();
    let _ =
        ctx.on_waterfall::<PreStepDecision, _, _>(EVENT_AGENT_PRE_STEP, move |decision, next| {
            let ctx = pre_ctx.clone();
            async move {
                if let Some(engine) = ctx.get::<BasicCompactionEngine>("compaction") {
                    let aborted = CompactionScope::try_current(|scope| scope.abort().is_aborted())
                        .unwrap_or(false);
                    if !aborted {
                        let _ = engine
                            .compact_if_needed_scoped(CompactionTrigger::Pressure)
                            .await;
                    }
                }
                next(decision).await
            }
        });
    let err_ctx = ctx.clone();
    let _ = ctx.on_waterfall::<RequestErrorAction, _, _>(
        EVENT_AGENT_REQUEST_ERROR,
        move |action, next| {
            let ctx = err_ctx.clone();
            async move {
                let Some(engine) = ctx.get::<BasicCompactionEngine>("compaction") else {
                    return next(action).await;
                };
                engine.recover_overflow(action, next).await
            }
        },
    );
}

/// Provide `tokenMeter`, `llm`, and `compaction` on `ctx`, and register auto listeners when `config.auto`.
pub fn install_compaction_auto(ctx: &Context, config: BasicCompactionConfig) {
    if ctx.get::<TokenMeter>("tokenMeter").is_none() {
        ctx.provide("tokenMeter", TokenMeter::new())
            .expect("tokenMeter");
    }
    if ctx.get::<Mutex<LlmRuntime>>("llm").is_none() {
        ctx.provide("llm", Mutex::new(LlmRuntime::new()))
            .expect("llm");
    }
    let llm = ctx.get::<Mutex<LlmRuntime>>("llm").expect("llm");
    let meter = ctx.get::<TokenMeter>("tokenMeter").expect("tokenMeter");
    let auto = config.auto;
    ctx.provide(
        "compaction",
        BasicCompactionEngine::new(llm, meter, config).with_kernel(ctx.clone()),
    )
    .expect("compaction");
    if auto {
        register_automatic_listeners(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::{BasicCompactionEngine, install_compaction_auto};
    use crate::config::BasicCompactionConfig;
    use dsh_agent_loop::{LoopAgent, LoopOptions};
    use dsh_compaction::{CompactionEngine, is_compact_checkpoint_source};
    use dsh_kernel::Context;
    use dsh_llm::{
        CONTEXT_WINDOW_EXCEEDED_CODE, LlmError, LlmRuntime, MockAdapter, MockScript, text_response,
    };
    use dsh_session::{
        ContentBlock, EpochHeader, LlmCallConfig, LogEvent, Message, MessageId, MessageRole,
        MessageSource, RequestHeaderData, RequestHeaderReason, SESSION_FORMAT_VERSION, Session,
        SessionEvent, SessionHeader, SessionId, SurfaceOp, TurnEndReason, TurnStartData,
    };
    use dsh_system_prompt::{SystemPrompt, SystemPromptConfig};
    use dsh_token_meter::TokenMeter;
    use dsh_tools::{AbortFlag, ToolPresentationMode, ToolRuntime};
    use std::sync::{Arc, Mutex};

    fn test_cfg() -> BasicCompactionConfig {
        BasicCompactionConfig::default()
    }

    fn opts() -> LoopOptions {
        LoopOptions {
            provider: "mock".into(),
            model: "mock".into(),
            max_tokens: None,
            max_parallel_tool_calls: 10,
        }
    }

    fn session_header(id: &str) -> SessionHeader {
        SessionHeader {
            version: SESSION_FORMAT_VERSION,
            id: SessionId::new(id),
            created_at: 1,
            cwd: None,
            parent_session: None,
            seed_length: None,
            origin: None,
            delegation_depth: None,
            agent_preset: None,
        }
    }

    fn user_text(id: &str, text: &str) -> Message {
        Message {
            id: MessageId::new(id),
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            source: MessageSource::User,
        }
    }

    fn user_message(seq: u64, text: &str) -> SessionEvent {
        SessionEvent::UserMessage {
            seq,
            time: seq as i64,
            data: user_text(&format!("m{seq}"), text),
            surface_op: Some(SurfaceOp::Append),
            source_event_seqs: None,
            ignorable: None,
        }
    }

    fn long_history() -> String {
        "alpha ".repeat(800)
    }

    fn append(session: &mut Session, event: SessionEvent) {
        session.append(event).expect("append");
    }

    fn engine_with_history() -> (Session, BasicCompactionEngine) {
        let adapter = Arc::new(MockAdapter::new(vec![MockScript::Chunks(text_response(
            "short",
        ))]));
        let mut llm = LlmRuntime::new();
        llm.register_adapter("mock", adapter);
        let engine = BasicCompactionEngine::new(
            Arc::new(Mutex::new(llm)),
            Arc::new(TokenMeter::new()),
            test_cfg(),
        );
        let mut session = Session::new(session_header("compact-region"));
        append(
            &mut session,
            SessionEvent::TurnStart {
                seq: 0,
                time: 0,
                data: TurnStartData { turn: 1 },
                ignorable: None,
            },
        );
        append(&mut session, user_message(1, &long_history()));
        append(&mut session, user_message(2, "tail"));
        (session, engine)
    }

    fn first_seq(session: &Session) -> u64 {
        session.surface_nodes()[0]
    }

    fn last_compactable(session: &Session) -> u64 {
        let nodes = session.surface_nodes();
        if nodes.len() >= 2 {
            nodes[nodes.len() - 2]
        } else {
            nodes[0]
        }
    }

    fn count_type(session: &Session, ty: &str) -> usize {
        session
            .events()
            .iter()
            .filter(|event| event.event_type() == ty)
            .count()
    }

    fn last_user_source(session: &Session) -> &MessageSource {
        session
            .events()
            .iter()
            .rev()
            .find_map(|event| match event {
                LogEvent::Known(SessionEvent::UserMessage { data, .. }) => Some(&data.source),
                _ => None,
            })
            .expect("user message")
    }

    fn request_header(seq: u64) -> SessionEvent {
        SessionEvent::RequestHeader {
            seq,
            time: seq as i64,
            data: RequestHeaderData {
                header: EpochHeader {
                    config: LlmCallConfig {
                        provider: "mock".into(),
                        model: "mock".into(),
                        reasoning_effort: None,
                        temperature: None,
                        max_tokens: None,
                        stop: None,
                    },
                    adapter_defaults: None,
                    system: None,
                    tools: None,
                },
                reason: RequestHeaderReason::Initial,
            },
            ignorable: None,
        }
    }

    fn seed_overflow_history(session: &mut Session) {
        append(session, user_message(0, &long_history()));
        append(session, request_header(1));
    }

    fn agent_from_ctx(ctx: Context, script: Vec<MockScript>, id: &str) -> LoopAgent {
        let llm = ctx.get::<Mutex<LlmRuntime>>("llm").expect("llm");
        let adapter = Arc::new(MockAdapter::new(script));
        llm.lock().expect("llm").register_adapter("mock", adapter);
        let mut session = Session::new(session_header(id));
        seed_overflow_history(&mut session);
        LoopAgent::new(
            ctx,
            session,
            opts(),
            Arc::new(Mutex::new(ToolRuntime::new(ToolPresentationMode::Native))),
            SystemPrompt::new(SystemPromptConfig::default()).unwrap(),
            llm,
        )
        .unwrap()
    }

    fn agent_overflow_then_ok(ctx: Context) -> LoopAgent {
        agent_from_ctx(
            ctx,
            vec![
                MockScript::Fail(LlmError::new("overflow", CONTEXT_WINDOW_EXCEEDED_CODE)),
                MockScript::Chunks(text_response("short")),
                MockScript::Chunks(text_response("after-compact")),
            ],
            "overflow-retry",
        )
    }

    fn agent_overflow_only(ctx: Context) -> LoopAgent {
        agent_from_ctx(
            ctx,
            vec![MockScript::Fail(LlmError::new(
                "overflow",
                CONTEXT_WINDOW_EXCEEDED_CODE,
            ))],
            "overflow-fail",
        )
    }

    fn last_assistant_text(agent: &LoopAgent) -> String {
        agent
            .session
            .events()
            .iter()
            .rev()
            .find_map(|event| match event {
                LogEvent::Known(SessionEvent::AssistantMessage { data, .. }) => {
                    data.message.content.iter().find_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    fn last_turn_end(agent: &LoopAgent) -> Option<&TurnEndReason> {
        agent
            .session
            .events()
            .iter()
            .rev()
            .find_map(|event| match event {
                LogEvent::Known(SessionEvent::TurnEnd { data, .. }) => Some(&data.reason),
                _ => None,
            })
    }

    #[tokio::test]
    async fn compact_region_writes_lock_summary_replace_end() {
        let (mut session, engine) = engine_with_history();
        let before = session.replace_generation();
        let result = engine
            .compact_region(
                first_seq(&session),
                last_compactable(&session),
                &mut session,
                &opts(),
                &AbortFlag::new(),
            )
            .await
            .unwrap();
        assert!(session.replace_generation() > before);
        assert!(is_compact_checkpoint_source(last_user_source(&session)));
        assert_eq!(count_type(&session, "compaction/start"), 1);
        assert_eq!(count_type(&session, "compaction/end"), 1);
        assert!(!result.shadowed_seqs().is_empty());
    }

    #[tokio::test]
    async fn overflow_retries_only_when_replace_generation_advances() {
        let ctx = Context::new();
        install_compaction_auto(
            &ctx,
            BasicCompactionConfig {
                max_overflow_retries: 1,
                ..test_cfg()
            },
        );
        let mut agent = agent_overflow_then_ok(ctx);
        let gen_before = agent.session.replace_generation();
        agent.followup(user_text("overflow", "overflow")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(agent.session.replace_generation() > gen_before);
        assert_eq!(last_assistant_text(&agent), "after-compact");
    }

    #[tokio::test]
    async fn overflow_without_generation_change_fails_the_turn() {
        let ctx = Context::new();
        install_compaction_auto(
            &ctx,
            BasicCompactionConfig {
                auto: false,
                ..test_cfg()
            },
        );
        let mut agent = agent_overflow_only(ctx);
        agent.followup(user_text("overflow", "overflow")).unwrap();
        agent.run_until_idle().await.unwrap();
        assert!(matches!(
            last_turn_end(&agent),
            Some(TurnEndReason::Error { .. })
        ));
    }
}
