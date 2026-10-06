use clew::documentation::{
    agent_jobs::{self, Amount, Budget, Cap, Config, Role, Usage},
    store::Repository,
};
use std::{collections::BTreeSet, fs};

struct Fixture {
    _temporary: tempfile::TempDir,
    repo: Repository,
    budget: Budget,
    config: Config,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("docs");
        Repository::init(&root, "Account recovery regression").unwrap();
        let repo = Repository::open(&root).unwrap();
        let maximum = Amount {
            input_tokens: 100,
            output_tokens: 50,
            cost_units: 10,
        };
        let role = Role {
            adapter: "fixture-stdio/1.0".into(),
            model: "account-recovery-test".into(),
            usage_authority: "TRANSPORT_METADATA".into(),
            model_representation: None,
            command: Vec::new(),
            runtime_reads: Vec::new(),
            environment: Vec::new(),
            network: false,
            cap: Cap {
                maximum,
                overhead_input_tokens: 7,
                timeout_ms: 1_000,
                output_bytes: 1_024,
            },
        };
        let budget = Budget {
            account: "account-recovery-test".into(),
            cost_unit: "fixture-unit".into(),
            ceiling: Amount {
                input_tokens: 1_000,
                output_tokens: 500,
                cost_units: 100,
            },
            stop_loss: Amount {
                input_tokens: 900,
                output_tokens: 450,
                cost_units: 90,
            },
        };
        let config = Config {
            schema: "codeclew-documentation-execution/1.0".into(),
            author: role.clone(),
            reviewer: role,
            author_output_contract: None,
            fallback: None,
            author_calls: 1,
            reviewer_calls: 1,
            fallback_calls: 0,
            repair_attempts: 0,
            expansions: 0,
            budget: budget.clone(),
        };
        Self {
            _temporary: temporary,
            repo,
            budget,
            config,
        }
    }

    fn dispatch_author(&self) -> String {
        let run = "a".repeat(32);
        let reserved = agent_jobs::reserve(&self.repo, &self.config, &run).unwrap();
        assert_eq!(reserved.len(), 2, "author and reviewer slots are reserved");
        agent_jobs::dispatch(&self.repo, &self.budget, &run, "author").unwrap()
    }

    fn ledger_bytes(&self) -> Vec<u8> {
        fs::read(
            self.repo
                .path("execution/accounts/account-recovery-test.json")
                .unwrap(),
        )
        .unwrap()
    }
}

fn reconcile_twice_with_usage(usage: Usage, overhead: u64, expected: Amount, status: &str) {
    let fixture = Fixture::new();
    let reservation = fixture.dispatch_author();

    agent_jobs::reconcile(
        &fixture.repo,
        &fixture.budget,
        &reservation,
        Some(usage.clone()),
        overhead,
    )
    .unwrap();
    let once = agent_jobs::account(&fixture.repo, &fixture.budget).unwrap();
    let reconciled = &once.reservations[&reservation];
    assert_eq!(reconciled.charged, expected);
    assert_eq!(reconciled.actual, Some(usage.clone()));
    assert_eq!(reconciled.status, status);
    let after_first = fixture.ledger_bytes();

    agent_jobs::reconcile(
        &fixture.repo,
        &fixture.budget,
        &reservation,
        Some(usage),
        overhead,
    )
    .unwrap();

    let twice = agent_jobs::account(&fixture.repo, &fixture.budget).unwrap();
    assert_eq!(twice.reservations[&reservation].charged, expected);
    assert_eq!(twice.reservations[&reservation].status, status);
    assert_eq!(fixture.ledger_bytes(), after_first);
}

#[test]
fn reconciled_full_and_partial_usage_are_byte_idempotent() {
    reconcile_twice_with_usage(
        Usage {
            input_tokens: Some(13),
            output_tokens: Some(9),
            cost_units: Some(2),
        },
        7,
        Amount {
            input_tokens: 20,
            output_tokens: 9,
            cost_units: 2,
        },
        "RECONCILED",
    );
    reconcile_twice_with_usage(
        Usage {
            input_tokens: Some(13),
            output_tokens: None,
            cost_units: Some(2),
        },
        7,
        Amount {
            input_tokens: 20,
            output_tokens: 50,
            cost_units: 2,
        },
        "UNRECONCILED_MAXIMUM_RETAINED",
    );
}

#[test]
fn changed_usage_or_overhead_fails_without_mutating_the_ledger() {
    let fixture = Fixture::new();
    let reservation = fixture.dispatch_author();
    let usage = Usage {
        input_tokens: Some(13),
        output_tokens: Some(9),
        cost_units: Some(2),
    };
    agent_jobs::reconcile(
        &fixture.repo,
        &fixture.budget,
        &reservation,
        Some(usage.clone()),
        7,
    )
    .unwrap();
    let original = fixture.ledger_bytes();
    let original_account = agent_jobs::account(&fixture.repo, &fixture.budget).unwrap();

    for (different_usage, different_overhead) in [
        (
            Usage {
                input_tokens: Some(14),
                ..usage.clone()
            },
            7,
        ),
        (usage, 8),
    ] {
        let error = agent_jobs::reconcile(
            &fixture.repo,
            &fixture.budget,
            &reservation,
            Some(different_usage),
            different_overhead,
        )
        .unwrap_err();
        assert!(
            error.message.contains("RECOVERY_ACCOUNTING_MISMATCH"),
            "{error}"
        );
        assert_eq!(fixture.ledger_bytes(), original);
        let account = agent_jobs::account(&fixture.repo, &fixture.budget).unwrap();
        assert_eq!(
            serde_json::to_value(&account.reservations[&reservation]).unwrap(),
            serde_json::to_value(&original_account.reservations[&reservation]).unwrap()
        );
    }
}

#[test]
fn missing_usage_retains_the_exact_maximum_and_reconcile_creates_no_slots() {
    let fixture = Fixture::new();
    let run = "a".repeat(32);
    let reserved = agent_jobs::reserve(&fixture.repo, &fixture.config, &run).unwrap();
    let expected_ids: BTreeSet<String> = reserved.into_iter().collect();
    assert_eq!(expected_ids.len(), 2);
    let reservation = agent_jobs::dispatch(&fixture.repo, &fixture.budget, &run, "author").unwrap();

    agent_jobs::reconcile(&fixture.repo, &fixture.budget, &reservation, None, 37).unwrap();
    let once = agent_jobs::account(&fixture.repo, &fixture.budget).unwrap();
    let reconciled = &once.reservations[&reservation];
    assert_eq!(
        reconciled.maximum,
        Amount {
            input_tokens: 100,
            output_tokens: 50,
            cost_units: 10,
        }
    );
    assert_eq!(reconciled.charged, reconciled.maximum);
    assert_eq!(reconciled.actual, None);
    assert_eq!(reconciled.status, "UNRECONCILED_MAXIMUM_RETAINED");
    assert_eq!(
        once.reservations.keys().cloned().collect::<BTreeSet<_>>(),
        expected_ids
    );
    let after_first = fixture.ledger_bytes();

    agent_jobs::reconcile(&fixture.repo, &fixture.budget, &reservation, None, 37).unwrap();
    let twice = agent_jobs::account(&fixture.repo, &fixture.budget).unwrap();
    assert_eq!(
        twice.reservations.keys().cloned().collect::<BTreeSet<_>>(),
        expected_ids
    );
    assert_eq!(twice.reservations[&reservation].charged, reconciled.maximum);
    assert_eq!(twice.reservations[&reservation].actual, None);
    assert_eq!(
        twice.reservations[&reservation].status,
        "UNRECONCILED_MAXIMUM_RETAINED"
    );
    assert_eq!(fixture.ledger_bytes(), after_first);
}
