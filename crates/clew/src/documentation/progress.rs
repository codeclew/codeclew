//! Best-effort phase diagnostics, separate from machine-readable command stdout.
use crate::error::{ClewError, ErrorCode};
use serde::Serialize;
use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
use std::io::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INTERVAL: Duration = Duration::from_secs(5);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
thread_local! { static CURRENT_SPAN: Cell<Option<u64>> = const { Cell::new(None) }; }
type Sink = Arc<dyn Fn(&Event) + Send + Sync>;
#[cfg(test)]
thread_local! { static TEST_SINK: RefCell<Option<Sink>> = const { RefCell::new(None) }; }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    schema: &'static str,
    pid: u32,
    span_id: u64,
    parent_span_id: Option<u64>,
    phase: &'static str,
    event: &'static str,
    elapsed_ms: u128,
    timestamp_unix_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    syntax_file_cache: Option<SyntaxFileCache>,
    #[serde(skip_serializing_if = "Option::is_none")]
    counts: Option<Counts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal_cause: Option<ErrorCode>,
}

/// Measured numbers only. Names, paths and source never enter default progress.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compilation_ordinal: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compilations_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_facts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_payload_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_records: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyntaxFileCache {
    hits: usize,
    misses: usize,
    parsed_files: usize,
}

/// Counts only, outside evidence digests: reuse must not change source identity.
pub(super) fn syntax_file_cache(hits: usize, misses: usize, parsed_files: usize) {
    if let Some(sink) = configured_sink() {
        let mut value = event(
            "SYNTAX_FILE_CACHE",
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
            CURRENT_SPAN.get(),
            Instant::now(),
            "COMPLETED",
        );
        value.syntax_file_cache = Some(SyntaxFileCache {
            hits,
            misses,
            parsed_files,
        });
        sink(&value);
    }
}

/// Use fixed, public phase names, never source text, paths, or user identifiers.
/// Explicit completion is required: early returns and unwinds are failures.
pub struct Phase {
    phase: &'static str,
    id: u64,
    parent: Option<u64>,
    active: bool,
    _same_thread: PhantomData<Rc<()>>,
    started: Instant,
    sink: Option<Sink>,
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
    counts: Arc<Mutex<Option<Counts>>>,
    terminal_cause: Option<ErrorCode>,
}

impl Phase {
    pub fn start(phase: &'static str) -> Self {
        let sink = configured_sink();
        Self::with_sink(phase, sink, INTERVAL)
    }

    fn with_sink(phase: &'static str, sink: Option<Sink>, interval: Duration) -> Self {
        Self::with_counts(phase, sink, interval, None)
    }

    fn with_counts(
        phase: &'static str,
        sink: Option<Sink>,
        interval: Duration,
        counts: Option<Counts>,
    ) -> Self {
        let started = Instant::now();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let parent = CURRENT_SPAN.replace(Some(id));
        let mut span = Self {
            phase,
            id,
            parent,
            active: true,
            _same_thread: PhantomData,
            started,
            sink,
            stop: None,
            worker: None,
            counts: Arc::new(Mutex::new(counts)),
            terminal_cause: None,
        };
        span.emit("STARTED");
        if let Some(sink) = span.sink.clone() {
            let (stop, receiver) = mpsc::channel();
            let counts = Arc::clone(&span.counts);
            // Failure to allocate a diagnostic thread must not fail the command.
            if let Ok(worker) = std::thread::Builder::new()
                .name("docs-progress".into())
                .spawn(move || {
                    while matches!(
                        receiver.recv_timeout(interval),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        let mut value = event(phase, id, parent, started, "HEARTBEAT");
                        value.counts = counts.lock().ok().and_then(|v| v.clone());
                        sink(&value);
                    }
                })
            {
                span.stop = Some(stop);
                span.worker = Some(worker);
            }
        }
        span
    }

    /// Updating counters does not claim forward progress beyond their measured values.
    pub fn set_counts(&self, counts: Counts) {
        if let Ok(mut current) = self.counts.lock() {
            *current = Some(counts);
        }
    }

    fn emit(&self, status: &'static str) {
        if let Some(sink) = &self.sink {
            let mut value = event(self.phase, self.id, self.parent, self.started, status);
            value.counts = self.counts.lock().ok().and_then(|v| v.clone());
            value.terminal_cause = self.terminal_cause.clone();
            sink(&value);
        }
    }

    fn finish(&mut self, status: &'static str) {
        if !self.active {
            return;
        }
        self.active = false;
        CURRENT_SPAN.set(self.parent);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.emit(status);
        self.sink = None;
    }

    pub fn fail(mut self, cause: ErrorCode) {
        self.terminal_cause = Some(cause);
        self.finish("FAILED");
    }

    pub fn complete(mut self) {
        self.finish("COMPLETED");
    }
}

impl Drop for Phase {
    fn drop(&mut self) {
        self.finish("FAILED");
    }
}

fn event(
    phase: &'static str,
    id: u64,
    parent: Option<u64>,
    started: Instant,
    status: &'static str,
) -> Event {
    Event {
        schema: "codeclew-documentation-progress/1.0",
        pid: std::process::id(),
        span_id: id,
        parent_span_id: parent,
        phase,
        event: status,
        elapsed_ms: started.elapsed().as_millis(),
        timestamp_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        syntax_file_cache: None,
        counts: None,
        terminal_cause: None,
    }
}

fn configured_sink() -> Option<Sink> {
    #[cfg(test)]
    if let Some(sink) = TEST_SINK.with(|current| current.borrow().clone()) {
        return Some(sink);
    }

    let enabled = !matches!(
        std::env::var("CODECLEW_DOCS_PROGRESS").as_deref(),
        Ok("off" | "0")
    );
    enabled.then(|| {
        Arc::new(|event: &Event| {
            if let Ok(mut line) = serde_json::to_vec(event) {
                line.push(b'\n');
                // Closed diagnostic pipes must not abort documentation work.
                let _ = std::io::stderr().lock().write_all(&line);
            }
        }) as Sink
    })
}

#[cfg(test)]
struct TestSinkRestore(Option<Sink>);

#[cfg(test)]
impl Drop for TestSinkRestore {
    fn drop(&mut self) {
        TEST_SINK.with(|current| {
            current.replace(self.0.take());
        });
    }
}

/// Route diagnostic events to a test callback on this thread, restoring the
/// previous sink even when the action unwinds.
#[cfg(test)]
pub(crate) fn with_test_sink<T>(
    callback: impl Fn(&serde_json::Value) + Send + Sync + 'static,
    action: impl FnOnce() -> T,
) -> T {
    let sink: Sink = Arc::new(move |event| {
        if let Ok(value) = serde_json::to_value(event) {
            callback(&value);
        }
    });
    let previous = TEST_SINK.with(|current| current.replace(Some(sink)));
    let _restore = TestSinkRestore(previous);
    action()
}

pub fn run<T>(
    phase: &'static str,
    action: impl FnOnce() -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    run_with_phase(phase, |_| action())
}

pub fn run_with_phase<T>(
    phase: &'static str,
    action: impl FnOnce(&Phase) -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    let span = Phase::start(phase);
    let result = action(&span);
    match &result {
        Ok(_) => span.complete(),
        Err(error) => span.fail(error.code.clone()),
    }
    result
}

/// Lower-level native tools add documentation progress only within a caller's
/// existing phase, never to unrelated CLI commands.
pub(crate) fn run_if_active<T>(
    phase: &'static str,
    action: impl FnOnce() -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    if CURRENT_SPAN.get().is_some() {
        run(phase, action)
    } else {
        action()
    }
}

pub(crate) fn run_counted_if_active<T>(
    phase: &'static str,
    counts: Counts,
    action: impl FnOnce() -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    if CURRENT_SPAN.get().is_none() {
        return action();
    }
    let span = Phase::with_counts(phase, configured_sink(), INTERVAL, Some(counts));
    let result = action();
    match &result {
        Ok(_) => span.complete(),
        Err(error) => span.fail(error.code.clone()),
    }
    result
}

/// A successful command transport can carry a failed retained Check. Preserve
/// that distinction in both the outer phase and generic request diagnostics.
pub fn unresolved_check_cause(value: &serde_json::Value) -> Option<ErrorCode> {
    if (value["schema"] != "codeclew-documentation-check/1.0"
        && value["reportSchema"] != "codeclew-documentation-check/1.0")
        || value["status"] != "UNRESOLVED"
    {
        return None;
    }
    if let Ok(cause) = serde_json::from_value(value["terminalCause"].clone()) {
        return Some(cause);
    }
    Some(
        value["unresolved"]
            .as_object()
            .into_iter()
            .flat_map(|v| v.values())
            .find_map(|failure| serde_json::from_value(failure["reason"].clone()).ok())
            .unwrap_or(ErrorCode::IncompleteSemanticAnalysis),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_check_is_not_a_successful_outer_completion() {
        let value = serde_json::json!({"schema":"codeclew-documentation-check/1.0", "status":"UNRESOLVED", "unresolved":{"owned":{"reason":"SLICE_BUDGET_EXCEEDED", "nextAction":"private-marker"}}});
        assert_eq!(
            unresolved_check_cause(&value),
            Some(ErrorCode::SliceBudgetExceeded)
        );
        let mut ready = value.clone();
        ready["status"] = serde_json::json!("CHECKED");
        assert!(unresolved_check_cause(&ready).is_none());
        let mut unrelated = value;
        unrelated["schema"] = serde_json::json!("unrelated-report/1.0");
        assert!(unresolved_check_cause(&unrelated).is_none());
    }

    #[test]
    fn live_heartbeat_carries_measured_counts_before_terminal_failure() {
        let (phase, receiver) = collecting_phase(Duration::from_millis(10));
        phase.set_counts(Counts {
            compilation_ordinal: Some(2),
            compilations_total: Some(3),
            facts: Some(41),
            payload_bytes: Some(900),
            ..Default::default()
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap()["event"],
            "STARTED"
        );
        let live = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(live["event"], "HEARTBEAT");
        assert_eq!(live["counts"]["facts"], 41);
        assert_eq!(live["counts"]["compilationOrdinal"], 2);
        assert!(live["counts"].get("plannedFacts").is_none());
        assert!(!live.to_string().contains("private"));
        phase.fail(ErrorCode::SliceBudgetExceeded);
        let terminal = receiver.try_iter().last().unwrap();
        assert_eq!(terminal["event"], "FAILED");
        assert_eq!(terminal["terminalCause"], "SLICE_BUDGET_EXCEEDED");
        assert_eq!(terminal["counts"]["payloadBytes"], 900);
    }

    #[test]
    fn file_cache_diagnostics_contain_only_counts_and_preserve_parent_span() {
        let (sender, receiver) = mpsc::channel();
        with_test_sink(
            move |value| sender.send(value.clone()).unwrap(),
            || {
                let phase = Phase::start("ACQUIRE_SYNTAX_EVIDENCE");
                syntax_file_cache(2, 0, 0);
                phase.complete();
            },
        );
        let values: Vec<_> = receiver.try_iter().collect();
        assert_eq!(values.len(), 3);
        assert_eq!(values[1]["parentSpanId"], values[0]["spanId"]);
        assert_eq!(values[1]["phase"], "SYNTAX_FILE_CACHE");
        assert_eq!(
            values[1]["syntaxFileCache"],
            serde_json::json!({"hits":2,"misses":0,"parsedFiles":0})
        );
        assert!(values[1].get("service").is_none());
        assert!(values[1].get("path").is_none());
    }

    fn collecting_phase(interval: Duration) -> (Phase, mpsc::Receiver<serde_json::Value>) {
        let (sender, receiver) = mpsc::channel();
        let sink: Sink = Arc::new(move |event| {
            sender.send(serde_json::to_value(event).unwrap()).unwrap();
        });
        (
            Phase::with_sink("test.phase", Some(sink), interval),
            receiver,
        )
    }

    #[test]
    fn heartbeat_reports_liveness_and_completion_stops_it() {
        let (span, receiver) = collecting_phase(Duration::from_millis(5));
        let start = receiver.recv().unwrap();
        assert_eq!(start["event"], "STARTED");
        let heartbeat = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(heartbeat["event"], "HEARTBEAT");
        assert_eq!(heartbeat["spanId"], start["spanId"]);
        span.complete();
        let remaining: Vec<_> = receiver.iter().collect();
        assert_eq!(remaining.last().unwrap()["event"], "COMPLETED");
        assert_eq!(
            remaining
                .iter()
                .filter(|row| row["event"] == "COMPLETED")
                .count(),
            1
        );
        assert!(!remaining.iter().any(|row| row["event"] == "FAILED"));
        // The allowlisted schema cannot accidentally include source or error text.
        assert_eq!(start.as_object().unwrap().len(), 8);
        assert!(start["timestampUnixMs"].as_u64().is_some());
    }

    #[test]
    fn dropped_phase_reports_failure_without_waiting_for_heartbeat() {
        let (span, receiver) = collecting_phase(Duration::from_secs(60));
        let started = Instant::now();
        drop(span);
        assert!(started.elapsed() < Duration::from_secs(2));
        let events: Vec<_> = receiver.iter().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["event"], "FAILED");
    }

    #[test]
    fn disabled_diagnostics_do_not_allocate_a_worker() {
        let span = Phase::with_sink("test.phase", None, INTERVAL);
        assert!(span.worker.is_none());
        assert!(span.stop.is_none());
        span.complete();
    }

    #[test]
    fn nested_spans_keep_order_and_failed_child_does_not_complete() {
        let (sender, receiver) = mpsc::channel();
        with_test_sink(
            move |event| sender.send(event.clone()).unwrap(),
            || {
                let parent = Phase::start("AUTHOR_OPERATION_DRAFT");
                let parent_started = receiver.recv().unwrap();
                let child = Phase::start("WAIT_AGENT_DRIVER_RESPONSE");
                let child_started = receiver.recv().unwrap();
                drop(child);
                parent.complete();

                let child_failed = receiver.recv().unwrap();
                let parent_completed = receiver.recv().unwrap();
                assert_eq!(parent_started["phase"], "AUTHOR_OPERATION_DRAFT");
                assert_eq!(child_started["phase"], "WAIT_AGENT_DRIVER_RESPONSE");
                assert_eq!(child_started["parentSpanId"], parent_started["spanId"]);
                assert_eq!(child_failed["event"], "FAILED");
                assert_eq!(child_failed["spanId"], child_started["spanId"]);
                assert_eq!(parent_completed["event"], "COMPLETED");
                assert_eq!(parent_completed["spanId"], parent_started["spanId"]);
            },
        );
    }
}
