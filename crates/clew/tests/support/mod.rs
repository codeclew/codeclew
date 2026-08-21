use clew::worker::workspace_root;
use std::path::Path;
use std::process::Command;

fn merge_tree(source: &Path, destination: &Path) {
    if !source.is_dir() {
        return;
    }
    for entry in walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .map(Result::unwrap)
    {
        let relative = entry.path().strip_prefix(source).unwrap();
        if relative.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("daemon" | ".tmp" | "notifications")
            )
        }) {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else if entry.file_type().is_file() && !target.exists() {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            // Gradle and Maven caches are mutable stores. Hard-linking their
            // files across concurrently running fixtures lets one build
            // truncate or replace another fixture's artifacts. Keep each
            // fixture isolated even though the cold copy is more expensive.
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

pub fn seed_build_caches(repo: &Path) {
    let workspace = workspace_root();
    if repo.join("gradlew").is_file() {
        merge_tree(
            &workspace.join("fixtures/kotlin-basic/.gradle"),
            &repo.join(".gradle"),
        );
    }
    if repo.join("mvnw").is_file() {
        merge_tree(
            &workspace.join("fixtures/kotlin-maven/.semantic-thread/maven-repository"),
            &repo.join(".semantic-thread/maven-repository"),
        );
    }
    prepare_project_native_artifacts(repo);
}

fn prepare_project_native_artifacts(repo: &Path) {
    if repo.join("gradlew").is_file() {
        if !repo.join("build/classes/kotlin/main").is_dir() {
            let output = Command::new("./gradlew")
                .args(["compileTestKotlin", "--no-daemon", "--quiet"])
                .current_dir(repo)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "project-native Gradle preflight failed for {}: {}",
                repo.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for relative in [
            "build/classes/java/main",
            "build/classes/java/test",
            "build/resources/main",
            "build/resources/test",
        ] {
            std::fs::create_dir_all(repo.join(relative)).unwrap();
        }
    }
    if repo.join("mvnw").is_file() && !repo.join("target/classes").is_dir() {
        let output = Command::new("./mvnw")
            .args(["-q", "-DskipTests", "compile"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "project-native Maven preflight failed for {}: {}",
            repo.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
