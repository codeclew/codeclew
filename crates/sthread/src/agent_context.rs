use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_DECLARATIONS: usize = 12;
const MAX_REFERENCES: usize = 24;
const MAX_TEST_FILES: usize = 12;
const MAX_SOURCE_BYTES_PER_DECLARATION: usize = 2_400;

#[derive(Clone, Debug)]
struct SourceFile {
    path: String,
    source: String,
    is_test: bool,
}

#[derive(Clone, Debug)]
struct SelectedDeclaration {
    declaration: Value,
    matched_terms: BTreeSet<String>,
    match_kinds: BTreeSet<String>,
    source_text: String,
    line_start: usize,
    line_end: usize,
}

#[derive(Clone, Debug)]
pub struct AgentContextSelection {
    files: Vec<SourceFile>,
    declarations: Vec<SelectedDeclaration>,
}

impl AgentContextSelection {
    pub fn function_symbols(&self) -> Vec<String> {
        self.declarations
            .iter()
            .filter(|selected| {
                selected.declaration["kind"]
                    .as_str()
                    .is_some_and(|kind| kind.contains("Function"))
            })
            .filter_map(|selected| selected.declaration["legacySymbolId"].as_str())
            .map(str::to_owned)
            .collect()
    }

    fn reference_symbols(&self) -> BTreeSet<String> {
        self.declarations
            .iter()
            .filter(|selected| {
                !selected.match_kinds.contains("semantic-dependency")
                    && (selected.match_kinds.len() > 1
                        || !selected.match_kinds.contains("file-alias"))
            })
            .filter_map(|selected| selected.declaration["name"].as_str())
            .map(str::to_owned)
            .collect()
    }

    fn dependency_symbols(&self) -> BTreeSet<String> {
        self.declarations
            .iter()
            .filter(|selected| selected.match_kinds.contains("semantic-dependency"))
            .filter_map(|selected| selected.declaration["name"].as_str())
            .map(str::to_owned)
            .collect()
    }
}

pub fn select(
    repo: &Path,
    index_facts: &Value,
    terms: &[String],
) -> Result<AgentContextSelection, SthreadError> {
    let files = scan_kotlin_sources(repo)?;
    let sources = files
        .iter()
        .map(|file| (file.path.as_str(), file.source.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut selected = BTreeMap::<String, SelectedDeclaration>::new();

    for file in index_facts["files"].as_array().into_iter().flatten() {
        let path = file["path"].as_str().unwrap_or_default();
        let Some(source) = sources.get(path) else {
            continue;
        };
        let file_stem = Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        for declaration in file["declarations"].as_array().into_iter().flatten() {
            let name = declaration["name"].as_str().unwrap_or_default();
            let legacy = declaration["legacySymbolId"].as_str().unwrap_or_default();
            let mut matches = Vec::new();
            for term in terms {
                let kind = match_kind(term, name, legacy, file_stem);
                if let Some(kind) = kind {
                    matches.push((term.clone(), kind.to_owned()));
                }
            }
            if matches.is_empty() {
                continue;
            }
            let id = declaration["declarationId"]
                .as_str()
                .unwrap_or(legacy)
                .to_owned();
            let entry = selected.entry(id).or_insert_with(|| {
                let start = declaration["rangeStart"].as_u64().unwrap_or_default() as usize;
                let end = declaration["rangeEnd"].as_u64().unwrap_or_default() as usize;
                let (byte_start, byte_end) = utf16_range_to_bytes(source, start, end);
                let source_text = source
                    .get(byte_start..byte_end)
                    .unwrap_or_default()
                    .to_owned();
                SelectedDeclaration {
                    declaration: declaration.clone(),
                    matched_terms: BTreeSet::new(),
                    match_kinds: BTreeSet::new(),
                    source_text,
                    line_start: line_number(source, byte_start),
                    line_end: line_number(source, byte_end),
                }
            });
            for (term, kind) in matches {
                entry.matched_terms.insert(term);
                entry.match_kinds.insert(kind);
            }
        }
    }

    let dependency_names = selected
        .values()
        .flat_map(|selected| declaration_type_names(&selected.declaration))
        .collect::<BTreeSet<_>>();
    for file in index_facts["files"].as_array().into_iter().flatten() {
        let path = file["path"].as_str().unwrap_or_default();
        let Some(source) = sources.get(path) else {
            continue;
        };
        for declaration in file["declarations"].as_array().into_iter().flatten() {
            let name = declaration["name"].as_str().unwrap_or_default();
            let kind = declaration["kind"].as_str().unwrap_or_default();
            if !kind.contains("Class") || !dependency_names.contains(name) {
                continue;
            }
            let id = declaration["declarationId"]
                .as_str()
                .unwrap_or(name)
                .to_owned();
            selected.entry(id).or_insert_with(|| {
                let start = declaration["rangeStart"].as_u64().unwrap_or_default() as usize;
                let end = declaration["rangeEnd"].as_u64().unwrap_or_default() as usize;
                let (byte_start, byte_end) = utf16_range_to_bytes(source, start, end);
                SelectedDeclaration {
                    declaration: declaration.clone(),
                    matched_terms: BTreeSet::new(),
                    match_kinds: BTreeSet::from(["semantic-dependency".to_owned()]),
                    source_text: source
                        .get(byte_start..byte_end)
                        .unwrap_or_default()
                        .to_owned(),
                    line_start: line_number(source, byte_start),
                    line_end: line_number(source, byte_end),
                }
            });
        }
    }

    let mut declarations = selected.into_values().collect::<Vec<_>>();
    declarations.sort_by_key(|selected| {
        (
            declaration_rank(selected),
            selected.declaration["file"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            selected.declaration["rangeStart"]
                .as_u64()
                .unwrap_or_default(),
        )
    });
    declarations.truncate(MAX_DECLARATIONS);
    Ok(AgentContextSelection {
        files,
        declarations,
    })
}

pub struct AgentContextBuild<'a> {
    pub repo: &'a Path,
    pub terms: &'a [String],
    pub compilation: &'a str,
    pub project: &'a Value,
    pub index_facts: &'a Value,
    pub selection: &'a AgentContextSelection,
    pub resolutions: &'a [Value],
    pub base_revision: &'a str,
    pub evidence_path: &'a Path,
    pub max_bytes: usize,
}

pub fn build(input: AgentContextBuild<'_>) -> Result<(Value, Value), SthreadError> {
    let AgentContextBuild {
        repo,
        terms,
        compilation,
        project,
        index_facts,
        selection,
        resolutions,
        base_revision,
        evidence_path,
        max_bytes,
    } = input;
    if max_bytes < 1_024 {
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "--max-bytes must be at least 1024",
        ));
    }
    let anchors = resolutions
        .iter()
        .filter_map(|resolution| {
            let id = resolution
                .pointer("/declaration/legacySymbolId")?
                .as_str()?;
            let anchor = resolution.get("bodyAnchor")?;
            Some((id.to_owned(), compact_anchor(anchor)))
        })
        .collect::<BTreeMap<_, _>>();

    let declarations = selection
        .declarations
        .iter()
        .map(|selected| {
            compact_declaration(
                selected,
                anchors.get(
                    selected.declaration["legacySymbolId"]
                        .as_str()
                        .unwrap_or_default(),
                ),
            )
        })
        .collect::<Vec<_>>();
    let matched_symbols = selection.reference_symbols();
    let references = collect_references(selection, terms, &matched_symbols);
    let tests = collect_tests(
        selection,
        terms,
        &matched_symbols,
        &selection.dependency_symbols(),
    );
    let file_headers = collect_file_headers(selection);
    let mut matched_terms = selection
        .declarations
        .iter()
        .flat_map(|selected| selected.matched_terms.iter().cloned())
        .collect::<BTreeSet<_>>();
    matched_terms.extend(
        terms
            .iter()
            .filter(|term| {
                selection
                    .files
                    .iter()
                    .any(|file| file.source.contains(term.as_str()))
            })
            .cloned(),
    );
    let unmatched_terms = terms
        .iter()
        .filter(|term| {
            !matched_terms.contains(*term)
                && !selection
                    .files
                    .iter()
                    .any(|file| file.source.contains(term.as_str()))
        })
        .cloned()
        .collect::<Vec<_>>();
    let test_tasks = project["testTasks"].as_array().cloned().unwrap_or_default();
    let build_system = project["buildSystem"].as_str().unwrap_or("GRADLE");
    let build_launcher = project["buildLauncher"]
        .as_str()
        .unwrap_or(match build_system {
            "MAVEN" => "mvn",
            _ => "./gradlew",
        });
    let targeted_tests = tests
        .iter()
        .filter_map(|test| test["path"].as_str())
        .filter_map(|path| Path::new(path).file_stem()?.to_str())
        .map(|stem| match build_system {
            "MAVEN" => format!("{build_launcher} -Dtest={stem} test"),
            _ => format!("{build_launcher} cleanTest --tests '*{stem}'"),
        })
        .collect::<BTreeSet<_>>();

    let full = json!({
        "schema": "semantic-agent-context-pack/0.1",
        "query": {
            "terms": terms,
            "matchedTerms": matched_terms,
            "unmatchedTerms": unmatched_terms,
        },
        "snapshot": {
            "baseRevision": base_revision,
            "projectModelHash": project["projectModelHash"],
            "indexHash": index_facts["indexHash"],
            "compilerVersion": project["compilerVersion"],
            "compilation": compilation,
        },
        "declarations": declarations,
        "fileHeaders": file_headers,
        "references": references,
        "tests": tests,
        "validationPlan": {
            "buildSystem": build_system,
            "buildLauncher": build_launcher,
            "compileTask": project["compileTask"],
            "testTasks": test_tasks,
            "targetedCommands": targeted_tests,
        },
        "evidence": evidence_display(repo, evidence_path),
        "completeness": {
            "status": "COMPLETE",
            "stdoutLimitBytes": max_bytes,
            "omitted": {"declarations": 0, "references": 0, "tests": 0, "sourceBytes": 0},
        },
    });
    let bounded = enforce_budget(full.clone(), max_bytes)?;
    let evidence = json!({
        "schema": "semantic-agent-context-evidence/0.1",
        "context": full,
        "project": project,
        "index": index_facts,
        "resolutions": resolutions,
    });
    Ok((bounded, evidence))
}

fn match_kind(term: &str, name: &str, legacy: &str, file_stem: &str) -> Option<&'static str> {
    if name == term || legacy == term {
        return Some("exact-symbol");
    }
    if legacy
        .substring_before_signature()
        .strip_suffix(term)
        .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('.'))
    {
        return Some("qualified-symbol");
    }
    if name.starts_with(term) {
        return Some("name-prefix");
    }
    if file_stem.eq_ignore_ascii_case(term) && (name.eq_ignore_ascii_case(term) || name == "main") {
        return Some("file-alias");
    }
    None
}

trait SymbolText {
    fn substring_before_signature(&self) -> &str;
}

impl SymbolText for str {
    fn substring_before_signature(&self) -> &str {
        self.split_once('(').map_or(self, |(prefix, _)| prefix)
    }
}

fn declaration_rank(selected: &SelectedDeclaration) -> usize {
    if selected.match_kinds.contains("exact-symbol") {
        0
    } else if selected.match_kinds.contains("qualified-symbol") {
        1
    } else if selected.match_kinds.contains("file-alias") {
        2
    } else if selected.match_kinds.contains("name-prefix") {
        3
    } else {
        4
    }
}

fn declaration_type_names(declaration: &Value) -> Vec<String> {
    ["receiverTypes", "parameterTypes"]
        .into_iter()
        .flat_map(|key| {
            declaration["symbolIdentity"][key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .chain(declaration["symbolIdentity"]["returnType"].as_str())
        .flat_map(|type_name| {
            type_name
                .split(|character: char| !is_kotlin_identifier_part(character))
                .filter(|part| part.chars().next().is_some_and(char::is_uppercase))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|name| {
            !matches!(
                name.as_str(),
                "Any"
                    | "Array"
                    | "Boolean"
                    | "Double"
                    | "Float"
                    | "Int"
                    | "Iterable"
                    | "List"
                    | "Long"
                    | "Map"
                    | "Nothing"
                    | "Number"
                    | "Sequence"
                    | "Set"
                    | "Short"
                    | "String"
                    | "Unit"
            )
        })
        .collect()
}

fn compact_declaration(selected: &SelectedDeclaration, anchor: Option<&Value>) -> Value {
    let declaration = &selected.declaration;
    let (source_text, source_truncated, omitted) =
        truncate_utf8(&selected.source_text, MAX_SOURCE_BYTES_PER_DECLARATION);
    let mut value = Map::from_iter([
        ("name".into(), declaration["name"].clone()),
        ("kind".into(), declaration["kind"].clone()),
        ("file".into(), declaration["file"].clone()),
        (
            "lines".into(),
            json!([selected.line_start, selected.line_end]),
        ),
        ("matchedTerms".into(), json!(selected.matched_terms)),
        ("matchKinds".into(), json!(selected.match_kinds)),
        ("sourceText".into(), json!(source_text)),
    ]);
    if source_truncated {
        value.insert("sourceTruncated".into(), json!(true));
        value.insert("sourceBytesOmitted".into(), json!(omitted));
    }
    if let Some(anchor) = anchor {
        value.insert("editAnchor".into(), anchor.clone());
    }
    Value::Object(value)
}

fn compact_anchor(anchor: &Value) -> Value {
    let mut compact = Map::new();
    for key in ["anchorId", "fileId", "rangeHint", "syntaxKind"] {
        if let Some(value) = anchor.get(key) {
            compact.insert(key.to_owned(), value.clone());
        }
    }
    Value::Object(compact)
}

fn collect_references(
    selection: &AgentContextSelection,
    terms: &[String],
    symbols: &BTreeSet<String>,
) -> Vec<Value> {
    let needles = terms
        .iter()
        .chain(symbols.iter())
        .filter(|needle| !needle.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    let selected_ranges = selection
        .declarations
        .iter()
        .filter_map(|selected| {
            Some((
                selected.declaration["file"].as_str()?.to_owned(),
                selected.line_start,
                selected.line_end,
            ))
        })
        .collect::<Vec<_>>();
    let mut references = Vec::new();
    for file in selection.files.iter().filter(|file| !file.is_test) {
        for (index, line) in file.source.lines().enumerate() {
            let line_number = index + 1;
            let in_selected_source = selected_ranges.iter().any(|(path, start, end)| {
                path == &file.path && line_number >= *start && line_number <= *end
            });
            if in_selected_source {
                continue;
            }
            let matches = needles
                .iter()
                .filter(|needle| contains_kotlin_name(line, needle))
                .cloned()
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            references.push(json!({
                "file": file.path,
                "line": line_number,
                "matched": matches,
                "text": truncate_line(line),
            }));
        }
    }
    references.truncate(MAX_REFERENCES);
    references
}

fn collect_tests(
    selection: &AgentContextSelection,
    terms: &[String],
    symbols: &BTreeSet<String>,
    dependency_symbols: &BTreeSet<String>,
) -> Vec<Value> {
    let needles = terms
        .iter()
        .chain(symbols.iter())
        .filter(|needle| !needle.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut tests = Vec::new();
    for file in selection.files.iter().filter(|file| file.is_test) {
        let mut matched = needles
            .iter()
            .filter(|needle| contains_kotlin_name(&file.source, needle))
            .cloned()
            .collect::<Vec<_>>();
        let file_stem = Path::new(&file.path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        matched.extend(
            dependency_symbols
                .iter()
                .filter(|symbol| common_prefix_len(file_stem, symbol) >= 8)
                .cloned(),
        );
        matched.sort();
        matched.dedup();
        if matched.is_empty() {
            continue;
        }
        let relevant_lines = file
            .source
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                matched
                    .iter()
                    .any(|needle| contains_kotlin_name(line, needle))
            })
            .take(4)
            .map(|(index, line)| json!({"line": index + 1, "text": truncate_line(line)}))
            .collect::<Vec<_>>();
        tests.push(json!({
            "path": file.path,
            "matched": matched,
            "relevantLines": relevant_lines,
            "sourceText": truncate_utf8(&file.source, 1_200).0,
        }));
    }
    tests.truncate(MAX_TEST_FILES);
    tests
}

fn collect_file_headers(selection: &AgentContextSelection) -> Vec<Value> {
    let selected_files = selection
        .declarations
        .iter()
        .filter_map(|selected| selected.declaration["file"].as_str())
        .collect::<BTreeSet<_>>();
    selection
        .files
        .iter()
        .filter(|file| selected_files.contains(file.path.as_str()))
        .filter_map(|file| {
            let header = file
                .source
                .lines()
                .take_while(|line| {
                    let trimmed = line.trim();
                    trimmed.is_empty()
                        || trimmed.starts_with("package ")
                        || trimmed.starts_with("import ")
                        || trimmed.starts_with("//")
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!header.is_empty()).then(|| json!({"file":file.path,"sourceText":header}))
        })
        .collect()
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left.eq_ignore_ascii_case(right))
        .count()
}

fn scan_kotlin_sources(repo: &Path) -> Result<Vec<SourceFile>, SthreadError> {
    let mut files = WalkDir::new(repo)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(".git" | ".gradle" | ".semantic-thread" | "build")
            )
        })
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("kt")
        })
        .map(|entry| {
            let path = entry.path();
            let relative = path
                .strip_prefix(repo)
                .map_err(|error| SthreadError::new(ErrorCode::Internal, error.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(path)
                .map_err(|error| SthreadError::new(ErrorCode::InvalidInput, error.to_string()))?;
            let is_test = relative.starts_with("src/test/") || relative.contains("/src/test/");
            Ok(SourceFile {
                path: relative,
                source,
                is_test,
            })
        })
        .collect::<Result<Vec<_>, SthreadError>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn utf16_range_to_bytes(source: &str, start: usize, end: usize) -> (usize, usize) {
    (
        utf16_offset_to_byte(source, start),
        utf16_offset_to_byte(source, end),
    )
}

fn utf16_offset_to_byte(source: &str, target: usize) -> usize {
    let mut units = 0;
    for (byte, character) in source.char_indices() {
        if units >= target {
            return byte;
        }
        units += character.len_utf16();
    }
    source.len()
}

fn line_number(source: &str, byte_offset: usize) -> usize {
    source
        .get(..byte_offset.min(source.len()))
        .unwrap_or_default()
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn truncate_line(line: &str) -> String {
    truncate_utf8(line.trim(), 240).0
}

fn contains_kotlin_name(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(needle).any(|(start, _)| {
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        let prefix = before.is_none_or(|character| !is_kotlin_identifier_part(character));
        let suffix = needle.ends_with('_')
            || after.is_none_or(|character| !is_kotlin_identifier_part(character));
        prefix && suffix
    })
}

fn is_kotlin_identifier_part(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool, usize) {
    if value.len() <= max_bytes {
        return (value.to_owned(), false, 0);
    }
    let mut end = max_bytes.saturating_sub(3).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}...", &value[..end]), true, value.len() - end)
}

fn evidence_display(repo: &Path, evidence_path: &Path) -> String {
    let absolute = if evidence_path.is_absolute() {
        PathBuf::from(evidence_path)
    } else {
        repo.join(evidence_path)
    };
    absolute
        .strip_prefix(repo)
        .unwrap_or(&absolute)
        .to_string_lossy()
        .replace('\\', "/")
}

fn enforce_budget(mut pack: Value, max_bytes: usize) -> Result<Value, SthreadError> {
    let original_declarations = array_len(&pack, "declarations");
    let original_references = array_len(&pack, "references");
    let original_tests = array_len(&pack, "tests");
    let mut source_bytes_omitted = 0usize;

    while serialized_len(&pack)? > max_bytes {
        if pop_array(&mut pack, "references") {
            continue;
        }
        if trim_test_line(&mut pack) {
            continue;
        }
        if pop_array(&mut pack, "tests") {
            continue;
        }
        if let Some(omitted) = trim_largest_source(&mut pack) {
            source_bytes_omitted += omitted;
            continue;
        }
        if array_len(&pack, "declarations") > 1 && pop_array(&mut pack, "declarations") {
            continue;
        }
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            format!("--max-bytes {max_bytes} is too small for the minimal context pack"),
        ));
    }

    update_omission_summary(
        &mut pack,
        original_declarations,
        original_references,
        original_tests,
        source_bytes_omitted,
    );
    while serialized_len(&pack)? > max_bytes {
        if pop_array(&mut pack, "references")
            || trim_test_line(&mut pack)
            || pop_array(&mut pack, "tests")
        {
            update_omission_summary(
                &mut pack,
                original_declarations,
                original_references,
                original_tests,
                source_bytes_omitted,
            );
            continue;
        }
        if let Some(omitted) = trim_largest_source(&mut pack) {
            source_bytes_omitted += omitted;
            update_omission_summary(
                &mut pack,
                original_declarations,
                original_references,
                original_tests,
                source_bytes_omitted,
            );
            continue;
        }
        if array_len(&pack, "declarations") > 1 && pop_array(&mut pack, "declarations") {
            update_omission_summary(
                &mut pack,
                original_declarations,
                original_references,
                original_tests,
                source_bytes_omitted,
            );
            continue;
        }
        return Err(SthreadError::new(
            ErrorCode::Internal,
            "context budget accounting failed",
        ));
    }
    Ok(pack)
}

fn update_omission_summary(
    pack: &mut Value,
    original_declarations: usize,
    original_references: usize,
    original_tests: usize,
    source_bytes_omitted: usize,
) {
    let omitted_declarations = original_declarations - array_len(pack, "declarations");
    let omitted_references = original_references - array_len(pack, "references");
    let omitted_tests = original_tests - array_len(pack, "tests");
    let partial =
        omitted_declarations + omitted_references + omitted_tests + source_bytes_omitted > 0;
    pack["completeness"]["status"] = json!(if partial {
        "PARTIAL_BUDGET"
    } else {
        "COMPLETE"
    });
    pack["completeness"]["omitted"] = json!({
        "declarations": omitted_declarations,
        "references": omitted_references,
        "tests": omitted_tests,
        "sourceBytes": source_bytes_omitted,
    });
}

fn serialized_len(value: &Value) -> Result<usize, SthreadError> {
    canonical::pretty(value)
        .map(|text| text.len() + 1)
        .map_err(|error| SthreadError::new(ErrorCode::Internal, error.to_string()))
}

fn array_len(pack: &Value, key: &str) -> usize {
    pack[key].as_array().map_or(0, Vec::len)
}

fn pop_array(pack: &mut Value, key: &str) -> bool {
    pack.get_mut(key)
        .and_then(Value::as_array_mut)
        .and_then(Vec::pop)
        .is_some()
}

fn trim_test_line(pack: &mut Value) -> bool {
    pack.get_mut("tests")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .rev()
        .find_map(|test| test.get_mut("relevantLines")?.as_array_mut())
        .and_then(Vec::pop)
        .is_some()
}

fn trim_largest_source(pack: &mut Value) -> Option<usize> {
    let declarations = pack.get_mut("declarations")?.as_array_mut()?;
    let (index, source_len) = declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| Some((index, declaration["sourceText"].as_str()?.len())))
        .filter(|(_, length)| *length > 384)
        .max_by_key(|(_, length)| *length)?;
    let new_limit = (source_len / 2).max(384);
    let declaration = declarations.get_mut(index)?;
    let source = declaration["sourceText"].as_str()?.to_owned();
    let (truncated, _, omitted) = truncate_utf8(&source, new_limit);
    declaration["sourceText"] = json!(truncated);
    declaration["sourceTruncated"] = json!(true);
    let previous = declaration["sourceBytesOmitted"]
        .as_u64()
        .unwrap_or_default() as usize;
    declaration["sourceBytesOmitted"] = json!(previous + omitted);
    Some(omitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_utf16_offsets_without_splitting_unicode() {
        let source = "fun привет() = \"🙂\"\n";
        let start = source
            .encode_utf16()
            .position(|unit| unit == 'п' as u16)
            .unwrap();
        let end = source.encode_utf16().count();
        let (byte_start, byte_end) = utf16_range_to_bytes(source, start, end);
        assert!(source.is_char_boundary(byte_start));
        assert!(source.is_char_boundary(byte_end));
        assert!(source[byte_start..byte_end].starts_with("привет"));
    }

    #[test]
    fn hard_budget_removes_low_priority_context_first() {
        let references = (0..8)
            .map(|_| json!({"text":"r".repeat(300)}))
            .collect::<Vec<_>>();
        let relevant_lines = (0..4)
            .map(|_| json!({"text":"t".repeat(300)}))
            .collect::<Vec<_>>();
        let pack = json!({
            "schema":"semantic-agent-context-pack/0.1",
            "declarations":[{"sourceText":"x".repeat(700), "name":"target"}],
            "references":references,
            "tests":[{"relevantLines":relevant_lines}],
            "completeness":{"status":"COMPLETE","omitted":{}},
        });
        let bounded = enforce_budget(pack, 1_024).unwrap();
        assert!(serialized_len(&bounded).unwrap() <= 1_024);
        assert_eq!(bounded["completeness"]["status"], "PARTIAL_BUDGET");
        assert_eq!(bounded["declarations"][0]["name"], "target");
    }

    #[test]
    fn reference_matching_obeys_kotlin_identifier_boundaries() {
        assert!(contains_kotlin_name("fun main()", "main"));
        assert!(!contains_kotlin_name("val remaining = 1", "main"));
        assert!(contains_kotlin_name(
            "System.getenv(\"ADAPTIVE_MIN_BATCH\")",
            "ADAPTIVE_"
        ));
    }
}
