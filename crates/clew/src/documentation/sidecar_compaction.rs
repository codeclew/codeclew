//! Bounded cleanup of legacy advisory `meta.json` cache sidecars.
//!
//! The plan is immutable and contains only validated metadata candidates. The
//! payload and object directory remain untouched; apply moves a sidecar into a
//! private staging slot before deleting the captured file.

use super::{cache::OBJECT_ROOT, invalid, io_error, store::Repository};
use crate::canonical;
use crate::error::ClewError;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const PLAN_SCHEMA: &str = "codeclew-documentation-sidecar-compaction-plan/1.0";
const REPORT_SCHEMA: &str = "codeclew-documentation-sidecar-compaction-report/1.0";
const POLICY: &str = "legacy-sidecar-v1";
const MAX_PAYLOAD: u64 = 128 * 1024 * 1024;
const MAX_META: u64 = 64 * 1024;
const DEFAULT_LIMIT: u64 = 100;
const DEFAULT_MAX_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Subcommand)]
pub enum Command {
    CompactSidecars(CompactArgs),
}

#[derive(Debug, Args)]
pub struct CompactArgs {
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long)]
    pub plan_output: Option<PathBuf>,
    #[arg(long)]
    pub apply: bool,
    #[arg(long)]
    pub plan: Option<PathBuf>,
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    pub limit: u64,
    #[arg(long, default_value_t = DEFAULT_MAX_BYTES)]
    pub max_bytes: u64,
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    size: u64,
    ctime_ns: i128,
    mtime_ns: i128,
    mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanHeader {
    schema: String,
    policy: String,
    root_identity: FileIdentity,
    object_root_identity: FileIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyMeta {
    schema: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanEntry {
    kind: String,
    record_index: u64,
    relative: String,
    digest: String,
    payload_size: u64,
    metadata_digest: String,
    metadata: LegacyMeta,
    meta_identity: FileIdentity,
    object_dir_identity: FileIdentity,
    logical_bytes: u64,
    allocated_bytes: u64,
    byte_offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanFooter {
    kind: String,
    entry_count: u64,
    candidate_count: u64,
    skipped_count: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    records_checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    policy: String,
    plan_digest: String,
    next_record_index: u64,
    next_byte_offset: u64,
    plan_identity: FileIdentity,
    records_checksum: String,
    entry_count: u64,
    footer_offset: u64,
    checksum: String,
}

#[cfg(unix)]
mod anchored;

#[cfg(unix)]
mod platform {
    use super::anchored::Dir;
    use super::*;
    use std::os::unix::fs::MetadataExt;

    fn identity(m: &fs::Metadata) -> FileIdentity {
        FileIdentity {
            dev: m.dev(),
            ino: m.ino(),
            size: m.len(),
            ctime_ns: i128::from(m.ctime()) * 1_000_000_000 + i128::from(m.ctime_nsec()),
            mtime_ns: i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec()),
            mode: m.mode(),
        }
    }
    fn same_dir(a: &FileIdentity, b: &FileIdentity) -> bool {
        a.dev == b.dev && a.ino == b.ino && a.mode == b.mode
    }
    fn allocated(m: &fs::Metadata) -> u64 {
        m.blocks().saturating_mul(512)
    }
    fn valid_digest(s: &str) -> bool {
        s.len() == 71
            && s.starts_with("sha256:")
            && s.as_bytes()[7..]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    }
    fn validate_entry(e: &PlanEntry, index: u64, offset: u64) -> Result<(), ClewError> {
        if e.kind != "ENTRY"
            || e.record_index != index
            || e.byte_offset != offset
            || !valid_digest(&e.digest)
            || e.relative != format!("objects/{}/meta.json", e.digest)
            || !valid_digest(&e.metadata_digest)
            || e.payload_size > MAX_PAYLOAD
            || e.metadata.size != e.payload_size
            || e.metadata.schema.trim().is_empty()
            || e.logical_bytes > MAX_META
            || e.meta_identity.size != e.logical_bytes
        {
            return Err(invalid("invalid sidecar plan entry"));
        }
        Ok(())
    }
    struct Roots {
        root: Dir,
        cache: Dir,
        objects: Dir,
    }
    impl Roots {
        fn open(repo: &Repository) -> Result<Self, ClewError> {
            let root = Dir::open_root(&repo.root)?;
            let cache = root.open_dir(".codeclew")?.open_dir("cache")?;
            let objects = cache.open_dir("objects")?;
            Ok(Self {
                root,
                cache,
                objects,
            })
        }
        fn check(&self, h: &PlanHeader) -> Result<(), ClewError> {
            if h.schema != PLAN_SCHEMA
                || h.policy != POLICY
                || !same_dir(&identity(&self.root.metadata()?), &h.root_identity)
                || !same_dir(
                    &identity(&self.objects.metadata()?),
                    &h.object_root_identity,
                )
            {
                return Err(invalid("sidecar compaction plan root or policy changed"));
            }
            Ok(())
        }
    }
    struct Meta {
        file: File,
        identity: FileIdentity,
        record: LegacyMeta,
        raw: Vec<u8>,
        allocated: u64,
    }
    fn read_meta(dir: &Dir, name: &str, limit: u64) -> Result<Meta, ClewError> {
        let mut file = dir.open_file(name, limit)?;
        let before = file.metadata().map_err(io_error)?;
        let mut raw = Vec::new();
        Read::by_ref(&mut file)
            .take(limit + 1)
            .read_to_end(&mut raw)
            .map_err(io_error)?;
        if raw.len() as u64 > limit
            || identity(&before) != identity(&file.metadata().map_err(io_error)?)
        {
            return Err(invalid("sidecar changed while being read"));
        }
        let record: LegacyMeta = serde_json::from_slice(&raw)
            .map_err(|_| invalid("sidecar is not a closed legacy record"))?;
        if record.schema.trim().is_empty() {
            return Err(invalid("empty legacy metadata schema"));
        }
        Ok(Meta {
            file,
            identity: identity(&before),
            record,
            raw,
            allocated: allocated(&before),
        })
    }
    fn payload(dir: &Dir, limit: u64) -> Result<(String, u64), ClewError> {
        let mut file = dir.open_file("object.json", limit)?;
        let before = identity(&file.metadata().map_err(io_error)?);
        let mut hasher = Sha256::new();
        let mut count = 0u64;
        let mut buf = [0u8; 65536];
        loop {
            let bound = ((limit + 1 - count) as usize).min(buf.len());
            let n = file.read(&mut buf[..bound]).map_err(io_error)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > limit {
                return Err(invalid("payload exceeds compaction bound"));
            }
            hasher.update(&buf[..n]);
        }
        if before != identity(&file.metadata().map_err(io_error)?) {
            return Err(invalid("payload changed while being read"));
        }
        Ok((format!("sha256:{}", hex::encode(hasher.finalize())), count))
    }
    fn plan_location(path: &Path, repo: &Repository) -> Result<(), ClewError> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let resolved = parent.canonicalize().map_err(io_error)?;
        let objects = repo
            .root
            .join(OBJECT_ROOT)
            .canonicalize()
            .map_err(io_error)?;
        if resolved.starts_with(objects) || path.file_name().is_none() {
            return Err(invalid("plan must be outside object namespace"));
        }
        Ok(())
    }
    fn line_bytes(value: &impl Serialize) -> Result<Vec<u8>, ClewError> {
        let mut bytes = serde_json::to_vec(value).map_err(io_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
    fn plan(repo: &Repository, output: &Path) -> Result<Value, ClewError> {
        let roots = Roots::open(repo)?;
        plan_location(output, repo)?;
        let header = PlanHeader {
            schema: PLAN_SCHEMA.into(),
            policy: POLICY.into(),
            root_identity: identity(&roots.root.metadata()?),
            object_root_identity: identity(&roots.objects.metadata()?),
        };
        let mut writer = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)
                .map_err(io_error)?,
        );
        let mut hash = Sha256::new();
        let mut records = Sha256::new();
        let mut offset = 0;
        let bytes = line_bytes(&header)?;
        writer.write_all(&bytes).map_err(io_error)?;
        hash.update(&bytes);
        records.update(&bytes);
        offset += bytes.len() as u64;
        let mut count = 0;
        let mut skipped = 0;
        let mut logical = 0;
        let mut allocation = 0;
        let mut reasons = Vec::new();
        let mut validated = 0;
        for item in fs::read_dir(repo.root.join(OBJECT_ROOT)).map_err(io_error)? {
            let item = item.map_err(io_error)?;
            let name = item.file_name().to_string_lossy().into_owned();
            let candidate = (|| -> Result<PlanEntry, ClewError> {
                if !valid_digest(&name) {
                    return Err(invalid("unknown object entry"));
                }
                let dir = roots.objects.open_dir(&name)?;
                let meta = read_meta(&dir, "meta.json", MAX_META)?;
                let (digest, size) = payload(&dir, MAX_PAYLOAD)?;
                validated += size;
                if digest != name || size != meta.record.size {
                    return Err(invalid("metadata/payload identity mismatch"));
                }
                Ok(PlanEntry {
                    kind: "ENTRY".into(),
                    record_index: count,
                    relative: format!("objects/{name}/meta.json"),
                    digest: name.clone(),
                    payload_size: size,
                    metadata_digest: canonical::hash_bytes(&meta.raw),
                    metadata: meta.record,
                    meta_identity: meta.identity,
                    object_dir_identity: identity(&dir.metadata()?),
                    logical_bytes: meta.raw.len() as u64,
                    allocated_bytes: meta.allocated,
                    byte_offset: offset,
                })
            })();
            match candidate {
                Ok(entry) => {
                    let bytes = line_bytes(&entry)?;
                    writer.write_all(&bytes).map_err(io_error)?;
                    hash.update(&bytes);
                    records.update(&bytes);
                    offset += bytes.len() as u64;
                    count += 1;
                    logical += entry.logical_bytes;
                    allocation += entry.allocated_bytes;
                }
                Err(error) => {
                    skipped += 1;
                    if reasons.len() < 128 {
                        reasons.push(json!({"object":name,"reason":error.to_string()}));
                    }
                }
            }
        }
        let footer = PlanFooter {
            kind: "FOOTER".into(),
            entry_count: count,
            candidate_count: count,
            skipped_count: skipped,
            logical_bytes: logical,
            allocated_bytes: allocation,
            records_checksum: format!("sha256:{}", hex::encode(records.finalize())),
        };
        let bytes = line_bytes(&footer)?;
        writer.write_all(&bytes).map_err(io_error)?;
        hash.update(&bytes);
        writer.flush().map_err(io_error)?;
        Ok(
            json!({"schema":REPORT_SCHEMA,"mode":"PLAN","plan":output,"planDigest":format!("sha256:{}",hex::encode(hash.finalize())),"candidateCount":count,"skipped":skipped,"logicalBytes":logical,"allocatedBytesEstimate":allocation,"validatedPayloadBytes":validated,"planBytesRead":0,"planBytesWritten":offset+bytes.len() as u64,"skips":reasons,"nextCursor":null}),
        )
    }

    // Count bytes returned by actual file reads, including buffer prefetch and re-reads.
    struct Counted {
        file: File,
        reads: u64,
    }
    impl Read for Counted {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = self.file.read(b)?;
            self.reads += n as u64;
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.file.seek(p)
        }
    }
    type PlanReader = BufReader<Counted>;
    fn read_line(reader: &mut PlanReader) -> Result<Vec<u8>, ClewError> {
        let mut b = Vec::new();
        Read::by_ref(reader)
            .take(MAX_META + 1)
            .read_until(b'\n', &mut b)
            .map_err(io_error)?;
        if b.len() as u64 > MAX_META || b.last() != Some(&b'\n') {
            return Err(invalid("incomplete or oversized plan record"));
        }
        Ok(b)
    }
    fn parse<T: serde::de::DeserializeOwned>(b: &[u8]) -> Result<T, ClewError> {
        serde_json::from_slice(b).map_err(|_| invalid("invalid closed plan record"))
    }
    fn open_plan(path: &Path) -> Result<(PlanReader, FileIdentity), ClewError> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .map_err(io_error)?;
        let m = file.metadata().map_err(io_error)?;
        if !m.is_file() || m.uid() != unsafe { libc::geteuid() } {
            return Err(invalid("plan must be an owned regular file"));
        }
        Ok((BufReader::new(Counted { file, reads: 0 }), identity(&m)))
    }
    fn same_plan(reader: &PlanReader, id: &FileIdentity) -> Result<(), ClewError> {
        if &identity(&reader.get_ref().file.metadata().map_err(io_error)?) != id {
            return Err(invalid("PLAN_CHANGED"));
        }
        Ok(())
    }
    fn cursor_text(c: &Cursor) -> Result<String, ClewError> {
        let mut c = c.clone();
        c.checksum.clear();
        c.checksum = canonical::hash(&c).map_err(io_error)?;
        serde_json::to_string(&c).map_err(io_error)
    }
    fn load_cursor(raw: &str) -> Result<Cursor, ClewError> {
        if raw.len() > 16384 {
            return Err(invalid("oversized cursor"));
        }
        let mut c: Cursor =
            serde_json::from_str(raw).map_err(|_| invalid("invalid sidecar cursor"))?;
        let checksum = std::mem::take(&mut c.checksum);
        if checksum != canonical::hash(&c).map_err(io_error)?
            || c.policy != POLICY
            || !valid_digest(&c.plan_digest)
        {
            return Err(invalid("invalid cursor checksum or policy"));
        }
        c.checksum = checksum;
        Ok(c)
    }
    fn admission(
        reader: &mut PlanReader,
        id: &FileIdentity,
        roots: &Roots,
        raw: Option<&str>,
    ) -> Result<Cursor, ClewError> {
        let header_bytes = read_line(reader)?;
        let header: PlanHeader = parse(&header_bytes)?;
        roots.check(&header)?;
        let first = header_bytes.len() as u64;
        if let Some(raw) = raw {
            let c = load_cursor(raw)?;
            if &c.plan_identity != id
                || c.next_record_index > c.entry_count
                || c.next_byte_offset < first
                || c.next_byte_offset > c.footer_offset
                || c.footer_offset >= id.size
            {
                return Err(invalid("PLAN_CHANGED or invalid cursor range"));
            }
            reader
                .seek(SeekFrom::Start(c.footer_offset))
                .map_err(io_error)?;
            let footer: PlanFooter = parse(&read_line(reader)?)?;
            if footer.kind != "FOOTER"
                || footer.entry_count != c.entry_count
                || footer.candidate_count != c.entry_count
                || footer.records_checksum != c.records_checksum
                || reader.stream_position().map_err(io_error)? != id.size
            {
                return Err(invalid("cursor footer mismatch"));
            }
            same_plan(reader, id)?;
            return Ok(c);
        }
        let mut hash = Sha256::new();
        let mut records = Sha256::new();
        hash.update(&header_bytes);
        records.update(&header_bytes);
        let mut index = 0;
        let mut offset = first;
        let mut logical = 0;
        let mut allocation = 0;
        loop {
            let b = read_line(reader)?;
            let v: Value = parse(&b)?;
            if v["kind"] == "FOOTER" {
                let f: PlanFooter = parse(&b)?;
                let checksum = format!("sha256:{}", hex::encode(records.finalize()));
                if f.entry_count != index
                    || f.candidate_count != index
                    || f.logical_bytes != logical
                    || f.allocated_bytes != allocation
                    || f.records_checksum != checksum
                    || reader.stream_position().map_err(io_error)? != id.size
                {
                    return Err(invalid("incomplete or inconsistent compaction plan"));
                }
                hash.update(&b);
                same_plan(reader, id)?;
                return Ok(Cursor {
                    policy: POLICY.into(),
                    plan_digest: format!("sha256:{}", hex::encode(hash.finalize())),
                    next_record_index: 0,
                    next_byte_offset: first,
                    plan_identity: id.clone(),
                    records_checksum: checksum,
                    entry_count: index,
                    footer_offset: offset,
                    checksum: String::new(),
                });
            }
            let entry: PlanEntry = parse(&b)?;
            validate_entry(&entry, index, offset)?;
            logical += entry.logical_bytes;
            allocation += entry.allocated_bytes;
            index += 1;
            offset += b.len() as u64;
            records.update(&b);
            hash.update(&b);
        }
    }

    #[derive(Default)]
    struct Outcome {
        reason: &'static str,
        payload_bytes: u64,
        logical: u64,
        allocated: u64,
        retained: bool,
    }
    fn matched(meta: &Meta, e: &PlanEntry, captured: bool) -> bool {
        let identity_matches = if captured {
            meta.identity.dev == e.meta_identity.dev
                && meta.identity.ino == e.meta_identity.ino
                && meta.identity.size == e.meta_identity.size
                && meta.identity.mtime_ns == e.meta_identity.mtime_ns
                && meta.identity.mode == e.meta_identity.mode
        } else {
            meta.identity == e.meta_identity
        };
        identity_matches
            && meta.record == e.metadata
            && canonical::hash_bytes(&meta.raw) == e.metadata_digest
    }
    #[derive(Clone, Copy)]
    enum Phase {
        BeforeCapture,
        AfterCapture,
        AfterUnlink,
    }
    fn process(
        objects: &Dir,
        stage: &Dir,
        e: &PlanEntry,
        mut hook: impl FnMut(Phase),
    ) -> Result<Outcome, ClewError> {
        validate_entry(e, e.record_index, e.byte_offset)?;
        let name = format!("{:020}.meta.json", e.record_index);
        // A captured slot is bound by the plan-digest directory and record index.
        // Recovery never touches a newly created original metadata file.
        let staged = stage.exists_nofollow(&name)?;
        let mut out = Outcome {
            reason: "CHANGED_UNSAFE_ENTRY",
            retained: staged,
            ..Default::default()
        };
        let object = match objects.open_dir(&e.digest) {
            Ok(d) => d,
            Err(_) => return Ok(out),
        };
        if !same_dir(&identity(&object.metadata()?), &e.object_dir_identity) {
            return Ok(out);
        }
        if !staged && !object.exists_nofollow("meta.json")? {
            out.reason = "ALREADY_ABSENT";
            return Ok(out);
        }
        let held = match read_meta(
            if staged { stage } else { &object },
            if staged { &name } else { "meta.json" },
            e.logical_bytes,
        ) {
            Ok(m) => m,
            Err(_) => return Ok(out),
        };
        if !matched(&held, e, staged) {
            out.reason = if staged {
                "CHANGED_CAPTURE_RETAINED"
            } else {
                "CHANGED_OR_RECREATED"
            };
            return Ok(out);
        }
        let (digest, size) = match payload(&object, e.payload_size) {
            Ok(v) => v,
            Err(_) => return Ok(out),
        };
        out.payload_bytes = size;
        if digest != e.digest || size != e.payload_size {
            out.reason = "PAYLOAD_CHANGED";
            return Ok(out);
        }
        if !staged {
            hook(Phase::BeforeCapture);
            object.capture("meta.json", stage, &name)?;
            hook(Phase::AfterCapture);
        }
        let captured = match read_meta(stage, &name, e.logical_bytes) {
            Ok(m) => m,
            Err(_) => {
                out.reason = "CHANGED_CAPTURE_RETAINED";
                out.retained = true;
                return Ok(out);
            }
        };
        // Keep held.file alive until after capture verification: same-content inode
        // replacement during capture must be retained, never mistaken for the plan.
        if !matched(&captured, e, true)
            || captured.identity.dev != held.identity.dev
            || captured.identity.ino != held.identity.ino
        {
            out.reason = "CHANGED_CAPTURE_RETAINED";
            out.retained = true;
            return Ok(out);
        }
        let _held_fd = &held.file;
        stage.unlink(&name)?;
        hook(Phase::AfterUnlink);
        out.reason = "REMOVED";
        out.retained = false;
        out.logical = captured.raw.len() as u64;
        out.allocated = captured.allocated;
        Ok(out)
    }
    fn add(report: &mut Value, key: &str, n: u64) {
        report[key] = json!(report[key].as_u64().unwrap_or(0) + n);
    }
    fn apply(repo: &Repository, args: &CompactArgs) -> Result<Value, ClewError> {
        let path = args
            .plan
            .as_ref()
            .ok_or_else(|| invalid("--apply requires --plan"))?;
        if args.limit == 0 || args.max_bytes == 0 {
            return Err(invalid("positive limits required"));
        }
        let roots = Roots::open(repo)?;
        plan_location(path, repo)?;
        let (mut reader, id) = open_plan(path)?;
        let mut c = admission(&mut reader, &id, &roots, args.cursor.as_deref())?;
        // Stable lock inode is intentionally retained. Unlinking flock lock files
        // would permit a third process to enter while a waiter holds the old inode.
        let maintenance = roots.cache.private_dir("sidecar-compaction")?;
        let _lock = maintenance.lock_file("apply.lock")?;
        let stage = maintenance.private_dir(&c.plan_digest[7..])?;
        reader
            .seek(SeekFrom::Start(c.next_byte_offset))
            .map_err(io_error)?;
        let mut report = json!({"schema":REPORT_SCHEMA,"mode":"APPLY","planDigest":c.plan_digest,"durability":"PROCESS_INTERRUPTION_ONLY","visited":0,"removed":0,"alreadyAbsent":0,"skipped":0,"actualRemovedLogicalBytes":0,"allocatedBytesEstimate":0,"validatedPayloadBytes":0,"stagedRetained":0,"skipReasons":[],"nextCursor":null});
        let mut budget = 0;
        while c.next_record_index < c.entry_count
            && report["visited"].as_u64().unwrap() < args.limit
        {
            let b = read_line(&mut reader)?;
            let e: PlanEntry = parse(&b)?;
            validate_entry(&e, c.next_record_index, c.next_byte_offset)?;
            // Two metadata reads cover original + captured, including recovery.
            let required = e
                .payload_size
                .saturating_add(1)
                .saturating_add(e.logical_bytes.saturating_add(1).saturating_mul(2));
            if budget + required > args.max_bytes {
                report["requiredBytes"] = json!(required);
                break;
            }
            same_plan(&reader, &id)?;
            let out = process(&roots.objects, &stage, &e, |_| {})?;
            budget += required;
            add(&mut report, "visited", 1);
            add(&mut report, "validatedPayloadBytes", out.payload_bytes);
            match out.reason {
                "REMOVED" => {
                    add(&mut report, "removed", 1);
                    add(&mut report, "actualRemovedLogicalBytes", out.logical);
                    add(&mut report, "allocatedBytesEstimate", out.allocated);
                }
                "ALREADY_ABSENT" => add(&mut report, "alreadyAbsent", 1),
                _ => {
                    add(&mut report, "skipped", 1);
                    let reasons = report["skipReasons"].as_array_mut().unwrap();
                    if reasons.len() < 128 {
                        reasons.push(json!({"recordIndex":e.record_index,"reason":out.reason}));
                    }
                }
            }
            if out.retained {
                add(&mut report, "stagedRetained", 1);
                report["retainedStage"] = json!(format!(
                    ".codeclew/cache/sidecar-compaction/{}/{:020}.meta.json",
                    &c.plan_digest[7..],
                    e.record_index
                ));
                break;
            }
            c.next_record_index += 1;
            c.next_byte_offset += b.len() as u64;
        }
        same_plan(&reader, &id)?;
        report["nextCursor"] = if c.next_record_index < c.entry_count {
            json!(cursor_text(&c)?)
        } else {
            Value::Null
        };
        report["planBytesRead"] = json!(reader.get_ref().reads);
        Ok(report)
    }

    pub(super) fn run(args: CompactArgs) -> Result<Value, ClewError> {
        let repo = Repository::open(&args.root)?;
        if args.apply {
            if args.plan_output.is_some() {
                return Err(invalid("--plan-output conflicts with --apply"));
            }
            apply(&repo, &args)
        } else {
            if args.plan.is_some() || args.cursor.is_some() {
                return Err(invalid("--plan and --cursor require --apply"));
            }
            plan(
                &repo,
                args.plan_output
                    .as_deref()
                    .ok_or_else(|| invalid("--plan-output required for dry-run"))?,
            )
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::symlink;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        fn fixture() -> (tempfile::TempDir, Dir, Dir, PlanEntry, PathBuf) {
            let temp = tempfile::tempdir().unwrap();
            let root = Dir::open_root(temp.path()).unwrap();
            fs::create_dir(temp.path().join("objects")).unwrap();
            let objects = root.open_dir("objects").unwrap();
            let bytes = b"retained payload";
            let digest = canonical::hash_bytes(bytes);
            let path = temp.path().join("objects").join(&digest);
            fs::create_dir(&path).unwrap();
            fs::write(path.join("object.json"), bytes).unwrap();
            fs::write(
                path.join("meta.json"),
                serde_json::to_vec(&json!({"schema":"fixture","size":bytes.len()})).unwrap(),
            )
            .unwrap();
            let d = objects.open_dir(&digest).unwrap();
            let m = read_meta(&d, "meta.json", MAX_META).unwrap();
            let e = PlanEntry {
                kind: "ENTRY".into(),
                record_index: 0,
                relative: format!("objects/{digest}/meta.json"),
                digest,
                payload_size: bytes.len() as u64,
                metadata_digest: canonical::hash_bytes(&m.raw),
                metadata: m.record,
                meta_identity: m.identity,
                object_dir_identity: identity(&d.metadata().unwrap()),
                logical_bytes: m.raw.len() as u64,
                allocated_bytes: m.allocated,
                byte_offset: 100,
            };
            let stage = root.private_dir("stage").unwrap();
            (temp, objects, stage, e, path)
        }
        #[test]
        fn replacement_between_validation_and_capture_is_retained() {
            let (t, o, s, e, p) = fixture();
            let original = fs::read(p.join("meta.json")).unwrap();
            let out = process(&o, &s, &e, |phase| {
                if let Phase::BeforeCapture = phase {
                    fs::write(p.join("new"), &original).unwrap();
                    fs::rename(p.join("new"), p.join("meta.json")).unwrap();
                }
            })
            .unwrap();
            assert!(out.retained);
            assert_eq!(out.reason, "CHANGED_CAPTURE_RETAINED");
            assert_eq!(
                fs::read(t.path().join("stage/00000000000000000000.meta.json")).unwrap(),
                original
            );
            assert_eq!(
                fs::read(p.join("object.json")).unwrap(),
                b"retained payload"
            );
        }
        #[test]
        fn recreated_original_after_capture_is_preserved() {
            let (_t, o, s, e, p) = fixture();
            let original = fs::read(p.join("meta.json")).unwrap();
            let out = process(&o, &s, &e, |phase| {
                if let Phase::AfterCapture = phase {
                    fs::write(p.join("meta.json"), &original).unwrap();
                }
            })
            .unwrap();
            assert_eq!(out.reason, "REMOVED");
            assert_eq!(fs::read(p.join("meta.json")).unwrap(), original);
        }
        #[test]
        fn interrupted_capture_recovers_without_recapturing_recreated_original() {
            let (_t, o, s, e, p) = fixture();
            let original = fs::read(p.join("meta.json")).unwrap();
            assert!(
                catch_unwind(AssertUnwindSafe(|| process(&o, &s, &e, |phase| {
                    if let Phase::AfterCapture = phase {
                        panic!("interrupted")
                    }
                })))
                .is_err()
            );
            fs::write(p.join("meta.json"), &original).unwrap();
            assert_eq!(process(&o, &s, &e, |_| {}).unwrap().reason, "REMOVED");
            assert_eq!(fs::read(p.join("meta.json")).unwrap(), original);
            assert_eq!(
                process(&o, &s, &e, |_| {}).unwrap().reason,
                "CHANGED_OR_RECREATED"
            );
        }
        #[test]
        fn interrupted_unlink_does_not_claim_previously_removed_bytes() {
            let (_t, o, s, e, _p) = fixture();
            assert!(
                catch_unwind(AssertUnwindSafe(|| process(&o, &s, &e, |phase| {
                    if let Phase::AfterUnlink = phase {
                        panic!("interrupted")
                    }
                })))
                .is_err()
            );
            let out = process(&o, &s, &e, |_| {}).unwrap();
            assert_eq!(out.reason, "ALREADY_ABSENT");
            assert_eq!(out.logical, 0);
        }
        #[test]
        fn dangling_staging_symlink_is_retained_and_private_dir_symlinks_rejected() {
            let (t, o, s, e, p) = fixture();
            let original = fs::read(p.join("meta.json")).unwrap();
            symlink(
                t.path().join("absent"),
                t.path().join("stage/00000000000000000000.meta.json"),
            )
            .unwrap();
            let out = process(&o, &s, &e, |_| {}).unwrap();
            assert!(out.retained);
            assert_eq!(fs::read(p.join("meta.json")).unwrap(), original);
            let root = Dir::open_root(t.path()).unwrap();
            symlink(t.path().join("stage"), t.path().join("alias")).unwrap();
            assert!(root.private_dir("alias").is_err());
        }
    }
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    let Command::CompactSidecars(args) = command;
    #[cfg(unix)]
    {
        platform::run(args)
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        Err(invalid(
            "sidecar compaction is unsupported on this platform",
        ))
    }
}
