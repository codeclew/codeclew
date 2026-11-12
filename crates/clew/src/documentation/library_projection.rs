//! Reached-only in-process library continuations over separately retained evidence.
use super::{
    cache, digest, invalid,
    library_bindings::{self, Binding},
    model::*,
    store::Repository,
};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const SCOPE: &str = "LIBRARY_SOURCE_SCOPE";
const LINK: &str = "LIBRARY_CONTINUATION";
const BOUNDARY: &str = "LIBRARY_ASSOCIATION_BOUNDARY";
fn scope(o: &Observation) -> &str {
    o.normalized["scope"].as_str().unwrap_or("")
}
fn alias(b: &Binding, original: &str) -> Result<String, ClewError> {
    Ok(format!(
        "{}:library:{}:{}",
        b.input.consumer,
        &digest(b)?[7..23],
        original
    ))
}
fn observation(
    a: &mut ServiceEvidence,
    kind: &str,
    key: &str,
    normalized: Value,
    sources: Vec<String>,
) -> Result<(), ClewError> {
    let id = format!(
        "{}:{}:{}",
        a.service,
        kind.to_ascii_lowercase(),
        &digest(&key)?[7..31]
    );
    a.observations.insert(
        id.clone(),
        Observation {
            id,
            kind: kind.into(),
            service: a.service.clone(),
            symbol: key.into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids: sources,
        },
    );
    Ok(())
}
/// Remove only derived library records; original native evidence and boundaries
/// remain available on stale bindings, detach and recomposition.
pub(super) fn strip(a: &mut ServiceEvidence) {
    a.observations.retain(|_, o| {
        !matches!(o.kind.as_str(), SCOPE | LINK | BOUNDARY)
            && o.normalized.get("libraryOrigin").is_none()
    });
    a.sources.retain(|_, s| s.library_origin.is_none());
}
fn rewrite(value: &mut Value, ids: &BTreeMap<String, String>) {
    // Only engine-created reference positions may change. Business strings,
    // names, topics and source expressions must survive namespace projection.
    for field in ["ownerDeclarationId", "declarationId"] {
        if let Some(id) = value[field].as_str().and_then(|id| ids.get(id)).cloned() {
            value[field] = json!(id);
        }
    }
    for field in ["callSite", "outlineSite", "outlineOwnerSource"] {
        if let Some(id) = value[field]["sourceId"]
            .as_str()
            .and_then(|id| ids.get(id))
            .cloned()
        {
            value[field]["sourceId"] = json!(id);
        }
    }
}

fn compiler_candidate<'a>(
    a: &ServiceEvidence,
    b: &'a ServiceEvidence,
    binding: &Binding,
    flow: &Observation,
) -> Option<&'a Observation> {
    if flow.normalized["resolution"] != "COMPILER_EXACT"
        || scope(flow) != binding.input.consumer_compilation
    {
        return None;
    }
    let target = flow.normalized["target"].as_str()?;
    let owners = a
        .observations
        .values()
        .filter(|o| o.kind == "DEPENDENCY_TARGET" && scope(o) == scope(flow) && o.symbol == target)
        .collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return None;
    };
    if owner.normalized["status"] != "UNIQUE_CLASSPATH_OWNER"
        || owner.normalized["owners"][0]["artifact"] != binding.input.artifact
    {
        return None;
    }
    let candidates = b
        .observations
        .values()
        .filter(|o| {
            super::process_graph::callable_observation(o)
                && scope(o) == binding.input.library.compilation
                && o.symbol == target
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [o] => Some(*o),
        _ => None,
    }
}
fn source_candidate<'a>(
    a: &ServiceEvidence,
    b: &'a ServiceEvidence,
    binding: &Binding,
    flow: &Observation,
) -> Option<&'a Observation> {
    if flow.normalized["authority"] != "SYNTAX" || !binding.input.consumer_compilation.is_empty() {
        return None;
    }
    let call = &flow.normalized["libraryCallCandidate"];
    if call["shadowedReceiver"] != false {
        return None;
    }
    let receiver = call["receiver"].as_str()?;
    let name = call["name"].as_str()?;
    let file = flow
        .source_ids
        .iter()
        .filter_map(|id| a.sources.get(id))
        .next()?
        .file
        .as_str();
    let imports = a
        .observations
        .values()
        .filter(|o| {
            o.kind == "SYNTAX_DETAIL"
                && matches!(
                    o.normalized["syntaxKind"].as_str(),
                    Some("import" | "import_header")
                )
                && o.source_ids
                    .iter()
                    .filter_map(|id| a.sources.get(id))
                    .any(|s| s.file == file)
        })
        .filter_map(|o| o.normalized["text"].as_str())
        .filter_map(|s| s.strip_prefix("import "))
        .map(|s| s.trim().trim_end_matches(';'))
        .collect::<Vec<_>>();
    let mut owner = receiver.to_owned();
    if !receiver.contains('.') {
        let imports = imports
            .iter()
            .filter(|s| s.rsplit('.').next() == Some(receiver))
            .collect::<Vec<_>>();
        let [import] = imports.as_slice() else {
            return None;
        };
        owner = (**import).into();
    }
    if owner.contains(['*', ' ', '\t', '\n']) {
        return None;
    }
    // No argument-type inference: even compatible arity cannot select overloads.
    let candidates = b
        .observations
        .values()
        .filter(|o| {
            super::process_graph::callable_observation(o)
                && scope(o) == binding.input.library.compilation
                && o.normalized["name"] == name
                && o.normalized["ownerIdentity"].as_str().is_some_and(|s| {
                    s.strip_prefix("class:")
                        .is_some_and(|s| s.replace('/', ".") == owner)
                })
        })
        .collect::<Vec<_>>();
    let [candidate] = candidates.as_slice() else {
        return None;
    };
    if candidate.normalized["parameterTypes"]
        .as_array()
        .map(Vec::len)
        .map(|n| n as u64)
        != call["argumentCount"].as_u64()
    {
        return None;
    }
    Some(*candidate)
}
fn project(
    b: &ServiceEvidence,
    binding: &Binding,
    root: &Observation,
    a: &mut ServiceEvidence,
) -> Result<String, ClewError> {
    let graph = super::process_graph::collect_bounded(
        b,
        root,
        "",
        "",
        Some(&super::process_extract::Budgets::default()),
    )?;
    if graph["frontiers"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|r| r["kind"] == "BUDGET"))
    {
        return Err(invalid(
            "LIBRARY_REACHABILITY_BUDGET: narrow the B source capture; no partial library projection was admitted",
        ));
    }
    let methods = graph["methods"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .collect::<BTreeSet<_>>();
    let symbols = methods
        .iter()
        .filter_map(|id| b.observations.get(*id))
        .map(|o| o.symbol.as_str())
        .collect::<BTreeSet<_>>();
    let selected = b
        .observations
        .values()
        .filter(|o| {
            scope(o) == binding.input.library.compilation
                && (methods.contains(o.id.as_str())
                    || symbols.contains(o.symbol.as_str())
                        && matches!(
                            o.kind.as_str(),
                            "FLOW" | "CALL_RELATION" | "PROVIDER_BOUNDARY"
                        ))
        })
        .collect::<Vec<_>>();
    let sources = selected
        .iter()
        .flat_map(|o| o.source_ids.iter())
        .collect::<BTreeSet<_>>();
    let mut ids = BTreeMap::new();
    for o in &selected {
        ids.insert(o.id.clone(), alias(binding, &o.id)?);
    }
    for id in &sources {
        ids.insert((*id).clone(), alias(binding, id)?);
    }
    for id in sources {
        let mut s = b
            .sources
            .get(id)
            .ok_or_else(|| invalid("library citation missing"))?
            .clone();
        if s.library_origin.is_some() {
            return Err(invalid("nested library captures are unsupported"));
        }
        s.library_origin = Some(LibrarySourceOrigin {
            binding: binding.input.id.clone(),
            original_source_id: s.id.clone(),
        });
        s.id = ids[id].clone();
        s.service = a.service.clone();
        a.sources.insert(s.id.clone(), s);
    }
    for o in selected {
        let mut o = o.clone();
        let original = o.id.clone();
        rewrite(&mut o.normalized, &ids);
        o.normalized["libraryOrigin"] = json!({"binding":binding.input.id,"role":"LIBRARY_SOURCE","repositoryId":binding.service.repository_id,"repository":binding.service.repository,"revision":b.revision,"compilerScope":binding.input.library.compilation,"declarationOrEvent":original,"sourceAuthority":"RETAINED_LIBRARY_COMPILER_EVIDENCE_NOT_REVERIFIED"});
        o.normalized["scope"] = json!(format!(
            "library:{}:{}",
            binding.input.id, binding.input.library.compilation
        ));
        o.id = ids[&o.id].clone();
        o.service = a.service.clone();
        o.source_ids = o.source_ids.iter().map(|id| ids[id].clone()).collect();
        o.digest = digest(&o.normalized)?;
        a.observations.insert(o.id.clone(), o);
    }
    alias(binding, &root.id)
}
pub(super) fn apply(
    repo: &Repository,
    bindings: &BTreeMap<String, Binding>,
    services: &mut BTreeMap<String, ServiceEvidence>,
) -> Result<(), ClewError> {
    for a in services.values_mut() {
        strip(a)
    }
    for binding in bindings.values() {
        let Some(a) = services.get_mut(&binding.input.consumer) else {
            continue;
        };
        let b = cache::load_capture(repo, &binding.capture).and_then(|b| {
            library_bindings::admit(&binding.input, a, &b)?;
            Ok(b)
        });
        let b = match b {
            Ok(b) => b,
            Err(e) => {
                observation(
                    a,
                    BOUNDARY,
                    &binding.input.id,
                    json!({"binding":binding.input.id,"status":"NOT_LINKED","reason":e.message,"nextAction":"Select the matching A artifact and original B compiler capture, replace this association and recompose; A evidence remains readable."}),
                    vec![],
                )?;
                continue;
            }
        };
        observation(
            a,
            SCOPE,
            &binding.input.id,
            json!({"binding":binding,"role":"LIBRARY_SOURCE","sourceToBinaryEquivalence":"NOT_ESTABLISHED","sourceAuthority":"RETAINED_LIBRARY_SOURCE_NOT_REVERIFIED","providerBoundaries":b.boundaries}),
            vec![],
        )?;
        let flows = a
            .observations
            .values()
            .filter(|o| {
                o.kind == "FLOW"
                    && o.normalized["kind"] == "CALL"
                    && o.normalized.get("libraryOrigin").is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        for flow in flows {
            let syntax = a.extractor == SOURCE_EXTRACTOR;
            let candidate = if syntax {
                source_candidate(a, &b, binding, &flow)
            } else {
                compiler_candidate(a, &b, binding, &flow)
            };
            let Some(root) = candidate else { continue };
            let target = match project(&b, binding, root, a) {
                Ok(id) => id,
                Err(e) => {
                    observation(
                        a,
                        BOUNDARY,
                        &format!("{}:{}", binding.input.id, flow.id),
                        json!({"binding":binding.input.id,"eventId":flow.id,"status":"NOT_LINKED","reason":e.message}),
                        flow.source_ids.clone(),
                    )?;
                    continue;
                }
            };
            observation(
                a,
                LINK,
                &format!("{}:{}", binding.input.id, flow.id),
                json!({"binding":binding.input.id,"eventId":flow.id,"targetMethodId":target,"targetIdentity":root.symbol,"status":"CANDIDATE_CONTINUATION","callAuthority":if syntax{"SOURCE_SYNTAX"}else{"COMPILER_EXACT_BINARY_TARGET"},"artifactAuthority":if syntax{"DECLARED_DEPENDENCY_NOT_RESOLVED"}else{"GRADLE_RESOLVED_COMPONENT_ARTIFACT"},"relationshipAuthority":"DECLARED_SOURCE_BINDING","libraryAuthority":"RETAINED_COMPILER_DECLARATION_NOT_RUNTIME_DISPATCH","sourceToBinaryEquivalence":"NOT_ESTABLISHED","execution":"IN_PROCESS_LIBRARY_NOT_NETWORK_HOP"}),
                flow.source_ids.clone(),
            )?;
        }
    }
    Ok(())
}
pub(super) fn link<'a>(e: &'a ServiceEvidence, event: &str) -> Option<&'a Observation> {
    let rows = e
        .observations
        .values()
        .filter(|o| o.kind == LINK && o.normalized["eventId"] == event)
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [o] => Some(*o),
        _ => None,
    }
}
pub(super) fn verify_source(e: &ServiceEvidence, s: &Source) -> Result<bool, ClewError> {
    let Some(origin) = &s.library_origin else {
        return Ok(false);
    };
    let scopes = e
        .observations
        .values()
        .filter(|o| o.kind == SCOPE && o.normalized["binding"]["input"]["id"] == origin.binding)
        .collect::<Vec<_>>();
    let [scope] = scopes.as_slice() else {
        return Err(invalid("library source has no unique retained binding"));
    };
    let binding: Binding = serde_json::from_value(scope.normalized["binding"].clone())
        .map_err(|_| invalid("invalid library source binding"))?;
    library_bindings::validate(&binding)?;
    if s.revision != binding.input.library.revision
        || s.id != alias(&binding, &origin.original_source_id)?
        || s.service != e.service
    {
        return Err(invalid(
            "library source provenance differs from its immutable binding",
        ));
    }
    Ok(true)
}

/// Resolve provenance through one compact citation pointer; do not repeat the
/// artifact/JVM/Gradle manifest beside every retained source snippet.
pub(super) fn source_service(
    e: &ServiceEvidence,
    s: &Source,
) -> Result<Option<Service>, ClewError> {
    let Some(origin) = &s.library_origin else {
        return Ok(None);
    };
    verify_source(e, s)?;
    let scope = e
        .observations
        .values()
        .find(|o| o.kind == SCOPE && o.normalized["binding"]["input"]["id"] == origin.binding)
        .unwrap();
    let binding: Binding = serde_json::from_value(scope.normalized["binding"].clone())
        .map_err(|_| invalid("invalid library source binding"))?;
    Ok(Some(binding.service))
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::library_bindings::{Input, LibrarySelection};
    use super::*;
    fn add(
        e: &mut ServiceEvidence,
        id: &str,
        kind: &str,
        symbol: &str,
        normalized: Value,
        sources: Vec<String>,
    ) {
        e.observations.insert(
            id.into(),
            Observation {
                id: id.into(),
                kind: kind.into(),
                symbol: symbol.into(),
                service: e.service.clone(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: sources,
            },
        );
    }
    fn method(e: &mut ServiceEvidence, id: &str, symbol: &str, name: &str, events: Vec<Value>) {
        let sid = format!("source-{id}");
        let text = format!("fun {name}() {{ /* owned unit fixture */ }}");
        e.sources.insert(
            sid.clone(),
            Source {
                id: sid.clone(),
                service: e.service.clone(),
                revision: e.revision.clone(),
                file: format!("{name}.kt"),
                start_line: 1,
                end_line: 1,
                text_digest: crate::canonical::hash_bytes(text.as_bytes()),
                text,
                evidence_digest: format!("sha256:{}", "a".repeat(64)),
                authority: "EXACT_SNAPSHOT_TEXT".into(),
                text_origin: None,
                library_origin: None,
                occurrence: None,
                url: None,
            },
        );
        add(
            e,
            id,
            "SYMBOL",
            symbol,
            json!({"declarationKind":"FUNCTION","name":name,"scope":":/main","ownerIdentity":"class:owned/library/Policy","parameterTypes":["kotlin/Int"],"documentation":{"bodyStatus":"RETAINED_BODY","parameterTypes":["kotlin/Int"],"events":events,"boundaries":[]}}),
            vec![sid.clone()],
        );
        for (i, mut event) in events.into_iter().enumerate() {
            event["scope"] = json!(":/main");
            event["ordinal"] = json!(i);
            add(
                e,
                &format!("{id}-event-{i}"),
                "FLOW",
                symbol,
                event,
                vec![sid.clone()],
            );
        }
    }
    pub(crate) fn fixture() -> (
        tempfile::TempDir,
        Repository,
        Binding,
        ServiceEvidence,
        ServiceEvidence,
    ) {
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Owned library projection unit").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let service:Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"library","title":"Library","repositoryId":"library","repository":"https://example.invalid/owned/library.git","language":"kotlin","profile":"kotlin-jvm-gradle-analysis","compilations":[":/main"],"targetRef":"main"})).unwrap();
        let mut b = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "library".into(),
            revision: "b".repeat(40),
            service_digest: digest(&service).unwrap(),
            extractor: EXTRACTOR.into(),
            runtime_mode: "RELEASE".into(),
            coverage: "PARTIAL".into(),
            boundaries: vec!["ORIGINAL_B_PROVIDER_LIMIT".into()],
            entrypoints: vec![],
            sources: BTreeMap::new(),
            observations: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let decide = "callable:owned/library/Policy.decide#jvm:(I)Ljava/lang/String;";
        let normalize = "callable:owned/library/Policy.normalize#jvm:(I)Ljava/lang/String;";
        method(
            &mut b,
            "decide",
            decide,
            "decide",
            vec![
                json!({"kind":"IF","detail":"negative input"}),
                json!({"kind":"RETURN","detail":"negative-v1"}),
                json!({"kind":"CALL","target":normalize,"resolution":"COMPILER_EXACT"}),
            ],
        );
        method(
            &mut b,
            "normalize",
            normalize,
            "normalize",
            vec![
                json!({"kind":"CALL","target":"callable:org/springframework/kafka/core/KafkaTemplate.send#jvm:(Ljava/lang/String;Ljava/lang/Object;)Ljava/util/concurrent/CompletableFuture;","resolution":"COMPILER_EXACT","kafka":{"topic":"owned.reached"}}),
                json!({"kind":"RETURN","detail":"accepted-v1"}),
            ],
        );
        method(
            &mut b,
            "unreachable",
            "callable:owned/library/Policy.unreachable#jvm:(I)V",
            "unreachable",
            vec![json!({"kind":"CALL","kafka":{"topic":"owned.unreachable"}})],
        );
        // A real call-relation-shaped record exercises the independent B
        // revision check used by Work, beyond the simpler FLOW-only traversal.
        let mut call_source = b.sources["source-decide"].clone();
        call_source.id = "relation-source".into();
        call_source.text = "normalize(amount)".into();
        call_source.text_digest = crate::canonical::hash_bytes(call_source.text.as_bytes());
        let relation = json!({"schema":"codeclew-kotlin-documentation-call/1.0","kind":"RELATION","relationKind":"CALLS","scope":":/main",
            "resolution":"COMPILER_EXACT","compilerResolution":"PROVEN","provider":"K2_FIR","compilerSchema":"declaration-relation/0.1","sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "sourceIdentity":decide,"targetIdentity":normalize,"sourceCompilerCallableId":"owned/library/Policy.decide","sourceJvmDescriptor":"(I)Ljava/lang/String;","targetCompilerCallableId":"owned/library/Policy.normalize","targetJvmDescriptor":"(I)Ljava/lang/String;",
            "evidenceBinding":call_source.evidence_digest,"callSite":{"sourceId":call_source.id,"sourceStatus":"SOURCE_RETAINED","file":call_source.file,"startLine":1,"endLine":1,"byteStart":0,"byteEnd":call_source.text.len(),"sourceDigest":call_source.text_digest,"evidenceDigest":call_source.evidence_digest}});
        add(
            &mut b,
            "decide-relation",
            "CALL_RELATION",
            decide,
            relation,
            vec![call_source.id.clone()],
        );
        b.sources.insert(call_source.id.clone(), call_source);
        add(
            &mut b,
            "publication",
            "LIBRARY_PUBLICATION",
            "publication",
            json!({"group":"owned","name":"library","version":"1.0.0","authority":"GRADLE_PUBLICATION_DECLARATION","scope":":/main"}),
            vec![],
        );
        let mut a = b.clone();
        a.service = "application".into();
        a.revision = "a".repeat(40);
        a.observations.clear();
        a.sources.clear();
        a.boundaries = vec!["ORIGINAL_A_PROVIDER_LIMIT".into()];
        method(
            &mut a,
            "handle",
            "callable:owned/application/Service.handle#jvm:(I)Ljava/lang/String;",
            "handle",
            vec![json!({"kind":"CALL","target":decide,"resolution":"COMPILER_EXACT"})],
        );
        let hash = format!("sha256:{}", "c".repeat(64));
        let artifact = json!({"id":format!("classpath:0:{hash}"),"classpathIndex":0,"fileName":"library.jar","contentDigest":hash,"size":100,"coordinates":{"group":"owned","name":"library","version":"1.0.0","extension":"jar","classifier":null,"scope":"compileClasspath"},"variant":{},"authority":"GRADLE_RESOLVED_COMPONENT_ARTIFACT","classpathAuthority":"KOTLIN_TASK_LIBRARIES"});
        let mut row = artifact.clone();
        row["scope"] = json!(":/main");
        row["evidenceBinding"] = json!(hash);
        add(
            &mut a,
            "artifact",
            "DEPENDENCY_ARTIFACT",
            "artifact",
            row,
            vec![],
        );
        add(
            &mut a,
            "owner",
            "DEPENDENCY_TARGET",
            decide,
            json!({"scope":":/main","status":"UNIQUE_CLASSPATH_OWNER","owners":[{"artifact":artifact}]}),
            vec![],
        );
        let input = Input {
            schema: "codeclew-documentation-library-binding-input/1.0".into(),
            id: "policy".into(),
            consumer: a.service.clone(),
            consumer_revision: None,
            consumer_compilation: ":/main".into(),
            artifact,
            library: LibrarySelection {
                service: service.id.clone(),
                repository_id: service.repository_id.clone(),
                repository: service.repository.clone(),
                revision: b.revision.clone(),
                compilation: ":/main".into(),
            },
            authority: "DECLARED_SOURCE_BINDING".into(),
            rationale: "Owned unit source association, no binary equivalence claim".into(),
        };
        let binding = Binding {
            schema: "codeclew-documentation-library-binding/1.0".into(),
            input,
            origin_snapshot_digest: format!("sha256:{}", "d".repeat(64)),
            origin_snapshot_bytes: 100,
            service,
            capture: cache::store_capture(&repo, &b).unwrap(),
        };
        (temp, repo, binding, a, b)
    }
    #[test]
    fn namespace_rewrites_reference_positions_without_changing_names_or_literals() {
        let ids = BTreeMap::from([
            ("decide".into(), "projected-declaration".into()),
            ("source-decide".into(), "projected-source".into()),
        ]);
        let mut value = json!({"name":"decide","ownerDeclarationId":"decide","kafka":{"topic":"source-decide"},"text":"decide","callSite":{"sourceId":"source-decide"},"outlineOwnerSource":{"sourceId":"source-decide"},"documentation":{"events":[{"text":"source-decide"}]}});
        rewrite(&mut value, &ids);
        assert_eq!(value["ownerDeclarationId"], "projected-declaration");
        assert_eq!(value["callSite"]["sourceId"], "projected-source");
        assert_eq!(value["outlineOwnerSource"]["sourceId"], "projected-source");
        assert_eq!(value["name"], "decide");
        assert_eq!(value["text"], "decide");
        assert_eq!(value["kafka"]["topic"], "source-decide");
        assert_eq!(value["documentation"]["events"][0]["text"], "source-decide");
    }

    #[test]
    fn compiler_thread_projects_reached_library_bodies_and_keeps_binary_binding_candidate() {
        let (_temp, repo, binding, a, _b) = fixture();
        let mut services = BTreeMap::from([(a.service.clone(), a)]);
        apply(
            &repo,
            &BTreeMap::from([(binding.input.id.clone(), binding.clone())]),
            &mut services,
        )
        .unwrap();
        let a = &services["application"];
        super::super::analysis::verify_evidence(a).unwrap();
        let graph = super::super::process_graph::collect_bounded(
            a,
            &a.observations["handle"],
            "snapshot",
            "input",
            None,
        )
        .unwrap();
        assert_eq!(graph["methods"].as_array().unwrap().len(), 3);
        let text = serde_json::to_string(&graph).unwrap();
        assert!(text.contains("owned.reached"));
        assert!(!text.contains("owned.unreachable"));
        let edge = graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["fromMethodId"] == "handle")
            .unwrap();
        assert_eq!(edge["resolution"], "LIBRARY_SOURCE_CANDIDATE");
        assert_eq!(edge["relationshipAuthority"], "DECLARED_SOURCE_BINDING");
        assert_eq!(
            edge["libraryContinuation"]["callAuthority"],
            "COMPILER_EXACT_BINARY_TARGET"
        );
        assert!(
            a.sources
                .values()
                .filter(|s| s.library_origin.is_some())
                .all(|s| s.revision == "b".repeat(40))
        );
        assert_eq!(a.boundaries, ["ORIGINAL_A_PROVIDER_LIMIT"]);
    }
    #[test]
    fn stale_artifact_and_detach_remove_current_projection_without_changing_history() {
        let (_temp, repo, binding, a, _b) = fixture();
        let mut services = BTreeMap::from([(a.service.clone(), a)]);
        let bindings = BTreeMap::from([(binding.input.id.clone(), binding)]);
        apply(&repo, &bindings, &mut services).unwrap();
        let historical = services.clone();
        let a = services.get_mut("application").unwrap();
        let row = a.observations.get_mut("artifact").unwrap();
        row.normalized["contentDigest"] = json!(format!("sha256:{}", "f".repeat(64)));
        row.digest = digest(&row.normalized).unwrap();
        apply(&repo, &bindings, &mut services).unwrap();
        let a = &services["application"];
        assert!(a.observations.values().any(|o| o.kind == BOUNDARY));
        assert!(a.observations.values().all(|o| o.kind != LINK));
        assert!(a.sources.values().all(|s| s.library_origin.is_none()));
        assert!(a.observations.contains_key("handle"));
        let mut current = historical.clone();
        apply(&repo, &BTreeMap::new(), &mut current).unwrap();
        assert!(
            current["application"]
                .observations
                .values()
                .all(|o| !matches!(o.kind.as_str(), SCOPE | LINK | BOUNDARY))
        );
        assert!(
            historical["application"]
                .observations
                .values()
                .any(|o| o.kind == LINK)
        );
    }
    #[test]
    fn syntax_to_compiler_context_keeps_each_authority_and_refuses_overload_or_shadow() {
        let (_temp, repo, mut binding, mut a, mut b) = fixture();
        a.extractor = SOURCE_EXTRACTOR.into();
        a.coverage = "SYNTAX".into();
        a.runtime_mode = "COMMITTED_SOURCE_NO_BUILD".into();
        binding.input.consumer_compilation.clear();
        binding.input.consumer_revision = Some(a.revision.clone());
        for o in a
            .observations
            .values_mut()
            .filter(|o| matches!(o.kind.as_str(), "SYMBOL" | "FLOW"))
        {
            o.normalized.as_object_mut().unwrap().remove("scope");
            o.normalized["authority"] = json!("SYNTAX");
            if o.kind == "FLOW" {
                o.normalized.as_object_mut().unwrap().remove("target");
                o.normalized["libraryCallCandidate"] = json!({"receiver":"Policy","name":"decide","argumentCount":1,"shadowedReceiver":false});
            }
            o.digest = digest(&o.normalized).unwrap();
        }
        add(
            &mut a,
            "import",
            "SYNTAX_DETAIL",
            "import",
            json!({"syntaxKind":"import","text":"import owned.library.Policy"}),
            vec!["source-handle".into()],
        );
        let mut services = BTreeMap::from([(a.service.clone(), a.clone())]);
        apply(
            &repo,
            &BTreeMap::from([(binding.input.id.clone(), binding.clone())]),
            &mut services,
        )
        .unwrap();
        let joined = &services["application"];
        let rows = super::super::thread_context::context(
            joined,
            "handle",
            "snapshot",
            "input",
            &Default::default(),
        )
        .unwrap();
        assert!(rows.iter().any(|r| r["obligationKind"] == "BODY"
            && r["record"]["declaration"] == "handle"
            && r["record"]["authority"] == "SOURCE_BODY_NOT_CALL_RESOLUTION"));
        assert!(rows.iter().any(|r| r["obligationKind"] == "BODY"
            && r["record"]["declaration"] != "handle"
            && r["record"]["authority"] == "RETAINED_COMPILER_DECLARATION_NOT_RUNTIME_DISPATCH"));
        let link = link(joined, "handle-event-0").unwrap();
        assert_eq!(
            link.normalized["artifactAuthority"],
            "DECLARED_DEPENDENCY_NOT_RESOLVED"
        );
        let mut shadow = a.clone();
        shadow
            .observations
            .get_mut("handle-event-0")
            .unwrap()
            .normalized["libraryCallCandidate"]["shadowedReceiver"] = json!(true);
        assert!(
            source_candidate(
                &shadow,
                &b,
                &binding,
                &shadow.observations["handle-event-0"]
            )
            .is_none()
        );
        method(
            &mut b,
            "decide-overload",
            "callable:owned/library/Policy.decide#jvm:(Ljava/lang/String;)Ljava/lang/String;",
            "decide",
            vec![],
        );
        assert!(source_candidate(&a, &b, &binding, &a.observations["handle-event-0"]).is_none());
    }
}
