//! Optional provenance attached to the collector's existing reference walk.
use super::*;

const MAX_TRACE_NODES: usize = 2_000_000;
const MAX_MISSING: usize = 1024;
const MAX_WITNESS_DEPTH: usize = 512;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDiagnostics {
    pub schema: String,
    pub report: StorageReport,
    pub missing: Vec<MissingReference>,
    pub scope: String,
    pub ownership_complete: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingReference {
    pub object: CasObject,
    pub witness: RetainingRoot,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetainingRoot {
    pub root_file: String,
    /// A witness from the directly retained object to the absent object.
    pub via: Vec<String>,
}
#[derive(Default)]
pub(super) struct ReachabilityTrace {
    parents: BTreeMap<String, (bool, String)>,
    missing: BTreeMap<String, CasObject>,
}
impl ReachabilityTrace {
    pub(super) fn reference(
        &mut self,
        digest: &str,
        owner: &str,
        root: bool,
    ) -> Result<(), ClewError> {
        // The collector calls this while discovering roots/children. Keep only
        // the first parent: it was discovered earlier, so later shared/cyclic
        // edges cannot enlarge this forest or replace its rooted witness.
        self.parents
            .entry(digest.into())
            .or_insert_with(|| (root, owner.into()));
        if self.parents.len() > MAX_TRACE_NODES {
            return Err(limit(
                "CAS diagnostic discovered-object count exceeds two million",
            ));
        }
        Ok(())
    }
    pub(super) fn missing(&mut self, object: &CasObject) -> Result<(), ClewError> {
        self.missing.insert(object.digest.clone(), object.clone());
        if self.missing.len() > MAX_MISSING {
            return Err(limit("CAS diagnostic missing-object count exceeds 1024"));
        }
        Ok(())
    }
    fn finish(self, report: StorageReport) -> Result<StorageDiagnostics, ClewError> {
        let mut missing = Vec::new();
        for (digest, object) in &self.missing {
            let mut via = vec![digest.clone()];
            let root_file = loop {
                if via.len() > MAX_WITNESS_DEPTH {
                    return Err(limit("CAS diagnostic witness depth exceeds 512"));
                }
                let current = via.last().expect("witness starts at missing object");
                let (root, parent) = self
                    .parents
                    .get(current)
                    .ok_or_else(|| corrupt("missing CAS reference has no discovery witness"))?;
                if *root {
                    break parent.clone();
                }
                if via.contains(parent) {
                    return Err(corrupt("CAS diagnostic discovery witness contains a cycle"));
                }
                via.push(parent.clone());
            };
            via.reverse();
            missing.push(MissingReference {
                object: object.clone(),
                witness: RetainingRoot { root_file, via },
            });
        }
        Ok(StorageDiagnostics { schema:"codeclew-storage-diagnostics/1.0".into(), report, missing, ownership_complete:false,
            scope:"Read-only retention diagnosis; one discovery witness per missing object, NOT a complete owner inventory or permission to release other roots. Shared owners can remain undisclosed. Missing bytes prevent inspection of unknown descendants. No source text or Git snapshot. After addressed reconciliation, require a complete storage GC dry-run with zero dangling objects before physical apply.".into() })
    }
}
fn limit(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

pub fn diagnose_storage(authority: &StateAuthority) -> Result<StorageDiagnostics, ClewError> {
    let released = released_session_cas_roots(authority)?;
    let mut trace = Some(ReachabilityTrace::default());
    let report = CasStore::open(authority)?
        .storage_plan_traced(authority, &released, &mut trace)?
        .report;
    trace.expect("diagnostic trace enabled").finish(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_attributes_shared_transitive_and_direct_missing_roots_without_deleting() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("v2")).unwrap();
        let missing = CasObject::for_bytes("test/missing", b"absent").unwrap();
        let parent = {
            let store = CasStore::open(&state).unwrap();
            store
                .put(
                    "test/parent",
                    &canonical::bytes(&serde_json::json!({"child":missing})).unwrap(),
                )
                .unwrap()
        };
        for (name, object) in [("a", &parent), ("b", &parent), ("c", &missing)] {
            state
                .write_private_atomic(
                    &state.root().join(format!("threads/{name}.json")),
                    &canonical::bytes(&serde_json::json!({"object":object})).unwrap(),
                )
                .unwrap();
        }
        let report = diagnose_storage(&state).unwrap();
        assert!(report.report.collection_blocked);
        assert_eq!(report.report.reclaimable_bytes, 0);
        assert_eq!(report.missing.len(), 1);
        let witness = &report.missing[0].witness;
        // All roots are discovered before traversal, so this direct reference
        // wins over the later path from either shared parent.
        assert_eq!(witness.root_file, "threads/c.json");
        assert_eq!(witness.via, vec![missing.digest]);
        assert!(!report.ownership_complete);
        state
            .directory(Path::new("threads"))
            .unwrap()
            .remove_file(OsStr::new("c.json"))
            .unwrap();
        let transitive = diagnose_storage(&state).unwrap();
        let witness = &transitive.missing[0].witness;
        assert!(matches!(
            witness.root_file.as_str(),
            "threads/a.json" | "threads/b.json"
        ));
        assert_eq!(
            witness.via,
            vec![
                parent.digest.clone(),
                report.missing[0].object.digest.clone()
            ]
        );
        assert!(CasStore::open(&state).unwrap().read(&parent, 4096).is_ok());
        assert!(garbage_collect_storage(&state).is_err());
    }
    #[test]
    fn discovery_witness_stays_rooted_when_later_edges_share_or_cycle() {
        let mut trace = ReachabilityTrace::default();
        let object = CasObject::for_bytes("test/missing", b"absent").unwrap();
        trace.missing(&object).unwrap();
        for (child, parent, root) in [
            ("parent", "threads/a.json", true),
            ("cycle", "parent", false),
            (&*object.digest, "cycle", false),
            ("parent", "cycle", false),
            ("cycle", "threads/b.json", true),
        ] {
            trace.reference(child, parent, root).unwrap();
        }
        // Exercise ownership resolution with an ordinary empty storage report.
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("v2")).unwrap();
        let result = trace.finish(storage_status(&state).unwrap()).unwrap();
        assert_eq!(result.missing[0].witness.root_file, "threads/a.json");
        assert_eq!(
            result.missing[0].witness.via,
            vec!["parent".to_owned(), "cycle".to_owned(), object.digest]
        );
    }
}
