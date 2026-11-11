//! One source-declared allocation/binding proof over producer-qualified storage.
//! Producers qualify syntax, compiler targets/formal slots and storage bindings;
//! this core never parses a language, equates types, or identifies runtime objects.
use super::super::model::Gap;
use super::gap;
use std::collections::BTreeMap;

#[cfg(test)]
pub(in crate::documentation) struct Input {
    pub endpoint_owner: String,
    pub worker_owner: String,
    pub endpoint_storage: String,
    pub worker_storage: String,
    pub operation_citations: Vec<String>,
    pub rows: Vec<Row>,
}
#[derive(Default)]
#[cfg(test)]
pub(in crate::documentation) struct Row {
    pub bindings: Vec<Binding>,
    pub locals: Vec<LocalChange>,
}
#[cfg(test)]
pub(in crate::documentation) struct Binding {
    pub owner: String,
    pub argument: String,
    pub argument_syntax: String,
    pub citation: String,
    pub storage_citation: String,
}
#[cfg(test)]
pub(in crate::documentation) struct LocalChange {
    pub identity: String,
    pub value: LocalValue,
}
#[cfg(test)]
pub(in crate::documentation) enum LocalValue {
    Allocation { id: String, citation: String },
    Reference(String),
    Unknown,
}

#[cfg(test)]
fn prove(
    input: Input,
    wiring_citation: Option<String>,
) -> Result<(String, String, String, Vec<String>), Gap> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut allocation_citations = BTreeMap::new();
    let mut bindings: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    for row in input.rows {
        for binding in row.bindings {
            let allocation = aliases.get(&binding.argument).ok_or_else(|| {
                gap(
                    "QUEUE_ARGUMENT_ALIAS_UNPROVEN",
                    format!(
                        "Constructor queue argument {} is not a bound local allocation or alias.",
                        binding.argument_syntax
                    ),
                    Some(binding.citation.clone()),
                )
            })?;
            bindings.entry(binding.owner).or_default().push((
                allocation.clone(),
                binding.citation,
                binding.storage_citation,
            ));
        }
        for local in row.locals {
            match local.value {
                LocalValue::Allocation { id, citation } => {
                    aliases.insert(local.identity, id.clone());
                    allocation_citations.insert(id, citation);
                }
                LocalValue::Reference(identity) => {
                    if let Some(allocation) = aliases.get(&identity).cloned() {
                        aliases.insert(local.identity, allocation);
                    } else {
                        aliases.remove(&local.identity);
                    }
                }
                LocalValue::Unknown => {
                    aliases.remove(&local.identity);
                }
            }
        }
    }
    let selected = |owner: &str| match bindings.get(owner).map(Vec::as_slice) {
        Some([binding]) => Some(binding.clone()),
        _ => None,
    };
    match (
        selected(&input.endpoint_owner),
        selected(&input.worker_owner),
    ) {
        (Some(endpoint), Some(worker)) if endpoint.0 == worker.0 => {
            let mut citations = input.operation_citations;
            citations.extend([endpoint.1, endpoint.2, worker.1, worker.2]);
            if let Some(citation) = allocation_citations.get(&endpoint.0) {
                citations.push(citation.clone());
            }
            citations.sort();
            citations.dedup();
            Ok((
                endpoint.0,
                input.endpoint_storage,
                input.worker_storage,
                citations,
            ))
        }
        _ => Err(gap(
            "SHARED_QUEUE_OBJECT_UNPROVEN",
            "Selected constructions do not bind exactly one endpoint and worker to the same local queue allocation.",
            wiring_citation,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn allocation(local: &str, id: &str) -> Row {
        Row {
            locals: vec![LocalChange {
                identity: local.into(),
                value: LocalValue::Allocation {
                    id: id.into(),
                    citation: format!("allocation-{id}"),
                },
            }],
            ..Row::default()
        }
    }
    fn binding(owner: &str, local: &str) -> Row {
        Row {
            bindings: vec![Binding {
                owner: owner.into(),
                argument: local.into(),
                argument_syntax: "same-spelling".into(),
                citation: format!("construction-{owner}"),
                storage_citation: format!("storage-{owner}"),
            }],
            ..Row::default()
        }
    }
    fn input(rows: Vec<Row>) -> Input {
        Input {
            endpoint_owner: "endpoint".into(),
            worker_owner: "worker".into(),
            endpoint_storage: "endpoint-storage".into(),
            worker_storage: "worker-storage".into(),
            operation_citations: vec!["submit".into(), "poll".into()],
            rows,
        }
    }
    #[test]
    fn local_aliases_prove_one_allocation_and_retain_all_evidence() {
        let alias = Row {
            locals: vec![LocalChange {
                identity: "alias-id".into(),
                value: LocalValue::Reference("queue-id".into()),
            }],
            ..Row::default()
        };
        let proof = prove(
            input(vec![
                allocation("queue-id", "q1"),
                alias,
                binding("endpoint", "queue-id"),
                binding("worker", "alias-id"),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(proof.0, "q1");
        assert_eq!(
            proof.3,
            [
                "allocation-q1",
                "construction-endpoint",
                "construction-worker",
                "poll",
                "storage-endpoint",
                "storage-worker",
                "submit"
            ]
        );
    }
    #[test]
    fn equal_spelling_or_types_never_unify_distinct_allocations_or_variable_ids() {
        let gap = prove(
            input(vec![
                allocation("local-1", "q1"),
                allocation("local-2", "q2"),
                binding("endpoint", "local-1"),
                binding("worker", "local-2"),
            ]),
            None,
        )
        .unwrap_err();
        assert_eq!(gap.code, "SHARED_QUEUE_OBJECT_UNPROVEN");
        let gap = prove(
            input(vec![
                allocation("local-1", "q1"),
                binding("endpoint", "local-2"),
            ]),
            None,
        )
        .unwrap_err();
        assert_eq!(gap.code, "QUEUE_ARGUMENT_ALIAS_UNPROVEN");
    }
    #[test]
    fn later_reassignment_does_not_rewrite_prior_constructor_arguments() {
        let gap = prove(
            input(vec![
                allocation("local", "q1"),
                binding("endpoint", "local"),
                allocation("local", "q2"),
                binding("worker", "local"),
            ]),
            None,
        )
        .unwrap_err();
        assert_eq!(gap.code, "SHARED_QUEUE_OBJECT_UNPROVEN");
    }
    #[test]
    fn ambiguous_owner_instances_and_unknown_aliases_withhold_proof() {
        let gap = prove(
            input(vec![
                allocation("local", "q1"),
                binding("endpoint", "local"),
                binding("endpoint", "local"),
                binding("worker", "local"),
            ]),
            None,
        )
        .unwrap_err();
        assert_eq!(gap.code, "SHARED_QUEUE_OBJECT_UNPROVEN");
        let unknown = Row {
            locals: vec![LocalChange {
                identity: "local".into(),
                value: LocalValue::Unknown,
            }],
            ..Row::default()
        };
        let gap = prove(
            input(vec![
                allocation("local", "q1"),
                unknown,
                binding("endpoint", "local"),
            ]),
            None,
        )
        .unwrap_err();
        assert_eq!(gap.code, "QUEUE_ARGUMENT_ALIAS_UNPROVEN");
    }
}
