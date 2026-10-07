//! Kotlin qualification adapter for the shared allocation/binding proof.
//! It consumes the same closed original-byte input as data-state projection.
use super::super::{
    data_state::input::{self as syntax, Kind, Node, Prepared, Role, VariableKind},
    model::{CallableProjection, Gap, NeutralExactCallSite},
};
use super::{
    Context, gap,
    handoff::{Binding, Input, LocalChange, LocalValue, Row},
};
use crate::documentation::{digest, model::Observation};
use crate::semantic_validation::{KotlinConstructorStorageProof, KotlinPropertyStorageProof};

pub(super) fn input(
    context: &mut Context<'_>,
    endpoint: &CallableProjection,
    worker: &CallableProjection,
    wiring: &CallableProjection,
) -> Result<Input, Gap> {
    let evidence = context.evidence;
    let prepared = |callable: &CallableProjection| {
        syntax::prepare_handoff(evidence, callable).map_err(|_| {
            gap(
                "WIRING_INPUT_INVALID",
                "Retained Kotlin input fails its closed compiler/source contract.",
                callable.citation_id.clone(),
            )
        })
    };
    let (
        Prepared::Body(endpoint_input),
        Prepared::Body(worker_input),
        Prepared::Body(wiring_input),
    ) = (prepared(endpoint)?, prepared(worker)?, prepared(wiring)?)
    else {
        return Err(gap(
            "WIRING_INPUT_UNAVAILABLE",
            "Selected Kotlin bodies lack bounded compiler-identity inputs.",
            wiring.citation_id.clone(),
        ));
    };
    let (endpoint_owner, endpoint_storage, submit_citation) =
        queue_storage(context, endpoint, &endpoint_input, &["offer", "put", "add"])?;
    let (worker_owner, worker_storage, consume_citation) =
        queue_storage(context, worker, &worker_input, &["poll", "take"])?;
    if endpoint_owner == worker_owner {
        return Err(gap(
            "WIRING_OWNER_AMBIGUOUS",
            "Endpoint and worker storage owners must be distinct in this bounded proof.",
            wiring.citation_id.clone(),
        ));
    }
    let mut rows = vec![];
    let body = wiring_input.body.ok_or_else(|| control_gap(wiring))?;
    qualify_rows(
        context,
        wiring,
        &wiring_input,
        wiring_input.node(body),
        &[
            (&endpoint_owner, &endpoint_storage),
            (&worker_owner, &worker_storage),
        ],
        &mut rows,
    )?;
    Ok(Input {
        endpoint_owner,
        worker_owner,
        endpoint_storage,
        worker_storage,
        operation_citations: vec![submit_citation, consume_citation],
        rows,
    })
}

fn control_gap(callable: &CallableProjection) -> Gap {
    gap(
        "WIRING_CONTROL_AMBIGUOUS",
        "Conditional, interrupted, effectful or unsupported wiring cannot establish one shared object allocation.",
        callable.citation_id.clone(),
    )
}
fn unwrap(mut node: Node<'_>) -> Node<'_> {
    while node.kind() == Kind::Parenthesized {
        let Some(child) = node.children().into_iter().next() else {
            break;
        };
        node = child;
    }
    node
}
fn site<'a>(
    callable: &'a CallableProjection,
    input: &syntax::Input<'_>,
    node: Node<'_>,
) -> Result<&'a NeutralExactCallSite, Gap> {
    let binding = input.calls.get(&node.range()).ok_or_else(|| {
        gap(
            "WIRING_CALL_UNAVAILABLE",
            "Invocation has no unique original-byte compiler site.",
            callable.citation_id.clone(),
        )
    })?;
    callable
        .retained_call_sites
        .as_ref()
        .and_then(|r| {
            r.sites
                .iter()
                .find(|s| binding.occurrence == format!("compiler-site/{}", s.relation_id))
        })
        .ok_or_else(|| {
            gap(
                "WIRING_CALL_UNAVAILABLE",
                "Invocation compiler site is not retained.",
                callable.citation_id.clone(),
            )
        })
}
fn queue_target(target: &str, methods: &[&str]) -> bool {
    methods.iter().any(|method| {
        target.starts_with(&format!(
            "callable:java/util/concurrent/BlockingQueue.{method}#jvm:"
        ))
    })
}
fn queue_allocation(target: &str) -> bool {
    [
        "LinkedBlockingQueue",
        "ArrayBlockingQueue",
        "PriorityBlockingQueue",
        "SynchronousQueue",
        "LinkedTransferQueue",
        "DelayQueue",
    ]
    .iter()
    .any(|name| {
        [
            format!("constructor:java/util/concurrent/{name}.{name}#jvm:"),
            format!("constructor:java/util/concurrent/{name}.<init>#jvm:"),
        ]
        .iter()
        .any(|prefix| target.starts_with(prefix))
    })
}
fn queue_storage(
    context: &mut Context<'_>,
    callable: &CallableProjection,
    input: &syntax::Input<'_>,
    methods: &[&str],
) -> Result<(String, String, String), Gap> {
    let matches: Vec<_> = input
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == Kind::Call)
        .filter_map(|(index, _)| {
            let node = input.node(index);
            site(callable, input, node)
                .ok()
                .filter(|s| queue_target(&s.target_identity, methods))
                .map(|s| (node, s))
        })
        .collect();
    let [(call, exact)] = matches.as_slice() else {
        return Err(gap(
            "QUEUE_OPERATION_UNAVAILABLE_OR_AMBIGUOUS",
            "Select one exact BlockingQueue submission or consumption occurrence.",
            callable.citation_id.clone(),
        ));
    };
    let receiver = call.child(Role::Receiver).map(unwrap).ok_or_else(|| {
        gap(
            "QUEUE_RECEIVER_UNSUPPORTED",
            "Queue operation lacks direct ordinary property storage.",
            Some(exact.citation_id.clone()),
        )
    })?;
    if !matches!(receiver.kind(), Kind::Variable | Kind::Member)
        || receiver
            .child(Role::Receiver)
            .is_some_and(|r| unwrap(r).kind() != Kind::This)
    {
        return Err(gap(
            "QUEUE_RECEIVER_UNSUPPORTED",
            "Queue receiver is not ordinary storage on the selected dispatch receiver.",
            Some(exact.citation_id.clone()),
        ));
    }
    let variable = input
        .variables
        .get(&receiver.range())
        .filter(|v| v.kind == VariableKind::Property)
        .ok_or_else(|| {
            gap(
                "QUEUE_RECEIVER_UNSUPPORTED",
                "Queue receiver has no admitted compiler PROPERTY binding.",
                Some(exact.citation_id.clone()),
            )
        })?;
    let property = variable
        .member
        .as_ref()
        .and_then(|m| context.evidence.observations.get(&m.declaration_id))
        .ok_or_else(|| {
            gap(
                "QUEUE_STORAGE_UNAVAILABLE",
                "Queue property declaration is not retained.",
                Some(exact.citation_id.clone()),
            )
        })?
        .clone();
    let owner = &context.evidence.observations[&callable.declaration_id];
    if property.normalized["ownerIdentity"] != owner.normalized["ownerIdentity"]
        || property.normalized["declaredType"]
            .as_str()
            .is_none_or(|t| !t.starts_with("java/util/concurrent/BlockingQueue"))
    {
        return Err(gap(
            "QUEUE_RECEIVER_UNSUPPORTED",
            "Queue property does not belong to the selected compiler owner or queue type.",
            Some(exact.citation_id.clone()),
        ));
    }
    if owner.normalized["documentation"]["dataInput"]["members"]
        .as_array()
        .is_some_and(|rows| {
            rows.iter()
                .any(|r| r["propertyIdentity"] == variable.identity && r["accessMode"] == "WRITE")
        })
    {
        return Err(gap(
            "QUEUE_STORAGE_REASSIGNED",
            "Selected body reassigns the queue property.",
            Some(exact.citation_id.clone()),
        ));
    }
    let owner_identity = owner.normalized["ownerIdentity"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    context.retain(&property);
    Ok((
        owner_identity,
        variable.identity.clone(),
        exact.citation_id.clone(),
    ))
}

fn constructor_slot(
    context: &mut Context<'_>,
    callable: &CallableProjection,
    exact: &NeutralExactCallSite,
    owner: &str,
    property: &str,
) -> Result<(usize, String), Gap> {
    let scope = &context.evidence.observations[&callable.declaration_id].normalized["scope"];
    let constructors: Vec<_> = context
        .evidence
        .observations
        .values()
        .filter(|o| {
            o.kind == "SYMBOL"
                && o.service == context.evidence.service
                && o.symbol == exact.target_identity
                && o.normalized["symbolIdentity"] == o.symbol
                && o.normalized["scope"] == *scope
                && o.normalized["ownerIdentity"] == owner
                && o.normalized["declarationKind"] == "CONSTRUCTOR"
                && o.normalized["schema"] == "declaration-descriptor/0.1"
                && o.normalized["provider"] == "K2_FIR"
                && o.normalized["resolution"] == "PROVEN"
                && o.normalized["sourceProvenance"] == "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
                && o.normalized["compilerAuthority"] == "fir-facts-extractor/0.6"
        })
        .cloned()
        .collect();
    let [constructor] = constructors.as_slice() else {
        return Err(gap(
            "CONSTRUCTOR_BINDING_UNAVAILABLE",
            "Selected queue owner lacks one exact retained constructor.",
            Some(exact.citation_id.clone()),
        ));
    };
    crate::semantic_validation::validate_constructor_storage_proof(&constructor.normalized)
        .map_err(|_| {
            gap(
                "CONSTRUCTOR_BINDING_UNAVAILABLE",
                "Constructor storage contract is invalid.",
                Some(exact.citation_id.clone()),
            )
        })?;
    let proof: KotlinConstructorStorageProof = serde_json::from_value(constructor.normalized["documentationConstructorStorage"].clone())
        .map_err(|_| gap("CONSTRUCTOR_BINDING_UNAVAILABLE", "Primary constructor storage mapping is unavailable; init blocks, custom storage and effectful initializers remain boundaries.", Some(exact.citation_id.clone())))?;
    let bindings: Vec<_> = proof
        .bindings
        .iter()
        .filter(|b| b.property_identity == property)
        .collect();
    let [binding] = bindings.as_slice() else {
        return Err(gap(
            "CONSTRUCTOR_BINDING_UNAVAILABLE",
            "No unique compiler parameter-to-property binding is retained.",
            Some(exact.citation_id.clone()),
        ));
    };
    let properties: Vec<_> = context
        .evidence
        .observations
        .values()
        .filter(|p| {
            p.kind == "SYMBOL"
                && p.service == context.evidence.service
                && p.symbol == property
                && p.normalized["symbolIdentity"] == property
                && p.normalized["scope"] == *scope
                && p.normalized["ownerIdentity"] == owner
        })
        .cloned()
        .collect();
    let [property] = properties.as_slice() else {
        return Err(gap(
            "CONSTRUCTOR_BINDING_UNAVAILABLE",
            "Constructor property identity is ambiguous.",
            Some(exact.citation_id.clone()),
        ));
    };
    let storage: KotlinPropertyStorageProof = serde_json::from_value(
        property.normalized["documentationStorage"].clone(),
    )
    .map_err(|_| {
        gap(
            "CONSTRUCTOR_BINDING_UNAVAILABLE",
            "Constructor target is not ordinary backing-property storage.",
            Some(exact.citation_id.clone()),
        )
    })?;
    let parameter = &constructor.normalized["parameterTypes"][binding.parameter_index];
    if !storage.admitted()
        || parameter["index"].as_u64() != Some(binding.parameter_index as u64)
        || parameter["type"] != property.normalized["declaredType"]
    {
        return Err(gap(
            "CONSTRUCTOR_BINDING_UNAVAILABLE",
            "Compiler parameter and ordinary property storage disagree.",
            Some(exact.citation_id.clone()),
        ));
    }
    let constructor_source = retained_storage_source(context, constructor)?;
    let property_source = retained_storage_source(context, property)?;
    context.retain(constructor);
    context.retain(property);
    // Property citations and constructor observations are both retained; the
    // binding citation denotes the compiler-qualified constructor source.
    context.citation(&property_source, 0, property_source.text.len());
    let citation = context.citation(&constructor_source, 0, constructor_source.text.len());
    Ok((binding.parameter_index, citation))
}
pub(super) fn retained_storage_source(
    context: &Context<'_>,
    observation: &Observation,
) -> Result<crate::documentation::model::Source, Gap> {
    let site = &observation.normalized["outlineOwnerSource"];
    let source = site["sourceId"]
        .as_str()
        .and_then(|id| context.evidence.sources.get(id))
        .filter(|s| {
            context.valid_source(s)
                && s.service == context.evidence.service
                && s.revision == context.evidence.revision
        })
        .ok_or_else(|| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Exact original storage/constructor source is unavailable.",
                None,
            )
        })?;
    if observation.digest
        != digest(&observation.normalized).map_err(|_| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Storage digest cannot be validated.",
                None,
            )
        })?
        || observation.source_ids != [source.id.clone()]
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || site["file"] != source.file
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || site["byteEnd"]
            .as_u64()
            .zip(site["byteStart"].as_u64())
            .and_then(|(end, start)| end.checked_sub(start))
            != Some(source.text.len() as u64)
    {
        return Err(gap(
            "CONSTRUCTOR_SOURCE_UNAVAILABLE",
            "Storage/constructor original source pins disagree.",
            None,
        ));
    }
    Ok(source.clone())
}

fn local_reference(input: &syntax::Input<'_>, node: Node<'_>) -> Option<String> {
    let node = unwrap(node);
    (node.kind() == Kind::Variable)
        .then(|| input.variables.get(&node.range()))
        .flatten()
        .filter(|v| v.kind == VariableKind::Local && !v.declaration)
        .map(|v| v.identity.clone())
}
fn simple_actual(input: &syntax::Input<'_>, node: Node<'_>) -> bool {
    let node = unwrap(node);
    node.kind() == Kind::Literal
        || (node.kind() == Kind::Variable
            && input
                .variables
                .get(&node.range())
                .is_some_and(|v| matches!(v.kind, VariableKind::Local | VariableKind::Parameter)))
}
fn qualify_rows(
    context: &mut Context<'_>,
    callable: &CallableProjection,
    input: &syntax::Input<'_>,
    node: Node<'_>,
    owners: &[(&String, &String)],
    rows: &mut Vec<Row>,
) -> Result<(), Gap> {
    match node.kind() {
        Kind::Block => {
            for child in node.children() {
                qualify_rows(context, callable, input, child, owners, rows)?;
            }
            return Ok(());
        }
        Kind::Local | Kind::Expression => {
            for child in node.children() {
                qualify_rows(context, callable, input, child, owners, rows)?;
            }
            return Ok(());
        }
        _ => {}
    }
    let (identity, expression) = match node.kind() {
        Kind::Declarator => {
            let variable = input
                .variables
                .get(&node.range())
                .filter(|v| v.declaration && v.kind == VariableKind::Local)
                .ok_or_else(|| control_gap(callable))?;
            (
                Some(variable.identity.clone()),
                node.child(Role::Initializer),
            )
        }
        Kind::Assignment => {
            let identity = node
                .child(Role::Left)
                .and_then(|n| local_reference(input, n))
                .ok_or_else(|| control_gap(callable))?;
            (Some(identity), node.child(Role::Right))
        }
        Kind::Construct => (None, Some(node)),
        _ => return Err(control_gap(callable)),
    };
    let mut row = Row::default();
    let value = if let Some(expression) = expression.map(unwrap) {
        if expression.kind() == Kind::Construct {
            let exact = site(callable, input, expression)?;
            if !expression.default_arguments().is_empty()
                || expression
                    .actuals()
                    .iter()
                    .any(|(n, slot)| slot.is_none() || !simple_actual(input, *n))
            {
                return Err(control_gap(callable));
            }
            for (owner, storage) in owners {
                let matching = context.evidence.observations.values().any(|o| {
                    o.symbol == exact.target_identity && o.normalized["ownerIdentity"] == **owner
                });
                if matching {
                    let (slot, citation) =
                        constructor_slot(context, callable, exact, owner, storage)?;
                    let actuals: Vec<_> = expression
                        .actuals()
                        .into_iter()
                        .filter(|(_, formal)| *formal == Some(slot))
                        .collect();
                    let [(actual, _)] = actuals.as_slice() else {
                        return Err(gap(
                            "CONSTRUCTOR_ARGUMENT_UNAVAILABLE",
                            "Queue formal slot has no unique explicit actual.",
                            Some(exact.citation_id.clone()),
                        ));
                    };
                    let argument = local_reference(input, *actual).ok_or_else(|| {
                        gap(
                            "QUEUE_ARGUMENT_ALIAS_UNPROVEN",
                            "Queue argument is not an exact local compiler variable.",
                            Some(exact.citation_id.clone()),
                        )
                    })?;
                    row.bindings.push(Binding {
                        owner: (*owner).clone(),
                        argument,
                        argument_syntax: actual.text(),
                        citation: exact.citation_id.clone(),
                        storage_citation: citation,
                    });
                }
            }
            if queue_allocation(&exact.target_identity) {
                LocalValue::Allocation {
                    id: format!("{}:{}", callable.declaration_id, exact.citation_id),
                    citation: exact.citation_id.clone(),
                }
            } else if !row.bindings.is_empty() {
                LocalValue::Unknown
            } else {
                return Err(control_gap(callable));
            }
        } else if let Some(reference) = local_reference(input, expression) {
            LocalValue::Reference(reference)
        } else if expression.kind() == Kind::Literal
            || (expression.kind() == Kind::Variable && simple_actual(input, expression))
        {
            LocalValue::Unknown
        } else {
            return Err(control_gap(callable));
        }
    } else {
        LocalValue::Unknown
    };
    if let Some(identity) = identity {
        row.locals.push(LocalChange { identity, value });
    }
    rows.push(row);
    Ok(())
}
