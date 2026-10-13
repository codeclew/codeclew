//! Explicit maintainer replay of a collected public weather documentation cache.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use support::Fixture;

fn copy_files(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink());
        let output = target.join(entry.file_name());
        if kind.is_dir() {
            copy_files(&entry.path(), &output);
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), output).unwrap();
        }
    }
}

#[test]
#[ignore = "requires explicit collected public-weather cache and declarative inputs"]
fn actual_saved_weather_definitions_render_contracts_and_receiving_fragments() {
    let cache = std::env::var("CLEW_WEATHER_COLLECTED_CACHE").unwrap();
    let original = std::env::var("CLEW_WEATHER_DOCUMENTATION_ROOT").unwrap();
    let candidates = std::env::var("CLEW_WEATHER_THREAD_CANDIDATES").unwrap();
    let snapshot = std::env::var("CLEW_WEATHER_SOURCE_SNAPSHOT").unwrap();
    let f = Fixture::new();
    copy_files(Path::new(&cache), &f.docs);
    // Supplement the cache with portable declarations and exact native output;
    // neither a source checkout nor Git administration is copied.
    for relative in ["catalog", "scenarios", "notes", "docs"] {
        copy_files(&Path::new(&original).join(relative), &f.docs.join(relative));
    }
    for id in ["dashboard-save-city", "dashboard-refresh-request"] {
        let input = Path::new(&candidates).join(format!("{id}-thread-metadata-candidate-v2.json"));
        let digest = f.ok(&["docs", "service", "list"])["inputDigest"]
            .as_str()
            .unwrap()
            .to_owned();
        f.ok(&[
            "docs",
            "process",
            "put",
            "--input",
            input.to_str().unwrap(),
            "--expected-input-digest",
            &digest,
        ]);
    }
    let (code, composed) = f.run(&["docs", "recompose", "--snapshot", &snapshot]);
    assert!(matches!(code, 0 | 3), "{composed}");
    assert_eq!(composed["status"], "CHECKED");
    let marker = f.temp.path().join("native-render-git.marker");
    fs::remove_file(f.tools_dir().join("git")).unwrap();
    fs::write(
        f.tools_dir().join("git"),
        format!(
            "#!/bin/sh\necho attempted >> {}\nexit 99\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(f.tools_dir().join("git"), fs::Permissions::from_mode(0o700)).unwrap();
    let (code, rendered) = f.run(&[
        "docs",
        "render",
        "--snapshot",
        composed["snapshot"].as_str().unwrap(),
        "--language",
        "en",
    ]);
    assert!(matches!(code, 0 | 3), "{rendered}");
    assert!(rendered["bundle"].is_string(), "{rendered}");
    assert!(!marker.exists(), "render acquired fresh source");
    let bundle = f
        .docs
        .join("docs/generated")
        .join(rendered["bundle"].as_str().unwrap());
    for id in ["dashboard-save-city", "dashboard-refresh-request"] {
        let page: Value =
            serde_json::from_slice(&fs::read(bundle.join(format!("scenarios/{id}.json"))).unwrap())
                .unwrap();
        let thread = &page["processThread"];
        assert_eq!(thread["status"], "SOURCE_BOUND_DECLARATION", "{id}");
        assert_eq!(thread["methods"].as_object().unwrap().len(), 2);
        assert_eq!(thread["boundaryContracts"].as_array().unwrap().len(), 1);
        assert_eq!(thread["serviceFragments"].as_array().unwrap().len(), 1);
        assert_eq!(thread["declaredBranchCoverage"], "NOT_PROVEN");
        assert_eq!(thread["selectedEffectCompleteness"], "NOT_PROVEN");
        assert!(
            fs::read_to_string(bundle.join(format!("scenarios/{id}.html")))
                .unwrap()
                .contains("SOURCE_BOUND_DECLARATION")
        );
    }
    let places: Value =
        serde_json::from_slice(&fs::read(bundle.join("services/places.json")).unwrap()).unwrap();
    assert_eq!(places["serviceFragments"].as_array().unwrap().len(), 2);
    if let Ok(output) = std::env::var("CLEW_WEATHER_REPLAY_OUTPUT") {
        let output = Path::new(&output);
        assert!(!output.exists(), "replay output must be a new directory");
        copy_files(&bundle, output);
        fs::write(
            output.join("native-replay-result.json"),
            serde_json::to_vec_pretty(&rendered).unwrap(),
        )
        .unwrap();
    }
    println!(
        "Native development replay passed: two real saved threads, four exact methods, two receiving fragments; installed-release acceptance remains separate."
    );
}
