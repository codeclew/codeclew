//! Best-effort phase diagnostics, separate from machine-readable command stdout.
use crate::error::ClewError;
use serde::Serialize;
use std::cell::Cell;
use std::io::Write;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(5);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
thread_local! { static CURRENT_SPAN: Cell<Option<u64>> = const { Cell::new(None) }; }
type Sink = Arc<dyn Fn(&Event) + Send + Sync>;

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
}

impl Phase {
    pub fn start(phase: &'static str) -> Self {
        let enabled = !matches!(
            std::env::var("CODECLEW_DOCS_PROGRESS").as_deref(),
            Ok("off" | "0")
        );
        let sink: Option<Sink> = enabled.then(|| {
            Arc::new(|event: &Event| {
                if let Ok(mut line) = serde_json::to_vec(event) {
                    line.push(b'\n');
                    // Closed diagnostic pipes must not abort documentation work.
                    let _ = std::io::stderr().lock().write_all(&line);
                }
            }) as Sink
        });
        Self::with_sink(phase, sink, INTERVAL)
    }

    fn with_sink(phase: &'static str, sink: Option<Sink>, interval: Duration) -> Self {
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
        };
        span.emit("STARTED");
        if let Some(sink) = span.sink.clone() {
            let (stop, receiver) = mpsc::channel();
            // Failure to allocate a diagnostic thread must not fail the command.
            if let Ok(worker) = std::thread::Builder::new()
                .name("docs-progress".into())
                .spawn(move || {
                    while matches!(
                        receiver.recv_timeout(interval),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        sink(&event(phase, id, parent, started, "HEARTBEAT"));
                    }
                })
            {
                span.stop = Some(stop);
                span.worker = Some(worker);
            }
        }
        span
    }

    fn emit(&self, status: &'static str) {
        if let Some(sink) = &self.sink {
            sink(&event(
                self.phase,
                self.id,
                self.parent,
                self.started,
                status,
            ));
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
    }
}

pub fn run<T>(
    phase: &'static str,
    action: impl FnOnce() -> Result<T, ClewError>,
) -> Result<T, ClewError> {
    let span = Phase::start(phase);
    let result = action();
    if result.is_ok() {
        span.complete();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(start.as_object().unwrap().len(), 7);
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
}
