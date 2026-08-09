//! Non-serializable authority receipts for semantic completeness inputs.
//!
//! A caller may propose a Thread IR, but cannot turn that proposal into an
//! authoritative receipt. This module rebuilds it through the Kotlin worker,
//! resolves every source anchor against the live checkout and runs the
//! snapshot's configured compile/tests before issuing session-bound handles.

use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use crate::model::{BuildSystem, CompletenessStatus, ThreadIr};
use crate::proto::RequestKind;
use crate::transaction;
use crate::worker::WorkerClient;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use uuid::Uuid;
use walkdir::WalkDir;

/// Capability handle. Its fields are private and the type deliberately has no
/// serde implementation: JSON cannot manufacture or replay it in a new
/// authority session.
#[derive(Debug)]
pub struct VerifiedThreadReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Capability issued for a compiler-resolved test whose assertion consumes
/// the result of the production callable proven by a thread receipt.
#[derive(Debug)]
pub struct VerifiedBehavioralTestReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// Capability issued only after the configured build/test lifecycle succeeds.
#[derive(Debug)]
pub struct ValidationReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
}

/// The currently proven structural contour. New families must add their own
/// worker-derived binder; changing this label never changes the evidence.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProvenStructuralFamily {
    ProducerTransformConsumer,
}

/// Model-owned intent for the narrow D02 family. It contains no symbols,
/// source, edges, anchors or oracle claims; those are bound by the authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProducerTransformConsumerGoal {
    pub schema: String,
    pub base_revision: String,
}

impl ProducerTransformConsumerGoal {
    pub fn new(base_revision: impl Into<String>) -> Self {
        Self {
            schema: "producer-transform-consumer-goal/0.1".into(),
            base_revision: base_revision.into(),
        }
    }

    fn is_valid_for(&self, revision: &str) -> bool {
        self.schema == "producer-transform-consumer-goal/0.1"
            && !self.base_revision.is_empty()
            && self.base_revision == revision
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompleteForSummary {
    pub schema: String,
    pub family: ProvenStructuralFamily,
    pub revision: String,
    pub producer_node: String,
    pub transformer_node: String,
    pub consumer_node: String,
    pub goal_fingerprint: String,
    pub evidence_fingerprint: String,
}

/// Opaque family-relative theorem receipt. Like its prerequisites, it cannot
/// be deserialized or constructed outside this module.
#[derive(Debug)]
pub struct CompleteForReceipt {
    session_id: Uuid,
    receipt_id: Uuid,
    summary: CompleteForSummary,
}

impl CompleteForReceipt {
    pub fn summary(&self) -> &CompleteForSummary {
        &self.summary
    }
}

/// The only transferable result is a lossy summary. It is evidence about an
/// authority decision, not a capability that can authorize another decision.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceBundleSummary {
    pub schema: String,
    pub revision: String,
    pub thread_count: usize,
    pub behavioral_test_count: usize,
    pub evidence_fingerprint: String,
    pub validation_artifact_hash: String,
    pub executed_test_count: usize,
    pub compile_duration_ms: u64,
    pub test_duration_ms: u64,
}

#[derive(Debug)]
pub struct AuthoritativeEvidenceBundle {
    summary: EvidenceBundleSummary,
}

impl AuthoritativeEvidenceBundle {
    pub fn summary(&self) -> &EvidenceBundleSummary {
        &self.summary
    }
}

#[derive(Debug)]
struct VerifiedThread {
    fingerprint: String,
    thread: ThreadIr,
    source_files: BTreeMap<PathBuf, String>,
}

#[derive(Debug)]
struct VerifiedBehavioralTest {
    fingerprint: String,
    target_compiler_symbol: String,
    class_name: String,
    test_name: String,
    source_files: BTreeMap<PathBuf, String>,
}

#[derive(Debug)]
struct ValidationRun {
    thread_set_fingerprint: String,
    test_set_fingerprint: String,
    artifact_hash: String,
    executed_test_count: usize,
    compile_duration_ms: u64,
    test_duration_ms: u64,
}

/// Process-local authority. Receipts from one instance are meaningless to
/// every other instance, even for the same checkout and revision.
pub struct EvidenceAuthority {
    session_id: Uuid,
    repo: PathBuf,
    revision: String,
    threads: BTreeMap<Uuid, VerifiedThread>,
    tests: BTreeMap<Uuid, VerifiedBehavioralTest>,
    validations: BTreeMap<Uuid, ValidationRun>,
    completions: BTreeSet<Uuid>,
}

impl EvidenceAuthority {
    pub fn open(repo: &Path, expected_revision: &str) -> Result<Self, SthreadError> {
        let repo = repo.canonicalize().map_err(|error| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                format!("cannot resolve evidence repository: {error}"),
            )
        })?;
        let revision = git_head(&repo)?;
        if revision != expected_revision {
            return Err(SthreadError::new(
                ErrorCode::ProjectModelChanged,
                format!("authority expected revision {expected_revision}, found {revision}"),
            ));
        }
        ensure_clean_checkout(&repo)?;
        Ok(Self {
            session_id: Uuid::new_v4(),
            repo,
            revision,
            threads: BTreeMap::new(),
            tests: BTreeMap::new(),
            validations: BTreeMap::new(),
            completions: BTreeSet::new(),
        })
    }

    /// Rebuilds the proposal through the live worker and accepts it only when
    /// the resulting Thread IR is byte-for-byte canonical-equivalent.
    pub fn verify_thread(
        &mut self,
        proposed: &ThreadIr,
        worker: &mut WorkerClient,
    ) -> Result<VerifiedThreadReceipt, SthreadError> {
        self.ensure_revision()?;
        if proposed.snapshot.base_revision != self.revision {
            return Err(SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                "proposed thread belongs to another revision",
            ));
        }
        let project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":proposed.snapshot.compilation}),
        )?;
        if project.get("projectModelHash").and_then(Value::as_str)
            != Some(proposed.snapshot.project_model_hash.as_str())
        {
            return Err(SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                "live project model does not match proposed Thread IR",
            ));
        }
        let rebuilt =
            transaction::rebuild_thread(&self.repo, proposed, &project, &self.revision, worker)?;
        let proposed_hash = canonical::hash(proposed).map_err(internal)?;
        let rebuilt_hash = canonical::hash(&rebuilt).map_err(internal)?;
        if proposed_hash != rebuilt_hash {
            return Err(SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                "worker-rebuilt Thread IR differs from the proposed evidence packet",
            ));
        }
        if rebuilt.completeness.status != CompletenessStatus::CompleteSupportedSubset
            || !rebuilt.completeness.boundaries.is_empty()
            || !rebuilt.external_summaries.is_empty()
        {
            return Err(SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "worker-rebuilt Thread IR has an unsupported boundary",
            ));
        }
        let source_files = verify_live_sources(&self.repo, &rebuilt)?;
        let receipt_id = Uuid::new_v4();
        self.threads.insert(
            receipt_id,
            VerifiedThread {
                fingerprint: rebuilt_hash,
                thread: rebuilt,
                source_files,
            },
        );
        Ok(VerifiedThreadReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Resolves a test through K2 and accepts it only when a recognized
    /// assertion consumes the exact compiler callable proven by `target`.
    pub fn verify_behavioral_test(
        &mut self,
        test_symbol: &str,
        compilation: &str,
        target: &VerifiedThreadReceipt,
        worker: &mut WorkerClient,
    ) -> Result<VerifiedBehavioralTestReceipt, SthreadError> {
        self.ensure_revision()?;
        let targets = self.resolve_threads(&[target])?;
        let [target] = targets.as_slice() else {
            unreachable!("one receipt resolves to one thread")
        };
        let candidates = producer_transform_consumer_candidates(&target.thread);
        let [binding] = candidates.as_slice() else {
            return Err(SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "target thread does not have one unique producer-transform-consumer binding",
            ));
        };
        let target_compiler_symbol = compiler_owner_symbol(&target.thread, &binding.0)?;
        transaction::validate_worktree(
            &self.repo,
            target.thread.snapshot.build_system,
            &target.thread.snapshot.build_launcher,
            &target.thread.snapshot.compile_task,
            &[],
        )?;
        let project = worker.request(
            RequestKind::OpenProject,
            &json!({"repo":self.repo,"compilation":compilation}),
        )?;
        if project.get("sourceSet").and_then(Value::as_str) != Some("test") {
            return Err(SthreadError::new(
                ErrorCode::InvalidInput,
                "behavioral evidence must come from the test compilation",
            ));
        }
        let resolution = worker.request(
            RequestKind::ResolveSymbol,
            &json!({"repo":self.repo,"compilation":compilation,"symbol":test_symbol}),
        )?;
        if resolution.get("k2Validated").and_then(Value::as_bool) != Some(true)
            || resolution
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.get("severity").and_then(Value::as_str) == Some("ERROR"))
                })
        {
            return Err(SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "behavioral test is not cleanly resolved by K2",
            ));
        }
        verify_assertion_of_target(&resolution, &target_compiler_symbol)?;
        let declaration = resolution
            .get("declaration")
            .ok_or_else(|| invalid_source("test resolution has no declaration"))?;
        let identity = declaration
            .get("symbolIdentity")
            .ok_or_else(|| invalid_source("test declaration has no symbol identity"))?;
        let package = required_str(identity, "package")?;
        let containers = identity
            .get("containingDeclarations")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_source("test declaration has no containing class"))?;
        if containers.is_empty() {
            return Err(invalid_source(
                "behavioral test must belong to a test class",
            ));
        }
        let class_name = std::iter::once(package)
            .chain(containers.iter().filter_map(Value::as_str))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(".");
        let test_name = required_str(declaration, "name")?.to_owned();
        let source_files = verify_resolution_source(&self.repo, &resolution)?;
        let fingerprint = canonical::hash(&(
            &self.revision,
            compilation,
            &target_compiler_symbol,
            &class_name,
            &test_name,
            &resolution,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.tests.insert(
            receipt_id,
            VerifiedBehavioralTest {
                fingerprint,
                target_compiler_symbol,
                class_name,
                test_name,
                source_files,
            },
        );
        Ok(VerifiedBehavioralTestReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Runs the exact compile/test plan carried by the verified snapshot. A
    /// validation handle cannot be created from a claimed exit code or hash.
    pub fn run_validation(
        &mut self,
        receipts: &[&VerifiedThreadReceipt],
        tests: &[&VerifiedBehavioralTestReceipt],
    ) -> Result<ValidationReceipt, SthreadError> {
        self.ensure_revision()?;
        let verified = self.resolve_threads(receipts)?;
        let verified_tests = self.resolve_tests(tests)?;
        let primary = verified[0];
        for item in &verified {
            verify_sources_current(&self.repo, &item.source_files)?;
            if item.thread.snapshot.build_system != primary.thread.snapshot.build_system
                || item.thread.snapshot.build_launcher != primary.thread.snapshot.build_launcher
                || item.thread.snapshot.compile_task != primary.thread.snapshot.compile_task
                || item.thread.snapshot.test_tasks != primary.thread.snapshot.test_tasks
            {
                return Err(SthreadError::new(
                    ErrorCode::InvalidInput,
                    "one validation receipt cannot cover different build plans",
                ));
            }
        }
        for test in &verified_tests {
            verify_sources_current(&self.repo, &test.source_files)?;
        }
        if primary.thread.snapshot.test_tasks.is_empty() {
            return Err(SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "snapshot has no configured behavioral test task",
            ));
        }
        if primary.thread.snapshot.build_system != BuildSystem::Gradle {
            return Err(SthreadError::new(
                ErrorCode::UnsupportedProjectConfiguration,
                "fresh behavioral-test receipts currently support Gradle only",
            ));
        }
        let (compile_duration_ms, test_duration_ms) = transaction::validate_worktree_fresh(
            &self.repo,
            primary.thread.snapshot.build_system,
            &primary.thread.snapshot.build_launcher,
            &primary.thread.snapshot.compile_task,
            &primary.thread.snapshot.test_tasks,
        )?;
        let thread_set_fingerprint = thread_set_fingerprint(&verified)?;
        let (test_artifact_hash, executed_test_count) = test_artifact(
            &self.repo,
            primary.thread.snapshot.build_system,
            &verified_tests,
        )?;
        let test_set_fingerprint = test_set_fingerprint(&verified_tests)?;
        let artifact_hash = canonical::hash(&(
            &self.revision,
            &thread_set_fingerprint,
            &test_set_fingerprint,
            primary.thread.snapshot.build_system,
            &primary.thread.snapshot.build_launcher,
            &primary.thread.snapshot.compile_task,
            &primary.thread.snapshot.test_tasks,
            &test_artifact_hash,
            executed_test_count,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.validations.insert(
            receipt_id,
            ValidationRun {
                thread_set_fingerprint,
                test_set_fingerprint,
                artifact_hash,
                executed_test_count,
                compile_duration_ms,
                test_duration_ms,
            },
        );
        Ok(ValidationReceipt {
            session_id: self.session_id,
            receipt_id,
        })
    }

    /// Joins worker/source evidence and validation evidence. This does not yet
    /// claim a structural-family theorem; it is the non-forgeable prerequisite
    /// that the rejected COMPLETE_FOR implementation lacked.
    pub fn authorize_bundle(
        &self,
        receipts: &[&VerifiedThreadReceipt],
        tests: &[&VerifiedBehavioralTestReceipt],
        validation: &ValidationReceipt,
    ) -> Result<AuthoritativeEvidenceBundle, SthreadError> {
        self.ensure_revision()?;
        if validation.session_id != self.session_id {
            return Err(wrong_session("validation"));
        }
        let verified = self.resolve_threads(receipts)?;
        let verified_tests = self.resolve_tests(tests)?;
        for item in &verified {
            verify_sources_current(&self.repo, &item.source_files)?;
        }
        for test in &verified_tests {
            verify_sources_current(&self.repo, &test.source_files)?;
        }
        let thread_set_fingerprint = thread_set_fingerprint(&verified)?;
        let test_set_fingerprint = test_set_fingerprint(&verified_tests)?;
        let run = self
            .validations
            .get(&validation.receipt_id)
            .ok_or_else(|| invalid_receipt("validation"))?;
        if run.thread_set_fingerprint != thread_set_fingerprint {
            return Err(SthreadError::new(
                ErrorCode::PreconditionFailed,
                "validation receipt covers a different exact thread set",
            ));
        }
        if run.test_set_fingerprint != test_set_fingerprint {
            return Err(SthreadError::new(
                ErrorCode::PreconditionFailed,
                "validation receipt covers a different exact behavioral-test set",
            ));
        }
        let evidence_fingerprint = canonical::hash(&(
            &self.session_id,
            &self.revision,
            &thread_set_fingerprint,
            &test_set_fingerprint,
            &run.artifact_hash,
        ))
        .map_err(internal)?;
        Ok(AuthoritativeEvidenceBundle {
            summary: EvidenceBundleSummary {
                schema: "authoritative-semantic-evidence/0.1".into(),
                revision: self.revision.clone(),
                thread_count: verified.len(),
                behavioral_test_count: verified_tests.len(),
                evidence_fingerprint,
                validation_artifact_hash: run.artifact_hash.clone(),
                executed_test_count: run.executed_test_count,
                compile_duration_ms: run.compile_duration_ms,
                test_duration_ms: run.test_duration_ms,
            },
        })
    }

    /// Proves the narrow producer-transform-consumer family from an exact
    /// worker-issued data-flow chain and the validation receipt for the same
    /// thread set. No role names or edge labels are accepted from the caller.
    pub fn complete_for_producer_transform_consumer(
        &mut self,
        goal: &ProducerTransformConsumerGoal,
        receipts: &[&VerifiedThreadReceipt],
        tests: &[&VerifiedBehavioralTestReceipt],
        validation: &ValidationReceipt,
    ) -> Result<CompleteForReceipt, SthreadError> {
        if !goal.is_valid_for(&self.revision) {
            return Err(SthreadError::new(
                ErrorCode::PreconditionFailed,
                "producer-transform-consumer goal does not match authority revision",
            ));
        }
        let bundle = self.authorize_bundle(receipts, tests, validation)?;
        let verified = self.resolve_threads(receipts)?;
        let verified_tests = self.resolve_tests(tests)?;
        let mut candidates = verified
            .iter()
            .flat_map(|item| {
                producer_transform_consumer_candidates(&item.thread)
                    .into_iter()
                    .map(|(producer, transformer, consumer)| {
                        (item.fingerprint.clone(), producer, transformer, consumer)
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        let [binding] = candidates.as_slice() else {
            return Err(SthreadError::new(
                if candidates.is_empty() {
                    ErrorCode::IncompleteSemanticAnalysis
                } else {
                    ErrorCode::AmbiguousTarget
                },
                if candidates.is_empty() {
                    "worker evidence has no complete producer-transform-consumer chain"
                } else {
                    "worker evidence has multiple producer-transform-consumer chains"
                },
            ));
        };
        let bound_thread = verified
            .iter()
            .find(|item| item.fingerprint == binding.0)
            .ok_or_else(|| invalid_source("bound producer has no verified thread"))?;
        let target_compiler_symbol = compiler_owner_symbol(&bound_thread.thread, &binding.1)?;
        if !verified_tests
            .iter()
            .any(|test| test.target_compiler_symbol == target_compiler_symbol)
        {
            return Err(SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "no verified behavioral test asserts the bound production callable",
            ));
        }
        let goal_fingerprint = canonical::hash(goal).map_err(internal)?;
        let evidence_fingerprint = canonical::hash(&(
            bundle.summary(),
            ProvenStructuralFamily::ProducerTransformConsumer,
            &goal_fingerprint,
            binding,
        ))
        .map_err(internal)?;
        let receipt_id = Uuid::new_v4();
        self.completions.insert(receipt_id);
        Ok(CompleteForReceipt {
            session_id: self.session_id,
            receipt_id,
            summary: CompleteForSummary {
                schema: "complete-for-authority/0.1".into(),
                family: ProvenStructuralFamily::ProducerTransformConsumer,
                revision: self.revision.clone(),
                producer_node: binding.1.clone(),
                transformer_node: binding.2.clone(),
                consumer_node: binding.3.clone(),
                goal_fingerprint,
                evidence_fingerprint,
            },
        })
    }

    pub fn recognizes_complete_for(
        &self,
        receipt: &CompleteForReceipt,
    ) -> Result<bool, SthreadError> {
        self.ensure_revision()?;
        Ok(receipt.session_id == self.session_id && self.completions.contains(&receipt.receipt_id))
    }

    fn resolve_threads<'a>(
        &'a self,
        receipts: &[&VerifiedThreadReceipt],
    ) -> Result<Vec<&'a VerifiedThread>, SthreadError> {
        if receipts.is_empty() {
            return Err(SthreadError::new(
                ErrorCode::InvalidInput,
                "authority needs at least one verified thread",
            ));
        }
        let mut ids = BTreeSet::new();
        receipts
            .iter()
            .map(|receipt| {
                if receipt.session_id != self.session_id {
                    return Err(wrong_session("thread"));
                }
                if !ids.insert(receipt.receipt_id) {
                    return Err(SthreadError::new(
                        ErrorCode::InvalidInput,
                        "duplicate verified thread receipt",
                    ));
                }
                self.threads
                    .get(&receipt.receipt_id)
                    .ok_or_else(|| invalid_receipt("thread"))
            })
            .collect()
    }

    fn resolve_tests<'a>(
        &'a self,
        receipts: &[&VerifiedBehavioralTestReceipt],
    ) -> Result<Vec<&'a VerifiedBehavioralTest>, SthreadError> {
        if receipts.is_empty() {
            return Err(SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "authority needs at least one verified behavioral test",
            ));
        }
        let mut ids = BTreeSet::new();
        receipts
            .iter()
            .map(|receipt| {
                if receipt.session_id != self.session_id {
                    return Err(wrong_session("behavioral test"));
                }
                if !ids.insert(receipt.receipt_id) {
                    return Err(SthreadError::new(
                        ErrorCode::InvalidInput,
                        "duplicate behavioral-test receipt",
                    ));
                }
                self.tests
                    .get(&receipt.receipt_id)
                    .ok_or_else(|| invalid_receipt("behavioral test"))
            })
            .collect()
    }

    fn ensure_revision(&self) -> Result<(), SthreadError> {
        let current = git_head(&self.repo)?;
        if current != self.revision {
            return Err(SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "authority revision changed from {} to {current}",
                    self.revision
                ),
            ));
        }
        ensure_clean_checkout(&self.repo)
    }
}

fn producer_transform_consumer_candidates(thread: &ThreadIr) -> Vec<(String, String, String)> {
    let node_by_id = thread
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let edge_exists = |from: &str, to: &str, kind: &str| {
        thread
            .edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.kind == kind)
    };
    let mut candidates = Vec::new();
    for transformer in thread.nodes.iter().filter(|node| {
        node.kind == "DEFINITION" && node.defines.as_ref().is_some_and(|name| !name.is_empty())
    }) {
        let transformed = transformer.defines.as_deref().unwrap_or_default();
        let immutable_local = transformer
            .origin
            .as_ref()
            .and_then(|origin| origin.get("sourceText"))
            .and_then(Value::as_str)
            .is_some_and(|source| source.trim_start().starts_with("val "));
        let has_transform_call = thread.edges.iter().any(|edge| {
            edge.from == transformer.id
                && edge.kind == "AST_CHILD"
                && node_by_id.get(edge.to.as_str()).is_some_and(|node| {
                    node.kind == "CALL_RESULT"
                        && edge_exists(&node.id, &transformer.id, "CFG_NORMAL")
                })
        });
        if !immutable_local || !has_transform_call {
            continue;
        }
        for producer in thread.nodes.iter().filter(|node| {
            node.kind == "PARAMETER"
                && node
                    .defines
                    .as_ref()
                    .is_some_and(|name| transformer.uses.contains(name))
                && edge_exists(&node.id, &transformer.id, "DEF_USE")
        }) {
            for consumer in thread.nodes.iter().filter(|node| {
                node.kind == "RETURN"
                    && node.uses.iter().any(|name| name == transformed)
                    && edge_exists(&transformer.id, &node.id, "DEF_USE")
                    && thread
                        .edges
                        .iter()
                        .any(|edge| edge.from == node.id && edge.kind == "RETURN")
            }) {
                candidates.push((
                    producer.id.clone(),
                    transformer.id.clone(),
                    consumer.id.clone(),
                ));
            }
        }
    }
    candidates
}

fn compiler_owner_symbol(thread: &ThreadIr, producer_id: &str) -> Result<String, SthreadError> {
    thread
        .nodes
        .iter()
        .find(|node| node.id == producer_id && node.kind == "PARAMETER")
        .and_then(|node| node.attributes.get("ownerCompilerSymbol"))
        .and_then(Value::as_str)
        .filter(|symbol| !symbol.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            invalid_source("bound producer has no compiler-issued owner callable symbol")
        })
}

fn verify_assertion_of_target(
    resolution: &Value,
    target_compiler_symbol: &str,
) -> Result<(), SthreadError> {
    let semantic_facts = resolution
        .get("semanticFacts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("test resolution has no semantic facts"))?;
    let is_test = semantic_facts.iter().any(|fact| {
        fact.get("kind").and_then(Value::as_str) == Some("FirAnnotationCallImpl")
            && fact
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("org/junit/jupiter/api/Test"))
    });
    if !is_test {
        return Err(invalid_source(
            "resolved function has no compiler-confirmed JUnit test annotation",
        ));
    }
    let calls = resolution
        .get("resolvedCalls")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_source("test resolution has no resolved calls"))?;
    let target_calls = calls
        .iter()
        .filter(|call| call.get("symbol").and_then(Value::as_str) == Some(target_compiler_symbol))
        .collect::<Vec<_>>();
    let [target_call] = target_calls.as_slice() else {
        return Err(invalid_source(
            "test must call the exact production callable once",
        ));
    };
    let target_start = target_call
        .get("start")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_source("target call has no compiler source range"))?;
    let assertion = calls.iter().find(|call| {
        call.get("symbol").and_then(Value::as_str) == Some("kotlin/test/assertEquals")
            && call
                .get("argumentToParameter")
                .and_then(Value::as_array)
                .is_some_and(|arguments| {
                    let has_expected = arguments.iter().any(|argument| {
                        argument.get("parameter").and_then(Value::as_str) == Some("expected")
                    });
                    let actual_is_target = arguments.iter().any(|argument| {
                        argument.get("parameter").and_then(Value::as_str) == Some("actual")
                            && argument.get("argumentStart").and_then(Value::as_u64)
                                == Some(target_start)
                    });
                    has_expected && actual_is_target
                })
    });
    if assertion.is_none() {
        return Err(invalid_source(
            "test does not assert the result of the exact production call",
        ));
    }
    Ok(())
}

fn verify_live_sources(
    repo: &Path,
    thread: &ThreadIr,
) -> Result<BTreeMap<PathBuf, String>, SthreadError> {
    let mut files = BTreeMap::new();
    let mut exact_origins = 0usize;
    for node in &thread.nodes {
        let Some(origin) = node.origin.as_ref() else {
            continue;
        };
        let file_id = required_str(origin, "fileId")?;
        let anchor_id = required_str(origin, "anchorId")?;
        let exact_text_hash = required_str(origin, "exactTextHash")?;
        let source_text = required_str(origin, "sourceText")?;
        let range = origin
            .get("rangeHint")
            .and_then(Value::as_array)
            .filter(|range| range.len() == 2)
            .ok_or_else(|| invalid_source("source origin has no exact byte range"))?;
        let start = range[0]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| invalid_source("invalid source range start"))?;
        let end = range[1]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| invalid_source("invalid source range end"))?;
        let relative = safe_relative_path(file_id)?;
        let path = repo
            .join(&relative)
            .canonicalize()
            .map_err(|error| invalid_source(format!("cannot resolve source {file_id}: {error}")))?;
        if !path.starts_with(repo) {
            return Err(invalid_source("source origin escapes authority repository"));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| invalid_source(format!("cannot read source {file_id}: {error}")))?;
        let exact = bytes
            .get(start..end)
            .ok_or_else(|| invalid_source("source origin range is outside the file"))?;
        if exact != source_text.as_bytes()
            || canonical::hash_bytes(exact) != exact_text_hash
            || !thread.read_set.iter().any(|fact| {
                fact.kind == "SOURCE_NODE" && fact.key == anchor_id && fact.hash == exact_text_hash
            })
        {
            return Err(invalid_source(
                "source bytes, anchor hash, and Thread IR ReadSet disagree",
            ));
        }
        files.insert(relative, canonical::hash_bytes(&bytes));
        exact_origins += 1;
    }
    if exact_origins == 0 {
        return Err(invalid_source(
            "worker-rebuilt Thread IR has no exact source origin",
        ));
    }
    Ok(files)
}

fn verify_resolution_source(
    repo: &Path,
    resolution: &Value,
) -> Result<BTreeMap<PathBuf, String>, SthreadError> {
    let anchor = resolution
        .get("bodyAnchor")
        .ok_or_else(|| invalid_source("test resolution has no exact body anchor"))?;
    let file_id = required_str(anchor, "fileId")?;
    let exact_text_hash = required_str(anchor, "exactTextHash")?;
    let source_text = required_str(anchor, "sourceText")?;
    let range = anchor
        .get("rangeHint")
        .and_then(Value::as_array)
        .filter(|range| range.len() == 2)
        .ok_or_else(|| invalid_source("test anchor has no exact byte range"))?;
    let start = range[0]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid_source("invalid test source range start"))?;
    let end = range[1]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid_source("invalid test source range end"))?;
    let relative = safe_relative_path(file_id)?;
    let path = repo
        .join(&relative)
        .canonicalize()
        .map_err(|error| invalid_source(format!("cannot resolve test source: {error}")))?;
    if !path.starts_with(repo) {
        return Err(invalid_source("test source escapes authority repository"));
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| invalid_source(format!("cannot read test source: {error}")))?;
    let exact = bytes
        .get(start..end)
        .ok_or_else(|| invalid_source("test source range is outside the file"))?;
    if exact != source_text.as_bytes() || canonical::hash_bytes(exact) != exact_text_hash {
        return Err(invalid_source(
            "worker test anchor does not match the authority checkout",
        ));
    }
    Ok(BTreeMap::from([(relative, canonical::hash_bytes(&bytes))]))
}

fn verify_sources_current(
    repo: &Path,
    expected: &BTreeMap<PathBuf, String>,
) -> Result<(), SthreadError> {
    for (relative, hash) in expected {
        let current = std::fs::read(repo.join(relative)).map_err(|error| {
            invalid_source(format!("cannot reread {}: {error}", relative.display()))
        })?;
        if canonical::hash_bytes(&current) != *hash {
            return Err(SthreadError::new(
                ErrorCode::StaleRequiresReslice,
                format!(
                    "source {} changed after authority verification",
                    relative.display()
                ),
            ));
        }
    }
    Ok(())
}

fn thread_set_fingerprint(verified: &[&VerifiedThread]) -> Result<String, SthreadError> {
    let mut fingerprints = verified
        .iter()
        .map(|item| item.fingerprint.clone())
        .collect::<Vec<_>>();
    fingerprints.sort();
    if fingerprints.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "duplicate semantic thread evidence is not an independent required root",
        ));
    }
    canonical::hash(&fingerprints).map_err(internal)
}

fn test_set_fingerprint(verified: &[&VerifiedBehavioralTest]) -> Result<String, SthreadError> {
    let mut fingerprints = verified
        .iter()
        .map(|item| item.fingerprint.clone())
        .collect::<Vec<_>>();
    fingerprints.sort();
    if fingerprints.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "duplicate behavioral-test evidence is not an independent oracle",
        ));
    }
    canonical::hash(&fingerprints).map_err(internal)
}

fn test_artifact(
    repo: &Path,
    build_system: BuildSystem,
    expected: &[&VerifiedBehavioralTest],
) -> Result<(String, usize), SthreadError> {
    let result_root = match build_system {
        BuildSystem::Gradle => repo.join("build/test-results"),
        BuildSystem::Maven => repo.join("target/surefire-reports"),
    };
    let mut reports = Vec::new();
    let mut executed = 0usize;
    let mut matched = BTreeSet::new();
    for entry in WalkDir::new(&result_root).follow_links(false) {
        let entry = entry.map_err(|error| {
            SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot inspect validation reports: {error}"),
            )
        })?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("xml")
        {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|error| {
            SthreadError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("cannot read validation report: {error}"),
            )
        })?;
        let text = String::from_utf8_lossy(&bytes);
        let testcases = testcase_tags(&text);
        executed += testcases.len();
        for (index, test) in expected.iter().enumerate() {
            let class = format!("classname=\"{}\"", xml_escape(&test.class_name));
            let plain_name = format!("name=\"{}\"", xml_escape(&test.test_name));
            let kotlin_name = format!("name=\"{}()\"", xml_escape(&test.test_name));
            if testcases.iter().any(|tag| {
                tag.contains(&class) && (tag.contains(&plain_name) || tag.contains(&kotlin_name))
            }) {
                matched.insert(index);
            }
        }
        reports.push((
            entry
                .path()
                .strip_prefix(repo)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/"),
            canonical::hash_bytes(&bytes),
        ));
    }
    reports.sort();
    if executed == 0 || matched.len() != expected.len() {
        return Err(SthreadError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "validation lifecycle did not execute every compiler-linked behavioral test",
        ));
    }
    Ok((canonical::hash(&reports).map_err(internal)?, executed))
}

fn testcase_tags(text: &str) -> Vec<&str> {
    let mut tags = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = text[cursor..].find('<') {
        let start = cursor + relative;
        if text[start..].starts_with("<![CDATA[") {
            let Some(end) = text[start + 9..].find("]]>") else {
                break;
            };
            cursor = start + 9 + end + 3;
            continue;
        }
        let Some(relative_end) = text[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &text[start..end];
        if tag.starts_with("<testcase")
            && tag
                .as_bytes()
                .get("<testcase".len())
                .is_some_and(u8::is_ascii_whitespace)
        {
            tags.push(tag);
        }
        cursor = end;
    }
    tags
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn safe_relative_path(value: &str) -> Result<PathBuf, SthreadError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid_source("source path is not a safe relative path"));
    }
    Ok(path.to_owned())
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, SthreadError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_source(format!("source origin has no {field}")))
}

fn git_head(repo: &Path) -> Result<String, SthreadError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|error| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                format!("cannot start git for evidence authority: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "evidence repository has no readable Git HEAD",
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(internal)
}

fn ensure_clean_checkout(repo: &Path) -> Result<(), SthreadError> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all", "--", "."])
        .current_dir(repo)
        .output()
        .map_err(|error| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                format!("cannot inspect authority checkout: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "cannot inspect authority checkout state",
        ));
    }
    if output.stdout.is_empty() {
        Ok(())
    } else {
        Err(SthreadError::new(
            ErrorCode::StaleRequiresReslice,
            "authority requires a clean repository subtree bound to Git HEAD",
        ))
    }
}

fn wrong_session(kind: &str) -> SthreadError {
    SthreadError::new(
        ErrorCode::PreconditionFailed,
        format!("{kind} receipt was issued by another authority session"),
    )
}

fn invalid_receipt(kind: &str) -> SthreadError {
    SthreadError::new(
        ErrorCode::PreconditionFailed,
        format!("unknown {kind} receipt"),
    )
}

fn invalid_source(message: impl Into<String>) -> SthreadError {
    SthreadError::new(ErrorCode::IncompleteSemanticAnalysis, message)
}

fn internal(error: impl std::fmt::Display) -> SthreadError {
    SthreadError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_change_and_path_escape_invalidate_authority_inputs() {
        let temporary = tempfile::tempdir().unwrap();
        let file = PathBuf::from("Source.kt");
        std::fs::write(temporary.path().join(&file), "fun stable() = 1\n").unwrap();
        let expected =
            BTreeMap::from([(file.clone(), canonical::hash_bytes(b"fun stable() = 1\n"))]);
        verify_sources_current(temporary.path(), &expected).unwrap();

        std::fs::write(temporary.path().join(&file), "fun stable() = 2\n").unwrap();
        let error = verify_sources_current(temporary.path(), &expected).unwrap_err();
        assert_eq!(error.code, ErrorCode::StaleRequiresReslice);
        assert!(safe_relative_path("../outside.kt").is_err());
        assert!(safe_relative_path("/absolute.kt").is_err());
    }

    #[test]
    fn testcase_parser_ignores_forged_cdata_output() {
        let report = r#"<testsuite>
          <testcase name="real()" classname="Example"/>
          <system-out><![CDATA[<testcase name="forged()" classname="Expected"/>]]></system-out>
        </testsuite>"#;
        let tags = testcase_tags(report);
        assert_eq!(tags.len(), 1);
        assert!(tags[0].contains("name=\"real()\""));
    }
}
