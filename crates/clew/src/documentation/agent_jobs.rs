//! Bounded answer jobs, saved run inspection and finite accounting.
use super::{
    bytes, digest, invalid, io_error,
    store::{self, Repository, WriteLock},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

#[path = "agent_jobs/operation_draft.rs"]
mod operation_draft;
#[path = "agent_jobs/operation_draft_review.rs"]
pub(super) mod operation_draft_review;
#[path = "agent_jobs/recorded_expansion.rs"]
pub(super) mod recorded_expansion;
#[path = "agent_jobs/recovery.rs"]
mod recovery;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Amount {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_units: u64,
}
impl Amount {
    pub fn add(&self, other: &Self) -> Result<Self, ClewError> {
        Ok(Self {
            input_tokens: self
                .input_tokens
                .checked_add(other.input_tokens)
                .ok_or_else(|| invalid("input reservation overflow"))?,
            output_tokens: self
                .output_tokens
                .checked_add(other.output_tokens)
                .ok_or_else(|| invalid("output reservation overflow"))?,
            cost_units: self
                .cost_units
                .checked_add(other.cost_units)
                .ok_or_else(|| invalid("cost reservation overflow"))?,
        })
    }
    pub fn within(&self, limit: &Self) -> bool {
        self.input_tokens <= limit.input_tokens
            && self.output_tokens <= limit.output_tokens
            && self.cost_units <= limit.cost_units
    }
    pub fn positive(&self) -> bool {
        self.input_tokens > 0 && self.output_tokens > 0 && self.cost_units > 0
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cap {
    pub maximum: Amount,
    pub overhead_input_tokens: u64,
    pub timeout_ms: u64,
    pub output_bytes: usize,
}
fn maximum_only() -> String {
    "MAXIMUM_ONLY".into()
}

pub(super) fn selection_guidance(work: &super::work::Work) -> Value {
    let mut available_kinds: BTreeSet<_> = work
        .checked
        .dependencies
        .values()
        .map(|dependency| dependency.kind.as_str())
        .collect();
    let source_inventory = super::work::source_inventory_available(work);
    if source_inventory {
        available_kinds.insert("SOURCE");
    }
    serde_json::json!({
        "availableKinds": available_kinds,
        "queryKind": "Matches an Observation.kind dependency-record kind. Use * for all dependency kinds. SOURCE is a separate retained source inventory only when listed in availableKinds; DEPENDENCY is not a query kind.",
        "symbolContains": "Searches captured symbols and method names with a case-sensitive substring. For example, {\"kind\":\"SYMBOL\",\"symbolContains\":\"helper\"} finds SYMBOL records whose captured symbol contains helper.",
        "symbolLookup": "A SYMBOL query is one bounded declaration-discovery page. Continue with its returned cursor and the same query to discover more declarations. These navigation rows are not citable provider facts; request an exact symbol identity or fullRecordReference for a full record, subject to the existing Work page limits. SYMBOL records include captured declarations that may be callable or non-callable; use declarationKind and syntaxKind when present. A symbols selection must use an exact compiler identity or qualified declaration name. If expansionFeedback reports NOT_FOUND or AMBIGUOUS, it describes only that lookup, not proof code is absent; do not guess a package, owner, or signature.",
        "exampleSelection": {"query":{"kind":"SYMBOL","symbolContains":"helper","projection":"NAVIGATION"}},
        "resultAuthority": "Dependency queries search captured records registered in this Work's influence set. SOURCE inventory searches only the Work service's registered captured file membership; its SOURCE_SCOPE is watched by immutable Work influence. Absence is not evidence of runtime behavior or uncaptured code.",
        "sourceInventoryAvailable": source_inventory,
        "sourceInventory": "When SOURCE is available, use {\"query\":{\"kind\":\"SOURCE\"}} and continue with the same selection and returned cursor. SOURCE has no symbol field: omit symbolContains or use an empty string; nonempty filters are rejected. Read file metadata and exact references from returned rows or omissions, then expand the exact reference or use docs work read-part for oversized source text. Discovery does not prove complete read coverage or parsed semantics. Compiler services without SOURCE_SCOPE inventory continue using declaration sourceReferences; scenario inventory is unsupported.",
        "navigation": "sourceReferences and dependencyReferences are navigation handles. Expand a handle in a separate recorded read before citing its contents, unless those contents are already delivered and allowed by this packet. For SYMBOL CALLABLE_SUMMARY rows, callableReadActions reports declaration and source delivery separately; use primaryRead.selection to read the fullRecordReference. A callableEntryReference points to the matching callables entry for immediateTargetHints. Oversized SOURCE records use recorded SOURCE_PART content."
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Role {
    pub adapter: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_representation: Option<String>,
    #[serde(default = "maximum_only")]
    pub usage_authority: String,
    pub command: Vec<String>,
    pub runtime_reads: Vec<PathBuf>,
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub network: bool,
    pub cap: Cap,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cost_units: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budget {
    pub account: String,
    pub cost_unit: String,
    pub ceiling: Amount,
    pub stop_loss: Amount,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub schema: String,
    pub author: Role,
    pub reviewer: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_output_contract: Option<String>,
    #[serde(default)]
    pub fallback: Option<Role>,
    pub author_calls: u32,
    pub reviewer_calls: u32,
    pub fallback_calls: u32,
    pub repair_attempts: u32,
    pub expansions: u32,
    pub budget: Budget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reservation {
    pub run: String,
    pub role: String,
    pub maximum: Amount,
    pub charged: Amount,
    pub status: String,
    pub actual: Option<Usage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Account {
    pub schema: String,
    pub cost_unit: String,
    pub ceiling: Amount,
    pub stop_loss: Amount,
    pub reservations: BTreeMap<String, Reservation>,
}
fn account_path(id: &str) -> Result<String, ClewError> {
    if !store::valid_id(id) {
        return Err(invalid("invalid budget account identity"));
    }
    Ok(format!("execution/accounts/{id}.json"))
}
pub fn account(repo: &Repository, budget: &Budget) -> Result<Account, ClewError> {
    let path = repo.path(&account_path(&budget.account)?)?;
    if repo
        .path(&format!(".codeclew/accounts/{}.json", budget.account))?
        .exists()
    {
        return Err(invalid(
            "DOCS_REINDEX_REQUIRED: obsolete budget account layout; initialize a new documentation root",
        ));
    }
    if path.exists() {
        let a: Account = store::read(&path, 16 * 1024 * 1024)?;
        if a.schema != "codeclew-documentation-budget/1.0"
            || a.cost_unit != budget.cost_unit
            || a.ceiling != budget.ceiling
            || a.stop_loss != budget.stop_loss
        {
            return Err(invalid(
                "account ceiling is immutable; select an explicitly new budget account",
            ));
        }
        Ok(a)
    } else {
        Ok(Account {
            schema: "codeclew-documentation-budget/1.0".into(),
            cost_unit: budget.cost_unit.clone(),
            ceiling: budget.ceiling.clone(),
            stop_loss: budget.stop_loss.clone(),
            reservations: BTreeMap::new(),
        })
    }
}
fn save_account(repo: &Repository, budget: &Budget, value: &Account) -> Result<(), ClewError> {
    let encoded = bytes(value)?;
    if encoded.len() > 16 * 1024 * 1024 {
        return Err(invalid("budget account ledger exceeds its bound"));
    }
    repo.atomic(&account_path(&budget.account)?, &encoded)
}
/// Reserve the whole configured bounded path, including reviewer and repair calls.
/// This conservative reservation is atomic across competing work items.
pub fn reserve(repo: &Repository, config: &Config, run: &str) -> Result<Vec<String>, ClewError> {
    reserve_with_account_guard(repo, config, run, false)
}

/// A fresh review retry owns its entire account. The guard and reservation
/// mutation share the same repository lock; a per-Work lock alone is insufficient.
fn reserve_review_retry(repo: &Repository, config: &Config, run: &str) -> Result<(), ClewError> {
    if config.reviewer_calls != 1 {
        return Err(invalid(
            "DRAFT_REVIEW_RETRY_ACCOUNT_CONFLICT: the legacy retry requires exactly one reviewer reservation",
        ));
    }
    reserve_with_account_guard(repo, config, run, true).map(|_| ())
}

pub(super) fn reserve_expanding_review_retry(
    repo: &Repository,
    config: &Config,
    run: &str,
) -> Result<(), ClewError> {
    reserve_with_account_guard(repo, config, run, true).map(|_| ())
}

fn reserve_with_account_guard(
    repo: &Repository,
    config: &Config,
    run: &str,
    review_retry: bool,
) -> Result<Vec<String>, ClewError> {
    if config.budget.cost_unit.trim().is_empty()
        || config.budget.cost_unit.len() > 64
        || !config.budget.ceiling.positive()
        || !config.budget.stop_loss.positive()
        || !config.budget.stop_loss.within(&config.budget.ceiling)
        || config.budget.stop_loss == config.budget.ceiling
    {
        return Err(invalid(
            "budget needs positive finite ceilings and a lower stop-loss",
        ));
    }
    let _lock = repo.lock()?;
    let mut ledger = account(repo, &config.budget)?;
    if review_retry {
        if config.author_calls != 0
            || config.reviewer_calls == 0
            || config.fallback_calls != 0
            || ledger
                .reservations
                .values()
                .any(|reservation| reservation.run != run)
        {
            return Err(invalid(
                "DRAFT_REVIEW_RETRY_ACCOUNT_CONFLICT: this retry requires an empty account or only its own exact reviewer reservation; historical and concurrent foreign reservations remain untouched",
            ));
        }
        if !ledger.reservations.is_empty() {
            validate_run_reservations(config, run, &ledger.reservations)?;
            return Ok(ledger.reservations.keys().cloned().collect());
        }
    }
    // Keep reservations outside disposable Work state so cleanup cannot reset spend.
    save_account(repo, &config.budget, &ledger)?;
    if ledger
        .reservations
        .values()
        .any(|r| r.status == "BOUND_VIOLATED")
    {
        return Err(invalid(
            "ACCOUNTING_BOUND_VIOLATED: this account cannot dispatch further calls",
        ));
    }
    let mut total = Amount::default();
    for reservation in ledger.reservations.values() {
        total = total.add(&reservation.charged)?;
    }
    let mut ids = Vec::new();
    for (role, driver, count) in [
        ("author", Some(&config.author), config.author_calls),
        ("reviewer", Some(&config.reviewer), config.reviewer_calls),
        ("fallback", config.fallback.as_ref(), config.fallback_calls),
    ] {
        let Some(driver) = driver else {
            continue;
        };
        for ordinal in 0..count {
            let id = digest(&(run, role, ordinal))?;
            if ledger.reservations.contains_key(&id) {
                return Err(invalid("run already reserved; inspect its durable result"));
            }
            total = total.add(&driver.cap.maximum)?;
            ledger.reservations.insert(
                id.clone(),
                Reservation {
                    run: run.into(),
                    role: role.into(),
                    maximum: driver.cap.maximum.clone(),
                    charged: driver.cap.maximum.clone(),
                    status: "RESERVED".into(),
                    actual: None,
                },
            );
            ids.push(id);
        }
    }
    if !total.within(&ledger.stop_loss) {
        return Err(invalid(
            "BUDGET_EXHAUSTED: cannot reserve the configured calls below stop-loss",
        ));
    }
    save_account(repo, &config.budget, &ledger)?;
    Ok(ids)
}

fn ensure_reserved(repo: &Repository, config: &Config, run: &str) -> Result<(), ClewError> {
    let existing: BTreeMap<_, _> = account(repo, &config.budget)?
        .reservations
        .into_iter()
        .filter(|(_, reservation)| reservation.run == run)
        .collect();
    if existing.is_empty() {
        reserve(repo, config, run)?;
        return Ok(());
    }
    validate_run_reservations(config, run, &existing)
}

fn validate_run_reservations(
    config: &Config,
    run: &str,
    existing: &BTreeMap<String, Reservation>,
) -> Result<(), ClewError> {
    validate_reservation_slots(config, run, existing, false)
}

fn validate_reservation_slots(
    config: &Config,
    run: &str,
    existing: &BTreeMap<String, Reservation>,
    terminal: bool,
) -> Result<(), ClewError> {
    let mut expected = BTreeMap::new();
    for (role, driver, count) in [
        ("author", Some(&config.author), config.author_calls),
        ("reviewer", Some(&config.reviewer), config.reviewer_calls),
        ("fallback", config.fallback.as_ref(), config.fallback_calls),
    ] {
        let Some(driver) = driver else {
            continue;
        };
        for ordinal in 0..count {
            expected.insert(
                digest(&(run, role, ordinal))?,
                (role.to_owned(), driver.cap.maximum.clone()),
            );
        }
    }
    if existing.len() != expected.len()
        || existing.iter().any(|(id, reservation)| {
            expected.get(id).is_none_or(|(role, maximum)| {
                reservation.role != *role
                    || reservation.run != run
                    || reservation.maximum != *maximum
                    || (!terminal && reservation.status == "RELEASED_NOT_DISPATCHED")
            })
        })
    {
        return Err(invalid(
            "RECOVERY_ACCOUNTING_MISMATCH: original finite reservations do not match this run config",
        ));
    }
    Ok(())
}

/// Replay a finished role loop without reserving or writing anything. Released
/// unused calls remain legitimate historical slots and cannot be dispatched.
pub(super) fn validate_terminal_reservations(
    repo: &Repository,
    config: &Config,
    report: &RunReport,
) -> Result<(), ClewError> {
    let existing: BTreeMap<_, _> = account(repo, &config.budget)?
        .reservations
        .into_iter()
        .filter(|(_, reservation)| reservation.run == report.run)
        .collect();
    validate_terminal_reservation_records(config, report, &existing)
}

fn validate_terminal_reservation_records(
    config: &Config,
    report: &RunReport,
    existing: &BTreeMap<String, Reservation>,
) -> Result<(), ClewError> {
    validate_reservation_slots(config, &report.run, existing, true)?;
    for (id, reservation) in existing {
        let attempts: Vec<_> = report
            .attempts
            .iter()
            .filter(|a| &a.reservation == id)
            .collect();
        if reservation.status == "RELEASED_NOT_DISPATCHED" {
            let prepared_cancel = attempts.len() == 1
                && report.status.ends_with("CANCELLED")
                && attempts[0].status == "PREPARED"
                && attempts[0].role == reservation.role
                && attempts[0].usage.is_none()
                && attempts[0].result_digest.is_none()
                && attempts[0].captured_stdout_bytes == 0
                && attempts[0].captured_stderr_bytes == 0;
            if (!attempts.is_empty() && !prepared_cancel)
                || reservation.charged != Amount::default()
                || reservation.actual.is_some()
            {
                return Err(invalid(
                    "RECOVERY_ACCOUNTING_MISMATCH: released call has spend or an invocation",
                ));
            }
        } else if !matches!(
            reservation.status.as_str(),
            "DISPATCHED" | "RECONCILED" | "UNRECONCILED_MAXIMUM_RETAINED" | "BOUND_VIOLATED"
        ) || attempts.len() != 1
            || attempts[0].role != reservation.role
        {
            return Err(invalid(
                "RECOVERY_ACCOUNTING_MISMATCH: terminal call slots differ from recorded attempts",
            ));
        } else {
            let attempt = attempts[0];
            let expected_usage = if attempt.usage_authority == "TRANSPORT_METADATA"
                && (attempt.status == "COMPLETED" || reservation.status == "BOUND_VIOLATED")
            {
                attempt.usage.as_ref()
            } else {
                None
            };
            if reservation.actual.as_ref() != expected_usage {
                return Err(invalid(
                    "RECOVERY_ACCOUNTING_MISMATCH: terminal usage differs from the saved invocation",
                ));
            }
            let driver = match reservation.role.as_str() {
                "author" => Some(&config.author),
                "reviewer" => Some(&config.reviewer),
                "fallback" => config.fallback.as_ref(),
                _ => None,
            }
            .ok_or_else(|| invalid("RECOVERY_ACCOUNTING_MISMATCH: unknown terminal role"))?;
            let expected = if reservation.status == "DISPATCHED" {
                (reservation.maximum.clone(), "DISPATCHED".to_owned())
            } else {
                let (charged, status, _) = reconciliation(
                    &reservation.maximum,
                    reservation.actual.as_ref(),
                    driver.cap.overhead_input_tokens,
                )?;
                (charged, status)
            };
            if reservation.charged != expected.0
                || reservation.status != expected.1
                || (reservation.status == "DISPATCHED" && reservation.actual.is_some())
            {
                return Err(invalid(
                    "RECOVERY_ACCOUNTING_MISMATCH: terminal charge differs from retained accounting",
                ));
            }
        }
    }
    if report
        .attempts
        .iter()
        .any(|attempt| !existing.contains_key(&attempt.reservation))
    {
        return Err(invalid(
            "RECOVERY_ACCOUNTING_MISMATCH: terminal invocation has no configured reservation",
        ));
    }
    Ok(())
}

/// Complete only the accounting tail of an interrupted terminal write. A normal
/// replay makes no changes; neither path allocates slots or dispatches a call.
pub(super) fn complete_terminal_accounting(
    repo: &Repository,
    config: &Config,
    report: &mut RunReport,
) -> Result<(), ClewError> {
    let guard = repo.lock()?;
    let mut ledger = account(repo, &config.budget)?;
    let mut changed = false;
    for (id, reservation) in ledger
        .reservations
        .iter_mut()
        .filter(|(_, r)| r.run == report.run)
    {
        if reservation.status != "RESERVED" {
            continue;
        }
        let attempts: Vec<_> = report
            .attempts
            .iter()
            .filter(|a| a.reservation == *id)
            .collect();
        let prepared_cancel = attempts.len() == 1
            && report.status.ends_with("CANCELLED")
            && attempts[0].status == "PREPARED";
        if (!attempts.is_empty() && !prepared_cancel)
            || reservation.actual.is_some()
            || reservation.charged != reservation.maximum
        {
            return Err(invalid(
                "RECOVERY_ACCOUNTING_MISMATCH: terminal reserved slot was dispatched or changed",
            ));
        }
        reservation.status = "RELEASED_NOT_DISPATCHED".into();
        reservation.charged = Amount::default();
        changed = true;
    }
    let existing: BTreeMap<_, _> = ledger
        .reservations
        .iter()
        .filter(|(_, r)| r.run == report.run)
        .map(|(id, r)| (id.clone(), r.clone()))
        .collect();
    validate_terminal_reservation_records(config, report, &existing)?;
    let accounting = serde_json::json!(
        existing
            .iter()
            .map(|(id, record)| serde_json::json!({"reservation":id,"record":record}))
            .collect::<Vec<_>>()
    );
    if report
        .accounting
        .as_ref()
        .is_some_and(|saved| *saved != accounting)
    {
        return Err(invalid(
            "RECOVERY_ACCOUNTING_MISMATCH: terminal accounting summary differs from retained slots",
        ));
    }
    if changed {
        save_account(repo, &config.budget, &ledger)?;
    }
    if report.accounting.is_none() {
        report.accounting = Some(accounting);
        save_report_locked(repo, &guard, report)?;
    }
    Ok(())
}
pub fn dispatch(
    repo: &Repository,
    budget: &Budget,
    run: &str,
    role: &str,
) -> Result<String, ClewError> {
    let _lock = repo.lock()?;
    let mut ledger = account(repo, budget)?;
    if ledger
        .reservations
        .values()
        .any(|r| r.status == "BOUND_VIOLATED")
    {
        return Err(invalid(
            "ACCOUNTING_BOUND_VIOLATED: this account cannot dispatch further calls",
        ));
    }
    let (id, reservation) = ledger
        .reservations
        .iter_mut()
        .find(|(_, r)| r.run == run && r.role == role && r.status == "RESERVED")
        .ok_or_else(|| invalid("CALLS_EXHAUSTED: no reserved call remains for this role"))?;
    reservation.status = "DISPATCHED".into();
    let id = id.clone();
    save_account(repo, budget, &ledger)?;
    Ok(id)
}

fn reservation(
    repo: &Repository,
    budget: &Budget,
    id: &str,
    run: &str,
    role: &str,
) -> Result<Reservation, ClewError> {
    let value = account(repo, budget)?
        .reservations
        .remove(id)
        .ok_or_else(|| invalid("RECOVERY_RESERVATION_MISSING: reserved call is absent"))?;
    if value.run != run || value.role != role {
        return Err(invalid(
            "RECOVERY_RESERVATION_MISMATCH: saved call refers to another run or role",
        ));
    }
    Ok(value)
}

fn next_reserved(
    repo: &Repository,
    budget: &Budget,
    run: &str,
    role: &str,
) -> Result<String, ClewError> {
    account(repo, budget)?
        .reservations
        .into_iter()
        .find(|(_, value)| value.run == run && value.role == role && value.status == "RESERVED")
        .map(|(id, _)| id)
        .ok_or_else(|| invalid("CALLS_EXHAUSTED: no reserved call remains for this role"))
}

fn dispatch_reserved(
    repo: &Repository,
    budget: &Budget,
    id: &str,
    run: &str,
    role: &str,
) -> Result<(), ClewError> {
    let _lock = repo.lock()?;
    let mut ledger = account(repo, budget)?;
    if ledger
        .reservations
        .values()
        .any(|r| r.status == "BOUND_VIOLATED")
    {
        return Err(invalid(
            "ACCOUNTING_BOUND_VIOLATED: this account cannot dispatch further calls",
        ));
    }
    let reservation = ledger
        .reservations
        .get_mut(id)
        .ok_or_else(|| invalid("unknown reservation"))?;
    if reservation.run != run || reservation.role != role || reservation.status != "RESERVED" {
        return Err(invalid(
            "RECOVERY_RESERVATION_MISMATCH: selected account slot is not reserved for this call",
        ));
    }
    reservation.status = "DISPATCHED".into();
    save_account(repo, budget, &ledger)
}

fn reconciliation(
    maximum: &Amount,
    usage: Option<&Usage>,
    overhead: u64,
) -> Result<(Amount, String, bool), ClewError> {
    let usage_value = usage.cloned().unwrap_or_default();
    let actual_input = usage_value
        .input_tokens
        .map(|tokens| {
            tokens
                .checked_add(overhead)
                .ok_or_else(|| invalid("usage overflow"))
        })
        .transpose()?;
    let charged = Amount {
        input_tokens: actual_input.unwrap_or(maximum.input_tokens),
        output_tokens: usage_value.output_tokens.unwrap_or(maximum.output_tokens),
        cost_units: usage_value.cost_units.unwrap_or(maximum.cost_units),
    };
    let violated = !charged.within(maximum);
    let status = if violated {
        "BOUND_VIOLATED"
    } else if actual_input.is_none()
        || usage_value.output_tokens.is_none()
        || usage_value.cost_units.is_none()
    {
        "UNRECONCILED_MAXIMUM_RETAINED"
    } else {
        "RECONCILED"
    };
    Ok((charged, status.into(), violated))
}

pub fn reconcile(
    repo: &Repository,
    budget: &Budget,
    id: &str,
    usage: Option<Usage>,
    overhead: u64,
) -> Result<(), ClewError> {
    let _lock = repo.lock()?;
    let mut ledger = account(repo, budget)?;
    let reservation = ledger
        .reservations
        .get_mut(id)
        .ok_or_else(|| invalid("unknown reservation"))?;
    let (charged, status, violated) =
        reconciliation(&reservation.maximum, usage.as_ref(), overhead)?;
    if reservation.status == "DISPATCHED" {
        reservation.charged = charged;
        reservation.actual = usage;
        reservation.status = status;
    } else if reservation.charged != charged
        || reservation.actual != usage
        || reservation.status != status
    {
        return Err(invalid(
            "RECOVERY_ACCOUNTING_MISMATCH: reservation was reconciled with different result data",
        ));
    }
    save_account(repo, budget, &ledger)?;
    if violated {
        return Err(invalid(
            "ACCOUNTING_BOUND_VIOLATED: adapter reported usage beyond its admitted upper bound; further calls are denied",
        ));
    }
    Ok(())
}
pub fn release_unused(repo: &Repository, budget: &Budget, run: &str) -> Result<(), ClewError> {
    let _lock = repo.lock()?;
    let mut ledger = account(repo, budget)?;
    for r in ledger
        .reservations
        .values_mut()
        .filter(|r| r.run == run && r.status == "RESERVED")
    {
        r.status = "RELEASED_NOT_DISPATCHED".into();
        r.charged = Amount::default();
    }
    save_account(repo, budget, &ledger)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reply {
    schema: String,
    model: String,
    invocation: String,
    role: String,
    #[serde(default)]
    usage: Option<Usage>,
    result: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attempt {
    pub invocation: String,
    pub model: String,
    pub usage_authority: String,
    pub role: String,
    pub input_digest: String,
    /// Complete canonical job envelope bytes, not provider tokens or billing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_bytes: Option<usize>,
    pub reservation: String,
    pub status: String,
    pub admission: Value,
    pub failure: Option<String>,
    pub usage: Option<Usage>,
    pub result_digest: Option<String>,
    pub captured_stdout_bytes: usize,
    pub captured_stderr_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_contract: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapted_proposal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncertainty_adaptation: Option<UncertaintyAdaptationReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion_selection: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UncertaintyAdaptationReceipt {
    pub original_result_digest: String,
    pub original_proposal_digest: String,
    pub adapted_value_digest: String,
    pub from: String,
    pub to: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunReport {
    pub schema: String,
    pub run: String,
    pub work: String,
    pub status: String,
    pub config_digest: Option<String>,
    pub attempts: Vec<Attempt>,
    pub proposal: Option<String>,
    pub review: Option<Value>,
    pub publication: Option<Value>,
    pub gap: Option<Value>,
    pub accounting: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_budget: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) draft_repair: Option<DraftRepairOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_review: Option<operation_draft_review::Origin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_review_retry: Option<operation_draft_review::RetryOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint: Option<recovery::CheckpointRef>,
}

const DRAFT_REPAIR_SCHEMA: &str = "codeclew-operation-draft-repair/1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DraftRepairOrigin {
    pub(super) schema: String,
    pub(super) source_run: String,
    pub(in crate::documentation::agent_jobs) source_checkpoint: recovery::CheckpointRef,
    pub(super) source_invocation: String,
    pub(super) source_input_digest: String,
    pub(super) source_result_digest: String,
    pub(super) packet_digest: String,
    pub(super) source_authoring_contract: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rejection: Option<DraftRepairRejection>,
}

const DRAFT_REPAIR_REJECTION_SCHEMA: &str = "codeclew-operation-draft-repair-rejection/1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DraftRepairRejection {
    pub(super) schema: String,
    pub(super) review_run: String,
    review_checkpoint: recovery::CheckpointRef,
    pub(super) reviewer_invocation: String,
    pub(super) reviewer_input_digest: String,
    pub(super) reviewer_result_digest: String,
    pub(super) coverage_digest: String,
}

const RUN_CHECKPOINT_SCHEMA: &str = "codeclew-documentation-agent-run-checkpoint/1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingCall {
    identity: recovery::CallIdentity,
    status: String,
    failure: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunCheckpoint {
    schema: String,
    run: String,
    work: String,
    snapshot: String,
    config_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_repair: Option<DraftRepairOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_review: Option<operation_draft_review::Origin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft_review_retry: Option<operation_draft_review::RetryOrigin>,
    driver_digests: BTreeMap<String, String>,
    phase: String,
    pages: Vec<Value>,
    source_parts: Vec<Value>,
    feedback: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expansion_feedback: Option<Value>,
    previous: Value,
    previous_section: Value,
    repairs_remaining: u32,
    expansions_remaining: u32,
    fallback: bool,
    fallback_candidates: u32,
    read_digest: String,
    pending_call: Option<PendingCall>,
    proposal_id: Option<String>,
    proposal_read_digest: Option<String>,
    proposal_evidence_digest: Option<String>,
    reviewer_invocation: Option<String>,
    reviewer_driver_digest: Option<String>,
    reviewer_identity: Option<recovery::CallIdentity>,
    review: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publication_baseline: Option<PublicationBaseline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publication_receipt: Option<super::history::PublicationReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicationBaseline {
    bundle: Option<String>,
    index_digest: Option<String>,
    bindings_digest: Option<String>,
}

impl RunCheckpoint {
    fn new(
        report: &RunReport,
        snapshot: String,
        config_digest: String,
        driver_digests: BTreeMap<String, String>,
        read_digest: String,
    ) -> Self {
        Self {
            schema: RUN_CHECKPOINT_SCHEMA.into(),
            run: report.run.clone(),
            work: report.work.clone(),
            snapshot,
            config_digest,
            execution_mode: report.execution_mode.clone(),
            draft_repair: report.draft_repair.clone(),
            draft_review: report.draft_review.clone(),
            draft_review_retry: report.draft_review_retry.clone(),
            driver_digests,
            phase: "AUTHOR".into(),
            pages: Vec::new(),
            source_parts: Vec::new(),
            feedback: Value::Null,
            expansion_feedback: None,
            previous: Value::Null,
            previous_section: Value::Null,
            repairs_remaining: 0,
            expansions_remaining: 0,
            fallback: false,
            fallback_candidates: 0,
            read_digest,
            pending_call: None,
            proposal_id: None,
            proposal_read_digest: None,
            proposal_evidence_digest: None,
            reviewer_invocation: None,
            reviewer_driver_digest: None,
            reviewer_identity: None,
            review: None,
            publication_baseline: None,
            publication_receipt: None,
        }
    }

    fn validate(
        &self,
        report: &RunReport,
        config_digest: &str,
        driver_digests: &BTreeMap<String, String>,
    ) -> Result<(), ClewError> {
        if self.schema != RUN_CHECKPOINT_SCHEMA
            || self.run != report.run
            || self.work != report.work
        {
            return Err(invalid(
                "RECOVERY_CHECKPOINT_MISMATCH: saved phase belongs to another run or Work",
            ));
        }
        if self.config_digest != config_digest || &self.driver_digests != driver_digests {
            return Err(invalid(
                "RECOVERY_CONFIG_MISMATCH: execution config or admitted driver changed during a nonterminal run",
            ));
        }
        if self.execution_mode != report.execution_mode {
            return Err(invalid(
                "RECOVERY_MODE_MISMATCH: saved phase belongs to a different execution mode",
            ));
        }
        if self.draft_review != report.draft_review {
            return Err(invalid(
                "RECOVERY_CHECKPOINT_MISMATCH: saved draft review origin differs from the run report",
            ));
        }
        if self.draft_review_retry != report.draft_review_retry {
            return Err(invalid(
                "RECOVERY_CHECKPOINT_MISMATCH: saved review retry lineage differs from the run report",
            ));
        }
        if self.draft_repair != report.draft_repair {
            return Err(invalid(
                "RECOVERY_CHECKPOINT_MISMATCH: saved draft repair origin differs from the run report",
            ));
        }
        if !matches!(
            self.phase.as_str(),
            "AUTHOR" | "REVIEWER" | "PUBLISH" | "TERMINAL"
        ) || self
            .read_digest
            .strip_prefix("sha256:")
            .is_none_or(|hex| hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(invalid(
                "RECOVERY_CHECKPOINT_CORRUPT: invalid phase or context bound",
            ));
        }
        if self.publication_receipt.is_some() && self.publication_baseline.is_none() {
            return Err(invalid(
                "RECOVERY_CHECKPOINT_CORRUPT: publication receipt has no selected original baseline",
            ));
        }
        Ok(())
    }
}

fn save_run_checkpoint(
    repo: &Repository,
    report: &mut RunReport,
    checkpoint: &RunCheckpoint,
) -> Result<(), ClewError> {
    let guard = repo.lock()?;
    save_run_checkpoint_locked(repo, &guard, report, checkpoint)
}

fn save_run_checkpoint_locked(
    repo: &Repository,
    guard: &WriteLock,
    report: &mut RunReport,
    checkpoint: &RunCheckpoint,
) -> Result<(), ClewError> {
    let sequence = report.checkpoint.as_ref().map_or(Ok(1), |reference| {
        reference
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("RECOVERY_CHECKPOINT_SEQUENCE_OVERFLOW"))
    })?;
    let reference =
        recovery::save_checkpoint_locked(repo, guard, &report.run, sequence, checkpoint)?;
    let mut candidate = report.clone();
    candidate.checkpoint = Some(reference);
    save_report_locked(repo, guard, &candidate)?;
    *report = candidate;
    Ok(())
}

fn load_run_checkpoint(
    repo: &Repository,
    report: &RunReport,
    config_digest: &str,
    driver_digests: &BTreeMap<String, String>,
) -> Result<Option<RunCheckpoint>, ClewError> {
    let Some(reference) = report.checkpoint.as_ref() else {
        if report.attempts.is_empty() {
            return Ok(None);
        }
        return Err(invalid(
            "RECOVERY_LEGACY_STATE: dispatched run has no durable phase checkpoint",
        ));
    };
    let checkpoint: RunCheckpoint = recovery::load_checkpoint(repo, reference)?;
    checkpoint.validate(report, config_digest, driver_digests)?;
    Ok(Some(checkpoint))
}

fn report_path(run: &str) -> Result<String, ClewError> {
    if run.len() != 32 || !run.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("invalid run identity"));
    }
    Ok(format!(".codeclew/jobs/{run}.json"))
}

/// Discover historical approvals without using the mutable latest pointer to
/// select candidates. A retained pointer is still checked for damaged history.
/// Every report is decoded and identity-checked, including unrelated Work, so a
/// damaged history cannot silently turn an incomplete search into a unique hit.
pub(in crate::documentation) fn historical_approved_runs(
    repo: &Repository,
    work_ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<String>>, ClewError> {
    // A missing report must not disappear from discovery merely because its
    // checkpoint directory was also lost. The pointer is an integrity locator,
    // never the source of the candidate set or a preference between answers.
    // Callers supply every validated manifest identity, before filtering by
    // request. An unrelated broken pointer also makes the history incomplete.
    for work in work_ids {
        super::work::directory(work)?;
        latest_report(repo, work)?;
    }
    let path = repo.path(".codeclew/jobs")?;
    if !path.try_exists().map_err(io_error)? {
        return Ok(BTreeMap::new());
    }
    let mut runs = BTreeSet::new();
    let mut retained_run_directories = BTreeSet::new();
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(invalid(
                "RECOVERY_REPORT_CORRUPT: report filename is not UTF-8",
            ));
        };
        let Some(run) = name.strip_suffix(".json") else {
            if name.len() == 32 && name.bytes().all(|c| c.is_ascii_hexdigit()) {
                let path = repo.path(&format!(".codeclew/jobs/{name}"))?;
                if fs::metadata(path).map_err(io_error)?.is_dir() {
                    if retained_run_directories.len() >= store::MAX_RECORDS {
                        return Err(invalid(
                            "RECOVERY_REPORT_CORRUPT: excessive retained run directories",
                        ));
                    }
                    retained_run_directories.insert(name.to_owned());
                }
            }
            continue;
        };
        report_path(run)?;
        if runs.len() >= store::MAX_RECORDS {
            return Err(invalid(
                "RECOVERY_REPORT_CORRUPT: excessive historical reports",
            ));
        }
        runs.insert(run.to_owned());
    }
    if let Some(run) = retained_run_directories.difference(&runs).next() {
        return Err(invalid(format!(
            "RECOVERY_REPORT_CORRUPT: retained run directory {run} has no historical report; restore complete documentation-root history before discovery"
        )));
    }
    let mut approved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for run in runs {
        let report: RunReport = store::read(&repo.path(&report_path(&run)?)?, 4 * 1024 * 1024)
            .map_err(|error| {
                invalid(format!(
                    "RECOVERY_REPORT_CORRUPT: historical report cannot be decoded ({})",
                    error.message
                ))
            })?;
        super::work::directory(&report.work)?;
        validate_report_identity(&report, &report.work, &run)?;
        if !work_ids.contains(&report.work) {
            return Err(invalid(format!(
                "RECOVERY_REPORT_MISMATCH: historical report {run} binds unavailable Work {}; restore complete documentation-root history before discovery",
                report.work
            )));
        }
        if report.status == "DRAFT_REVIEW_APPROVED" {
            approved.entry(report.work).or_default().push(run);
        }
    }
    Ok(approved)
}

/// Additional current-policy admission after `load_approved_answer` establishes
/// the durable historical approval. Unsupported history remains historical.
pub(in crate::documentation) fn validate_reusable_chain(
    repo: &Repository,
    work: &super::work::Work,
    review_run: &str,
) -> Result<(), ClewError> {
    operation_draft_review::validate_reusable_chain(repo, work, review_run)
}

fn validate_reuse_policy(saved: &Value, current: &Value, role: &str) -> Result<(), ClewError> {
    if saved != current {
        return Err(invalid(format!(
            "ANSWER_REUSE_UNSUPPORTED: saved {role} input differs from current {role} policy"
        )));
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn reusable_author_fixture_setup()
-> (tempfile::TempDir, Repository, super::work::Work, PathBuf) {
    let (temp, repo, work, path) = operation_draft::tests::setup("success");
    let mut config: Value = store::read(&path, store::MAX_RECORD).unwrap();
    config["schema"] = serde_json::json!("codeclew-documentation-operation-draft-execution/1.1");
    config["authorCalls"] = serde_json::json!(1);
    let driver = config["author"]["command"][4].as_str().unwrap().replace(
        "\"result\" => answer",
        "\"result\" => {\"action\" => \"answer\", \"answer\" => answer}",
    );
    config["author"]["command"][4] = serde_json::json!(driver);
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    (temp, repo, work, path)
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn model_ids_grouped_author_fixture_setup() -> (
    tempfile::TempDir,
    Repository,
    super::work::Work,
    PathBuf,
    Value,
) {
    operation_draft::model_ids_grouped_fixture_setup()
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn write_model_ids_grouped_review_fixture_config(
    path: &std::path::Path,
    author: &std::path::Path,
) -> Value {
    operation_draft_review::model_ids_grouped_review_config(path, author)
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn interrupt_model_head_publication_once_for_test() {
    recovery::interrupt_model_head_once();
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn interrupt_raw_model_result_once_for_test() {
    recovery::interrupt_raw_model_result_once();
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn validate_frozen_model_records_for_test(
    repo: &Repository,
    invocation: &str,
) -> Result<Value, crate::error::ClewError> {
    recovery::validate_frozen_model_records(repo, invocation)
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::documentation) fn write_reusable_review_fixture_config(
    path: &std::path::Path,
    author: &std::path::Path,
    mode: &str,
) {
    operation_draft_review::write_reusable_review_fixture_config(path, author, mode);
}

fn latest_report(repo: &Repository, work: &str) -> Result<Option<RunReport>, ClewError> {
    let pointer_path = repo.path(&format!(".codeclew/work/{work}/latest-run.json"))?;
    if !pointer_path.exists() {
        return Ok(None);
    }
    let pointer: Value = store::read(&pointer_path, store::MAX_RECORD).map_err(|error| {
        invalid(format!(
            "RECOVERY_REPORT_CORRUPT: latest run pointer cannot be decoded ({})",
            error.message
        ))
    })?;
    let run = pointer["run"]
        .as_str()
        .ok_or_else(|| invalid("RECOVERY_REPORT_CORRUPT: latest run pointer is invalid"))?;
    load_report_by_id(repo, work, run).map(Some)
}

pub(super) fn load_report_by_id(
    repo: &Repository,
    work: &str,
    run: &str,
) -> Result<RunReport, ClewError> {
    let report: RunReport =
        store::read(&repo.path(&report_path(run)?)?, 4 * 1024 * 1024).map_err(|error| {
            invalid(format!(
                "RECOVERY_REPORT_CORRUPT: selected run report cannot be decoded ({})",
                error.message
            ))
        })?;
    validate_report_identity(&report, work, run)?;
    Ok(report)
}

fn validate_report_identity(report: &RunReport, work: &str, run: &str) -> Result<(), ClewError> {
    if report.schema != "codeclew-documentation-work-run/1.0"
        || report.work != work
        || report.run != run
    {
        return Err(invalid(
            "RECOVERY_REPORT_MISMATCH: selected report belongs to another Work or run",
        ));
    }
    Ok(())
}
fn save_report(repo: &Repository, report: &RunReport) -> Result<(), ClewError> {
    let guard = repo.lock()?;
    save_report_locked(repo, &guard, report)
}

fn save_report_locked(
    repo: &Repository,
    _guard: &WriteLock,
    report: &RunReport,
) -> Result<(), ClewError> {
    let data = bytes(report)?;
    if data.len() > 4 * 1024 * 1024 {
        return Err(invalid("job report exceeds 4 MiB"));
    }
    repo.atomic(&report_path(&report.run)?, &data)?;
    repo.atomic(
        &format!(".codeclew/work/{}/latest-run.json", report.work),
        &bytes(&serde_json::json!({"run":report.run}))?,
    )
}

pub fn status(
    repo: &Repository,
    work: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    super::work::load(repo, work)?;
    let pointer: Value = store::read(
        &repo.path(&format!(".codeclew/work/{work}/latest-run.json"))?,
        store::MAX_RECORD,
    )?;
    let report: RunReport = store::read(
        &repo.path(&report_path(
            pointer["run"]
                .as_str()
                .ok_or_else(|| invalid("invalid run pointer"))?,
        )?)?,
        4 * 1024 * 1024,
    )?;
    let rows=report.attempts.iter().map(|a|serde_json::json!({"kind":"ATTEMPT","id":a.invocation,"record":a})).chain(report.accounting.as_ref().and_then(|v|v.as_array()).into_iter().flatten().enumerate().map(|(index,a)|serde_json::json!({"kind":"ACCOUNTING","id":format!("accounting-{index}"),"record":a}))).collect();
    let mut metadata = serde_json::json!({"reportSchema":report.schema,"run":report.run,"work":report.work,"status":report.status,"executionMode":report.execution_mode,"draft":report.draft,"draftRepair":report.draft_repair,"configDigest":report.config_digest,"proposal":report.proposal,"publication":report.publication,"gap":report.gap,"contextBudget":report.context_budget});
    if let Some(origin) = report.draft_review.as_ref() {
        metadata["draftReview"] = serde_json::json!(origin);
        metadata["review"] = serde_json::json!(report.review);
        if let Some(retry) = &report.draft_review_retry {
            metadata["draftReviewRetry"] = serde_json::json!(retry);
        }
    }
    super::cli::page(&digest(&report)?, rows, cursor, limit, metadata)
}
pub fn cancel(repo: &Repository, work: &str) -> Result<Value, ClewError> {
    super::work::load(repo, work)?;
    let report = latest_report(repo, work)?.ok_or_else(|| {
        invalid("NO_ACTIVE_RUN: prepare or start a Work run before cancelling it")
    })?;
    // Cancellation is an idempotent signal on its own path, independent of the
    // publication/account lock; a busy coordinator must not prevent cancellation.
    // The marker belongs to this run so it cannot cancel a corrected later run.
    repo.atomic(
        &format!(".codeclew/jobs/{}/cancel.json", report.run),
        &bytes(&serde_json::json!({"schema":"codeclew-documentation-cancel/1.0","work":work,"run":report.run.clone()}))?,
    )?;
    Ok(
        serde_json::json!({"schema":"codeclew-documentation-cancel/1.0","status":"CANCELLATION_REQUESTED","work":work,"run":report.run}),
    )
}

fn job_envelope(
    report: &RunReport,
    role_name: &str,
    driver: &Role,
    invocation: &str,
    payload: Value,
    expansions_remaining: Option<u32>,
) -> Value {
    let mut request = serde_json::json!({
        "schema":"codeclew-documentation-agent-job/1.0",
        "invocation":invocation,
        "role":role_name,
        "model":driver.model,
        "work":report.work,
        "cap":driver.cap,
        "payload":payload
    });
    if let Some(remaining) = expansions_remaining {
        request["expansionBudget"] = serde_json::json!({
            "scope":"SHARED_ACROSS_ROLES",
            "remaining":remaining,
            "meaning":"Each registered expansion action consumes one unit, whether it reads a navigation page or a full result set. The count is shared across author, repair, fallback, and reviewer calls."
        });
    }
    request
}

fn ensure_input_cap(driver: &Role, request: &Value) -> Result<usize, ClewError> {
    let request_bytes = bytes(request)?.len();
    ensure_input_bytes_cap(driver, request_bytes)?;
    Ok(request_bytes)
}

fn ensure_input_bytes_cap(driver: &Role, request_bytes: usize) -> Result<(), ClewError> {
    if u64::try_from(request_bytes)
        .ok()
        .and_then(|bytes| bytes.checked_add(driver.cap.overhead_input_tokens))
        .is_none_or(|n| n > driver.cap.maximum.input_tokens)
    {
        return Err(invalid(
            "INPUT_CAP_EXCEEDED: expand a narrower work package before calling a model",
        ));
    }
    Ok(())
}

// Keep call identity and driver inputs explicit at this checkpoint boundary.
#[allow(clippy::too_many_arguments)]
fn call(
    repo: &Repository,
    c: &Config,
    report: &mut RunReport,
    checkpoint: &mut RunCheckpoint,
    role_name: &str,
    driver: &Role,
    payload: Value,
    author_contract: Option<Value>,
    retry_uncertain_dispatch: bool,
    include_expansion_budget: bool,
) -> Result<(Value, String, String), ClewError> {
    let cancel_path = repo.path(&format!(".codeclew/jobs/{}/cancel.json", report.run))?;
    if checkpoint.pending_call.is_none() && cancel_path.exists() {
        return Err(invalid(
            "CANCELLED: cancellation was requested before preparing a new call",
        ));
    }
    let admission = super::agent_adapter::admit(repo, driver)?;
    let driver_digest = admission["driverDigest"]
        .as_str()
        .ok_or_else(|| invalid("missing driver admission digest"))?
        .to_owned();
    if checkpoint
        .driver_digests
        .get(role_name)
        .is_none_or(|expected| expected != &driver_digest)
    {
        return Err(invalid(
            "RECOVERY_DRIVER_MISMATCH: admitted driver changed during the run",
        ));
    }
    let config_digest = report
        .config_digest
        .as_deref()
        .ok_or_else(|| invalid("recovery run has no configuration digest"))?;
    let read_digest = digest(&super::work::read_state(repo, &report.work)?)?;
    if read_digest != checkpoint.read_digest {
        return Err(invalid(
            "RECOVERY_CONTEXT_MISMATCH: recorded Work reads changed since the selected checkpoint",
        ));
    }

    let mut resumed = false;
    let (identity, input, request, attempt_index) = if let Some(pending) = &checkpoint.pending_call
    {
        resumed = true;
        let identity = pending.identity.clone();
        if identity.run != report.run
            || identity.work != report.work
            || identity.config_digest != config_digest
            || identity.driver_digest != driver_digest
            || identity.role != role_name
            || identity.model != driver.model
            || identity.usage_authority != driver.usage_authority
            || identity.snapshot != checkpoint.snapshot
        {
            return Err(invalid(
                "RECOVERY_CALL_BINDING_MISMATCH: pending invocation does not match this run config or Work snapshot",
            ));
        }
        let expansion_budget = include_expansion_budget.then_some(checkpoint.expansions_remaining);
        let request = job_envelope(
            report,
            role_name,
            driver,
            &identity.invocation,
            payload,
            expansion_budget,
        );
        ensure_input_cap(driver, &request)?;
        let candidate = recovery::InputRecord::new(
            recovery::CallBinding {
                run: report.run.clone(),
                work: report.work.clone(),
                snapshot: identity.snapshot.clone(),
                reservation: identity.reservation.clone(),
                invocation: identity.invocation.clone(),
                role: role_name.into(),
                model: driver.model.clone(),
                usage_authority: driver.usage_authority.clone(),
                config_digest: config_digest.into(),
                driver_digest: driver_digest.clone(),
            },
            request.clone(),
        )?;
        if candidate.identity != identity {
            return Err(invalid(
                "RECOVERY_CALL_BINDING_MISMATCH: regenerated semantic input differs from the saved invocation",
            ));
        }
        let input = recovery::load_input(repo, &identity)?;
        if input.request != request {
            return Err(invalid(
                "RECOVERY_INPUT_BINDING_MISMATCH: regenerated request differs from the immutable input",
            ));
        }
        let attempt_index = report
            .attempts
            .iter()
            .position(|attempt| attempt.invocation == identity.invocation)
            .ok_or_else(|| invalid("RECOVERY_REPORT_MISMATCH: pending attempt is absent"))?;
        let attempt = &report.attempts[attempt_index];
        if attempt.reservation != identity.reservation
            || attempt.role != role_name
            || attempt.model != driver.model
            || attempt.input_digest != identity.input_digest
        {
            return Err(invalid(
                "RECOVERY_REPORT_MISMATCH: attempt does not match its immutable input binding",
            ));
        }
        (identity, input, request, attempt_index)
    } else {
        let invocation = uuid::Uuid::new_v4().simple().to_string();
        let reservation = next_reserved(repo, &c.budget, &report.run, role_name)?;
        let expansion_budget = include_expansion_budget.then_some(checkpoint.expansions_remaining);
        let request = job_envelope(
            report,
            role_name,
            driver,
            &invocation,
            payload,
            expansion_budget,
        );
        let request_bytes = ensure_input_cap(driver, &request)?;
        let snapshot = checkpoint.snapshot.clone();
        let input = recovery::InputRecord::new(
            recovery::CallBinding {
                run: report.run.clone(),
                work: report.work.clone(),
                snapshot,
                reservation: reservation.clone(),
                invocation: invocation.clone(),
                role: role_name.into(),
                model: driver.model.clone(),
                usage_authority: driver.usage_authority.clone(),
                config_digest: config_digest.into(),
                driver_digest: driver_digest.clone(),
            },
            request.clone(),
        )?;
        recovery::save_input(repo, &input)?;
        report.attempts.push(Attempt {
            invocation,
            model: driver.model.clone(),
            usage_authority: driver.usage_authority.clone(),
            role: role_name.into(),
            input_digest: input.identity.input_digest.clone(),
            request_bytes: Some(request_bytes),
            reservation,
            status: "PREPARED".into(),
            admission: admission.clone(),
            failure: None,
            usage: None,
            result_digest: None,
            captured_stdout_bytes: 0,
            captured_stderr_bytes: 0,
            author_contract: author_contract.clone(),
            adapted_proposal: None,
            uncertainty_adaptation: None,
            expansion_selection: None,
        });
        let attempt_index = report.attempts.len() - 1;
        checkpoint.pending_call = Some(PendingCall {
            identity: input.identity.clone(),
            status: "PREPARED".into(),
            failure: None,
        });
        save_run_checkpoint(repo, report, checkpoint)?;
        (input.identity.clone(), input, request, attempt_index)
    };

    if let Some(pending) = checkpoint.pending_call.as_ref()
        && pending.status == "FAILED"
    {
        return Err(invalid(pending.failure.as_deref().unwrap_or(
            "RECOVERY_DRIVER_FAILURE: saved attempt already failed",
        )));
    }

    let model_input = match driver.model_representation.as_deref() {
        None => None,
        Some(version) if super::model_ids::supported_version(version) => {
            if resumed
                && checkpoint
                    .pending_call
                    .as_ref()
                    .is_some_and(|pending| pending.status != "PREPARED")
            {
                Some(recovery::try_load_model_input(repo,&input)?.ok_or_else(||invalid("RECOVERY_MODEL_INPUT_MISSING: dispatched call requires exact retained model carrier"))?)
            } else {
                let previous = report.attempts[..attempt_index]
                    .iter()
                    .rev()
                    .find(|attempt| attempt.role == role_name)
                    .map(|attempt| attempt.invocation.as_str());
                Some(recovery::prepare_model_input(
                    repo, &input, previous, version,
                )?)
            }
        }
        Some(_) => {
            return Err(invalid(
                "MODEL_REPRESENTATION_UNSUPPORTED: select a compatible versioned driver representation",
            ));
        }
    };
    if let Some(model) = &model_input
        && driver.model_representation.as_deref() != Some(model.prepared.version.as_str())
    {
        return Err(invalid(
            "RECOVERY_MODEL_INPUT_VERSION_MISMATCH: selected role mode differs from its frozen carrier",
        ));
    }
    let wire_request = model_input
        .as_ref()
        .map_or(&request, |model| &model.carrier);
    let delivered_bytes = ensure_input_cap(driver, wire_request)?;
    if model_input.is_some() {
        report.attempts[attempt_index].request_bytes = Some(delivered_bytes);
    }

    // A crash after receiving wire output but before decoding cannot turn into
    // an uncertain dispatch retry. Recover that exact delivered output first.
    if recovery::try_load_result(repo, &input)?.is_none()
        && let Some(model) = &model_input
        && let Some(raw) = recovery::try_load_raw_model_result(repo, model)?
    {
        if let Some(failure) = &raw.failure {
            return fail_call(
                repo,
                c,
                report,
                checkpoint,
                attempt_index,
                &identity,
                driver,
                failure.clone(),
            );
        }
        let reply =
            match decode_delivered_reply(raw.output.clone(), &identity, model_input.as_ref()) {
                Ok(reply) => reply,
                Err(error) => {
                    return fail_call(
                        repo,
                        c,
                        report,
                        checkpoint,
                        attempt_index,
                        &identity,
                        driver,
                        error.message,
                    );
                }
            };
        recovery::save_result_with_model_binding(
            repo,
            &input,
            reply.usage,
            reply.result,
            raw.stdout_bytes,
            raw.stderr_bytes,
            Some(raw.binding(model)),
        )?;
    }

    if let Some(saved) = recovery::try_load_result(repo, &input)? {
        if checkpoint
            .pending_call
            .as_ref()
            .is_some_and(|pending| pending.status == "RESULT_SAVED")
            || resumed
        {
            return finish_saved_result(
                repo,
                c,
                report,
                checkpoint,
                attempt_index,
                &identity,
                saved,
                &driver_digest,
                driver,
            );
        }
    } else if resumed
        && checkpoint
            .pending_call
            .as_ref()
            .is_some_and(|pending| pending.status == "RESULT_SAVED")
    {
        return Err(invalid(
            "RECOVERY_RESULT_MISSING: checkpoint selected a saved result that is absent",
        ));
    }

    let reservation = reservation(
        repo,
        &c.budget,
        &identity.reservation,
        &report.run,
        role_name,
    )?;
    if resumed
        && (reservation.status == "DISPATCHED"
            || (reservation.status == "UNRECONCILED_MAXIMUM_RETAINED"
                && reservation.actual.is_none()
                && reservation.charged == reservation.maximum))
    {
        if reservation.status == "DISPATCHED" {
            reconcile(
                repo,
                &c.budget,
                &identity.reservation,
                None,
                driver.cap.overhead_input_tokens,
            )?;
        }
        if !retry_uncertain_dispatch {
            let reason = if role_name == "reviewer" {
                "DRAFT_REVIEW_DISPATCH_UNCERTAIN: the reviewer may have received the request, no durable response exists, and the maximum reservation is retained; inspect the provider before any new review"
            } else {
                "DRAFT_DISPATCH_UNCERTAIN: the author may have received the request, no durable response exists, and the maximum reservation is retained; inspect the provider before preparing new Work"
            };
            report.attempts[attempt_index].status = "DISPATCH_UNCERTAIN_MAXIMUM_RETAINED".into();
            report.attempts[attempt_index].failure = Some(reason.into());
            if let Some(pending) = checkpoint.pending_call.as_mut() {
                pending.status = "UNCERTAIN_NO_RESULT".into();
                pending.failure = Some(reason.into());
            }
            save_run_checkpoint(repo, report, checkpoint)?;
            save_report(repo, report)?;
            return Err(invalid(reason));
        }
        report.attempts[attempt_index].status = "INTERRUPTED_MAXIMUM_RETAINED".into();
        report.attempts[attempt_index].failure =
            Some("RECOVERY_INTERRUPTED_NO_DURABLE_RESULT".into());
        checkpoint.pending_call = None;
        save_run_checkpoint(repo, report, checkpoint)?;
        return call(
            repo,
            c,
            report,
            checkpoint,
            role_name,
            driver,
            request["payload"].clone(),
            author_contract,
            retry_uncertain_dispatch,
            include_expansion_budget,
        );
    }

    if reservation.status != "RESERVED" {
        return Err(invalid(
            "RECOVERY_ACCOUNTING_MISMATCH: pending result has no replayable account reservation",
        ));
    }
    if cancel_path.exists() {
        return Err(invalid(
            "CANCELLED: cancellation was requested before dispatch",
        ));
    }
    dispatch_reserved(
        repo,
        &c.budget,
        &identity.reservation,
        &report.run,
        role_name,
    )?;
    report.attempts[attempt_index].status = "DISPATCHED".into();
    if let Some(pending) = checkpoint.pending_call.as_mut() {
        pending.status = "DISPATCHED".into();
    }
    save_run_checkpoint(repo, report, checkpoint)?;

    let executed = super::agent_adapter::execute(repo, driver, wire_request, &cancel_path);
    let (reply, failure) = match executed {
        Ok(result) => {
            report.attempts[attempt_index].admission = result.admission;
            report.attempts[attempt_index].captured_stdout_bytes = result.stdout_bytes;
            report.attempts[attempt_index].captured_stderr_bytes = result.stderr_bytes;
            let mut reply = None;
            let mut failure = result.failure;
            if let Some(output) = result.output {
                if let Some(model) = &model_input {
                    recovery::save_raw_model_result(
                        repo,
                        model,
                        output.clone(),
                        result.stdout_bytes,
                        result.stderr_bytes,
                        failure.clone(),
                    )?;
                }
                if failure.is_none() || model_input.is_none() {
                    match decode_delivered_reply(output, &identity, model_input.as_ref()) {
                        Ok(value) => {
                            reply = Some(value);
                        }
                        Err(error) => failure = Some(error.message),
                    }
                }
            } else if failure.is_none() {
                failure = Some("ROLE_MODEL_OR_DISPATCH_PROTOCOL_MISMATCH".into());
            }
            (reply, failure)
        }
        Err(error) => (None, Some(error.message)),
    };

    if let Some(value) = &reply {
        let model_binding = if let Some(model) = &model_input {
            Some(
                recovery::try_load_raw_model_result(repo, model)?
                    .ok_or_else(|| {
                        invalid(
                            "RECOVERY_MODEL_RESULT_MISSING: delivered wire output was not retained",
                        )
                    })?
                    .binding(model),
            )
        } else {
            None
        };
        recovery::save_result_with_model_binding(
            repo,
            &input,
            value.usage.clone(),
            value.result.clone(),
            report.attempts[attempt_index].captured_stdout_bytes,
            report.attempts[attempt_index].captured_stderr_bytes,
            model_binding,
        )?;
        report.attempts[attempt_index].usage = value.usage.clone();
        report.attempts[attempt_index].result_digest = Some(digest(&value.result)?);
        if let Some(pending) = checkpoint.pending_call.as_mut() {
            pending.status = "RESULT_SAVED".into();
        }
        save_run_checkpoint(repo, report, checkpoint)?;
    }
    if let Some(failure) = failure {
        return fail_call(
            repo,
            c,
            report,
            checkpoint,
            attempt_index,
            &identity,
            driver,
            failure,
        );
    }
    finish_saved_result(
        repo,
        c,
        report,
        checkpoint,
        attempt_index,
        &identity,
        recovery::load_result(repo, &input)?,
        &driver_digest,
        driver,
    )
}

fn decode_delivered_reply(
    output: Value,
    identity: &recovery::CallIdentity,
    model: Option<&recovery::ModelInputRecord>,
) -> Result<Reply, ClewError> {
    let mut reply: Reply = serde_json::from_value(output)
        .map_err(|_| invalid("ROLE_MODEL_OR_DISPATCH_PROTOCOL_MISMATCH"))?;
    if reply.schema != "codeclew-documentation-agent-result/1.0"
        || reply.invocation != identity.invocation
        || reply.role != identity.role
        || reply.model != identity.model
    {
        return Err(invalid("ROLE_MODEL_OR_DISPATCH_PROTOCOL_MISMATCH"));
    }
    if let Some(model) = model {
        reply.result = super::model_ids::decode_result(&reply.result, &model.prepared)?;
    }
    Ok(reply)
}

#[allow(clippy::too_many_arguments)]
fn fail_call(
    repo: &Repository,
    config: &Config,
    report: &mut RunReport,
    checkpoint: &mut RunCheckpoint,
    attempt_index: usize,
    identity: &recovery::CallIdentity,
    driver: &Role,
    failure: String,
) -> Result<(Value, String, String), ClewError> {
    report.attempts[attempt_index].status = "FAILED".into();
    report.attempts[attempt_index].failure = Some(failure.clone());
    if let Some(pending) = checkpoint.pending_call.as_mut() {
        pending.status = "FAILED".into();
        pending.failure = Some(failure.clone());
    }
    save_run_checkpoint(repo, report, checkpoint)?;
    reconcile(
        repo,
        &config.budget,
        &identity.reservation,
        None,
        driver.cap.overhead_input_tokens,
    )?;
    save_report(repo, report)?;
    Err(invalid(failure))
}

// Saved-result reconciliation deliberately receives the complete run state.
#[allow(clippy::too_many_arguments)]
fn finish_saved_result(
    repo: &Repository,
    config: &Config,
    report: &mut RunReport,
    checkpoint: &mut RunCheckpoint,
    attempt_index: usize,
    identity: &recovery::CallIdentity,
    saved: recovery::SavedResult,
    driver_digest: &str,
    driver: &Role,
) -> Result<(Value, String, String), ClewError> {
    let usage = if identity.usage_authority == "TRANSPORT_METADATA" {
        saved.usage.clone()
    } else {
        None
    };
    let accounting = reconcile(
        repo,
        &config.budget,
        &identity.reservation,
        usage,
        driver.cap.overhead_input_tokens,
    );
    let attempt = &mut report.attempts[attempt_index];
    attempt.usage = saved.usage.clone();
    attempt.result_digest = Some(saved.result_digest.clone());
    attempt.captured_stdout_bytes = saved.stdout_bytes;
    attempt.captured_stderr_bytes = saved.stderr_bytes;
    if let Err(error) = accounting {
        attempt.status = "FAILED".into();
        attempt.failure = Some(error.message.clone());
        if let Some(pending) = checkpoint.pending_call.as_mut() {
            pending.status = "FAILED".into();
            pending.failure = Some(error.message.clone());
        }
        save_run_checkpoint(repo, report, checkpoint)?;
        return Err(error);
    }
    attempt.status = "COMPLETED".into();
    attempt.failure = None;
    save_report(repo, report)?;
    Ok((
        saved.result,
        identity.invocation.clone(),
        driver_digest.into(),
    ))
}

// Phase persistence keeps each durable transition input visible at the callsite.

// Feedback advancement updates several bounded counters as one transition.

struct RunLock(File);
impl Drop for RunLock {
    fn drop(&mut self) {
        // SAFETY: the guard owns this open lock descriptor until Drop.
        let _ = unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

fn acquire_run_lock(repo: &Repository, work: &str) -> Result<RunLock, ClewError> {
    let path = repo.path(&format!(".codeclew/work/{work}/run.lock"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid("documentation run lock is not a regular file"));
    }
    // SAFETY: flock takes ownership of no Rust state and the FD remains open
    // for the complete coordinator run.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Err(crate::error::ClewError::new(
                crate::error::ErrorCode::WwConflict,
                "WORK_ALREADY_RUNNING: inspect or cancel the existing run before recovery",
            ));
        }
        return Err(io_error(error));
    }
    Ok(RunLock(file))
}
pub fn run_operation_draft(
    repo: &Repository,
    id: &str,
    config_path: Option<&std::path::Path>,
    new_run: bool,
    repair_from_run: Option<&str>,
    repair_from_review: Option<&str>,
) -> Result<Value, ClewError> {
    operation_draft::run(
        repo,
        id,
        config_path,
        new_run,
        repair_from_run,
        repair_from_review,
    )
}

pub fn review_operation_draft(
    repo: &Repository,
    id: &str,
    source_run: &str,
    config_path: &std::path::Path,
    retry_from_review: Option<&str>,
) -> Result<Value, ClewError> {
    operation_draft_review::run(repo, id, source_run, config_path, retry_from_review)
}

#[cfg(test)]
mod input_cap_tests {
    use super::*;

    #[test]
    fn absent_model_representation_preserves_legacy_role_bytes() {
        let legacy = serde_json::json!({"adapter":"local-command","model":"fixture","usageAuthority":"MAXIMUM_ONLY",
            "command":["/usr/bin/false"],"runtimeReads":[],"environment":[],"network":false,
            "cap":{"maximum":{"inputTokens":1000,"outputTokens":100,"costUnits":1},"overheadInputTokens":0,"timeoutMs":1000,"outputBytes":1000}});
        let role: Role = serde_json::from_value(legacy.clone()).unwrap();
        assert!(role.model_representation.is_none());
        assert_eq!(bytes(&role).unwrap(), bytes(&legacy).unwrap());
        assert_eq!(digest(&role).unwrap(), digest(&legacy).unwrap());
        let mut selected = role.clone();
        selected.model_representation = Some(super::super::model_ids::VERSION.into());
        assert_ne!(digest(&selected).unwrap(), digest(&role).unwrap());
    }
    use serde_json::json;

    #[test]
    fn recorded_grouped_retrieval_chunks_large_batches_drains_pages_and_is_idempotent() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Grouped retrieval").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let mut work = overview_work();
        work.id = "d".repeat(64);
        work.request.entrypoint = None;
        work.request.max_items = 2;
        work.request.max_bytes = 8192;
        let mut references = Vec::new();
        for index in 0..19 {
            let id = format!("dependency-{index}");
            let reference = format!("reference-{index}");
            let normalized = json!({"name": format!("method-{index}"), "value": index});
            let observation = super::super::model::Observation {
                id: id.clone(),
                kind: "SYMBOL".into(),
                service: "orders".into(),
                symbol: format!("method-{index}"),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: Vec::new(),
            };
            work.influence
                .insert(id.clone(), observation.digest.clone());
            work.checked.dependencies.insert(id.clone(), observation);
            work.handles.insert(
                reference.clone(),
                super::super::work::Handle {
                    kind: "DEPENDENCY".into(),
                    id,
                },
            );
            references.push(reference);
        }
        let selections = [super::super::work::Selection {
            references: references.clone(),
            ..Default::default()
        }];
        let mut pages = Vec::new();
        let mut parts = Vec::new();
        let mut page_checks = 0;
        let first = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |stage, _, _| {
                assert_eq!(stage, recorded_expansion::ProgressStage::Page);
                page_checks += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(first.selections.len(), 3);
        assert_eq!(page_checks, 10);
        assert_eq!(pages.len(), 10);
        let supplied: BTreeSet<_> = pages
            .iter()
            .flat_map(|page| page["items"].as_array().unwrap())
            .map(|item| item["reference"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(supplied, references.into_iter().collect());
        assert!(parts.is_empty());
        let state = super::super::work::read_state(&repo, &work.id).unwrap();
        let frozen_pages = pages.clone();
        let second = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(pages, frozen_pages);
        assert_eq!(first.selections, second.selections);
        assert_eq!(first.receipts, second.receipts);
        assert_eq!(
            digest(&state).unwrap(),
            digest(&super::super::work::read_state(&repo, &work.id).unwrap()).unwrap()
        );
    }

    #[test]
    fn recorded_retrieval_assembles_complete_source_parts_and_preserves_replayed_receipts() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Grouped source parts").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let text = "é".repeat(4000);
        let work = source_work(text.clone(), 2048);
        let selections = [super::super::work::Selection {
            references: vec!["source-ref".into()],
            ..Default::default()
        }];
        let mut pages = Vec::new();
        let mut parts = Vec::new();
        let mut part_checks = 0;
        let first = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |stage, _, _| {
                if stage == recorded_expansion::ProgressStage::SourcePart {
                    part_checks += 1;
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(part_checks > 1);
        assert_eq!(part_checks, parts.len());
        assert_eq!(
            parts
                .iter()
                .map(|part| part["text"].as_str().unwrap())
                .collect::<String>(),
            text
        );
        assert!(
            parts
                .iter()
                .all(|part| part["snapshot"] == work.snapshot.as_deref().unwrap())
        );
        let state = super::super::work::read_state(&repo, &work.id).unwrap();
        let frozen_pages = pages.clone();
        let frozen_parts = parts.clone();
        let second = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(pages, frozen_pages);
        assert_eq!(parts, frozen_parts);
        assert_eq!(first.receipts, second.receipts);
        assert_eq!(
            digest(&state).unwrap(),
            digest(&super::super::work::read_state(&repo, &work.id).unwrap()).unwrap()
        );
        assert_eq!(
            super::super::work_parts::delivered_source_references(&work, &state, &parts).unwrap(),
            BTreeSet::from(["source-ref".to_owned()])
        );
    }

    #[test]
    fn recorded_retrieval_resumes_a_durable_source_prefix_without_duplicates() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Interrupted parts").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let text = "é".repeat(4000);
        let work = source_work(text.clone(), 2048);
        let selections = [super::super::work::Selection {
            references: vec!["source-ref".into()],
            ..Default::default()
        }];
        let mut pages = Vec::new();
        let mut parts = Vec::new();
        let interrupted = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |stage, _, _| {
                if stage == recorded_expansion::ProgressStage::SourcePart {
                    return Err(invalid(
                        "fixture interruption after checkpointing the first part",
                    ));
                }
                Ok(())
            },
        );
        assert!(interrupted.is_err());
        assert_eq!(parts.len(), 1);
        assert!(parts[0]["nextCursor"].is_string());
        let first_part = parts[0].clone();
        let mut resumed_parts = 0;
        let completed = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |stage, _, _| {
                if stage == recorded_expansion::ProgressStage::SourcePart {
                    resumed_parts += 1;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(parts[0], first_part);
        assert_eq!(parts.len(), resumed_parts + 1);
        assert_eq!(
            parts
                .iter()
                .map(|part| part["text"].as_str().unwrap())
                .collect::<String>(),
            text
        );
        let state = super::super::work::read_state(&repo, &work.id).unwrap();
        assert_eq!(state.source_part_receipts.len(), parts.len());
        let frozen_parts = parts.clone();
        let replayed = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut pages,
            &mut parts,
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(parts, frozen_parts);
        assert_eq!(completed.receipts, replayed.receipts);
        assert_eq!(
            digest(&state).unwrap(),
            digest(&super::super::work::read_state(&repo, &work.id).unwrap()).unwrap()
        );
        // A receipt may reach disk before its array checkpoint. Reconstructing
        // packet text from that same deterministic plan must not duplicate receipts.
        let mut reconstructed_pages = Vec::new();
        let mut reconstructed_parts = Vec::new();
        let reconstructed = recorded_expansion::retrieve(
            &repo,
            &work,
            &selections,
            &mut reconstructed_pages,
            &mut reconstructed_parts,
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(reconstructed_parts, parts);
        assert_eq!(reconstructed_pages, pages);
        assert_eq!(reconstructed.receipts, completed.receipts);
        assert_eq!(
            digest(&state).unwrap(),
            digest(&super::super::work::read_state(&repo, &work.id).unwrap()).unwrap()
        );
        let mut forged_prefix = vec![first_part];
        forged_prefix[0]["text"] = json!("forged");
        assert!(
            recorded_expansion::retrieve(
                &repo,
                &work,
                &selections,
                &mut Vec::new(),
                &mut forged_prefix,
                |_, _, _| Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn recorded_retrieval_rejects_foreign_handles_and_untracked_reads() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Invalid retrieval").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let work = source_work("saved source".into(), 8192);
        for selection in [
            super::super::work::Selection {
                references: vec!["foreign-ref".into()],
                ..Default::default()
            },
            super::super::work::Selection {
                references: vec!["source-ref".into()],
                untracked_reads: true,
                ..Default::default()
            },
        ] {
            let mut pages = Vec::new();
            let mut parts = Vec::new();
            assert!(
                recorded_expansion::retrieve(
                    &repo,
                    &work,
                    &[selection],
                    &mut pages,
                    &mut parts,
                    |_, _, _| Ok(())
                )
                .is_err()
            );
            assert!(pages.is_empty());
            assert!(parts.is_empty());
            assert!(
                super::super::work::read_state(&repo, &work.id)
                    .unwrap()
                    .receipts
                    .is_empty()
            );
        }
    }

    fn overview_work() -> super::super::work::Work {
        let mut checked = super::super::check::assemble(
            "input".into(),
            BTreeMap::new(),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        checked.dependencies.insert(
            "process:dispatch".into(),
            super::super::model::Observation {
                id: "process:dispatch".into(),
                kind: "PROCESS_DEFINITION".into(),
                service: String::new(),
                symbol: "process:dispatch".into(),
                normalized: json!({}),
                digest: "digest".into(),
                source_ids: Vec::new(),
            },
        );
        checked.scenarios.insert(
            "dispatch".into(),
            super::super::check::ScenarioContext {
                id: "dispatch".into(),
                steps: Vec::new(),
                boundaries: Vec::new(),
                truncated: false,
                dependency_ids: vec!["process:dispatch".into()],
            },
        );
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-work/1.0", "id":"work", "subject":"scenario:dispatch",
            "request":{"schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers", "entrypoint":"process-overview"},
            "checked":checked, "snapshot":"snapshot", "retained":null, "externalInputs":{}, "handles":{}, "influence":{}, "obligations":[],
        })).unwrap()
    }

    fn source_work(text: String, max_bytes: usize) -> super::super::work::Work {
        let text_digest = crate::canonical::hash_bytes(text.as_bytes());
        let source = super::super::model::Source {
            id: "source-one".into(),
            service: "orders".into(),
            revision: "revision-a".into(),
            file: "src/orders.java".into(),
            start_line: 1,
            end_line: 1,
            text,
            text_digest: text_digest.clone(),
            evidence_digest: text_digest,
            authority: "CAPTURED_SOURCE".into(),
            library_origin: None,
            text_origin: None,
            occurrence: None,
            url: None,
        };
        let service = super::super::model::ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: "orders".into(),
            revision: "revision-a".into(),
            service_digest: "sha256:service".into(),
            extractor: "test".into(),
            runtime_mode: "TEST".into(),
            coverage: "COMPLETE".into(),
            boundaries: Vec::new(),
            entrypoints: Vec::new(),
            observations: std::collections::BTreeMap::new(),
            sources: std::collections::BTreeMap::from([(source.id.clone(), source.clone())]),
            contracts: std::collections::BTreeMap::new(),
        };
        let mut work = overview_work();
        work.id = "d".repeat(64);
        work.request.max_bytes = max_bytes;
        work.checked.services.insert("orders".into(), service);
        work.handles.insert(
            "source-ref".into(),
            super::super::work::Handle {
                kind: "SOURCE".into(),
                id: source.id,
            },
        );
        work
    }

    #[test]
    fn locked_checkpoint_selection_uses_held_guard_and_reloads_after_release() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Locked checkpoint").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let work = "a".repeat(64);
        let run = "b".repeat(32);
        let config_digest = digest(&json!({"config":"fixture"})).unwrap();
        let driver_digests = BTreeMap::from([
            ("author".into(), digest(&json!("author-driver")).unwrap()),
            (
                "reviewer".into(),
                digest(&json!("reviewer-driver")).unwrap(),
            ),
        ]);
        let mut report = RunReport {
            schema: "codeclew-documentation-work-run/1.0".into(),
            run: run.clone(),
            work: work.clone(),
            status: "PREPARED".into(),
            config_digest: Some(config_digest.clone()),
            attempts: Vec::new(),
            proposal: None,
            review: None,
            publication: None,
            gap: None,
            accounting: None,
            context_budget: None,
            execution_mode: None,
            draft: None,
            draft_repair: None,
            draft_review: None,
            draft_review_retry: None,
            checkpoint: None,
        };
        let checkpoint = RunCheckpoint::new(
            &report,
            format!("sha256:{}/1", "0".repeat(64)),
            config_digest.clone(),
            driver_digests.clone(),
            digest(&json!({"reads":[]})).unwrap(),
        );
        let guard = repo.lock().unwrap();
        save_run_checkpoint_locked(&repo, &guard, &mut report, &checkpoint).unwrap();
        let selected_reference = report.checkpoint.clone().unwrap();
        drop(guard);

        let selected_report = latest_report(&repo, &work).unwrap().unwrap();
        assert_eq!(selected_report.checkpoint, Some(selected_reference));
        let loaded = load_run_checkpoint(&repo, &selected_report, &config_digest, &driver_digests)
            .unwrap()
            .expect("selected immutable checkpoint reloads after guard release");
        assert_eq!(loaded.run, run);
        assert_eq!(loaded.work, work);
        assert_eq!(loaded.phase, "AUTHOR");
    }

    #[test]
    fn terminal_reservation_audit_preserves_released_slots_and_rejects_forged_usage() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Terminal accounting").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let maximum = json!({"inputTokens":100,"outputTokens":10,"costUnits":1});
        let role = json!({"adapter":"test-only","model":"fixture",
            "usageAuthority":"TRANSPORT_METADATA","command":[],"runtimeReads":[],
            "environment":[],"network":false,"cap":{"maximum":maximum,
                "overheadInputTokens":2,"timeoutMs":1000,"outputBytes":1024}});
        let config: Config = serde_json::from_value(json!({
            "schema":"codeclew-documentation-execution/1.0","author":role,"reviewer":role,
            "authorCalls":2,"reviewerCalls":0,"fallbackCalls":0,"repairAttempts":0,"expansions":0,
            "budget":{"account":"terminal-audit","costUnit":"fixture-unit",
                "ceiling":{"inputTokens":1000,"outputTokens":100,"costUnits":10},
                "stopLoss":{"inputTokens":900,"outputTokens":90,"costUnits":9}}
        }))
        .unwrap();
        let run = "a".repeat(32);
        let ids = reserve(&repo, &config, &run).unwrap();
        let used = &ids[0];
        dispatch_reserved(&repo, &config.budget, used, &run, "author").unwrap();
        let usage = Usage {
            input_tokens: Some(50),
            output_tokens: Some(5),
            cost_units: Some(1),
        };
        reconcile(&repo, &config.budget, used, Some(usage.clone()), 2).unwrap();
        release_unused(&repo, &config.budget, &run).unwrap();
        let report: RunReport = serde_json::from_value(json!({
            "schema":"codeclew-documentation-work-run/1.0","run":run,"work":"b".repeat(64),"status":"DRAFT",
            "attempts":[{"invocation":"c".repeat(32),"model":"fixture","usageAuthority":"TRANSPORT_METADATA",
                "role":"author","inputDigest":"sha256:fixture","reservation":used,"status":"COMPLETED",
                "admission":{},"usage":usage,"capturedStdoutBytes":0,"capturedStderrBytes":0}]
        })).unwrap();
        let path = repo
            .path(&account_path(&config.budget.account).unwrap())
            .unwrap();
        let original = fs::read(&path).unwrap();
        validate_terminal_reservations(&repo, &config, &report).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(ensure_reserved(&repo, &config, &run).is_err());

        let mut cancelled = report.clone();
        cancelled.status = "DRAFT_CANCELLED".into();
        let mut prepared = cancelled.attempts[0].clone();
        prepared.invocation = "d".repeat(32);
        prepared.reservation = ids[1].clone();
        prepared.status = "PREPARED".into();
        prepared.usage = None;
        prepared.result_digest = None;
        cancelled.attempts.push(prepared);
        validate_terminal_reservations(&repo, &config, &cancelled).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        cancelled.attempts[1].status = "COMPLETED".into();
        assert!(validate_terminal_reservations(&repo, &config, &cancelled).is_err());

        let mut interrupted = account(&repo, &config.budget).unwrap();
        let unused = interrupted.reservations.get_mut(&ids[1]).unwrap();
        unused.status = "RESERVED".into();
        unused.charged = unused.maximum.clone();
        save_account(&repo, &config.budget, &interrupted).unwrap();
        let mut recovered = report.clone();
        complete_terminal_accounting(&repo, &config, &mut recovered).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(recovered.checkpoint, report.checkpoint);
        assert!(recovered.accounting.is_some());
        let report_path = repo
            .path(&super::report_path(&report.run).unwrap())
            .unwrap();
        let report_bytes = fs::read(&report_path).unwrap();
        complete_terminal_accounting(&repo, &config, &mut recovered).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&report_path).unwrap(), report_bytes);

        let mut forged = account(&repo, &config.budget).unwrap();
        let reservation = forged.reservations.get_mut(used).unwrap();
        reservation.actual.as_mut().unwrap().input_tokens = Some(1);
        reservation.charged.input_tokens = 3;
        save_account(&repo, &config.budget, &forged).unwrap();
        assert!(validate_terminal_reservations(&repo, &config, &report).is_err());
        repo.atomic(&account_path(&config.budget.account).unwrap(), &original)
            .unwrap();
        let mut forged = account(&repo, &config.budget).unwrap();
        forged
            .reservations
            .get_mut(&ids[1])
            .unwrap()
            .charged
            .cost_units = 1;
        save_account(&repo, &config.budget, &forged).unwrap();
        assert!(validate_terminal_reservations(&repo, &config, &report).is_err());
    }

    #[test]
    fn checkpointed_zero_attempt_run_keeps_its_original_reservations() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Run recovery").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let maximum = Amount {
            input_tokens: 100,
            output_tokens: 10,
            cost_units: 1,
        };
        let role = Role {
            model_representation: None,
            adapter: "macos-seatbelt-stdio/1.0".into(),
            model: "resume-fixture".into(),
            usage_authority: "MAXIMUM_ONLY".into(),
            command: vec!["/usr/bin/true".into()],
            runtime_reads: Vec::new(),
            environment: Vec::new(),
            network: false,
            cap: Cap {
                maximum: maximum.clone(),
                overhead_input_tokens: 0,
                timeout_ms: 1_000,
                output_bytes: 1_024,
            },
        };
        let config = Config {
            schema: "codeclew-documentation-execution/1.0".into(),
            author: role.clone(),
            reviewer: role,
            author_output_contract: None,
            fallback: None,
            author_calls: 2,
            reviewer_calls: 2,
            fallback_calls: 0,
            repair_attempts: 1,
            expansions: 0,
            budget: Budget {
                account: "resume-fixture".into(),
                cost_unit: "fixture-unit".into(),
                ceiling: Amount {
                    input_tokens: 2_000,
                    output_tokens: 200,
                    cost_units: 20,
                },
                stop_loss: Amount {
                    input_tokens: 1_000,
                    output_tokens: 100,
                    cost_units: 10,
                },
            },
        };
        let run = "c".repeat(32);
        let mut reserved = reserve(&repo, &config, &run).unwrap();
        reserved.sort();
        let report = RunReport {
            schema: "codeclew-documentation-work-run/1.0".into(),
            run: run.clone(),
            work: "a".repeat(64),
            status: "PREPARED".into(),
            config_digest: Some(digest(&config).unwrap()),
            attempts: Vec::new(),
            proposal: None,
            review: None,
            publication: None,
            gap: None,
            accounting: None,
            context_budget: None,
            execution_mode: None,
            draft: None,
            draft_repair: None,
            draft_review: None,
            draft_review_retry: None,
            checkpoint: Some(recovery::CheckpointRef {
                schema: "codeclew-documentation-recovery-checkpoint-ref/1.0".into(),
                run: run.clone(),
                sequence: 1,
                checkpoint_digest: format!("sha256:{}", "0".repeat(64)),
            }),
        };
        assert!(report.checkpoint.is_some());
        assert!(report.attempts.is_empty());
        let before = account(&repo, &config.budget).unwrap();
        assert_eq!(
            before
                .reservations
                .iter()
                .filter(|(_, value)| value.run == run && value.status == "RESERVED")
                .count(),
            reserved.len()
        );

        ensure_reserved(&repo, &config, &run).unwrap();

        let after = account(&repo, &config.budget).unwrap();
        let after_ids: Vec<_> = after
            .reservations
            .iter()
            .filter(|(_, value)| value.run == run)
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(after_ids, reserved);
        for id in &reserved {
            assert_eq!(
                serde_json::to_value(&before.reservations[id]).unwrap(),
                serde_json::to_value(&after.reservations[id]).unwrap()
            );
        }
    }

    #[test]
    fn exact_job_boundary_counts_utf8_envelope_and_placeholder_matches_real_nonce() {
        let report = RunReport {
            schema: "codeclew-documentation-work-run/1.0".into(),
            run: "test-run".into(),
            work: "a".repeat(64),
            status: "PREPARED".into(),
            config_digest: None,
            attempts: Vec::new(),
            proposal: None,
            review: None,
            publication: None,
            gap: None,
            accounting: None,
            context_budget: None,
            execution_mode: None,
            draft: None,
            draft_repair: None,
            draft_review: None,
            draft_review_retry: None,
            checkpoint: None,
        };
        let mut driver = Role {
            model_representation: None,
            adapter: "test-only".into(),
            model: "fixture".into(),
            usage_authority: "MAXIMUM_ONLY".into(),
            command: Vec::new(),
            runtime_reads: Vec::new(),
            environment: Vec::new(),
            network: false,
            cap: Cap {
                maximum: Amount {
                    input_tokens: 10000,
                    output_tokens: 100,
                    cost_units: 1,
                },
                overhead_input_tokens: 7,
                timeout_ms: 1000,
                output_bytes: 1024,
            },
        };
        let payload = json!({"text":"Unicode: \u{1f9f5}; quote: \"; line:\n", "pages":[{"id":"first"},{"id":"second"}]});
        // The cap is itself in the envelope; converge its decimal width before
        // testing equality at the actual serialized boundary.
        for _ in 0..4 {
            let request = job_envelope(
                &report,
                "author",
                &driver,
                &"0".repeat(32),
                payload.clone(),
                Some(3),
            );
            driver.cap.maximum.input_tokens =
                bytes(&request).unwrap().len() as u64 + driver.cap.overhead_input_tokens;
        }
        let placeholder = job_envelope(
            &report,
            "author",
            &driver,
            &"0".repeat(32),
            payload.clone(),
            Some(3),
        );
        let actual = job_envelope(
            &report,
            "author",
            &driver,
            &uuid::Uuid::new_v4().simple().to_string(),
            payload,
            Some(3),
        );
        assert_eq!(
            bytes(&placeholder).unwrap().len(),
            bytes(&actual).unwrap().len()
        );
        assert_eq!(
            driver.cap.maximum.input_tokens,
            bytes(&actual).unwrap().len() as u64 + 7
        );
        ensure_input_cap(&driver, &actual).unwrap();
        assert_eq!(actual["expansionBudget"]["remaining"], 3);
        assert_eq!(actual["expansionBudget"]["scope"], "SHARED_ACROSS_ROLES");
        assert!(
            actual["expansionBudget"]["meaning"]
                .as_str()
                .unwrap()
                .contains("navigation page or a full result set")
        );
        let public = job_envelope(
            &report,
            "author",
            &driver,
            &"0".repeat(32),
            json!({"instruction":"public or dry request"}),
            None,
        );
        assert!(public.get("expansionBudget").is_none());
        driver.cap.maximum.input_tokens -= 1;
        assert!(ensure_input_cap(&driver, &actual).is_err());
        driver.cap.overhead_input_tokens = u64::MAX;
        assert!(ensure_input_cap(&driver, &actual).is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn saved_author_result_replays_from_dispatched_reservation_without_second_call() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::store::Repository::init(temporary.path(), "Agent recovery").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let work = "a".repeat(64);
        let run = "b".repeat(32);
        let maximum = Amount {
            input_tokens: 20_000,
            output_tokens: 2_000,
            cost_units: 10,
        };
        let driver = Role {
            model_representation: None,
            adapter: "macos-seatbelt-stdio/1.0".into(),
            model: "recovery-fixture".into(),
            usage_authority: "TRANSPORT_METADATA".into(),
            command: vec!["/usr/bin/true".into()],
            runtime_reads: Vec::new(),
            environment: Vec::new(),
            network: false,
            cap: Cap {
                maximum: maximum.clone(),
                overhead_input_tokens: 3,
                timeout_ms: 1_000,
                output_bytes: 16_384,
            },
        };
        let config = Config {
            schema: "codeclew-documentation-execution/1.0".into(),
            author: driver.clone(),
            reviewer: driver.clone(),
            author_output_contract: None,
            fallback: None,
            author_calls: 1,
            reviewer_calls: 1,
            fallback_calls: 0,
            repair_attempts: 0,
            expansions: 0,
            budget: Budget {
                account: "recovery-fixture".into(),
                cost_unit: "fixture-unit".into(),
                ceiling: Amount {
                    input_tokens: 100_000,
                    output_tokens: 10_000,
                    cost_units: 100,
                },
                stop_loss: Amount {
                    input_tokens: 90_000,
                    output_tokens: 9_000,
                    cost_units: 90,
                },
            },
        };
        let config_digest = digest(&config).unwrap();
        let admission = super::super::agent_adapter::admit(&repo, &driver).unwrap();
        let driver_digest = admission["driverDigest"].as_str().unwrap().to_owned();
        let read_digest = digest(&super::super::work::read_state(&repo, &work).unwrap()).unwrap();
        let mut report = RunReport {
            schema: "codeclew-documentation-work-run/1.0".into(),
            run: run.clone(),
            work: work.clone(),
            status: "PREPARED".into(),
            config_digest: Some(config_digest.clone()),
            attempts: Vec::new(),
            proposal: None,
            review: None,
            publication: None,
            gap: None,
            accounting: None,
            context_budget: None,
            execution_mode: None,
            draft: None,
            draft_repair: None,
            draft_review: None,
            draft_review_retry: None,
            checkpoint: None,
        };
        let mut checkpoint = RunCheckpoint::new(
            &report,
            format!("sha256:{}/1", "0".repeat(64)),
            config_digest.clone(),
            BTreeMap::from([("author".into(), driver_digest.clone())]),
            read_digest,
        );
        checkpoint.pages = vec![json!({"items":[],"nextCursor":null})];
        let reservation_ids = reserve(&repo, &config, &run).unwrap();
        let reservation_id = reservation_ids.first().unwrap().clone();
        let invocation = uuid::Uuid::new_v4().simple().to_string();
        let request = job_envelope(
            &report,
            "author",
            &driver,
            &invocation,
            json!({"instruction":"fixture author request"}),
            Some(checkpoint.expansions_remaining),
        );
        let input = recovery::InputRecord::new(
            recovery::CallBinding {
                run: run.clone(),
                work: work.clone(),
                snapshot: checkpoint.snapshot.clone(),
                reservation: reservation_id.clone(),
                invocation: invocation.clone(),
                role: "author".into(),
                model: driver.model.clone(),
                usage_authority: driver.usage_authority.clone(),
                config_digest,
                driver_digest: driver_digest.clone(),
            },
            request.clone(),
        )
        .unwrap();
        recovery::save_input(&repo, &input).unwrap();
        dispatch_reserved(&repo, &config.budget, &reservation_id, &run, "author").unwrap();
        let request_bytes = bytes(&request).unwrap().len();
        report.attempts.push(Attempt {
            invocation: invocation.clone(),
            model: driver.model.clone(),
            usage_authority: driver.usage_authority.clone(),
            role: "author".into(),
            input_digest: input.identity.input_digest.clone(),
            request_bytes: Some(request_bytes),
            reservation: reservation_id.clone(),
            status: "DISPATCHED".into(),
            admission,
            failure: None,
            usage: None,
            result_digest: None,
            captured_stdout_bytes: 0,
            captured_stderr_bytes: 0,
            author_contract: None,
            adapted_proposal: None,
            uncertainty_adaptation: None,
            expansion_selection: None,
        });
        checkpoint.pending_call = Some(PendingCall {
            identity: input.identity.clone(),
            status: "DISPATCHED".into(),
            failure: None,
        });
        // Simulate interruption after the validated result is durable but
        // before the selected checkpoint/account reconciliation advances.
        let authored = json!({"action":"proposal","proposal":{"fixture":"saved"}});
        recovery::save_result(
            &repo,
            &input,
            Some(Usage {
                input_tokens: Some(10),
                output_tokens: Some(5),
                cost_units: Some(1),
            }),
            authored.clone(),
            128,
            0,
        )
        .unwrap();
        save_run_checkpoint(&repo, &mut report, &checkpoint).unwrap();

        let (replayed, replayed_invocation, replayed_driver) = call(
            &repo,
            &config,
            &mut report,
            &mut checkpoint,
            "author",
            &driver,
            request["payload"].clone(),
            None,
            true,
            true,
        )
        .unwrap();
        assert_eq!(replayed, authored);
        assert_eq!(replayed_invocation, invocation);
        assert_eq!(replayed_driver, driver_digest);
        assert_eq!(report.attempts.len(), 1);
        assert_eq!(report.attempts[0].status, "COMPLETED");
        let saved_reservation =
            reservation(&repo, &config.budget, &reservation_id, &run, "author").unwrap();
        assert_eq!(saved_reservation.status, "RECONCILED");
        assert_eq!(saved_reservation.charged.input_tokens, 13);
        assert_eq!(saved_reservation.charged.output_tokens, 5);
    }
}
