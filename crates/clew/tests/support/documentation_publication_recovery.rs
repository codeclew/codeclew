#![cfg(target_os = "macos")]

//! Public recovery regressions for damaged receipt-backed publications.
use super::*;
use std::{collections::BTreeMap, path::Path};

fn tree_bytes(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            let path = entry.path();
            if kind.is_dir() {
                visit(root, &path, files);
            } else {
                assert!(
                    kind.is_file(),
                    "fixture tree contains an unexpected special file"
                );
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                files.insert(relative, fs::read(path).unwrap());
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(directory, directory, &mut files);
    files
}

#[cfg(target_os = "macos")]
#[test]
fn accepted_recovery_refuses_tampered_bundle_without_mutating_run_or_outputs() {
    use serde_json::json;

    for (case, damaged_relative) in [
        ("generated service HTML", "services/orders.html"),
        ("compact bindings", "bindings.json"),
        ("publication manifest", "publication.json"),
    ] {
        let f = Fixture::new();
        f.service("orders");
        let (work, _, _) = proposal_fixture(&f);
        let config = execution_config(&f, json!({}), json!({}), None);
        let config_path = f.input("publication-recovery-config.json", &config);
        let accepted = work_run(&f, &work, &config);
        assert_eq!(accepted["status"], "ACCEPTED", "{case}: {accepted}");
        let report = run_report(&f, &accepted);
        let run = accepted["run"].as_str().unwrap();
        let bundle = accepted["publication"]["bundle"].as_str().unwrap();
        let selected_reference = report["checkpoint"].clone();
        let selected_checkpoint = selected_checkpoint_path(&f, run, &selected_reference);
        let checkpoint = read(&selected_checkpoint);
        assert_eq!(checkpoint["checkpoint"]["phase"], "TERMINAL", "{case}");
        assert_eq!(
            checkpoint["checkpoint"]["publicationReceipt"]["bundleId"], bundle,
            "{case}"
        );

        let damaged_path = f.bundle(bundle, damaged_relative);
        let mut damaged_bytes = fs::read(&damaged_path).unwrap();
        damaged_bytes.extend_from_slice(b"\n tampered after acceptance");
        fs::write(&damaged_path, &damaged_bytes).unwrap();

        let report_path = f.docs.join(format!(".codeclew/jobs/{run}.json"));
        let latest_path = f
            .docs
            .join(format!(".codeclew/work/{work}/latest-run.json"));
        let account_path = f.docs.join("execution/accounts/fixture.json");
        let history_path = f.docs.join("docs/history.html");
        let pointer_path = f.docs.join("docs/index.html");
        let result_directory = f.docs.join(".codeclew/job-results");
        let bundle_directory = f.docs.join(format!("docs/generated/{bundle}"));
        let stable_report = fs::read(&report_path).unwrap();
        let stable_latest = fs::read(&latest_path).unwrap();
        let stable_account = fs::read(&account_path).unwrap();
        let stable_history = fs::read(&history_path).unwrap();
        let stable_pointer = fs::read(&pointer_path).unwrap();
        let stable_checkpoint = fs::read(&selected_checkpoint).unwrap();
        let stable_results = tree_bytes(&result_directory);
        let stable_bundle = tree_bytes(&bundle_directory);
        let stable_bundles = directory_entries(&f.docs.join("docs/generated"));

        for retry in 0..2 {
            let (code, refusal) = f.run(&[
                "docs",
                "work",
                "run",
                "--work",
                &work,
                "--config",
                config_path.to_str().unwrap(),
            ]);
            assert_ne!(
                code, 0,
                "{case} retry {retry} unexpectedly succeeded: {refusal}"
            );
            assert!(
                refusal.to_string().contains("RECOVERY_"),
                "{case} retry {retry} did not refuse as recovery damage: {refusal}"
            );
            assert_eq!(fs::read(&report_path).unwrap(), stable_report, "{case}");
            assert_eq!(fs::read(&latest_path).unwrap(), stable_latest, "{case}");
            assert_eq!(fs::read(&account_path).unwrap(), stable_account, "{case}");
            assert_eq!(fs::read(&history_path).unwrap(), stable_history, "{case}");
            assert_eq!(fs::read(&pointer_path).unwrap(), stable_pointer, "{case}");
            assert_eq!(
                fs::read(&selected_checkpoint).unwrap(),
                stable_checkpoint,
                "{case}"
            );
            assert_eq!(tree_bytes(&result_directory), stable_results, "{case}");
            assert_eq!(tree_bytes(&bundle_directory), stable_bundle, "{case}");
            assert_eq!(
                directory_entries(&f.docs.join("docs/generated")),
                stable_bundles,
                "{case}"
            );
        }

        assert_eq!(fs::read(damaged_path).unwrap(), damaged_bytes, "{case}");
        let final_report = run_report(&f, &accepted);
        assert_eq!(final_report["attempts"], report["attempts"], "{case}");
        assert_eq!(final_report["checkpoint"], selected_reference, "{case}");
    }
}
