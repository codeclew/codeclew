//! Publication selection is independent of immutable source and authoring inputs.
use super::{check::Check, invalid, model::*, store::Repository};
use crate::error::{ClewError, ErrorCode};
use clap::{Args, Subcommand};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const SCHEMA: &str = "codeclew-endpoint-publication-policy/1.0";
const RECORD: &str = "publication/endpoint-selection.json";
const MAX_BYTES: u64 = 1024 * 1024;
const MAX_SELECTORS: usize = 4096;

/// One callable, including all of its discovered registrations and native selections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selector {
    pub service: String,
    pub scope: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Policy {
    pub schema: String,
    pub exclusions: BTreeSet<Selector>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            exclusions: BTreeSet::new(),
        }
    }
}

impl<'de> Deserialize<'de> for Policy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Record {
            schema: String,
            exclusions: Vec<Selector>,
        }
        let record = Record::deserialize(deserializer)?;
        let exclusions: BTreeSet<_> = record.exclusions.iter().cloned().collect();
        if exclusions.len() != record.exclusions.len() {
            return Err(serde::de::Error::custom(
                "duplicate endpoint publication selector",
            ));
        }
        let policy = Self {
            schema: record.schema,
            exclusions,
        };
        policy.validate().map_err(serde::de::Error::custom)?;
        Ok(policy)
    }
}

impl Selector {
    fn validate(&self) -> Result<(), ClewError> {
        if !super::store::valid_id(&self.service)
            || self.scope.len() > 4096
            || self.scope.chars().any(char::is_control)
            || self.symbol.trim().is_empty()
            || self.symbol.len() > 8192
            || self.symbol.chars().any(char::is_control)
        {
            return Err(invalid(
                "endpoint publication selector requires an exact service, scope and callable symbol",
            ));
        }
        Ok(())
    }
}

impl Policy {
    fn validate(&self) -> Result<(), ClewError> {
        if self.schema != SCHEMA || self.exclusions.len() > MAX_SELECTORS {
            return Err(invalid(
                "endpoint publication policy schema or selector count is invalid",
            ));
        }
        for selector in &self.exclusions {
            selector.validate()?;
        }
        Ok(())
    }

    pub(super) fn digest(&self) -> Result<String, ClewError> {
        super::digest(self)
    }

    pub(super) fn excludes(&self, selector: &Selector) -> bool {
        self.exclusions.contains(selector)
    }
}

pub(super) fn load(repo: &Repository) -> Result<Policy, ClewError> {
    let path = repo.path(RECORD)?;
    if !path.try_exists().map_err(super::io_error)? {
        return Ok(Policy::default());
    }
    super::store::read(&path, MAX_BYTES)
}

fn declaration_selector(
    evidence: &ServiceEvidence,
    declaration: &str,
) -> Result<Selector, ClewError> {
    let observation = evidence.observations.get(declaration).ok_or_else(|| {
        invalid("endpoint declaration is absent from the selected service snapshot")
    })?;
    if observation.id != declaration
        || observation.service != evidence.service
        || !super::process_candidates::callable(observation)
    {
        return Err(invalid(
            "endpoint declaration must identify an exact callable SYMBOL in the selected service snapshot",
        ));
    }
    let scope = match observation.normalized["scope"].as_str() {
        Some(scope) if !scope.is_empty() => scope,
        None | Some("")
            if observation.normalized["authority"] == "SYNTAX"
                && evidence.extractor == SOURCE_EXTRACTOR
                && (observation.normalized["scope"].is_null()
                    || observation.normalized["scope"] == "") =>
        {
            ""
        }
        _ => {
            return Err(invalid(
                "endpoint declaration has no supported exact compilation scope",
            ));
        }
    };
    let selector = Selector {
        service: evidence.service.clone(),
        scope: scope.into(),
        symbol: observation.symbol.clone(),
    };
    selector.validate()?;
    Ok(selector)
}

type Declarations = BTreeMap<String, Result<Selector, ClewError>>;

/// Resolve scope groups once so listing a large retained service stays bounded.
fn declarations(evidence: &ServiceEvidence) -> Declarations {
    let mut declarations: Declarations = evidence
        .observations
        .keys()
        .map(|id| (id.clone(), declaration_selector(evidence, id)))
        .collect();
    let mut groups: BTreeMap<&Selector, (&Value, bool)> = BTreeMap::new();
    for (id, result) in &declarations {
        if let Ok(selector) = result {
            let normalized = &evidence.observations[id].normalized;
            let group = groups.entry(selector).or_insert((normalized, false));
            group.1 |= group.0 != normalized;
        }
    }
    let ambiguous: BTreeSet<_> = groups
        .into_iter()
        .filter(|(_, (_, ambiguous))| *ambiguous)
        .map(|(selector, _)| selector.clone())
        .collect();
    for result in declarations.values_mut() {
        if result
            .as_ref()
            .is_ok_and(|selector| ambiguous.contains(selector))
        {
            *result = Err(invalid(
                "endpoint callable identity is ambiguous in the selected compilation scope",
            ));
        }
    }
    declarations
}

pub(super) fn selector_for_declaration(
    evidence: &ServiceEvidence,
    declaration: &str,
) -> Result<Selector, ClewError> {
    declarations(evidence)
        .remove(declaration)
        .unwrap_or_else(|| {
            Err(invalid(
                "endpoint declaration is absent from the selected service snapshot",
            ))
        })
}

/// Resolve exact callable identities once for a retained service.
pub(super) fn selectors_for_declarations(evidence: &ServiceEvidence) -> BTreeMap<String, Selector> {
    declarations(evidence)
        .into_iter()
        .filter_map(|(id, selector)| selector.ok().map(|selector| (id, selector)))
        .collect()
}

pub(super) fn selector_for_entrypoint(
    evidence: &ServiceEvidence,
    entrypoint: &Entrypoint,
) -> Result<Selector, ClewError> {
    entrypoint_selector(evidence, entrypoint, &declarations(evidence))
}

/// Resolve a whole retained catalogue once; conflicting duplicate IDs have no identity.
pub(super) fn selectors_for_entrypoints(evidence: &ServiceEvidence) -> BTreeMap<String, Selector> {
    let declarations = declarations(evidence);
    let mut selectors = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut duplicate_ids = BTreeSet::new();
    for entrypoint in &evidence.entrypoints {
        if !seen.insert(entrypoint.id.clone()) {
            duplicate_ids.insert(entrypoint.id.clone());
        }
        if let Ok(selector) = entrypoint_selector(evidence, entrypoint, &declarations) {
            selectors.insert(entrypoint.id.clone(), selector);
        }
    }
    selectors.retain(|id, _| !duplicate_ids.contains(id));
    selectors
}

fn entrypoint_selector(
    evidence: &ServiceEvidence,
    entrypoint: &Entrypoint,
    declarations: &Declarations,
) -> Result<Selector, ClewError> {
    if entrypoint.service != evidence.service {
        return Err(invalid("endpoint service identity is inconsistent"));
    }
    let dependencies: Vec<_> = entrypoint
        .dependency_ids
        .iter()
        .filter(|id| {
            evidence.observations.get(*id).is_some_and(|observation| {
                observation.kind == "SYMBOL" && observation.symbol == entrypoint.symbol
            })
        })
        .collect();
    if dependencies.is_empty() {
        return Err(invalid("endpoint has no retained callable declaration"));
    }
    let selectors: BTreeSet<_> = dependencies
        .into_iter()
        .map(|id| declarations[id].clone())
        .collect::<Result<_, _>>()?;
    if selectors.len() != 1 {
        return Err(invalid(
            "endpoint callable spans ambiguous compilation scopes",
        ));
    }
    Ok(selectors.into_iter().next().unwrap())
}

/// Unsupported unrelated endpoints remain included; resolving an edit is strict.
#[cfg(test)]
pub(super) fn excluded_entrypoint_ids(
    policy: &Policy,
    checked: &Check,
    service: &str,
) -> BTreeSet<String> {
    checked
        .services
        .get(service)
        .into_iter()
        .flat_map(|evidence| {
            let declarations = declarations(evidence);
            evidence.entrypoints.iter().filter_map(move |entrypoint| {
                entrypoint_selector(evidence, entrypoint, &declarations)
                    .ok()
                    .filter(|selector| policy.excludes(selector))
                    .map(|_| entrypoint.id.clone())
            })
        })
        .collect()
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List callable groups and saved exclusions using an explicit retained snapshot.
    List(ListArgs),
    /// Exclude every registration and native page selection of one exact callable.
    Exclude(EditArgs),
    /// Restore a callable, including a saved exclusion whose endpoint has disappeared.
    Include(EditArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub service: String,
    #[arg(long)]
    pub snapshot: String,
}

#[derive(Debug, Clone, Args)]
pub struct EditArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub service: String,
    /// Required to resolve endpoint or declaration IDs, and for every exclusion.
    #[arg(long)]
    pub snapshot: Option<String>,
    #[arg(long, requires = "snapshot", conflicts_with_all = ["declaration", "scope", "symbol"])]
    pub endpoint: Option<String>,
    #[arg(long, requires = "snapshot", conflicts_with_all = ["endpoint", "scope", "symbol"])]
    pub declaration: Option<String>,
    /// Exact returned compilation scope; use an empty string for source syntax.
    #[arg(long, requires = "symbol")]
    pub scope: Option<String>,
    #[arg(long, requires = "scope")]
    pub symbol: Option<String>,
    /// Copy policyDigest from list to reject a concurrent policy change.
    #[arg(long)]
    pub expected_policy_digest: Option<String>,
}

fn retained(repo: &Repository, service: &str, snapshot: &str) -> Result<Check, ClewError> {
    // Read the exact saved record, without consulting latest or recapturing sources.
    let checked = Check::load_snapshot(repo, snapshot)?;
    if !checked.services.contains_key(service) {
        return Err(invalid(
            "selected service is absent from the endpoint snapshot",
        ));
    }
    Ok(checked)
}

fn affected(
    evidence: &ServiceEvidence,
    selector: &Selector,
    declarations: &Declarations,
) -> Vec<String> {
    let mut ids: Vec<_> = evidence
        .entrypoints
        .iter()
        .filter(|entrypoint| {
            entrypoint_selector(evidence, entrypoint, declarations)
                .is_ok_and(|value| value == *selector)
        })
        .map(|entrypoint| entrypoint.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn next_action(snapshot: Option<&str>) -> Value {
    let snapshot = snapshot.unwrap_or("SAVED_SNAPSHOT");
    json!({"ordinary":format!("Run docs render --root ROOT --snapshot {snapshot} --publish to publish the selection."),
        "native":"Run docs pages render with the original selections and a new output directory to publish the selection."})
}

fn list(
    repo: &Repository,
    checked: &Check,
    service: &str,
    snapshot: &str,
) -> Result<Value, ClewError> {
    let policy = load(repo)?;
    let evidence = &checked.services[service];
    let declarations = declarations(evidence);
    let mut selectors = BTreeSet::new();
    let mut unresolved = Vec::new();
    for entrypoint in &evidence.entrypoints {
        match entrypoint_selector(evidence, entrypoint, &declarations) {
            Ok(selector) => { selectors.insert(selector); }
            Err(error) => unresolved.push(json!({"endpoint":entrypoint.id,"symbol":entrypoint.symbol,"status":"UNRESOLVED_PUBLICATION_IDENTITY","reason":error.message})),
        }
    }
    // Native selections can refer to retained callable declarations without an ordinary registration.
    let mut declaration_ids: BTreeMap<Selector, Vec<String>> = BTreeMap::new();
    for (id, result) in &declarations {
        if let Ok(selector) = result {
            selectors.insert(selector.clone());
            declaration_ids
                .entry(selector.clone())
                .or_default()
                .push(id.clone());
        }
    }
    let items: Vec<_> = selectors.iter().map(|selector| {
        json!({"selector":selector,"entrypointIds":affected(evidence, selector, &declarations),"declarationIds":declaration_ids.get(selector),"excluded":policy.excludes(selector),"grouping":"ALL_REGISTRATIONS_AND_NATIVE_SELECTIONS_OF_CALLABLE"})
    }).collect();
    let saved: Vec<_> = policy.exclusions.iter().filter(|selector| selector.service == service).map(|selector| {
        json!({"selector":selector,"matched":selectors.contains(selector),"entrypointIds":affected(evidence, selector, &declarations)})
    }).collect();
    Ok(
        json!({"schema":"codeclew-endpoint-publication-list/1.0","service":service,"snapshot":snapshot,
        "authority":"PINNED_SNAPSHOT_NOT_REVERIFIED","policyDigest":policy.digest()?,"items":items,
        "exclusions":saved,"unresolved":unresolved,"nextAction":next_action(Some(snapshot))}),
    )
}

fn resolve(evidence: &ServiceEvidence, args: &EditArgs) -> Result<Selector, ClewError> {
    match (&args.endpoint, &args.declaration, &args.scope, &args.symbol) {
        (Some(id), None, None, None) => {
            let entries: Vec<_> = evidence
                .entrypoints
                .iter()
                .filter(|entrypoint| entrypoint.id == *id)
                .collect();
            if entries.len() != 1 {
                return Err(invalid(
                    "endpoint ID must identify exactly one retained registration",
                ));
            }
            selector_for_entrypoint(evidence, entries[0])
        }
        (None, Some(id), None, None) => selector_for_declaration(evidence, id),
        (None, None, Some(scope), Some(symbol)) => {
            let selector = Selector {
                service: args.service.clone(),
                scope: scope.clone(),
                symbol: symbol.clone(),
            };
            selector.validate()?;
            let found = declarations(evidence)
                .values()
                .any(|result| result.as_ref().is_ok_and(|value| *value == selector));
            if !found {
                return Err(invalid(
                    "exact endpoint selector does not resolve an unambiguous retained callable",
                ));
            }
            Ok(selector)
        }
        _ => Err(invalid(
            "select exactly one --endpoint, --declaration or paired --scope and --symbol",
        )),
    }
}

fn edit(args: EditArgs, exclude: bool) -> Result<Value, ClewError> {
    let repo = Repository::open(&args.root)?;
    let checked = args
        .snapshot
        .as_deref()
        .map(|snapshot| retained(&repo, &args.service, snapshot))
        .transpose()?;
    let _lock = repo.lock()?;
    let mut policy = load(&repo)?;
    let before = policy.digest()?;
    if args
        .expected_policy_digest
        .as_ref()
        .is_some_and(|expected| *expected != before)
    {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "endpoint publication policy changed; list again and retry with its policyDigest",
        ));
    }
    let direct = match (&args.endpoint, &args.declaration, &args.scope, &args.symbol) {
        (None, None, Some(scope), Some(symbol)) => Some(Selector {
            service: args.service.clone(),
            scope: scope.clone(),
            symbol: symbol.clone(),
        }),
        _ => None,
    };
    let selector = if let Some(selector) = direct.filter(|_| !exclude) {
        // Removing an exact saved key is safe and idempotent even after its
        // callable disappears or a previous include already removed the key.
        selector.validate()?;
        selector
    } else {
        let checked = checked.as_ref().ok_or_else(|| invalid("endpoint ID resolution and exclusions require an explicit --snapshot; include a disappeared exclusion with exact --scope and --symbol"))?;
        resolve(&checked.services[&args.service], &args)?
    };
    let changed = if exclude {
        policy.exclusions.insert(selector.clone())
    } else {
        policy.exclusions.remove(&selector)
    };
    if changed {
        policy.validate()?;
        let encoded = super::bytes(&policy)?;
        if encoded.len() as u64 > MAX_BYTES {
            return Err(invalid("endpoint publication policy exceeds one MiB"));
        }
        repo.atomic(RECORD, &encoded)?;
    }
    let ids = checked
        .as_ref()
        .map(|checked| {
            let evidence = &checked.services[&args.service];
            affected(evidence, &selector, &declarations(evidence))
        })
        .unwrap_or_default();
    Ok(
        json!({"schema":"codeclew-endpoint-publication-edit/1.0","selector":selector,"excluded":exclude,
        "changed":changed,"policyDigest":policy.digest()?,"previousPolicyDigest":before,"entrypointIds":ids,
        "grouping":"ALL_REGISTRATIONS_AND_NATIVE_SELECTIONS_OF_CALLABLE","snapshot":args.snapshot,
        "nextAction":next_action(args.snapshot.as_deref())}),
    )
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::List(args) => {
            let repo = Repository::open(&args.root)?;
            let checked = retained(&repo, &args.service, &args.snapshot)?;
            list(&repo, &checked, &args.service, &args.snapshot)
        }
        Command::Exclude(args) => edit(args, true),
        Command::Include(args) => edit(args, false),
    }
}
