//! Bounded part I/O. Complete typed validation remains in evidence_package.
use super::*;
use std::io::Write;

pub(super) const MAX_JSON_NODES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(super) struct JsonBudget {
    nodes: usize,
}
impl JsonBudget {
    fn add(&mut self, count: usize) -> Result<(), ClewError> {
        let observed = self
            .nodes
            .checked_add(count)
            .ok_or_else(|| invalid("evidence JSON node counter overflow"))?;
        if observed > MAX_JSON_NODES {
            let mut error = invalid(format!(
                "EVIDENCE_PACKAGE_LIMIT_EXCEEDED: jsonNodes observed={observed} limit={MAX_JSON_NODES}"
            ));
            error.details = Some(Box::new(json!({"stage":"VALIDATE_EVIDENCE_PACKAGE_JSON",
                "bound":"jsonNodes","observed":observed,"limit":MAX_JSON_NODES,
                "nextAction":"Use the exact retained evidence for bounded local questions; do not repeat capture or recovery to change this resource limit."})));
            return Err(error);
        }
        self.nodes = observed;
        Ok(())
    }
    fn value(&mut self, value: &Value) -> Result<(), ClewError> {
        self.add(1)?;
        match value {
            Value::Array(rows) => {
                for row in rows {
                    self.value(row)?;
                }
            }
            Value::Object(fields) => {
                self.add(fields.len())?; // Retained object keys also allocate.
                for row in fields.values() {
                    self.value(row)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn part(&mut self, part: &Part) -> Result<(), ClewError> {
        // Root object, three keys, schema/kind strings and records array.
        self.add(7)?;
        for row in &part.records {
            self.value(row)?;
        }
        Ok(())
    }
}

pub(super) fn verified_bytes(input: &Path, part: &PartRef) -> Result<Vec<u8>, ClewError> {
    let data = read_bytes(input, &part.path, part.bytes)?;
    if data.len() as u64 != part.bytes || canonical::hash_bytes(&data) != part.digest {
        return Err(invalid("evidence part digest mismatch"));
    }
    Ok(data)
}
pub(super) fn verified_part(input: &Path, part: &PartRef) -> Result<Part, ClewError> {
    let data = verified_bytes(input, part)?;
    let value: Part = serde_json::from_slice(&data)
        .map_err(|_| invalid("invalid evidence part JSON or schema"))?;
    if value.schema != PART_SCHEMA || value.kind != part.kind || value.records.len() != part.records
    {
        return Err(invalid("evidence part schema or count mismatch"));
    }
    Ok(value)
}

struct LimitedRow(Vec<u8>);
impl Write for LimitedRow {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(data.len())
            .is_none_or(|n| n > MAX_PART as usize)
        {
            return Err(std::io::Error::other("portable row exceeds its byte bound"));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn row_value<T: Serialize>(row: T) -> Result<(Value, usize), ClewError> {
    // Stop an oversized typed row before constructing a cloned, unbounded Value.
    let mut output = LimitedRow(Vec::new());
    serde_json::to_writer(&mut output, &row)
        .map_err(|_| invalid("one indexed record exceeds the portable part limit"))?;
    let size = output.0.len();
    let value = serde_json::from_slice(&output.0)
        .map_err(|_| invalid("portable row JSON exceeds its supported depth or schema"))?;
    Ok((value, size))
}

pub(super) fn for_each_part(
    kind: &str,
    rows: impl IntoIterator<Item = Result<(Value, usize), ClewError>>,
    mut emit: impl FnMut(PartRef, Vec<u8>, &Part) -> Result<(), ClewError>,
) -> Result<(), ClewError> {
    let mut group = Vec::new();
    let mut size = 0usize;
    let mut flush = |group: &mut Vec<Value>| -> Result<(), ClewError> {
        if group.is_empty() {
            return Ok(());
        }
        let part = Part {
            schema: PART_SCHEMA.into(),
            kind: kind.into(),
            records: std::mem::take(group),
        };
        let data = bytes(&part)?;
        if data.len() as u64 > MAX_PART {
            return Err(invalid(
                "one indexed record exceeds the portable part limit",
            ));
        }
        let digest = canonical::hash_bytes(&data);
        emit(
            PartRef {
                kind: kind.into(),
                path: format!("parts/{}.json", &digest[7..]),
                digest,
                bytes: data.len() as u64,
                records: part.records.len(),
                encoding: "json".into(),
            },
            data,
            &part,
        )
    };
    for row in rows {
        let (value, next) = row?;
        if size.checked_add(next).is_none_or(|n| n > 512 * 1024) || group.len() >= PART_RECORDS {
            flush(&mut group)?;
            size = 0;
        }
        group.push(value);
        size = size
            .checked_add(next)
            .ok_or_else(|| invalid("evidence part size counter overflow"))?;
    }
    flush(&mut group)
}

pub(super) struct Writer {
    stage: tempfile::TempDir,
    output: PathBuf,
    refs: Vec<PartRef>,
    total: u64,
    records: usize,
    json: JsonBudget,
}
impl Writer {
    pub(super) fn new(output: &Path) -> Result<Self, ClewError> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(io_error)?;
        let parent = parent.canonicalize().map_err(io_error)?;
        let output = parent.join(
            output
                .file_name()
                .ok_or_else(|| invalid("package output requires a directory name"))?,
        );
        if fs::symlink_metadata(&output).is_ok() {
            return Err(invalid(
                "package output already exists; select a new immutable output directory",
            ));
        }
        let stage = tempfile::tempdir_in(parent).map_err(io_error)?;
        fs::create_dir(stage.path().join("parts")).map_err(io_error)?;
        Ok(Self {
            stage,
            output,
            refs: vec![],
            total: 0,
            records: 0,
            json: JsonBudget::default(),
        })
    }
    pub(super) fn emit<T: Serialize>(
        &mut self,
        kind: &str,
        rows: impl IntoIterator<Item = T>,
    ) -> Result<(), ClewError> {
        for_each_part(
            kind,
            rows.into_iter().map(row_value),
            |reference, data, part| {
                self.json.part(part)?;
                self.total = self
                    .total
                    .checked_add(reference.bytes)
                    .ok_or_else(|| invalid("evidence byte counter overflow"))?;
                self.records = self
                    .records
                    .checked_add(reference.records)
                    .ok_or_else(|| invalid("evidence record counter overflow"))?;
                if self.total > MAX_TOTAL {
                    return Err(limit_error("totalBytes", self.total, MAX_TOTAL));
                }
                if self.records > MAX_RECORDS {
                    return Err(limit_error(
                        "records",
                        self.records as u64,
                        MAX_RECORDS as u64,
                    ));
                }
                if self.refs.len() >= MAX_PARTS {
                    return Err(limit_error(
                        "parts",
                        (self.refs.len() + 1) as u64,
                        MAX_PARTS as u64,
                    ));
                }
                fs::write(self.stage.path().join(&reference.path), data).map_err(io_error)?;
                self.refs.push(reference);
                Ok(())
            },
        )
    }
    pub(super) fn parts(&self) -> Vec<PartRef> {
        self.refs.clone()
    }
    pub(super) fn finish(self, manifest: &Manifest) -> Result<String, ClewError> {
        super::super::progress::run("VALIDATE_EVIDENCE_PACKAGE_LIMITS", || {
            validate_parts(&manifest.parts)
        })?;
        fs::write(self.stage.path().join("manifest.json"), bytes(manifest)?).map_err(io_error)?;
        let digest = {
            let package =
                super::super::progress::run("VERIFY_EVIDENCE_PACKAGE", || load(self.stage.path()))?;
            package.digest
        }; // Drop the one typed validation model before publishing.
        if fs::symlink_metadata(&self.output).is_ok() {
            return Err(invalid(
                "package output already exists; select a new immutable output directory",
            ));
        }
        fs::rename(self.stage.path(), self.output).map_err(io_error)?;
        Ok(digest)
    }
}

/// Reread raw rows after full validation, preserving legacy JSON shape. Every
/// part reread is checked again; no byte or typed-model inventory is retained.
pub(super) struct RawRows<'a> {
    input: &'a Path,
    parts: std::vec::IntoIter<&'a PartRef>,
    current: std::vec::IntoIter<Value>,
    failed: bool,
}
impl<'a> RawRows<'a> {
    pub(super) fn new(input: &'a Path, manifest: &'a Manifest, kind: &str) -> Self {
        Self {
            input,
            parts: manifest
                .parts
                .iter()
                .filter(|p| p.kind == kind)
                .collect::<Vec<_>>()
                .into_iter(),
            current: vec![].into_iter(),
            failed: false,
        }
    }
}
impl Iterator for RawRows<'_> {
    type Item = Result<Value, ClewError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(row) = self.current.next() {
                return Some(Ok(row));
            }
            let reference = self.parts.next()?;
            match verified_part(self.input, reference) {
                Ok(part) => self.current = part.records.into_iter(),
                Err(error) => {
                    self.failed = true;
                    return Some(Err(error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_budget_counts_keys_values_and_wrapper_with_checked_boundaries() {
        let part = Part {
            schema: PART_SCHEMA.into(),
            kind: "contracts".into(),
            records: vec![json!({"id":"one", "value":[null, false, 1]})],
        };
        let mut budget = JsonBudget::default();
        budget.part(&part).unwrap();
        assert_eq!(budget.nodes, 15); // wrapper7, object1, keys2, string1, array1, values3
        budget.nodes = MAX_JSON_NODES - 15;
        budget.part(&part).unwrap();
        assert_eq!(budget.nodes, MAX_JSON_NODES);
        let error = budget.add(1).unwrap_err();
        assert_eq!(error.details.unwrap()["observed"], MAX_JSON_NODES + 1);
        budget.nodes = usize::MAX;
        assert!(budget.add(1).unwrap_err().message.contains("overflow"));
    }

    #[test]
    fn bounded_rows_and_spool_failure_publish_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("output");
        {
            let mut writer = Writer::new(&output).unwrap();
            writer
                .emit("contracts", [json!({"id":"valid", "value":true})])
                .unwrap();
            assert!(
                writer
                    .emit("contracts", ["x".repeat(MAX_PART as usize + 1)])
                    .is_err()
            );
        }
        assert!(!output.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        {
            let mut writer = Writer::new(&output).unwrap();
            fs::remove_dir(writer.stage.path().join("parts")).unwrap();
            assert!(writer.emit("contracts", [true]).is_err());
        }
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn spool_counters_refuse_overflow_and_part_count_before_publication() {
        for mode in 0..3 {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("output");
            {
                let mut writer = Writer::new(&output).unwrap();
                match mode {
                    0 => writer.total = u64::MAX,
                    1 => writer.records = usize::MAX,
                    _ => {
                        writer.refs = vec![
                            PartRef {
                                kind: "contracts".into(),
                                path: String::new(),
                                digest: String::new(),
                                bytes: 1,
                                records: 1,
                                encoding: "json".into()
                            };
                            MAX_PARTS
                        ]
                    }
                }
                let error = writer.emit("contracts", [true]).unwrap_err();
                assert!(
                    error
                        .message
                        .contains(if mode < 2 { "overflow" } else { "parts" })
                );
            }
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn dense_json_control_remains_bounded_and_counts_every_element() {
        let part = Part {
            schema: PART_SCHEMA.into(),
            kind: "contracts".into(),
            records: vec![json!({"id":"dense", "value":vec![Value::Null; 200_000]})],
        };
        let data = bytes(&part).unwrap();
        assert!(data.len() < MAX_PART as usize);
        let mut budget = JsonBudget::default();
        budget.part(&part).unwrap();
        assert_eq!(budget.nodes, 200_012);
        // Single-digit values are denser than nulls. Wrapper syntax leaves a
        // strict margin below one charged value/key per two encoded bytes.
        let dense = Part {
            schema: PART_SCHEMA.into(),
            kind: "contracts".into(),
            records: vec![json!({"id":"dense", "value":vec![0; 200_000]})],
        };
        let mut budget = JsonBudget::default();
        budget.part(&dense).unwrap();
        assert!(budget.nodes * 2 <= bytes(&dense).unwrap().len());
    }
}
