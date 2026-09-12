//! First built-in typed view: declared entities and evidence-bound static data flow.
use super::{
    bytes,
    check::Check,
    cli::ListArgs,
    digest, invalid, io_error,
    model::*,
    proposals::Claim,
    store::{self, Repository},
    work,
};
use crate::error::{ClewError, ErrorCode};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
pub const ROOT: &str = "entity-dataflow";
pub const MODULE: &str = "entity-dataflow/1.0";
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HumanMaterial {
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default)]
    pub layout: BTreeMap<String, Position>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Position {
    pub column: u8,
    pub row: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Details {
    pub module: String,
    pub input_objects: Vec<String>,
    pub services: Vec<String>,
    #[serde(default)]
    pub contracts: Vec<String>,
    #[serde(default)]
    pub related_processes: Vec<String>,
    pub scope: String,
    #[serde(default)]
    pub human: HumanMaterial,
    #[serde(default)]
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub entity: String,
    pub kind: String,
    pub service: String,
    pub representation: String,
    pub meaning: Fragment,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub authority: String,
    pub match_basis: String,
    pub meaning: Fragment,
    pub uncertainty: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Graph {
    pub schema: String,
    pub view: String,
    pub definition_digest: String,
    pub module_digest: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedNode {
    pub id: String,
    pub entity: String,
    pub kind: String,
    pub service: String,
    pub representation: String,
    pub meaning: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub authority: String,
    pub match_basis: String,
    pub meaning: Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposedGraph {
    pub nodes: Vec<ProposedNode>,
    pub edges: Vec<ProposedEdge>,
}
pub fn module() -> Result<Value, ClewError> {
    Ok(
        json!({"schema":"codeclew-documentation-view-module/1.0","id":MODULE,"inputObjects":["DOMAIN_ENTITY","SOURCE","CONTRACT_OPERATION","PROCESS_COMPONENT"],"representationKinds":["domain","dto","message","table","field","function"],"edgeKinds":["read","transform","write","transfer","candidate"],"authority":["SOURCE_INTERPRETATION","DECLARED_TRANSFER","UNKNOWN"],"dependencyDerivation":"Explicit node/edge claims, definition and domain IDs, source/module/contract scopes, linked accepted component versions and all captured work influence.","validation":"entity-dataflow-validation/1.0","renderer":"entity-dataflow-svg/1.0","executable":false,"implementationDigest":digest(&(include_str!("dataflow.rs"),include_str!("proposals.rs"),include_str!("../../assets/documentation/app.js")))?,"limitations":["Static interpretation; not a runtime trace or universal taint analysis.","Name equality is only an unknown candidate; declared transfers do not prove wire compatibility.","Representation-to-domain mapping is an agent interpretation, not authoritative ownership."]}),
    )
}
pub fn validate_definition(
    s: &Scenario,
    services: &BTreeMap<String, Service>,
) -> Result<(), ClewError> {
    let Some(v) = &s.view else {
        return if s.schema == "codeclew-documentation-view/1.0" {
            Err(invalid("saved view requires typed view metadata"))
        } else {
            Ok(())
        };
    };
    let text = |s: &str| !s.trim().is_empty() && s.len() <= 2048;
    if s.schema != "codeclew-documentation-view/1.0"
        || s.process.is_some()
        || s.id == ROOT
        || v.module != MODULE
        || !text(&s.title)
        || !text(&s.summary)
        || !text(&v.scope)
        || v.input_objects.is_empty()
        || v.input_objects.len() > 32
        || v.input_objects
            .iter()
            .any(|id| !id.strip_prefix("entity:").is_some_and(store::valid_id))
        || v.services.is_empty()
        || v.services.len() > 8
        || !v.services.contains(&s.root.service)
        || v.services.iter().any(|id| !services.contains_key(id))
        || v.contracts.len() > 32
        || v.contracts
            .iter()
            .any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_whitespace))
        || v.related_processes.len() > 32
        || v.related_processes
            .iter()
            .any(|id| !store::valid_id(id) || id == &s.id)
        || [
            &v.input_objects,
            &v.services,
            &v.contracts,
            &v.related_processes,
        ]
        .iter()
        .any(|v| v.iter().collect::<BTreeSet<_>>().len() != v.len())
        || v.limitations.len() > 32
        || v.limitations.iter().any(|v| !text(v))
        || bytes(&v.human)?.len() > 32768
        || v.human.annotations.len() > 64
        || v.human.layout.len() > 64
        || v.human
            .annotations
            .iter()
            .any(|(id, v)| !store::valid_id(id) || !text(v))
        || v.human
            .layout
            .iter()
            .any(|(id, p)| !store::valid_id(id) || p.column > 7 || p.row > 31)
    {
        return Err(invalid(
            "invalid view module, explicit object/service scope, protected material or bounds",
        ));
    }
    Ok(())
}
pub fn is_root(checked: &Check, subject: &str, root: &str) -> bool {
    root == ROOT
        && subject
            .strip_prefix("scenario:")
            .is_some_and(|id| checked.dependencies.contains_key(&format!("view:{id}")))
}
pub fn details(checked: &Check, id: &str) -> Result<Details, ClewError> {
    serde_json::from_value(
        checked
            .dependencies
            .get(&format!("view:{id}"))
            .ok_or_else(|| invalid("unknown saved view"))?
            .normalized["definition"]["view"]
            .clone(),
    )
    .map_err(io_error)
}
#[derive(Debug, Subcommand)]
pub enum Command {
    Modules {
        #[arg(long)]
        root: PathBuf,
    },
    List {
        #[command(flatten)]
        page: ListArgs,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
    },
    Put {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_input_digest: String,
        #[arg(long)]
        human: bool,
    },
    Prepare {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
    },
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    let root = match &command {
        Command::Modules { root }
        | Command::Show { root, .. }
        | Command::Put { root, .. }
        | Command::Prepare { root, .. } => root,
        Command::List { page } => &page.root,
    };
    let repo = Repository::open(root)?;
    match command {
        Command::Modules { .. } => {
            Ok(json!({"items":[module()?],"inputDigest":repo.input_digest()?}))
        }
        Command::List { page } => {
            let rows:Vec<_>=repo.scenarios()?.into_values().filter(|s|s.view.is_some()).map(|s|json!({"id":s.id,"title":s.title,"subject":format!("scenario:{}",s.id),"view":s.view})).collect();
            super::cli::page(
                &digest(&rows)?,
                rows,
                page.cursor.as_deref(),
                page.limit as usize,
                json!({"inputDigest":repo.input_digest()?}),
            )
        }
        Command::Show { id, .. } => {
            let definitions = repo.scenarios()?;
            let s = definitions
                .get(&id)
                .filter(|s| s.view.is_some())
                .ok_or_else(|| invalid("unknown saved view"))?;
            Ok(json!({"definition":s,"module":module()?,"inputDigest":repo.input_digest()?}))
        }
        Command::Put {
            input,
            expected_input_digest,
            human,
            ..
        } => {
            let definition: Scenario = store::read(&input, 256 * 1024)?;
            if definition.view.is_none()
                || !store::valid_id(&definition.id)
                || definition.max_depth > 16
                || definition.max_nodes == 0
                || definition.max_nodes > 512
            {
                return Err(invalid("invalid view identity or traversal bounds"));
            }
            validate_definition(&definition, &repo.services()?)?;
            store::endpoint(&definition.root, &repo.services()?)?;
            let entities = super::entities::records(&repo)?;
            let v = definition.view.as_ref().unwrap();
            if v.input_objects
                .iter()
                .any(|id| !entities.contains_key(&id[7..]))
            {
                return Err(invalid(
                    "view inputs must name existing explicit domain entities",
                ));
            }
            let links = repo.interactions()?;
            if definition
                .interactions
                .iter()
                .any(|id| !links.contains_key(id))
                || definition
                    .interactions
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != definition.interactions.len()
            {
                return Err(invalid(
                    "view has a dangling or duplicate declared interaction",
                ));
            }
            let _lock = repo.lock()?;
            if repo.input_digest()? != expected_input_digest {
                return Err(ClewError::new(
                    ErrorCode::WwConflict,
                    "view inputs changed; reload inputDigest",
                ));
            }
            let existing = repo.scenarios()?;
            let old = existing.get(&definition.id);
            if old.is_some_and(|s| s.view.is_none()) {
                return Err(invalid(
                    "view ID belongs to an existing scenario or process",
                ));
            }
            if !human
                && v.human
                    != old
                        .and_then(|s| s.view.as_ref())
                        .map(|v| v.human.clone())
                        .unwrap_or_default()
            {
                return Err(invalid(
                    "agent view updates cannot add, replace or remove human annotations or layout metadata",
                ));
            }
            repo.atomic(
                &format!("scenarios/{}.yaml", definition.id),
                &bytes(&definition)?,
            )?;
            Ok(
                json!({"status":"SAVED","inputDigest":repo.input_digest()?,"ownership":"Human annotations/layout remain separate from generated graph content"}),
            )
        }
        Command::Prepare { id, .. } => {
            if repo.scenarios()?.get(&id).is_none_or(|s| s.view.is_none()) {
                return Err(invalid("unknown saved view"));
            }
            work::prepare(&repo,format!("scenario:{id}"),serde_json::from_value(json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Domain and service maintainers","entrypoint":ROOT,"maxItems":20,"maxBytes":40960})).map_err(io_error)?)
        }
    }
}
pub fn attach(repo: &Repository, checked: &mut Check) -> Result<(), ClewError> {
    let definitions = repo.scenarios()?;
    let interactions = repo.interactions()?;
    if !definitions.values().any(|s| s.view.is_some()) {
        return Ok(());
    }
    let contract = module()?;
    let module_id = format!("view-module:{MODULE}");
    checked.dependencies.insert(
        module_id.clone(),
        Observation {
            id: module_id.clone(),
            kind: "VIEW_MODULE".into(),
            service: String::new(),
            symbol: MODULE.into(),
            digest: digest(&contract)?,
            normalized: contract,
            source_ids: vec![],
        },
    );
    for (id, s) in &definitions {
        let Some(v) = &s.view else {
            continue;
        };
        let key = format!("view:{id}");
        let missing: Vec<_> = v
            .input_objects
            .iter()
            .chain(&v.contracts)
            .filter(|id| !checked.dependencies.contains_key(*id))
            .collect();
        let deps: Vec<_> = v
            .input_objects
            .iter()
            .chain(&v.contracts)
            .cloned()
            .chain([format!("scenario:{id}"), module_id.clone()])
            .collect();
        let normalized = json!({"definition":s,"dependencyIds":deps,"missingInputs":missing,"authority":"EXPLICIT_REQUEST_AND_PROTECTED_HUMAN_METADATA"});
        checked.dependencies.insert(
            key.clone(),
            Observation {
                id: key.clone(),
                kind: "VIEW_DEFINITION".into(),
                service: String::new(),
                symbol: s.title.clone(),
                digest: digest(&normalized)?,
                normalized,
                source_ids: vec![],
            },
        );
        let mut deps: BTreeSet<_> = checked
            .dependencies
            .values()
            .filter(|d| {
                v.services.contains(&d.service)
                    && matches!(
                        d.kind.as_str(),
                        "SOURCE_SCOPE"
                            | "MODULE_SCOPE"
                            | "CONTRACT_SCOPE"
                            | "ENTITY_SCOPE"
                            | "NOTE_SCOPE"
                    )
            })
            .map(|d| d.id.clone())
            .collect();
        deps.insert(key.clone());
        deps.extend(v.related_processes.iter().map(|p| format!("scenario:{p}")));
        let unavailable: Vec<_> = v
            .services
            .iter()
            .filter(|id| !checked.services.contains_key(*id))
            .cloned()
            .collect();
        let scope = format!("view-scope:{id}");
        let interaction_membership: Vec<_> = interactions
            .values()
            .filter(|i| v.services.contains(&i.from.service) && v.services.contains(&i.to.service))
            .map(|i| &i.id)
            .collect();
        let normalized = json!({"dependencyIds":deps,"unavailableServices":unavailable,"interactionMembership":interaction_membership,"authority":"BOUNDED_STATIC_VIEW_SCOPE"});
        checked.dependencies.insert(
            scope.clone(),
            Observation {
                id: scope.clone(),
                kind: "VIEW_SCOPE".into(),
                service: String::new(),
                symbol: id.clone(),
                digest: digest(&normalized)?,
                normalized,
                source_ids: vec![],
            },
        );
        if let Some(context) = checked.scenarios.get_mut(id) {
            context.dependency_ids.extend(
                v.input_objects
                    .iter()
                    .chain(&v.contracts)
                    .filter(|id| checked.dependencies.contains_key(*id))
                    .cloned(),
            );
            context
                .dependency_ids
                .extend([key, scope, module_id.clone()]);
            context.boundaries.extend(
                unavailable
                    .iter()
                    .map(|s| format!("VIEW_SERVICE_UNAVAILABLE:{s}")),
            );
        }
    }
    checked.refresh_digest()
}
pub fn materialize(
    checked: &Check,
    id: &str,
    p: &ProposedGraph,
    mut claim: impl FnMut(&str, &Claim) -> Result<Fragment, ClewError>,
) -> Result<Graph, ClewError> {
    if p.nodes.is_empty() || p.nodes.len() > 64 || p.edges.len() > 128 {
        return Err(invalid(
            "data-flow view requires 1..64 nodes and at most 128 edges",
        ));
    }
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for node in &p.nodes {
        nodes.push(Node {
            id: node.id.clone(),
            entity: node.entity.clone(),
            kind: node.kind.clone(),
            service: node.service.clone(),
            representation: node.representation.clone(),
            meaning: claim(&format!("node-{}", node.id), &node.meaning)?,
        });
    }
    for edge in &p.edges {
        edges.push(Edge {
            id: edge.id.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
            kind: edge.kind.clone(),
            authority: edge.authority.clone(),
            match_basis: edge.match_basis.clone(),
            meaning: claim(&format!("edge-{}", edge.id), &edge.meaning)?,
            uncertainty: edge.meaning.uncertainty.clone().unwrap_or_default(),
        });
    }
    Ok(Graph {
        schema: "codeclew-documentation-dataflow/1.0".into(),
        view: id.into(),
        definition_digest: checked.dependencies[&format!("view:{id}")].digest.clone(),
        module_digest: checked.dependencies[&format!("view-module:{MODULE}")]
            .digest
            .clone(),
        nodes,
        edges,
    })
}
fn supported(
    f: &Fragment,
    checked: &Check,
    services: &BTreeSet<String>,
    declaration: bool,
    sources: &BTreeMap<String, Source>,
) -> Result<(), ClewError> {
    if f.text.trim().is_empty()
        || f.text.len() > 2048
        || f.dependency_ids.is_empty()
        || f.dependency_ids.len() > 128
        || f.source_ids.len() > 32
    {
        return Err(invalid(
            "view claim needs bounded text and dependency evidence",
        ));
    }
    let mut supported = BTreeSet::new();
    for id in &f.dependency_ids {
        let d = checked
            .dependencies
            .get(id)
            .ok_or_else(|| invalid("view claim dependency is unavailable"))?;
        if !d.service.is_empty() && !services.contains(&d.service) {
            return Err(invalid("view claim crosses its explicit service scope"));
        }
        supported.extend(d.source_ids.iter().cloned());
    }
    if f.source_ids.iter().any(|id| {
        !supported.contains(id)
            || sources
                .get(id)
                .is_none_or(|s| !services.contains(&s.service))
    }) {
        return Err(invalid(
            "view source is not bound to its evidence and service scope",
        ));
    }
    if f.source_ids.is_empty()
        && (!declaration
            || f.dependency_ids.iter().any(|id| {
                !matches!(
                    checked.dependencies[id].kind.as_str(),
                    "DOMAIN_ENTITY" | "VIEW_DEFINITION"
                )
            }))
    {
        return Err(invalid(
            "source-free view claims must remain explicit domain declarations or unknown candidates",
        ));
    }
    Ok(())
}
pub fn validate_graph(o: &Operation, checked: &Check, subject: &str) -> Result<(), ClewError> {
    let id = subject
        .strip_prefix("scenario:")
        .ok_or_else(|| invalid("view requires its saved subject"))?;
    let g = o
        .dataflow
        .as_ref()
        .ok_or_else(|| invalid("view root requires a typed data-flow graph"))?;
    let v = details(checked, id)?;
    let services = v.services.iter().cloned().collect();
    let sources = checked.sources();
    if g.schema != "codeclew-documentation-dataflow/1.0"
        || g.view != id
        || g.definition_digest != checked.dependencies[&format!("view:{id}")].digest
        || g.module_digest != checked.dependencies[&format!("view-module:{MODULE}")].digest
        || g.nodes.is_empty()
        || g.nodes.len() > 64
        || g.edges.len() > 128
    {
        return Err(invalid(
            "view schema, captured definition/module or graph bounds mismatch",
        ));
    }
    if !o.summary.dependency_ids.contains(&format!("view:{id}")) {
        return Err(invalid(
            "view summary must cite its explicit definition and source evidence",
        ));
    }
    supported(&o.summary, checked, &services, false, &sources)?;
    let mut ids = BTreeSet::new();
    let mut claims = BTreeSet::from([o.summary.id.clone()]);
    let mut nodes = BTreeMap::new();
    for n in &g.nodes {
        if !store::valid_id(&n.id)
            || !ids.insert(n.id.clone())
            || !v.input_objects.contains(&n.entity)
            || !v.services.contains(&n.service)
            || !matches!(
                n.kind.as_str(),
                "domain" | "dto" | "message" | "table" | "field" | "function"
            )
            || n.representation.trim().is_empty()
            || n.representation.len() > 512
            || n.meaning.text.len() > 512
            || !claims.insert(n.meaning.id.clone())
        {
            return Err(invalid("invalid, duplicate or out-of-scope data-flow node"));
        }
        supported(&n.meaning, checked, &services, n.kind == "domain", &sources)?;
        if n.kind != "domain"
            && n.meaning
                .source_ids
                .iter()
                .any(|id| sources.get(id).is_none_or(|s| s.service != n.service))
        {
            return Err(invalid(
                "representation source must belong to its declared service",
            ));
        }
        if n.kind == "domain"
            && (n.representation != n.entity
                || !n.meaning.dependency_ids.contains(&n.entity)
                || checked
                    .dependencies
                    .get(&n.entity)
                    .is_none_or(|d| d.normalized["entity"]["title"] != n.meaning.text))
        {
            return Err(invalid(
                "domain nodes must use the explicit entity ID and declared title",
            ));
        }
        nodes.insert(n.id.as_str(), n);
    }
    for e in &g.edges {
        let from = nodes
            .get(e.from.as_str())
            .ok_or_else(|| invalid("data-flow edge source node is missing"))?;
        let to = nodes
            .get(e.to.as_str())
            .ok_or_else(|| invalid("data-flow edge target node is missing"))?;
        if !store::valid_id(&e.id)
            || !ids.insert(e.id.clone())
            || !claims.insert(e.meaning.id.clone())
            || e.uncertainty.len() > 2048
        {
            return Err(invalid("invalid or duplicate edge/claim identity"));
        }
        supported(
            &e.meaning,
            checked,
            &services,
            e.authority == "UNKNOWN",
            &sources,
        )?;
        match (
            e.kind.as_str(),
            e.authority.as_str(),
            e.match_basis.as_str(),
        ) {
            ("candidate", "UNKNOWN", "name-only" | "unknown")
                if !e.uncertainty.trim().is_empty() => {}
            ("read" | "transform" | "write", "SOURCE_INTERPRETATION", "source-dataflow")
                if from.service == to.service
                    && e.meaning.dependency_ids.iter().any(|id| {
                        matches!(
                            checked.dependencies[id].kind.as_str(),
                            "FLOW" | "SYMBOL" | "SEMANTIC_SYMBOL"
                        ) && checked.dependencies[id].service == from.service
                    }) => {}
            ("transfer", "DECLARED_TRANSFER", "declared-contract")
                if !e.uncertainty.trim().is_empty()
                    && e.meaning.dependency_ids.iter().any(|id| {
                        let d = &checked.dependencies[id];
                        d.kind == "DECLARED_INTERACTION"
                            && d.normalized["from"]["service"] == from.service
                            && d.normalized["to"]["service"] == to.service
                            && d.normalized["declaration"]["origin"] != "agent-proposal"
                    }) => {}
            _ => {
                return Err(invalid(
                    "edge authority is unsupported: name equality is UNKNOWN, static changes need source and transfers need an explicit declared link",
                ));
            }
        }
    }
    if checked.scenarios[id]
        .boundaries
        .iter()
        .any(|b| !o.boundaries.contains(b))
    {
        return Err(invalid(
            "data-flow view must retain every current source and composition boundary",
        ));
    }
    Ok(())
}
pub fn page(checked: &Check, subject: &str) -> Value {
    let Some(id) = subject.strip_prefix("scenario:") else {
        return Value::Null;
    };
    let Some(def) = checked.dependencies.get(&format!("view:{id}")) else {
        return Value::Null;
    };
    json!({"definition":def.normalized["definition"],"definitionDigest":def.digest,"missingInputs":def.normalized["missingInputs"],"module":checked.dependencies.get(&format!("view-module:{MODULE}")).map(|d|&d.normalized),"relatedComponents":checked.dependencies.values().filter(|d|d.kind=="PROCESS_COMPONENT"&&d.normalized["parent"]==id).map(|d|&d.normalized).collect::<Vec<_>>()})
}
pub fn mark_targets(data: &mut Value, checked: &Check, subject: &str) {
    if data["view"].is_null() {
        return;
    }
    let mut old = data["view"].clone();
    old.as_object_mut().unwrap().remove("targetChanged");
    data["view"]["targetChanged"] = json!(old != page(checked, subject));
}
pub fn markdown(view: &Value, n: &Narrative) -> String {
    if view.is_null() {
        return String::new();
    }
    let mut out="\n## Saved entity data-flow view\n\nStatic source interpretation, not a runtime trace or universal taint analysis. Name-only matches remain unknown candidates.\n\n".to_owned();
    out.push_str(&format!(
        "Requested scope: {}\n\n",
        super::render::escape(view["definition"]["view"]["scope"].as_str().unwrap_or(""))
    ));
    if view["targetChanged"] == true {
        out.push_str("The captured definition or its related components have changed; retained graph content needs review.\n\n");
    }
    if let Some(g) = n.operations.iter().find_map(|o| o.dataflow.as_ref()) {
        out.push_str("| Node | Domain identity | Representation kind | Representation |\n| --- | --- | --- | --- |\n");
        for node in &g.nodes {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                super::render::escape(&node.id),
                super::render::escape(&node.entity),
                super::render::escape(&node.kind),
                super::render::escape(&node.representation).replace('|', "&#124;")
            ));
        }
        out.push_str(
            "\n| Edge | Kind / authority | Explanation and uncertainty |\n| --- | --- | --- |\n",
        );
        for e in &g.edges {
            out.push_str(&format!(
                "| {} → {} | {} / {} | {} {} |\n",
                super::render::escape(&e.from),
                super::render::escape(&e.to),
                super::render::escape(&e.kind),
                super::render::escape(&e.authority),
                super::render::escape(&e.meaning.text).replace('|', "&#124;"),
                super::render::escape(&e.uncertainty).replace('|', "&#124;")
            ));
        }
    }
    out.push_str("\n### Protected human annotations and layout\n\n");
    out.push_str(&format!(
        "```json\n{}\n```\n",
        serde_json::to_string_pretty(&view["definition"]["view"]["human"]).unwrap_or_default()
    ));
    out
}
pub fn mermaid(graph: &Graph) -> String {
    let label = |s: &str| {
        s.replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace(['\n', '\r'], " ")
    };
    let mut out="flowchart LR\n    %% Static entity data flow; not a runtime trace or universal taint analysis.\n".to_owned();
    let mut ids = BTreeMap::new();
    for (i, n) in graph.nodes.iter().enumerate() {
        ids.insert(&n.id, i);
        out.push_str(&format!(
            "    %% node {}: dependencies {}\n    n{i}[\"{}: {}\"]\n",
            n.id,
            n.meaning.dependency_ids.join(","),
            label(&n.kind),
            label(&n.meaning.text)
        ));
    }
    for e in &graph.edges {
        out.push_str(&format!(
            "    %% edge {}: dependencies {}\n    n{} {}|\"{} / {}: {}\"| n{}\n",
            e.id,
            e.meaning.dependency_ids.join(","),
            ids[&e.from],
            if e.authority == "UNKNOWN" {
                "-.->"
            } else {
                "-->"
            },
            label(&e.kind),
            label(&e.authority),
            label(&e.meaning.text),
            ids[&e.to]
        ));
    }
    out
}
