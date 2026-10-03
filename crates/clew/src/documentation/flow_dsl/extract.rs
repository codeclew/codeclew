use super::*;
use crate::canonical;
use std::collections::BTreeSet;
use tree_sitter::{Node, Parser};

const MAX_TEXT: usize = 2 * 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_ITEMS: usize = 4096;

fn kids(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn text<'a>(n: Node<'_>, s: &'a str) -> &'a str {
    &s[n.byte_range()]
}
fn field<'a>(n: Node<'a>, f: &str) -> Option<Node<'a>> {
    n.child_by_field_name(f)
}
fn name(n: Node<'_>, s: &str) -> String {
    field(n, "name")
        .map(|x| text(x, s).to_owned())
        .unwrap_or_default()
}
fn args(n: Node<'_>) -> Vec<Node<'_>> {
    field(n, "arguments").map(kids).unwrap_or_default()
}
fn nodes(n: Node<'_>) -> Result<Vec<Node<'_>>, ClewError> {
    let mut stack = vec![(n, 0)];
    let mut out = vec![];
    while let Some((n, d)) = stack.pop() {
        if out.len() >= MAX_NODES || d > 128 {
            return Err(invalid("flow DSL AST budget exceeded; narrow the profile"));
        }
        out.push(n);
        if matches!(
            n.kind(),
            "line_comment" | "block_comment" | "string_literal" | "character_literal"
        ) {
            continue;
        }
        stack.extend(kids(n).into_iter().rev().map(|n| (n, d + 1)));
    }
    Ok(out)
}
fn parse(s: &str) -> Result<tree_sitter::Tree, ClewError> {
    if s.len() > MAX_TEXT {
        return Err(invalid("flow DSL source exceeds its bounded scope"));
    }
    let mut p = Parser::new();
    p.set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(super::super::io_error)?;
    p.parse(s, None)
        .ok_or_else(|| invalid("flow DSL source could not be parsed"))
}
struct Selected<'a> {
    source: &'a Source,
    authority: &'static str,
    observation: Option<String>,
}
fn selected<'a>(
    e: &'a super::super::model::ServiceEvidence,
    b: &MethodBinding,
) -> Result<Selected<'a>, ClewError> {
    let mut candidates = vec![];
    if let Some(symbol) = &b.compiler_symbol {
        for o in e.observations.values().filter(|o| {
            o.kind == "SYMBOL"
                && o.normalized["kind"] == "DECLARATION"
                && o.normalized["authority"] != "SYNTAX"
                && o.normalized["ownerIdentity"].as_str() == Some(&format!("class:{}", b.owner))
                && o.normalized["name"].as_str() == Some(&b.method)
                && &o.symbol == symbol
        }) {
            if b.scope
                .as_ref()
                .is_some_and(|scope| o.normalized["scope"].as_str() != Some(scope))
            {
                continue;
            }
            for id in &o.source_ids {
                if let Some(s) = e.sources.get(id).filter(|s| s.file == b.file) {
                    candidates.push((s, Some(o.id.clone())));
                }
            }
        }
        if !candidates.is_empty() {
            return unique(candidates, "COMPILER_DECLARATION_MATCHED");
        }
    }
    for o in e.observations.values().filter(|o| {
        o.kind == "SYMBOL"
            && o.normalized["kind"] == "DECLARATION"
            && o.normalized["authority"] == "SYNTAX"
    }) {
        if o.normalized["ownerIdentity"].as_str() != Some(&format!("class:{}", b.owner))
            || o.normalized["name"].as_str() != Some(&b.method)
        {
            continue;
        }
        for id in &o.source_ids {
            if let Some(s) = e.sources.get(id).filter(|s| s.file == b.file) {
                candidates.push((s, None));
            }
        }
    }
    unique(candidates, "DECLARED_OWNER_AST_SYNTAX")
}
fn unique<'a>(
    c: Vec<(&'a Source, Option<String>)>,
    authority: &'static str,
) -> Result<Selected<'a>, ClewError> {
    // Distinct compilation candidates are not collapsed on equal text alone.
    if c.len() != 1 {
        return Err(invalid(
            "flow DSL method binding is missing or ambiguous in retained declarations; select an exact method and compilation scope",
        ));
    }
    let (s, o) = c.into_iter().next().unwrap();
    if s.text_digest != canonical::hash_bytes(s.text.as_bytes()) {
        return Err(invalid("flow DSL retained source digest mismatch"));
    }
    Ok(Selected {
        source: s,
        authority,
        observation: o,
    })
}
fn method_node<'a>(root: Node<'a>, s: &str, method: &str) -> Result<Node<'a>, ClewError> {
    let found: Vec<_> = nodes(root)?
        .into_iter()
        .filter(|n| {
            matches!(n.kind(), "method_declaration" | "constructor_declaration")
                && name(*n, s) == method
        })
        .collect();
    if found.len() != 1 {
        return Err(invalid(
            "flow DSL retained method text is incomplete or ambiguous",
        ));
    }
    Ok(found[0])
}
struct Extractor<'a> {
    e: &'a super::super::model::ServiceEvidence,
    p: &'a Profile,
    items: Vec<Item>,
    sources: BTreeMap<String, Source>,
}
impl Extractor<'_> {
    // Keep semantic record fields explicit at each emission site.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        kind: &str,
        label: String,
        expression: String,
        members: Vec<String>,
        conditions: &[String],
        sel: &Selected<'_>,
        n: Node<'_>,
        limitations: Vec<String>,
    ) -> Result<(), ClewError> {
        if self.items.len() >= MAX_ITEMS {
            return Err(invalid("flow DSL item budget exceeded; narrow the profile"));
        }
        self.sources
            .insert(sel.source.id.clone(), sel.source.clone());
        let relations = self.call_relations(n, sel);
        let call_target_authority = if n.kind() != "method_invocation" {
            "NOT_A_CALL"
        } else if relations.len() == 1 {
            "COMPILER_CALL_OCCURRENCE_MATCHED"
        } else if relations.len() > 1 {
            "AMBIGUOUS_COMPILER_CALL_SCOPE"
        } else {
            "DECLARED_RECEIVER_AST_SYNTAX"
        };
        let compiler_call_relation = if relations.len() == 1 {
            Some(relations[0].id.clone())
        } else {
            None
        };
        self.items.push(Item {
            id: format!("step-{}", self.items.len() + 1),
            kind: kind.into(),
            label,
            expression,
            members,
            conditions: conditions.to_vec(),
            reference: Reference {
                source_id: sel.source.id.clone(),
                start_line: sel.source.start_line + n.start_position().row as u64,
                end_line: sel.source.start_line + n.end_position().row as u64,
                binding_authority: sel.authority.into(),
                compiler_observation: sel.observation.clone(),
                call_target_authority: call_target_authority.into(),
                compiler_call_relation,
            },
            limitations,
        });
        Ok(())
    }
    fn gap(&mut self, sel: &Selected<'_>, n: Node<'_>, message: &str) -> Result<(), ClewError> {
        self.add(
            "gap",
            message.into(),
            text(n, &sel.source.text).into(),
            vec![],
            &[],
            sel,
            n,
            vec![message.into()],
        )
    }
    fn label(&self, id: &str) -> String {
        self.p.labels.get(id).cloned().unwrap_or_else(|| human(id))
    }
    fn call_relations(
        &self,
        n: Node<'_>,
        sel: &Selected<'_>,
    ) -> Vec<&super::super::model::Observation> {
        if n.kind() != "method_invocation" {
            return vec![];
        }
        let Some(occurrence) = &sel.source.occurrence else {
            return vec![];
        };
        let scope = sel
            .observation
            .as_ref()
            .and_then(|id| self.e.observations.get(id))
            .and_then(|o| o.normalized["scope"].as_str());
        self.e
            .observations
            .values()
            .filter(|o| {
                o.kind == "CALL_RELATION"
                    && o.normalized["relationKind"] == "CALLS"
                    && scope.is_none_or(|scope| o.normalized["scope"].as_str() == Some(scope))
                    && o.source_ids.iter().any(|id| {
                        self.e.sources.get(id).is_some_and(|src| {
                            src.service == sel.source.service
                                && src.revision == sel.source.revision
                                && src.file == sel.source.file
                                && src.text == text(n, &sel.source.text)
                                && src.text_digest == canonical::hash_bytes(src.text.as_bytes())
                                && src.occurrence.as_ref().is_some_and(|other| {
                                    other.snapshot == occurrence.snapshot
                                        && other.blob == occurrence.blob
                                        && other.start_byte
                                            == occurrence.start_byte + n.start_byte()
                                        && other.end_byte == occurrence.start_byte + n.end_byte()
                                })
                        })
                    })
            })
            .collect()
    }
    fn call_match(&self, n: Node<'_>, sel: &Selected<'_>, b: &CallBinding) -> bool {
        let s = &sel.source.text;
        if n.kind() != "method_invocation"
            || name(n, s) != b.method
            || field(n, "object").map(|x| text(x, s)) != Some(&b.receiver)
        {
            return false;
        }
        if let Some(target) = &b.compiler_target {
            let relations = self.call_relations(n, sel);
            if !relations.is_empty() {
                return relations.len() == 1
                    && relations[0].normalized["targetIdentity"].as_str() == Some(target);
            }
        }
        true
    }
    fn construction(&mut self) -> Result<(), ClewError> {
        let sel = selected(self.e, &self.p.construction)?;
        let s = &sel.source.text;
        let tree = parse(s)?;
        if tree.root_node().has_error() {
            return self.gap(
                &sel,
                tree.root_node(),
                "Construction source has Java parse errors; no DSL extraction accepted",
            );
        }
        let method = method_node(tree.root_node(), s, &self.p.construction.method)?;
        let all = nodes(method)?;
        if self.p.family == Family::BuilderChain {
            let root = self.p.root_variable.as_deref().unwrap();
            let b = self.p.builder.as_ref().unwrap();
            let mut init = 0;
            for n in &all {
                if n.kind() == "method_invocation" && self.call_match(*n, &sel, b) {
                    if guarded(*n, method, s) {
                        self.gap(
                            &sel,
                            *n,
                            "Conditional or nested construction is unsupported",
                        )?;
                        continue;
                    }
                    let bound = n.parent().is_some_and(|parent| match parent.kind() {
                        "assignment_expression" => {
                            field(parent, "left").is_some_and(|v| text(v, s) == root)
                        }
                        "variable_declarator" => {
                            field(parent, "name").is_some_and(|v| text(v, s) == root)
                        }
                        _ => false,
                    });
                    if !bound {
                        self.gap(
                            &sel,
                            *n,
                            "Builder call is not bound to the declared root variable",
                        )?;
                        continue;
                    }
                    let members = callbacks(&args(*n), s);
                    if members.is_none() {
                        self.gap(&sel, *n, "Unsupported builder callback expression")?;
                        continue;
                    }
                    init += 1;
                    self.add("callback-group", "Initialize context".into(), text(*n,s).into(), members.unwrap(), &[], &sel,*n,vec!["Declaration order; callback group does not establish parallel execution".into()])?;
                }
            }
            if init != 1 {
                self.gap(&sel,method,"Root initialization is missing or multiple; chain activation remains unresolved")?;
                return Ok(());
            }
            for n in &all {
                if n.kind() != "method_invocation" || name(*n, s) != "then" {
                    continue;
                }
                if n.parent().is_some_and(|p| {
                    p.kind() == "method_invocation"
                        && field(p, "object") == Some(*n)
                        && name(p, s) == "then"
                }) {
                    continue;
                }
                let mut chain = vec![];
                let mut current = *n;
                loop {
                    if current.kind() == "method_invocation" && name(current, s) == "then" {
                        chain.push(current);
                        if let Some(o) = field(current, "object") {
                            current = o;
                            continue;
                        }
                    }
                    break;
                }
                if text(current, s) != root {
                    continue;
                }
                if guarded(*n, method, s) {
                    self.gap(
                        &sel,
                        *n,
                        "Conditional or nested callback chain is unsupported",
                    )?;
                    continue;
                }
                for call in chain.into_iter().rev() {
                    if let Some(members) = callbacks(&args(call), s) {
                        let label = members
                            .first()
                            .map(|m| self.label(m.rsplit("::").next().unwrap_or(m)))
                            .unwrap_or_else(|| "Declared callback group".into());
                        self.add("callback-group",label,text(call,s).into(),members,&[],&sel,call,vec!["Declared chain group; scheduler, exceptions and parallelism remain unresolved".into()])?;
                    } else {
                        self.gap(&sel, call, "Unsupported chain callback expression")?;
                    }
                }
            }
        } else {
            let constructors: Vec<_> = all
                .into_iter()
                .filter(|n| {
                    n.kind() == "object_creation_expression"
                        && field(*n, "type").is_some_and(|t| {
                            kids(t)
                                .first()
                                .map(|x| text(*x, s))
                                .unwrap_or_else(|| text(t, s))
                                == self.p.queue_type.as_deref().unwrap()
                        })
                })
                .collect();
            if constructors.len() != 1 {
                self.gap(&sel, method, "Queue constructor is missing or ambiguous")?;
                return Ok(());
            }
            let n = constructors[0];
            if guarded(n, method, s) {
                self.gap(
                    &sel,
                    n,
                    "Conditional or nested queue construction is unsupported",
                )?;
                return Ok(());
            }
            let a = args(n);
            let start = self.p.initial_argument.unwrap_or(2);
            if start >= a.len() {
                self.gap(&sel, n, "Initial operation arguments are absent")?;
                return Ok(());
            }
            let mut members = vec![];
            for op in a.into_iter().skip(start) {
                if let Some(v) = operation(op, s, &self.p.operation_type) {
                    members.push(v);
                } else {
                    self.gap(&sel, op, "Unsupported initial operation expression")?;
                }
            }
            self.add("initial-queue","Declared initial operation queue".into(),text(n,s).into(),members,&[],&sel,n,vec!["Constructor argument order only; dequeue and delivery semantics require engine evidence".into()])?;
        }
        Ok(())
    }
    fn registry(&mut self) -> Result<(), ClewError> {
        let Some(binding) = &self.p.registry else {
            return Ok(());
        };
        let sel = selected(self.e, binding)?;
        let s = &sel.source.text;
        let tree = parse(s)?;
        if tree.root_node().has_error() {
            return self.gap(&sel, tree.root_node(), "Registry source has parse errors");
        }
        let m = method_node(tree.root_node(), s, &binding.method)?;
        let b = CallBinding {
            receiver: self.p.registry_receiver.clone().unwrap_or_default(),
            method: "put".into(),
            compiler_target: None,
        };
        let mut seen = BTreeSet::new();
        for n in nodes(m)? {
            if !self.call_match(n, &sel, &b) {
                continue;
            }
            if guarded(n, m, s) {
                self.gap(
                    &sel,
                    n,
                    "Conditional or nested registry binding is unsupported",
                )?;
                continue;
            }
            let a = args(n);
            if a.len() != 2 {
                self.gap(&sel, n, "Unsupported registry binding")?;
                continue;
            }
            if let (Some(op), Some(cb)) = (
                operation(a[0], s, &self.p.operation_type),
                callback(a[1], s),
            ) {
                let duplicate = !seen.insert(op.clone());
                self.add(
                    "registry-binding",
                    self.label(&op),
                    text(n, s).into(),
                    vec![op, cb],
                    &[],
                    &sel,
                    n,
                    vec![
                        if duplicate {
                            "Duplicate registry key: replacement/activation semantics unresolved"
                        } else {
                            "Unordered operation lookup binding; not an execution sequence"
                        }
                        .into(),
                    ],
                )?;
            } else {
                self.gap(&sel, n, "Unsupported registry key or callback")?;
            }
        }
        Ok(())
    }
    fn factory(&mut self) -> Result<(), ClewError> {
        let Some(b) = &self.p.factory else {
            return Ok(());
        };
        let sel = selected(self.e, b)?;
        let s = &sel.source.text;
        let tree = parse(s)?;
        if tree.root_node().has_error() {
            return self.gap(&sel, tree.root_node(), "Factory source has parse errors");
        }
        let m = method_node(tree.root_node(), s, &b.method)?;
        for n in nodes(m)?
            .into_iter()
            .filter(|n| n.kind() == "return_statement")
        {
            if guarded(n, m, s) {
                self.gap(
                    &sel,
                    n,
                    "Conditional or nested factory return is unsupported",
                )?;
                continue;
            }
            let v = kids(n)
                .first()
                .and_then(|v| operation(*v, s, &self.p.task_type));
            if let Some(v) = v {
                self.add("factory-task-type","Declared factory task type".into(),text(n,s).into(),vec![v],&[],&sel,n,vec!["Factory return declaration does not prove runtime registration or trigger activation".into()])?;
            } else {
                self.gap(
                    &sel,
                    n,
                    "Factory task type is not a supported constant return",
                )?;
            }
        }
        Ok(())
    }
    fn contexts(&mut self) -> Result<(), ClewError> {
        let Some(c) = &self.p.context else {
            return Ok(());
        };
        let mut methods = BTreeSet::new();
        for o in self.e.observations.values().filter(|o| {
            o.kind == "SYMBOL"
                && o.normalized["kind"] == "DECLARATION"
                && o.normalized["ownerIdentity"] == format!("class:{}", c.owner)
        }) {
            if o.source_ids
                .iter()
                .any(|id| self.e.sources.get(id).is_some_and(|s| s.file == c.file))
                && let Some(n) = o.normalized["name"].as_str()
            {
                methods.insert(n.to_owned());
            }
        }
        let callback_owner = |kind: &str| {
            if kind == "registry-binding" {
                self.p
                    .registry
                    .as_ref()
                    .map(|b| b.owner.as_str())
                    .unwrap_or("")
            } else {
                self.p.construction.owner.as_str()
            }
        };
        for item in &mut self.items {
            let owner = callback_owner(&item.kind);
            if owner != c.owner && item.members.iter().any(|m| m.starts_with("this::")) {
                item.limitations.push(format!("this callback belongs to {owner}; delegation to configured context {} is unresolved",c.owner));
            }
        }
        let referenced: BTreeSet<_> = self
            .items
            .iter()
            .flat_map(|i| i.members.iter().map(move |m| (i, m)))
            .filter_map(|(i, m)| m.split_once("::").map(|pair| (i, pair)))
            .filter(|(i, (owner, _))| {
                (*owner == "this" && callback_owner(&i.kind) == c.owner)
                    || *owner == c.owner
                    || *owner == c.owner.rsplit('.').next().unwrap_or("")
            })
            .map(|(_, (_, name))| name.to_owned())
            .chain(self.p.decision_methods.iter().cloned())
            .collect();
        for method in referenced {
            if !methods.contains(&method) {
                // A missing callback is a local gap, not an invented stage.
                self.items
                    .iter_mut()
                    .filter(|i| {
                        i.members
                            .iter()
                            .any(|m| m.ends_with(&format!("::{method}")))
                    })
                    .for_each(|i| {
                        i.limitations
                            .push(format!("Callback source unavailable: {method}"))
                    });
                continue;
            }
            let compiler: Vec<_> = self
                .e
                .observations
                .values()
                .filter(|o| {
                    o.kind == "SYMBOL"
                        && o.normalized["kind"] == "DECLARATION"
                        && o.normalized["authority"] != "SYNTAX"
                        && o.normalized["ownerIdentity"] == format!("class:{}", c.owner)
                        && o.normalized["name"] == method
                        && o.source_ids
                            .iter()
                            .any(|id| self.e.sources.get(id).is_some_and(|s| s.file == c.file))
                })
                .collect();
            if compiler.len() > 1 {
                return Err(invalid(
                    "flow DSL callback declaration has ambiguous compiler scopes",
                ));
            }
            let b = MethodBinding {
                file: c.file.clone(),
                owner: c.owner.clone(),
                method: method.clone(),
                compiler_symbol: compiler.first().map(|o| o.symbol.clone()),
                scope: compiler
                    .first()
                    .and_then(|o| o.normalized["scope"].as_str())
                    .map(str::to_owned),
            };
            let sel = selected(self.e, &b)?;
            let s = &sel.source.text;
            let tree = parse(s)?;
            if tree.root_node().has_error() {
                self.gap(&sel, tree.root_node(), "Context source has parse errors")?;
                continue;
            }
            let n = method_node(tree.root_node(), s, &method)?;
            if let Some(body) = field(n, "body") {
                self.walk(
                    body,
                    &sel,
                    &[],
                    self.p.decision_methods.contains(&method),
                    0,
                )?;
            }
        }
        Ok(())
    }
    fn walk(
        &mut self,
        n: Node<'_>,
        sel: &Selected<'_>,
        conditions: &[String],
        decision: bool,
        depth: usize,
    ) -> Result<(), ClewError> {
        if depth > 128 {
            return Err(invalid("flow DSL control nesting exceeds its bound"));
        }
        let s = &sel.source.text;
        if matches!(
            n.kind(),
            "line_comment" | "block_comment" | "string_literal" | "character_literal"
        ) {
            return Ok(());
        }
        if matches!(
            n.kind(),
            "lambda_expression"
                | "ternary_expression"
                | "switch_expression"
                | "switch_statement"
                | "for_statement"
                | "enhanced_for_statement"
                | "while_statement"
                | "do_statement"
                | "try_statement"
                | "class_declaration"
        ) {
            return self.gap(sel,n,"Unsupported local control construct; enclosed selections/effects are not interpreted");
        }
        if n.kind() == "binary_expression"
            && field(n, "operator").is_some_and(|o| matches!(text(o, s), "&&" | "||"))
        {
            return self.gap(sel,n,"Short-circuit expression requires explicit evaluation semantics; enclosed calls are not interpreted");
        }
        if n.kind() == "object_creation_expression"
            && kids(n).iter().any(|c| c.kind() == "class_body")
        {
            return self.gap(
                sel,
                n,
                "Anonymous class body is outside the selected callback scope",
            );
        }
        if n.kind() == "block" {
            let mut conditional_exit = false;
            for child in kids(n) {
                if conditional_exit {
                    self.gap(
                        sel,
                        child,
                        "Control-dependent tail after a conditional exit; path is unresolved",
                    )?;
                    continue;
                }
                self.walk(child, sel, conditions, decision, depth + 1)?;
                if child.kind() == "if_statement"
                    && nodes(child)?
                        .iter()
                        .any(|n| matches!(n.kind(), "return_statement" | "throw_statement"))
                {
                    conditional_exit = true;
                }
            }
            return Ok(());
        }
        if n.kind() == "if_statement" {
            let Some(condition) = field(n, "condition") else {
                return self.gap(sel, n, "Missing decision condition");
            };
            let expr = text(condition, s).to_owned();
            // Preserve unknown helper semantics next to the condition.
            let helpers: Vec<_> = nodes(condition)?
                .into_iter()
                .filter(|n| n.kind() == "method_invocation")
                .map(|n| text(n, s).to_owned())
                .collect();
            self.add(
                "condition",
                "Local if/else condition".into(),
                expr.clone(),
                vec![],
                conditions,
                sel,
                condition,
                helpers
                    .into_iter()
                    .map(|h| format!("Condition helper semantics unresolved: {h}"))
                    .collect(),
            )?;
            let mut yes = conditions.to_vec();
            yes.push(expr.clone());
            if let Some(body) = field(n, "consequence") {
                self.walk(body, sel, &yes, decision, depth + 1)?;
            }
            if let Some(body) = field(n, "alternative") {
                let mut no = conditions.to_vec();
                no.push(format!("else of {expr}"));
                self.walk(body, sel, &no, decision, depth + 1)?;
            }
            return Ok(());
        }
        if n.kind() == "assignment_expression" {
            self.add(
                "mapping",
                "Assign local data".into(),
                text(n, s).into(),
                vec![],
                conditions,
                sel,
                n,
                vec!["Assignment is an effect, not a condition".into()],
            )?;
        }
        if n.kind() == "method_invocation" {
            if decision
                && self
                    .p
                    .selection_calls
                    .iter()
                    .any(|b| self.call_match(n, sel, b))
            {
                let members = operation_list(&args(n), s, &self.p.operation_type);
                match members {Some(m) if !m.is_empty()=>self.add("selected-next-operations","Conditionally select next operations".into(),text(n,s).into(),m,conditions,sel,n,vec!["Syntactic selection; activation, queue merge and repeat policy remain unresolved".into()])?,_=>self.gap(sel,n,"Unsupported next-operation expression")?};
                return Ok(());
            }
            if let Some(effect) = self
                .p
                .effects
                .iter()
                .find(|e| self.call_match(n, sel, &e.call))
            {
                let limits = if effect.kind == "opaque-external-call" {
                    vec!["Outbound request is an opaque external decision boundary; local rules, destination configuration, delivery and response semantics are unresolved".into()]
                } else {
                    vec![
                        "Profile-declared effect classification; source proves the local call only"
                            .into(),
                    ]
                };
                self.add(
                    &effect.kind,
                    effect.label.clone(),
                    text(n, s).into(),
                    vec![],
                    conditions,
                    sel,
                    n,
                    limits,
                )?;
                for arg in args(n) {
                    self.walk(arg, sel, conditions, decision, depth + 1)?;
                }
                return Ok(());
            }
            // Other invocations may contribute data or hide effects; retain them explicitly.
            if name(n, s) != "name" {
                self.add(
                    "unknown-helper",
                    "Local helper or call".into(),
                    text(n, s).into(),
                    vec![],
                    conditions,
                    sel,
                    n,
                    vec!["Target/effects/engine semantics not interpreted by this profile".into()],
                )?;
            }
        }
        if n.kind() == "return_statement" || n.kind() == "throw_statement" {
            if decision
                && n.kind() == "return_statement"
                && let Some(v) = kids(n).first()
                && let Some(members) = operation_list(&[*v], s, &self.p.operation_type)
                && !members.is_empty()
            {
                self.add("selected-next-operations","Return selected next operations".into(),text(n,s).into(),members,conditions,sel,n,vec!["Return selection is not runtime execution or successful business completion".into()])?;
                return Ok(());
            }
            self.add(
                "exit",
                if n.kind() == "throw_statement" {
                    "Throw from local method"
                } else {
                    "Return from local method"
                }
                .into(),
                text(n, s).into(),
                vec![],
                conditions,
                sel,
                n,
                vec!["Local exit marker; overall process completion remains unresolved".into()],
            )?;
        }
        for c in kids(n) {
            self.walk(c, sel, conditions, decision, depth + 1)?;
        }
        Ok(())
    }
}
fn guarded(mut n: Node<'_>, method: Node<'_>, s: &str) -> bool {
    while let Some(p) = n.parent() {
        if p == method {
            return false;
        }
        if p.kind() == "binary_expression"
            && field(p, "operator").is_some_and(|o| matches!(text(o, s), "&&" | "||"))
        {
            return true;
        }
        if matches!(
            p.kind(),
            "if_statement"
                | "lambda_expression"
                | "class_body"
                | "switch_expression"
                | "switch_statement"
                | "for_statement"
                | "enhanced_for_statement"
                | "while_statement"
                | "do_statement"
                | "try_statement"
                | "ternary_expression"
        ) {
            return true;
        }
        n = p;
    }
    false
}
fn callback(n: Node<'_>, s: &str) -> Option<String> {
    if n.kind() != "method_reference" {
        return None;
    }
    let c = kids(n);
    if c.len() != 2
        || !matches!(
            c[0].kind(),
            "identifier" | "this" | "type_identifier" | "scoped_type_identifier"
        )
        || c[1].kind() != "identifier"
    {
        return None;
    }
    Some(format!("{}::{}", text(c[0], s), text(c[1], s)))
}
fn callbacks(a: &[Node<'_>], s: &str) -> Option<Vec<String>> {
    if a.is_empty() {
        None
    } else {
        a.iter().map(|n| callback(*n, s)).collect()
    }
}
fn operation(n: Node<'_>, s: &str, ty: &str) -> Option<String> {
    if n.kind() == "method_invocation" && name(n, s) == "name" && args(n).is_empty() {
        return field(n, "object").and_then(|n| operation(n, s, ty));
    }
    if n.kind() != "field_access" {
        return None;
    }
    let owner = field(n, "object")?;
    let member = field(n, "field")?;
    if text(owner, s) != ty || member.kind() != "identifier" {
        return None;
    }
    Some(text(member, s).into())
}
fn operation_list(a: &[Node<'_>], s: &str, ty: &str) -> Option<Vec<String>> {
    let mut out = vec![];
    for n in a {
        if let Some(op) = operation(*n, s, ty) {
            out.push(op);
            continue;
        }
        if n.kind() == "method_invocation"
            && matches!(
                (
                    field(*n, "object").map(|x| text(x, s)),
                    name(*n, s).as_str()
                ),
                (Some("List"), "of")
                    | (Some("Arrays"), "asList")
                    | (Some("Collections"), "singletonList")
            )
        {
            out.extend(operation_list(&args(*n), s, ty)?);
            continue;
        }
        if n.kind() == "method_invocation"
            && field(*n, "object").is_some_and(|x| text(x, s) == "Collections")
            && name(*n, s) == "emptyList"
            && args(*n).is_empty()
        {
            continue;
        }
        return None;
    }
    Some(out)
}
fn human(id: &str) -> String {
    let mut s = String::new();
    let mut prev = false;
    for c in id.chars() {
        if c == '_' {
            s.push(' ');
            prev = false;
        } else {
            if c.is_uppercase() && prev {
                s.push(' ');
            }
            s.push(c.to_ascii_lowercase());
            prev = c.is_lowercase();
        }
    }
    if let Some(c) = s.get_mut(..1) {
        c.make_ascii_uppercase();
    }
    s
}
pub(super) fn project(
    e: &super::super::model::ServiceEvidence,
    snapshot: &str,
    p: &Profile,
) -> Result<Projection, ClewError> {
    p.validate()?;
    if e.sources
        .values()
        .any(|s| s.service != e.service || s.revision != e.revision)
    {
        return Err(invalid("flow DSL source mixes service or revision scopes"));
    }
    let mut x = Extractor {
        e,
        p,
        items: vec![],
        sources: BTreeMap::new(),
    };
    x.factory()?;
    x.construction()?;
    x.registry()?;
    x.contexts()?;
    let mut phases = vec![];
    for (id, label, kinds) in [
        (
            "entry",
            "Entry and context initialization",
            vec!["factory-task-type", "initial-queue"],
        ),
        ("stages", "Declared callback stages", vec!["callback-group"]),
        (
            "lookup",
            "Operation lookup bindings",
            vec!["registry-binding"],
        ),
        (
            "decisions",
            "Local decisions and selected operations",
            vec!["condition", "selected-next-operations"],
        ),
        (
            "data",
            "Data mappings and state effects",
            vec!["mapping", "state-change", "persistence-call"],
        ),
        (
            "boundaries",
            "External boundaries and helper calls",
            vec!["opaque-external-call", "unknown-helper"],
        ),
        (
            "exits",
            "Exits and unresolved constructs",
            vec!["exit", "gap"],
        ),
    ] {
        let item_ids: Vec<_> = x
            .items
            .iter()
            .filter(|i| kinds.contains(&i.kind.as_str()))
            .map(|i| i.id.clone())
            .collect();
        if !item_ids.is_empty() {
            phases.push(Phase {
                id: id.into(),
                label: label.into(),
                item_ids,
            });
        }
    }
    // Short flows keep their true size. Large builder chains are coarsened into five contiguous declaration groups.
    if phases.len() < 5
        && x.items
            .iter()
            .filter(|i| i.kind == "callback-group")
            .count()
            >= 8
        && let Some(index) = phases.iter().position(|p| p.id == "stages")
    {
        let original = phases.remove(index);
        let slots = (7 - phases.len()).min(5);
        let chunk = original.item_ids.len().div_ceil(slots);
        for (i, ids) in original.item_ids.chunks(chunk).enumerate().rev() {
            phases.insert(
                index,
                Phase {
                    id: format!("stages-{}", i + 1),
                    label: format!(
                        "Declared stages {}–{} (grouped)",
                        i * chunk + 1,
                        i * chunk + ids.len()
                    ),
                    item_ids: ids.to_vec(),
                },
            );
        }
    }
    Ok(Projection{schema:PROJECTION_SCHEMA.into(),id:p.id.clone(),title:p.title.clone(),service:e.service.clone(),revision:e.revision.clone(),snapshot:snapshot.into(),profile_digest:digest(p)?,framework:p.framework.clone(),engine_version:p.engine_version.clone(),authority:"SOURCE_INTERPRETATION_WITH_DECLARED_PROFILE_NOT_RUNTIME_TRACE".into(),family:p.family.clone(),items:x.items,phases,limitations:vec!["Overview phases are editorial groups; their order does not assert execution across categories".into(),"Engine version is profile-declared or unresolved; framework scheduling, runtime activation, retries, ordering and delivery require separate evidence".into(),"This static projection describes possible source structure, not incident causality or successful completion".into()],sources:x.sources})
}
