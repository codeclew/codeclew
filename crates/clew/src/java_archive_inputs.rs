//! Bounded admission checks for Java classpath archives.
//!
//! This only reads ZIP central-directory metadata and, when present, the small
//! main manifest. It never extracts an archive or starts a Java process.

use crate::error::{ClewError, ErrorCode};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use zip::ZipArchive;

const MAX_ARCHIVE_ENTRIES: usize = 65_536;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Return whether an archive is safe for closed Java classpath admission.
///
/// A malformed or unsupported archive, a duplicate manifest, an invalid main
/// manifest, or a main-manifest `Class-Path` attribute returns `Ok(false)` so
/// callers can retain the ordinary legacy path. Filesystem failures while
/// opening or reading the admitted archive remain typed errors.
pub(crate) fn closed_archive_lookup(path: &Path) -> Result<bool, ClewError> {
    let mut file = File::open(path).map_err(io_error)?;
    // Bound the parser's metadata allocation before ZipArchive reads it. The
    // first closed profile deliberately leaves ZIP64/multi-disk to legacy.
    if !bounded_zip_directory(&mut file)? {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut archive = match ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(_) => return Ok(false),
    };
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Ok(false);
    }

    let mut manifest_index = None;
    for index in 0..archive.len() {
        let file = match archive.by_index_raw(index) {
            Ok(file) => file,
            Err(_) => return Ok(false),
        };
        if file.name().eq_ignore_ascii_case("META-INF/MANIFEST.MF")
            && manifest_index.replace(index).is_some()
        {
            return Ok(false);
        }
    }
    let Some(index) = manifest_index else {
        return Ok(true);
    };
    let manifest = match archive.by_index(index) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(false),
    };
    if manifest.size() > MAX_MANIFEST_BYTES {
        return Ok(false);
    }
    let mut bytes = Vec::with_capacity(manifest.size() as usize + 1);
    let mut limited = manifest.take(MAX_MANIFEST_BYTES + 1);
    limited.read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Ok(false);
    }
    Ok(!main_manifest_has_class_path(&bytes))
}

fn bounded_zip_directory(file: &mut File) -> Result<bool, ClewError> {
    let size = file.metadata().map_err(io_error)?.len();
    let tail_size = size.min(65_557) as usize;
    file.seek(SeekFrom::End(-(tail_size as i64)))
        .map_err(io_error)?;
    let mut tail = vec![0; tail_size];
    file.read_exact(&mut tail).map_err(io_error)?;
    for index in (0..tail_size.saturating_sub(21)).rev() {
        if tail.get(index..index + 4) != Some(&[0x50, 0x4b, 0x05, 0x06]) {
            continue;
        }
        let u16_at = |offset| u16::from_le_bytes([tail[index + offset], tail[index + offset + 1]]);
        let u32_at = |offset| {
            u32::from_le_bytes(
                tail[index + offset..index + offset + 4]
                    .try_into()
                    .unwrap_or_default(),
            )
        };
        if index + 22 + usize::from(u16_at(20)) != tail_size {
            continue;
        }
        let count = u16_at(10);
        let directory_bytes = u32_at(12);
        let directory_offset = u32_at(16);
        return Ok(u16_at(4) == 0
            && u16_at(6) == 0
            && u16_at(8) == count
            && count != u16::MAX
            && usize::from(count) <= MAX_ARCHIVE_ENTRIES
            && directory_bytes <= 16 * 1024 * 1024
            && directory_offset != u32::MAX
            && u64::from(directory_offset) + u64::from(directory_bytes) <= size);
    }
    Ok(false)
}

fn main_manifest_has_class_path(bytes: &[u8]) -> bool {
    let mut seen = Vec::<String>::new();
    let mut current_name: Option<String> = None;
    for raw in bytes.split(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            break;
        }
        if line.first() == Some(&b' ') {
            if current_name.is_none() {
                return true;
            }
            continue;
        }
        if let Some(name) = current_name.take() {
            if seen.iter().any(|prior| prior.eq_ignore_ascii_case(&name)) {
                return true;
            }
            if name.eq_ignore_ascii_case("Class-Path") {
                return true;
            }
            seen.push(name);
        }
        let Some(colon) = line.iter().position(|byte| *byte == b':') else {
            return true;
        };
        if colon == 0 || line.get(colon + 1) != Some(&b' ') {
            return true;
        }
        let Ok(name) = std::str::from_utf8(&line[..colon]) else {
            return true;
        };
        current_name = Some(name.to_owned());
    }
    if let Some(name) = current_name {
        if seen.iter().any(|prior| prior.eq_ignore_ascii_case(&name)) {
            return true;
        }
        if name.eq_ignore_ascii_case("Class-Path") {
            return true;
        }
    }
    false
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use tempfile::TempDir;
    use zip::CompressionMethod;
    use zip::write::{SimpleFileOptions, ZipWriter};

    fn archive(
        manifest: Option<&[u8]>,
        compression: CompressionMethod,
    ) -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("entry.bin");
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(compression);
        if let Some(manifest) = manifest {
            writer.start_file("META-INF/MANIFEST.MF", options).unwrap();
            writer.write_all(manifest).unwrap();
        } else {
            writer.start_file("classes/Example.class", options).unwrap();
            writer.write_all(b"not a class").unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        std::fs::write(&path, bytes).unwrap();
        (temp, path)
    }

    #[test]
    fn stored_and_deflated_archives_without_manifest_lookup_are_safe() {
        let (_stored_temp, stored) = archive(None, CompressionMethod::Stored);
        assert!(closed_archive_lookup(&stored).unwrap());
        let (_deflated_temp, deflated) = archive(None, CompressionMethod::Deflated);
        assert!(closed_archive_lookup(&deflated).unwrap());
    }

    #[test]
    fn folded_case_insensitive_class_path_is_refused() {
        let manifest = b"Manifest-Version: 1.0\r\ncLaSs-PaTh: first.jar\r\n second.jar\r\n\r\n";
        let (_temp, path) = archive(Some(manifest), CompressionMethod::Deflated);
        assert!(!closed_archive_lookup(&path).unwrap());
    }

    #[test]
    fn duplicate_manifest_attributes_and_oversized_manifest_are_refused() {
        let duplicate = b"Manifest-Version: 1.0\nX-Test: one\nx-test: two\n\n";
        let (_duplicate_temp, duplicate_path) = archive(Some(duplicate), CompressionMethod::Stored);
        assert!(!closed_archive_lookup(&duplicate_path).unwrap());

        let oversized = vec![b'x'; (MAX_MANIFEST_BYTES + 1) as usize];
        let (_oversized_temp, oversized_path) =
            archive(Some(&oversized), CompressionMethod::Stored);
        assert!(!closed_archive_lookup(&oversized_path).unwrap());
    }

    #[test]
    fn malformed_archive_is_ineligible_without_inflation() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("malformed.jar");
        std::fs::write(&path, b"not a zip").unwrap();
        assert!(!closed_archive_lookup(&path).unwrap());
    }
    #[test]
    fn oversized_central_directory_is_refused_before_metadata_allocation() {
        let (_temp, path) = archive(None, CompressionMethod::Stored);
        let mut bytes = std::fs::read(&path).unwrap();
        let footer = bytes.len() - 22;
        bytes[footer + 12..footer + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(!closed_archive_lookup(&path).unwrap());
    }
}
