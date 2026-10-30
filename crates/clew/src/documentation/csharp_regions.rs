//! Roslyn source regions: lexical order and conditional normal completion only.
use super::*;
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "codeclew-csharp-source-regions/1.0";
const EVIDENCE: &str = "codeclew-csharp-source-regions-evidence/1.0";
pub(crate) const KIND: &str = "CSHARP_SOURCE_REGIONS";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Span {
    file: String,
    start: u64,
    end: u64,
    start_line: u64,
    end_line: u64,
    byte_start: usize,
    byte_end: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Region {
    id: usize,
    parent: Option<usize>,
    role: String,
    kind: String,
    source: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opaque: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    loop_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    continue_phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    has_finally: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exit_boundary: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope_cleanup: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expression {
    region: usize,
    role: String,
    source: Span,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Site {
    region: usize,
    kind: String,
    source: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_in_source: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    boundary: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Graph {
    schema: String,
    authority: String,
    interpretation: String,
    execution: String,
    owner_symbol_identity: String,
    file: String,
    nodes: Vec<Region>,
    sites: Vec<Site>,
    expressions: Vec<Expression>,
    boundaries: Vec<String>,
    exception_completeness: String,
    interprocedural_completion: String,
}

fn contains(outer: &Span, inner: &Span) -> bool {
    outer.file == inner.file
        && outer.byte_start <= inner.byte_start
        && inner.byte_end <= outer.byte_end
}

fn validate(
    raw: &Value,
    owner: &str,
    file: &str,
    text: &str,
    start: usize,
    end: usize,
) -> Result<Graph, ClewError> {
    let g: Graph = serde_json::from_value(raw.clone())
        .map_err(|_| invalid("C# source-region contract is invalid"))?;
    if g.schema != SCHEMA
        || g.authority != "ROSLYN_SOURCE_REGIONS"
        || g.interpretation != "LEXICAL_AND_CONDITIONAL_NORMAL_ONLY"
        || g.execution != "NOT_ASSERTED"
        || g.exception_completeness != "UNKNOWN"
        || g.interprocedural_completion != "NOT_MODELED_USE_ORIGINAL_SOURCE"
        || g.owner_symbol_identity != owner
        || g.file != file
        || g.nodes.is_empty()
        || g.nodes.len() + g.sites.len() + g.expressions.len() > 96
    {
        return Err(invalid(
            "C# source-region authority, owner or budget changed",
        ));
    }
    let coordinates = super::super::csharp::SourceCoordinates::new(text);
    let span_valid = |span: &Span| {
        span.file == file
            && start <= span.byte_start
            && span.byte_start < span.byte_end
            && span.byte_end <= end
            && coordinates.valid_anchor(&serde_json::to_value(span).unwrap())
            && span.start_line
                == text.as_bytes()[..span.byte_start]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count() as u64
                    + 1
            && span.end_line
                == text.as_bytes()[..span.byte_end - 1]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count() as u64
                    + 1
    };
    for (index, node) in g.nodes.iter().enumerate() {
        if node.id != index
            || !span_valid(&node.source)
            || !matches!(
                node.kind.as_str(),
                "BLOCK"
                    | "IF"
                    | "LOOP"
                    | "TRY"
                    | "CATCH"
                    | "USING"
                    | "LOCAL"
                    | "EXPRESSION"
                    | "RETURN"
                    | "THROW"
                    | "BREAK"
                    | "CONTINUE"
                    | "EMPTY"
                    | "OPAQUE"
            )
        {
            return Err(invalid("C# source-region node identity or source changed"));
        }
        if let Some(parent) = node.parent {
            if parent >= index || !contains(&g.nodes[parent].source, &node.source) {
                return Err(invalid("C# source-region parent or containment changed"));
            }
            let kind = g.nodes[parent].kind.as_str();
            let valid = match node.role.as_str() {
                "STATEMENT" => kind == "BLOCK",
                "THEN" | "ELSE" => kind == "IF",
                "LOOP_BODY" => kind == "LOOP",
                "TRY_BODY" | "CATCH" | "FINALLY" => kind == "TRY",
                "CATCH_BODY" => kind == "CATCH",
                "USING_BODY" => kind == "USING",
                _ => false,
            };
            if !valid {
                return Err(invalid("C# source-region role does not match its parent"));
            }
        } else if index != 0 || node.role != "BODY" {
            return Err(invalid("C# source regions require one body root"));
        }
        if node.kind == "LOOP" {
            let expected = match node.loop_kind.as_deref() {
                Some("WHILE" | "DO") => "CONDITION",
                Some("FOR") => "INCREMENTOR_THEN_CONDITION",
                Some("FOREACH") => "ENUMERATOR_ADVANCE",
                _ => return Err(invalid("C# loop kind is invalid")),
            };
            if node.continue_phase.as_deref() != Some(expected) {
                return Err(invalid("C# loop continuation phase changed"));
            }
        }
        if let Some(exit) = node.exit_boundary
            && (node.kind != "TRY"
                || node.has_finally != Some(true)
                || g.nodes.get(exit).is_none_or(|n| {
                    n.parent != Some(index) || n.role != "FINALLY" || n.opaque.is_none()
                }))
        {
            return Err(invalid("C# finally boundary changed"));
        }
        if node.scope_cleanup.is_some() && node.kind != "LOCAL" {
            return Err(invalid("C# cleanup declaration scope changed"));
        }
        if node.kind == "OPAQUE" && node.opaque.is_none() {
            return Err(invalid("C# opaque region requires an explicit boundary"));
        }
        if matches!(node.kind.as_str(), "BREAK" | "CONTINUE")
            && !ancestors(&g, index)
                .iter()
                .any(|i| g.nodes[*i].kind == "LOOP")
        {
            return Err(invalid("C# loop exit lacks an enclosing supported loop"));
        }
        if node.opaque.as_deref() != Some("SOURCE_REGIONS_BUDGET") {
            let required = match node.kind.as_str() {
                "IF" => Some("THEN"),
                "LOOP" => Some("LOOP_BODY"),
                "TRY" => Some("TRY_BODY"),
                "CATCH" => Some("CATCH_BODY"),
                "USING" => Some("USING_BODY"),
                _ => None,
            };
            if required.is_some_and(|role| {
                g.nodes
                    .iter()
                    .filter(|n| n.parent == Some(index) && n.role == role)
                    .count()
                    != 1
            }) || (node.has_finally == Some(true) && node.exit_boundary.is_none())
            {
                return Err(invalid(
                    "C# source-region supported child or cleanup is missing",
                ));
            }
        }
    }
    for expression in &g.expressions {
        if !matches!(
            expression.role.as_str(),
            "CONDITION"
                | "INITIALIZER"
                | "INCREMENTOR"
                | "ENUMERABLE"
                | "CATCH_TYPE"
                | "FILTER"
                | "RESOURCE"
                | "VALUE"
                | "RETURN_VALUE"
                | "THROW_VALUE"
        ) || g
            .nodes
            .get(expression.region)
            .is_none_or(|n| !contains(&n.source, &expression.source))
            || !span_valid(&expression.source)
        {
            return Err(invalid("C# exact expression source changed"));
        }
    }
    for site in &g.sites {
        if g.nodes
            .get(site.region)
            .is_none_or(|n| !contains(&n.source, &site.source))
            || !span_valid(&site.source)
            || !matches!(
                site.kind.as_str(),
                "CALL" | "CONSTRUCT" | "AWAIT" | "OPAQUE"
            )
            || site.target.as_ref().is_some_and(|t| {
                t.len() > 2048
                    || site.resolution.as_deref()
                        != Some("COMPILER_EXACT_TARGET_NOT_RUNTIME_DISPATCH")
            })
            || (site.kind == "AWAIT" && (site.target.is_some() || site.resolution.is_some()))
            || (site.kind == "OPAQUE" && site.boundary.as_deref().is_none_or(str::is_empty))
        {
            return Err(invalid("C# call/await source-region binding changed"));
        }
    }
    Ok(g)
}

fn children(g: &Graph, id: usize) -> Vec<usize> {
    g.nodes
        .iter()
        .filter(|n| n.parent == Some(id))
        .map(|n| n.id)
        .collect()
}

fn ancestors(g: &Graph, id: usize) -> Vec<usize> {
    let mut result = Vec::new();
    let mut current = Some(id);
    while let Some(id) = current {
        result.push(id);
        current = g.nodes[id].parent;
    }
    result
}

fn cleanup(g: &Graph, id: usize) -> Vec<usize> {
    let mut result = Vec::new();
    for parent in ancestors(g, id) {
        let n = &g.nodes[parent];
        // A loop exit crosses only scopes inside its target loop. Continue
        // retains the enumerator; break leaves it. Neither exits an outer using.
        if matches!(g.nodes[id].kind.as_str(), "BREAK" | "CONTINUE") && n.kind == "LOOP" {
            if g.nodes[id].kind == "BREAK" && n.loop_kind.as_deref() == Some("FOREACH") {
                result.push(parent);
            }
            break;
        }
        if n.kind == "USING"
            || n.has_finally == Some(true)
            || (n.kind == "LOOP" && n.opaque.is_some())
        {
            result.push(n.exit_boundary.unwrap_or(parent));
        }
        if n.kind == "BLOCK" {
            result.extend(children(g, parent).into_iter().filter(|i| {
                g.nodes[*i].scope_cleanup.is_some()
                    && g.nodes[*i].source.byte_start < g.nodes[id].source.byte_start
            }));
        }
    }
    result
}

fn relationships(g: &Graph) -> Value {
    // A small bottom-up source rule, not path enumeration or exception dispatch:
    // an unconditional source exit cannot become normal completion merely by
    // wrapping it in a block or handler. Unknown constructs remain possible.
    let mut possible = vec![true; g.nodes.len()];
    for n in g.nodes.iter().rev() {
        let kids = children(g, n.id);
        possible[n.id] = match n.kind.as_str() {
            "RETURN" | "THROW" | "BREAK" | "CONTINUE" => false,
            "BLOCK" => kids.iter().all(|i| possible[*i]),
            "IF" => {
                !kids.iter().any(|i| g.nodes[*i].role == "ELSE")
                    || kids.iter().any(|i| possible[*i])
            }
            "CATCH" | "USING" => kids.first().is_none_or(|i| possible[*i]),
            "TRY" => kids
                .iter()
                .any(|i| g.nodes[*i].role != "FINALLY" && possible[*i]),
            _ => true,
        };
    }
    fn walk(
        g: &Graph,
        possible: &[bool],
        id: usize,
        target: Value,
        normal: &mut Vec<Value>,
        lexical: &mut Vec<Value>,
        abrupt: &mut Vec<Value>,
    ) {
        let n = &g.nodes[id];
        let kids = children(g, id);
        let scoped_cleanup =
            n.kind == "BLOCK" && kids.iter().any(|i| g.nodes[*i].scope_cleanup.is_some());
        let blocked = n.opaque.is_some() || n.has_finally == Some(true) || scoped_cleanup;
        let exit = if blocked {
            json!({"opaqueExitBoundary":n.exit_boundary.unwrap_or(id)})
        } else {
            target.clone()
        };
        if matches!(n.kind.as_str(), "RETURN" | "THROW" | "BREAK" | "CONTINUE") {
            let enclosing_loop = ancestors(g, id)
                .into_iter()
                .find(|i| g.nodes[*i].kind == "LOOP");
            abrupt.push(json!({"from":id,"kind":n.kind,"targetScope":if n.kind=="RETURN" {Some(0)} else if n.kind=="THROW" {None} else {enclosing_loop},"continuePhase":if n.kind=="CONTINUE" {enclosing_loop.and_then(|i|g.nodes[i].continue_phase.clone())} else {None},"unknownCleanupBoundaries":cleanup(g,id),"interpretation":"SOURCE_EXIT_IF_OPERAND_AND_REQUIRED_CLEANUP_COMPLETE"}));
            return;
        }
        if !blocked && n.kind != "OPAQUE" && possible[id] {
            normal.push(json!({"from":id,"to":target,"meaning":"IF_REGION_COMPLETES_NORMALLY"}));
        }
        match n.kind.as_str() {
            "BLOCK" => {
                for pair in kids.windows(2) {
                    lexical.push(
                        json!({"from":pair[0],"to":pair[1],"meaning":"SOURCE_SIBLINGS_ONLY"}),
                    );
                }
                for (index, child) in kids.iter().enumerate() {
                    walk(
                        g,
                        possible,
                        *child,
                        kids.get(index + 1)
                            .map_or_else(|| exit.clone(), |i| json!(i)),
                        normal,
                        lexical,
                        abrupt,
                    );
                }
            }
            "LOOP" => {
                for child in kids {
                    walk(
                        g,
                        possible,
                        child,
                        json!({"loop":id,"phase":n.continue_phase,"meaning":"IF_ITERATION_COMPLETES_NORMALLY"}),
                        normal,
                        lexical,
                        abrupt,
                    );
                }
            }
            "TRY" | "IF" | "CATCH" | "USING" => {
                for child in kids {
                    walk(g, possible, child, exit.clone(), normal, lexical, abrupt);
                }
            }
            _ => {}
        }
    }
    let (mut normal, mut lexical, mut abrupt) = (Vec::new(), Vec::new(), Vec::new());
    walk(
        g,
        &possible,
        0,
        json!({"methodExit":"NORMAL_IF_BODY_AND_CLEANUP_COMPLETE"}),
        &mut normal,
        &mut lexical,
        &mut abrupt,
    );
    let sites=g.sites.iter().enumerate().map(|(index,s)|{
        let chain=ancestors(g,s.region);
        let protected=chain.iter().find_map(|i| {let n=&g.nodes[*i]; (n.role=="TRY_BODY").then_some(n.parent).flatten()});
        json!({"site":index,"region":s.region,"enclosingRegions":chain,"protectedTry":protected,"awaitCompletion":if s.kind=="AWAIT" {Some("NORMAL_RESUME_OR_POSSIBLE_FAULT_CANCELLATION")}else{None},"interproceduralCompletion":"NOT_MODELED_USE_ORIGINAL_SOURCE"})
    }).collect::<Vec<_>>();
    json!({"lexicalNext":lexical,"normalContinuation":normal,"abrupt":abrupt,"siteContexts":sites,"exceptions":"UNKNOWN; CATCH ORDER/FILTER SOURCE IS NOT EXHAUSTIVE DISPATCH PROOF"})
}

pub(super) fn project(
    e: &mut ServiceEvidence,
    fact: &Value,
    binding: &str,
    scope: &str,
    owner_id: &str,
    site: Option<&Value>,
    text: &str,
) -> Result<(), ClewError> {
    let Some(raw) = fact["documentation"].get("controlRegions") else {
        return Ok(());
    };
    let site = site.ok_or_else(|| invalid("C# source regions require an exact owner source"))?;
    let owner = fact["symbolIdentity"].as_str().unwrap_or_default();
    let file = fact["file"].as_str().unwrap_or_default();
    let start = fact["byteStart"]
        .as_u64()
        .ok_or_else(|| invalid("C# region owner start missing"))? as usize;
    let end = fact["byteEnd"]
        .as_u64()
        .ok_or_else(|| invalid("C# region owner end missing"))? as usize;
    let graph = validate(raw, owner, file, text, start, end)?;
    let id = dependency_id(&e.service, "source-regions", &scoped_identity(scope, owner))?;
    let normalized = json!({"schema":EVIDENCE,"kind":KIND,"ownerId":owner_id,"scope":scope,"ownerSymbolIdentity":owner,"file":file,"compilerFactBinding":binding,"sourceSite":site,"fullCompilationSourceDigest":canonical::hash_bytes(text.as_bytes()),"graph":graph,"relationships":relationships(&graph)});
    e.observations.insert(
        id.clone(),
        Observation {
            id,
            kind: KIND.into(),
            service: e.service.clone(),
            symbol: owner.into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids: vec![
                site["sourceId"]
                    .as_str()
                    .ok_or_else(|| invalid("C# region owner source ID missing"))?
                    .into(),
            ],
        },
    );
    Ok(())
}

pub(super) fn verify(e: &ServiceEvidence, observation: &Observation) -> Result<(), ClewError> {
    if observation.kind != KIND {
        return Ok(());
    }
    let n = &observation.normalized;
    let scope = n["scope"]
        .as_str()
        .ok_or_else(|| invalid("C# region scope missing"))?;
    let owner = e
        .observations
        .get(n["ownerId"].as_str().unwrap_or_default())
        .ok_or_else(|| invalid("C# region owner missing"))?;
    let body = e
        .sources
        .get(n["sourceSite"]["sourceId"].as_str().unwrap_or_default())
        .ok_or_else(|| invalid("C# region body missing"))?;
    let unit = e
        .observations
        .values()
        .find(|o| {
            o.kind == "SOURCE_FILE"
                && o.normalized["scope"] == scope
                && o.normalized["file"] == n["file"]
        })
        .and_then(|o| o.source_ids.first())
        .and_then(|id| e.sources.get(id))
        .ok_or_else(|| invalid("C# region compilation unit missing"))?;
    let start = n["sourceSite"]["byteStart"]
        .as_u64()
        .ok_or_else(|| invalid("C# region body start missing"))? as usize;
    let end = n["sourceSite"]["byteEnd"]
        .as_u64()
        .ok_or_else(|| invalid("C# region body end missing"))? as usize;
    if n["schema"] != EVIDENCE
        || n["kind"] != KIND
        || owner.kind != "SYMBOL"
        || owner.normalized["schema"] != super::super::csharp::CSHARP_FACT_SCHEMA
        || owner.symbol != observation.symbol
        || n["ownerSymbolIdentity"] != owner.symbol
        || owner.normalized["scope"] != scope
        || owner.normalized["outlineOwnerSource"] != n["sourceSite"]
        || observation.service != e.service
        || observation.source_ids != vec![body.id.clone()]
        || unit.file != n["file"]
        || body.file != unit.file
        || body.revision != unit.revision
        || n["fullCompilationSourceDigest"] != unit.text_digest
        || n["compilerFactBinding"] != body.evidence_digest
        || n["sourceSite"]["sourceDigest"] != body.text_digest
        || n["sourceSite"]["evidenceDigest"] != body.evidence_digest
        || unit.text.get(start..end) != Some(body.text.as_str())
        || observation.id
            != dependency_id(
                &e.service,
                "source-regions",
                &scoped_identity(scope, &owner.symbol),
            )?
    {
        return Err(invalid(
            "C# source-region immutable owner/scope/source binding changed",
        ));
    }
    let g = validate(
        &n["graph"],
        &owner.symbol,
        &unit.file,
        &unit.text,
        start,
        end,
    )?;
    if n["relationships"] != relationships(&g) {
        return Err(invalid(
            "C# source-region conditional relationships changed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "method:class:Neutral#Run()V";

    fn span(text: &str, needle: &str) -> Value {
        let start = text.find(needle).unwrap();
        let end = start + needle.len();
        let units = |offset| {
            text[..offset]
                .trim_start_matches('\u{feff}')
                .encode_utf16()
                .count()
        };
        json!({"file":"Unit.cs","start":units(start),"end":units(end),
            "byteStart":start,"byteEnd":end,
            "startLine":text.as_bytes()[..start].iter().filter(|b|**b==b'\n').count()+1,
            "endLine":text.as_bytes()[..end-1].iter().filter(|b|**b==b'\n').count()+1})
    }
    fn node(
        text: &str,
        id: usize,
        parent: Option<usize>,
        role: &str,
        kind: &str,
        needle: &str,
    ) -> Value {
        json!({"id":id,"parent":parent,"role":role,"kind":kind,"source":span(text,needle)})
    }
    fn graph(nodes: Vec<Value>, sites: Vec<Value>, expressions: Vec<Value>) -> Value {
        json!({"schema":SCHEMA,"authority":"ROSLYN_SOURCE_REGIONS","interpretation":"LEXICAL_AND_CONDITIONAL_NORMAL_ONLY",
            "execution":"NOT_ASSERTED","ownerSymbolIdentity":OWNER,"file":"Unit.cs","nodes":nodes,"sites":sites,"expressions":expressions,
            "boundaries":[],"exceptionCompleteness":"UNKNOWN","interproceduralCompletion":"NOT_MODELED_USE_ORIGINAL_SOURCE"})
    }
    fn read(raw: &Value, text: &str) -> Graph {
        validate(raw, OWNER, "Unit.cs", text, 0, text.len()).unwrap()
    }
    fn pair(inside: bool) -> (String, Value) {
        let statement = if inside {
            "try { await A(); } catch { Log(); await Pause(); }"
        } else {
            "try { await A(); } catch { Log(); } await Pause();"
        };
        let text = format!("\u{feff}{{\r\n// π🙂\r\n{statement}\r\n}}");
        let body = &text[3..];
        let guarded = if inside {
            "try { await A(); } catch { Log(); await Pause(); }"
        } else {
            "try { await A(); } catch { Log(); }"
        };
        let caught = if inside {
            "catch { Log(); await Pause(); }"
        } else {
            "catch { Log(); }"
        };
        let caught_body = if inside {
            "{ Log(); await Pause(); }"
        } else {
            "{ Log(); }"
        };
        let raw = graph(
            vec![
                node(&text, 0, None, "BODY", "BLOCK", body),
                node(&text, 1, Some(0), "STATEMENT", "TRY", guarded),
                node(&text, 2, Some(1), "TRY_BODY", "BLOCK", "{ await A(); }"),
                node(&text, 3, Some(2), "STATEMENT", "EXPRESSION", "await A();"),
                node(&text, 4, Some(1), "CATCH", "CATCH", caught),
                node(&text, 5, Some(4), "CATCH_BODY", "BLOCK", caught_body),
                node(&text, 6, Some(5), "STATEMENT", "EXPRESSION", "Log();"),
                node(
                    &text,
                    7,
                    Some(if inside { 5 } else { 0 }),
                    "STATEMENT",
                    "EXPRESSION",
                    "await Pause();",
                ),
            ],
            vec![
                json!({"region":3,"kind":"AWAIT","source":span(&text,"await A()")}),
                json!({"region":7,"kind":"AWAIT","source":span(&text,"await Pause()")}),
            ],
            vec![],
        );
        (text, raw)
    }
    #[test]
    fn normal_shared_continuation_is_distinct_from_catch_only_and_await_protection() {
        let (text, raw) = pair(false);
        let rel = relationships(&read(&raw, &text));
        for from in [1, 2, 4, 5] {
            assert!(
                rel["normalContinuation"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["from"] == from && r["to"] == 7)
            );
        }
        assert!(
            rel["lexicalNext"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["from"] == 1 && r["to"] == 7)
        );
        assert_eq!(rel["siteContexts"][0]["protectedTry"], 1);
        assert!(rel["siteContexts"][1]["protectedTry"].is_null());
        let (text, raw) = pair(true);
        let rel = relationships(&read(&raw, &text));
        assert!(
            !rel["normalContinuation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["from"] == 1 && r["to"] == 7)
        );
        assert!(
            rel["normalContinuation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["from"] == 6 && r["to"] == 7)
        );
    }
    #[test]
    fn abrupt_exits_have_scope_and_never_normal_successors() {
        let text = "{ while (p) { if (q) return; break; continue; } Later(); }";
        let mut loop_node = node(
            text,
            1,
            Some(0),
            "STATEMENT",
            "LOOP",
            "while (p) { if (q) return; break; continue; }",
        );
        loop_node["loopKind"] = json!("WHILE");
        loop_node["continuePhase"] = json!("CONDITION");
        let raw = graph(
            vec![
                node(text, 0, None, "BODY", "BLOCK", text),
                loop_node,
                node(
                    text,
                    2,
                    Some(1),
                    "LOOP_BODY",
                    "BLOCK",
                    "{ if (q) return; break; continue; }",
                ),
                node(text, 3, Some(2), "STATEMENT", "IF", "if (q) return;"),
                node(text, 4, Some(3), "THEN", "RETURN", "return;"),
                node(text, 5, Some(2), "STATEMENT", "BREAK", "break;"),
                node(text, 6, Some(2), "STATEMENT", "CONTINUE", "continue;"),
                node(text, 7, Some(0), "STATEMENT", "EXPRESSION", "Later();"),
            ],
            vec![],
            vec![],
        );
        let rel = relationships(&read(&raw, text));
        for id in [2, 4, 5, 6] {
            assert!(
                !rel["normalContinuation"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["from"] == id)
            );
        }
        assert_eq!(rel["abrupt"][0]["targetScope"], 0);
        assert_eq!(rel["abrupt"][1]["targetScope"], 1);
        assert_eq!(rel["abrupt"][2]["continuePhase"], "CONDITION");
    }
    #[test]
    fn finally_is_an_opaque_exit_barrier_without_hiding_try_return() {
        let text = "{ try { return; } finally { throw new System.Exception(); } Later(); }";
        let mut guarded = node(
            text,
            1,
            Some(0),
            "STATEMENT",
            "TRY",
            "try { return; } finally { throw new System.Exception(); }",
        );
        guarded["hasFinally"] = json!(true);
        guarded["exitBoundary"] = json!(4);
        let mut cleanup = node(
            text,
            4,
            Some(1),
            "FINALLY",
            "OPAQUE",
            "finally { throw new System.Exception(); }",
        );
        cleanup["opaque"] = json!("FINALLY_COMPLETION_NOT_MODELED");
        let raw = graph(
            vec![
                node(text, 0, None, "BODY", "BLOCK", text),
                guarded,
                node(text, 2, Some(1), "TRY_BODY", "BLOCK", "{ return; }"),
                node(text, 3, Some(2), "STATEMENT", "RETURN", "return;"),
                cleanup,
                node(text, 5, Some(0), "STATEMENT", "EXPRESSION", "Later();"),
            ],
            vec![],
            vec![],
        );
        let rel = relationships(&read(&raw, text));
        assert_eq!(rel["abrupt"][0]["unknownCleanupBoundaries"], json!([4]));
        assert!(
            !rel["normalContinuation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["from"] == 1 || r["from"] == 4)
        );
    }
    #[test]
    fn using_retains_nested_empty_return_and_unknown_cleanup_instead_of_normal_proof() {
        let text = "{ using (resource) { if (empty) return; Work(); } Later(); }";
        let mut resource = node(
            text,
            1,
            Some(0),
            "STATEMENT",
            "USING",
            "using (resource) { if (empty) return; Work(); }",
        );
        resource["opaque"] = json!("USING_EXIT_CLEANUP_NOT_MODELED");
        let raw = graph(
            vec![
                node(text, 0, None, "BODY", "BLOCK", text),
                resource,
                node(
                    text,
                    2,
                    Some(1),
                    "USING_BODY",
                    "BLOCK",
                    "{ if (empty) return; Work(); }",
                ),
                node(text, 3, Some(2), "STATEMENT", "IF", "if (empty) return;"),
                node(text, 4, Some(3), "THEN", "RETURN", "return;"),
                node(text, 5, Some(2), "STATEMENT", "EXPRESSION", "Work();"),
                node(text, 6, Some(0), "STATEMENT", "EXPRESSION", "Later();"),
            ],
            vec![],
            vec![],
        );
        let rel = relationships(&read(&raw, text));
        assert_eq!(rel["abrupt"][0]["unknownCleanupBoundaries"], json!([1]));
        assert!(
            rel["normalContinuation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["from"] == 3 && r["to"] == 5)
        );
        assert!(
            !rel["normalContinuation"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["from"] == 1)
        );
    }
    #[test]
    fn malformed_source_scope_parent_budget_and_kotlin_authority_fail_closed() {
        let (text, raw) = pair(false);
        for change in [
            "byte",
            "utf16",
            "parent",
            "authority",
            "owner",
            "file",
            "budget",
        ] {
            let mut bad = raw.clone();
            match change {
                "byte" => bad["nodes"][3]["source"]["byteStart"] = json!(0),
                "utf16" => bad["nodes"][3]["source"]["start"] = json!(0),
                "parent" => bad["nodes"][3]["parent"] = json!(3),
                "authority" => bad["authority"] = json!("K2_FIR_CFG"),
                "owner" => bad["ownerSymbolIdentity"] = json!("other"),
                "file" => bad["file"] = json!("other.cs"),
                _ => {
                    bad["nodes"] = json!(vec![raw["nodes"][0].clone(); 97]);
                }
            }
            assert!(
                validate(&bad, OWNER, "Unit.cs", &text, 0, text.len()).is_err(),
                "{change}"
            );
        }
    }

    #[test]
    fn break_and_continue_do_not_exit_an_outer_using_scope() {
        let text = "{ using (resource) { while (p) { break; continue; return; } } }";
        let mut resource = node(
            text,
            1,
            Some(0),
            "STATEMENT",
            "USING",
            "using (resource) { while (p) { break; continue; return; } }",
        );
        resource["opaque"] = json!("USING_EXIT_CLEANUP_NOT_MODELED");
        let mut loop_node = node(
            text,
            3,
            Some(2),
            "STATEMENT",
            "LOOP",
            "while (p) { break; continue; return; }",
        );
        loop_node["loopKind"] = json!("WHILE");
        loop_node["continuePhase"] = json!("CONDITION");
        let raw = graph(
            vec![
                node(text, 0, None, "BODY", "BLOCK", text),
                resource,
                node(
                    text,
                    2,
                    Some(1),
                    "USING_BODY",
                    "BLOCK",
                    "{ while (p) { break; continue; return; } }",
                ),
                loop_node,
                node(
                    text,
                    4,
                    Some(3),
                    "LOOP_BODY",
                    "BLOCK",
                    "{ break; continue; return; }",
                ),
                node(text, 5, Some(4), "STATEMENT", "BREAK", "break;"),
                node(text, 6, Some(4), "STATEMENT", "CONTINUE", "continue;"),
                node(text, 7, Some(4), "STATEMENT", "RETURN", "return;"),
            ],
            vec![],
            vec![],
        );
        let rel = relationships(&read(&raw, text));
        assert_eq!(rel["abrupt"][0]["unknownCleanupBoundaries"], json!([]));
        assert_eq!(rel["abrupt"][1]["unknownCleanupBoundaries"], json!([]));
        assert_eq!(rel["abrupt"][2]["unknownCleanupBoundaries"], json!([1]));
    }

    #[test]
    fn immutable_source_owner_scope_digest_and_derived_relationships_are_rechecked() {
        let (text, raw) = pair(false);
        let hash = canonical::hash_bytes(text.as_bytes());
        let source = |id: &str| {
            json!({"id":id,"service":"neutral","revision":"r1","file":"Unit.cs",
            "startLine":1,"endLine":5,"text":text,"textDigest":hash,"evidenceDigest":"binding","authority":"COMPILER","url":null})
        };
        let site = json!({"sourceId":"body","file":"Unit.cs","byteStart":0,"byteEnd":text.len(),"sourceDigest":hash,"evidenceDigest":"binding"});
        let owner = Observation {
            id: "owner".into(),
            kind: "SYMBOL".into(),
            service: "neutral".into(),
            symbol: OWNER.into(),
            normalized: json!({"schema":super::super::super::csharp::CSHARP_FACT_SCHEMA,"scope":"main","outlineOwnerSource":site}),
            digest: "owner-digest".into(),
            source_ids: vec!["body".into()],
        };
        let unit = Observation {
            id: "unit".into(),
            kind: "SOURCE_FILE".into(),
            service: "neutral".into(),
            symbol: "Unit.cs".into(),
            normalized: json!({"scope":"main","file":"Unit.cs"}),
            digest: "unit-digest".into(),
            source_ids: vec!["unit".into()],
        };
        let mut e: ServiceEvidence = serde_json::from_value(json!({"schema":"test","service":"neutral","revision":"r1","serviceDigest":"test",
            "extractor":"test","runtimeMode":"test","coverage":"test","boundaries":[],"entrypoints":[],"observations":{},"sources":{"body":source("body"),"unit":source("unit")},"contracts":{}})).unwrap();
        e.observations.insert(owner.id.clone(), owner);
        e.observations.insert(unit.id.clone(), unit);
        let fact = json!({"symbolIdentity":OWNER,"file":"Unit.cs","byteStart":0,"byteEnd":text.len(),"documentation":{"controlRegions":raw}});
        project(
            &mut e,
            &fact,
            "binding",
            "main",
            "owner",
            Some(&site),
            &text,
        )
        .unwrap();
        let record = e
            .observations
            .values()
            .find(|o| o.kind == KIND)
            .unwrap()
            .clone();
        verify(&e, &record).unwrap();
        for key in [
            "ownerId",
            "scope",
            "fullCompilationSourceDigest",
            "compilerFactBinding",
            "sourceSite",
            "relationships",
        ] {
            let mut changed = record.clone();
            changed.normalized[key] = json!("tampered");
            changed.digest = digest(&changed.normalized).unwrap();
            assert!(verify(&e, &changed).is_err(), "{key}");
        }
        let mut changed = e.clone();
        changed
            .sources
            .get_mut("unit")
            .unwrap()
            .text
            .replace_range(3..4, "[");
        assert!(verify(&changed, &record).is_err());
    }
}
