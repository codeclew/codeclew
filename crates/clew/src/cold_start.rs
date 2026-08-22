use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::state::StateAuthority;
use crossbeam_channel::{RecvTimeoutError, Sender, bounded};
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const DAG_SCHEMA: &str = "codeclew-cold-start-dag/2.0";
pub const DAG_REPORT_SCHEMA: &str = "codeclew-cold-start-report/2.0";
pub const PROGRESS_SCHEMA: &str = "codeclew-cold-start-progress/2.0";
pub const ATTEMPT_SCHEMA: &str = "codeclew-generation-attempt/2.0";

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceDescriptor {
    pub class: String,
    pub min_rss_bytes: u64,
    pub expected_rss_bytes: u64,
    pub max_rss_bytes: u64,
    pub min_cpu: usize,
    pub max_cpu: usize,
    pub max_instances: usize,
    pub exclusivity_key: Option<String>,
}

impl ResourceDescriptor {
    fn validate(&self) -> Result<(), ClewError> {
        if !safe_identifier(&self.class)
            || self.min_rss_bytes > self.expected_rss_bytes
            || self.expected_rss_bytes > self.max_rss_bytes
            || self.min_cpu == 0
            || self.min_cpu > self.max_cpu
            || self.max_instances == 0
            || self
                .exclusivity_key
                .as_deref()
                .is_some_and(|value| !safe_identifier(value))
        {
            return Err(invalid("stage resource descriptor is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageSpec {
    pub id: String,
    pub dependencies: Vec<String>,
    pub resources: ResourceDescriptor,
    pub operation_uri: String,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DagPlan {
    pub schema: String,
    pub stages: Vec<StageSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageOutput {
    pub stage_id: String,
    pub output: Value,
    pub duration_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DagReport {
    pub schema: String,
    pub outputs: BTreeMap<String, StageOutput>,
    pub max_admitted_cpu: usize,
    pub max_admitted_rss_bytes: u64,
    pub duration_millis: u128,
    pub total_work_millis: u128,
    pub critical_path_millis: u128,
    pub observed_parallelism_milli: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostResources {
    pub logical_cpu: usize,
    pub total_memory_bytes: u64,
    pub codeclew_memory_budget_bytes: u64,
}

impl HostResources {
    pub fn detect() -> Result<Self, ClewError> {
        let logical_cpu = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let total_memory_bytes = detected_memory_bytes().ok_or_else(|| {
            ClewError::new(
                ErrorCode::ResourceLimit,
                "host memory authority is unavailable",
            )
        })?;
        Ok(Self::bounded(logical_cpu, total_memory_bytes))
    }

    pub fn bounded(logical_cpu: usize, total_memory_bytes: u64) -> Self {
        let logical_cpu = logical_cpu.max(1);
        let reserved = GIB.max(total_memory_bytes.saturating_mul(15) / 100);
        let usable = total_memory_bytes.saturating_sub(reserved);
        let codeclew_memory_budget_bytes = usable.saturating_mul(70) / 100;
        Self {
            logical_cpu,
            total_memory_bytes,
            codeclew_memory_budget_bytes,
        }
    }
}

pub trait ProgressObserver: Send + Sync + 'static {
    fn observe(&self, event: &ProgressEvent) -> Result<(), ClewError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProgressEvent {
    pub schema: String,
    pub event: String,
    pub stage_id: Option<String>,
    pub queued: usize,
    pub running: usize,
    pub done: usize,
    pub admitted_cpu: usize,
    pub admitted_rss_bytes: u64,
    pub unix_millis: u128,
}

pub struct StderrProgress;

impl ProgressObserver for StderrProgress {
    fn observe(&self, event: &ProgressEvent) -> Result<(), ClewError> {
        let line = serde_json::to_string(event)
            .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
        eprintln!("{line}");
        Ok(())
    }
}

pub struct CompositeProgress {
    observers: Vec<Arc<dyn ProgressObserver>>,
}

impl CompositeProgress {
    pub fn new(observers: Vec<Arc<dyn ProgressObserver>>) -> Result<Self, ClewError> {
        if observers.is_empty() {
            return Err(invalid("composite progress requires at least one observer"));
        }
        Ok(Self { observers })
    }
}

impl ProgressObserver for CompositeProgress {
    fn observe(&self, event: &ProgressEvent) -> Result<(), ClewError> {
        for observer in &self.observers {
            observer.observe(event)?;
        }
        Ok(())
    }
}

pub struct PersistentProgress {
    file: Mutex<File>,
}

impl PersistentProgress {
    pub fn open(authority: &StateAuthority, attempt_id: &str) -> Result<Self, ClewError> {
        let component = attempt_id
            .strip_prefix("attempt:")
            .filter(|value| safe_identifier(value))
            .ok_or_else(|| invalid("progress attempt id is invalid"))?;
        let root = authority
            .directory(Path::new("attempts"))?
            .child(Path::new(component))?;
        let file = root.open_append(std::ffi::OsStr::new("progress.jsonl"))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl ProgressObserver for PersistentProgress {
    fn observe(&self, event: &ProgressEvent) -> Result<(), ClewError> {
        let mut bytes = canonical::bytes(event)
            .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
        bytes.push(b'\n');
        let mut file = self
            .file
            .lock()
            .map_err(|_| ClewError::new(ErrorCode::Internal, "progress journal lock poisoned"))?;
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_data().map_err(io_error)
    }
}

pub struct DagScheduler {
    resources: HostResources,
    pool: ThreadPool,
    observer: Arc<dyn ProgressObserver>,
    heartbeat_interval: Duration,
}

impl DagScheduler {
    pub fn new(
        resources: HostResources,
        observer: Arc<dyn ProgressObserver>,
    ) -> Result<Self, ClewError> {
        if resources.codeclew_memory_budget_bytes == 0 {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "Codeclew memory budget is empty",
            ));
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(resources.logical_cpu)
            .thread_name(|index| format!("clew-cold-{index}"))
            .build()
            .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
        Ok(Self {
            resources,
            pool,
            observer,
            heartbeat_interval: Duration::from_secs(5),
        })
    }

    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Result<Self, ClewError> {
        if interval.is_zero() || interval > Duration::from_secs(5) {
            return Err(invalid(
                "cold-start heartbeat interval must be greater than zero and at most five seconds",
            ));
        }
        self.heartbeat_interval = interval;
        Ok(self)
    }

    pub fn execute<F>(&self, plan: DagPlan, executor: F) -> Result<DagReport, ClewError>
    where
        F: Fn(&StageSpec, &AtomicBool) -> Result<Value, ClewError> + Send + Sync + 'static,
    {
        let validated = ValidatedDag::new(plan, self.resources)?;
        let started = Instant::now();
        let cancelled = Arc::new(AtomicBool::new(false));
        let executor = Arc::new(executor);
        let capacity = self.resources.logical_cpu.saturating_mul(2).max(1);
        let (sender, receiver) = bounded::<CompletedStage>(capacity);
        let mut coordinator = Coordinator::new(validated);
        coordinator.budgets(self.resources);
        self.emit("DAG_STARTED", None, &coordinator)?;
        let mut first_error = None;

        while coordinator.done.len() < coordinator.total {
            if first_error.is_none() {
                while let Some(id) = coordinator.next_admissible() {
                    let stage = coordinator.start(&id)?;
                    self.emit("STAGE_STARTED", Some(id.clone()), &coordinator)?;
                    spawn_stage(
                        &self.pool,
                        sender.clone(),
                        stage,
                        Arc::clone(&executor),
                        Arc::clone(&cancelled),
                    );
                }
            }
            if coordinator.running.is_empty() {
                if let Some(error) = first_error {
                    return Err(error);
                }
                return Err(invalid("DAG has no admissible stage and no running work"));
            }
            let completed = match receiver.recv_timeout(self.heartbeat_interval) {
                Ok(completed) => completed,
                Err(RecvTimeoutError::Timeout) => {
                    self.emit("HEARTBEAT", None, &coordinator)?;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(ClewError::new(
                        ErrorCode::Internal,
                        "stage result channel disconnected",
                    ));
                }
            };
            match completed.result {
                Ok(output) => {
                    coordinator.finish(&completed.id, output)?;
                    self.emit("STAGE_COMPLETED", Some(completed.id), &coordinator)?;
                }
                Err(error) => {
                    cancelled.store(true, Ordering::Release);
                    coordinator.fail(&completed.id)?;
                    self.emit("STAGE_FAILED", Some(completed.id), &coordinator)?;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        self.emit("DAG_READY", None, &coordinator)?;
        let duration_millis = started.elapsed().as_millis();
        let (total_work_millis, critical_path_millis) =
            work_span(&coordinator.stages, &coordinator.done)?;
        Ok(DagReport {
            schema: DAG_REPORT_SCHEMA.into(),
            outputs: coordinator.done,
            max_admitted_cpu: coordinator.max_admitted_cpu,
            max_admitted_rss_bytes: coordinator.max_admitted_rss,
            duration_millis,
            total_work_millis,
            critical_path_millis,
            observed_parallelism_milli: total_work_millis
                .saturating_mul(1_000)
                .checked_div(duration_millis.max(1))
                .unwrap_or(0),
        })
    }

    fn emit(
        &self,
        event: &str,
        stage_id: Option<String>,
        coordinator: &Coordinator,
    ) -> Result<(), ClewError> {
        self.observer.observe(&ProgressEvent {
            schema: PROGRESS_SCHEMA.into(),
            event: event.into(),
            stage_id,
            queued: coordinator.ready.len(),
            running: coordinator.running.len(),
            done: coordinator.done.len(),
            admitted_cpu: coordinator.admitted_cpu,
            admitted_rss_bytes: coordinator.admitted_rss,
            unix_millis: unix_millis(),
        })
    }
}

fn work_span(
    stages: &BTreeMap<String, StageSpec>,
    outputs: &BTreeMap<String, StageOutput>,
) -> Result<(u128, u128), ClewError> {
    let total_work = outputs.values().map(|output| output.duration_millis).sum();
    let mut spans = BTreeMap::<String, u128>::new();
    while spans.len() < stages.len() {
        let before = spans.len();
        for (id, stage) in stages {
            if spans.contains_key(id)
                || !stage
                    .dependencies
                    .iter()
                    .all(|dependency| spans.contains_key(dependency))
            {
                continue;
            }
            let own = outputs
                .get(id)
                .ok_or_else(|| invalid("completed DAG report is missing a stage output"))?
                .duration_millis;
            let dependency_span = stage
                .dependencies
                .iter()
                .filter_map(|dependency| spans.get(dependency).copied())
                .max()
                .unwrap_or(0);
            spans.insert(id.clone(), dependency_span.saturating_add(own));
        }
        if spans.len() == before {
            return Err(invalid(
                "completed DAG report has no computable critical path",
            ));
        }
    }
    Ok((total_work, spans.values().copied().max().unwrap_or(0)))
}

pub fn persist_dag_report(
    authority: &StateAuthority,
    attempt_id: &str,
    report: &DagReport,
) -> Result<(), ClewError> {
    let component = attempt_id
        .strip_prefix("attempt:")
        .filter(|value| safe_identifier(value))
        .ok_or_else(|| invalid("DAG report attempt id is invalid"))?;
    let path = authority
        .attempts_root()
        .join(component)
        .join("dag-report.json");
    let mut bytes = canonical::bytes(report)
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    bytes.push(b'\n');
    authority.write_private_atomic(&path, &bytes)
}

fn spawn_stage<F>(
    pool: &ThreadPool,
    sender: Sender<CompletedStage>,
    stage: StageSpec,
    executor: Arc<F>,
    cancelled: Arc<AtomicBool>,
) where
    F: Fn(&StageSpec, &AtomicBool) -> Result<Value, ClewError> + Send + Sync + 'static,
{
    pool.spawn(move || {
        let started = Instant::now();
        let id = stage.id.clone();
        let result = if cancelled.load(Ordering::Acquire) {
            Err(ClewError::new(
                ErrorCode::TransactionRecoveryRequired,
                "cold-start stage cancelled before execution",
            ))
        } else {
            executor(&stage, &cancelled).map(|output| StageOutput {
                stage_id: id.clone(),
                output,
                duration_millis: started.elapsed().as_millis(),
            })
        };
        let _ = sender.send(CompletedStage { id, result });
    });
}

struct CompletedStage {
    id: String,
    result: Result<StageOutput, ClewError>,
}

struct ValidatedDag {
    stages: BTreeMap<String, StageSpec>,
    dependents: BTreeMap<String, Vec<String>>,
    remaining_dependencies: BTreeMap<String, usize>,
}

impl ValidatedDag {
    fn new(plan: DagPlan, resources: HostResources) -> Result<Self, ClewError> {
        if plan.schema != DAG_SCHEMA || plan.stages.is_empty() || plan.stages.len() > 4096 {
            return Err(invalid("cold-start DAG schema or size is invalid"));
        }
        let mut stages = BTreeMap::new();
        for mut stage in plan.stages {
            if !safe_identifier(&stage.id)
                || stage.operation_uri.is_empty()
                || stage.operation_uri.len() > 256
                || stage.dependencies.len() > 4096
            {
                return Err(invalid("cold-start stage identity is invalid"));
            }
            stage.resources.validate()?;
            if stage.resources.max_cpu > resources.logical_cpu
                || stage.resources.max_rss_bytes > resources.codeclew_memory_budget_bytes
            {
                return Err(ClewError::new(
                    ErrorCode::ResourceLimit,
                    format!("stage {} cannot fit the host resource budget", stage.id),
                ));
            }
            stage.dependencies.sort();
            stage.dependencies.dedup();
            if stages.insert(stage.id.clone(), stage).is_some() {
                return Err(invalid("cold-start DAG contains duplicate stage ids"));
            }
        }
        let mut dependents = BTreeMap::<String, Vec<String>>::new();
        let mut remaining_dependencies = BTreeMap::new();
        for (id, stage) in &stages {
            if stage.dependencies.iter().any(|dependency| dependency == id) {
                return Err(invalid("cold-start stage depends on itself"));
            }
            for dependency in &stage.dependencies {
                if !stages.contains_key(dependency) {
                    return Err(invalid("cold-start stage dependency is missing"));
                }
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(id.clone());
            }
            remaining_dependencies.insert(id.clone(), stage.dependencies.len());
        }
        let mut queue = remaining_dependencies
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
            .collect::<VecDeque<_>>();
        let mut seen = 0;
        let mut counts = remaining_dependencies.clone();
        while let Some(id) = queue.pop_front() {
            seen += 1;
            for dependent in dependents.get(&id).into_iter().flatten() {
                let count = counts.get_mut(dependent).expect("validated dependent");
                *count -= 1;
                if *count == 0 {
                    queue.push_back(dependent.clone());
                }
            }
        }
        if seen != stages.len() {
            return Err(invalid("cold-start DAG contains a dependency cycle"));
        }
        Ok(Self {
            stages,
            dependents,
            remaining_dependencies,
        })
    }
}

struct RunningStage {
    resources: ResourceDescriptor,
}

struct Coordinator {
    stages: BTreeMap<String, StageSpec>,
    dependents: BTreeMap<String, Vec<String>>,
    remaining_dependencies: BTreeMap<String, usize>,
    ready: BTreeSet<String>,
    wait_rounds: BTreeMap<String, u64>,
    running: BTreeMap<String, RunningStage>,
    done: BTreeMap<String, StageOutput>,
    failed: BTreeSet<String>,
    class_instances: BTreeMap<String, usize>,
    exclusive: BTreeSet<String>,
    admitted_cpu: usize,
    admitted_rss: u64,
    max_admitted_cpu: usize,
    max_admitted_rss: u64,
    cpu_budget: usize,
    rss_budget: u64,
    total: usize,
}

impl Coordinator {
    fn new(dag: ValidatedDag) -> Self {
        let ready = dag
            .remaining_dependencies
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
            .collect();
        let wait_rounds = dag.stages.keys().map(|id| (id.clone(), 0)).collect();
        let total = dag.stages.len();
        Self {
            stages: dag.stages,
            dependents: dag.dependents,
            remaining_dependencies: dag.remaining_dependencies,
            ready,
            wait_rounds,
            running: BTreeMap::new(),
            done: BTreeMap::new(),
            failed: BTreeSet::new(),
            class_instances: BTreeMap::new(),
            exclusive: BTreeSet::new(),
            admitted_cpu: 0,
            admitted_rss: 0,
            max_admitted_cpu: 0,
            max_admitted_rss: 0,
            cpu_budget: 0,
            rss_budget: 0,
            total,
        }
    }

    fn budgets(&mut self, resources: HostResources) {
        self.cpu_budget = resources.logical_cpu;
        self.rss_budget = resources.codeclew_memory_budget_bytes;
    }

    fn next_admissible(&mut self) -> Option<String> {
        if self.cpu_budget == 0 {
            return None;
        }
        for id in &self.ready {
            *self.wait_rounds.entry(id.clone()).or_default() += 1;
        }
        let mut candidates = self.ready.iter().cloned().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            self.wait_rounds[right]
                .cmp(&self.wait_rounds[left])
                .then_with(|| left.cmp(right))
        });
        candidates.into_iter().find(|id| self.can_admit(id))
    }

    fn can_admit(&self, id: &str) -> bool {
        let resources = &self.stages[id].resources;
        self.admitted_cpu.saturating_add(resources.max_cpu) <= self.cpu_budget
            && self.admitted_rss.saturating_add(resources.max_rss_bytes) <= self.rss_budget
            && self
                .class_instances
                .get(&resources.class)
                .copied()
                .unwrap_or_default()
                < resources.max_instances
            && resources
                .exclusivity_key
                .as_ref()
                .is_none_or(|key| !self.exclusive.contains(key))
    }

    fn start(&mut self, id: &str) -> Result<StageSpec, ClewError> {
        let stage = self
            .stages
            .get(id)
            .cloned()
            .ok_or_else(|| invalid("ready stage is missing"))?;
        if !self.ready.remove(id) || !self.can_admit_spec(&stage) {
            return Err(ClewError::new(
                ErrorCode::Internal,
                "coordinator admitted an unavailable stage",
            ));
        }
        let resources = stage.resources.clone();
        self.admitted_cpu += resources.max_cpu;
        self.admitted_rss += resources.max_rss_bytes;
        self.max_admitted_cpu = self.max_admitted_cpu.max(self.admitted_cpu);
        self.max_admitted_rss = self.max_admitted_rss.max(self.admitted_rss);
        *self
            .class_instances
            .entry(resources.class.clone())
            .or_default() += 1;
        if let Some(key) = &resources.exclusivity_key {
            self.exclusive.insert(key.clone());
        }
        self.running.insert(id.into(), RunningStage { resources });
        Ok(stage)
    }

    fn can_admit_spec(&self, stage: &StageSpec) -> bool {
        let resources = &stage.resources;
        self.admitted_cpu.saturating_add(resources.max_cpu) <= self.cpu_budget
            && self.admitted_rss.saturating_add(resources.max_rss_bytes) <= self.rss_budget
    }

    fn finish(&mut self, id: &str, output: StageOutput) -> Result<(), ClewError> {
        self.release(id)?;
        self.done.insert(id.into(), output);
        for dependent in self.dependents.get(id).into_iter().flatten() {
            let count = self
                .remaining_dependencies
                .get_mut(dependent)
                .ok_or_else(|| invalid("dependent stage state is missing"))?;
            *count = count
                .checked_sub(1)
                .ok_or_else(|| invalid("dependent stage count underflow"))?;
            if *count == 0 {
                self.ready.insert(dependent.clone());
            }
        }
        Ok(())
    }

    fn fail(&mut self, id: &str) -> Result<(), ClewError> {
        self.release(id)?;
        self.failed.insert(id.into());
        Ok(())
    }

    fn release(&mut self, id: &str) -> Result<(), ClewError> {
        let running = self
            .running
            .remove(id)
            .ok_or_else(|| invalid("completed stage was not running"))?;
        self.admitted_cpu = self.admitted_cpu.saturating_sub(running.resources.max_cpu);
        self.admitted_rss = self
            .admitted_rss
            .saturating_sub(running.resources.max_rss_bytes);
        let count = self
            .class_instances
            .get_mut(&running.resources.class)
            .ok_or_else(|| invalid("stage class admission state is missing"))?;
        *count = count.saturating_sub(1);
        if let Some(key) = running.resources.exclusivity_key {
            self.exclusive.remove(&key);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttemptState {
    Created,
    Snapshotted,
    Modeled,
    Analyzing,
    Finalizing,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationAttempt {
    pub schema: String,
    pub attempt_id: String,
    pub generation_key: String,
    pub state: AttemptState,
    pub retry: u32,
    pub transitions: Vec<AttemptTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptTransition {
    pub state: AttemptState,
    pub unix_millis: u128,
    pub evidence: String,
}

pub struct AttemptJournal {
    path: PathBuf,
    attempt: GenerationAttempt,
    authority: StateAuthority,
}

impl AttemptJournal {
    pub fn create(
        authority: StateAuthority,
        generation_key: &str,
        retry: u32,
    ) -> Result<Self, ClewError> {
        digest_component(generation_key)?;
        let attempt_id = format!("attempt:{}", Uuid::new_v4());
        let root = authority
            .directory(Path::new("attempts"))?
            .child(Path::new(
                attempt_id
                    .strip_prefix("attempt:")
                    .expect("known attempt prefix"),
            ))?;
        let path = root.path().join("journal.json");
        let attempt = GenerationAttempt {
            schema: ATTEMPT_SCHEMA.into(),
            attempt_id,
            generation_key: generation_key.into(),
            state: AttemptState::Created,
            retry,
            transitions: vec![AttemptTransition {
                state: AttemptState::Created,
                unix_millis: unix_millis(),
                evidence: "attempt created".into(),
            }],
        };
        let journal = Self {
            path,
            attempt,
            authority,
        };
        journal.persist()?;
        Ok(journal)
    }

    pub fn attempt(&self) -> &GenerationAttempt {
        &self.attempt
    }

    pub fn transition(
        &mut self,
        next: AttemptState,
        evidence: impl Into<String>,
    ) -> Result<(), ClewError> {
        if !allowed_transition(self.attempt.state, next) {
            return Err(invalid("generation attempt transition is invalid"));
        }
        let evidence = evidence.into();
        if evidence.is_empty() || evidence.len() > 1024 {
            return Err(invalid("generation attempt evidence is invalid"));
        }
        self.attempt.state = next;
        self.attempt.transitions.push(AttemptTransition {
            state: next,
            unix_millis: unix_millis(),
            evidence,
        });
        self.persist()
    }

    fn persist(&self) -> Result<(), ClewError> {
        let bytes = canonical::bytes(&self.attempt)
            .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
        self.authority.write_private_atomic(&self.path, &bytes)
    }
}

fn allowed_transition(current: AttemptState, next: AttemptState) -> bool {
    use AttemptState::*;
    matches!(
        (current, next),
        (Created, Snapshotted)
            | (Snapshotted, Modeled)
            | (Modeled, Analyzing)
            | (Analyzing, Finalizing)
            | (Finalizing, Ready)
            | (
                Created | Snapshotted | Modeled | Analyzing | Finalizing,
                Failed | Cancelled
            )
    )
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn digest_component(value: &str) -> Result<&str, ClewError> {
    let component = value
        .strip_prefix("sha256:")
        .ok_or_else(|| invalid("generation key has no sha256 prefix"))?;
    if component.len() != 64
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("generation key is not canonical sha256"));
    }
    Ok(component)
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn detected_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let cgroup = fs::read_to_string("/sys/fs/cgroup/memory.max")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok());
        let host = fs::read_to_string("/proc/meminfo").ok().and_then(|value| {
            value.lines().find_map(|line| {
                line.strip_prefix("MemTotal:")
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|kib| kib.parse::<u64>().ok())
                    .map(|kib| kib.saturating_mul(1024))
            })
        });
        return match (host, cgroup) {
            (Some(host), Some(cgroup)) => Some(host.min(cgroup)),
            (Some(host), None) => Some(host),
            (None, Some(cgroup)) => Some(cgroup),
            (None, None) => None,
        };
    }
    #[cfg(target_os = "macos")]
    {
        let mut value: u64 = 0;
        let mut size = std::mem::size_of::<u64>();
        let name = b"hw.memsize\0";
        let result = unsafe {
            libc::sysctlbyname(
                name.as_ptr().cast(),
                (&mut value as *mut u64).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        return (result == 0 && value > 0).then_some(value);
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestProgress;

    impl ProgressObserver for TestProgress {
        fn observe(&self, _event: &ProgressEvent) -> Result<(), ClewError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordedProgress {
        events: Mutex<Vec<ProgressEvent>>,
    }

    impl ProgressObserver for RecordedProgress {
        fn observe(&self, event: &ProgressEvent) -> Result<(), ClewError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    fn resources(class: &str, cpu: usize, rss: u64) -> ResourceDescriptor {
        ResourceDescriptor {
            class: class.into(),
            min_rss_bytes: rss,
            expected_rss_bytes: rss,
            max_rss_bytes: rss,
            min_cpu: cpu,
            max_cpu: cpu,
            max_instances: 16,
            exclusivity_key: None,
        }
    }

    fn stage(id: &str, dependencies: &[&str], cpu: usize, rss: u64) -> StageSpec {
        StageSpec {
            id: id.into(),
            dependencies: dependencies.iter().map(|value| (*value).into()).collect(),
            resources: resources("test", cpu, rss),
            operation_uri: "test://stage".into(),
            input: json!({}),
        }
    }

    fn scheduler(cpu: usize, rss: u64) -> DagScheduler {
        let resources = HostResources {
            logical_cpu: cpu,
            total_memory_bytes: rss * 2,
            codeclew_memory_budget_bytes: rss,
        };
        let mut scheduler = DagScheduler::new(resources, Arc::new(TestProgress)).unwrap();
        scheduler.resources = resources;
        scheduler
    }

    #[test]
    fn host_budget_reserves_system_memory() {
        let host = HostResources::bounded(8, 16 * GIB);
        assert_eq!(host.logical_cpu, 8);
        assert_eq!(host.codeclew_memory_budget_bytes, 10_222_022_164);
    }

    #[test]
    fn dependency_independent_stages_run_in_parallel_and_report_is_sorted() {
        let plan = DagPlan {
            schema: DAG_SCHEMA.into(),
            stages: vec![
                stage("seal", &["left", "right"], 1, 10),
                stage("right", &[], 1, 10),
                stage("left", &[], 1, 10),
            ],
        };
        let running = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let report = scheduler(2, 20)
            .execute(plan, {
                let running = Arc::clone(&running);
                let maximum = Arc::clone(&maximum);
                move |stage, _| {
                    let current = running.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    if stage.id != "seal" {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    running.fetch_sub(1, Ordering::SeqCst);
                    Ok(json!({"id":stage.id}))
                }
            })
            .unwrap();
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        assert_eq!(
            report.outputs.keys().cloned().collect::<Vec<_>>(),
            ["left", "right", "seal"]
        );
        assert_eq!(report.max_admitted_cpu, 2);
        assert_eq!(report.max_admitted_rss_bytes, 20);
        assert!(report.total_work_millis >= report.critical_path_millis);
        assert!(report.critical_path_millis >= 30);
        assert!(report.observed_parallelism_milli >= 1_000);
    }

    #[test]
    fn resource_admission_never_exceeds_memory_budget() {
        let plan = DagPlan {
            schema: DAG_SCHEMA.into(),
            stages: (0..4)
                .map(|index| stage(&format!("s{index}"), &[], 1, 60))
                .collect(),
        };
        let report = scheduler(4, 100)
            .execute(plan, |_, _| {
                std::thread::sleep(Duration::from_millis(5));
                Ok(json!({}))
            })
            .unwrap();
        assert_eq!(report.max_admitted_rss_bytes, 60);
    }

    #[test]
    fn exclusivity_and_class_instance_limits_are_enforced() {
        let mut exclusive_left = stage("exclusive-left", &[], 1, 1);
        exclusive_left.resources.exclusivity_key = Some("gradle".into());
        let mut exclusive_right = stage("exclusive-right", &[], 1, 1);
        exclusive_right.resources.exclusivity_key = Some("gradle".into());
        let mut class_left = stage("class-left", &[], 1, 1);
        class_left.resources.class = "compiler".into();
        class_left.resources.max_instances = 1;
        let mut class_right = stage("class-right", &[], 1, 1);
        class_right.resources.class = "compiler".into();
        class_right.resources.max_instances = 1;
        let plan = DagPlan {
            schema: DAG_SCHEMA.into(),
            stages: vec![exclusive_left, exclusive_right, class_left, class_right],
        };
        let gradle_running = Arc::new(AtomicUsize::new(0));
        let gradle_max = Arc::new(AtomicUsize::new(0));
        let compiler_running = Arc::new(AtomicUsize::new(0));
        let compiler_max = Arc::new(AtomicUsize::new(0));
        scheduler(4, 100)
            .execute(plan, {
                let gradle_running = Arc::clone(&gradle_running);
                let gradle_max = Arc::clone(&gradle_max);
                let compiler_running = Arc::clone(&compiler_running);
                let compiler_max = Arc::clone(&compiler_max);
                move |stage, _| {
                    let (running, maximum) = if stage.resources.class == "compiler" {
                        (&compiler_running, &compiler_max)
                    } else {
                        (&gradle_running, &gradle_max)
                    };
                    let current = running.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(10));
                    running.fetch_sub(1, Ordering::SeqCst);
                    Ok(json!({}))
                }
            })
            .unwrap();
        assert_eq!(gradle_max.load(Ordering::SeqCst), 1);
        assert_eq!(compiler_max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cycle_and_impossible_stage_fail_before_execution() {
        let cycle = DagPlan {
            schema: DAG_SCHEMA.into(),
            stages: vec![stage("a", &["b"], 1, 1), stage("b", &["a"], 1, 1)],
        };
        assert_eq!(
            scheduler(2, 100)
                .execute(cycle, |_, _| Ok(json!({})))
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
        let impossible = DagPlan {
            schema: DAG_SCHEMA.into(),
            stages: vec![stage("large", &[], 3, 1)],
        };
        assert_eq!(
            scheduler(2, 100)
                .execute(impossible, |_, _| Ok(json!({})))
                .unwrap_err()
                .code,
            ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn first_failure_cancels_new_admission() {
        let plan = DagPlan {
            schema: DAG_SCHEMA.into(),
            stages: vec![stage("fail", &[], 1, 1), stage("after", &["fail"], 1, 1)],
        };
        let executions = Arc::new(AtomicUsize::new(0));
        let error = scheduler(2, 100)
            .execute(plan, {
                let executions = Arc::clone(&executions);
                move |stage, _| {
                    executions.fetch_add(1, Ordering::SeqCst);
                    if stage.id == "fail" {
                        Err(ClewError::new(ErrorCode::WorkerCrashed, "boom"))
                    } else {
                        Ok(json!({}))
                    }
                }
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkerCrashed);
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn attempt_journal_enforces_lifecycle() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let key = format!("sha256:{}", "a".repeat(64));
        let mut journal = AttemptJournal::create(authority, &key, 0).unwrap();
        for state in [
            AttemptState::Snapshotted,
            AttemptState::Modeled,
            AttemptState::Analyzing,
            AttemptState::Finalizing,
            AttemptState::Ready,
        ] {
            journal.transition(state, "verified").unwrap();
        }
        assert_eq!(journal.attempt().state, AttemptState::Ready);
        assert!(journal.transition(AttemptState::Failed, "late").is_err());
    }

    #[test]
    fn dag_report_is_persisted_as_private_attempt_evidence() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let attempt = format!("attempt:{}", Uuid::new_v4());
        let report = scheduler(1, 100)
            .execute(
                DagPlan {
                    schema: DAG_SCHEMA.into(),
                    stages: vec![stage("only", &[], 1, 1)],
                },
                |_, _| Ok(json!({"sealedCompilerStreams":1})),
            )
            .unwrap();
        persist_dag_report(&authority, &attempt, &report).unwrap();
        let bytes = authority
            .read_private_file(
                &authority
                    .attempts_root()
                    .join(attempt.strip_prefix("attempt:").unwrap())
                    .join("dag-report.json"),
                1024 * 1024,
            )
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], DAG_REPORT_SCHEMA);
        assert_eq!(
            value["outputs"]["only"]["output"]["sealedCompilerStreams"],
            1
        );
        assert!(value.get("totalWorkMillis").is_some());
        assert!(value.get("criticalPathMillis").is_some());
    }

    #[test]
    fn long_stage_emits_heartbeat_and_persists_private_progress() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let attempt = format!("attempt:{}", Uuid::new_v4());
        let persisted = Arc::new(PersistentProgress::open(&authority, &attempt).unwrap());
        let recorded = Arc::new(RecordedProgress::default());
        let observers: Vec<Arc<dyn ProgressObserver>> = vec![persisted.clone(), recorded.clone()];
        let resources = HostResources {
            logical_cpu: 1,
            total_memory_bytes: 100,
            codeclew_memory_budget_bytes: 100,
        };
        DagScheduler::new(
            resources,
            Arc::new(CompositeProgress::new(observers).unwrap()),
        )
        .unwrap()
        .with_heartbeat_interval(Duration::from_millis(5))
        .unwrap()
        .execute(
            DagPlan {
                schema: DAG_SCHEMA.into(),
                stages: vec![stage("slow", &[], 1, 1)],
            },
            |_, _| {
                std::thread::sleep(Duration::from_millis(18));
                Ok(json!({}))
            },
        )
        .unwrap();
        assert!(
            recorded
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.event == "HEARTBEAT")
        );
        let path = authority
            .attempts_root()
            .join(attempt.strip_prefix("attempt:").unwrap())
            .join("progress.jsonl");
        let metadata = fs::metadata(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(metadata.permissions().mode() & 0o077, 0);
        }
        assert!(fs::read_to_string(path).unwrap().contains("HEARTBEAT"));
    }

    #[cfg(unix)]
    #[test]
    fn progress_journal_refuses_symlink_substitution() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let attempt = format!("attempt:{}", Uuid::new_v4());
        let attempt_root = authority
            .attempts_root()
            .join(attempt.strip_prefix("attempt:").unwrap());
        fs::create_dir(&attempt_root).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, b"private").unwrap();
        symlink(&outside, attempt_root.join("progress.jsonl")).unwrap();
        assert!(PersistentProgress::open(&authority, &attempt).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"private");
    }

    #[cfg(unix)]
    #[test]
    fn progress_journal_stays_bound_after_state_root_replacement() {
        let root = tempfile::tempdir().unwrap();
        let state_root = root.path().join("v2");
        let pinned_root = root.path().join("pinned-v2");
        let authority = StateAuthority::open(state_root.clone()).unwrap();
        let attempt = format!("attempt:{}", Uuid::new_v4());
        let journal = PersistentProgress::open(&authority, &attempt).unwrap();
        fs::rename(&state_root, &pinned_root).unwrap();
        fs::create_dir(&state_root).unwrap();
        let event = ProgressEvent {
            schema: PROGRESS_SCHEMA.into(),
            event: "BOUND".into(),
            stage_id: None,
            queued: 1,
            running: 0,
            done: 0,
            admitted_cpu: 0,
            admitted_rss_bytes: 0,
            unix_millis: unix_millis(),
        };

        journal.observe(&event).unwrap();

        let component = attempt.strip_prefix("attempt:").unwrap();
        assert!(
            fs::read_to_string(
                pinned_root
                    .join("attempts")
                    .join(component)
                    .join("progress.jsonl")
            )
            .unwrap()
            .contains("BOUND")
        );
        assert!(fs::read_dir(&state_root).unwrap().next().is_none());
    }
}
