//! Optional provenance attached to the collector's existing reference walk.
use super::*;

const MAX_TRACE_EDGES: usize = 4_000_000;
const MAX_MISSING: usize = 1024;
const MAX_OWNERS: usize = 4096;
const MAX_WITNESS_DEPTH: usize = 512;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDiagnostics {
    pub schema: String,
    pub report: StorageReport,
    pub missing: Vec<MissingReference>,
    pub scope: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingReference {
    pub object: CasObject,
    pub owners: Vec<RetainingRoot>,
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
    parents: BTreeMap<String, BTreeSet<(bool, String)>>,
    missing: BTreeMap<String, CasObject>,
    edges: usize,
}
impl ReachabilityTrace {
    pub(super) fn reference(
        &mut self,
        digest: &str,
        owner: &str,
        root: bool,
    ) -> Result<(), ClewError> {
        if self
            .parents
            .entry(digest.into())
            .or_default()
            .insert((root, owner.into()))
        {
            self.edges += 1;
            if self.edges > MAX_TRACE_EDGES {
                return Err(limit("CAS diagnostic edge count exceeds four million"));
            }
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
            let mut pending = VecDeque::from([digest.clone()]);
            let mut next = BTreeMap::from([(digest.clone(), None::<String>)]);
            let mut owners = BTreeMap::new();
            while let Some(current) = pending.pop_front() {
                for (root, parent) in self.parents.get(&current).into_iter().flatten() {
                    if *root {
                        let mut via = vec![current.clone()];
                        while let Some(Some(child)) =
                            next.get(via.last().expect("witness has a head"))
                        {
                            if via.len() >= MAX_WITNESS_DEPTH {
                                return Err(limit("CAS diagnostic witness depth exceeds 512"));
                            }
                            via.push(child.clone());
                        }
                        owners.entry(parent.clone()).or_insert(via);
                        if owners.len() > MAX_OWNERS {
                            return Err(limit("CAS diagnostic owning-root count exceeds 4096"));
                        }
                    } else if !next.contains_key(parent) {
                        next.insert(parent.clone(), Some(current.clone()));
                        pending.push_back(parent.clone());
                    }
                }
            }
            if owners.is_empty() {
                return Err(corrupt("missing CAS reference has no traced retained root"));
            }
            missing.push(MissingReference {
                object: object.clone(),
                owners: owners
                    .into_iter()
                    .map(|(root_file, via)| RetainingRoot { root_file, via })
                    .collect(),
            });
        }
        Ok(StorageDiagnostics { schema:"codeclew-storage-diagnostics/1.0".into(), report, missing,
            scope:"Read-only retention diagnosis; one witness per owning metadata file. Missing bytes prevent inspection of their unknown descendants. No repository snapshot or source text is included.".into() })
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
        let owners = &report.missing[0].owners;
        assert_eq!(owners.len(), 3);
        assert_eq!(owners[0].root_file, "threads/a.json");
        assert_eq!(
            owners[0].via,
            vec![parent.digest.clone(), missing.digest.clone()]
        );
        assert_eq!(owners[2].via, vec![missing.digest]);
        assert!(CasStore::open(&state).unwrap().read(&parent, 4096).is_ok());
        assert!(garbage_collect_storage(&state).is_err());
    }
    #[test]
    fn trace_preserves_all_owners_through_shared_ancestors_and_cycles() {
        let mut trace = ReachabilityTrace::default();
        let object = CasObject::for_bytes("test/missing", b"absent").unwrap();
        trace.missing(&object).unwrap();
        for (child, parent, root) in [
            (&*object.digest, "parent", false),
            ("parent", "cycle", false),
            ("cycle", "parent", false),
            ("parent", "threads/a.json", true),
            ("cycle", "threads/b.json", true),
        ] {
            trace.reference(child, parent, root).unwrap();
        }
        // Exercise ownership resolution with an ordinary empty storage report.
        let temp = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temp.path().join("v2")).unwrap();
        let result = trace.finish(storage_status(&state).unwrap()).unwrap();
        assert_eq!(result.missing[0].owners.len(), 2);
    }
}
