//! Complete advisory coverage diagnostics using the proposal's real resolver.
//! This never delivers evidence, generates steps, or relaxes renderer validation.
use super::*;

pub(super) fn already_reported(error: &ClewError, diagnostics: &[Value]) -> bool {
    let ids = &error.relevant_anchors_or_symbols;
    render::is_sequence_coverage_error(error)
        && ids.len() >= 3
        && diagnostics.iter().any(|row| {
            row["diagnosticCode"] == "MANDATORY_FLOW_UNCOVERED"
                && row["flowId"] == ids[1]
                && row["operationId"] == ids[2]
        })
}

pub(super) fn for_input(
    work: &Work,
    input: &Proposal,
    state: &work::ReadState,
) -> Result<Vec<Value>, ClewError> {
    let builder = Builder {
        work,
        received: received_references(work, state)?,
        claims: BTreeMap::new(),
        diagnostics: Vec::new(),
        events: 0,
    };
    collect(&builder, input)
}

pub(super) fn collect(builder: &Builder<'_>, input: &Proposal) -> Result<Vec<Value>, ClewError> {
    if input.operations.len() > 100 {
        return Ok(Vec::new());
    }
    let work = builder.work;
    let mut missing = BTreeMap::new();
    // Each distinct claim evidence list is resolved once, including SOURCE
    // equivalents. No invented dependency mapping or undelivered citation.
    let mut cache: BTreeMap<Vec<String>, Option<BTreeSet<String>>> = BTreeMap::new();
    let mut resolve = |claim: &Claim| {
        cache
            .entry(claim.evidence.clone())
            .or_insert_with(|| {
                builder
                    .evidence(&claim.evidence)
                    .ok()
                    .map(|(deps, _)| deps.into_iter().collect())
            })
            .clone()
            .unwrap_or_default()
    };
    for proposed in &input.operations {
        let Some(operation_id) = operation_target_id(
            work,
            &proposed.entrypoint,
            work.handles.get(&proposed.entrypoint),
        ) else {
            continue;
        };
        let required =
            render::required_sequence_flows(&work.checked, &work.subject, &operation_id)?;
        if required.is_empty() {
            continue;
        }
        let summary = resolve(&proposed.summary);
        let explanation: BTreeSet<_> = proposed.explanation.iter().flat_map(&mut resolve).collect();
        let mut steps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut unread = BTreeSet::new();
        let mut pending: Vec<_> = proposed.steps.iter().map(|step| (step, 0)).collect();
        let mut count = 0;
        while let Some((step, depth)) = pending.pop() {
            count += 1;
            if count > 256 || depth > 16 {
                // The materializer rejects this input too. Do not claim that
                // a partial traversal establishes complete coverage diagnostics.
                return Ok(vec![
                    json!({"code":"MANDATORY_FLOW_ANALYSIS_LIMIT","complete":false,
                    "nextAction":"Proposal exceeds the existing limit of 256 steps and nesting depth 16; reduce its invalid structure before coverage analysis."}),
                ]);
            }
            for id in resolve(&step.meaning) {
                steps.entry(id).or_default().insert(step.kind.clone());
            }
            for reference in &step.meaning.evidence {
                if !builder.received.contains(reference)
                    && let Some(handle) = work.handles.get(reference)
                    && handle.kind == "DEPENDENCY"
                {
                    unread.insert(handle.id.clone());
                }
            }
            pending.extend(
                step.children
                    .iter()
                    .chain(&step.otherwise)
                    .map(|child| (child, depth + 1)),
            );
        }
        for flow in required {
            let kind = flow.normalized["kind"].as_str().unwrap_or("");
            let allowed = render::sequence_event_kinds(kind);
            if steps
                .get(&flow.id)
                .is_some_and(|kinds| kinds.iter().any(|kind| allowed.contains(&kind.as_str())))
            {
                continue;
            }
            let reason = if steps.contains_key(&flow.id) {
                "WRONG_STEP_KIND"
            } else if unread.contains(&flow.id) {
                "EVIDENCE_NOT_DELIVERED"
            } else if summary.contains(&flow.id) {
                "SUMMARY_ONLY"
            } else if explanation.contains(&flow.id) {
                "EXPLANATION_ONLY"
            } else {
                "NO_STEP_EVIDENCE"
            };
            let refs = work.references_for("DEPENDENCY", &flow.id);
            let reference = if let [reference] = refs {
                Some(reference)
            } else {
                None
            };
            let safe_id = render::sequence_diagnostic_id(&flow.id);
            missing.insert((operation_id.clone(), flow.id.clone()), json!({
                "code":"STRUCTURE_OR_COVERAGE_INVALID","diagnosticCode":"MANDATORY_FLOW_UNCOVERED",
                "operationId":render::sequence_diagnostic_id(&operation_id),"flowId":safe_id,
                "flowReference":reference,"referenceStatus":if reference.is_some() {"SAME_WORK_HANDLE"} else if refs.is_empty() {"NO_WORK_HANDLE"} else {"AMBIGUOUS_WORK_HANDLES"},
                "flowReferenceDelivered":reference.is_some_and(|r| builder.received.contains(r)),
                "flowKind":kind,"allowedStepKinds":allowed,"reason":reason,
                "nextAction":if reference.is_none() {"No unique same-Work handle is available. Preserve an explicit evidence gap; do not copy a reference from another Work."} else {"Read the same-Work FLOW reference before citing it in step.meaning.evidence with an allowed step kind. Summary-only evidence does not cover a step. Author the supported behavior; no semantic step is generated."}
            }));
        }
    }
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    let mut output = vec![
        json!({"code":"MANDATORY_FLOW_COVERAGE_INCOMPLETE","work":work.id,"snapshot":work.snapshot,
        "contextDigest":work.checked.context_digest,"missingFlowCount":missing.len(),"complete":true,
        "nextAction":"Drain all proposal show pages using this proposal's nextCursor. Each uncovered operation/FLOW pair appears once; no missing entries are discarded."}),
    ];
    output.extend(missing.into_values());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(counts: &[usize]) -> (Work, Proposal) {
        let mut flows = BTreeMap::new();
        let mut sources = BTreeMap::new();
        let mut handles = BTreeMap::new();
        let mut entries = Vec::new();
        let mut operations = Vec::new();
        for (operation, count) in counts.iter().enumerate() {
            let symbol = format!("method{operation}");
            let mut first = None;
            for index in 0..*count {
                let id = format!("owned:flow:{operation:03}:{index:05}");
                let source_id = format!("source-{operation}-{index}");
                first.get_or_insert((id.clone(), source_id.clone()));
                sources.insert(source_id.clone(),json!({"id":source_id,"service":"owned","revision":"revision","file":"Owned.java","startLine":1,"endLine":1,"text":"return value;","textDigest":"digest","evidenceDigest":"evidence","authority":"TEST"}));
                flows.insert(id.clone(),Observation{id:id.clone(),kind:"FLOW".into(),service:"owned".into(),symbol:symbol.clone(),digest:format!("digest-{operation}-{index}"),source_ids:vec![source_id.clone()],normalized:json!({"kind":if index%3==0 {"IF"} else if index%3==1 {"RETURN"} else {"THROW"}})});
                handles.insert(
                    format!("d{operation}_{index}"),
                    work::Handle {
                        kind: "DEPENDENCY".into(),
                        id,
                    },
                );
                handles.insert(
                    format!("s{operation}_{index}"),
                    work::Handle {
                        kind: "SOURCE".into(),
                        id: source_id,
                    },
                );
            }
            let (first_flow, first_source) = first.unwrap();
            let op_id = format!("operation-{operation}");
            let reference = format!("e{operation}");
            entries.push(json!({"id":op_id,"service":"owned","symbol":symbol,"kind":"SOURCE_DECLARATION","trigger":{},"dependencyIds":[first_flow],"sourceIds":[first_source],"boundaries":[]}));
            handles.insert(
                reference.clone(),
                work::Handle {
                    kind: "ENTRYPOINT".into(),
                    id: op_id,
                },
            );
            operations.push(json!({"entrypoint":reference,"title":"Owned method","summary":{"text":"Owned static behavior.","evidence":[reference]},"steps":[]}));
        }
        let influence: BTreeMap<_, _> = flows
            .iter()
            .map(|(id, row)| (id.clone(), row.digest.clone()))
            .collect();
        let checked: check::Check=serde_json::from_value(json!({"schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"context","services":{"owned":{"schema":"evidence","service":"owned","revision":"revision","serviceDigest":"service","extractor":"source-syntax","runtimeMode":"source","coverage":"SYNTAX","boundaries":[],"entrypoints":entries,"observations":flows,"sources":sources,"contracts":{}}},"dependencies":flows,"unresolved":{},"interactions":{},"scenarios":{}})).unwrap();
        let work: Work=serde_json::from_value(json!({"schema":"codeclew-documentation-work/1.0","id":"work","subject":"service:owned","snapshot":"snapshot-A","request":{"schema":"codeclew-documentation-work-request/1.0","audience":"Owned diagnostics","maxItems":2,"maxBytes":49152},"checked":checked,"retained":null,"externalInputs":{},"handles":handles,"influence":influence,"obligations":[]})).unwrap();
        let input = serde_json::from_value(
            json!({"schema":"codeclew-documentation-proposal/1.0","operations":operations}),
        )
        .unwrap();
        (work, input)
    }

    fn builder(work: &Work) -> Builder<'_> {
        Builder {
            work,
            received: work.handles.keys().cloned().collect(),
            claims: BTreeMap::new(),
            diagnostics: Vec::new(),
            events: 0,
        }
    }

    fn step(kind: &str, evidence: &str) -> Step {
        serde_json::from_value(
            json!({"kind":kind,"meaning":{"text":"Owned behavior.","evidence":[evidence]}}),
        )
        .unwrap()
    }

    #[test]
    fn mandatory_flow_diagnostics_collect_all_reasons_nested_and_source_equivalents() {
        let (work, mut input) = fixture(&[3]);
        let mut b = builder(&work);
        let rows = collect(&b, &input).unwrap();
        assert_eq!(rows[0]["missingFlowCount"], 3);
        assert_eq!(rows[1]["reason"], "SUMMARY_ONLY");
        assert_eq!(rows[2]["reason"], "NO_STEP_EVIDENCE");
        input.operations[0].steps = vec![
            step("note", "d0_0"),
            step("alt", "d0_1"),
            step("note", "d0_2"),
        ];
        let rows = collect(&b, &input).unwrap();
        assert_eq!(rows[0]["missingFlowCount"], 2);
        assert!(rows[1..].iter().all(|r| r["reason"] == "WRONG_STEP_KIND"));
        input.operations[0].steps = vec![step("alt", "s0_0")];
        input.operations[0].steps[0].children = vec![step("note", "d0_1")];
        input.operations[0].steps[0].otherwise = vec![step("note", "d0_2")];
        assert!(collect(&b, &input).unwrap().is_empty());
        b.received.remove("d0_1");
        let rows = collect(&b, &input).unwrap();
        assert_eq!(rows[0]["missingFlowCount"], 1);
        assert_eq!(rows[1]["reason"], "EVIDENCE_NOT_DELIVERED");
        assert_eq!(rows[1]["flowReferenceDelivered"], false);
        let mut unavailable = work.clone();
        unavailable.identity_references.take();
        unavailable.handles.remove("d0_1");
        let rows = collect(&builder(&unavailable), &input).unwrap();
        assert_eq!(rows[0]["missingFlowCount"], 1);
        assert!(rows[1]["flowReference"].is_null());
        assert_eq!(rows[1]["referenceStatus"], "NO_WORK_HANDLE");
        assert!(
            rows[1]["nextAction"]
                .as_str()
                .unwrap()
                .contains("explicit evidence gap")
        );
    }

    #[test]
    fn mandatory_flow_diagnostics_preserve_every_large_missing_row_through_cache_and_paging() {
        let (work, input) = fixture(&[4000, 4053]);
        let rows = collect(&builder(&work), &input).unwrap();
        assert_eq!(rows[0]["missingFlowCount"], 8053);
        assert_eq!(rows.len(), 8054);
        let artifact = Artifact {
            schema: "codeclew-documentation-proposal-result/1.0".into(),
            id: String::new(),
            work: work.id.clone(),
            input,
            narrative: None,
            status: "NEEDS_REPAIR".into(),
            diagnostics: rows.clone(),
            claims: BTreeMap::new(),
            read_digest: "reads".into(),
            influence: work.influence.clone(),
            meaning_review: "UNASSESSED".into(),
        };
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path(), "Owned diagnostics").unwrap();
        let repo = Repository::open(temp.path()).unwrap();
        let stored = store_artifact(&repo, artifact).unwrap();
        assert!(stored.diagnostics_ref.is_some());
        assert!(stored.artifact.diagnostics.is_empty());
        assert!(bytes(&stored).unwrap().len() < 4 * 1024 * 1024);
        stored.validate_identity(&stored.artifact.id).unwrap();
        let reference = stored.diagnostics_ref.clone();
        let a = stored.clone().hydrate(work.influence.clone()).unwrap();
        let hydrated = hydrate_diagnostics(&repo, reference.clone(), a).unwrap();
        assert_eq!(hydrated.diagnostics, rows);
        let mut cursor = None;
        let mut returned = Vec::new();
        loop {
            let items=hydrated.diagnostics.iter().enumerate().map(|(index,row)|json!({"kind":"DIAGNOSTIC","id":format!("diagnostic-{index}"),"record":row})).collect();
            let page = super::super::super::cli::page(
                &hydrated.id,
                items,
                cursor.as_deref(),
                20,
                json!({"work":work.id}),
            )
            .unwrap();
            assert!(page["omitted"].as_array().unwrap().is_empty());
            returned.extend(
                page["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["record"].clone()),
            );
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(returned, rows);
        let unique: BTreeSet<_> = returned
            .iter()
            .skip(1)
            .map(|r| {
                (
                    r["operationId"].as_str().unwrap(),
                    r["flowId"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(unique.len(), 8053);
        let mut wrong = reference.unwrap();
        wrong.schema = "wrong".into();
        assert!(hydrate_diagnostics(&repo, Some(wrong), hydrated.clone()).is_err());
        let mut changed = stored;
        changed.diagnostics_ref.as_mut().unwrap().size += 1;
        assert!(changed.validate_identity(&hydrated.id).is_err());
        assert!(
            hydrate_diagnostics(&repo, changed.diagnostics_ref.clone(), hydrated.clone()).is_err()
        );
        let absent = super::super::super::cache::ObjectRef::new(
            DIAGNOSTICS_SCHEMA.into(),
            format!("sha256:{}", "0".repeat(64)),
            1,
        );
        assert!(hydrate_diagnostics(&repo, Some(absent), hydrated).is_err());
    }

    #[test]
    fn exact_flow_lookup_is_work_scoped_guarded_and_indexed_after_one_handle_build() {
        let (work, _) = fixture(&[8053]);
        assert!(work.identity_references.get().is_none());
        let selection: work::Selection=serde_json::from_value(json!({"query":{"kind":"FLOW","ids":["owned:flow:000:08052","owned:flow:000:00000"],"expectedSnapshot":"snapshot-A","expectedContextDigest":"context"}})).unwrap();
        let rows = work::rows(&work, &selection).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["reference"], "d0_8052");
        assert_eq!(
            rows[0]["record"],
            json!(work.checked.dependencies["owned:flow:000:08052"])
        );
        let index = work.identity_references.get().unwrap() as *const _;
        assert_eq!(work::rows(&work, &selection).unwrap(), rows);
        assert_eq!(work.identity_references.get().unwrap() as *const _, index);
        for ids in [json!([]), json!(null)] {
            assert!(
                serde_json::from_value::<work::Selection>(
                    json!({"query":{"kind":"FLOW","ids":ids}})
                )
                .is_err()
            );
        }
        for query in [
            json!({"kind":"FLOW","ids":["missing"]}),
            json!({"kind":"FLOW","ids":["owned:flow:000:00000"],"expectedSnapshot":"foreign"}),
            json!({"kind":"FLOW","ids":["owned:flow:000:00000"],"expectedContextDigest":"stale"}),
            json!({"kind":"FLOW","ids":["owned:flow:000:00000","owned:flow:000:00000"]}),
            json!({"kind":"SYMBOL","ids":["owned:flow:000:00000"]}),
            json!({"kind":"FLOW","ids":["owned:flow:000:00000"],"symbolContains":"method"}),
        ] {
            let selection = serde_json::from_value(json!({"query":query})).unwrap();
            assert!(work::rows(&work, &selection).is_err());
        }
        let mut foreign = work.clone();
        foreign
            .checked
            .dependencies
            .get_mut("owned:flow:000:08052")
            .unwrap()
            .service = "other".into();
        assert!(work::rows(&foreign, &selection).is_err());
        let mut ambiguous = work.clone();
        ambiguous.identity_references.take();
        ambiguous.handles.insert(
            "duplicate".into(),
            work::Handle {
                kind: "DEPENDENCY".into(),
                id: "owned:flow:000:08052".into(),
            },
        );
        assert!(
            work::rows(&ambiguous, &selection)
                .unwrap_err()
                .message
                .contains("WORK_IDENTITY_AMBIGUOUS")
        );
        let mut unregistered = work.clone();
        unregistered.identity_references.take();
        unregistered.handles.remove("d0_8052");
        assert!(
            work::rows(&unregistered, &selection)
                .unwrap_err()
                .message
                .contains("WORK_IDENTITY_NOT_REGISTERED")
        );
        let mut stale = work.clone();
        stale
            .influence
            .insert("owned:flow:000:08052".into(), "stale-digest".into());
        assert!(
            work::rows(&stale, &selection)
                .unwrap_err()
                .message
                .contains("WORK_IDENTITY_NOT_FOUND")
        );
        let oversized:work::Selection=serde_json::from_value(json!({"query":{"kind":"FLOW","ids":(0..9).map(|n|format!("owned:flow:000:{n:05}")).collect::<Vec<_>>()}})).unwrap();
        assert!(work::rows(&work, &oversized).is_err());
    }

    #[test]
    fn mandatory_flow_diagnostics_only_deduplicate_the_matching_renderer_error() {
        let (work, input) = fixture(&[2]);
        let rows = collect(&builder(&work), &input).unwrap();
        let error = invalid("Owned coverage")
            .with_relevant("MANDATORY_FLOW_UNCOVERED")
            .with_relevant("owned:flow:000:00000")
            .with_relevant("operation-0");
        assert!(already_reported(&error, &rows));
        assert!(!already_reported(&error, &[]));
        let other = invalid("Owned retained coverage")
            .with_relevant("MANDATORY_FLOW_UNCOVERED")
            .with_relevant("owned:flow:000:00000")
            .with_relevant("retained-operation");
        assert!(!already_reported(&other, &rows));
    }
}
