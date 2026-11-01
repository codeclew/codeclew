//! Build-time identity of the complete portable skill package.
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};
use walkdir::WalkDir;

const MAX_FILES: usize = 128;
const MAX_TOTAL_BYTES: usize = 1152 * 1024;

/// Matches scripts/install_agent_skill.py:package_files/digest. Keep the
/// cross-language parity tests when the installer contract changes.
pub(crate) fn digest(root: &Path) -> io::Result<String> {
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(io::Error::other("bundled skill root is not a directory"));
    }
    let mut rows = Vec::new();
    let mut total = 0usize;
    for entry in WalkDir::new(root).min_depth(1).follow_links(false) {
        let entry = entry.map_err(io::Error::other)?;
        if entry.file_name() == ".DS_Store" {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            continue;
        }
        if !metadata.file_type().is_file() {
            return Err(io::Error::other(
                "bundled skill entry is not a regular file",
            ));
        }
        if metadata.len() > MAX_TOTAL_BYTES as u64 || rows.len() >= MAX_FILES {
            return Err(io::Error::other("bundled skill exceeds package limits"));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|part| {
                part.as_os_str()
                    .to_str()
                    .ok_or_else(|| io::Error::other("skill path is not UTF-8"))
            })
            .collect::<io::Result<Vec<_>>>()?
            .join("/");
        let content = fs::read(entry.path())?;
        total += content.len();
        if total > MAX_TOTAL_BYTES {
            return Err(io::Error::other("bundled skill exceeds package limits"));
        }
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        rows.push((
            relative,
            content,
            if executable { 0o755u32 } else { 0o644u32 },
        ));
    }
    if !rows.iter().any(|(relative, _, _)| relative == "SKILL.md") {
        return Err(io::Error::other("bundled skill has no SKILL.md"));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hash = Sha256::new();
    for (relative, content, mode) in rows {
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update(mode.to_string().as_bytes());
        hash.update([0]);
        hash.update(content);
        hash.update([0]);
    }
    Ok(format!("sha256:{}", hex::encode(hash.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn installer_output(root: &Path) -> std::process::Output {
        let installer =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install_agent_skill.py");
        Command::new("python3").args(["-I", "-S", "-B", "-c",
            "import runpy,sys; from pathlib import Path; m=runpy.run_path(sys.argv[1]); print(m['digest'](m['package_files'](Path(sys.argv[2]))))"])
            .arg(installer).arg(root).output().unwrap()
    }

    fn installer_digest(root: &Path) -> String {
        let result = installer_output(root);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn nested_files_and_normalized_modes_match_portable_installer() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("SKILL.md"), "Owned skill fixture\n").unwrap();
        let original = digest(root).unwrap();
        assert_eq!(original, installer_digest(root));
        fs::write(root.join(".DS_Store"), "ignored platform metadata").unwrap();
        assert_eq!(digest(root).unwrap(), original);
        let directory = root.join("references/future/nested");
        fs::create_dir_all(&directory).unwrap();
        let nested = directory.join("schema.json");
        fs::write(&nested, "{\"type\":\"string\"}\n").unwrap();
        let added = digest(root).unwrap();
        assert_ne!(added, original);
        assert_eq!(added, installer_digest(root));
        fs::write(&nested, "{\"type\":\"integer\"}\n").unwrap();
        let changed = digest(root).unwrap();
        assert_ne!(changed, added);
        assert_eq!(changed, installer_digest(root));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o755)).unwrap();
            let executable = digest(root).unwrap();
            assert_ne!(executable, changed);
            assert_eq!(executable, installer_digest(root));
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(digest(root).unwrap(), executable);
            assert_eq!(executable, installer_digest(root));
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o640)).unwrap();
            assert_eq!(digest(root).unwrap(), changed);
            assert_eq!(changed, installer_digest(root));
        }
        fs::remove_file(nested).unwrap();
        assert_eq!(digest(root).unwrap(), original);
    }

    #[test]
    fn expanded_package_keeps_a_shared_bounded_file_limit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("SKILL.md"), "Owned bounded fixture\n").unwrap();
        for index in 1..128 {
            fs::write(root.join(format!("part-{index}.md")), "fixture\n").unwrap();
        }
        assert_eq!(digest(root).unwrap(), installer_digest(root));
        fs::write(root.join("one-too-many.md"), "fixture\n").unwrap();
        assert!(digest(root).is_err());
    }

    #[test]
    fn expanded_package_keeps_a_shared_bounded_byte_limit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let skill = b"Owned bounded fixture\n";
        fs::write(root.join("SKILL.md"), skill).unwrap();
        let payload = root.join("payload");
        fs::write(&payload, vec![0u8; MAX_TOTAL_BYTES - skill.len()]).unwrap();
        assert_eq!(digest(root).unwrap(), installer_digest(root));
        fs::write(&payload, vec![0u8; MAX_TOTAL_BYTES - skill.len() + 1]).unwrap();
        assert!(digest(root).is_err());
        assert!(!installer_output(root).status.success());
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_package_entries_are_refused() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skill");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("SKILL.md"), "Owned skill fixture\n").unwrap();
        symlink(&root, temp.path().join("root-alias")).unwrap();
        assert!(digest(&temp.path().join("root-alias")).is_err());
        fs::write(temp.path().join("external"), "external").unwrap();
        symlink(temp.path().join("external"), root.join("linked")).unwrap();
        assert!(digest(&root).is_err());
        fs::remove_file(root.join("linked")).unwrap();
        fs::write(root.join("oversized"), vec![0u8; MAX_TOTAL_BYTES + 1]).unwrap();
        assert!(digest(&root).is_err());
    }
}
