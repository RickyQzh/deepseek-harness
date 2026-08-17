//! Persistent PTY session over the subprocess terminal handle.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use dsh_subprocess::{
    SubprocessError, SubprocessOutcome, SubprocessTerminalForeground, SubprocessTerminalHandle,
    SubprocessTerminalSignal,
};
use dsh_terminal::{
    TerminalBackendSession, TerminalError, TerminalErrorCode, TerminalReadRequest,
    TerminalReadResult, TerminalSendOperation, TerminalSendRead, TerminalSendRequest,
    TerminalSendResult, TerminalSessionStatus, TerminalSignal, TerminalSignalResult,
    TerminalWaitReason,
};
use tokio::sync::oneshot;

use crate::config::ResolvedConfig;
use crate::sanitize::{CONTROLLED_PROMPT, TerminalSanitizer};

fn pty_error(message: impl Into<String>) -> TerminalError {
    TerminalError::new(message, TerminalErrorCode::NoBackend)
}

fn lock_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn utf8_tail(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let mut bytes = 0usize;
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        let next = ch.len_utf8();
        if bytes.saturating_add(next) > max_bytes {
            break;
        }
        bytes = bytes.saturating_add(next);
        start = idx;
    }
    (text[start..].to_string(), true)
}

struct BoundedTextBuffer {
    value: String,
    dropped: bool,
    max_bytes: usize,
    max_lines: Option<usize>,
}

impl BoundedTextBuffer {
    fn new(max_bytes: usize, max_lines: Option<usize>) -> Self {
        Self {
            value: String::new(),
            dropped: false,
            max_bytes,
            max_lines,
        }
    }

    fn append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.value.push_str(text);
        if let Some(max_lines) = self.max_lines {
            let count = self.value.split('\n').count();
            if count > max_lines {
                let skip = count - max_lines;
                self.value = self
                    .value
                    .split('\n')
                    .skip(skip)
                    .collect::<Vec<_>>()
                    .join("\n");
                self.dropped = true;
            }
        }
        let (tail, truncated) = utf8_tail(&self.value, self.max_bytes);
        self.value = tail;
        self.dropped |= truncated;
    }

    fn consume(&mut self) -> TerminalSendRead {
        let delta = std::mem::take(&mut self.value);
        let truncated = self.dropped;
        self.dropped = false;
        TerminalSendRead::new(delta, truncated)
    }

    fn snapshot(&self) -> (String, bool) {
        (self.value.clone(), self.dropped)
    }

    fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
}

struct ActiveSend {
    id: u64,
    output: BoundedTextBuffer,
    settle_tx: Option<oneshot::Sender<TerminalSendResult>>,
    cancel_requested: bool,
    interrupting: bool,
    writing: bool,
    finished: bool,
    started_at: Instant,
    initial_foreground_pgid: Option<i32>,
    initial_foreground_left_wait: bool,
}

impl ActiveSend {
    fn set_initial_foreground(&mut self, foreground: Option<&SubprocessTerminalForeground>) {
        self.initial_foreground_pgid =
            foreground.map(SubprocessTerminalForeground::process_group_id);
        self.initial_foreground_left_wait = match foreground {
            Some(fg) => !fg.input_waiting(),
            None => true,
        };
    }

    fn accepts_stdin_wait(&mut self, pgid: i32, waiting: bool) -> bool {
        // The same group may still expose the wait that existed before write.
        // Observe every poll so a departure before the exact-settlement threshold
        // still makes a later return to that wait post-write evidence.
        if Some(pgid) != self.initial_foreground_pgid {
            return waiting;
        }
        if !waiting {
            self.initial_foreground_left_wait = true;
        }
        waiting && self.initial_foreground_left_wait
    }
}

struct Inner {
    config: ResolvedConfig,
    terminal: SubprocessTerminalHandle,
    sanitizer: TerminalSanitizer,
    scrollback: BoundedTextBuffer,
    status: TerminalSessionStatus,
    active: Option<ActiveSend>,
    prompt_seen: bool,
    prompt_text_seen: bool,
    prompt_tail: String,
    shell_pgid: Option<i32>,
    initializing: bool,
    last_output_at: Instant,
    closing: bool,
    close_started: bool,
    poll_gen: u64,
    next_send_id: u64,
    output_ended: bool,
    pump_started: bool,
    output_rx: Option<tokio::sync::broadcast::Receiver<String>>,
}

/// Backend session wrapping one provider-owned terminal process.
#[derive(Clone)]
pub(crate) struct LocalPtySession {
    inner: Arc<Mutex<Inner>>,
    motd: Arc<OnceLock<String>>,
    pid: i32,
}

impl LocalPtySession {
    pub(crate) fn new(terminal: SubprocessTerminalHandle, config: ResolvedConfig) -> Self {
        let pid = terminal.pid();
        let output_rx = terminal.output();
        let max_read = config.max_read_bytes;
        Self {
            inner: Arc::new(Mutex::new(Inner {
                sanitizer: TerminalSanitizer::new(max_read),
                scrollback: BoundedTextBuffer::new(
                    config.scrollback_max_bytes,
                    Some(config.scrollback_lines),
                ),
                terminal,
                config,
                status: TerminalSessionStatus::Running,
                active: None,
                prompt_seen: false,
                prompt_text_seen: false,
                prompt_tail: String::new(),
                shell_pgid: None,
                initializing: false,
                last_output_at: Instant::now(),
                closing: false,
                close_started: false,
                poll_gen: 0,
                next_send_id: 0,
                output_ended: false,
                pump_started: false,
                output_rx: Some(output_rx),
            })),
            motd: Arc::new(OnceLock::new()),
            pid,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock_poison(&self.inner)
    }

    /// Capture startup output through the same readiness contract as later sends.
    pub(crate) async fn initialize(&self) -> Result<(), TerminalError> {
        {
            let mut inner = self.lock();
            inner.initializing = true;
        }
        let operation = self.start_send(TerminalSendRequest::new("", false));
        let result = operation.done().await;
        {
            let mut inner = self.lock();
            inner.initializing = false;
        }
        match result.wait_reason() {
            TerminalWaitReason::SessionExit => Err(pty_error("PTY shell exited during startup")),
            TerminalWaitReason::Timeout => Err(pty_error(
                "PTY shell did not reach readiness before startup timeout",
            )),
            TerminalWaitReason::InferredIdle | TerminalWaitReason::StdinRead => {
                let _ = self.motd.set(result.viewport().to_string());
                Ok(())
            }
        }
    }

    fn ensure_pump(&self) {
        let rx = {
            let mut inner = self.lock();
            if inner.pump_started {
                return;
            }
            inner.pump_started = true;
            inner.output_rx.take()
        };
        let Some(mut output) = rx else {
            return;
        };
        let session = self.clone();
        let terminal = session.lock().terminal.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    chunk = output.recv() => {
                        match chunk {
                            Ok(text) => session.on_data(&text),
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                session.on_output_end();
                                let outcome = terminal.done().await;
                                session.on_exit(outcome);
                                return;
                            }
                        }
                    }
                    outcome = terminal.done() => {
                        loop {
                            match tokio::time::timeout(Duration::from_millis(25), output.recv()).await {
                                Ok(Ok(text)) => session.on_data(&text),
                                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) | Err(_) => break,
                            }
                        }
                        session.on_output_end();
                        session.on_exit(outcome);
                        return;
                    }
                }
            }
        });
    }

    fn reset_readiness_evidence(inner: &mut Inner) {
        inner.last_output_at = Instant::now();
        inner.prompt_seen = false;
        inner.prompt_text_seen = false;
        inner.prompt_tail.clear();
    }

    fn on_data(&self, data: &str) {
        let mut inner = self.lock();
        let sanitized = inner.sanitizer.push(data);
        inner_append(&mut inner, &sanitized.text);
        if sanitized.prompt {
            inner.prompt_seen = true;
            inner.prompt_tail.clear();
            inner.last_output_at = Instant::now();
        }
        if inner.prompt_seen {
            if let Some(tail) = sanitized.prompt_tail.as_deref() {
                let remaining = CONTROLLED_PROMPT
                    .chars()
                    .count()
                    .saturating_add(1)
                    .saturating_sub(inner.prompt_tail.chars().count());
                let extra: String = tail.chars().take(remaining).collect();
                inner.prompt_tail.push_str(&extra);
                if tail.chars().count() > remaining {
                    inner.prompt_tail = format!("{CONTROLLED_PROMPT}\0");
                }
                inner.prompt_text_seen = inner.prompt_tail == CONTROLLED_PROMPT;
            }
        }
    }

    fn on_output_end(&self) {
        let mut inner = self.lock();
        if inner.output_ended {
            return;
        }
        inner.output_ended = true;
        let rest = inner.sanitizer.flush();
        inner_append(&mut inner, &rest);
    }

    fn on_exit(&self, outcome: Result<SubprocessOutcome, SubprocessError>) {
        let send_id;
        {
            let mut inner = self.lock();
            inner.status = match outcome {
                Ok(result) => TerminalSessionStatus::Exited {
                    exit_code: result.exit_code,
                    signal: result.signal.map(|code| code.to_string()),
                },
                Err(_) => TerminalSessionStatus::Exited {
                    exit_code: None,
                    signal: None,
                },
            };
            send_id = inner.active.as_ref().map(|active| active.id);
        }
        if let Some(id) = send_id {
            self.settle(id, TerminalWaitReason::SessionExit, false);
        }
    }

    fn start_poll(&self, send_id: u64) {
        let generation = {
            let mut inner = self.lock();
            inner.poll_gen = inner.poll_gen.saturating_add(1);
            inner.poll_gen
        };
        let session = self.clone();
        tokio::spawn(async move {
            session.poll_loop(send_id, generation).await;
        });
    }

    async fn poll_loop(&self, send_id: u64, generation: u64) {
        let interval = Duration::from_millis(self.lock().config.poll_interval_ms);
        loop {
            tokio::time::sleep(interval).await;
            if !self.poll_once(send_id, generation).await {
                return;
            }
        }
    }

    async fn poll_once(&self, send_id: u64, generation: u64) -> bool {
        let terminal;
        {
            let inner = self.lock();
            if inner.poll_gen != generation {
                return false;
            }
            match inner.active.as_ref() {
                Some(active)
                    if active.id == send_id && !active.interrupting && !active.finished => {}
                _ => return false,
            }
            if inner.closing {
                return false;
            }
            if let TerminalSessionStatus::Exited { .. } = inner.status {
                drop(inner);
                self.settle(send_id, TerminalWaitReason::SessionExit, false);
                return false;
            }
            terminal = inner.terminal.clone();
        }
        let foreground = match terminal.inspect_foreground().await {
            Ok(value) => value,
            Err(_) => {
                self.settle(send_id, TerminalWaitReason::SessionExit, false);
                return false;
            }
        };
        let mut inner = self.lock();
        if inner.poll_gen != generation {
            return false;
        }
        match inner.active.as_ref() {
            Some(active) if active.id == send_id && !active.interrupting && !active.finished => {}
            _ => return false,
        }
        if inner.closing {
            return false;
        }
        if inner.prompt_seen {
            if let Some(fg) = foreground.as_ref() {
                if inner.shell_pgid.is_none() {
                    inner.shell_pgid = Some(fg.process_group_id());
                }
            }
        }
        let idle_for = inner.last_output_at.elapsed();
        let poll_interval = Duration::from_millis(inner.config.poll_interval_ms);
        let fg_pgid = foreground
            .as_ref()
            .map(SubprocessTerminalForeground::process_group_id);
        if inner.prompt_seen && inner.prompt_text_seen && idle_for >= poll_interval {
            match (fg_pgid, inner.shell_pgid) {
                (Some(fg), Some(shell)) if fg == shell => {
                    drop(inner);
                    self.settle(send_id, TerminalWaitReason::StdinRead, false);
                    return false;
                }
                _ => {}
            }
        }
        let elapsed = match inner.active.as_ref() {
            Some(active) if active.id == send_id => active.started_at.elapsed(),
            _ => Duration::ZERO,
        };
        let startup_has_output = !inner.initializing || !inner.scrollback.is_empty();
        let accepts_stdin_wait = if startup_has_output {
            match (foreground.as_ref(), inner.active.as_mut()) {
                (Some(fg), Some(active)) if active.id == send_id => {
                    active.accepts_stdin_wait(fg.process_group_id(), fg.input_waiting())
                }
                _ => false,
            }
        } else {
            false
        };
        let exact_after = Duration::from_millis(inner.config.exact_probe_after_ms);
        if elapsed >= exact_after && accepts_stdin_wait {
            drop(inner);
            self.settle(send_id, TerminalWaitReason::StdinRead, false);
            return false;
        }
        let idle_silence = Duration::from_millis(inner.config.idle_silence_ms);
        let handoff = Duration::from_millis(inner.config.handoff_grace_ms);
        let handoff_grace = if inner.prompt_seen {
            handoff
        } else {
            Duration::ZERO
        };
        if startup_has_output && idle_for >= idle_silence.saturating_add(handoff_grace) {
            drop(inner);
            self.settle(send_id, TerminalWaitReason::InferredIdle, false);
            return false;
        }
        true
    }

    fn settle(&self, send_id: u64, reason: TerminalWaitReason, retain: bool) {
        let mut inner = self.lock();
        let skip = match inner.active.as_ref() {
            Some(active) => active.id != send_id || active.finished,
            None => true,
        };
        if skip {
            return;
        }
        if let Some(active) = inner.active.as_mut() {
            active.finished = true;
        }
        if !retain {
            inner.poll_gen = inner.poll_gen.saturating_add(1);
        }
        let (viewport, op_truncated) = match inner.active.as_ref() {
            Some(active) => active.output.snapshot(),
            None => return,
        };
        let scrollback_truncated = inner.scrollback.snapshot().1;
        let truncated = op_truncated || scrollback_truncated;
        let status = inner.status.clone();
        let tx = inner
            .active
            .as_mut()
            .and_then(|active| active.settle_tx.take());
        if !retain {
            inner.active = None;
        }
        drop(inner);
        if let Some(tx) = tx {
            let _ = tx.send(TerminalSendResult::new(viewport, reason, status, truncated));
        }
    }

    async fn begin_send(&self, send_id: u64, text: String, submit: bool) {
        let terminal = self.lock().terminal.clone();
        let foreground = match terminal.inspect_foreground().await {
            Ok(value) => value,
            Err(_) => {
                let interrupting = self
                    .lock()
                    .active
                    .as_ref()
                    .map(|active| active.id == send_id && active.interrupting)
                    .unwrap_or(false);
                if !interrupting {
                    self.settle(send_id, TerminalWaitReason::SessionExit, false);
                }
                return;
            }
        };
        let cancel = self
            .lock()
            .active
            .as_ref()
            .map(|active| {
                active.id != send_id
                    || active.cancel_requested
                    || active.finished
                    || active.interrupting
            })
            .unwrap_or(true);
        if cancel {
            return;
        }
        {
            let mut inner = self.lock();
            if inner.closing {
                return;
            }
            if let Some(active) = inner.active.as_mut() {
                if active.id == send_id {
                    active.set_initial_foreground(foreground.as_ref());
                }
            }
        }
        let input = if submit { format!("{text}\r") } else { text };
        if !input.is_empty() {
            {
                let mut inner = self.lock();
                let matches = inner
                    .active
                    .as_ref()
                    .map(|active| active.id == send_id)
                    .unwrap_or(false);
                if !matches {
                    return;
                }
                Self::reset_readiness_evidence(&mut inner);
                if let Some(active) = inner.active.as_mut() {
                    active.writing = true;
                }
            }
            let write_ok = terminal.write(&input).await.is_ok();
            {
                let mut inner = self.lock();
                if let Some(active) = inner.active.as_mut() {
                    if active.id == send_id {
                        active.writing = false;
                    }
                }
            }
            let cancel = self
                .lock()
                .active
                .as_ref()
                .map(|active| active.id == send_id && active.cancel_requested)
                .unwrap_or(false);
            if cancel {
                if write_ok {
                    let _ = terminal
                        .signal_foreground(SubprocessTerminalSignal::Sigint)
                        .await;
                }
                return;
            }
        }
        let still_active = self
            .lock()
            .active
            .as_ref()
            .map(|active| active.id == send_id && !active.finished && !active.interrupting)
            .unwrap_or(false);
        if still_active {
            self.start_poll(send_id);
        }
    }

    fn request_cancel(&self, send_id: u64) -> bool {
        let terminal;
        {
            let mut inner = self.lock();
            let Some(active) = inner.active.as_mut() else {
                return false;
            };
            if active.id != send_id || active.finished {
                return false;
            }
            active.cancel_requested = true;
            active.interrupting = true;
            inner.poll_gen = inner.poll_gen.saturating_add(1);
            terminal = inner.terminal.clone();
        }
        let session = self.clone();
        tokio::spawn(async move {
            loop {
                let (writing, finished) = {
                    let inner = session.lock();
                    match inner.active.as_ref() {
                        Some(active) if active.id == send_id => (active.writing, active.finished),
                        _ => (false, true),
                    }
                };
                if finished {
                    return;
                }
                if !writing {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let _ = terminal
                .signal_foreground(SubprocessTerminalSignal::Sigint)
                .await;
            {
                let mut inner = session.lock();
                if let Some(active) = inner.active.as_mut() {
                    if active.id == send_id {
                        active.interrupting = false;
                        if active.finished {
                            inner.active = None;
                        }
                    }
                }
            }
            let still = session
                .lock()
                .active
                .as_ref()
                .map(|active| active.id == send_id && !active.finished)
                .unwrap_or(false);
            if still {
                session.start_poll(send_id);
            }
        });
        true
    }

    async fn close_once(&self, reason: &str) -> Result<(), TerminalError> {
        let (terminal, already) = {
            let mut inner = self.lock();
            let already = inner.close_started;
            if !already {
                inner.close_started = true;
                inner.closing = true;
                inner.poll_gen = inner.poll_gen.saturating_add(1);
            }
            (inner.terminal.clone(), already)
        };
        if already {
            let _ = terminal.done().await;
            return Ok(());
        }
        if let Err(error) = terminal.terminate().await {
            return Err(pty_error(format!("PTY cleanup failed ({reason}): {error}")));
        }
        let send_id = self.lock().active.as_ref().map(|active| active.id);
        if let Some(id) = send_id {
            self.settle(id, TerminalWaitReason::SessionExit, false);
        }
        let _ = terminal.done().await;
        Ok(())
    }
}

fn inner_append(inner: &mut Inner, text: &str) {
    if text.is_empty() {
        return;
    }
    inner.last_output_at = Instant::now();
    inner.scrollback.append(text);
    if let Some(active) = inner.active.as_mut() {
        if !active.finished {
            active.output.append(text);
        }
    }
}

fn map_signal(signal: TerminalSignal) -> SubprocessTerminalSignal {
    match signal {
        TerminalSignal::Sigint => SubprocessTerminalSignal::Sigint,
        TerminalSignal::Sigterm => SubprocessTerminalSignal::Sigterm,
        TerminalSignal::Sigkill => SubprocessTerminalSignal::Sigkill,
        TerminalSignal::Sigtstp => SubprocessTerminalSignal::Sigtstp,
        TerminalSignal::Sighup => SubprocessTerminalSignal::Sighup,
    }
}

impl TerminalBackendSession for LocalPtySession {
    fn motd(&self) -> &str {
        self.motd.get().map(String::as_str).unwrap_or("")
    }

    fn pid(&self) -> Option<i32> {
        if self.pid > 0 { Some(self.pid) } else { None }
    }

    fn start_send(&self, request: TerminalSendRequest) -> TerminalSendOperation {
        let (tx, rx) = oneshot::channel();
        let send_id;
        let timeout_ms;
        {
            let mut inner = self.lock();
            if inner.closing {
                drop(inner);
                return ready_operation(
                    String::new(),
                    TerminalWaitReason::SessionExit,
                    TerminalSessionStatus::Exited {
                        exit_code: None,
                        signal: None,
                    },
                );
            }
            if let TerminalSessionStatus::Exited { .. } = inner.status {
                let status = inner.status.clone();
                drop(inner);
                return ready_operation(String::new(), TerminalWaitReason::SessionExit, status);
            }
            if inner.active.is_some() {
                drop(inner);
                return ready_operation(
                    String::new(),
                    TerminalWaitReason::Timeout,
                    TerminalSessionStatus::Running,
                );
            }
            inner.next_send_id = inner.next_send_id.saturating_add(1);
            send_id = inner.next_send_id;
            timeout_ms = inner.config.timeout_ms;
            let max_read = inner.config.max_read_bytes;
            inner.active = Some(ActiveSend {
                id: send_id,
                output: BoundedTextBuffer::new(max_read, None),
                settle_tx: Some(tx),
                cancel_requested: false,
                interrupting: false,
                writing: false,
                finished: false,
                started_at: Instant::now(),
                initial_foreground_pgid: None,
                initial_foreground_left_wait: true,
            });
            Self::reset_readiness_evidence(&mut inner);
        }
        self.ensure_pump();
        let session = self.clone();
        let text = request.text().to_string();
        let submit = request.submit();
        tokio::spawn(async move {
            session.begin_send(send_id, text, submit).await;
        });
        let session = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
            let retain = session
                .lock()
                .active
                .as_ref()
                .map(|active| active.id == send_id && (active.interrupting || active.writing))
                .unwrap_or(false);
            session.settle(send_id, TerminalWaitReason::Timeout, retain);
        });
        let inner_slot = Arc::clone(&self.inner);
        TerminalSendOperation::new(
            Box::pin(async move {
                rx.await.unwrap_or_else(|_| {
                    TerminalSendResult::new(
                        String::new(),
                        TerminalWaitReason::SessionExit,
                        TerminalSessionStatus::Exited {
                            exit_code: None,
                            signal: None,
                        },
                        false,
                    )
                })
            }),
            Box::new(move || {
                let mut inner = lock_poison(&inner_slot);
                match inner.active.as_mut() {
                    Some(active) if active.id == send_id => active.output.consume(),
                    _ => TerminalSendRead::new(String::new(), false),
                }
            }),
            {
                let session = self.clone();
                Box::new(move || session.request_cancel(send_id))
            },
        )
    }

    fn read(&self, request: TerminalReadRequest) -> TerminalReadResult {
        let inner = self.lock();
        let (text, truncated) = inner.scrollback.snapshot();
        let lines: Vec<&str> = if text.is_empty() {
            Vec::new()
        } else {
            text.split('\n').collect()
        };
        let total_lines = lines.len();
        let offset = request.offset().unwrap_or(0);
        let count = request.count().unwrap_or(500);
        if count == 0 || offset >= total_lines {
            return TerminalReadResult::new(String::new(), total_lines, offset, offset, truncated);
        }
        let end = total_lines.saturating_sub(offset);
        let start = end.saturating_sub(count);
        let requested = lines[start..end].join("\n");
        let (bounded, bound_truncated) = utf8_tail(&requested, inner.config.max_read_bytes);
        let returned_lines = if bounded.is_empty() {
            0
        } else {
            bounded.split('\n').count()
        };
        TerminalReadResult::new(
            bounded,
            total_lines,
            offset,
            offset.saturating_add(returned_lines),
            truncated || bound_truncated,
        )
    }

    fn signal(
        &self,
        signal: TerminalSignal,
    ) -> Pin<Box<dyn Future<Output = TerminalSignalResult> + Send>> {
        let terminal = self.lock().terminal.clone();
        Box::pin(async move {
            match terminal.signal_foreground(map_signal(signal)).await {
                Ok(pgid) => TerminalSignalResult::new(true, pgid),
                Err(_) => TerminalSignalResult::new(false, 0),
            }
        })
    }

    fn status(&self) -> TerminalSessionStatus {
        self.lock().status.clone()
    }

    fn close(
        &self,
        reason: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), TerminalError>> + Send>> {
        let session = self.clone();
        let reason = reason.to_string();
        Box::pin(async move { session.close_once(&reason).await })
    }
}

fn ready_operation(
    viewport: String,
    reason: TerminalWaitReason,
    status: TerminalSessionStatus,
) -> TerminalSendOperation {
    let result = TerminalSendResult::new(viewport, reason, status, false);
    TerminalSendOperation::new(
        Box::pin(std::future::ready(result)),
        Box::new(|| TerminalSendRead::new(String::new(), false)),
        Box::new(|| false),
    )
}

#[cfg(test)]
mod tests {
    use super::LocalPtySession;
    use crate::config::parse_config;
    use dsh_subprocess::{
        ProcessIdentity, ProcessInspector, SubprocessTerminalHandle, SubprocessTerminalSignal,
        TermKill,
    };
    use dsh_terminal::{TerminalBackendSession, TerminalSendRequest, TerminalWaitReason};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    struct ScriptedInspector {
        pgid: Mutex<Option<i32>>,
        waiting: AtomicBool,
    }

    impl ScriptedInspector {
        fn new(pgid: Option<i32>, waiting: bool) -> Arc<Self> {
            Arc::new(Self {
                pgid: Mutex::new(pgid),
                waiting: AtomicBool::new(waiting),
            })
        }

        fn set_pgid(&self, pgid: Option<i32>) {
            *self.pgid.lock().unwrap_or_else(PoisonError::into_inner) = pgid;
        }

        fn set_waiting(&self, waiting: bool) {
            self.waiting.store(waiting, Ordering::SeqCst);
        }
    }

    impl ProcessInspector for ScriptedInspector {
        fn foreground_pgid(&self, _shell_pid: i32) -> Option<i32> {
            *self.pgid.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn is_stdin_waiting(&self, _pgid: i32) -> bool {
            self.waiting.load(Ordering::SeqCst)
        }

        fn process_tree(&self, root_pid: i32) -> Vec<ProcessIdentity> {
            vec![ProcessIdentity::new(root_pid, "shell")]
        }

        fn process_session(&self, _session_id: i32) -> Vec<ProcessIdentity> {
            Vec::new()
        }

        fn is_alive(&self, _identity: &ProcessIdentity) -> bool {
            false
        }

        fn signal_group(&self, _pgid: i32, _signal: SubprocessTerminalSignal) {}

        fn signal_process(&self, _identity: &ProcessIdentity, _signal: TermKill) {}
    }

    fn test_config() -> crate::config::ResolvedConfig {
        parse_config(&serde_json::json!({
            "pollIntervalMs": 10,
            "exactProbeAfterMs": 20,
            "idleSilenceMs": 2000,
            "handoffGraceMs": 20,
            "timeoutMs": 800,
        }))
        .expect("config")
    }

    fn make_session(
        inspector: Arc<ScriptedInspector>,
    ) -> (LocalPtySession, SubprocessTerminalHandle) {
        let handle = SubprocessTerminalHandle::injected(
            123,
            Arc::clone(&inspector) as Arc<dyn ProcessInspector>,
            20,
        );
        let session = LocalPtySession::new(handle.clone(), test_config());
        (session, handle)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tier1_stdin_wait_is_stdin_read() {
        let inspector = ScriptedInspector::new(Some(456), true);
        let (session, _handle) = make_session(Arc::clone(&inspector));
        let operation = session.start_send(TerminalSendRequest::new("cat", true));
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = tx.send(operation.done().await);
        });

        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            matches!(
                rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "same-PGID wait that existed before write must not settle stdin_read"
        );

        inspector.set_waiting(false);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            matches!(
                rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "leaving wait is not stdin_read by itself"
        );

        inspector.set_waiting(true);
        let result = tokio::time::timeout(Duration::from_millis(400), rx)
            .await
            .expect("stdin_read after leave-then-reenter")
            .expect("send result");
        assert_eq!(result.wait_reason(), TerminalWaitReason::StdinRead);

        let unknown = ScriptedInspector::new(Some(456), true);
        unknown.set_pgid(None);
        let (session2, _) = make_session(Arc::clone(&unknown));
        let operation2 = session2.start_send(TerminalSendRequest::new("cat", true));
        if let Ok(result) =
            tokio::time::timeout(Duration::from_millis(400), operation2.done()).await
        {
            assert_ne!(
                result.wait_reason(),
                TerminalWaitReason::StdinRead,
                "unknown foreground is never stdin_read"
            );
        }

        let _ = session.close("test").await;
        let _ = session2.close("test").await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tier1_stdin_wait_prompt_same_shell_pgid_is_stdin_read() {
        let inspector = ScriptedInspector::new(Some(456), false);
        let (session, handle) = make_session(Arc::clone(&inspector));
        let operation = session.start_send(TerminalSendRequest::new("true", true));
        tokio::time::sleep(Duration::from_millis(20)).await;
        handle.emit_output("\x1b]133;D;0\x07dsh> ");
        let result = tokio::time::timeout(Duration::from_millis(200), operation.done())
            .await
            .expect("prompt+shell_pgid stdin_read");
        assert_eq!(result.wait_reason(), TerminalWaitReason::StdinRead);
        let _ = session.close("test").await;
    }
}
