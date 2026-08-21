use clew::worker::workspace_root;
use std::path::Path;

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
            if std::fs::hard_link(entry.path(), &target).is_err() {
                std::fs::copy(entry.path(), &target).unwrap();
            }
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
}
