//! Read-only declaration comparison. No recapture, network, or runtime verdict.
use super::{check::Check, invalid, model::*, store::Repository};
use crate::error::ClewError;
use clap::Subcommand;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const SCHEMA: &str = "codeclew-route-contract-report/1.0";
const METHODS: &[&str] = &[
    "GET", "PUT", "POST", "DELETE", "OPTIONS", "HEAD", "PATCH", "TRACE", "CONNECT",
];

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report both directions over retained declarations; success means the report was produced, not API conformance.
    Check {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        snapshot: String,
        /// Explicit service selection from this snapshot; repeat for several services.
        #[arg(long = "service", required = true)]
        services: Vec<String>,
    },
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    let Command::Check {
        root,
        snapshot,
        services,
    } = command;
    let repo = Repository::open(&root)?;
    // Unlike latest/current readers, this permits historical inputs. Nothing is saved.
    let checked = Check::load_snapshot(&repo, &snapshot)?;
    report(&checked, &snapshot, &services.into_iter().collect())
}

#[derive(Debug)]
struct Route<'a> {
    entry: &'a Entrypoint,
    method: String,
    path: String,
    key: Option<String>,
    unknown: BTreeSet<String>,
}

// Only simple whole-segment OpenAPI/Spring templates are equivalent here.
// Wildcards, regex, prefixes and slash rules require a framework-specific proof.
fn path_key(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if let Some(name) = part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
            if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                return None;
            }
            parts.push("{}");
        } else {
            if part.chars().any(|c| "{}*?:#$%\\".contains(c)) {
                return None;
            }
            parts.push(part);
        }
    }
    Some(parts.join("/"))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn routes(e: &ServiceEvidence) -> Vec<Route<'_>> {
    let mut result = Vec::new();
    for entry in &e.entrypoints {
        if !matches!(entry.kind.as_str(), "HTTP_ENDPOINT" | "SOURCE_HTTP_MAPPING") {
            continue;
        }
        let mut pairs = BTreeSet::new();
        let mut unknown = BTreeSet::new();
        if let Some(rows) = entry.trigger["sourceRoutes"].as_array() {
            for row in rows {
                if let (Some(method), Some(path)) = (row["method"].as_str(), row["path"].as_str()) {
                    pairs.insert((method.to_uppercase(), path.to_owned()));
                } else {
                    unknown.insert("UNREADABLE_CORRELATED_ROUTE".into());
                }
            }
        } else {
            let methods = strings(&entry.trigger["methods"]);
            let paths = strings(&entry.trigger["paths"]);
            if methods.len() > 1 && paths.len() > 1 {
                unknown.insert("METHOD_PATH_CORRELATION_UNAVAILABLE".into());
            } else if methods.is_empty() {
                unknown.insert("HTTP_METHOD_UNSPECIFIED".into());
                for path in paths {
                    pairs.insert((String::new(), path));
                }
            } else {
                for method in methods {
                    for path in &paths {
                        pairs.insert((method.to_uppercase(), path.clone()));
                    }
                }
            }
        }
        if pairs.is_empty() {
            unknown.insert("NO_COMPARABLE_ROUTE_PAIR".into());
            pairs.insert((String::new(), String::new()));
        }
        for (method, path) in pairs {
            let mut gaps = unknown.clone();
            if !METHODS.contains(&method.as_str()) {
                gaps.insert("HTTP_METHOD_UNKNOWN".into());
            }
            let key = path_key(&path);
            if key.is_none() {
                gaps.insert("PATH_SEMANTICS_UNSUPPORTED".into());
            }
            result.push(Route {
                entry,
                method,
                path,
                key,
                unknown: gaps,
            });
        }
    }
    result
        .sort_by(|a, b| (&a.path, &a.method, &a.entry.id).cmp(&(&b.path, &b.method, &b.entry.id)));
    result
}

fn candidate(e: &ServiceEvidence, entry: &Entrypoint) -> bool {
    e.extractor == SOURCE_EXTRACTOR
        || entry.kind == "SOURCE_HTTP_MAPPING"
        || entry.trigger["authority"] == "FRAMEWORK_DERIVED_SOURCE"
        || !entry.boundaries.is_empty()
}

fn citation(source: &Source) -> Value {
    json!({"id":source.id,"service":source.service,"revision":source.revision,
        "file":source.file,"startLine":source.start_line,"endLine":source.end_line,
        "textDigest":source.text_digest,"evidenceDigest":source.evidence_digest,
        "authority":source.authority,"textOrigin":source.text_origin,"occurrence":source.occurrence,"url":source.url})
}

fn source_ref(e: &ServiceEvidence, route: &Route<'_>) -> Value {
    let entry = route.entry;
    json!({"entrypoint":entry.id,"kind":entry.kind,"symbol":entry.symbol,
        "method":route.method,"path":route.path,"normalizedPath":route.key,
        "authority":entry.trigger["authority"],"candidate":candidate(e,entry),
        "conditions":entry.trigger["conditions"],"trigger":entry.trigger,
        "dependencyReferences":entry.dependency_ids.iter().map(|id| {
            let o = e.observations.get(id);
            json!({"id":id,"kind":o.map(|v| &v.kind),"digest":o.map(|v| &v.digest)})
        }).collect::<Vec<_>>(),"sourceReferences":entry.source_ids,
        "boundaries":entry.boundaries,"unknown":route.unknown})
}

fn pointer(path: &str, method: &str) -> String {
    format!(
        "/paths/{}/{}",
        path.replace('~', "~0").replace('/', "~1"),
        method.to_lowercase()
    )
}

fn contract_ref(e: &ServiceEvidence, o: &Observation) -> Value {
    let logical = pointer(
        o.normalized["path"].as_str().unwrap_or(""),
        o.normalized["method"].as_str().unwrap_or(""),
    );
    let item = logical.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    let document = o.normalized["declaredSource"]
        .as_str()
        .and_then(|file| e.contracts.get(file));
    let (exact, role) = if document.and_then(|d| d.pointer(&logical)).is_some() {
        (Some(logical.as_str()), "DECLARED_OPERATION")
    } else if document.and_then(|d| d.pointer(item)).is_some() {
        (Some(item), "PATH_ITEM_RESOLUTION_REQUIRED")
    } else {
        (None, "DECLARED_LOCATION_UNAVAILABLE")
    };
    json!({"dependency":o.id,"digest":o.digest,"method":o.normalized["method"],"path":o.normalized["path"],
        "declaredSource":o.normalized["declaredSource"],"jsonPointer":exact,"pointerRole":role,"logicalOperationPointer":logical,
        "sourceMapping":o.normalized["sourceMapping"],"entrypoint":o.normalized["entrypoint"],
        "authority":o.normalized["authority"],"readerSelection":o.normalized["readerSelection"],
        "sourceReferences":o.source_ids,"boundaries":o.normalized["boundaries"]})
}

fn status(e: &ServiceEvidence, matches: &[&Route<'_>]) -> &'static str {
    // Distinct entrypoints stay distinct, even when conditions differ.
    let ids: BTreeSet<_> = matches.iter().map(|r| &r.entry.id).collect();
    if ids.len() > 1 {
        "AMBIGUOUS"
    } else if matches.iter().any(|r| candidate(e, r.entry)) {
        "CANDIDATE_MATCH"
    } else {
        "DECLARATION_MATCH"
    }
}

pub fn report(
    checked: &Check,
    snapshot: &str,
    selected: &BTreeSet<String>,
) -> Result<Value, ClewError> {
    if selected.is_empty() {
        return Err(invalid(
            "contract check requires explicit service selection",
        ));
    }
    let mut services = BTreeMap::new();
    let mut counts = BTreeMap::<String, usize>::new();
    for service in selected {
        let e = checked.services.get(service).ok_or_else(|| {
            invalid(format!(
                "service {service} has no evidence in selected snapshot"
            ))
        })?;
        let routes = routes(e);
        let contracts: Vec<_> = e
            .observations
            .values()
            .filter(|o| o.kind == "CONTRACT_OPERATION")
            .collect();
        let mut by_route = BTreeMap::<(String, String), Vec<&Route<'_>>>::new();
        let mut by_path = BTreeMap::<String, Vec<&Route<'_>>>::new();
        let mut contracts_by_route = BTreeMap::<(String, String), Vec<&Observation>>::new();
        let mut contracts_by_path = BTreeMap::<String, Vec<&Observation>>::new();
        for r in &routes {
            if let Some(key) = &r.key {
                by_path.entry(key.clone()).or_default().push(r);
            }
            if r.unknown.is_empty()
                && let Some(key) = &r.key
            {
                by_route
                    .entry((r.method.clone(), key.clone()))
                    .or_default()
                    .push(r);
            }
        }
        for o in &contracts {
            if let (Some(method), Some(path)) = (
                o.normalized["method"].as_str(),
                o.normalized["path"].as_str(),
            ) && let Some(key) = path_key(path)
            {
                contracts_by_path.entry(key.clone()).or_default().push(o);
                contracts_by_route
                    .entry((method.to_uppercase(), key))
                    .or_default()
                    .push(o);
            }
        }
        let mut findings = Vec::new();
        for o in &contracts {
            let method = o.normalized["method"].as_str().unwrap_or("").to_uppercase();
            let path = o.normalized["path"].as_str().unwrap_or("");
            let key = path_key(path);
            let matched = key
                .as_ref()
                .and_then(|k| by_route.get(&(method.clone(), k.clone())))
                .cloned()
                .unwrap_or_default();
            let same_path = key
                .as_ref()
                .and_then(|k| by_path.get(k))
                .cloned()
                .unwrap_or_default();
            let state = if key.is_none() || !METHODS.contains(&method.as_str()) {
                "UNSUPPORTED_INCOMPLETE"
            } else if matched.is_empty() {
                "CONTRACT_ONLY_UNPROVEN"
            } else {
                status(e, &matched)
            };
            findings.push(json!({"direction":"CONTRACT_TO_SOURCE","status":state,"method":method,"path":path,
                "contract":contract_ref(e,o),"associations":matched.iter().map(|r| source_ref(e,r)).collect::<Vec<_>>(),
                "samePathDeclarations":same_path.iter().filter(|r| r.method != method || !r.unknown.is_empty()).map(|r|source_ref(e,r)).collect::<Vec<_>>(),
                "methodDifferenceObserved":matched.is_empty() && same_path.iter().any(|r| r.unknown.is_empty() && r.method != method),
                "unknown":["RUNTIME_ACTIVATION_UNVERIFIED","SOURCE_ROUTE_ABSENCE_UNPROVEN","SERVLET_AND_GATEWAY_PREFIXES_UNVERIFIED"],
                "wireConformance":"UNVERIFIED"}));
        }
        for r in &routes {
            let matched = r
                .key
                .as_ref()
                .and_then(|k| contracts_by_route.get(&(r.method.clone(), k.clone())))
                .cloned()
                .unwrap_or_default();
            let same_path = r
                .key
                .as_ref()
                .and_then(|k| contracts_by_path.get(k))
                .cloned()
                .unwrap_or_default();
            let state = if !r.unknown.is_empty() {
                "UNSUPPORTED_INCOMPLETE"
            } else if matched.is_empty() {
                "DECLARED_ROUTE_ONLY"
            } else if matched.len() > 1 {
                "AMBIGUOUS"
            } else {
                status(e, &by_route[&(r.method.clone(), r.key.clone().unwrap())])
            };
            findings.push(json!({"direction":"SOURCE_TO_CONTRACT","status":state,"method":r.method,"path":r.path,
                "declaration":source_ref(e,r),"contracts":matched.iter().map(|o|contract_ref(e,o)).collect::<Vec<_>>(),
                "samePathContracts":same_path.iter().filter(|o|o.normalized["method"] != r.method).map(|o|contract_ref(e,o)).collect::<Vec<_>>(),
                "methodDifferenceObserved":!same_path.is_empty() && matched.is_empty() && r.unknown.is_empty(),
                "unknown":["RUNTIME_ACTIVATION_UNVERIFIED","CONTRACT_COMPLETENESS_UNPROVEN","SERVLET_AND_GATEWAY_PREFIXES_UNVERIFIED"],
                "wireConformance":"UNVERIFIED"}));
        }
        for finding in &mut findings {
            finding["service"] = json!(service);
            finding["revision"] = json!(e.revision);
            finding["snapshot"] = json!(snapshot);
            *counts
                .entry(finding["status"].as_str().unwrap().into())
                .or_default() += 1;
        }
        let referenced: BTreeSet<_> = routes
            .iter()
            .flat_map(|r| r.entry.source_ids.iter())
            .chain(contracts.iter().flat_map(|o| o.source_ids.iter()))
            .collect();
        let citations: BTreeMap<_, _> = referenced
            .into_iter()
            .map(|id| {
                e.sources
                    .get(id)
                    .map(|s| (id.clone(), citation(s)))
                    .ok_or_else(|| invalid("route report source citation is unavailable"))
            })
            .collect::<Result<_, _>>()?;
        services.insert(service.clone(),json!({"revision":e.revision,"extractor":e.extractor,"runtimeMode":e.runtime_mode,
            "coverage":e.coverage,"boundaries":e.boundaries,"scopeClosure":"UNPROVEN",
            "scope":e.observations.values().filter(|o|matches!(o.kind.as_str(),"SOURCE_SCOPE"|"CAPTURE_SCOPE"|"CONTRACT_SCOPE")).collect::<Vec<_>>(),
            "routePairs":routes.len(),"contractOperations":contracts.len(),"findings":findings,"sourceReferences":citations}));
    }
    let summary = format!(
        "Compared {} selected service(s) in both directions. Counts describe declarations, not runtime endpoints. Scope closure and wire conformance remain unverified.",
        services.len()
    );
    Ok(
        json!({"schema":SCHEMA,"status":"REPORTED","snapshot":snapshot,"inputDigest":checked.input_digest,
        "contextDigest":checked.context_digest,"authority":"PINNED_DECLARATIONS_NOT_REVERIFIED",
        "selection":selected,"delivery":{"complete":true,"nextCursor":null},"scopeClosure":"UNPROVEN",
        "wireConformance":"UNVERIFIED","summary":summary,"counts":counts,"services":services,
        "unknown":["PRODUCTION_ACTIVATION_UNVERIFIED","FRAMEWORK_CONDITIONS_NOT_EVALUATED","WIRE_BINDINGS_NOT_EVALUATED","SERVLET_AND_GATEWAY_PREFIXES_UNVERIFIED"],
        "unresolved":checked.unresolved}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> ServiceEvidence {
        serde_json::from_value(json!({"schema":"test","service":"svc","revision":"frozen-revision",
            "serviceDigest":"digest","extractor":"compiler","runtimeMode":"RELEASE","coverage":"COMPLETE",
            "boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}})).unwrap()
    }
    fn entry(id: &str, trigger: Value) -> Entrypoint {
        Entrypoint {
            id: id.into(),
            service: "svc".into(),
            symbol: id.into(),
            kind: "HTTP_ENDPOINT".into(),
            trigger,
            source_ids: vec![],
            dependency_ids: vec![],
            boundaries: vec![],
        }
    }
    fn contract(id: &str, method: &str, path: &str) -> Observation {
        Observation {
            id: id.into(),
            kind: "CONTRACT_OPERATION".into(),
            service: "svc".into(),
            symbol: id.into(),
            digest: id.into(),
            normalized: json!({"method":method,"path":path,"declaredSource":"openapi.json","sourceMapping":"NO_SOURCE_ROUTE_MATCH","authority":"DECLARED_OPENAPI","boundaries":["EXTERNAL_REF_NOT_FETCHED"]}),
            source_ids: vec![],
        }
    }
    fn run_report(e: ServiceEvidence) -> Value {
        let checked = Check {
            schema: "test".into(),
            input_digest: "input".into(),
            context_digest: "context".into(),
            services: BTreeMap::from([("svc".into(), e)]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_inputs: None,
            composition: None,
        };
        report(
            &checked,
            "sha256:frozen/123",
            &BTreeSet::from(["svc".into()]),
        )
        .unwrap()
    }
    #[test]
    fn bidirectional_differences_candidates_and_legacy_mapping() {
        let mut e = evidence();
        for (id, method, path) in [
            ("extra", "GET", "/extra"),
            ("method", "GET", "/method"),
            ("inherited", "GET", "/inherited"),
            ("wire", "GET", "/wire"),
        ] {
            let mut row = entry(
                id,
                json!({"methods":[method],"paths":[path],"authority":"ORIGINAL"}),
            );
            if id == "inherited" {
                row.kind = "SOURCE_HTTP_MAPPING".into();
                row.boundaries.push("REGISTRATION_UNPROVEN".into());
            }
            e.entrypoints.push(row);
        }
        for (id, method, path) in [
            ("put", "PUT", "/method"),
            ("inherited", "GET", "/inherited"),
            ("missing", "GET", "/missing"),
            ("wire", "GET", "/wire"),
        ] {
            let c = contract(id, method, path);
            e.observations.insert(id.into(), c);
        }
        let before = e.clone();
        let report = run_report(e.clone());
        assert_eq!(e, before);
        assert_eq!(report, run_report(e));
        let rows = report["services"]["svc"]["findings"].as_array().unwrap();
        let get = |direction, path| {
            rows.iter()
                .find(|r| r["direction"] == direction && r["path"] == path)
                .unwrap()
        };
        assert_eq!(
            get("SOURCE_TO_CONTRACT", "/extra")["status"],
            "DECLARED_ROUTE_ONLY"
        );
        assert_eq!(
            get("CONTRACT_TO_SOURCE", "/method")["methodDifferenceObserved"],
            true
        );
        assert_eq!(
            get("CONTRACT_TO_SOURCE", "/missing")["status"],
            "CONTRACT_ONLY_UNPROVEN"
        );
        let inherited = get("CONTRACT_TO_SOURCE", "/inherited");
        assert_eq!(inherited["status"], "CANDIDATE_MATCH");
        assert_eq!(
            inherited["contract"]["sourceMapping"],
            "NO_SOURCE_ROUTE_MATCH"
        );
        assert_eq!(inherited["associations"][0]["kind"], "SOURCE_HTTP_MAPPING");
        assert_eq!(
            inherited["contract"]["boundaries"][0],
            "EXTERNAL_REF_NOT_FETCHED"
        );
        assert_eq!(
            get("CONTRACT_TO_SOURCE", "/wire")["wireConformance"],
            "UNVERIFIED"
        );
        assert_eq!(report["scopeClosure"], "UNPROVEN");
    }
    #[test]
    fn correlated_routes_never_cross_product() {
        let mut e = evidence();
        e.entrypoints.push(entry(
            "pairs",
            json!({"methods":["GET","POST"],"paths":["/alpha","/beta"],
            "sourceRoutes":[{"method":"GET","path":"/alpha"},{"method":"POST","path":"/beta"}]}),
        ));
        for (id, method, path) in [
            ("bad1", "POST", "/alpha"),
            ("bad2", "GET", "/beta"),
            ("good", "GET", "/alpha"),
        ] {
            e.observations.insert(id.into(), contract(id, method, path));
        }
        let r = run_report(e);
        let rows = r["services"]["svc"]["findings"].as_array().unwrap();
        for row in rows
            .iter()
            .filter(|r| r["direction"] == "CONTRACT_TO_SOURCE")
        {
            assert_eq!(
                row["status"],
                if row["contract"]["dependency"] == "good" {
                    "DECLARATION_MATCH"
                } else {
                    "CONTRACT_ONLY_UNPROVEN"
                }
            );
            assert_eq!(
                row["methodDifferenceObserved"],
                row["contract"]["dependency"] != "good"
            );
        }
    }
    #[test]
    fn distinct_registrations_and_conditions_stay_ambiguous() {
        let mut e = evidence();
        for (id, param) in [("first", "mode=a"), ("second", "mode=b")] {
            e.entrypoints.push(entry(
                id,
                json!({"methods":["GET"],"paths":["/condition"],"conditions":{"params":[param]}}),
            ));
        }
        e.observations
            .insert("c".into(), contract("c", "GET", "/condition"));
        let r = run_report(e);
        let row = &r["services"]["svc"]["findings"][0];
        assert_eq!(row["status"], "AMBIGUOUS");
        assert_eq!(row["associations"][0]["conditions"]["params"][0], "mode=a");
        assert_eq!(row["associations"][1]["conditions"]["params"][0], "mode=b");
    }
    #[test]
    fn unknown_methods_and_uncorrelated_alternatives_are_not_guessed() {
        let mut e = evidence();
        e.entrypoints
            .push(entry("any", json!({"methods":[],"paths":["/any"]})));
        e.entrypoints.push(entry(
            "uncorrelated",
            json!({"methods":["GET","POST"],"paths":["/alpha","/beta"]}),
        ));
        for path in ["/any", "/alpha"] {
            e.observations
                .insert(path.into(), contract(path, "GET", path));
        }
        let r = run_report(e);
        for row in r["services"]["svc"]["findings"].as_array().unwrap() {
            assert!(matches!(
                row["status"].as_str(),
                Some("CONTRACT_ONLY_UNPROVEN" | "UNSUPPORTED_INCOMPLETE")
            ));
        }
    }
    #[test]
    fn simple_template_names_are_explicitly_normalized() {
        assert_eq!(path_key("/a/{id}"), path_key("/a/{key}"));
        for path in [
            "/a/{id:[0-9]+}",
            "/a/*",
            "relative",
            "/a/:id",
            "/a/${prefix}",
        ] {
            assert!(path_key(path).is_none(), "{path}");
        }
        assert_ne!(path_key("/a/"), path_key("/a"));
        assert_eq!(pointer("/a/{id}", "GET"), "/paths/~1a~1{id}/get");
    }

    #[test]
    fn pointers_identify_retained_locations_without_inventing_resolved_targets() {
        let mut e = evidence();
        e.contracts.insert(
            "openapi.json".into(),
            json!({"paths":{"/direct":{"get":{}},"/alias":{"$ref":"other.json#/route"}}}),
        );
        assert_eq!(
            contract_ref(&e, &contract("direct", "GET", "/direct"))["jsonPointer"],
            "/paths/~1direct/get"
        );
        let alias = contract_ref(&e, &contract("alias", "GET", "/alias"));
        assert_eq!(alias["jsonPointer"], "/paths/~1alias");
        assert_eq!(alias["pointerRole"], "PATH_ITEM_RESOLUTION_REQUIRED");
        assert_eq!(
            contract_ref(&e, &contract("missing", "GET", "/unknown"))["jsonPointer"],
            Value::Null
        );
        // An excluded/empty route scope never becomes an all-clear verdict.
        e.observations
            .insert("missing".into(), contract("missing", "GET", "/unknown"));
        let report = run_report(e);
        assert_eq!(report["status"], "REPORTED");
        assert_eq!(report["scopeClosure"], "UNPROVEN");
        assert_eq!(
            report["services"]["svc"]["findings"][0]["status"],
            "CONTRACT_ONLY_UNPROVEN"
        );
    }
}
