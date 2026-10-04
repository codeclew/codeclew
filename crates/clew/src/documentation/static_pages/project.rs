//! Deterministic projection of selected Java source, not an operation answer.
use super::model::*;
use crate::documentation::{
    check::Check,
    digest, invalid,
    model::{Observation, ServiceEvidence, Source},
    notes::Association,
    store::RepositoryInputs,
};
use crate::error::ClewError;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Parser};

const JAVA_SCHEMA: &str = "codeclew-java-compiler-fact/1.0";
const WRAP: &str = "class __Projection {\n";

pub fn project(checked: &Check, selections: &[Selection]) -> Result<BundleProjection, ClewError> {
    if selections.iter().any(|s| !s.authored_paragraphs.is_empty()) {
        return Err(invalid(
            "frozen authored paragraphs require repository-aware docs pages render",
        ));
    }
    project_unresolved(checked, selections)
}
pub(super) fn project_unresolved(
    checked: &Check,
    selections: &[Selection],
) -> Result<BundleProjection, ClewError> {
    let note_inputs = if selections
        .iter()
        .any(|selection| !selection.note_ids.is_empty())
    {
        Some(pinned_note_inputs(checked)?)
    } else {
        None
    };
    let mut ids = BTreeSet::new();
    let mut pages = Vec::new();
    for selection in selections {
        if selection.expand_data_state && !selection.expand_source_calls {
            return Err(invalid("expandDataState requires expandSourceCalls"));
        }
        if selection.id.is_empty() || !ids.insert(&selection.id) {
            return Err(invalid(
                "native page selection IDs must be nonempty and unique",
            ));
        }
        let human_instructions = human_instructions(note_inputs, selection)?;
        let evidence = checked
            .services
            .get(&selection.service)
            .ok_or_else(|| invalid("native page selected service is not retained in Check"))?;
        let mut ctx = Context {
            evidence,
            citations: BTreeMap::new(),
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
        };
        let endpoint = ctx.callable(&selection.endpoint_declaration)?;
        let worker = ctx.callable(&selection.worker_declaration)?;
        let wiring = selection
            .wiring_declaration
            .as_ref()
            .map(|id| ctx.callable(id))
            .transpose()?;
        let handoff = ctx.handoff(&endpoint, &worker, wiring.as_ref());
        let diagnostics = diagnostics(&worker);
        let mut limitations = vec![gap(
            "RUNTIME_NOT_OBSERVED",
            "Source declarations do not establish deployed activation, delivery, external success, or durable completion.",
            None,
        )];
        for boundary in &evidence.boundaries {
            limitations.push(gap("RETAINED_SERVICE_BOUNDARY", boundary, None));
        }
        pages.push(PageContent {
            id: selection.id.clone(),
            title: format!("{} → {}", endpoint.symbol, worker.symbol),
            selection: selection.clone(),
            service_revision: evidence.revision.clone(),
            service_digest: evidence.service_digest.clone(),
            endpoint,
            worker,
            wiring,
            handoff,
            diagnostics,
            citations: ctx.citations,
            observations: ctx.observations,
            sources: ctx.sources,
            limitations,
            human_instructions,
            authored_paragraphs: vec![],
            examined_sources: None,
            data_state: None,
        });
    }
    let mut projection = BundleProjection {
        schema: SCHEMA.into(),
        input_digest: checked.input_digest.clone(),
        context_digest: checked.context_digest.clone(),
        selection_digest: digest(&selections)?,
        pages,
        source_call_graph: None,
    };
    super::linked::attach(checked, &mut projection)?;
    super::data_state::attach(checked, &mut projection)?;
    Ok(projection)
}

fn note_target(inputs: &RepositoryInputs, target: &str, service: &str) -> (bool, bool) {
    if let Some(target) = target.strip_prefix("service:") {
        let (id, section) = target
            .split_once('/')
            .map(|(id, section)| (id, Some(section)))
            .unwrap_or((target, None));
        let exists = inputs.services.contains_key(id)
            && section.is_none_or(crate::documentation::sections::contains);
        return (exists, exists && id == service);
    }
    (false, false)
}

fn pinned_note_inputs(checked: &Check) -> Result<&RepositoryInputs, ClewError> {
    let pinned = checked
        .source_inputs
        .as_ref()
        .ok_or_else(|| invalid("native page notes require pinned captured source inputs"))?;
    crate::documentation::source_inputs::validate(pinned)?;
    if pinned.input_digest != checked.input_digest {
        return Err(invalid(
            "native page note input identity does not match Check",
        ));
    }
    Ok(&pinned.inputs)
}

fn human_instructions(
    inputs: Option<&RepositoryInputs>,
    selection: &Selection,
) -> Result<Vec<HumanInstruction>, ClewError> {
    if selection.note_ids.is_empty() {
        return Ok(Vec::new());
    }
    if selection.note_ids.len() > 128 {
        return Err(invalid("native page note selection exceeds 128 IDs"));
    }
    let inputs =
        inputs.ok_or_else(|| invalid("native page notes require pinned captured source inputs"))?;
    let mut ids = BTreeSet::new();
    selection.note_ids.iter().map(|id| {
        if !crate::documentation::store::valid_id(id) || !ids.insert(id) {
            return Err(invalid("native page note IDs must be valid and unique"));
        }
        let captured = inputs.notes.get(id)
            .ok_or_else(|| invalid("native page selected note is not captured in Check"))?;
        let association: Association = serde_json::from_value(captured["association"].clone())
            .map_err(|_| invalid("native page captured note association is invalid"))?;
        let association_digest = digest(&association)?;
        if association.id != *id
            || association.schema != "codeclew-documentation-note-association/1.0"
            || captured["associationDigest"] != association_digest
            || captured["authority"] != "HUMAN_OR_IMPORTED_UNVERIFIED"
        {
            return Err(invalid("native page captured note identity or association digest is inconsistent"));
        }
        let author = association.metadata.get("author").and_then(serde_json::Value::as_str)
            .filter(|author| !author.trim().is_empty() && author.len() <= 512)
            .ok_or_else(|| invalid("native page selected note requires a nonblank metadata.author string of at most 512 bytes"))?;
        let targets: Vec<_> = association.targets.iter()
            .map(|target| note_target(inputs, target, &selection.service)).collect();
        if targets.is_empty() || targets.iter().any(|(exists, _)| !exists) {
            return Err(invalid("native page selected note has unavailable or unsupported captured targets"));
        }
        if !targets.iter().any(|(_, related)| *related) {
            return Err(invalid("native page selected note is unrelated to the selected service"));
        }
        let text = captured["original"]["text"].as_str()
            .filter(|text| text.len() <= 256 * 1024)
            .ok_or_else(|| invalid("native page selected note original text is unavailable"))?;
        let content_digest = digest(&text)?;
        if captured["original"]["status"] != "CAPTURED"
            || captured["original"]["digest"] != content_digest
        {
            return Err(invalid("native page selected note original capture or digest is inconsistent"));
        }
        Ok(HumanInstruction {
            id: id.clone(),
            title: association.title.clone(),
            declared_author: author.into(),
            classification: association.classification.clone(),
            period: association.period.clone(),
            version_digest: digest(&(&association_digest, &content_digest))?,
            content_digest,
            association_digest,
            text: text.into(),
            authority: "HUMAN_OR_IMPORTED_UNVERIFIED".into(),
            source_claim_status: "UNASSESSED".into(),
            association: captured["association"].clone(),
        })
    }).collect()
}

fn gap(code: &str, detail: impl Into<String>, citation_id: Option<String>) -> Gap {
    Gap {
        code: code.into(),
        detail: detail.into(),
        citation_id,
    }
}

pub(super) fn compiler(o: &Observation) -> bool {
    o.kind == "SYMBOL"
        && o.normalized["schema"] == JAVA_SCHEMA
        && matches!(
            o.normalized["declarationKind"].as_str(),
            Some("METHOD" | "CONSTRUCTOR" | "FIELD")
        )
        && o.normalized["scope"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        && o.normalized["symbolIdentity"] == o.symbol
}

fn named<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn descendants<'a>(node: Node<'a>, kind: &str) -> Vec<Node<'a>> {
    let mut found = Vec::new();
    let mut pending = vec![node];
    while let Some(n) = pending.pop() {
        if n.kind() == kind {
            found.push(n);
        }
        pending.extend(named(n).into_iter().rev());
    }
    found
}

fn active_nodes<'a>(parsed: &Parsed, node: Node<'a>, kind: &str) -> Vec<Node<'a>> {
    if matches!(
        node.kind(),
        "lambda_expression" | "class_body" | "ternary_expression" | "switch_expression"
    ) {
        return Vec::new();
    }
    let mut found = if node.kind() == kind {
        vec![node]
    } else {
        vec![]
    };
    if short_circuit(parsed, node) {
        if let Some(left) = node.child_by_field_name("left") {
            found.extend(active_nodes(parsed, left, kind));
        }
    } else {
        for child in named(node) {
            found.extend(active_nodes(parsed, child, kind));
        }
    }
    found
}

pub(super) struct Parsed {
    text: String,
    tree: tree_sitter::Tree,
}
impl Parsed {
    pub(super) fn new(source: &str) -> Option<Self> {
        let text = format!("{WRAP}{source}\n}}");
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .ok()?;
        let tree = parser.parse(&text, None)?;
        Some(Self { text, tree })
    }
    pub(super) fn text(&self, node: Node<'_>) -> String {
        self.text[node.byte_range()].to_owned()
    }
    pub(super) fn range(&self, node: Node<'_>) -> (usize, usize) {
        (
            node.start_byte().saturating_sub(WRAP.len()),
            node.end_byte().saturating_sub(WRAP.len()),
        )
    }
    pub(super) fn callable(&self, o: &Observation) -> Option<Node<'_>> {
        let name = o.normalized["name"].as_str().unwrap_or_else(|| {
            o.symbol
                .split('#')
                .nth(1)
                .unwrap_or("")
                .split('(')
                .next()
                .unwrap_or("")
        });
        let mut candidates = Vec::new();
        for kind in [
            "method_declaration",
            "constructor_declaration",
            "compact_constructor_declaration",
        ] {
            for node in descendants(self.tree.root_node(), kind) {
                if node
                    .child_by_field_name("name")
                    .is_some_and(|n| self.text(n) == name || name == "<init>")
                {
                    candidates.push(node);
                }
            }
        }
        match candidates.as_slice() {
            [node] => Some(*node),
            _ => None,
        }
    }
}

pub(super) struct Context<'a> {
    pub(super) evidence: &'a ServiceEvidence,
    pub(super) citations: BTreeMap<String, Citation>,
    pub(super) observations: BTreeMap<String, Observation>,
    pub(super) sources: BTreeMap<String, Source>,
}

impl Context<'_> {
    fn handoff(
        &mut self,
        endpoint: &CallableProjection,
        worker: &CallableProjection,
        wiring: Option<&CallableProjection>,
    ) -> HandoffProjection {
        let mut result = HandoffProjection { status: "LOCAL_GAP".into(), queue_allocation: None, endpoint_field: None, worker_field: None, citation_ids: vec![], gaps: vec![], limitation: "Source-declared shared object wiring does not prove enqueue success, scheduling, deployed activation, delivery, or completion.".into() };
        let proof = self.handoff_proof(endpoint, worker, wiring);
        match proof {
            Ok((allocation, endpoint_field, worker_field, citations)) => {
                result.status = "SOURCE_DECLARED_SHARED_QUEUE".into();
                result.queue_allocation = Some(allocation);
                result.endpoint_field = Some(endpoint_field);
                result.worker_field = Some(worker_field);
                result.citation_ids = citations;
            }
            Err(reason) => result.gaps.push(reason),
        }
        result
    }

    fn handoff_proof(
        &mut self,
        endpoint: &CallableProjection,
        worker: &CallableProjection,
        wiring: Option<&CallableProjection>,
    ) -> Result<(String, String, String, Vec<String>), Gap> {
        let wiring = wiring.ok_or_else(|| gap("WIRING_NOT_SELECTED", "No explicit callable wiring declaration was selected; equal queue names and types do not establish object identity.", None))?;
        for callable in [endpoint, worker, wiring] {
            if callable.authority != "COMPILER_DECLARATION" {
                return Err(gap(
                    "WIRING_COMPILER_UNAVAILABLE",
                    "All three selected callable declarations require applicable compiler evidence.",
                    callable.citation_id.clone(),
                ));
            }
        }
        let wire_rows = all_steps(&wiring.steps);
        if wire_rows.iter().any(|s| {
            !s.reachable
                || !s.conditions.is_empty()
                || matches!(
                    s.kind,
                    StatementKind::If
                        | StatementKind::Unsupported
                        | StatementKind::Return
                        | StatementKind::Throw
                )
        }) {
            return Err(gap(
                "WIRING_CONTROL_AMBIGUOUS",
                "Conditional, interrupted, or unsupported wiring cannot establish one shared object allocation.",
                wiring.citation_id.clone(),
            ));
        }
        let queue_call = |callable: &CallableProjection,
                          methods: &[&str]|
         -> Result<CallProjection, Gap> {
            let calls: Vec<_> = all_steps(&callable.steps)
                .into_iter()
                .filter(|s| s.reachable)
                .flat_map(|s| &s.calls)
                .filter(|c| {
                    methods.contains(&c.name.as_str())
                        && c.target.as_ref().is_some_and(|t| queue_target(t))
                })
                .cloned()
                .collect();
            match calls.as_slice() {
                [call] => Ok(call.clone()),
                _ => Err(gap(
                    "QUEUE_OPERATION_UNAVAILABLE_OR_AMBIGUOUS",
                    "Select a callable with one exact compiler-bound BlockingQueue submission or consumption occurrence.",
                    callable.citation_id.clone(),
                )),
            }
        };
        let submission = queue_call(endpoint, &["offer", "put", "add"])?;
        let consumption = queue_call(worker, &["poll", "take"])?;
        let endpoint_field = self.receiver_field(endpoint, &submission)?;
        let worker_field = self.receiver_field(worker, &consumption)?;
        let e = self
            .evidence
            .observations
            .get(&endpoint.declaration_id)
            .unwrap()
            .clone();
        let w = self
            .evidence
            .observations
            .get(&worker.declaration_id)
            .unwrap()
            .clone();
        let wiring_obs = self
            .evidence
            .observations
            .get(&wiring.declaration_id)
            .unwrap()
            .clone();
        if e.normalized["scope"] != w.normalized["scope"]
            || e.normalized["scope"] != wiring_obs.normalized["scope"]
        {
            return Err(gap(
                "WIRING_SCOPE_MISMATCH",
                "Selected endpoint, worker and wiring belong to different compiler scopes.",
                wiring.citation_id.clone(),
            ));
        }
        let mut aliases: BTreeMap<String, String> = BTreeMap::new();
        let mut allocation_citations = BTreeMap::new();
        let mut bindings: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
        for row in wire_rows {
            for call in &row.calls {
                if call.phase != "CREATION" || call.authority != "COMPILER_EXACT_CALL_RELATION" {
                    continue;
                }
                let target = call.target.as_ref().unwrap();
                let candidate_owners = [
                    (
                        e.normalized["ownerIdentity"].as_str().unwrap_or(""),
                        &endpoint_field,
                    ),
                    (
                        w.normalized["ownerIdentity"].as_str().unwrap_or(""),
                        &worker_field,
                    ),
                ];
                for (owner, field) in candidate_owners {
                    let constructors: Vec<_> = self
                        .evidence
                        .observations
                        .values()
                        .filter(|o| {
                            compiler(o)
                                && o.normalized["declarationKind"] == "CONSTRUCTOR"
                                && o.symbol == *target
                                && o.normalized["ownerIdentity"] == owner
                                && o.normalized["scope"] == wiring_obs.normalized["scope"]
                        })
                        .cloned()
                        .collect();
                    if let [constructor] = constructors.as_slice() {
                        let (position, proof_citation) =
                            self.constructor_field_parameter(constructor, field)?;
                        let argument = call.arguments.get(position).ok_or_else(|| {
                            gap(
                                "CONSTRUCTOR_ARGUMENT_UNAVAILABLE",
                                "Selected construction lacks the queue parameter argument.",
                                Some(call.citation_id.clone()),
                            )
                        })?;
                        if let Some(allocation) = aliases.get(argument.trim()) {
                            bindings.entry(owner.into()).or_default().push((
                                allocation.clone(),
                                call.citation_id.clone(),
                                proof_citation,
                            ));
                        } else {
                            return Err(gap(
                                "QUEUE_ARGUMENT_ALIAS_UNPROVEN",
                                format!(
                                    "Constructor queue argument {argument} is not a bound local allocation or alias."
                                ),
                                Some(call.citation_id.clone()),
                            ));
                        }
                    }
                }
            }
            for local in wiring.state.iter().filter(|s| {
                s.citation_id == row.citation_id
                    && matches!(s.kind.as_str(), "LOCAL_DECLARATION" | "ASSIGNMENT")
            }) {
                if !identifier(&local.name) {
                    continue;
                }
                let allocation = row.calls.iter().find(|c| {
                    c.expression == local.expression
                        && c.phase == "CREATION"
                        && c.target.as_ref().is_some_and(|t| {
                            [
                                "LinkedBlockingQueue",
                                "ArrayBlockingQueue",
                                "PriorityBlockingQueue",
                                "SynchronousQueue",
                                "LinkedTransferQueue",
                                "DelayQueue",
                            ]
                            .iter()
                            .any(|name| t.contains(&format!("java.util.concurrent.{name}#")))
                        })
                });
                if let Some(call) = allocation {
                    let id = format!("{}:{}", wiring.declaration_id, call.citation_id);
                    aliases.insert(local.name.clone(), id.clone());
                    allocation_citations.insert(id, call.citation_id.clone());
                } else if let Some(bound) = aliases.get(local.expression.trim()).cloned() {
                    aliases.insert(local.name.clone(), bound);
                } else {
                    aliases.remove(&local.name);
                }
            }
        }
        let selected = |owner: &str| -> Option<(String, String, String)> {
            match bindings.get(owner)?.as_slice() {
                [binding] => Some(binding.clone()),
                _ => None,
            }
        };
        let eb = selected(e.normalized["ownerIdentity"].as_str().unwrap_or(""));
        let wb = selected(w.normalized["ownerIdentity"].as_str().unwrap_or(""));
        match (eb, wb) {
            (Some(eb), Some(wb)) if eb.0 == wb.0 => {
                let mut citations = vec![
                    submission.citation_id,
                    consumption.citation_id,
                    eb.1,
                    eb.2,
                    wb.1,
                    wb.2,
                ];
                if let Some(citation) = allocation_citations.get(&eb.0) {
                    citations.push(citation.clone());
                }
                citations.sort();
                citations.dedup();
                Ok((eb.0, endpoint_field, worker_field, citations))
            }
            _ => Err(gap(
                "SHARED_QUEUE_OBJECT_UNPROVEN",
                "Selected constructions do not bind exactly one endpoint and worker to the same local queue allocation.",
                wiring.citation_id.clone(),
            )),
        }
    }

    fn receiver_field(
        &mut self,
        callable: &CallableProjection,
        call: &CallProjection,
    ) -> Result<String, Gap> {
        let receiver = call.receiver.as_deref().unwrap_or("");
        let field = receiver.strip_prefix("this.").unwrap_or(receiver);
        if !identifier(field) {
            return Err(gap(
                "QUEUE_RECEIVER_UNSUPPORTED",
                "Queue operation receiver is not a direct field.",
                Some(call.citation_id.clone()),
            ));
        }
        let owner = self
            .evidence
            .observations
            .get(&callable.declaration_id)
            .unwrap()
            .clone();
        if receiver == field {
            let source = owner
                .source_ids
                .first()
                .and_then(|id| self.evidence.sources.get(id));
            let shadowed = source.and_then(|s| Parsed::new(&s.text)).is_some_and(|p| {
                p.callable(&owner).is_some_and(|n| {
                    descendants(n, "formal_parameter").iter().any(|pnode| {
                        pnode
                            .child_by_field_name("name")
                            .is_some_and(|name| p.text(name) == field)
                    })
                })
            });
            if shadowed
                || callable
                    .state
                    .iter()
                    .any(|s| s.kind == "LOCAL_DECLARATION" && s.name == field)
            {
                return Err(gap(
                    "QUEUE_RECEIVER_SHADOWED",
                    "Queue receiver is shadowed by a local variable or parameter.",
                    Some(call.citation_id.clone()),
                ));
            }
        }
        if callable.state.iter().any(|s| {
            s.kind == "ASSIGNMENT" && (s.name == receiver || s.name == format!("this.{field}"))
        }) {
            return Err(gap(
                "QUEUE_FIELD_REASSIGNED",
                "Selected callable reassigns its queue field; constructor object identity is insufficient.",
                Some(call.citation_id.clone()),
            ));
        }
        let fields: Vec<_> = self
            .evidence
            .observations
            .values()
            .filter(|o| {
                compiler(o)
                    && o.normalized["declarationKind"] == "FIELD"
                    && o.normalized["ownerIdentity"] == owner.normalized["ownerIdentity"]
                    && o.normalized["scope"] == owner.normalized["scope"]
                    && o.normalized["name"] == field
            })
            .cloned()
            .collect();
        match fields.as_slice() {
            [o] if blocking_queue_descriptor(
                o.normalized["jvmDescriptor"].as_str().unwrap_or(""),
            ) =>
            {
                self.retain(o);
                Ok(field.into())
            }
            _ => Err(gap(
                "QUEUE_FIELD_DECLARATION_UNAVAILABLE",
                "Queue receiver needs one exact compiler BlockingQueue field declaration in the selected owner and scope.",
                Some(call.citation_id.clone()),
            )),
        }
    }

    fn constructor_field_parameter(
        &mut self,
        constructor: &Observation,
        field: &str,
    ) -> Result<(usize, String), Gap> {
        let projection = self
            .callable(&constructor.id)
            .map_err(|e| gap("CONSTRUCTOR_SOURCE_UNAVAILABLE", e.message, None))?;
        let rows = all_steps(&projection.steps);
        if rows.iter().any(|r| {
            !r.conditions.is_empty()
                || !r.calls.is_empty()
                || !r.gaps.is_empty()
                || !matches!(
                    r.kind,
                    StatementKind::Assignment
                        | StatementKind::Declaration
                        | StatementKind::Expression
                )
        }) {
            return Err(gap(
                "CONSTRUCTOR_BINDING_UNSUPPORTED",
                "Constructor binding requires straight-line source assignments without helper calls or unsupported control.",
                projection.citation_id,
            ));
        }
        let assignments: Vec<_> = projection
            .state
            .iter()
            .filter(|s| s.kind == "ASSIGNMENT" && s.name == format!("this.{field}"))
            .collect();
        let [assignment] = assignments.as_slice() else {
            return Err(gap(
                "CONSTRUCTOR_FIELD_BINDING_AMBIGUOUS",
                "Constructor must assign the queue field exactly once directly from a formal parameter.",
                projection.citation_id,
            ));
        };
        let source = match constructor.source_ids.as_slice() {
            [id] => self
                .evidence
                .sources
                .get(id)
                .filter(|s| self.valid_source(s)),
            _ => None,
        }
        .ok_or_else(|| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Constructor source is unavailable or invalid.",
                projection.citation_id.clone(),
            )
        })?;
        let parsed = Parsed::new(&source.text).ok_or_else(|| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Constructor source cannot be parsed.",
                projection.citation_id.clone(),
            )
        })?;
        let callable = parsed.callable(constructor).ok_or_else(|| {
            gap(
                "CONSTRUCTOR_SOURCE_UNAVAILABLE",
                "Constructor source is ambiguous.",
                projection.citation_id.clone(),
            )
        })?;
        let parameters = callable
            .child_by_field_name("parameters")
            .map(named)
            .unwrap_or_default();
        let matches: Vec<_> = parameters
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.child_by_field_name("name")
                    .is_some_and(|name| parsed.text(name) == assignment.expression)
            })
            .collect();
        match matches.as_slice() {
            [(index, _)] => {
                if projection
                    .state
                    .iter()
                    .any(|s| s.kind == "ASSIGNMENT" && s.name == assignment.expression)
                {
                    return Err(gap(
                        "CONSTRUCTOR_PARAMETER_REASSIGNED",
                        "The queue formal parameter is reassigned in the constructor; the incoming allocation no longer proves the field binding.",
                        Some(assignment.citation_id.clone()),
                    ));
                }
                Ok((*index, assignment.citation_id.clone()))
            }
            _ => Err(gap(
                "CONSTRUCTOR_PARAMETER_UNPROVEN",
                "Queue field assignment does not bind one formal parameter directly.",
                Some(assignment.citation_id.clone()),
            )),
        }
    }

    fn retain(&mut self, o: &Observation) {
        self.observations.insert(o.id.clone(), o.clone());
        for id in &o.source_ids {
            if let Some(source) = self.evidence.sources.get(id) {
                self.sources.insert(id.clone(), source.clone());
            }
        }
    }
    fn valid_source(&self, source: &Source) -> bool {
        source.service == self.evidence.service
            && source.revision == self.evidence.revision
            && !source.authority.is_empty()
            && !source.evidence_digest.is_empty()
            && !source.text.is_empty()
            && source.start_line > 0
            && source.end_line >= source.start_line
            && source.end_line - source.start_line + 1 == source.text.lines().count() as u64
            && source
                .occurrence
                .as_ref()
                .is_none_or(|o| o.end_byte.checked_sub(o.start_byte) == Some(source.text.len()))
            && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
    }
    fn citation(&mut self, source: &Source, start: usize, end: usize) -> String {
        let start = start.min(source.text.len());
        let end = end.min(source.text.len()).max(start);
        let id = format!(
            "citation-{}",
            &crate::canonical::hash_bytes(
                format!("{}:{start}:{end}:{}", source.id, source.text_digest).as_bytes()
            )[7..31]
        );
        let start_line =
            source.start_line + source.text[..start].bytes().filter(|b| *b == b'\n').count() as u64;
        let end_line = source.start_line
            + source.text.as_bytes()[..end.saturating_sub(1).max(start)]
                .iter()
                .filter(|b| **b == b'\n')
                .count() as u64;
        let url = source.url.as_ref().map(|u| {
            format!(
                "{}#L{start_line}-L{end_line}",
                u.split('#').next().unwrap_or(u)
            )
        });
        self.sources.insert(source.id.clone(), source.clone());
        self.citations.entry(id.clone()).or_insert(Citation {
            id: id.clone(),
            source_id: source.id.clone(),
            service: source.service.clone(),
            revision: source.revision.clone(),
            file: source.file.clone(),
            start_line,
            end_line,
            start_byte: start,
            end_byte: end,
            text_digest: crate::canonical::hash_bytes(&source.text.as_bytes()[start..end]),
            evidence_digest: source.evidence_digest.clone(),
            authority: source.authority.clone(),
            url,
        });
        id
    }
    pub(super) fn callable(&mut self, id: &str) -> Result<CallableProjection, ClewError> {
        let o = self
            .evidence
            .observations
            .get(id)
            .ok_or_else(|| invalid(format!("selected callable declaration {id} is missing")))?
            .clone();
        if o.kind != "SYMBOL"
            || (!matches!(
                o.normalized["declarationKind"].as_str(),
                Some("METHOD" | "CONSTRUCTOR")
            ) && o.normalized["syntaxKind"] != "method_declaration"
                && o.normalized["syntaxKind"] != "constructor_declaration")
        {
            return Err(invalid(format!(
                "selected declaration {id} is not a Java callable"
            )));
        }
        self.retain(&o);
        // Keep selected-body provider facts even inside unsupported control.
        let outgoing: Vec<_> = self
            .evidence
            .observations
            .values()
            .filter(|f| {
                matches!(f.kind.as_str(), "FLOW" | "CALL_RELATION")
                    && (f.symbol == o.symbol || f.normalized["sourceIdentity"] == o.symbol)
                    && f.normalized["scope"] == o.normalized["scope"]
            })
            .cloned()
            .collect();
        for fact in outgoing {
            self.retain(&fact);
            if fact.kind == "CALL_RELATION"
                && fact.normalized["resolution"] == "COMPILER_EXACT"
                && let Some(target) = fact.normalized["targetIdentity"].as_str()
            {
                let related: Vec<_> = self
                    .evidence
                    .observations
                    .values()
                    .filter(|d| {
                        matches!(d.kind.as_str(), "SYMBOL" | "DEPENDENCY_TARGET")
                            && d.normalized["symbolIdentity"] == target
                            && d.normalized["scope"] == o.normalized["scope"]
                    })
                    .cloned()
                    .collect();
                for declaration in related {
                    self.retain(&declaration);
                }
            }
        }
        let mut out = CallableProjection {
            declaration_id: o.id.clone(),
            symbol: o.symbol.clone(),
            authority: if compiler(&o) {
                "COMPILER_DECLARATION"
            } else {
                "SYNTAX_SOURCE"
            }
            .into(),
            citation_id: None,
            steps: vec![],
            state: vec![],
            gaps: vec![],
        };
        let source = match o.source_ids.as_slice() {
            [id] => self
                .evidence
                .sources
                .get(id)
                .filter(|s| self.valid_source(s))
                .cloned(),
            _ => None,
        };
        let Some(source) = source else {
            out.gaps.push(gap(
                "CALLABLE_SOURCE_UNAVAILABLE",
                "Selected declaration lacks one valid retained source body.",
                None,
            ));
            return Ok(out);
        };
        out.citation_id = Some(self.citation(&source, 0, source.text.len()));
        let Some(parsed) = Parsed::new(&source.text) else {
            out.gaps.push(gap(
                "JAVA_PARSE_UNAVAILABLE",
                "Pinned Java syntax parser could not parse retained source.",
                out.citation_id.clone(),
            ));
            return Ok(out);
        };
        let Some(callable) = parsed.callable(&o) else {
            out.gaps.push(gap(
                "CALLABLE_BODY_AMBIGUOUS",
                "Retained source does not contain exactly one selected named callable.",
                out.citation_id.clone(),
            ));
            return Ok(out);
        };
        if callable.has_error() {
            out.gaps.push(gap("CALLABLE_SYNTAX_PARTIAL", "Selected Java callable contains syntax errors; no unconditional statement projection is available.", out.citation_id.clone()));
            return Ok(out);
        }
        if let Some(body) = callable.child_by_field_name("body") {
            let (steps, _) = self.block(&parsed, body, &source, &o, &[], true, &mut out.state);
            out.steps = steps;
        } else {
            out.gaps.push(gap(
                "CALLABLE_BODY_UNAVAILABLE",
                "Callable has no source body.",
                out.citation_id.clone(),
            ));
        }
        // Field declarations are evidence, not inferred values or runtime state.
        let fields: Vec<_> = self
            .evidence
            .observations
            .values()
            .filter(|f| {
                f.kind == "SYMBOL"
                    && f.normalized["declarationKind"] == "FIELD"
                    && f.normalized["ownerIdentity"] == o.normalized["ownerIdentity"]
                    && f.normalized["scope"] == o.normalized["scope"]
            })
            .cloned()
            .collect();
        for field in fields {
            self.retain(&field);
            for id in &field.source_ids {
                if let Some(s) = self
                    .evidence
                    .sources
                    .get(id)
                    .filter(|s| self.valid_source(s))
                    .cloned()
                {
                    let citation_id = self.citation(&s, 0, s.text.len());
                    out.state.push(StateRow {
                        name: field.normalized["name"]
                            .as_str()
                            .unwrap_or(&field.symbol)
                            .into(),
                        expression: s.text.clone(),
                        kind: "FIELD_DECLARATION".into(),
                        conditions: vec![],
                        citation_id,
                    });
                }
            }
        }
        if !compiler(&o) {
            out.gaps.push(gap("COMPILER_DECLARATION_UNAVAILABLE", "Source syntax remains readable; exact call targets and object wiring require applicable compiler declarations and relations.", out.citation_id.clone()));
        }
        Ok(out)
    }

    /// `continues` is conservative structural reachability, never runtime truth.
    // Explicit source provenance and inherited path inputs stay visible at the recursion boundary.
    #[allow(clippy::too_many_arguments)]
    fn block(
        &mut self,
        parsed: &Parsed,
        node: Node<'_>,
        source: &Source,
        owner: &Observation,
        inherited: &[PathCondition],
        reachable: bool,
        state: &mut Vec<StateRow>,
    ) -> (Vec<Statement>, bool) {
        let nodes = if matches!(node.kind(), "block" | "constructor_body") {
            named(node)
        } else {
            vec![node]
        };
        let mut rows = Vec::new();
        let mut conditions = inherited.to_vec();
        let mut continues = reachable;
        for node in nodes {
            if matches!(node.kind(), "line_comment" | "block_comment") {
                continue;
            }
            let (row, next, guard) =
                self.statement(parsed, node, source, owner, &conditions, continues, state);
            rows.push(row);
            continues = next;
            conditions.extend(guard);
        }
        (rows, continues)
    }

    // Keep immutable source/owner evidence separate from path state and the state-row output.
    #[allow(clippy::too_many_arguments)]
    fn statement(
        &mut self,
        parsed: &Parsed,
        node: Node<'_>,
        source: &Source,
        owner: &Observation,
        conditions: &[PathCondition],
        reachable: bool,
        state: &mut Vec<StateRow>,
    ) -> (Statement, bool, Vec<PathCondition>) {
        let (start, end) = parsed.range(node);
        let citation = self.citation(source, start, end);
        let mut row = Statement {
            id: format!("step-{citation}"),
            kind: StatementKind::Expression,
            expression: parsed.text(node),
            citation_id: citation.clone(),
            conditions: conditions.to_vec(),
            reachable,
            children: vec![],
            alternative: vec![],
            calls: vec![],
            gaps: vec![],
        };
        let mut continues = reachable;
        let mut guard = Vec::new();
        match node.kind() {
            "if_statement" => {
                row.kind = StatementKind::If;
                let condition = node.child_by_field_name("condition").unwrap();
                row.expression = parsed.text(condition);
                let (cs, ce) = parsed.range(condition);
                let cond = PathCondition {
                    expression: row.expression.clone(),
                    holds: true,
                    citation_id: self.citation(source, cs, ce),
                };
                let mut yes = conditions.to_vec();
                yes.push(cond.clone());
                let mut no = conditions.to_vec();
                no.push(PathCondition {
                    holds: false,
                    ..cond.clone()
                });
                let then = node.child_by_field_name("consequence").unwrap();
                let (children, a) = self.block(parsed, then, source, owner, &yes, reachable, state);
                row.children = children;
                let b = if let Some(other) = node.child_by_field_name("alternative") {
                    let (alternative, b) =
                        self.block(parsed, other, source, owner, &no, reachable, state);
                    row.alternative = alternative;
                    b
                } else {
                    reachable
                };
                continues = a || b;
                if reachable {
                    let then_guards = continuation_guards(&row.children);
                    let else_guards = continuation_guards(&row.alternative);
                    match (a, b, then_guards, else_guards) {
                        (true, false, Ok(Some(extra)), _) => {
                            guard.push(PathCondition {
                                holds: true,
                                ..cond.clone()
                            });
                            guard.extend(extra);
                        }
                        (false, true, _, Ok(Some(extra))) => {
                            guard.push(PathCondition {
                                holds: false,
                                ..cond.clone()
                            });
                            guard.extend(extra);
                        }
                        (true, true, Ok(Some(a)), Ok(Some(b))) if a.is_empty() && b.is_empty() => {}
                        (false, false, _, _) => {}
                        _ => {
                            row.gaps.push(gap("CONTINUATION_PATH_PARTIAL", "Nested continuing alternatives require a path disjunction; later calls retain an opaque continuation condition rather than an incomplete path.", Some(citation.clone())));
                            guard.push(PathCondition { expression: format!("retained nested alternatives of {} continue normally (path disjunction unresolved)", row.expression), holds: true, citation_id: citation.clone() });
                        }
                    }
                }
                row.calls = self.calls(parsed, condition, source, owner);
            }
            "return_statement" => {
                row.kind = StatementKind::Return;
                continues = false;
                row.calls = self.calls(parsed, node, source, owner);
            }
            "throw_statement" => {
                row.kind = StatementKind::Throw;
                continues = false;
                row.calls = self.calls(parsed, node, source, owner);
            }
            "local_variable_declaration" => {
                row.kind = StatementKind::Declaration;
                for variable in named(node)
                    .into_iter()
                    .filter(|n| n.kind() == "variable_declarator")
                {
                    if let Some(name) = variable.child_by_field_name("name") {
                        state.push(StateRow {
                            name: parsed.text(name),
                            expression: variable
                                .child_by_field_name("value")
                                .map(|v| parsed.text(v))
                                .unwrap_or_default(),
                            kind: "LOCAL_DECLARATION".into(),
                            conditions: conditions.to_vec(),
                            citation_id: citation.clone(),
                        });
                    }
                }
                row.calls = self.calls(parsed, node, source, owner);
            }
            "expression_statement" => {
                for assignment in active_nodes(parsed, node, "assignment_expression") {
                    row.kind = StatementKind::Assignment;
                    if let (Some(left), Some(right)) = (
                        assignment.child_by_field_name("left"),
                        assignment.child_by_field_name("right"),
                    ) {
                        let operator = &parsed.text[left.end_byte()..right.start_byte()];
                        let simple = operator.trim() == "=";
                        state.push(StateRow {
                            name: parsed.text(left),
                            expression: if simple {
                                parsed.text(right)
                            } else {
                                parsed.text(assignment)
                            },
                            kind: if simple {
                                "ASSIGNMENT"
                            } else {
                                "UNSUPPORTED_MUTATION"
                            }
                            .into(),
                            conditions: conditions.to_vec(),
                            citation_id: citation.clone(),
                        });
                        if !simple {
                            row.gaps.push(gap("COMPOUND_ASSIGNMENT_UNSUPPORTED", "Compound assignment is preserved as its full source expression; no replacement-value inference is made.", Some(citation.clone())));
                        }
                    }
                }
                row.calls = self.calls(parsed, node, source, owner);
            }
            "block" => {
                let (children, next) =
                    self.block(parsed, node, source, owner, conditions, reachable, state);
                row.children = children;
                continues = next;
            }
            _ => {
                row.kind = StatementKind::Unsupported;
                row.gaps.push(gap("UNSUPPORTED_CONTROL", format!("{} is retained as source; its control and calls are not projected as unconditional facts.", node.kind()), Some(citation.clone())));
                // Later statements remain conditional on unresolved completion.
                guard.push(PathCondition {
                    expression: format!("unsupported {} completes normally", node.kind()),
                    holds: true,
                    citation_id: citation.clone(),
                });
            }
        }
        let expression_node = if row.kind == StatementKind::If {
            node.child_by_field_name("condition").unwrap_or(node)
        } else {
            node
        };
        if !matches!(row.kind, StatementKind::Unsupported) {
            for kind in [
                "lambda_expression",
                "ternary_expression",
                "switch_expression",
                "class_body",
            ] {
                if !descendants(expression_node, kind).is_empty() {
                    row.gaps.push(gap("UNSUPPORTED_EXPRESSION", format!("{kind} is retained; conditional or deferred expression effects are not projected."), Some(citation.clone())));
                }
            }
        }
        if !descendants(node, "update_expression").is_empty() {
            row.gaps.push(gap("UPDATE_EXPRESSION_UNSUPPORTED", "Increment/decrement is retained in the statement expression without inferred state values.", Some(citation.clone())));
        }
        if descendants(node, "binary_expression")
            .iter()
            .any(|n| short_circuit(parsed, *n))
        {
            row.gaps.push(gap("SHORT_CIRCUIT_CALLS_CONDITIONAL", "Calls in the right operand of && or || depend on the left operand and remain in source/provider evidence without unconditional call projection.", Some(citation)));
        }
        (row, continues, guard)
    }

    fn calls(
        &mut self,
        parsed: &Parsed,
        node: Node<'_>,
        source: &Source,
        owner: &Observation,
    ) -> Vec<CallProjection> {
        let mut calls = Vec::new();
        self.collect_calls(parsed, node, source, owner, &mut calls);
        calls
    }
    fn collect_calls(
        &mut self,
        parsed: &Parsed,
        node: Node<'_>,
        source: &Source,
        owner: &Observation,
        out: &mut Vec<CallProjection>,
    ) {
        if matches!(
            node.kind(),
            "lambda_expression" | "class_body" | "ternary_expression" | "switch_expression"
        ) {
            return;
        }
        if short_circuit(parsed, node) {
            if let Some(left) = node.child_by_field_name("left") {
                self.collect_calls(parsed, left, source, owner, out);
            }
            return;
        }
        // Java evaluates nested receiver/argument expressions before the call.
        for child in named(node) {
            self.collect_calls(parsed, child, source, owner, out);
        }
        if !matches!(
            node.kind(),
            "method_invocation" | "object_creation_expression"
        ) {
            return;
        }
        let construct = node.kind() == "object_creation_expression";
        let name = node
            .child_by_field_name(if construct { "type" } else { "name" })
            .map(|n| parsed.text(n))
            .unwrap_or_default();
        let receiver = node.child_by_field_name("object").map(|n| parsed.text(n));
        let arguments = node
            .child_by_field_name("arguments")
            .map(named)
            .unwrap_or_default()
            .into_iter()
            .map(|n| parsed.text(n))
            .collect();
        let (start, end) = parsed.range(node);
        let citation_id = self.citation(source, start, end);
        let expression = parsed.text(node);
        let relations: Vec<_> = self
            .evidence
            .observations
            .values()
            .filter(|r| {
                compiler(owner)
                    && r.kind == "CALL_RELATION"
                    && r.normalized["sourceIdentity"] == owner.symbol
                    && r.normalized["scope"] == owner.normalized["scope"]
                    && r.normalized["resolution"] == "COMPILER_EXACT"
                    && r.normalized["relationKind"]
                        == if construct { "CONSTRUCTS" } else { "CALLS" }
                    && target_name_matches(r, &name, construct)
                    && self.call_bound(r, source, start, end, &expression)
            })
            .cloned()
            .collect();
        let mut call = CallProjection {
            expression,
            receiver,
            name,
            arguments,
            target: None,
            authority: "SYNTAX_SOURCE".into(),
            relation_id: None,
            citation_id: citation_id.clone(),
            phase: if construct {
                "CREATION"
            } else {
                "CALL_ATTEMPT"
            }
            .into(),
            external_boundary: None,
            gaps: vec![],
            expanded_node: None,
        };
        match relations.as_slice() {
            [relation] => {
                if let Some(target) = relation.normalized["targetIdentity"].as_str().filter(|s| !s.is_empty()) {
                    self.retain(relation); call.target = Some(target.into()); call.authority = "COMPILER_EXACT_CALL_RELATION".into(); call.relation_id = Some(relation.id.clone());
                    let dependencies: Vec<_> = self.evidence.observations.values().filter(|d| d.kind == "DEPENDENCY_TARGET" && d.normalized["schema"] == JAVA_SCHEMA && d.normalized["resolution"] == "COMPILER_EXACT" && d.normalized["symbolIdentity"] == target && d.normalized["scope"] == owner.normalized["scope"]).cloned().collect();
                    if let [dependency] = dependencies.as_slice() {
                        self.retain(dependency);
                        let retained_sources: Vec<_> = dependency.source_ids.iter().filter_map(|id| self.evidence.sources.get(id).filter(|s| self.valid_source(s)).cloned()).collect();
                        let citation_ids = retained_sources.iter().map(|s| self.citation(s, 0, s.text.len())).collect();
                        call.external_boundary = Some(ExternalBoundary { target: target.into(), dependency_id: dependency.id.clone(), source_status: dependency.normalized["sourceStatus"].as_str().unwrap_or("SOURCE_UNAVAILABLE").into(), citation_ids, limitation: "DEP-01 identifies the admitted dependency target and source availability. Invocation, response, implementation choice and success are not observed.".into() });
                        if !construct { call.phase = "DEPENDENCY_CALL_ATTEMPT".into(); }
                    } else if dependencies.len() > 1 { call.gaps.push(gap("DEPENDENCY_TARGET_AMBIGUOUS", "Multiple dependency targets match the exact call scope.", Some(citation_id.clone()))); }
                    else if !self.evidence.observations.values().any(|d| compiler(d) && d.symbol == target && d.normalized["scope"] == owner.normalized["scope"]) {
                        call.gaps.push(gap("TARGET_METADATA_UNAVAILABLE", "Exact call target has neither a retained repository declaration nor DEP-01 metadata; external source status is unknown.", Some(citation_id.clone())));
                    } else if !construct {
                        call.gaps.push(gap("HELPER_BODY_NOT_EXPANDED", "The repository call target is retained, but this selected-body projection does not establish its internal effects or completion.", Some(citation_id.clone())));
                    }
                }
            }
            [] => call.gaps.push(gap("EXACT_CALL_UNAVAILABLE", "Readable source call has no unique source-bound compiler relation in the selected declaration scope.", Some(citation_id.clone()))),
            _ => call.gaps.push(gap("EXACT_CALL_AMBIGUOUS", "More than one source-bound compiler relation matches this occurrence.", Some(citation_id.clone()))),
        }
        if !construct && call.target.as_ref().is_some_and(|t| queue_target(t)) {
            if ["offer", "put", "add"].contains(&call.name.as_str()) {
                call.phase = "SUBMISSION_ATTEMPT".into();
            }
            if ["poll", "take"].contains(&call.name.as_str()) {
                call.phase = "CONSUMPTION_ATTEMPT".into();
            }
        }
        out.push(call);
    }

    fn call_bound(
        &self,
        relation: &Observation,
        body: &Source,
        start: usize,
        end: usize,
        expression: &str,
    ) -> bool {
        let site = &relation.normalized["callSite"];
        let [id] = relation.source_ids.as_slice() else {
            return false;
        };
        let Some(source) = self.evidence.sources.get(id) else {
            return false;
        };
        if !self.valid_source(source)
            || site["sourceId"] != *id
            || site["sourceStatus"] != "SOURCE_RETAINED"
            || site["sourceDigest"] != source.text_digest
            || site["evidenceDigest"] != source.evidence_digest
            || source.file != body.file
        {
            return false;
        }
        if let (Some(body_range), Some(site_range)) = (&body.occurrence, &source.occurrence) {
            return body_range.snapshot == site_range.snapshot
                && body_range.blob == site_range.blob
                && body_range.start_byte + start == site_range.start_byte
                && body_range.start_byte + end == site_range.end_byte;
        }
        let first =
            body.start_line + body.text[..start].bytes().filter(|b| *b == b'\n').count() as u64;
        let last = body.start_line
            + body.text.as_bytes()[..end.saturating_sub(1)]
                .iter()
                .filter(|b| **b == b'\n')
                .count() as u64;
        // Compiler retention may be line-based. Admit only a unique identical
        // expression within that exact line range; duplicate same-line calls
        // stay ambiguous rather than borrowing the wrong target.
        site["byteStart"]
            .as_u64()
            .zip(site["byteEnd"].as_u64())
            .is_none_or(|(a, b)| b.checked_sub(a) == Some(expression.len() as u64))
            && source.start_line == first
            && source.end_line == last
            && source.text.match_indices(expression).count() == 1
            && body
                .text
                .lines()
                .skip((first - body.start_line) as usize)
                .take((last - first + 1) as usize)
                .collect::<Vec<_>>()
                .join("\n")
                .match_indices(expression)
                .count()
                == 1
    }
}

fn all_steps(steps: &[Statement]) -> Vec<&Statement> {
    let mut result = Vec::new();
    for step in steps {
        result.push(step);
        result.extend(all_steps(&step.children));
        result.extend(all_steps(&step.alternative));
    }
    result
}

/// A conjunction for the sole continuing path, or an explicit unresolved
/// disjunction. This includes guards at the end of nested blocks, where no
/// following statement exists to carry their conditions yet.
fn continuation_guards(steps: &[Statement]) -> Result<Option<Vec<PathCondition>>, ()> {
    let mut guards = Vec::new();
    for row in steps {
        match row.kind {
            StatementKind::Return | StatementKind::Throw => return Ok(None),
            StatementKind::If => {
                let yes = continuation_guards(&row.children)?;
                let no = continuation_guards(&row.alternative)?;
                let citation_id = row
                    .children
                    .iter()
                    .chain(&row.alternative)
                    .flat_map(|s| &s.conditions)
                    .find(|c| c.expression == row.expression)
                    .map(|c| c.citation_id.clone())
                    .unwrap_or_else(|| row.citation_id.clone());
                match (yes, no) {
                    (None, None) => return Ok(None),
                    (Some(extra), None) | (None, Some(extra)) => {
                        let holds = continuation_guards(&row.children)?.is_some();
                        guards.push(PathCondition {
                            expression: row.expression.clone(),
                            holds,
                            citation_id,
                        });
                        guards.extend(extra);
                    }
                    (Some(a), Some(b)) if a.is_empty() && b.is_empty() => {}
                    _ => return Err(()),
                }
            }
            StatementKind::Unsupported => guards.push(PathCondition {
                expression: format!("unsupported statement {} completes normally", row.id),
                holds: true,
                citation_id: row.citation_id.clone(),
            }),
            _ => {
                if !row.children.is_empty() {
                    match continuation_guards(&row.children)? {
                        Some(extra) => guards.extend(extra),
                        None => return Ok(None),
                    }
                }
            }
        }
    }
    Ok(Some(guards))
}

fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn queue_target(t: &str) -> bool {
    t.contains("java.util.Queue#")
        || (t.contains("java.util.concurrent.")
            && (t.contains("BlockingQueue#") || t.contains("TransferQueue#")))
}

fn blocking_queue_descriptor(descriptor: &str) -> bool {
    [
        "BlockingQueue",
        "BlockingDeque",
        "LinkedBlockingQueue",
        "LinkedBlockingDeque",
        "ArrayBlockingQueue",
        "PriorityBlockingQueue",
        "SynchronousQueue",
        "LinkedTransferQueue",
        "DelayQueue",
    ]
    .iter()
    .any(|name| descriptor == format!("Ljava/util/concurrent/{name};"))
}

fn short_circuit(parsed: &Parsed, node: Node<'_>) -> bool {
    node.kind() == "binary_expression"
        && node
            .child_by_field_name("operator")
            .is_some_and(|o| matches!(parsed.text(o).as_str(), "&&" | "||"))
}

fn target_name_matches(relation: &Observation, name: &str, construct: bool) -> bool {
    let Some(target) = relation.normalized["targetIdentity"].as_str() else {
        return false;
    };
    let Some((owner, method)) = target.rsplit_once('#') else {
        return false;
    };
    let method = method.split('(').next().unwrap_or("");
    if construct {
        let simple_name = name
            .split('<')
            .next()
            .unwrap_or(name)
            .rsplit('.')
            .next()
            .unwrap_or(name);
        (method == "<init>" || method == simple_name)
            && owner
                .rsplit('.')
                .next()
                .is_some_and(|o| o.trim_start_matches("class:").ends_with(simple_name))
    } else {
        method == name
    }
}

fn diagnostics(worker: &CallableProjection) -> Vec<Diagnostic> {
    let rows = all_steps(&worker.steps);
    let mut result = Vec::new();
    for row in &rows {
        for call in &row.calls {
            if call.external_boundary.is_none() {
                continue;
            }
            for condition in &row.conditions {
                result.push(Diagnostic { condition: format!("{} is {}", condition.expression, !condition.holds), possible_reason: "This alternative can leave the selected call unreached through an earlier return or throw, or choose a different branch.".into(), inspect: vec![condition.expression.clone(), call.expression.clone()], selected_call: call.target.clone().unwrap_or_else(|| call.expression.clone()), citation_ids: vec![condition.citation_id.clone(), call.citation_id.clone()] });
            }
        }
    }
    result
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
