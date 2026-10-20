//! Source-free export projection over validated retained evidence. The local
//! records remain untouched; remote output is not an evidence reimport format.
mod links;

use super::{check::Check, digest, invalid, io_error, store};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMode {
    #[default]
    Embedded,
    Remote,
}

#[derive(Debug, Default, Clone, clap::Args)]
pub struct Args {
    /// Embed retained source (default), or export exact GitHub/GitLab links without source attachments.
    #[arg(long, value_enum, default_value_t = SourceMode::Embedded)]
    pub source_mode: SourceMode,
    /// JSON provider/web-base mappings keyed by immutable repository ID; required for private hosts.
    #[arg(long)]
    pub source_map: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositoryMapping {
    pub provider: String,
    pub web_base: String,
    /// Exact old clone origin when its repository is absent from the selected snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub mode: SourceMode,
    #[serde(default)]
    pub repositories: BTreeMap<String, RepositoryMapping>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MappingFile {
    schema: String,
    repositories: BTreeMap<String, RepositoryMapping>,
}

impl Args {
    pub fn policy(&self) -> Result<Policy, ClewError> {
        let mut policy = Policy {
            mode: self.source_mode,
            repositories: BTreeMap::new(),
        };
        if let Some(path) = &self.source_map {
            let mapping: MappingFile = store::read(path, 1024 * 1024)?;
            if mapping.schema != "codeclew-source-repositories/1.0" {
                return Err(invalid("unsupported source-map schema"));
            }
            for (id, entry) in mapping.repositories {
                if id.is_empty()
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
                {
                    return Err(invalid(
                        "source-map keys must be immutable repository IDs, not URLs or credentials",
                    ));
                }
                if !matches!(entry.provider.as_str(), "github" | "gitlab") {
                    return Err(invalid("source-map provider must be github or gitlab"));
                }
                let web_base = links::web_base(&entry.web_base).map_err(invalid)?;
                policy.repositories.insert(
                    id,
                    RepositoryMapping {
                        provider: entry.provider,
                        web_base,
                        origin: entry
                            .origin
                            .as_deref()
                            .map(links::remote_identity)
                            .transpose()
                            .map_err(invalid)?,
                    },
                );
            }
        }
        Ok(policy)
    }
}

impl Policy {
    pub fn remote(&self) -> bool {
        self.mode == SourceMode::Remote
    }
    pub fn manifest(&self) -> Result<Value, ClewError> {
        Ok(
            json!({"schema":"codeclew-source-export/1.0","mode":self.mode,"repositories":self.repositories,"policyDigest":digest(self)?,"localEvidence":"UNCHANGED","authoredQuotations":"PRESERVED; this option removes automatic source attachments, not authored quotations or semantic identities"}),
        )
    }
}

/// SSH transport usernames are admitted locally, never emitted in web links.
pub(super) fn safe_ssh_repository(value: &str) -> bool {
    (value.starts_with("ssh://") || (!value.contains("://") && value.contains('@')))
        && links::repository(value).is_ok()
}

type SourceKey = (String, String, String, String, String, String);
fn key(v: &Value) -> SourceKey {
    let [a, b, c, d, e] = [
        "service",
        "revision",
        "file",
        "textDigest",
        "evidenceDigest",
    ]
    .map(|k| v[k].as_str().unwrap_or("").to_owned());
    let url = v["url"]
        .as_str()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .to_owned();
    (a, b, c, d, e, url)
}
fn is_source(v: &Value) -> bool {
    v["file"].is_string()
        && v["revision"].is_string()
        && v["textDigest"].is_string()
        && v["startLine"].is_u64()
        && v["endLine"].is_u64()
}

fn exact_source(v: &Value) -> bool {
    matches!(
        v["authority"].as_str(),
        Some("EXACT_SNAPSHOT_TEXT" | "DECLARED_OPENAPI")
    ) && !v["occurrence"]["generatedOrigin"].is_string()
}

#[derive(Clone)]
pub(super) struct Projection<'a> {
    policy: &'a Policy,
    current: BTreeMap<SourceKey, RepositoryMapping>,
    exact: std::collections::BTreeSet<SourceKey>,
    origins: BTreeMap<String, Vec<RepositoryMapping>>,
}

impl<'a> Projection<'a> {
    pub fn new(policy: &'a Policy, checked: &Check) -> Result<Self, ClewError> {
        let mut current = BTreeMap::new();
        let mut exact = std::collections::BTreeSet::new();
        if policy.remote() {
            for (service, evidence) in &checked.services {
                let Some(binding) = checked
                    .source_inputs
                    .as_ref()
                    .and_then(|i| i.inputs.services.get(service))
                else {
                    continue;
                };
                // A failure applies only to exported records: unrelated selected
                // services with no exported source need no repository mapping.
                let mapping = if let Some(mapping) = policy.repositories.get(&binding.repository_id)
                {
                    Some(mapping.clone())
                } else {
                    links::repository(&binding.repository)
                        .ok()
                        .and_then(|(base, provider)| {
                            provider.map(|p| RepositoryMapping {
                                provider: p.into(),
                                web_base: base,
                                origin: None,
                            })
                        })
                };
                if let Some(mapping) = mapping {
                    for source in evidence.sources.values() {
                        let value = serde_json::to_value(source).map_err(io_error)?;
                        if exact_source(&value) {
                            exact.insert(key(&value));
                        }
                        current.insert(key(&value), mapping.clone());
                    }
                }
            }
        }
        let mut projection = Self {
            policy,
            current,
            exact,
            origins: BTreeMap::new(),
        };
        for mapping in policy.repositories.values() {
            if let Some(origin) = &mapping.origin {
                projection
                    .origins
                    .entry(links::remote_identity(origin).map_err(invalid)?)
                    .or_default()
                    .push(mapping.clone());
            }
        }
        if let Some(inputs) = &checked.source_inputs {
            projection.register_inputs(&serde_json::to_value(&inputs.inputs).map_err(io_error)?);
        }
        Ok(projection)
    }

    fn register_inputs(&mut self, inputs: &Value) {
        for service in inputs["services"]
            .as_object()
            .into_iter()
            .flat_map(|o| o.values())
        {
            let (Some(id), Some(repository)) = (
                service["repositoryId"].as_str(),
                service["repository"].as_str(),
            ) else {
                continue;
            };
            let Ok(origin) = links::remote_identity(repository) else {
                continue;
            };
            if let Some(mapping) = self.policy.repositories.get(id) {
                self.origins
                    .entry(origin)
                    .or_default()
                    .push(mapping.clone());
            }
        }
    }

    pub fn register_frozen_files(
        &mut self,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), ClewError> {
        if self.policy.remote() {
            for name in ["inputs.json", "frozen-inputs.json"] {
                if let Some(bytes) = files.get(name) {
                    self.register_inputs(&serde_json::from_slice(bytes).map_err(io_error)?);
                }
            }
        }
        Ok(())
    }

    fn repository(&self, source: &Value) -> Result<RepositoryMapping, ClewError> {
        if let Some(mapping) = self.current.get(&key(source)) {
            return Ok(mapping.clone());
        }
        if source.get("text").is_none() && source["sourceId"].is_string() {
            let k = key(source);
            let candidates = self
                .current
                .iter()
                .filter(|(a, _)| a.0 == k.0 && a.1 == k.1 && a.2 == k.2 && a.4 == k.4 && a.5 == k.5)
                .map(|(_, m)| m)
                .collect::<Vec<_>>();
            if let Some(first) = candidates.first()
                && candidates.iter().all(|m| *m == *first)
            {
                return Ok((*first).clone());
            }
        }
        // Historical citations own their URL/revision. Never substitute the
        // currently selected service's repository or targetRef for old records.
        let url=source["url"].as_str().ok_or_else(||invalid("remote source has no exact retained repository binding; supply a source-map for its captured repository ID or use embedded mode"))?;
        let revision = source["revision"].as_str().unwrap_or("");
        for marker in [format!("/-/blob/{revision}/"), format!("/blob/{revision}/")] {
            if let Some((base, tail)) = url.split_once(&marker) {
                let origin = links::remote_identity(base).map_err(invalid)?;
                let (base, inferred) = links::repository(base).map_err(invalid)?;
                let mapped = self
                    .policy
                    .repositories
                    .values()
                    .filter(|m| m.web_base == base)
                    .chain(self.origins.get(&origin).into_iter().flatten())
                    .collect::<Vec<_>>();
                let mapping = if let Some(first) = mapped.first() {
                    if !mapped
                        .iter()
                        .all(|m| m.provider == first.provider && m.web_base == first.web_base)
                    {
                        return Err(invalid(
                            "ambiguous source-map entries for retained repository",
                        ));
                    }
                    (*first).clone()
                } else {
                    RepositoryMapping{provider:inferred.ok_or_else(||invalid("ambiguous retained repository host; configure provider/webBase and the exact retained origin in --source-map"))?.into(),web_base:base,origin:None}
                };
                let expected = links::link(
                    &mapping.provider,
                    &mapping.web_base,
                    revision,
                    source["file"].as_str().unwrap_or(""),
                    source["startLine"].as_u64().unwrap_or(0),
                    source["endLine"].as_u64().unwrap_or(0),
                )
                .map_err(invalid)?;
                let expected_tail = expected
                    .split_once(&format!("/blob/{revision}/"))
                    .unwrap()
                    .1
                    .split('#')
                    .next()
                    .unwrap();
                if tail.split('#').next() != Some(expected_tail)
                    && !links::file_matches(tail, source["file"].as_str().unwrap_or(""))
                {
                    return Err(invalid(
                        "retained source URL file differs from its exact source binding; no current-repository fallback is permitted",
                    ));
                }
                return Ok(mapping);
            }
        }
        Err(invalid(
            "retained source URL has no exact blob revision binding; use embedded mode or restore exact captured repository metadata",
        ))
    }

    fn source(&self, value: &mut Value) -> Result<(), ClewError> {
        // Frozen fragment summaries omit authority and service. Resolve them
        // against a complete exact record; a SHA-looking summary alone is not
        // evidence that the source was committed or not generated.
        let mapping = if value.get("authority").is_none() && value.get("text").is_none() {
            let mut candidates = self.exact.iter().filter_map(|k| {
                let mapping = self.current.get(k)?;
                if value["revision"].as_str() != Some(k.1.as_str())
                    || value["file"].as_str() != Some(k.2.as_str())
                    || value["textDigest"].as_str() != Some(k.3.as_str())
                {
                    return None;
                }
                let url = value["url"].as_str()?;
                let canonical = links::link(
                    &mapping.provider,
                    &mapping.web_base,
                    &k.1,
                    &k.2,
                    value["startLine"].as_u64()?,
                    value["endLine"].as_u64()?,
                )
                .ok()?;
                (url.split('#').next() == Some(k.5.as_str()) || url == canonical).then_some(mapping)
            });
            let first = candidates.next().ok_or_else(|| {
                invalid(format!(
                    "remote source summary has no complete exact retained binding: {} at {}",
                    value["file"], value["revision"]
                ))
            })?;
            if candidates
                .any(|other| other.provider != first.provider || other.web_base != first.web_base)
            {
                return Err(invalid(
                    "remote source summary has ambiguous repository bindings",
                ));
            }
            first.clone()
        } else {
            if !exact_source(value) {
                return Err(invalid(format!(
                    "remote export cannot link dirty, generated or unbound source {} at {}; use embedded mode for this evidence",
                    value["file"], value["revision"]
                )));
            }
            self.repository(value)?
        };
        let url = links::link(
            &mapping.provider,
            &mapping.web_base,
            value["revision"].as_str().unwrap_or(""),
            value["file"].as_str().unwrap_or(""),
            value["startLine"].as_u64().unwrap_or(0),
            value["endLine"].as_u64().unwrap_or(0),
        )
        .map_err(invalid)?;
        if value.get("text").is_some() {
            value["text"] = json!("");
        }
        value["url"] = json!(url);
        Ok(())
    }

    pub fn value(&self, value: &mut Value) -> Result<(), ClewError> {
        if !self.policy.remote() {
            return Ok(());
        }
        self.walk(value, false)
    }

    fn walk(&self, value: &mut Value, automatic: bool) -> Result<(), ClewError> {
        if is_source(value) {
            return self.source(value);
        }
        if let (Some(id), Some(repository)) =
            (value["repositoryId"].as_str(), value["repository"].as_str())
        {
            let web = if let Some(mapping) = self.policy.repositories.get(id) {
                mapping.web_base.clone()
            } else {
                links::repository(repository).map_err(invalid)?.0
            };
            value["repository"] = json!(web);
            if value.get("sourceLinkTemplate").is_some() {
                value["sourceLinkTemplate"] = Value::Null;
            }
        }
        if matches!(
            value["schema"].as_str(),
            Some("codeclew-http-comparison-view/1.0" | "codeclew-http-comparison/1.0")
        ) {
            let mut ids = value["artifactBindings"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(value["sources"].as_array().into_iter().flatten())
                .filter_map(|b| b["artifact"].as_str().map(str::to_owned))
                .collect::<std::collections::BTreeSet<_>>();
            let reports = [
                value["probeArtifact"].as_str(),
                value["assessmentArtifact"].as_str(),
            ]
            .into_iter()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>();
            for id in value["artifacts"]
                .as_object()
                .into_iter()
                .flat_map(|a| a.keys())
            {
                if !reports.contains(id.as_str()) {
                    ids.insert(id.clone());
                }
            }
            for id in ids {
                if let Some(artifact) = value["artifacts"].get_mut(&id) {
                    // Keep the declared source inventory as metadata, without
                    // retaining an opaque attached file or inventing Git URLs.
                    if let Some(manifest) = artifact["text"]
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    {
                        let sources=manifest["sources"].as_array().into_iter().flatten()
                            .filter(|s|s["file"].is_string()&&s["revision"].is_string()&&s["sha256"].is_string())
                            .map(|s|json!({"file":s["file"],"revision":s["revision"],"sha256":s["sha256"],"localCopy":s["localCopy"]})).collect::<Vec<_>>();
                        if !sources.is_empty() {
                            artifact["sourceInventory"] =
                                json!({"sources":sources,"scope":manifest["scope"]});
                        }
                    }
                    artifact["text"] = json!("");
                    artifact["sourceExport"] = json!({"mode":"remote","status":"SOURCE_TEXT_OMITTED","detail":"Attachment text is omitted; digests and available source bindings are preserved. Unbound test files have no Git link. Originals remain in local retained evidence."});
                }
            }
        }
        let observation = value["kind"].is_string()
            && !matches!(
                value["kind"].as_str(),
                Some("CONTRACT_OPERATION" | "CONTRACT_SCHEMA" | "DOMAIN_ENTITY")
            )
            && value["digest"].is_string()
            && value["sourceIds"].is_array()
            && value.get("normalized").is_some();
        let dsl = value["schema"] == "codeclew-flow-dsl-projection/1.0";
        let syntax_record = value["citationId"].is_string() && value.get("expression").is_some();
        if automatic && value["diagramStem"].is_string() {
            if value.get("svgAvailable").is_some() {
                value["svgAvailable"] = json!(false);
            }
            if value.get("pumlAvailable").is_some() {
                value["pumlAvailable"] = json!(false);
            }
        }
        match value {
            Value::Object(object) => {
                for (name, value) in object {
                    // Contract examples and authored quotations are not automatic
                    // source attachments even when their business fields are named
                    // expression, snippet, text or initializer.
                    if matches!(name.as_str(), "example" | "examples") {
                        continue;
                    }
                    let scoped = automatic
                        || syntax_record
                        || (observation && name == "normalized")
                        || (dsl && name == "items")
                        || matches!(
                            name.as_str(),
                            "sourceOutline"
                                | "processOutline"
                                | "processExtraction"
                                | "lifecycleOperations"
                                | "sourceCallGraph"
                                | "examinedSources"
                                | "sourceDataContext"
                                | "sourceSpans"
                                | "dataState"
                                | "retainedCallSites"
                                | "exactCallSite"
                        )
                        || (name == "callable" && value.is_object());
                    let syntax = scoped
                        && matches!(
                            name.as_str(),
                            "expression"
                                | "sourceText"
                                | "rawText"
                                | "sourceCode"
                                | "codeText"
                                | "snippet"
                                | "initializer"
                                | "text"
                                | "tree"
                                | "body"
                                | "raw"
                                | "condition"
                        );
                    if scoped
                        && name == "conditions"
                        && value
                            .as_array()
                            .is_some_and(|v| v.iter().all(Value::is_string))
                    {
                        for item in value.as_array_mut().unwrap() {
                            *item = json!(
                                "Source condition omitted from remote export; open the exact source binding."
                            );
                        }
                    } else if scoped
                        && name == "arguments"
                        && value
                            .as_array()
                            .is_some_and(|v| v.iter().all(Value::is_string))
                    {
                        for item in value.as_array_mut().unwrap() {
                            *item = json!("");
                        }
                    } else if syntax && value.is_string() {
                        *value = json!("");
                    } else {
                        self.walk(value, scoped)?;
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.walk(value, automatic)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Range changes retain the selected provider's syntax, including old GitLab
/// citations whose capture appended GitHub-style line fragments.
pub(super) fn range_url(url: &str, start: u64, end: u64) -> String {
    let base = url.split('#').next().unwrap_or(url);
    if base.contains("/-/blob/") {
        format!("{base}#L{start}-{end}")
    } else {
        format!("{base}#L{start}-L{end}")
    }
}

impl Projection<'_> {
    pub fn typed<T: Serialize + serde::de::DeserializeOwned>(
        &self,
        original: &T,
    ) -> Result<T, ClewError> {
        let mut value = serde_json::to_value(original).map_err(io_error)?;
        let mut projection = self.clone();
        projection.register_sources(&value)?;
        projection.value(&mut value)?;
        serde_json::from_value(value).map_err(io_error)
    }

    /// Run before portable normalization so downloads and shared object files
    /// contain the same projected evidence as visible pages.
    pub fn files(&mut self, files: &mut BTreeMap<String, Vec<u8>>) -> Result<(), ClewError> {
        if !self.policy.remote() {
            return Ok(());
        }
        let mut diagrams = std::collections::BTreeSet::new();
        fn source_diagrams(
            value: &Value,
            automatic: bool,
            diagrams: &mut std::collections::BTreeSet<String>,
        ) {
            if automatic && let Some(stem) = value["diagramStem"].as_str() {
                // Exact generated asset names only; never accept a filesystem path.
                if store::valid_id(stem) {
                    diagrams.insert(format!("diagrams/{stem}.puml"));
                    diagrams.insert(format!("diagrams/{stem}.svg"));
                }
            }
            match value {
                Value::Object(object) => {
                    for (name, v) in object {
                        if !matches!(name.as_str(), "example" | "examples") {
                            source_diagrams(
                                v,
                                automatic
                                    || matches!(
                                        name.as_str(),
                                        "sourceOutline" | "processOutline" | "lifecycleOperations"
                                    ),
                                diagrams,
                            );
                        }
                    }
                }
                Value::Array(values) => {
                    for v in values {
                        source_diagrams(v, automatic, diagrams)
                    }
                }
                _ => {}
            }
        }
        for (name, bytes) in files.iter() {
            if name.ends_with(".json") {
                let value = serde_json::from_slice(bytes).map_err(io_error)?;
                self.register_sources(&value)?;
                source_diagrams(&value, false, &mut diagrams);
            }
        }
        for name in diagrams {
            files.remove(&name);
        }
        // Markdown in a frozen publication already contains generated thread
        // appendices. Replace only exact producer-generated blocks; ordinary
        // authored prose and deliberate code quotations are never scrubbed.
        let mut markdown = Vec::new();
        for (name, bytes) in files.iter().filter(|(name, _)| name.ends_with(".json")) {
            let original: Value = serde_json::from_slice(bytes).map_err(io_error)?;
            let Some(subject) = original["subject"].as_str() else {
                continue;
            };
            let Some(md_name) = name.strip_suffix(".json").map(|n| format!("{n}.md")) else {
                continue;
            };
            let Some(md) = files.get(&md_name) else {
                continue;
            };
            let mut projected = original.clone();
            self.value(&mut projected)?;
            let language = original["language"].as_str().unwrap_or("en");
            let mut text = std::str::from_utf8(md).map_err(io_error)?.to_owned();
            let mut replace = |old: String, new: String| {
                if !old.is_empty() && old != new {
                    text = text.replace(&old, &new);
                }
            };
            replace(
                super::process_thread::markdown(&original["processThread"], subject, language),
                super::process_thread::markdown(&projected["processThread"], subject, language),
            );
            for (old, new) in original["serviceFragments"]
                .as_array()
                .into_iter()
                .flatten()
                .zip(
                    projected["serviceFragments"]
                        .as_array()
                        .into_iter()
                        .flatten(),
                )
            {
                let subject = format!("scenario:{}", old["processId"].as_str().unwrap_or(""));
                replace(
                    super::process_thread::markdown(&old["thread"], &subject, language),
                    super::process_thread::markdown(&new["thread"], &subject, language),
                );
            }
            if !original["processExtraction"].is_null() {
                let block = |payload: &Value| -> Result<String, ClewError> {
                    Ok(format!(
                        "\n## {}\n\n```json\n{}\n```\n",
                        super::reader::process_thread_text(language, "Extraction evidence packet"),
                        serde_json::to_string_pretty(payload).map_err(io_error)?
                    ))
                };
                replace(
                    block(&original["processExtraction"])?,
                    block(&projected["processExtraction"])?,
                );
            }
            markdown.push((md_name, text.into_bytes()));
        }
        files.extend(markdown);
        for (name, bytes) in files {
            if name.ends_with(".json") {
                let mut value: Value = serde_json::from_slice(bytes).map_err(io_error)?;
                let original = value.clone();
                self.value(&mut value)?;
                if value["subject"].is_string() {
                    value["sourceExport"] = self.policy.manifest()?;
                }
                if value != original {
                    *bytes = serde_json::to_vec_pretty(&value).map_err(io_error)?;
                }
            } else if name.ends_with(".html") {
                *bytes = self
                    .html(
                        std::str::from_utf8(bytes).map_err(io_error)?,
                        matches!(
                            name.as_str(),
                            "overview.html" | "index.html" | "retained-overview.html"
                        ),
                    )?
                    .into_bytes();
            }
        }
        Ok(())
    }

    fn html(&self, original: &str, diagnostics: bool) -> Result<String, ClewError> {
        let mut out = String::new();
        let mut rest = original;
        // Only inert JSON script blocks are data. Never parse or rewrite script
        // source, authored paragraphs, Markdown or arbitrary quoted examples.
        while let Some(start) = rest.find("<script") {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let end = rest
                .find('>')
                .ok_or_else(|| invalid("malformed generated script tag"))?
                + 1;
            let close = rest[end..]
                .find("</script>")
                .map(|n| end + n)
                .ok_or_else(|| invalid("malformed generated script block"))?;
            out.push_str(&rest[..end]);
            if rest[..end].contains("application/json") {
                let mut value: Value = serde_json::from_str(&rest[end..close]).map_err(io_error)?;
                self.value(&mut value)?;
                if value["subject"].is_string() {
                    value["sourceExport"] = self.policy.manifest()?;
                }
                out.push_str(
                    &serde_json::to_string(&value)
                        .map_err(io_error)?
                        .replace('<', "\\u003c")
                        .replace('\u{2028}', "\\u2028")
                        .replace('\u{2029}', "\\u2029"),
                );
            } else {
                out.push_str(&rest[end..close]);
            }
            out.push_str("</script>");
            rest = &rest[close + 9..];
        }
        out.push_str(rest);
        if diagnostics {
            self.diagnostics_html(&out)
        } else {
            Ok(out)
        }
    }

    fn register_sources(&mut self, value: &Value) -> Result<(), ClewError> {
        if !self.policy.remote() {
            return Ok(());
        }
        if is_source(value) && value["text"].is_string() && value["id"].is_string() {
            let mapping = self.repository(value)?;
            if exact_source(value) {
                self.exact.insert(key(value));
            }
            self.current.insert(key(value), mapping);
        }
        match value {
            Value::Object(object) => {
                for (name, value) in object {
                    if !matches!(name.as_str(), "example" | "examples") {
                        self.register_sources(value)?;
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.register_sources(value)?
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn diagnostics_html(&self, original: &str) -> Result<String, ClewError> {
        let mut out = String::new();
        let mut rest = original;
        while let Some(start) = rest.find("<pre") {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let Some(end) = rest.find('>').map(|n| n + 1) else {
                out.push_str(rest);
                return Ok(out);
            };
            let Some(close) = rest[end..].find("</pre>").map(|n| n + end) else {
                out.push_str(rest);
                return Ok(out);
            };
            out.push_str(&rest[..end]);
            let raw = &rest[end..close];
            let decoded = raw
                .replace("&quot;", "\"")
                .replace("&#39;", "'")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&");
            if let Ok(mut value) = serde_json::from_str::<Value>(&decoded) {
                let before = value.clone();
                self.value(&mut value)?;
                if value != before {
                    out.push_str(&super::render::escape(
                        &serde_json::to_string_pretty(&value).map_err(io_error)?,
                    ));
                } else {
                    out.push_str(raw);
                }
            } else {
                out.push_str(raw);
            }
            out.push_str("</pre>");
            rest = &rest[close + 6..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const REV: &str = "0123456789abcdef0123456789abcdef01234567";
    fn checked() -> Check {
        serde_json::from_value(json!({"schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"context","services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap()
    }
    #[test]
    fn remote_markdown_regenerates_exact_source_appendices_and_preserves_authored_quotes() {
        let checked = checked();
        let policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        let mut projection = Projection::new(&policy, &checked).unwrap();
        let source = json!({"id":"s","service":"svc","revision":REV,"file":"A.kt","startLine":7,"endLine":9,"text":"AUTOMATIC_THREAD_BODY_ONLY","textDigest":"body","evidenceDigest":"evidence","authority":"EXACT_SNAPSHOT_TEXT","url":format!("https://github.com/org/repo/blob/{REV}/A.kt#L7-L9")});
        let thread = json!({"status":"SOURCE_BOUND_DECLARATION","methods":{"root":{"label":"Root","source":source}},"methodOrder":["root"],"steps":[{"id":"step","title":"Authored step","text":"Authored explanation with a deliberate `code()` quotation.","method":"root","startLine":8,"endLine":9}],"boundaries":[],"scope":"Bounded source"});
        let original = super::super::process_thread::markdown(&thread, "scenario:test", "en");
        let mut files = BTreeMap::from([
            (
                "scenarios/test.json".into(),
                serde_json::to_vec(
                    &json!({"subject":"scenario:test","language":"en","processThread":thread}),
                )
                .unwrap(),
            ),
            (
                "scenarios/test.md".into(),
                format!("Authored introduction.\n{original}").into_bytes(),
            ),
        ]);
        projection.files(&mut files).unwrap();
        let markdown = std::str::from_utf8(&files["scenarios/test.md"]).unwrap();
        assert!(!markdown.contains("AUTOMATIC_THREAD_BODY_ONLY"));
        assert!(!markdown.contains("```"));
        assert!(
            markdown.contains("Authored introduction.")
                && markdown.contains("deliberate `code()` quotation")
        );
        assert!(markdown.contains(&format!(
            "https://github.com/org/repo/blob/{REV}/A.kt#L8-L9"
        )));
        assert!(!markdown.contains("#thread-source-root-L"));
    }

    #[test]
    fn remote_comparisons_omit_source_and_test_files_but_preserve_reports_and_authored_prose() {
        let checked = checked();
        let policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        let projection = Projection::new(&policy, &checked).unwrap();
        let raw = "ORIGINAL_PROGRAM_SOURCE_ONLY";
        let hash = crate::canonical::hash_bytes(raw.as_bytes());
        let manifest = json!({"sources":[{"file":"Program.cs","revision":REV,"sha256":hash,"localCopy":"Program.cs"}]});
        let authored = "A deliberately authored test implementation";
        let mut value = json!({"schema":"codeclew-http-comparison/1.0","sources":[],"probeArtifact":"probe","assessmentArtifact":"assessment","scope":authored,"example":{"snippet":authored},"artifacts":{
            "probe":{"text":"recorded probe report","digest":"probe-digest"},
            "assessment":{"text":"recorded assessment report","digest":"assessment-digest"},
            "inventory":{"text":manifest.to_string(),"digest":"manifest-digest"},
            "program":{"text":raw,"digest":hash},
            "example":{"text":authored,"digest":crate::canonical::hash_bytes(authored.as_bytes())}
        }});
        projection.value(&mut value).unwrap();
        assert_eq!(value["artifacts"]["program"]["text"], "");
        assert_eq!(value["artifacts"]["program"]["digest"], hash);
        assert_eq!(value["artifacts"]["example"]["text"], "");
        assert_eq!(value["scope"], authored);
        assert_eq!(value["example"]["snippet"], authored);
        assert_eq!(value["artifacts"]["probe"]["text"], "recorded probe report");
        assert_eq!(
            value["artifacts"]["assessment"]["text"],
            "recorded assessment report"
        );
        assert_eq!(
            value["artifacts"]["inventory"]["sourceInventory"]["sources"][0]["sha256"],
            hash
        );
        let mut declared = json!({"id":"api","service":"svc","revision":REV,"file":"openapi.yaml","startLine":1,"endLine":3,"text":"DECLARED_CONTRACT_BODY","textDigest":"body","authority":"DECLARED_OPENAPI","url":format!("https://github.com/org/repo/blob/{REV}/openapi.yaml#L1-L3")});
        projection.value(&mut declared).unwrap();
        assert_eq!(declared["authority"], "DECLARED_OPENAPI");
        assert_eq!(declared["text"], "");
    }

    #[test]
    fn remote_rejects_unbound_dirty_generated_and_ambiguous_sources() {
        let checked = checked();
        let policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        let projection = Projection::new(&policy, &checked).unwrap();
        let exact = json!({"id":"s","service":"svc","revision":REV,"file":"src/A.kt","startLine":1,"endLine":2,"text":"PRIVATE_SOURCE","textDigest":"body","evidenceDigest":"binding","authority":"EXACT_SNAPSHOT_TEXT","url":format!("https://github.com/org/repo/blob/{REV}/src/A.kt#L1-L2")});
        for (field, value) in [
            ("revision", json!("dirty-working-tree")),
            ("authority", json!("WORKING_TREE")),
            ("occurrence", json!({"generatedOrigin":"generated"})),
            (
                "url",
                json!(format!(
                    "https://unknown.example/org/repo/blob/{REV}/src/A.kt#L1-L2"
                )),
            ),
        ] {
            let mut source = exact.clone();
            source[field] = value;
            assert!(projection.value(&mut source).is_err(), "{field}");
            assert_eq!(source["text"], "PRIVATE_SOURCE");
        }
        let mut value = exact.clone();
        Projection::new(&Policy::default(), &checked)
            .unwrap()
            .value(&mut value)
            .unwrap();
        assert_eq!(
            value, exact,
            "embedded remains the default and preserves source bytes"
        );
    }

    #[test]
    fn remote_omits_only_source_derived_diagrams_and_trees() {
        let checked = checked();
        let policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        let mut projection = Projection::new(&policy, &checked).unwrap();
        let value = json!({"processOutline":{"tree":"SOURCE_TREE_MARKER","diagramStem":"source-flow","svgAvailable":true,"pumlAvailable":true},"authoredDiagram":{"diagramStem":"authored-flow","text":"Authored explanation"}});
        let mut files = BTreeMap::from([
            ("scenario.json".into(), serde_json::to_vec(&value).unwrap()),
            (
                "diagrams/source-flow.svg".into(),
                b"SOURCE_TREE_MARKER".to_vec(),
            ),
            (
                "diagrams/source-flow.puml".into(),
                b"SOURCE_TREE_MARKER".to_vec(),
            ),
            (
                "diagrams/authored-flow.svg".into(),
                b"Authored diagram".to_vec(),
            ),
        ]);
        projection.files(&mut files).unwrap();
        assert!(!files.contains_key("diagrams/source-flow.svg"));
        assert!(!files.contains_key("diagrams/source-flow.puml"));
        assert_eq!(files["diagrams/authored-flow.svg"], b"Authored diagram");
        let projected: Value = serde_json::from_slice(&files["scenario.json"]).unwrap();
        assert_eq!(projected["processOutline"]["tree"], "");
        assert_eq!(projected["processOutline"]["svgAvailable"], false);
        assert_eq!(projected["authoredDiagram"], value["authoredDiagram"]);
    }

    #[test]
    fn remote_removes_automatic_source_without_erasing_authored_and_contract_fields() {
        let check = checked();
        let policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        let projection = Projection::new(&policy, &check).unwrap();
        let source = json!({"id":"s","service":"svc","revision":REV,"file":"src/A.kt","startLine":7,"endLine":12,"text":"SOURCE_BODY_SENTINEL","textDigest":"sha256:original-text","evidenceDigest":"sha256:evidence","authority":"EXACT_SNAPSHOT_TEXT","url":format!("https://github.com/org/repo/blob/{REV}/src/A.kt#L7-L12")});
        let contract = json!({"example":{"expression":"quantity * price","snippet":"user example","sourceText":"authored quote"},"properties":{"expression":{"type":"string"}},"expression":"authored expression"});
        let mut value = json!({"summary":{"text":"AUTHORED_PROSE_SENTINEL"},"contract":contract,"sources":{"s":source},"observations":[{"id":"o","kind":"FLOW","digest":"sha256:flow","sourceIds":["s"],"normalized":{"expression":"AUTOMATIC_EXPRESSION_SENTINEL","text":"AUTOMATIC_TEXT_SENTINEL","kind":"CALL"}}]});
        projection.value(&mut value).unwrap();
        let bytes = serde_json::to_string(&value).unwrap();
        for marker in [
            "SOURCE_BODY_SENTINEL",
            "AUTOMATIC_EXPRESSION_SENTINEL",
            "AUTOMATIC_TEXT_SENTINEL",
        ] {
            assert!(!bytes.contains(marker), "{marker}");
        }
        assert_eq!(value["contract"], contract);
        assert_eq!(value["summary"]["text"], "AUTHORED_PROSE_SENTINEL");
        assert_eq!(value["sources"]["s"]["textDigest"], "sha256:original-text");
        assert_eq!(
            value["sources"]["s"]["url"],
            format!("https://github.com/org/repo/blob/{REV}/src/A.kt#L7-L12")
        );
    }
    #[test]
    fn historical_sources_keep_their_own_repositories_and_revisions() {
        let check = checked();
        let mut policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        policy.repositories.insert(
            "retained-repo".into(),
            RepositoryMapping {
                provider: "gitlab".into(),
                web_base: "https://git.example.invalid/group/nested/old".into(),
                origin: None,
            },
        );
        let projection = Projection::new(&policy, &check).unwrap();
        for (repo, revision, provider) in [
            ("https://github.com/org/new", REV, "github"),
            (
                "https://git.example.invalid/group/nested/old",
                "abcdef0123456789abcdef0123456789abcdef01",
                "gitlab",
            ),
        ] {
            let mut source = json!({"service":"same-service","revision":revision,"file":"src/A.kt","startLine":3,"endLine":9,"text":"old bytes","textDigest":"sha256:body","evidenceDigest":"sha256:binding","authority":"EXACT_SNAPSHOT_TEXT","url":format!("{repo}/blob/{revision}/src/A.kt#L3-L9")});
            projection.value(&mut source).unwrap();
            assert_eq!(
                source["url"],
                links::link(provider, repo, revision, "src/A.kt", 3, 9).unwrap()
            );
        }
    }
    #[test]
    fn export_identity_binds_mode_and_repository_mapping() {
        let embedded = Policy::default();
        let mut remote = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        assert_ne!(digest(&embedded).unwrap(), digest(&remote).unwrap());
        let before = digest(&remote).unwrap();
        remote.repositories.insert(
            "repo".into(),
            RepositoryMapping {
                provider: "gitlab".into(),
                web_base: "https://git.example.invalid/prefix/group/repo".into(),
                origin: None,
            },
        );
        assert_ne!(before, digest(&remote).unwrap());
    }
    #[test]
    fn old_ssh_origin_uses_its_web_override_without_selecting_a_new_repository() {
        let check = checked();
        let mut policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        policy.repositories.insert(
            "retained-old".into(),
            RepositoryMapping {
                provider: "gitlab".into(),
                web_base: "https://git.example.invalid:8443/gitlab/team/nested/repo".into(),
                origin: Some("ssh://git@git.example.invalid:2222/team/nested/repo.git".into()),
            },
        );
        let mut projection = Projection::new(&policy, &check).unwrap();
        let mut old = json!({"id":"same-id","service":"same-service","revision":REV,"file":"src/A file#.kt","startLine":3,"endLine":9,"text":"OLD_SOURCE_BODY","textDigest":"old-digest","evidenceDigest":"old-evidence","authority":"EXACT_SNAPSHOT_TEXT","url":format!("ssh://git@git.example.invalid:2222/team/nested/repo.git/blob/{REV}/src/A file#.kt#L3-L9")});
        let mut new = old.clone();
        new["url"] = json!(format!(
            "https://github.com/new/repository/blob/{REV}/src/A%20file%23.kt#L3-L9"
        ));
        projection.current.insert(
            key(&new),
            RepositoryMapping {
                provider: "github".into(),
                web_base: "https://github.com/new/repository".into(),
                origin: None,
            },
        );
        projection.value(&mut old).unwrap();
        assert_eq!(
            old["url"],
            format!(
                "https://git.example.invalid:8443/gitlab/team/nested/repo/-/blob/{REV}/src/A%20file%23.kt#L3-9"
            )
        );
    }

    #[test]
    fn contract_normalization_preserves_business_fields_named_like_source_syntax() {
        let check = checked();
        let policy = Policy {
            mode: SourceMode::Remote,
            ..Policy::default()
        };
        let projection = Projection::new(&policy, &check).unwrap();
        let mut contract = json!({"kind":"CONTRACT_OPERATION","digest":"contract","sourceIds":[],"normalized":{"expression":"quantity * price","snippet":"user example","example":{"sourceText":"authored quotation"}}});
        let before = contract.clone();
        projection.value(&mut contract).unwrap();
        assert_eq!(contract, before);
    }
}
