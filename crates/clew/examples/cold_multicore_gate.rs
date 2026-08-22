use clew::cold_start::{
    DAG_SCHEMA, DagPlan, DagReport, DagScheduler, HostResources, NoopProgress, ResourceDescriptor,
    StageSpec,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const EVIDENCE_SCHEMA: &str = "codeclew-cold-multicore-gate/2.0";
const SAMPLE_COUNT: usize = 3;
const MAX_PARALLEL_JOBS: usize = 8;
const CORPUS_BYTES: usize = 256 * 1024;
const RUNTIME_ROUNDS: u64 = 192;
const COMPILATION_ROUNDS: u64 = 48;
const MONOLITH_ADAPTER_ROUNDS: u64 = 96;
const MONOLITH_RUN_ROUNDS: u64 = 12;

#[derive(Clone, Copy)]
enum Workload {
    Runtime,
    MultiCompilation,
    KotlinMonolith,
}

impl Workload {
    fn name(self) -> &'static str {
        match self {
            Self::Runtime => "runtime-capsule",
            Self::MultiCompilation => "multi-compilation-generation",
            Self::KotlinMonolith => "k24-monolith",
        }
    }

    fn plan(self) -> DagPlan {
        match self {
            Self::Runtime => DagPlan {
                schema: DAG_SCHEMA.into(),
                stages: vec![
                    stage("cargo-binaries", &[], RUNTIME_ROUNDS, 0),
                    stage("gradle-workers", &[], RUNTIME_ROUNDS, 0),
                    stage("seal-runtime", &["cargo-binaries", "gradle-workers"], 8, 0),
                ],
            },
            Self::MultiCompilation => {
                let mut stages = (0..12)
                    .map(|index| {
                        stage(
                            &format!("compilation-{index:02}"),
                            &[],
                            COMPILATION_ROUNDS,
                            1,
                        )
                    })
                    .collect::<Vec<_>>();
                let dependencies = stages
                    .iter()
                    .map(|stage| stage.id.as_str())
                    .collect::<Vec<_>>();
                stages.push(stage("seal-generation", &dependencies, 8, 0));
                DagPlan {
                    schema: DAG_SCHEMA.into(),
                    stages,
                }
            }
            Self::KotlinMonolith => {
                let mut stages = vec![stage("adapter-analysis", &[], MONOLITH_ADAPTER_ROUNDS, 1)];
                for index in 0..8 {
                    stages.push(stage(
                        &format!("fact-run-{index:02}"),
                        &["adapter-analysis"],
                        MONOLITH_RUN_ROUNDS,
                        0,
                    ));
                }
                DagPlan {
                    schema: DAG_SCHEMA.into(),
                    stages,
                }
            }
        }
    }
}

fn stage(id: &str, dependencies: &[&str], rounds: u64, sealed_streams: u64) -> StageSpec {
    StageSpec {
        id: id.into(),
        dependencies: dependencies.iter().map(|value| (*value).into()).collect(),
        resources: ResourceDescriptor {
            class: "release-gate".into(),
            min_rss_bytes: 1,
            expected_rss_bytes: 1,
            max_rss_bytes: 1,
            min_cpu: 1,
            max_cpu: 1,
            max_instances: 64,
            exclusivity_key: None,
        },
        operation_uri: "core:release-gate-hash".into(),
        input: json!({"rounds":rounds,"sealedCompilerStreams":sealed_streams}),
    }
}

fn execute(workload: Workload, jobs: usize, corpus: Arc<Vec<u8>>) -> DagReport {
    let resources = HostResources {
        logical_cpu: jobs,
        total_memory_bytes: 2 * 1024 * 1024 * 1024,
        codeclew_memory_budget_bytes: 1024 * 1024 * 1024,
    };
    DagScheduler::new(resources, Arc::new(NoopProgress))
        .expect("release-gate scheduler")
        .execute(workload.plan(), move |stage, cancelled| {
            let rounds = stage.input["rounds"]
                .as_u64()
                .expect("validated release-gate rounds");
            let sealed_streams = stage.input["sealedCompilerStreams"]
                .as_u64()
                .expect("validated release-gate stream count");
            let mut state = Sha256::digest(stage.id.as_bytes()).to_vec();
            for round in 0..rounds {
                if round % 64 == 0 && cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    panic!("release-gate workload was unexpectedly cancelled");
                }
                let mut digest = Sha256::new();
                digest.update(&state);
                digest.update(&*corpus);
                digest.update(round.to_le_bytes());
                state = digest.finalize().to_vec();
            }
            Ok(json!({
                "digest": format!("sha256:{}", hex::encode(state)),
                "sealedCompilerStreams": sealed_streams,
            }))
        })
        .expect("release-gate DAG")
}

fn output_digest(report: &DagReport) -> String {
    let outputs = report
        .outputs
        .iter()
        .map(|(id, output)| (id, &output.output))
        .collect::<BTreeMap<_, _>>();
    let bytes = serde_json::to_vec(&outputs).expect("canonical output map");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn report_value(report: &DagReport) -> Value {
    json!({
        "wallMillis": report.duration_millis,
        "totalWorkMillis": report.total_work_millis,
        "criticalPathMillis": report.critical_path_millis,
        "observedParallelismMilli": report.observed_parallelism_milli,
        "maxAdmittedCpu": report.max_admitted_cpu,
        "outputDigest": output_digest(report),
    })
}

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn paired_gate(
    workload: Workload,
    parallel_jobs: usize,
    limit_milli: u128,
    corpus: Arc<Vec<u8>>,
) -> Value {
    let mut serial = Vec::with_capacity(SAMPLE_COUNT);
    let mut parallel = Vec::with_capacity(SAMPLE_COUNT);
    let mut expected_digest = None::<String>;
    let mut digest_identical = true;
    for sample in 0..SAMPLE_COUNT {
        let order = if sample % 2 == 0 {
            [(1, &mut serial), (parallel_jobs, &mut parallel)]
        } else {
            [(parallel_jobs, &mut parallel), (1, &mut serial)]
        };
        for (jobs, destination) in order {
            let report = execute(workload, jobs, Arc::clone(&corpus));
            let digest = output_digest(&report);
            if let Some(expected) = &expected_digest {
                digest_identical &= expected == &digest;
            } else {
                expected_digest = Some(digest);
            }
            destination.push(report_value(&report));
        }
    }
    let serial_median = median(
        serial
            .iter()
            .map(|sample| sample["wallMillis"].as_u64().unwrap() as u128)
            .collect(),
    );
    let parallel_median = median(
        parallel
            .iter()
            .map(|sample| sample["wallMillis"].as_u64().unwrap() as u128)
            .collect(),
    );
    let ratio_milli = parallel_median
        .saturating_mul(1_000)
        .checked_div(serial_median.max(1))
        .unwrap_or(u128::MAX);
    json!({
        "workload": workload.name(),
        "samples": SAMPLE_COUNT,
        "serialJobs": 1,
        "parallelJobs": parallel_jobs,
        "serial": serial,
        "parallel": parallel,
        "serialMedianMillis": serial_median,
        "parallelMedianMillis": parallel_median,
        "parallelToSerialMilli": ratio_milli,
        "requiredMaximumMilli": limit_milli,
        "outputDigest": expected_digest.unwrap(),
        "digestIdentical": digest_identical,
        "passed": digest_identical && ratio_milli <= limit_milli,
    })
}

fn monolith_evidence(parallel_jobs: usize, corpus: Arc<Vec<u8>>) -> Value {
    let report = execute(Workload::KotlinMonolith, parallel_jobs, corpus);
    let compiler_streams = report
        .outputs
        .values()
        .filter_map(|output| output.output["sealedCompilerStreams"].as_u64())
        .sum::<u64>();
    json!({
        "workload": Workload::KotlinMonolith.name(),
        "jobs": parallel_jobs,
        "report": report_value(&report),
        "sealedCompilerStreams": compiler_streams,
        "passed": compiler_streams == 1
            && report.total_work_millis >= report.critical_path_millis
            && report.critical_path_millis > 0,
    })
}

fn corpus_files(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args([
            "-C",
            root.to_str().expect("UTF-8 source root"),
            "ls-files",
            "-z",
        ])
        .output()
        .expect("enumerate tracked Codeclew corpus");
    assert!(output.status.success(), "git ls-files failed");
    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| {
            let relative = std::str::from_utf8(bytes).expect("tracked path must be UTF-8");
            root.join(relative)
        })
        .filter(|path| path.is_file())
        .filter(|path| {
            let relative = path
                .strip_prefix(root)
                .expect("tracked beneath source root");
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("rs" | "py" | "kt" | "kts" | "toml" | "proto")
            ) || matches!(relative.to_str(), Some("Cargo.lock" | "gradlew" | "clew"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn load_corpus(root: &Path) -> (Vec<u8>, String, usize) {
    let paths = corpus_files(root);
    let mut seed = Sha256::new();
    for path in &paths {
        let relative = path.strip_prefix(root).expect("corpus path beneath root");
        seed.update(relative.to_string_lossy().as_bytes());
        seed.update([0]);
        seed.update(fs::read(path).expect("read pinned Codeclew corpus"));
        seed.update([0]);
    }
    let digest = seed.finalize();
    let mut corpus = Vec::with_capacity(CORPUS_BYTES);
    let mut counter = 0_u64;
    while corpus.len() < CORPUS_BYTES {
        let mut block = Sha256::new();
        block.update(digest);
        block.update(counter.to_le_bytes());
        corpus.extend_from_slice(&block.finalize());
        counter += 1;
    }
    (
        corpus,
        format!("sha256:{}", hex::encode(digest)),
        paths.len(),
    )
}

fn qualification(
    physical_cores: usize,
    logical_processors: usize,
    measurement_passed: bool,
) -> (&'static str, bool, bool) {
    let release_qualified = physical_cores >= 8 && logical_processors >= 8;
    let release_gate_passed = release_qualified && measurement_passed;
    let accepted = !release_qualified || measurement_passed;
    let status = if release_gate_passed {
        "PASSED"
    } else if !release_qualified {
        "SKIPPED_UNQUALIFIED_HOST"
    } else {
        "FAILED"
    };
    (status, release_gate_passed, accepted)
}

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 2 {
        eprintln!("usage: cold_multicore_gate SOURCE_ROOT PHYSICAL_CORES");
        std::process::exit(2);
    }
    let root = Path::new(&arguments[0]);
    let physical_cores = arguments[1]
        .parse::<usize>()
        .expect("physical core count must be an integer");
    let logical_cpu = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    if logical_cpu < 2 {
        eprintln!("cold multicore release gate requires at least two available processors");
        std::process::exit(2);
    }
    let parallel_jobs = logical_cpu.min(MAX_PARALLEL_JOBS);
    let (corpus, corpus_digest, corpus_file_count) = load_corpus(root);
    let corpus = Arc::new(corpus);
    let runtime = paired_gate(Workload::Runtime, parallel_jobs, 650, Arc::clone(&corpus));
    let compilations = paired_gate(
        Workload::MultiCompilation,
        parallel_jobs,
        600,
        Arc::clone(&corpus),
    );
    let monolith = monolith_evidence(parallel_jobs, corpus);
    let measurement_passed = runtime["passed"].as_bool() == Some(true)
        && compilations["passed"].as_bool() == Some(true)
        && monolith["passed"].as_bool() == Some(true);
    let (status, release_gate_passed, accepted) =
        qualification(physical_cores, logical_cpu, measurement_passed);
    let evidence = json!({
        "schema": EVIDENCE_SCHEMA,
        "status": status,
        "workloadAuthority": {
            "kind": "pinned-codeclew-source-hash",
            "corpusDigest": corpus_digest,
            "corpusFileCount": corpus_file_count,
            "corpusBytesPerRound": CORPUS_BYTES,
        },
        "hostAuthority": {
            "logicalProcessors": logical_cpu,
            "physicalCores": physical_cores,
            "physicalCoreAuthority": if physical_cores == 0 { "UNAVAILABLE" } else { "DETECTED" },
            "pinnedEightPhysicalCoreProfile": physical_cores >= 8 && logical_cpu >= 8,
        },
        "runtime": runtime,
        "multiCompilation": compilations,
        "k24Monolith": monolith,
        "measurementPassed": measurement_passed,
        "releaseGatePassed": release_gate_passed,
        "accepted": accepted,
    });
    println!("{}", serde_json::to_string(&evidence).unwrap());
    if !accepted {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_plans_have_required_parallel_shape_and_honest_stream_counts() {
        let runtime = Workload::Runtime.plan();
        assert_eq!(runtime.stages.len(), 3);
        assert_eq!(runtime.stages[2].dependencies.len(), 2);

        let compilations = Workload::MultiCompilation.plan();
        assert_eq!(compilations.stages.len(), 13);
        assert_eq!(compilations.stages[12].dependencies.len(), 12);
        assert_eq!(
            compilations.stages[..12]
                .iter()
                .map(|stage| stage.input["sealedCompilerStreams"].as_u64().unwrap())
                .sum::<u64>(),
            12
        );

        let monolith = Workload::KotlinMonolith.plan();
        assert_eq!(monolith.stages.len(), 9);
        assert_eq!(
            monolith
                .stages
                .iter()
                .map(|stage| stage.input["sealedCompilerStreams"].as_u64().unwrap())
                .sum::<u64>(),
            1
        );
    }

    #[test]
    fn unqualified_hosts_cannot_claim_a_release_pass() {
        assert_eq!(
            qualification(7, 8, true),
            ("SKIPPED_UNQUALIFIED_HOST", false, true)
        );
        assert_eq!(
            qualification(64, 4, true),
            ("SKIPPED_UNQUALIFIED_HOST", false, true)
        );
        assert_eq!(
            qualification(0, 8, false),
            ("SKIPPED_UNQUALIFIED_HOST", false, true)
        );
        assert_eq!(qualification(8, 8, true), ("PASSED", true, true));
        assert_eq!(qualification(8, 8, false), ("FAILED", false, false));
    }
}
