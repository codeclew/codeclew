//! Normalize proven compiler registrations while retaining declaration witnesses.
use super::{digest, model::*};
use crate::error::ClewError;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

struct Registration {
    index: usize,
    effective: bool,
    declaration: bool,
    scope: String,
    declared_methods: BTreeSet<String>,
    registration_key: String,
}

fn registration(e: &ServiceEvidence, index: usize) -> Result<Option<Registration>, ClewError> {
    let entry = &e.entrypoints[index];
    if entry.kind != "HTTP_ENDPOINT" {
        return Ok(None);
    }
    let Some(observation) = entry
        .dependency_ids
        .iter()
        .filter_map(|id| e.observations.get(id))
        .find(|o| o.kind == "ENTRYPOINT" && o.normalized["binding"].is_object())
    else {
        return Ok(None);
    };
    let value = &observation.normalized;
    if !matches!(
        value["frameworkDerivation"]["inputAuthority"].as_str(),
        Some("K2_RESOLVED_ANNOTATIONS" | "JAVAC_RESOLVED_ANNOTATIONS")
    ) {
        return Ok(None);
    }
    let binding = &value["binding"];
    let Some(bean) = binding["beanClass"].as_str() else {
        return Ok(None);
    };
    let witness = entry
        .dependency_ids
        .iter()
        .filter_map(|id| e.observations.get(id))
        .find(|o| {
            let facts = &o.normalized["jvmAnnotations"];
            facts["coverage"]["status"] == "COMPLETE"
                && facts["boundaries"].as_array().is_some_and(Vec::is_empty)
                && facts["callables"].as_array().is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["method"]["identity"] == entry.symbol && row["beanClass"] == bean
                    })
                })
        });
    let Some(witness) = witness else {
        return Ok(None);
    };
    let effective = binding["controller"] == true && entry.boundaries.is_empty();
    let declaration = binding["controller"] == false
        && binding["targetSymbol"].is_null()
        && witness.normalized["jvmAnnotations"]["declaration"] == entry.symbol
        && entry
            .boundaries
            .iter()
            .any(|b| b == "CONTROLLER_REGISTRATION_UNPROVEN")
        && entry.boundaries.iter().all(|b| {
            matches!(
                b.as_str(),
                "CONTROLLER_REGISTRATION_UNPROVEN" | "ABSTRACT_HANDLER_REQUIRES_IMPLEMENTATION"
            )
        });
    if !effective && !declaration {
        return Ok(None);
    }
    let method_key = digest(&json!([
        value["scope"],
        entry.symbol,
        binding["kind"],
        binding["annotation"],
        binding["annotationChain"],
        binding["attributes"],
        binding["handlerAttributes"]
    ]))?;
    let registration_key = digest(&json!([
        method_key,
        bean,
        binding["classAttributes"],
        binding["registration"],
        entry.trigger
    ]))?;
    let method = witness.normalized["jvmAnnotations"]["callables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["method"]["identity"] == entry.symbol && row["beanClass"] == bean)
        .unwrap()["method"]
        .clone();
    let mut declared_methods = BTreeSet::new();
    let mut pending = vec![&method];
    while let Some(method) = pending.pop() {
        if let Some(id) = method["identity"].as_str() {
            declared_methods.insert(id.into());
        }
        if let Some(parents) = method["overrides"].as_array() {
            pending.extend(parents);
        }
    }
    Ok(Some(Registration {
        index,
        effective,
        declaration,
        scope: digest(&value["scope"])?,
        declared_methods,
        registration_key,
    }))
}

pub(super) fn normalize(e: &mut ServiceEvidence) -> Result<(), ClewError> {
    let registrations = (0..e.entrypoints.len())
        .map(|i| registration(e, i))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for row in registrations.iter().filter(|row| row.effective) {
        groups
            .entry(row.registration_key.clone())
            .or_default()
            .push(row.index);
    }
    let mut representative = BTreeMap::new();
    let mut removed = BTreeSet::new();
    let mut merges = BTreeSet::new();
    for indexes in groups.values() {
        let keep = *indexes
            .iter()
            .min_by_key(|i| &e.entrypoints[**i].id)
            .unwrap();
        for index in indexes {
            representative.insert(*index, keep);
            if *index != keep {
                removed.insert(*index);
                merges.insert((*index, keep));
            }
        }
    }
    let mut inherited = BTreeMap::<(String, String), BTreeSet<usize>>::new();
    for row in registrations.iter().filter(|row| row.effective) {
        for method in &row.declared_methods {
            inherited
                .entry((row.scope.clone(), method.clone()))
                .or_default()
                .insert(representative[&row.index]);
        }
    }
    for row in registrations.iter().filter(|row| row.declaration) {
        if let Some(handlers) =
            inherited.get(&(row.scope.clone(), e.entrypoints[row.index].symbol.clone()))
        {
            removed.insert(row.index);
            for handler in handlers {
                merges.insert((row.index, *handler));
            }
        }
    }
    // Observations stay intact. Only the effective inventory is coalesced, and
    // each surviving registration retains every source/dependency witness.
    for (from, to) in merges {
        let sources = e.entrypoints[from].source_ids.clone();
        let dependencies = e.entrypoints[from].dependency_ids.clone();
        let entry = &mut e.entrypoints[to];
        // Keep the effective handler's witnesses first; declaration provenance
        // must not replace its primary source or binding on a later projection.
        for source in sources {
            if !entry.source_ids.contains(&source) {
                entry.source_ids.push(source);
            }
        }
        for dependency in dependencies {
            if !entry.dependency_ids.contains(&dependency) {
                entry.dependency_ids.push(dependency);
            }
        }
    }
    let mut index = 0;
    e.entrypoints.retain(|_| {
        let keep = !removed.contains(&index);
        index += 1;
        keep
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn add(
        e: &mut ServiceEvidence,
        id: &str,
        bean: &str,
        target: &str,
        controller: bool,
        inherited: bool,
    ) {
        let fact_id = format!("fact-{id}");
        let route_id = format!("route-{id}");
        let boundaries = if controller {
            vec![]
        } else {
            vec!["CONTROLLER_REGISTRATION_UNPROVEN".into()]
        };
        let authority = "K2_RESOLVED_ANNOTATIONS";
        let source = vec![format!("source-{id}")];
        e.observations.insert(fact_id.clone(),Observation{id:fact_id.clone(),kind:"SYMBOL".into(),service:"svc".into(),symbol:target.into(),digest:"test".into(),source_ids:source.clone(),normalized:json!({"jvmAnnotations":{"declaration":if inherited {bean}else{target},"coverage":{"status":"COMPLETE"},"boundaries":[],"callables":[{"method":{"identity":target},"beanClass":bean}]}})});
        e.observations.insert(route_id.clone(),Observation{id:route_id.clone(),kind:"ENTRYPOINT".into(),service:"svc".into(),symbol:target.into(),digest:"test".into(),source_ids:source.clone(),normalized:json!({"scope":":/main","frameworkDerivation":{"inputAuthority":authority},"binding":{"kind":"HTTP_ENDPOINT","beanClass":bean,"controller":controller,"targetSymbol":inherited.then_some(target),"annotation":"RequestMapping","annotationChain":["GetMapping"],"attributes":{"method":["GET"],"path":["/same"]},"classAttributes":[],"registration":"RUNTIME_CONDITIONAL"}})});
        e.entrypoints.push(Entrypoint {
            id: id.into(),
            service: "svc".into(),
            symbol: target.into(),
            kind: "HTTP_ENDPOINT".into(),
            trigger: json!({"paths":["/same"],"methods":["GET"]}),
            source_ids: source,
            dependency_ids: vec![fact_id, route_id],
            boundaries,
        });
    }
    fn evidence() -> ServiceEvidence {
        serde_json::from_value(json!({"schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"a".repeat(40),"serviceDigest":"fixture","extractor":EXTRACTOR,"runtimeMode":"RELEASE","coverage":"PARTIAL","boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}})).unwrap()
    }
    #[test]
    fn inherited_registration_keeps_declaration_sources_without_a_phantom_base_handler() {
        let mut e = evidence();
        add(&mut e, "base", "class:Base", "Base.method", false, false);
        add(&mut e, "child", "class:Child", "Base.method", true, true);
        normalize(&mut e).unwrap();
        assert_eq!(e.entrypoints.len(), 1);
        let row = &e.entrypoints[0];
        assert_eq!(row.id, "child");
        assert_eq!(row.source_ids, vec!["source-child", "source-base"]);
        assert_eq!(row.dependency_ids[1], "route-child");
        assert_eq!(row.dependency_ids.len(), 4);
        assert_eq!(e.observations.len(), 4);
        let once = serde_json::to_value(&e).unwrap();
        normalize(&mut e).unwrap();
        assert_eq!(serde_json::to_value(&e).unwrap(), once);
    }
    #[test]
    fn only_identical_proven_bean_registrations_coalesce() {
        let mut e = evidence();
        add(&mut e, "a", "class:Child", "Base.method", true, true);
        add(&mut e, "b", "class:Child", "Base.method", true, true);
        normalize(&mut e).unwrap();
        assert_eq!(e.entrypoints.len(), 1);
        for (bean, target) in [
            ("class:Other", "Base.method"),
            ("class:Child", "Child.override"),
        ] {
            let mut e = evidence();
            add(&mut e, "a", "class:Child", "Base.method", true, true);
            add(&mut e, "b", bean, target, true, true);
            normalize(&mut e).unwrap();
            assert_eq!(e.entrypoints.len(), 2);
        }
        let mut e = evidence();
        add(&mut e, "base", "class:Base", "Base.method", true, false);
        add(&mut e, "child", "class:Child", "Base.method", true, true);
        normalize(&mut e).unwrap();
        assert_eq!(e.entrypoints.len(), 2);
        let mut e = evidence();
        add(&mut e, "base", "class:Base", "Base.method", false, false);
        add(&mut e, "a", "class:A", "Base.method", true, true);
        add(&mut e, "b", "class:B", "Base.method", true, true);
        normalize(&mut e).unwrap();
        assert_eq!(e.entrypoints.len(), 2);
        assert!(
            e.entrypoints
                .iter()
                .all(|row| row.source_ids.contains(&"source-base".into()))
        );
    }
    #[test]
    fn proven_override_links_the_declaration_without_restoring_its_old_mapping() {
        let mut e = evidence();
        add(&mut e, "base", "class:Base", "Base.method", false, false);
        add(&mut e, "child", "class:Child", "Child.method", true, false);
        e.observations.get_mut("fact-child").unwrap().normalized["jvmAnnotations"]["callables"]
            [0]["method"]["overrides"] = json!([{"identity":"Base.method","overrides":[]}]);
        e.observations.get_mut("route-child").unwrap().normalized["binding"]["attributes"]["path"] =
            json!(["/replacement"]);
        e.entrypoints[1].trigger["paths"] = json!(["/replacement"]);
        normalize(&mut e).unwrap();
        assert_eq!(e.entrypoints.len(), 1);
        assert_eq!(e.entrypoints[0].trigger["paths"], json!(["/replacement"]));
        assert!(e.entrypoints[0].source_ids.contains(&"source-base".into()));
    }

    #[test]
    fn partial_or_source_only_witnesses_do_not_prove_registration_identity() {
        for source in [false, true] {
            let mut e = evidence();
            add(&mut e, "base", "class:Base", "Base.method", false, false);
            add(&mut e, "child", "class:Child", "Base.method", true, true);
            if source {
                e.observations.get_mut("route-base").unwrap().normalized["frameworkDerivation"]["inputAuthority"] =
                    json!("SOURCE_ANNOTATIONS");
            } else {
                e.observations.get_mut("fact-base").unwrap().normalized["jvmAnnotations"]["coverage"]
                    ["status"] = json!("PARTIAL");
            }
            normalize(&mut e).unwrap();
            assert_eq!(e.entrypoints.len(), 2);
        }
    }
}
