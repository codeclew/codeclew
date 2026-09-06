//! Bounded, single-repository comparison of immutable saved source inputs.
//! Text deltas and compiler-projected declaration shapes have separate authority.

use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::ReadyGenerationSet;
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::session::SessionAuthority;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "codeclew-working-tree-change/1.0";
pub const MAX_REPORT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 512;
pub const MAX_DECLARATIONS: usize = 16_384;
pub const MAX_CHANGES: usize = 4_096;
pub const MAX_HUNKS_PER_FILE: usize = 64;
const MAX_PREVIEW_BYTES: usize = 2_048;
const MAX_DIFF_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceFile {
    pub content: CasObject,
    pub executable: bool,
}

/// Stage-zero bytes are a fallback only when the snapshot has no worktree
/// override. An explicit deletion must never revive the indexed version.
pub fn source_files(snapshot: &RepositoryInputSnapshot) -> BTreeMap<String, SourceFile> {
    let mut files = snapshot
        .index
        .iter()
        .filter(|e| e.stage == 0)
        .map(|entry| {
            (
                entry.path.clone(),
                SourceFile {
                    content: entry.content.clone(),
                    executable: entry.mode & 0o111 != 0,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for entry in &snapshot.worktree {
        files.remove(&entry.path);
        if entry.kind == WorktreeKind::Regular
            && let Some(content) = &entry.content
        {
            files.insert(
                entry.path.clone(),
                SourceFile {
                    content: content.clone(),
                    executable: entry.mode & 0o111 != 0,
                },
            );
        }
    }
    files
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAnchor {
    pub file: String,
    pub content: CasObject,
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub end_line: usize,
}

impl SourceAnchor {
    pub fn text(&self, store: &CasStore) -> Result<String, ClewError> {
        let lease = store.read(
            &self.content,
            crate::repository_snapshot::WorkingTreeLimits::default().max_file_bytes as usize,
        )?;
        let text =
            std::str::from_utf8(lease.bytes()).map_err(|_| invalid("source is not UTF-8"))?;
        text.get(self.start..self.end)
            .map(str::to_owned)
            .ok_or_else(|| invalid("source anchor is not an exact UTF-8 range"))
    }
}

#[derive(Default)]
pub(crate) struct SourceCache {
    bytes: usize,
    sources: BTreeMap<String, (CasObject, String, Vec<usize>)>,
}

impl SourceCache {
    fn source(
        &mut self,
        store: &CasStore,
        object: &CasObject,
    ) -> Result<&(CasObject, String, Vec<usize>), ClewError> {
        if !self.sources.contains_key(&object.digest) {
            let lease = store.read(
                object,
                crate::repository_snapshot::WorkingTreeLimits::default().max_file_bytes as usize,
            )?;
            let text = std::str::from_utf8(lease.bytes())
                .map_err(|_| invalid("declaration source is not UTF-8"))?
                .to_owned();
            let mut lines = vec![0];
            for (offset, byte) in text.bytes().enumerate() {
                if byte == b'\n' {
                    if lines.len() == 262_144 {
                        return Err(ClewError::new(
                            ErrorCode::ResourceLimit,
                            "comparison source exceeds line-index budget",
                        ));
                    }
                    lines.push(offset + 1);
                }
            }
            let size = text.len() + lines.len() * std::mem::size_of::<usize>();
            if self.bytes + size > 64 * 1024 * 1024 {
                self.sources.clear();
                self.bytes = 0;
            }
            self.bytes += size;
            self.sources
                .insert(object.digest.clone(), (object.clone(), text, lines));
        }
        let source = self.sources.get(&object.digest).unwrap();
        if &source.0 != object {
            return Err(invalid("source digest repeats with conflicting metadata"));
        }
        Ok(source)
    }

    pub(crate) fn anchor(
        &mut self,
        store: &CasStore,
        file: &str,
        object: &CasObject,
        start: usize,
        end: usize,
    ) -> Result<SourceAnchor, ClewError> {
        let (_, text, lines) = self.source(store, object)?;
        if text.get(start..end).is_none() {
            return Err(invalid("declaration range is not valid UTF-8"));
        }
        Ok(SourceAnchor {
            file: file.into(),
            content: object.clone(),
            start,
            end,
            start_line: lines.partition_point(|offset| *offset <= start),
            end_line: lines.partition_point(|offset| *offset <= end),
        })
    }

    fn text(&mut self, store: &CasStore, anchor: &SourceAnchor) -> Result<String, ClewError> {
        let (_, text, _) = self.source(store, &anchor.content)?;
        text.get(anchor.start..anchor.end)
            .map(str::to_owned)
            .ok_or_else(|| invalid("declaration range is not valid UTF-8"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declaration {
    pub compilation: String,
    pub symbol: String,
    pub family: Option<String>,
    pub kind: String,
    pub source: SourceAnchor,
    pub fact_key: String,
    pub payload: CasObject,
    pub projected_shape: Value,
    pub authority: String,
    pub complete_shape: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Analysis {
    pub status: String,
    pub authority: String,
    pub ready: Option<ReadyGenerationSet>,
    pub declarations: Vec<Declaration>,
    pub declaration_coverage_complete: bool,
    pub boundaries: Vec<Value>,
    pub failure: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Side {
    pub profile_id: String,
    pub session: SessionAuthority,
    pub snapshot: CasObject,
    pub analysis: Analysis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hunk {
    pub before: Option<SourceAnchor>,
    pub after: Option<SourceAnchor>,
    pub before_text: String,
    pub after_text: String,
    pub preview_truncated: bool,
    /// COARSE_REPLACEMENT is exact but may include unchanged intervening lines.
    pub granularity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileChange {
    pub file: String,
    pub kind: String,
    pub before: Option<SourceFile>,
    pub after: Option<SourceFile>,
    pub text_changed: bool,
    pub binary: bool,
    pub scope: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarationChange {
    pub change_id: String,
    pub correspondence: String,
    pub changes: Vec<String>,
    pub before: Option<Declaration>,
    pub after: Option<Declaration>,
    pub changed_shape_fields: Vec<String>,
    pub source_text_changed: bool,
    pub behavioral_equivalence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Comparison {
    pub schema: String,
    pub comparison_id: String,
    pub status: String,
    pub before: Side,
    pub after: Side,
    pub files: Vec<FileChange>,
    pub declarations: Vec<DeclarationChange>,
    pub unchanged_declaration_count: usize,
    pub total_changed_file_count: usize,
    pub total_changed_declaration_count: usize,
    pub omitted_file_count: usize,
    pub omitted_declaration_count: usize,
    pub comparability: String,
    pub obligations: Vec<String>,
    pub tests_executed: bool,
}

impl Comparison {
    pub fn seal(&mut self) -> Result<(), ClewError> {
        self.comparison_id.clear();
        self.comparison_id = format!("comparison:{}", canonical::hash(self).map_err(internal)?);
        Ok(())
    }

    pub fn verify(&self) -> Result<(), ClewError> {
        let mut unsigned = self.clone();
        unsigned.seal()?;
        if self.schema != SCHEMA
            || unsigned.comparison_id != self.comparison_id
            || self.before.session.repository_key != self.after.session.repository_key
            || self.before.session.base_revision != self.after.session.base_revision
            || self.before.session.runtime_key != self.after.session.runtime_key
            || self.before.session.compilations != self.after.session.compilations
            || self.before.session.language != self.after.session.language
            || self.before.profile_id != self.after.profile_id
            || self
                .after
                .session
                .working_tree
                .as_ref()
                .map(|b| &b.profile_id)
                != Some(&self.after.profile_id)
            || self.before.session.working_tree.is_some()
            || self
                .after
                .session
                .working_tree
                .as_ref()
                .map(|b| &b.snapshot)
                != Some(&self.after.snapshot)
            || self.tests_executed
        {
            return Err(invalid("working-tree comparison authority is invalid"));
        }
        Ok(())
    }
}

pub fn compare(
    store: &CasStore,
    before: Side,
    after: Side,
    before_snapshot: &RepositoryInputSnapshot,
    after_snapshot: &RepositoryInputSnapshot,
) -> Result<Comparison, ClewError> {
    let before_files = source_files(before_snapshot);
    let after_files = source_files(after_snapshot);
    let paths = before_files
        .keys()
        .chain(after_files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut files = Vec::new();
    let mut total_files: usize = 0;
    let mut diff_cells = MAX_DIFF_CELLS;
    let mut build_changed = false;
    for file in paths {
        let old = before_files.get(&file);
        let new = after_files.get(&file);
        if old.map(|f| (&f.content, f.executable)) == new.map(|f| (&f.content, f.executable)) {
            continue;
        }
        total_files += 1;
        build_changed |= is_build_input(&file);
        if files.len() == MAX_FILES {
            continue;
        }
        let old_bytes = old
            .map(|f| read_source(store, &f.content))
            .transpose()?
            .unwrap_or_default();
        let new_bytes = new
            .map(|f| read_source(store, &f.content))
            .transpose()?
            .unwrap_or_default();
        let text_changed = old_bytes != new_bytes;
        let binary = old_bytes.contains(&0)
            || new_bytes.contains(&0)
            || std::str::from_utf8(&old_bytes).is_err()
            || std::str::from_utf8(&new_bytes).is_err();
        let in_scope = before
            .analysis
            .declarations
            .iter()
            .chain(&after.analysis.declarations)
            .any(|d| d.source.file == file);
        let scope = if is_build_input(&file) {
            "BUILD_INPUT"
        } else if in_scope {
            "SELECTED_DECLARATION_SOURCE"
        } else {
            "OUTSIDE_AVAILABLE_DECLARATION_EVIDENCE"
        };
        let hunks = if text_changed && !binary {
            text_hunks(
                &file,
                old,
                new,
                std::str::from_utf8(&old_bytes).unwrap(),
                std::str::from_utf8(&new_bytes).unwrap(),
                &mut diff_cells,
            )?
        } else {
            vec![]
        };
        files.push(FileChange {
            file,
            kind: if old.is_none() {
                "ADDED"
            } else if new.is_none() {
                "DELETED"
            } else if text_changed {
                "MODIFIED"
            } else {
                "MODE_CHANGED"
            }
            .into(),
            before: old.cloned(),
            after: new.cloned(),
            text_changed,
            binary,
            scope: scope.into(),
            hunks,
        });
    }
    let (declarations, unchanged) = compare_declarations(
        store,
        &before.analysis,
        &after.analysis,
        &before_files,
        &after_files,
    )?;
    let total_declarations = declarations.len();
    let complete = before.analysis.status == "AVAILABLE" && after.analysis.status == "AVAILABLE";
    let mut report = Comparison {
        schema: SCHEMA.into(),
        comparison_id: String::new(),
        status: if complete {
            "BOUNDED_COMPARISON"
        } else {
            "INCOMPLETE"
        }
        .into(),
        before,
        after,
        omitted_file_count: total_files.saturating_sub(files.len()),
        omitted_declaration_count: total_declarations.saturating_sub(MAX_CHANGES),
        files,
        declarations: declarations.into_iter().take(MAX_CHANGES).collect(),
        total_changed_file_count: total_files,
        total_changed_declaration_count: total_declarations,
        unchanged_declaration_count: unchanged,
        comparability: if !complete {
            "ANALYSIS_SIDE_UNAVAILABLE"
        } else if build_changed {
            "BUILD_INPUTS_CHANGED_SEPARATELY_BOUND_MODELS_COMPARE_WITH_BOUNDARIES"
        } else {
            "SAME_SCOPE_PROFILE_RUNTIME"
        }
        .into(),
        obligations: vec![
            "TEXT_CHANGES_DO_NOT_PROVE_BEHAVIORAL_CHANGES".into(),
            "UNCHANGED_PROJECTED_SHAPE_DOES_NOT_PROVE_UNCHANGED_BEHAVIOR".into(),
            "ONLY_SELECTED_COMPILATIONS_ARE_ANALYZED".into(),
            "TESTS_WERE_NOT_EXECUTED".into(),
            "UNMATCHED_FILE_RENAMES_REMAIN_ADDITIONS_AND_DELETIONS".into(),
        ],
        tests_executed: false,
    };
    if report.omitted_file_count > 0 || report.omitted_declaration_count > 0 {
        report
            .obligations
            .push("COMPARISON_ROWS_OMITTED_BY_BUDGET".into());
    }
    if build_changed {
        report
            .obligations
            .push("VERIFY_CHANGED_BUILD_MODEL_AND_DEPENDENCY_ASSUMPTIONS".into());
    }
    if !report.before.analysis.declaration_coverage_complete
        || !report.after.analysis.declaration_coverage_complete
    {
        report
            .obligations
            .push("DECLARATION_COVERAGE_IS_PARTIAL".into());
    }
    report.seal()?;
    report.verify()?;
    Ok(report)
}

fn compare_declarations(
    store: &CasStore,
    before: &Analysis,
    after: &Analysis,
    before_files: &BTreeMap<String, SourceFile>,
    after_files: &BTreeMap<String, SourceFile>,
) -> Result<(Vec<DeclarationChange>, usize), ClewError> {
    let key = |d: &Declaration| (d.compilation.clone(), d.symbol.clone());
    let mut old = before
        .declarations
        .iter()
        .map(|d| (key(d), d))
        .collect::<BTreeMap<_, _>>();
    let mut new = after
        .declarations
        .iter()
        .map(|d| (key(d), d))
        .collect::<BTreeMap<_, _>>();
    if old.len() != before.declarations.len() || new.len() != after.declarations.len() {
        return Err(invalid(
            "comparison repeats a declaration identity in one compilation",
        ));
    }
    let mut pairs = Vec::new();
    for k in old.keys().cloned().collect::<Vec<_>>() {
        if let Some(after) = new.remove(&k) {
            pairs.push((old.remove(&k), Some(after), "EXACT_SYMBOL_AND_COMPILATION"));
        }
    }
    // A compiler callable family identifies a qualified declaration name, not
    // a fuzzy textual similarity. Multiple unmatched overloads remain unresolved.
    let group = |items: &BTreeMap<(String, String), &Declaration>| {
        let mut groups = BTreeMap::<(String, String), Vec<(String, String)>>::new();
        for (key, declaration) in items {
            if let Some(family) = &declaration.family {
                groups
                    .entry((declaration.compilation.clone(), family.clone()))
                    .or_default()
                    .push(key.clone());
            }
        }
        groups
    };
    let left_groups = group(&old);
    let right_groups = group(&new);
    for ((compilation, family), left) in left_groups {
        let Some(right) = right_groups.get(&(compilation, family.clone())) else {
            continue;
        };
        if left.len() == 1 && right.len() == 1 {
            pairs.push((
                old.remove(&left[0]),
                new.remove(&right[0]),
                if family.starts_with("syntax:") {
                    "UNIQUE_SYNTAX_NAME_IN_SAME_FILE_NOT_RENAME_PROOF"
                } else {
                    "SINGLE_REMAINING_DECLARATION_IN_COMPILER_CALLABLE_FAMILY"
                },
            ));
        }
    }
    pairs.extend(
        old.into_values()
            .map(|d| (Some(d), None, "UNMATCHED_BEFORE_DECLARATION")),
    );
    pairs.extend(
        new.into_values()
            .map(|d| (None, Some(d), "UNMATCHED_AFTER_DECLARATION")),
    );
    let mut cache = SourceCache::default();
    let mut changes = Vec::new();
    let mut unchanged = 0;
    for (old, new, correspondence) in pairs {
        let old_text = old.map(|d| cache.text(store, &d.source)).transpose()?;
        let new_text = new.map(|d| cache.text(store, &d.source)).transpose()?;
        let source_text_changed = old_text != new_text;
        let mut kinds = Vec::new();
        let mut fields = Vec::new();
        match (old, new) {
            (None, Some(d)) => kinds.push(
                if before.declaration_coverage_complete
                    || !before_files.contains_key(&d.source.file)
                {
                    "ADDED"
                } else {
                    "BEFORE_DECLARATION_UNRESOLVED"
                }
                .into(),
            ),
            (Some(d), None) => kinds.push(
                if after.declaration_coverage_complete || !after_files.contains_key(&d.source.file)
                {
                    "REMOVED"
                } else {
                    "AFTER_DECLARATION_UNRESOLVED"
                }
                .into(),
            ),
            (Some(a), Some(b)) => {
                fields = shape_fields(&a.projected_shape, &b.projected_shape);
                if !fields.is_empty() {
                    kinds.push(
                        if a.complete_shape && b.complete_shape {
                            "PROJECTED_SHAPE_CHANGED"
                        } else {
                            "PARTIAL_PROJECTED_SHAPE_CHANGED"
                        }
                        .into(),
                    );
                }
                if source_text_changed {
                    kinds.push("DECLARATION_SOURCE_TEXT_CHANGED".into());
                }
                if a.source.file != b.source.file {
                    kinds.push("DECLARATION_SOURCE_MOVED".into());
                }
            }
            _ => unreachable!(),
        }
        if kinds.is_empty() {
            unchanged += 1;
            continue;
        }
        let mut change = DeclarationChange {
            change_id: String::new(),
            correspondence: correspondence.into(),
            changes: kinds,
            before: old.cloned(),
            after: new.cloned(),
            changed_shape_fields: fields,
            source_text_changed,
            behavioral_equivalence: "NOT_PROVEN_BY_THIS_COMPARISON".into(),
        };
        change.change_id = canonical::hash(&change).map_err(internal)?;
        changes.push(change);
    }
    changes.sort_by(|a, b| a.change_id.cmp(&b.change_id));
    Ok((changes, unchanged))
}

fn shape_fields(before: &Value, after: &Value) -> Vec<String> {
    let keys = before
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
        .chain(after.as_object().into_iter().flatten().map(|(k, _)| k))
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .filter(|k| before.get(*k) != after.get(*k))
        .cloned()
        .collect()
}

pub fn is_build_input(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    matches!(
        name,
        "Cargo.toml" | "Cargo.lock" | "pom.xml" | "gradle.properties" | "gradlew" | "gradlew.bat"
    ) || name.ends_with(".gradle")
        || name.ends_with(".gradle.kts")
        || file.starts_with("gradle/")
        || file.starts_with("buildSrc/")
        || file.starts_with("build-logic/")
}

fn read_source(store: &CasStore, object: &CasObject) -> Result<Vec<u8>, ClewError> {
    Ok(store
        .read(
            object,
            crate::repository_snapshot::WorkingTreeLimits::default().max_file_bytes as usize,
        )?
        .bytes()
        .to_vec())
}

fn preview(text: &str) -> (String, bool) {
    let mut end = text.len().min(MAX_PREVIEW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), end < text.len())
}

fn text_hunks(
    file: &str,
    old: Option<&SourceFile>,
    new: Option<&SourceFile>,
    before: &str,
    after: &str,
    budget: &mut usize,
) -> Result<Vec<Hunk>, ClewError> {
    // Count before allocating per-line slices: a newline-heavy source must not
    // amplify a bounded file into hundreds of MiB of diff scratch space.
    if before.bytes().filter(|b| *b == b'\n').count() > 262_144
        || after.bytes().filter(|b| *b == b'\n').count() > 262_144
    {
        let anchor = |source: &SourceFile, text: &str| SourceAnchor {
            file: file.into(),
            content: source.content.clone(),
            start: 0,
            end: text.len(),
            start_line: 1,
            end_line: text.bytes().filter(|b| *b == b'\n').count() + 1,
        };
        let (before_text, bt) = preview(before);
        let (after_text, at) = preview(after);
        return Ok(vec![Hunk {
            before: old.map(|s| anchor(s, before)),
            after: new.map(|s| anchor(s, after)),
            before_text,
            after_text,
            preview_truncated: bt || at,
            granularity: "COARSE_REPLACEMENT".into(),
        }]);
    }
    let a = before.split_inclusive('\n').collect::<Vec<_>>();
    let b = after.split_inclusive('\n').collect::<Vec<_>>();
    let offsets = |lines: &[&str]| {
        let mut out = vec![0];
        for line in lines {
            out.push(out.last().unwrap() + line.len());
        }
        out
    };
    let ao = offsets(&a);
    let bo = offsets(&b);
    let cells = (a.len() + 1).saturating_mul(b.len() + 1);
    let mut coarse = cells > (*budget).min(1_000_000);
    let mut ranges = Vec::new();
    if !coarse {
        *budget -= cells;
        let width = b.len() + 1;
        let mut lcs = vec![0u32; cells];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                lcs[i * width + j] = if a[i] == b[j] {
                    lcs[(i + 1) * width + j + 1] + 1
                } else {
                    lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                i += 1;
                j += 1;
                continue;
            }
            let (start_i, start_j) = (i, j);
            while i < a.len() || j < b.len() {
                if i < a.len() && j < b.len() && a[i] == b[j] {
                    break;
                }
                if j < b.len()
                    && (i == a.len() || lcs[i * width + j + 1] > lcs[(i + 1) * width + j])
                {
                    j += 1;
                } else {
                    i += 1;
                }
            }
            ranges.push((start_i, i, start_j, j));
        }
        coarse = ranges.len() > MAX_HUNKS_PER_FILE;
    }
    if coarse {
        let mut prefix = 0;
        while prefix < a.len().min(b.len()) && a[prefix] == b[prefix] {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < a.len().min(b.len()) - prefix
            && a[a.len() - suffix - 1] == b[b.len() - suffix - 1]
        {
            suffix += 1;
        }
        ranges = vec![(prefix, a.len() - suffix, prefix, b.len() - suffix)];
    }
    Ok(ranges
        .into_iter()
        .map(|(ai, aj, bi, bj)| {
            let anchor =
                |source: &SourceFile, start: usize, end: usize, offsets: &[usize]| SourceAnchor {
                    file: file.into(),
                    content: source.content.clone(),
                    start: offsets[start],
                    end: offsets[end],
                    start_line: start + 1,
                    end_line: end.max(start + 1),
                };
            let (before_text, bt) = preview(&before[ao[ai]..ao[aj]]);
            let (after_text, at) = preview(&after[bo[bi]..bo[bj]]);
            Hunk {
                before: old.map(|s| anchor(s, ai, aj, &ao)),
                after: new.map(|s| anchor(s, bi, bj, &bo)),
                before_text,
                after_text,
                preview_truncated: bt || at,
                granularity: if coarse {
                    "COARSE_REPLACEMENT"
                } else {
                    "EXACT_LINE_DIFF"
                }
                .into(),
            }
        })
        .collect())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}
fn internal(message: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateAuthority;
    use serde_json::json;

    #[test]
    fn exact_and_coarse_hunks_reconstruct_saved_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        for (before, after) in [
            ("a\nb\nc\nd\n", "a\nB\nc\nD\n"),
            ("", "new\n"),
            ("deleted\n", ""),
            ("λ no newline", "λ with newline\n"),
        ] {
            let old = SourceFile {
                content: store.put("test/source/1", before.as_bytes()).unwrap(),
                executable: false,
            };
            let new = SourceFile {
                content: store.put("test/source/1", after.as_bytes()).unwrap(),
                executable: false,
            };
            for mut budget in [MAX_DIFF_CELLS, 0] {
                let hunks =
                    text_hunks("A.kt", Some(&old), Some(&new), before, after, &mut budget).unwrap();
                let mut result = before.to_owned();
                for hunk in hunks.iter().rev() {
                    assert!(!hunk.preview_truncated);
                    let old = hunk.before.as_ref().unwrap();
                    assert_eq!(old.text(&store).unwrap(), hunk.before_text);
                    assert_eq!(
                        hunk.after.as_ref().unwrap().text(&store).unwrap(),
                        hunk.after_text
                    );
                    result.replace_range(old.start..old.end, &hunk.after_text);
                }
                assert_eq!(result, after);
            }
        }
    }

    #[test]
    fn newline_heavy_diff_uses_bounded_coarse_evidence() {
        let text = "\n".repeat(262_145);
        let mut budget = MAX_DIFF_CELLS;
        let hunks = text_hunks("large.txt", None, None, &text, "replacement", &mut budget).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].granularity, "COARSE_REPLACEMENT");
        assert!(hunks[0].preview_truncated);
        assert_eq!(budget, MAX_DIFF_CELLS);
    }

    #[test]
    fn unchanged_signature_keeps_body_edits_and_failed_after_is_not_a_removal() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let declaration = |text: &str| {
            let content = store.put("test/source/1", text.as_bytes()).unwrap();
            Declaration {
                compilation: ":/main".into(),
                symbol: "callable:p/price#jvm:()I".into(),
                family: Some("p/price".into()),
                kind: "FUNCTION".into(),
                source: SourceAnchor {
                    file: "A.kt".into(),
                    content,
                    start: 0,
                    end: text.len(),
                    start_line: 1,
                    end_line: 1,
                },
                fact_key: "descriptor:price".into(),
                payload: store.put("test/fact/1", b"{}").unwrap(),
                projected_shape: json!({"returnType":"kotlin/Int"}),
                authority: "COMPILER_PROJECTED_DECLARATION".into(),
                complete_shape: true,
            }
        };
        let side = |declaration| Analysis {
            status: "AVAILABLE".into(),
            authority: "KOTLIN_COMPILER_SUPPORTED_SUBSET".into(),
            ready: None,
            declarations: vec![declaration],
            declaration_coverage_complete: true,
            boundaries: vec![],
            failure: None,
        };
        let before = side(declaration("fun price() = 1"));
        let after = side(declaration("fun price() = 2"));
        let files = BTreeMap::from([(
            "A.kt".into(),
            SourceFile {
                content: before.declarations[0].source.content.clone(),
                executable: false,
            },
        )]);
        let (changes, unchanged) =
            compare_declarations(&store, &before, &after, &files, &files).unwrap();
        assert_eq!(unchanged, 0);
        assert_eq!(changes[0].changes, ["DECLARATION_SOURCE_TEXT_CHANGED"]);
        assert!(changes[0].changed_shape_fields.is_empty());
        assert_eq!(
            changes[0].behavioral_equivalence,
            "NOT_PROVEN_BY_THIS_COMPARISON"
        );
        let failed = Analysis {
            status: "FAILED".into(),
            declarations: vec![],
            declaration_coverage_complete: false,
            ..after
        };
        let (changes, _) = compare_declarations(&store, &before, &failed, &files, &files).unwrap();
        assert_eq!(changes[0].changes, ["AFTER_DECLARATION_UNRESOLVED"]);
    }
}
