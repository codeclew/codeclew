#![cfg(unix)]
use clew::documentation::{model::Service, store::Repository};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn put(root: &Path, kind: &str, digest: &str, input: &[u8]) -> (bool, Value) {
    put_path(root, kind, digest, "-", input)
}
fn put_path(root: &Path, kind: &str, digest: &str, path: &str, input: &[u8]) -> (bool, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "docs",
            kind,
            "put",
            "--root",
            root.to_str().unwrap(),
            "--expected-input-digest",
            digest,
        ])
        .arg(format!("--input={path}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(input);
    let result = child.wait_with_output().unwrap();
    (
        result.status.success(),
        serde_json::from_slice(&result.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&result.stderr))),
    )
}

#[test]
fn authored_put_stdin_preserves_schemas_limits_and_cas() {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Owned input fixture").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    let service:Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"owned","repository":"https://example.invalid/orders.git","language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["src"],"dialect":"17"}})).unwrap();
    repo.service_add(service, Some(&repo.input_digest().unwrap()))
        .unwrap();
    let initial = repo.input_digest().unwrap();
    let external = json!({"schema":"codeclew-documentation-external-service/1.0","id":"remote","title":"Remote","description":"Owned declared destination","declaration":{"origin":"agent-proposal","rationale":"Owned fixture"}});
    let mut exact = serde_json::to_vec(&external).unwrap();
    exact.resize(64 * 1024, b' ');
    let (ok, saved) = put(temp.path(), "external", &initial, &exact);
    assert!(ok, "{saved}");
    let digest = saved["inputDigest"].as_str().unwrap();
    let (ok, current) = put(temp.path(), "external", &initial, &exact);
    assert!(ok, "{current}");
    assert_eq!(current["status"], "CURRENT");
    assert_eq!(current["inputDigest"], digest);
    for data in [b"".as_slice(), b"{broken", b"unknown: value"] {
        let (ok, error) = put(temp.path(), "external", digest, data);
        assert!(!ok, "{error}");
        assert_eq!(repo.input_digest().unwrap(), digest);
    }
    exact.push(b' ');
    let (ok, error) = put(temp.path(), "external", digest, &exact);
    assert!(!ok, "{error}");
    assert_eq!(repo.input_digest().unwrap(), digest);
    let mut other = external.clone();
    other["id"] = json!("not-stdin");
    let (ok, error) = put_path(
        temp.path(),
        "external",
        digest,
        "-/",
        &serde_json::to_vec(&other).unwrap(),
    );
    assert!(!ok, "only the literal dash selects stdin: {error}");
    assert_eq!(repo.input_digest().unwrap(), digest);
    let mut changed = external.clone();
    changed["description"] = json!("Changed intent");
    let (ok, error) = put(
        temp.path(),
        "external",
        &initial,
        &serde_json::to_vec(&changed).unwrap(),
    );
    assert!(!ok);
    assert_eq!(error["error"]["code"], "WW_CONFLICT");
    assert_eq!(error["error"]["details"]["currentInputDigest"], digest);
    let interaction = json!({"schema":"codeclew-documentation-interaction/1.0","id":"request","title":"Request","from":{"service":"orders"},"to":{"service":"remote"},"external":true,"transport":{"kind":"http","method":"GET","path":"/items"},"declaration":{"origin":"agent-proposal","rationale":"Owned fixture"}});
    let (ok, saved) = put(
        temp.path(),
        "interaction",
        digest,
        &serde_json::to_vec(&interaction).unwrap(),
    );
    assert!(ok, "{saved}");
    let digest = saved["inputDigest"].as_str().unwrap();
    let yaml=b"schema: codeclew-documentation-process/1.0\nid: checkout\ntitle: Checkout\nsummary: Owned process\nroot: {service: orders}\nprocess:\n  scope: Owned scope\n  participants: [orders, remote]\n  trigger: Request\n  outcomes: [Complete]\n";
    let (ok, saved) = put(temp.path(), "process", digest, yaml);
    assert!(ok, "{saved}");
    assert_eq!(repo.scenarios().unwrap().len(), 1);
    let digest = saved["inputDigest"].as_str().unwrap();
    let missing_scope = String::from_utf8_lossy(yaml).replace("  scope: Owned scope\n", "");
    let (ok, error) = put(temp.path(), "process", digest, missing_scope.as_bytes());
    assert!(!ok);
    assert_eq!(
        error["error"]["details"]["reason"],
        "MISSING_REQUIRED_FIELD"
    );
    assert_eq!(error["error"]["details"]["field"], "process.scope");
    assert_eq!(error["error"]["details"]["recordId"], "checkout");
    assert_eq!(repo.input_digest().unwrap(), digest);
    for kind in ["external", "interaction", "process"] {
        let help = Command::new(env!("CARGO_BIN_EXE_clew"))
            .args(["docs", kind, "put", "--help"])
            .output()
            .unwrap();
        assert!(help.status.success());
        assert!(String::from_utf8_lossy(&help.stdout).contains("--input -"));
    }
}
