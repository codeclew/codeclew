//! Immutable per-service interchange. Integrity is separate from coordinator trust.
use super::{
    analysis, bytes, digest, invalid, io_error,
    model::*,
    store::{self, Repository},
};
use crate::{canonical, error::ClewError};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const SCHEMA: &str = "codeclew-documentation-evidence-package/1.0";
const PART_SCHEMA: &str = "codeclew-documentation-evidence-part/1.0";
const POLICY_SCHEMA: &str = "codeclew-documentation-evidence-expectation/1.0";
const MAX_PART: u64 = 2 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_PARTS: usize = 4096;
const MAX_RECORDS: usize = 131_072;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Collect a source-free support report; --include-index explicitly includes source and facts.
    Report {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        include_index: bool,
    },
    /// Capture through the local supported producer; output contains selected source text.
    Capture {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        diagnostics_only: bool,
    },
    /// Configure a digest obtained through a trusted CI/coordinator channel.
    Expect {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_input_digest: String,
    },
    /// Admit a package matching the configured immutable expectation.
    Import {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    /// Inspect integrity and the shareable diagnostic report without admitting facts.
    Inspect {
        #[arg(long)]
        input: PathBuf,
    },
    /// Read a bounded page of portable index records without a checkout or compiler.
    Read {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectation {
    pub schema: String,
    pub service: String,
    pub repository_id: String,
    pub service_digest: String,
    pub revision: String,
    pub manifest_digest: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PartRef {
    kind: String,
    path: String,
    digest: String,
    bytes: u64,
    records: usize,
    encoding: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    service: Service,
    service_digest: String,
    revision: Option<String>,
    producer_version: String,
    compatibility_digest: String,
    outcome: String,
    parts: Vec<PartRef>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Part {
    schema: String,
    kind: String,
    records: Vec<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Header {
    schema: String,
    service: String,
    revision: String,
    service_digest: String,
    extractor: String,
    runtime_mode: String,
    coverage: String,
    boundaries: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Contract {
    id: String,
    value: Value,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    schema: String,
    service: String,
    manifest_digest: String,
    expectation_digest: String,
}

struct Package {
    manifest: Manifest,
    digest: String,
    records: BTreeMap<String, Vec<Value>>,
    raw_parts: BTreeMap<String, Vec<u8>>,
}

fn hash(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
fn sha(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn kind(s: &str) -> bool {
    matches!(
        s,
        "header" | "sources" | "observations" | "entrypoints" | "contracts" | "report"
    )
}
fn policy_path(id: &str) -> Result<String, ClewError> {
    if !store::valid_id(id) {
        return Err(invalid("invalid evidence service ID"));
    }
    Ok(format!("catalog/evidence-trust/{id}.json"))
}
fn selection_path(id: &str) -> Result<String, ClewError> {
    policy_path(id)?;
    Ok(format!(".codeclew/evidence/selected/{id}.json"))
}
fn package_path(d: &str) -> Result<String, ClewError> {
    if !hash(d) {
        return Err(invalid("invalid evidence package digest"));
    }
    Ok(format!("evidence/packages/{}", &d[7..]))
}
pub(super) fn retained(repo: &Repository, id: &str) -> Result<bool, ClewError> {
    let path = repo.path(&package_path(id)?)?;
    Ok(load(&path).is_ok_and(|p| p.digest == id))
}

/// Semantic compatibility does not require an installed compiler at the consumer.
/// Exact producer binaries remain recorded in MODULE_SCOPE/SOURCE_SCOPE and pinned
/// by the coordinator's expected manifest digest.
fn compatibility() -> Result<String, ClewError> {
    digest(&(
        SCHEMA,
        EXTRACTOR,
        SOURCE_EXTRACTOR,
        canonical::hash_bytes(include_bytes!("analysis.rs")),
        canonical::hash_bytes(include_bytes!("evidence_package.rs")),
        canonical::hash_bytes(include_bytes!("syntax.rs")),
        canonical::hash_bytes(include_bytes!("source_annotations.rs")),
        canonical::hash_bytes(include_bytes!("contracts.rs")),
        canonical::hash_bytes(include_bytes!("modules.rs")),
        canonical::hash_bytes(include_bytes!("../../../clew-facts/src/lib.rs")),
        canonical::hash_bytes(include_bytes!("../../../../Cargo.lock")),
        clew_framework_spring::implementation_digest(),
    ))
}

fn validate_policy(p: &Expectation, service: &Service) -> Result<(), ClewError> {
    if p.schema != POLICY_SCHEMA
        || p.service != service.id
        || !store::valid_id(&p.repository_id)
        || !hash(&p.service_digest)
        || !hash(&p.manifest_digest)
        || !sha(&p.revision)
        || p.sequence == 0
    {
        return Err(invalid(
            "invalid evidence expectation identity, revision or digest",
        ));
    }
    Ok(())
}
pub(super) fn policies(repo: &Repository) -> Result<BTreeMap<String, Expectation>, ClewError> {
    let rows: BTreeMap<String, Expectation> = repo.records("catalog/evidence-trust", "json")?;
    let services = repo.services()?;
    for (id, p) in &rows {
        let s = services
            .get(id)
            .ok_or_else(|| invalid("evidence expectation references an unknown service"))?;
        validate_policy(p, s)?;
        if p.service != *id {
            return Err(invalid("evidence expectation filename mismatch"));
        }
    }
    Ok(rows)
}

fn read_bytes(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>, ClewError> {
    store::relative(relative)?;
    let mut path = root.to_path_buf();
    for segment in Path::new(relative).components() {
        path.push(segment);
        if fs::symlink_metadata(&path)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid("evidence package symlinks are not supported"));
        }
    }
    let mut file = fs::File::open(&path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("evidence part exceeds its file byte bound"));
    }
    use std::io::Read;
    let mut data = Vec::new();
    file.by_ref()
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(io_error)?;
    if data.len() as u64 > limit {
        return Err(invalid("evidence part changed beyond its byte bound"));
    }
    Ok(data)
}

fn load(input: &Path) -> Result<Package, ClewError> {
    if fs::symlink_metadata(input)
        .map_err(io_error)?
        .file_type()
        .is_symlink()
    {
        return Err(invalid("package root cannot be a symlink"));
    }
    let data = read_bytes(input, "manifest.json", store::MAX_RECORD)?;
    let manifest: Manifest = serde_json::from_slice(&data).map_err(io_error)?;
    store::validate_service(&manifest.service)?;
    if manifest.schema != SCHEMA
        || manifest.service_digest != digest(&manifest.service)?
        || manifest.revision.as_ref().is_some_and(|r| !sha(r))
        || !hash(&manifest.compatibility_digest)
        || manifest.producer_version.len() > 80
        || !matches!(
            manifest.outcome.as_str(),
            "CAPTURED" | "PRODUCER_FAILURE" | "DIAGNOSTICS_ONLY"
        )
        || manifest.parts.is_empty()
        || manifest.parts.len() > MAX_PARTS
    {
        return Err(invalid("unsupported or inconsistent evidence manifest"));
    }
    let mut records: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut raw_parts = BTreeMap::new();
    let mut total = 0u64;
    let mut count = 0usize;
    for part in &manifest.parts {
        total = total
            .checked_add(part.bytes)
            .ok_or_else(|| invalid("evidence package byte overflow"))?;
        count = count
            .checked_add(part.records)
            .ok_or_else(|| invalid("evidence package record overflow"))?;
        if !kind(&part.kind)
            || !hash(&part.digest)
            || part.path != format!("parts/{}.json", &part.digest[7..])
            || part.encoding != "json"
            || part.bytes == 0
            || part.bytes > MAX_PART
            || total > MAX_TOTAL
            || part.records == 0
            || count > MAX_RECORDS
            || raw_parts.contains_key(&part.path)
        {
            return Err(invalid(
                "invalid evidence part kind, path, encoding, digest or aggregate limits",
            ));
        }
        let data = read_bytes(input, &part.path, part.bytes)?;
        if data.len() as u64 != part.bytes || canonical::hash_bytes(&data) != part.digest {
            return Err(invalid("evidence part digest mismatch"));
        }
        let value: Part = serde_json::from_slice(&data).map_err(io_error)?;
        if value.schema != PART_SCHEMA
            || value.kind != part.kind
            || value.records.len() != part.records
        {
            return Err(invalid("evidence part schema or count mismatch"));
        }
        records
            .entry(part.kind.clone())
            .or_default()
            .extend(value.records);
        raw_parts.insert(part.path.clone(), data);
    }
    if records.get("report").map(Vec::len) != Some(1) {
        return Err(invalid("package requires exactly one diagnostic report"));
    }
    if manifest.outcome == "CAPTURED" {
        if records.get("header").map(Vec::len) != Some(1) || manifest.revision.is_none() {
            return Err(invalid(
                "captured package requires exactly one evidence header and revision",
            ));
        }
    } else if records.keys().any(|k| k != "report") {
        return Err(invalid(
            "diagnostic-only or failed capture cannot carry admitted source facts",
        ));
    }
    let package = Package {
        digest: digest(&manifest)?,
        manifest,
        records,
        raw_parts,
    };
    if package.manifest.outcome == "CAPTURED" {
        package.evidence()?;
    }
    validate_report(&package.records["report"][0])?;
    let report: Report =
        serde_json::from_value(package.records["report"][0].clone()).map_err(io_error)?;
    if report.outcome != package.manifest.outcome
        || report.source_included != (package.manifest.outcome == "CAPTURED")
        || (package.manifest.outcome == "PRODUCER_FAILURE") != report.error_code.is_some()
    {
        return Err(invalid(
            "diagnostic report disagrees with package outcome or source inclusion",
        ));
    }
    Ok(package)
}

impl Package {
    fn evidence(&self) -> Result<ServiceEvidence, ClewError> {
        let h: Header = serde_json::from_value(
            self.records
                .get("header")
                .and_then(|r| r.first())
                .cloned()
                .ok_or_else(|| invalid("package contains no usable indexed evidence"))?,
        )
        .map_err(io_error)?;
        let mut e = ServiceEvidence {
            schema: h.schema,
            service: h.service,
            revision: h.revision,
            service_digest: h.service_digest,
            extractor: h.extractor,
            runtime_mode: h.runtime_mode,
            coverage: h.coverage,
            boundaries: h.boundaries,
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        for (kind, values) in &self.records {
            for value in values {
                match kind.as_str() {
                    "sources" => {
                        let v: Source = serde_json::from_value(value.clone()).map_err(io_error)?;
                        if e.sources.insert(v.id.clone(), v).is_some() {
                            return Err(invalid("duplicate package source ID"));
                        }
                    }
                    "observations" => {
                        let v: Observation =
                            serde_json::from_value(value.clone()).map_err(io_error)?;
                        if e.observations.insert(v.id.clone(), v).is_some() {
                            return Err(invalid("duplicate package observation ID"));
                        }
                    }
                    "entrypoints" => e
                        .entrypoints
                        .push(serde_json::from_value(value.clone()).map_err(io_error)?),
                    "contracts" => {
                        let v: Contract =
                            serde_json::from_value(value.clone()).map_err(io_error)?;
                        if e.contracts.insert(v.id, v.value).is_some() {
                            return Err(invalid("duplicate package contract ID"));
                        }
                    }
                    _ => {}
                }
            }
        }
        if e.service != self.manifest.service.id
            || Some(&e.revision) != self.manifest.revision.as_ref()
            || e.service_digest != self.manifest.service_digest
        {
            return Err(invalid(
                "package evidence disagrees with selected service or revision",
            ));
        }
        analysis::verify_evidence(&e)?;
        if (e.extractor == SOURCE_EXTRACTOR
            && (e.runtime_mode != "COMMITTED_SOURCE_NO_BUILD"
                || !matches!(e.coverage.as_str(), "SYNTAX" | "PARTIAL")))
            || (e.extractor == EXTRACTOR
                && !matches!(e.runtime_mode.as_str(), "DEVELOPMENT" | "RELEASE"))
        {
            return Err(invalid(
                "portable extractor cannot claim stronger runtime or coverage authority",
            ));
        }
        let mut seen = BTreeSet::new();
        for s in e.sources.values() {
            if !matches!(
                s.authority.as_str(),
                "EXACT_SNAPSHOT_TEXT" | "DECLARED_OPENAPI"
            ) || !hash(&s.evidence_digest)
                || s.url.as_ref().is_some_and(|s| {
                    let (base, anchor) = s.split_once('#').unwrap_or((s.as_str(), ""));
                    !store::safe_url(base)
                        || !anchor
                            .bytes()
                            .all(|b| b.is_ascii_digit() || matches!(b, b'L' | b'-'))
                })
            {
                return Err(invalid("invalid portable source authority or URL"));
            }
        }
        for o in e.observations.values() {
            if o.service != e.service || o.id.is_empty() || o.id.len() > 512 {
                return Err(invalid("portable observation service or identity mismatch"));
            }
        }
        for entry in &e.entrypoints {
            if entry.service != e.service
                || !seen.insert(&entry.id)
                || entry
                    .source_ids
                    .iter()
                    .any(|id| !e.sources.contains_key(id))
                || entry
                    .dependency_ids
                    .iter()
                    .any(|id| !e.observations.contains_key(id))
            {
                return Err(invalid("portable entrypoint graph has invalid references"));
            }
        }
        Ok(e)
    }
}

/// A closed diagnostic envelope; source text is present only in separate index parts.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Report {
    schema: String,
    producer_version: String,
    platform: String,
    architecture: String,
    language: String,
    profile: String,
    declared_dialect: Option<String>,
    project_java_minimum: u16,
    kotlin_worker_java: u16,
    outcome: String,
    error_code: Option<crate::error::ErrorCode>,
    stage: String,
    runtime_mode: String,
    retry: Vec<String>,
    worker_failure: Option<Value>,
    capabilities: Vec<Value>,
    semantic_outcomes: Vec<Value>,
    source_count: usize,
    observation_count: usize,
    entrypoint_count: usize,
    source_included: bool,
    limitations: Vec<String>,
}
fn validate_report(value: &Value) -> Result<(), ClewError> {
    let r: Report = serde_json::from_value(value.clone()).map_err(io_error)?;
    if r.schema != "codeclew-documentation-diagnostic-report/1.0" || bytes(value)?.len() > 16 * 1024
    {
        return Err(invalid("unsupported or oversized diagnostic report"));
    }
    Ok(())
}
fn report(
    service: &Service,
    outcome: &str,
    evidence: Option<&ServiceEvidence>,
    error: Option<&ClewError>,
) -> Result<Value, ClewError> {
    let worker_failure = error
        .and_then(|e| crate::worker_diagnostics::from_evidence(&e.evidence))
        .and_then(|d| crate::worker_diagnostics::safe_summary(&d));
    serde_json::to_value(Report {
        schema:"codeclew-documentation-diagnostic-report/1.0".into(), producer_version:env!("CARGO_PKG_VERSION").into(),
        platform:std::env::consts::OS.into(), architecture:std::env::consts::ARCH.into(), language:service.language.clone(),profile:service.profile.clone(),
        declared_dialect:service.source.as_ref().map(|s|s.dialect.clone()),project_java_minimum:crate::analysis_modules::JAVA_MIN_MAJOR,
        kotlin_worker_java:21,outcome:outcome.into(),error_code:error.map(|e|e.code.clone()),worker_failure:worker_failure.clone(),
        stage:worker_failure.as_ref().and_then(|v|v["stage"].as_str()).unwrap_or("LOCAL_SERVICE_CAPTURE").into(),
        runtime_mode:crate::runtime::RuntimeAuthority::from_environment().ok().flatten().map(|r|format!("{:?}",r.mode)).unwrap_or_else(||"UNAVAILABLE".into()),
        retry:vec![format!("clew docs modules list --root <docs> --service {}",service.id),format!("clew docs evidence report --root <docs> --service {} --output <new-report-directory>",service.id)],
        capabilities:super::modules::catalog()?.into_iter().map(|r|json!({"id":r["id"],"implementationDigest":r["implementationDigest"],"availability":r["availability"],"producer":r["producer"],"producers":r["producers"],"knownAnalyzers":r["knownAnalyzers"]})).collect(),
        semantic_outcomes:evidence.map(|e|e.observations.values().filter(|o|o.kind=="SOURCE_SCOPE").filter_map(|o|o.normalized.get("semantic").and_then(|s|s.get("provider")).cloned()).collect()).unwrap_or_default(),
        source_count:evidence.map_or(0,|e|e.sources.len()), observation_count:evidence.map_or(0,|e|e.observations.len()),entrypoint_count:evidence.map_or(0,|e|e.entrypoints.len()),
        source_included:outcome=="CAPTURED", limitations:vec![
            "Declared dialect is not a compiler admission result. Kotlin project targeting and the worker JVM are separate requirements.".into(),
            "Diagnostic metadata omits raw stderr, environment values, local paths and mutable session identities.".into(),
            "Indexed source parts preserve application text and literals. A report alone cannot reproduce omitted source or compiler failures.".into(),
            "Offline evidence describes a selected immutable revision; it cannot discover a newer remote HEAD.".into(),
        ],
    }).map_err(io_error)
}

fn make_parts(
    kind: &str,
    values: Vec<Value>,
    parts: &mut BTreeMap<String, Vec<u8>>,
    refs: &mut Vec<PartRef>,
) -> Result<(), ClewError> {
    let mut group = Vec::new();
    let mut size = 0;
    let flush = |group: &mut Vec<Value>,
                 parts: &mut BTreeMap<String, Vec<u8>>,
                 refs: &mut Vec<PartRef>|
     -> Result<(), ClewError> {
        if group.is_empty() {
            return Ok(());
        }
        let records = group.len();
        let data = bytes(&Part {
            schema: PART_SCHEMA.into(),
            kind: kind.into(),
            records: std::mem::take(group),
        })?;
        if data.len() as u64 > MAX_PART {
            return Err(invalid(
                "one indexed record exceeds the portable part limit",
            ));
        }
        let digest = canonical::hash_bytes(&data);
        let path = format!("parts/{}.json", &digest[7..]);
        refs.push(PartRef {
            kind: kind.into(),
            path: path.clone(),
            digest,
            bytes: data.len() as u64,
            records,
            encoding: "json".into(),
        });
        parts.insert(path, data);
        Ok(())
    };
    for value in values {
        let next = bytes(&value)?.len();
        if size + next > 512 * 1024 || group.len() >= 512 {
            flush(&mut group, parts, refs)?;
            size = 0;
        }
        group.push(value);
        size += next;
    }
    flush(&mut group, parts, refs)
}
fn capture(
    repo: &Repository,
    id: &str,
    output: &Path,
    diagnostic_only: bool,
) -> Result<Value, ClewError> {
    let service = repo
        .services()?
        .remove(id)
        .ok_or_else(|| invalid("unknown service"))?;
    let input_digest = repo.input_digest()?;
    // Never turn an imported package back into a claimed local producer result.
    let revision = analysis::bound_repository(repo, &service)
        .and_then(|p| analysis::git(&p, &["rev-parse", "--verify", "HEAD^{commit}"]))
        .ok();
    let result = analysis::capture_local(repo, &service);
    if repo.input_digest()? != input_digest {
        return Err(invalid(
            "documentation input changed during evidence capture",
        ));
    }
    let outcome = if result.is_err() {
        "PRODUCER_FAILURE"
    } else if diagnostic_only {
        "DIAGNOSTICS_ONLY"
    } else {
        "CAPTURED"
    };
    let report = report(
        &service,
        outcome,
        result.as_ref().ok(),
        result.as_ref().err(),
    )?;
    let mut parts = BTreeMap::new();
    let mut refs = Vec::new();
    make_parts("report", vec![report.clone()], &mut parts, &mut refs)?;
    let revision = if let Ok(e) = result {
        let revision = Some(e.revision.clone());
        if !diagnostic_only {
            make_parts(
                "header",
                vec![json!(Header {
                    schema: e.schema,
                    service: e.service,
                    revision: e.revision,
                    service_digest: e.service_digest,
                    extractor: e.extractor,
                    runtime_mode: e.runtime_mode,
                    coverage: e.coverage,
                    boundaries: e.boundaries
                })],
                &mut parts,
                &mut refs,
            )?;
            make_parts(
                "sources",
                e.sources.into_values().map(|v| json!(v)).collect(),
                &mut parts,
                &mut refs,
            )?;
            make_parts(
                "observations",
                e.observations.into_values().map(|v| json!(v)).collect(),
                &mut parts,
                &mut refs,
            )?;
            make_parts(
                "entrypoints",
                e.entrypoints.into_iter().map(|v| json!(v)).collect(),
                &mut parts,
                &mut refs,
            )?;
            make_parts(
                "contracts",
                e.contracts
                    .into_iter()
                    .map(|(id, value)| json!(Contract { id, value }))
                    .collect(),
                &mut parts,
                &mut refs,
            )?;
        }
        revision
    } else {
        revision
    };
    let manifest = Manifest {
        schema: SCHEMA.into(),
        service_digest: digest(&service)?,
        service,
        revision,
        producer_version: env!("CARGO_PKG_VERSION").into(),
        compatibility_digest: compatibility()?,
        outcome: outcome.into(),
        parts: refs,
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(io_error)?;
    let parent = parent.canonicalize().map_err(io_error)?;
    let output = parent.join(
        output
            .file_name()
            .ok_or_else(|| invalid("package output requires a directory name"))?,
    );
    if output.exists() || fs::symlink_metadata(&output).is_ok() {
        return Err(invalid(
            "package output already exists; select a new immutable output directory",
        ));
    }
    let temporary = tempfile::tempdir_in(&parent).map_err(io_error)?;
    fs::create_dir(temporary.path().join("parts")).map_err(io_error)?;
    for (path, data) in parts {
        fs::write(temporary.path().join(path), data).map_err(io_error)?;
    }
    fs::write(temporary.path().join("manifest.json"), bytes(&manifest)?).map_err(io_error)?;
    let package = load(temporary.path())?;
    fs::rename(temporary.path(), &output).map_err(io_error)?;
    Ok(
        json!({"schema":"codeclew-docs-evidence-capture/1.0","status":outcome,"manifestDigest":package.digest,"service":id,"revision":manifest.revision,"serviceDigest":manifest.service_digest,"report":report}),
    )
}

fn check_expected(repo: &Repository, package: &Package) -> Result<Expectation, ClewError> {
    let service = repo
        .services()?
        .remove(&package.manifest.service.id)
        .ok_or_else(|| invalid("package service is not registered"))?;
    let policy = policies(repo)?
        .remove(&service.id)
        .ok_or_else(|| invalid("configure a trusted evidence expectation before import"))?;
    if policy.repository_id != service.repository_id
        || policy.service_digest != digest(&service)?
        || package.manifest.service != service
        || package.digest != policy.manifest_digest
        || package.manifest.revision.as_ref() != Some(&policy.revision)
        || package.manifest.compatibility_digest != compatibility()?
    {
        return Err(invalid(
            "package does not match configured service, revision, trusted digest or supported producer rules",
        ));
    }
    Ok(policy)
}
fn import(repo: &Repository, input: &Path) -> Result<Value, ClewError> {
    let package = load(input)?;
    if package.manifest.outcome == "DIAGNOSTICS_ONLY" {
        return Err(invalid(
            "a diagnostic-only report cannot become source evidence",
        ));
    }
    let _lock = repo.lock()?;
    let policy = check_expected(repo, &package)?;
    super::updates::admit_package(repo, &policy.service, package.manifest.revision.as_deref())?;
    let root = package_path(&package.digest)?;
    for (path, data) in &package.raw_parts {
        repo.atomic(&format!("{root}/{path}"), data)?;
    }
    repo.atomic(&format!("{root}/manifest.json"), &bytes(&package.manifest)?)?;
    let pointer = Selection {
        schema: "codeclew-documentation-evidence-selection/1.0".into(),
        service: policy.service.clone(),
        manifest_digest: package.digest.clone(),
        expectation_digest: digest(&policy)?,
    };
    let pointer_path = selection_path(&policy.service)?;
    let same = repo.path(&pointer_path)?.exists()
        && fs::read(repo.path(&pointer_path)?).map_err(io_error)? == bytes(&pointer)?;
    repo.atomic(&pointer_path, &bytes(&pointer)?)?;
    Ok(
        json!({"schema":"codeclew-docs-evidence-import/1.0","status":if same {"CURRENT"} else if package.manifest.outcome=="PRODUCER_FAILURE" {"PRODUCER_FAILURE_RECORDED"} else {"IMPORTED"},"service":policy.service,"revision":policy.revision,"manifestDigest":package.digest,"sequence":policy.sequence}),
    )
}

pub(super) fn selected(
    repo: &Repository,
    service: &Service,
) -> Result<Option<ServiceEvidence>, ClewError> {
    let Some(policy) = policies(repo)?.remove(&service.id) else {
        return Ok(None);
    };
    let pointer_path = repo.path(&selection_path(&service.id)?)?;
    let pointer: Selection = if pointer_path.exists() {
        store::read(&pointer_path, store::MAX_RECORD)?
    } else {
        // A trusted expectation and a retained immutable artifact are sufficient
        // to reconstruct disposable coordinator cache after loss.
        Selection {
            schema: "codeclew-documentation-evidence-selection/1.0".into(),
            service: service.id.clone(),
            manifest_digest: policy.manifest_digest.clone(),
            expectation_digest: digest(&policy)?,
        }
    };
    if pointer.schema != "codeclew-documentation-evidence-selection/1.0"
        || pointer.service != service.id
        || pointer.manifest_digest != policy.manifest_digest
        || pointer.expectation_digest != digest(&policy)?
    {
        return Err(invalid(
            "selected portable result is missing or superseded by the coordinator expectation",
        ));
    }
    let package = load(&repo.path(&package_path(&pointer.manifest_digest)?)?)?;
    check_expected(repo, &package)?;
    if package.manifest.outcome != "CAPTURED" {
        let report: Report =
            serde_json::from_value(package.records["report"][0].clone()).map_err(io_error)?;
        let mut error = ClewError::new(
            report
                .error_code
                .unwrap_or(crate::error::ErrorCode::InvalidInput),
            format!(
                "portable producer failure at {}; inspect the selected evidence report",
                policy.revision
            ),
        );
        error.evidence.push(format!("documentation-evidence-report:{}",json!({"manifestDigest":package.digest,"revision":policy.revision,"report":package.records["report"][0]})));
        return Err(error);
    }
    let mut e = package.evidence()?;
    let id = analysis::dependency_id(
        &e.service,
        "EVIDENCE_PACKAGE",
        "coordinator-selected-revision",
    )?;
    let normalized = json!({"schema":"codeclew-documentation-portable-influence/1.0","expectation":policy,"producerRules":package.manifest.compatibility_digest});
    e.observations.insert(
        id.clone(),
        Observation {
            id,
            kind: "EVIDENCE_PACKAGE".into(),
            service: e.service.clone(),
            symbol: "coordinator-selected-revision".into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids: vec![],
        },
    );
    e.boundaries
        .push("PORTABLE_EVIDENCE_AT_SELECTED_REVISION".into());
    Ok(Some(e))
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Report {
            root,
            service,
            output,
            include_index,
        } => capture(&Repository::open(&root)?, &service, &output, !include_index),
        Command::Capture {
            root,
            service,
            output,
            diagnostics_only,
        } => capture(
            &Repository::open(&root)?,
            &service,
            &output,
            diagnostics_only,
        ),
        Command::Import { root, input } => import(&Repository::open(&root)?, &input),
        Command::Expect {
            root,
            input,
            expected_input_digest,
        } => {
            let repo = Repository::open(&root)?;
            let value: Expectation = store::read(&input, store::MAX_RECORD)?;
            let service = repo
                .services()?
                .remove(&value.service)
                .ok_or_else(|| invalid("unknown expectation service"))?;
            validate_policy(&value, &service)?;
            if value.repository_id != service.repository_id
                || value.service_digest != digest(&service)?
            {
                return Err(invalid(
                    "expectation must bind the current service configuration",
                ));
            }
            // Hold the same lock through sequence comparison and CAS publication.
            let _lock = repo.lock()?;
            let current = repo.input_digest()?;
            if let Some(old) = policies(&repo)?.get(&value.service) {
                if old == &value {
                    return Ok(json!({"status":"CURRENT","inputDigest":current}));
                }
                if value.sequence <= old.sequence {
                    return Err(invalid(
                        "evidence expectation replay: sequence must increase",
                    ));
                }
            }
            if current != expected_input_digest {
                return Err(invalid(
                    "documentation input changed before expectation update",
                ));
            }
            repo.atomic(&policy_path(&value.service)?, &bytes(&value)?)?;
            Ok(
                json!({"schema":"codeclew-docs-evidence-expect/1.0","status":"EXPECTED","inputDigest":repo.input_digest()?,"service":value.service,"sequence":value.sequence}),
            )
        }
        Command::Inspect { input } => {
            let p = load(&input)?;
            Ok(
                json!({"schema":"codeclew-docs-evidence-inspect/1.0","status":"INTEGRITY_CHECKED_NOT_ADMITTED","manifestDigest":p.digest,"service":p.manifest.service.id,"repositoryId":p.manifest.service.repository_id,"serviceDigest":p.manifest.service_digest,"revision":p.manifest.revision,"compatible":p.manifest.compatibility_digest==compatibility()?,"outcome":p.manifest.outcome,"parts":p.manifest.parts.len(),"report":p.records["report"][0]}),
            )
        }
        Command::Read {
            input,
            kind: k,
            cursor,
            limit,
        } => {
            if !kind(&k) {
                return Err(invalid("unknown portable index record kind"));
            }
            let mut p = load(&input)?;
            super::cli::page(
                &digest(&(&p.digest, &k))?,
                p.records.remove(&k).unwrap_or_default(),
                cursor.as_deref(),
                limit as usize,
                json!({"schema":"codeclew-docs-evidence-read/1.0","status":"INTEGRITY_CHECKED_NOT_ADMITTED","manifestDigest":p.digest,"kind":k}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_records_are_split_and_oversized_records_refused() {
        let mut parts = BTreeMap::new();
        let mut refs = Vec::new();
        make_parts(
            "observations",
            (0..1500)
                .map(|id| json!({"id":id,"value":"retained"}))
                .collect(),
            &mut parts,
            &mut refs,
        )
        .unwrap();
        assert_eq!(refs.len(), 3);
        assert_eq!(refs.iter().map(|p| p.records).sum::<usize>(), 1500);
        for part in refs {
            assert_eq!(canonical::hash_bytes(&parts[&part.path]), part.digest);
            assert!(part.bytes <= MAX_PART);
        }
        assert!(
            make_parts(
                "sources",
                vec![json!({"text":"x".repeat(MAX_PART as usize)})],
                &mut parts,
                &mut vec![]
            )
            .is_err()
        );
    }
    #[test]
    fn diagnostic_report_drops_private_worker_envelope_and_error_text() {
        let service: Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"orders","repository":"https://example.invalid/orders","language":"kotlin","profile":"source-syntax","source":{"roots":["."],"dialect":"1.9"},"targetRef":"HEAD"})).unwrap();
        let mut failure = ClewError::new(
            crate::error::ErrorCode::WorkerCrashed,
            "secret-error-marker /synthetic-private-root/.credentials/session",
        );
        failure.evidence.push(format!("worker-process-diagnostic:{}",json!({"schema":"codeclew-worker-process-diagnostic/1.0","stage":"INDEX_FILES","identity":{"session":"private-session-marker"},"process":{"status":"EXITED","exitCode":17,"signal":null},"stderr":{"path":"/synthetic-private-root/stderr","text":"secret-stderr-marker"}})));
        let value = report(&service, "PRODUCER_FAILURE", None, Some(&failure)).unwrap();
        let text = value.to_string();
        for secret in [
            "secret-error-marker",
            "private-session-marker",
            "secret-stderr-marker",
            "/synthetic-private-root",
        ] {
            assert!(!text.contains(secret));
        }
        assert_eq!(value["errorCode"], "WORKER_CRASHED");
        assert_eq!(value["workerFailure"]["exitCode"], 17);
        assert_eq!(value["stage"], "INDEX_FILES");
        assert_eq!(value["declaredDialect"], "1.9");
    }
}
