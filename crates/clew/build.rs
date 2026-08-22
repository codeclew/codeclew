use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn main() {
    generate_worker_input_manifests();
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe { std::env::set_var("PROTOC", protoc) };
    let mut config = prost_build::Config::new();
    config.enum_attribute(".", "#[allow(clippy::large_enum_variant)]");
    config
        .compile_protos(&["../../schemas/worker.proto"], &["../../schemas"])
        .expect("compile worker protocol");
}

fn generate_worker_input_manifests() {
    let crate_root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repo = crate_root.join("../..").canonicalize().unwrap();
    let common_roots = ["workers/kotlin/src/main"];
    let common_files = [
        "build.gradle.kts",
        "settings.gradle.kts",
        "gradlew",
        "gradle/wrapper/gradle-wrapper.jar",
        "gradle/wrapper/gradle-wrapper.properties",
        "schemas/worker.proto",
    ];
    let variants = [
        (
            "KOTLIN21",
            "kotlin21",
            ":workers:kotlin21:installDist",
            "workers/manifests/kotlin21.json",
            vec![common_roots[0], "workers/kotlin21/src/main"],
            common_files
                .iter()
                .copied()
                .chain(["workers/kotlin21/build.gradle.kts"])
                .collect::<Vec<_>>(),
        ),
        (
            "KOTLIN23",
            "kotlin23",
            ":workers:kotlin23:installDist",
            "workers/manifests/kotlin23.json",
            vec![common_roots[0], "workers/kotlin23/src/main"],
            common_files
                .iter()
                .copied()
                .chain(["workers/kotlin23/build.gradle.kts"])
                .collect::<Vec<_>>(),
        ),
        (
            "KOTLIN24",
            "kotlin24",
            ":workers:kotlin:installDist",
            "workers/manifests/kotlin24.json",
            vec![common_roots[0]],
            common_files
                .iter()
                .copied()
                .chain(["workers/kotlin/build.gradle.kts"])
                .collect::<Vec<_>>(),
        ),
    ];
    let mut generated = String::new();
    for (name, variant, install_task, output_manifest, roots, files) in variants {
        let entries = collect_inputs(&repo, &roots, &files);
        let digest = manifest_digest(&entries);
        generated.push_str(&format!(
            "pub(crate) const PINNED_{name}_INPUT_DIGEST: &str = \"{digest}\";\n"
        ));
        generated.push_str(&format!(
            "pub(crate) static PINNED_{name}_INPUT_ROOTS: &[&str] = &["
        ));
        for root in &roots {
            generated.push_str(&format!("\"{root}\","));
            println!("cargo:rerun-if-changed={}", repo.join(root).display());
        }
        generated.push_str("];\n");
        generated.push_str(&format!(
            "pub(crate) static PINNED_{name}_INPUT_FILES: &[&str] = &["
        ));
        for file in &files {
            generated.push_str(&format!("\"{file}\","));
            println!("cargo:rerun-if-changed={}", repo.join(file).display());
        }
        generated.push_str("];\n");
        generated.push_str(&format!(
            "pub(crate) static PINNED_{name}_INPUTS: &[(&str,&str)] = &["
        ));
        for (path, hash) in entries {
            generated.push_str(&format!("(\"{path}\",\"{hash}\"),"));
        }
        generated.push_str("];\n");
        let (output_digest, output_entries) =
            validate_output_manifest(&repo, output_manifest, variant, install_task);
        println!(
            "cargo:rerun-if-changed={}",
            repo.join(output_manifest).display()
        );
        generated.push_str(&format!(
            "pub(crate) const PINNED_{name}_OUTPUT_DIGEST: &str = \"{output_digest}\";\n"
        ));
        generated.push_str(&format!(
            "pub(crate) static PINNED_{name}_OUTPUTS: &[(&str,u32,u64,&str)] = &["
        ));
        for (path, mode, size, hash) in output_entries {
            generated.push_str(&format!("(\"{path}\",{mode},{size},\"{hash}\"),"));
        }
        generated.push_str("];\n");
    }
    std::fs::write(
        PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("worker_build_inputs.rs"),
        generated,
    )
    .unwrap();
}

fn validate_output_manifest(
    repo: &Path,
    relative: &str,
    variant: &str,
    install_task: &str,
) -> (String, Vec<(String, u32, u64, String)>) {
    let bytes = std::fs::read(repo.join(relative)).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema"], "trusted-worker-distribution/0.2");
    assert_eq!(value["variant"], variant);
    assert_eq!(value["installTask"], install_task);
    let canonical = format!("{}\n", serde_json::to_string(&value).unwrap());
    assert_eq!(
        bytes,
        canonical.as_bytes(),
        "output manifest must be canonical JSON"
    );
    let mut entries = Vec::new();
    for row in value["files"].as_array().unwrap() {
        let path = row["path"].as_str().unwrap().to_owned();
        let raw_mode = row["mode"].as_u64().unwrap();
        assert!(raw_mode == 0 || raw_mode == 0o111);
        let mode = u32::try_from(raw_mode).unwrap();
        let size = row["size"].as_u64().unwrap();
        let hash = row["sha256"].as_str().unwrap().to_owned();
        assert!(
            !path.is_empty() && !path.starts_with('/') && !path.split('/').any(|part| part == "..")
        );
        assert!(hash.starts_with("sha256:") && hash.len() == 71);
        entries.push((path, mode, size, hash));
    }
    assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
    let digest = distribution_manifest_digest(&entries);
    assert_eq!(value["treeHash"].as_str(), Some(digest.as_str()));
    (digest, entries)
}

fn distribution_manifest_digest(entries: &[(String, u32, u64, String)]) -> String {
    let mut digest = Sha256::new();
    for (path, mode, size, hash) in entries {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(mode.to_string().as_bytes());
        digest.update([0]);
        digest.update(size.to_string().as_bytes());
        digest.update([0]);
        digest.update(hash.as_bytes());
        digest.update([0]);
    }
    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn collect_inputs(repo: &Path, roots: &[&str], files: &[&str]) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    for root in roots {
        for entry in WalkDir::new(repo.join(root)).follow_links(false) {
            let entry = entry.unwrap();
            let metadata = std::fs::symlink_metadata(entry.path()).unwrap();
            assert!(
                !metadata.file_type().is_symlink(),
                "worker input symlink: {}",
                entry.path().display()
            );
            if metadata.is_file() {
                paths.push(entry.path().to_path_buf());
            }
        }
    }
    paths.extend(files.iter().map(|path| repo.join(path)));
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(repo)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).unwrap();
            (
                relative,
                format!("sha256:{}", hex::encode(Sha256::digest(bytes))),
            )
        })
        .collect()
}

fn manifest_digest(entries: &[(String, String)]) -> String {
    let mut digest = Sha256::new();
    for (path, hash) in entries {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(hash.as_bytes());
        digest.update([0]);
    }
    format!("sha256:{}", hex::encode(digest.finalize()))
}
