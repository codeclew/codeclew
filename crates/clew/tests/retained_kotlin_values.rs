//! Public source-launcher qualification; no caller-supplied fact JSON.
use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn run(launcher: &Path, args: &[&str]) -> Value {
    let output = Command::new(launcher).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore = "requires native source launcher, JDK21, cached Kotlin 2.4.10 and Gradle"]
fn source_launcher_retains_k2_values_with_original_unicode_crlf_and_historical_bytes() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let launcher = source_root.join("clew");
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repo");
    fs::create_dir(&repo).unwrap();
    for file in [
        "gradlew",
        "gradlew.bat",
        "gradle/wrapper/gradle-wrapper.jar",
        "gradle/wrapper/gradle-wrapper.properties",
        "build.gradle.kts",
        "settings.gradle.kts",
    ] {
        let destination = repo.join(file);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(
            source_root.join("fixtures/kotlin-basic").join(file),
            destination,
        )
        .unwrap();
    }
    let file = "src/main/kotlin/values/Values.kt";
    let original = format!(
        "\u{feff}{}",
        include_str!("../../../fixtures/kotlin-values/Values.kt").replace('\n', "\r\n")
    );
    fs::create_dir_all(repo.join("src/main/kotlin/values")).unwrap();
    fs::write(repo.join(file), &original).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let opened = run(
        &launcher,
        &[
            "context",
            "open",
            "--repo",
            repo.to_str().unwrap(),
            "--target-ref",
            "main",
            "--language",
            "kotlin",
            "--compilation",
            ":/main",
            "--profile",
            "kotlin-2.4.10-gradle-single",
            "--operation",
            "analysis",
            "--intent",
            "Qualify retained value bindings",
            "--term",
            "Values",
        ],
    );
    let session = opened["session"]["sessionId"]
        .as_str()
        .expect("admitted native session");
    let query = |owner: &str, target: &str, max_nodes: &str| {
        run(
            &launcher,
            &[
                "session",
                "values",
                "--session",
                session,
                "--compilation",
                ":/main",
                "--owner",
                owner,
                "--target",
                target,
                "--max-nodes",
                max_nodes,
            ],
        )
    };
    let owner = "callable:values/Values.choose#jvm:(Ljava/lang/String;)Ljava/lang/String;";
    let packet = query(owner, "result", "128");
    assert_eq!(packet["status"], "AVAILABLE", "{packet}");
    assert_eq!(
        packet["observationAuthority"],
        "AUTHENTICATED_RETAINED_COMPILER_PROJECTION"
    );
    assert_eq!(
        packet["result"]["calculationAuthority"],
        "DECLARED_MODEL_EQUATIONS"
    );
    let root = |p: &Value| {
        assert_eq!(p["status"], "AVAILABLE", "{p}");
        p["result"]["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "result")
            .unwrap()["normalValueNullness"]
            .clone()
    };
    assert_eq!(root(&packet), "NON_NULL");
    assert_eq!(packet["programSemanticClosure"], false);
    assert_eq!(packet["sources"][0]["file"], file);
    assert_eq!(
        packet["sources"][0]["contentDigest"],
        clew::canonical::hash_bytes(original.as_bytes())
    );
    for occurrence in packet["occurrences"].as_array().unwrap() {
        assert_eq!(occurrence["owner"], owner);
        let start = occurrence["byteStart"].as_u64().unwrap() as usize;
        let end = occurrence["byteEnd"].as_u64().unwrap() as usize;
        assert!(!original.get(start..end).unwrap().is_empty());
    }
    assert_eq!(query(owner, "result", "128"), packet);
    assert_eq!(
        root(&query(
            "callable:values/Values.nullValue#jvm:()Ljava/lang/String;",
            "result",
            "128"
        )),
        "NULL"
    );
    assert_eq!(
        root(&query(
            "callable:values/Values.choose#jvm:(Ljava/lang/Integer;)I",
            "result",
            "128"
        )),
        "NON_NULL"
    );
    assert_eq!(
        root(&query(
            "callable:values/Values.defaulted#jvm:(Ljava/lang/String;)Ljava/lang/String;",
            "result",
            "128"
        )),
        "UNKNOWN"
    );
    assert_eq!(
        root(&query(
            "callable:values/Values.mutated#jvm:(Ljava/lang/String;)Ljava/lang/String;",
            "result",
            "128"
        )),
        "UNKNOWN"
    );
    assert_eq!(
        root(&query(
            "callable:values/Values.field#jvm:()Ljava/lang/String;",
            "result",
            "128"
        )),
        "UNKNOWN"
    );
    let bounded = query(owner, "result", "1");
    assert_eq!(root(&bounded), "UNKNOWN");
    assert!(
        bounded["result"]["frontier"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "BUDGET")
    );
    assert_eq!(
        query(owner, "missing-definition", "128")["status"],
        "UNKNOWN"
    );
    assert_eq!(
        query("callable:values/Values.choose#jvm:()V", "result", "128")["status"],
        "UNKNOWN"
    );
    fs::write(repo.join(file), "broken live checkout").unwrap();
    assert_eq!(query(owner, "result", "128"), packet);
    fs::write(repo.join(file), &original).unwrap();
    if let Some(directory) = std::env::var_os("CODECLEW_TEST_KOTLIN_QUERY_EXAMPLE_DIRECTORY") {
        let directory = Path::new(&directory);
        fs::create_dir_all(directory).unwrap();
        fs::write(
            directory.join("query.json"),
            serde_json::to_string_pretty(&packet).unwrap() + "\n",
        )
        .unwrap();
        fs::write(directory.join("Values.kt"), &original).unwrap();
        fs::write(
            directory.join("open.json"),
            serde_json::to_string_pretty(&opened).unwrap() + "\n",
        )
        .unwrap();
    }
    run(&launcher, &["session", "close", "--session", session]);
}
