//! Pure, opt-in model representation for expanding method author and reviewer jobs.
//!
//! Canonical validation remains the caller's responsibility. This codec never
//! recalculates packet bindings from projected content and never edits evidence
//! text, semantic symbols, native selections, or local dataflow identities.

use crate::{
    canonical,
    error::{ClewError, ErrorCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const VERSION: &str = "codeclew-model-ids/1.0";
pub const COMPACT_VERSION: &str = "codeclew-model-ids/1.1";

pub fn supported_version(version: &str) -> bool {
    matches!(version, VERSION | COMPACT_VERSION)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub work: String,
    pub run: String,
    pub role: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Domain {
    Work,
    Run,
    Invocation,
    Snapshot,
    Digest,
    SourceSpan,
}

impl Domain {
    fn prefix(self) -> &'static str {
        match self {
            Self::Work => "w",
            Self::Run => "r",
            Self::Invocation => "i",
            Self::Snapshot => "s",
            Self::Digest => "d",
            Self::SourceSpan => "p",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AliasEntry {
    pub domain: Domain,
    pub canonical: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AliasMap {
    pub version: String,
    pub scope: Scope,
    pub scope_tag: String,
    pub entries: Vec<AliasEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Prepared {
    pub version: String,
    pub scope: Scope,
    pub canonical_job_digest: String,
    pub model_payload: Value,
    pub output_schema: Value,
    pub map: AliasMap,
    pub model_payload_digest: String,
    pub output_schema_digest: String,
    pub map_digest: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInput {
    pub payload: Value,
    pub output_schema: Value,
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(
        ErrorCode::InvalidInput,
        format!("MODEL_IDS_INVALID: {}", message.into()),
    )
}

fn hash(value: &impl Serialize) -> Result<String, ClewError> {
    canonical::hash(value).map_err(|error| invalid(error.to_string()))
}

fn scope_tag(scope: &Scope, version: &str) -> Result<String, ClewError> {
    // A scope tag is part of every alias, including aliases reused in later
    // calls. Full scope equality is additionally required for map extension.
    Ok(hash(&json!({"version":version,"scope":scope}))?[7..23].to_owned())
}

impl AliasMap {
    fn validate(&self, scope: &Scope, version: &str) -> Result<(), ClewError> {
        if !supported_version(version)
            || self.version != version
            || self.scope != *scope
            || self.scope_tag != scope_tag(scope, version)?
        {
            return Err(invalid("map version or scope differs"));
        }
        let mut canonical = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if !eligible(entry.domain, &entry.canonical)
                || entry.alias
                    != format!(
                        "{}{ordinal}_{tag}",
                        entry.domain.prefix(),
                        ordinal = index + 1,
                        tag = self.scope_tag
                    )
                || !canonical.insert((entry.domain, &entry.canonical))
                || !aliases.insert(&entry.alias)
            {
                return Err(invalid("map has a conflicting or noncanonical entry"));
            }
        }
        Ok(())
    }

    fn project(&mut self, domain: Domain, value: &str) -> Result<String, ClewError> {
        if !eligible(domain, value) {
            return Ok(value.to_owned());
        }
        if self.entries.iter().any(|entry| entry.alias == value) {
            return Err(invalid("canonical input already contains an alias"));
        }
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.domain == domain && entry.canonical == value)
        {
            return Ok(entry.alias.clone());
        }
        let alias = format!(
            "{}{ordinal}_{tag}",
            domain.prefix(),
            ordinal = self.entries.len() + 1,
            tag = self.scope_tag
        );
        self.entries.push(AliasEntry {
            domain,
            canonical: value.to_owned(),
            alias: alias.clone(),
        });
        Ok(alias)
    }

    fn decode(&self, domain: Domain, value: &str) -> Result<String, ClewError> {
        self.entries
            .iter()
            .find(|entry| entry.domain == domain && entry.alias == value)
            .map(|entry| entry.canonical.clone())
            .ok_or_else(|| invalid("unknown, wrong-domain, or foreign-scope result alias"))
    }
}

fn eligible(domain: Domain, value: &str) -> bool {
    match domain {
        Domain::Digest => value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }),
        Domain::Work => {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        Domain::Run | Domain::Invocation => {
            value.len() == 32
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        // Existing short citations (c1, d192) are never representation targets.
        Domain::Snapshot => !value.is_empty(),
        Domain::SourceSpan => value.len() >= 24,
    }
}

fn binding_domain(field: &str) -> Option<Domain> {
    Some(match field {
        "work" => Domain::Work,
        "sourceRun" => Domain::Run,
        "sourceInvocation" => Domain::Invocation,
        "snapshot" => Domain::Snapshot,
        "packetDigest" | "answerDigest" | "coverageDigest" | "reviewContextDigest" => {
            Domain::Digest
        }
        _ => return None,
    })
}

fn field(
    object: &mut Value,
    key: &str,
    domain: Domain,
    map: &mut AliasMap,
) -> Result<(), ClewError> {
    if let Some(value) = object.get_mut(key)
        && let Some(text) = value.as_str()
    {
        *value = json!(map.project(domain, text)?);
    }
    Ok(())
}

fn fields(
    object: &mut Value,
    keys: &[&str],
    domain: Domain,
    map: &mut AliasMap,
) -> Result<(), ClewError> {
    for key in keys {
        field(object, key, domain, map)?;
    }
    Ok(())
}

fn each(
    object: &mut Value,
    key: &str,
    map: &mut AliasMap,
    visit: fn(&mut Value, &mut AliasMap) -> Result<(), ClewError>,
) -> Result<(), ClewError> {
    if let Some(rows) = object.get_mut(key).and_then(Value::as_array_mut) {
        for row in rows {
            visit(row, map)?;
        }
    }
    Ok(())
}

fn source(source: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    fields(
        source,
        &["textDigest", "evidenceDigest"],
        Domain::Digest,
        map,
    )?;
    if let Some(occurrence) = source.get_mut("occurrence") {
        field(occurrence, "snapshot", Domain::Snapshot, map)?;
        field(occurrence, "blob", Domain::Digest, map)?;
    }
    Ok(())
}

fn part(part: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    field(part, "work", Domain::Work, map)?;
    field(part, "snapshot", Domain::Snapshot, map)?;
    fields(
        part,
        &["recordDigest", "fragmentDigest", "receiptDigest"],
        Domain::Digest,
        map,
    )?;
    if let Some(value) = part.get_mut("source") {
        source(value, map)?;
    }
    Ok(())
}

fn page(page: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    field(page, "work", Domain::Work, map)?;
    field(page, "snapshot", Domain::Snapshot, map)?;
    fields(
        page,
        &[
            "contextDigest",
            "inputDigest",
            "membershipDigest",
            "receiptDigest",
        ],
        Domain::Digest,
        map,
    )?;
    if let Some(items) = page.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            field(item, "recordDigest", Domain::Digest, map)?;
            if item["kind"] == "SOURCE"
                && let Some(record) = item.get_mut("record")
            {
                source(record, map)?;
            }
        }
    }
    Ok(())
}

fn receipt(receipt: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    fields(
        receipt,
        &["resultDigest", "membershipDigest"],
        Domain::Digest,
        map,
    )
}

fn delivery(delivery: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    field(delivery, "work", Domain::Work, map)?;
    field(delivery, "snapshot", Domain::Snapshot, map)?;
    field(delivery, "deliveredDigest", Domain::Digest, map)?;
    each(delivery, "pages", map, page)?;
    each(delivery, "sourceParts", map, part)?;
    each(delivery, "pageReceipts", map, receipt)?;
    each(delivery, "sourcePartReceipts", map, part)?;
    if let Some(presentation) = delivery.get_mut("presentation") {
        each(presentation, "pages", map, page)?;
        each(presentation, "sourceParts", map, part)?;
        // Callable provenance is host-generated metadata. Callable identity,
        // symbol tokens, event tokens and all native read selections stay exact.
        if let Some(callables) = presentation
            .get_mut("callables")
            .and_then(Value::as_array_mut)
        {
            for callable in callables {
                if let Some(provenance) = callable.get_mut("provenance") {
                    fields(
                        provenance,
                        &[
                            "recordDigest",
                            "observationDigest",
                            "sourceDigest",
                            "textDigest",
                            "evidenceDigest",
                        ],
                        Domain::Digest,
                        map,
                    )?;
                    field(provenance, "snapshot", Domain::Snapshot, map)?;
                }
            }
        }
    }
    Ok(())
}

fn packet(packet: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    field(packet, "packetDigest", Domain::Digest, map)?;
    each(packet, "methodSources", map, source)?;
    if let Some(binding) = packet.get_mut("graphAuditBinding") {
        field(binding, "artifactDigest", Domain::Digest, map)?;
        field(binding, "snapshot", Domain::Snapshot, map)?;
    }
    if let Some(context) = packet.get_mut("contextDelivery") {
        delivery(context, map)?;
    }
    if let Some(context) = packet.get_mut("sourceDataContext") {
        field(context, "snapshot", Domain::Snapshot, map)?;
        fields(
            context,
            &["sourceDataDigest", "examinedSourceDigest"],
            Domain::Digest,
            map,
        )?;
        each(context, "sources", map, source)?;
        if let Some(spans) = context
            .get_mut("sourceSpans")
            .and_then(Value::as_object_mut)
        {
            let old = std::mem::take(spans);
            for (key, mut binding) in old {
                fields(
                    &mut binding,
                    &["sourceDigest", "textDigest"],
                    Domain::Digest,
                    map,
                )?;
                let key = map.project(Domain::SourceSpan, &key)?;
                if spans.insert(key, binding).is_some() {
                    return Err(invalid("source-span key collision"));
                }
            }
        }
        if let Some(nodes) = context.get_mut("nodes").and_then(Value::as_array_mut) {
            for node in nodes {
                fields(
                    node,
                    &["examinedSourceDigest", "variableFactsDigest"],
                    Domain::Digest,
                    map,
                )?;
                each(node, "callSites", map, span_reference)?;
                if let Some(state) = node.get_mut("dataState") {
                    each(state, "definitions", map, span_reference)?;
                    each(state, "gaps", map, span_reference)?;
                }
            }
        }
    }
    Ok(())
}

fn span_reference(value: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    field(value, "sourceSpan", Domain::SourceSpan, map)
}

fn project_schema(schema: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for (key, node) in properties {
            if let Some(domain) = binding_domain(key) {
                for keyword in ["const", "enum"] {
                    if let Some(value) = node.get_mut(keyword) {
                        match value {
                            Value::String(text) => *text = map.project(domain, text)?,
                            Value::Array(values) => {
                                for value in values {
                                    if let Some(text) = value.as_str() {
                                        *value = json!(map.project(domain, text)?);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(object) = node.as_object_mut() {
                    object.remove("pattern");
                    object.insert(
                        "pattern".into(),
                        json!(format!(
                            "^{}[1-9][0-9]*_{}$",
                            domain.prefix(),
                            map.scope_tag
                        )),
                    );
                }
            }
            project_schema(node, map)?;
        }
    }
    for key in ["$defs", "definitions"] {
        if let Some(definitions) = schema.get_mut(key).and_then(Value::as_object_mut) {
            for definition in definitions.values_mut() {
                project_schema(definition, map)?;
            }
        }
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(branches) = schema.get_mut(key).and_then(Value::as_array_mut) {
            for branch in branches {
                project_schema(branch, map)?;
            }
        }
    }
    if let Some(items) = schema.get_mut("items") {
        project_schema(items, map)?;
    }
    Ok(())
}

fn contract(payload: &mut Value, map: &mut AliasMap) -> Result<(), ClewError> {
    if let Some(value) = payload.get_mut("packet") {
        packet(value, map)?;
    }
    if let Some(value) = payload.get_mut("outputSchema") {
        project_schema(value, map)?;
    }
    // Rewrite only this exact host-generated binding sentence. Freeform prose,
    // retained instructions and source text are never subject to substitution.
    if let Some(instruction) = payload.get("instruction").and_then(Value::as_str) {
        const PREFIX: &str = "packetDigest exactly to `";
        if let Some(start) = instruction.find(PREFIX) {
            let value_start = start + PREFIX.len();
            if let Some(end) = instruction[value_start..].find('`') {
                let value_end = value_start + end;
                let alias = map.project(Domain::Digest, &instruction[value_start..value_end])?;
                payload["instruction"] = json!(format!(
                    "{}{}{}",
                    &instruction[..value_start],
                    alias,
                    &instruction[value_end..]
                ));
            }
        }
    }
    if let Some(repair) = payload.get_mut("repair") {
        // Previous candidates are data, but their answer binding is typed.
        for key in ["previousAnswer", "answer"] {
            if let Some(answer) = repair.get_mut(key) {
                field(answer, "packetDigest", Domain::Digest, map)?;
            }
        }
    }
    Ok(())
}

/// Prepare an exact model payload and schema after canonical job validation.
/// Pass the preceding map for this same Work/run/role to retain every alias.
/// The returned map is host/driver metadata and must never enter model input.
pub fn prepare(
    canonical_job: &Value,
    scope: &Scope,
    previous: Option<&AliasMap>,
) -> Result<Prepared, ClewError> {
    prepare_with_version(canonical_job, scope, previous, VERSION)
}

/// Prepare either the immutable 1.0 encoding or the explicitly selected 1.1
/// encoding. Version 1.1 additionally removes raw delivery arrays preserved by
/// the checked native presentation builder.
pub fn prepare_with_version(
    canonical_job: &Value,
    scope: &Scope,
    previous: Option<&AliasMap>,
    version: &str,
) -> Result<Prepared, ClewError> {
    if !supported_version(version) {
        return Err(invalid("unsupported representation version"));
    }
    if canonical_job["schema"] != "codeclew-documentation-agent-job/1.0"
        || canonical_job["work"] != scope.work
        || canonical_job["role"] != scope.role
        || !matches!(scope.role.as_str(), "author" | "reviewer")
        || scope.run.is_empty()
    {
        return Err(invalid(
            "canonical job and scope differ or role is unsupported",
        ));
    }
    let canonical_payload = &canonical_job["payload"];
    let contract_version = if scope.role == "author" {
        &canonical_payload["authoringContract"]
    } else {
        &canonical_payload["savedAuthorContract"]["authoringContract"]
    };
    if contract_version != "codeclew-operation-draft-authoring/1.6"
        || !canonical_payload["outputSchema"].is_object()
    {
        return Err(invalid(
            "only method authoring contract 1.6 and its independent reviewer are supported",
        ));
    }
    let mut map = previous.cloned().unwrap_or(AliasMap {
        version: version.into(),
        scope: scope.clone(),
        scope_tag: scope_tag(scope, version)?,
        entries: vec![],
    });
    map.validate(scope, version)?;
    // Verify the canonical presentation before typed IDs are projected. The
    // native builder validates exact source/token/event aliases using canonical
    // provenance; invoking it on model aliases would change those checks.
    let compact_paths = if version == COMPACT_VERSION {
        checked_compact_deliveries(canonical_payload)?
    } else {
        vec![]
    };
    let mut payload = canonical_payload.clone();
    contract(&mut payload, &mut map)?;
    if let Some(source) = payload.get_mut("source") {
        for key in [
            "sourceRun",
            "sourceInvocation",
            "snapshot",
            "packetDigest",
            "answerDigest",
        ] {
            field(source, key, binding_domain(key).unwrap(), &mut map)?;
        }
        fields(
            source,
            &["sourceInputDigest", "sourceResultDigest"],
            Domain::Digest,
            &mut map,
        )?;
        if let Some(checkpoint) = source.get_mut("sourceCheckpoint") {
            field(checkpoint, "checkpointDigest", Domain::Digest, &mut map)?;
            field(checkpoint, "run", Domain::Run, &mut map)?;
        }
    }
    if let Some(answer) = payload.get_mut("answer") {
        field(answer, "packetDigest", Domain::Digest, &mut map)?;
    }
    if let Some(context) = payload.get_mut("reviewContext") {
        delivery(context, &mut map)?;
    }
    if let Some(saved) = payload.get_mut("savedAuthorContract") {
        contract(saved, &mut map)?;
    }
    // Author native schemas constrain a digest pattern rather than one const.
    // Bind that schema leaf to this call's delivered packet alias as well.
    if scope.role == "author" {
        let alias = payload["packet"]["packetDigest"].clone();
        bind_packet_schema(&mut payload["outputSchema"], &alias);
    }
    if version == COMPACT_VERSION {
        for pointer in compact_paths {
            let delivery = payload
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap();
            delivery.remove("pages");
            delivery.remove("sourceParts");
        }
    }
    let instruction = payload["instruction"]
        .as_str()
        .ok_or_else(|| invalid("missing host instruction"))?;
    payload["instruction"] = json!(format!(
        "{instruction}\n\nModel identity encoding {version}: copy typed opaque identity aliases exactly from this payload and outputSchema. Aliases are scoped to this Work, run and role. Evidence labels, code, semantic symbols, native references, queries and source text keep their original meaning. Never invent or translate an identity alias."
    ));
    if version == COMPACT_VERSION {
        let instruction = payload["instruction"].as_str().unwrap();
        payload["instruction"] = json!(format!(
            "{instruction} When raw delivery arrays are omitted, read complete pages and sourceParts from that delivery's presentation. Its receipts, citations and deliveredDigest retain their binding."
        ));
    }
    let output_schema = payload["outputSchema"].clone();
    map.validate(scope, version)?;
    Ok(Prepared {
        version: version.into(),
        scope: scope.clone(),
        canonical_job_digest: hash(canonical_job)?,
        model_payload_digest: hash(&payload)?,
        output_schema_digest: hash(&output_schema)?,
        map_digest: hash(&map)?,
        model_payload: payload,
        output_schema,
        map,
    })
}

fn checked_compact_deliveries(payload: &Value) -> Result<Vec<&'static str>, ClewError> {
    let mut paths = Vec::new();
    for pointer in ["/packet/contextDelivery", "/reviewContext"] {
        if let Some(delivery) = payload.pointer(pointer)
            && checked_compact_delivery(delivery)?
        {
            paths.push(pointer);
        }
    }
    Ok(paths)
}

fn checked_compact_delivery(delivery: &Value) -> Result<bool, ClewError> {
    // A delivery without presentation keeps its original evidence. An existing
    // presentation must equal the existing pure native builder exactly,
    // including retainedAt and all
    // checked source/token/event aliases. Never rewrite a supplied presentation.
    let Some(presentation) = delivery.get("presentation") else {
        return Ok(false);
    };
    let pages = delivery
        .get("pages")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("compact delivery requires canonical pages"))?;
    let source_parts = delivery
        .get("sourceParts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("compact delivery requires canonical sourceParts"))?;
    if *presentation != super::job_context::present(pages, source_parts) {
        return Err(invalid(
            "compact delivery presentation differs from checked native presentation",
        ));
    }
    Ok(true)
}

fn bind_packet_schema(value: &mut Value, alias: &Value) {
    if let Some(properties) = value.get_mut("properties").and_then(Value::as_object_mut) {
        if let Some(binding) = properties.get_mut("packetDigest") {
            binding["const"] = alias.clone();
        }
        for node in properties.values_mut() {
            bind_packet_schema(node, alias);
        }
    }
    if let Some(definitions) = value.get_mut("$defs").and_then(Value::as_object_mut) {
        for node in definitions.values_mut() {
            bind_packet_schema(node, alias);
        }
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(nodes) = value.get_mut(key).and_then(Value::as_array_mut) {
            for node in nodes {
                bind_packet_schema(node, alias);
            }
        }
    }
}

/// Verify the frozen model form against unchanged canonical input. This checks
/// representation integrity, not canonical packet correctness or model delivery.
pub fn validate(canonical_job: &Value, prepared: &Prepared) -> Result<(), ClewError> {
    let expected = prepare_with_version(
        canonical_job,
        &prepared.scope,
        Some(&prepared.map),
        &prepared.version,
    )?;
    if *prepared != expected {
        return Err(invalid("frozen prepared representation differs"));
    }
    Ok(())
}

/// Supported serializer boundary: existing canonical checks run first, then
/// the exact frozen projection is checked and returned without map metadata.
pub fn forward_model_input(
    canonical_job: &Value,
    prepared: &Prepared,
    validate_canonical: impl FnOnce(&Value) -> Result<(), ClewError>,
) -> Result<ModelInput, ClewError> {
    validate_canonical(canonical_job)?;
    validate(canonical_job, prepared)?;
    Ok(ModelInput {
        payload: prepared.model_payload.clone(),
        output_schema: prepared.output_schema.clone(),
    })
}

/// Decode only result binding fields, before the existing semantic validator.
/// Callers must durably retain delivered raw output before invoking this method.
pub fn decode_result(wire_result: &Value, prepared: &Prepared) -> Result<Value, ClewError> {
    prepared.map.validate(&prepared.scope, &prepared.version)?;
    if !supported_version(&prepared.version) || prepared.map_digest != hash(&prepared.map)? {
        return Err(invalid("prepared result map differs"));
    }
    let mut result = wire_result.clone();
    let key = if prepared.scope.role == "author" {
        "answer"
    } else {
        "review"
    };
    if let Some(object) = result.get_mut(key).and_then(Value::as_object_mut) {
        for (field, value) in object {
            if let Some(domain) = binding_domain(field) {
                let text = value
                    .as_str()
                    .ok_or_else(|| invalid("result identity must be a string"))?;
                *value = json!(prepared.map.decode(domain, text)?);
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn scope(role: &str) -> Scope {
        Scope {
            work: "a".repeat(64),
            run: "b".repeat(32),
            role: role.into(),
        }
    }

    fn author_job() -> Value {
        let packet_digest = digest('c');
        let span = "e".repeat(64);
        let native_schema = super::super::operation_answer::output_schema();
        json!({"schema":"codeclew-documentation-agent-job/1.0","work":"a".repeat(64),
        "role":"author","invocation":"f".repeat(32),"model":"synthetic-test-model", "payload":{
            "authoringContract":"codeclew-operation-draft-authoring/1.6",
            "instruction":format!("Return one answer. Set packetDigest exactly to `{packet_digest}`. Keep code unchanged."),
            "packet":{"packetDigest":packet_digest,"citations":{"c1":"source","d192":"dependency"},
                "methodSources":[{"text":format!("String id = \"{}\";",digest('d')),"file":"Orders.java","textDigest":digest('d'),
                    "evidenceDigest":digest('e'),"occurrence":{"snapshot":digest('f'),"blob":digest('f'),"startByte":0,"endByte":17}}],
                "graphAuditBinding":{"artifactDigest":digest('b'),"snapshot":digest('f'),"rootMethodId":digest('a')},
                "sourceDataContext":{"snapshot":digest('f'),"sourceDataDigest":digest('a'),"examinedSourceDigest":digest('b'),
                    "sourceSpans":{span.clone():{"reference":"c1","sourceDigest":digest('e'),"textDigest":digest('d')}},
                    "sources":[],"nodes":[{"id":digest('b'),"symbol":digest('c'),"variableFactsDigest":digest('d'),
                        "callSites":[{"sourceSpan":span,"citationId":"c1","targetNode":digest('b')}],
                        "dataState":{"definitions":[{"id":digest('d'),"sourceSpan":"e".repeat(64),"storageRef":"d192","guardSetRef":"g1"}],
                            "gaps":[],"shared":{"storages":[{"id":"d192","variable":digest('f')} ]}}}] }},
            "selectionGuidance":{"symbols":[digest('d')],"query":{"symbolContains":digest('c')}},
            "outputSchema":{"oneOf":[{"type":"object","properties":{"action":{"const":"answer"},"answer":native_schema}}]}
        }})
    }

    fn reviewer_job() -> Value {
        let author = author_job();
        let mut properties = serde_json::Map::new();
        for (field, value) in [
            ("work", "a".repeat(64)),
            ("sourceRun", "b".repeat(32)),
            ("sourceInvocation", "f".repeat(32)),
            ("snapshot", digest('f')),
            ("packetDigest", digest('c')),
            ("answerDigest", digest('d')),
            ("coverageDigest", digest('e')),
            ("reviewContextDigest", digest('a')),
        ] {
            properties.insert(field.into(), json!({"type":"string","const":value}));
        }
        json!({"schema":"codeclew-documentation-agent-job/1.0","work":"a".repeat(64),"role":"reviewer","invocation":"e".repeat(32),
        "payload":{"instruction":"Review independently.","packet":author["payload"]["packet"],
            "savedAuthorContract":author["payload"],"answer":{"packetDigest":digest('c'),"title":digest('c')},
            "source":{"sourceRun":"b".repeat(32),"sourceInvocation":"f".repeat(32),"snapshot":digest('f'),"packetDigest":digest('c'),
                "answerDigest":digest('d'),"sourceInputDigest":digest('e'),"sourceResultDigest":digest('a'),
                "sourceCheckpoint":{"run":"b".repeat(32),"checkpointDigest":digest('a'),"sequence":1}},
            "reviewContext":{"work":"a".repeat(64),"snapshot":digest('f'),"deliveredDigest":digest('a'),"pages":[],"sourceParts":[]},
            "blocks":[{"id":"/summary","evidence":["c1","d192"],"text":digest('a')}],
                "outputSchema":{"$defs":{"review":{"properties":properties}},"oneOf":[{"properties":{"action":{"const":"review"},"review":{"$ref":"#/$defs/review"}}}]}
        }})
    }

    #[test]
    fn allowlisted_projection_preserves_source_semantics_queries_and_short_labels() {
        let job = author_job();
        let unchanged = job.clone();
        let prepared = prepare(&job, &scope("author"), None).unwrap();
        assert_eq!(job, unchanged);
        for pointer in [
            "/packet/citations",
            "/packet/methodSources/0/text",
            "/packet/methodSources/0/file",
            "/packet/graphAuditBinding/rootMethodId",
            "/packet/sourceDataContext/nodes/0/id",
            "/packet/sourceDataContext/nodes/0/symbol",
            "/packet/sourceDataContext/nodes/0/callSites/0/targetNode",
            "/packet/sourceDataContext/nodes/0/dataState/definitions/0/id",
            "/packet/sourceDataContext/nodes/0/dataState/definitions/0/storageRef",
            "/packet/sourceDataContext/nodes/0/dataState/definitions/0/guardSetRef",
            "/packet/sourceDataContext/nodes/0/dataState/shared",
            "/selectionGuidance",
        ] {
            assert_eq!(
                prepared.model_payload.pointer(pointer),
                job["payload"].pointer(pointer),
                "{pointer}"
            );
        }
        assert_ne!(
            prepared.model_payload["packet"]["packetDigest"],
            job["payload"]["packet"]["packetDigest"]
        );
        let projected_span = prepared.model_payload["packet"]["sourceDataContext"]["nodes"][0]["callSites"][0]["sourceSpan"].as_str().unwrap();
        assert!(projected_span.starts_with('p'));
        assert!(
            prepared.model_payload["packet"]["sourceDataContext"]["sourceSpans"]
                .get(projected_span)
                .is_some()
        );
        assert_eq!(
            prepared
                .map
                .entries
                .iter()
                .filter(|entry| entry.canonical == digest('f'))
                .map(|entry| entry.domain)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([Domain::Snapshot, Domain::Digest])
        );
        assert_ne!(
            prepared.model_payload["packet"]["methodSources"][0]["occurrence"]["snapshot"],
            prepared.model_payload["packet"]["methodSources"][0]["occurrence"]["blob"]
        );
        validate(&job, &prepared).unwrap();
    }

    #[test]
    fn schema_binding_patterns_and_generated_instruction_match_projected_packet() {
        let job = author_job();
        let prepared = prepare(&job, &scope("author"), None).unwrap();
        let alias = prepared.model_payload["packet"]["packetDigest"]
            .as_str()
            .unwrap();
        let binding = &prepared.output_schema["oneOf"][0]["properties"]["answer"]["properties"]["packetDigest"];
        assert_eq!(binding["const"], alias);
        assert_eq!(
            binding["pattern"],
            format!("^d[1-9][0-9]*_{}$", prepared.map.scope_tag)
        );
        assert!(
            prepared.model_payload["instruction"]
                .as_str()
                .unwrap()
                .contains(&format!("packetDigest exactly to `{alias}`"))
        );
        assert_eq!(
            prepared.output_schema,
            prepared.model_payload["outputSchema"]
        );
        let result = json!({"action":"answer","answer":{"packetDigest":alias,"title":alias,"evidence":["c1","d192"]}});
        let decoded = decode_result(&result, &prepared).unwrap();
        assert_eq!(decoded["answer"]["packetDigest"], digest('c'));
        assert_eq!(decoded["answer"]["title"], alias);
        assert_eq!(decoded["answer"]["evidence"], json!(["c1", "d192"]));
    }

    #[test]
    fn expansion_maps_append_without_renumbering_and_repeat_preparation_is_exact() {
        let job = author_job();
        let first = prepare(&job, &scope("author"), None).unwrap();
        let mut expanded = job.clone();
        expanded["invocation"] = json!("1".repeat(32));
        expanded["payload"]["packet"]["packetDigest"] = json!(digest('1'));
        expanded["payload"]["packet"]["contextDelivery"] = json!({"work":"a".repeat(64),"snapshot":digest('f'),"deliveredDigest":digest('2'),
            "pages":[{"receiptDigest":digest('3'),"items":[{"kind":"SOURCE","recordDigest":digest('4'),"record":{"text":"unchanged source","textDigest":digest('5')}}]}]});
        let next = prepare(&expanded, &scope("author"), Some(&first.map)).unwrap();
        assert_eq!(
            &next.map.entries[..first.map.entries.len()],
            first.map.entries.as_slice()
        );
        assert!(next.map.entries.len() > first.map.entries.len());
        assert_eq!(
            next,
            prepare(&expanded, &scope("author"), Some(&next.map)).unwrap()
        );
        assert_eq!(
            next.model_payload["packet"]["methodSources"],
            first.model_payload["packet"]["methodSources"]
        );
        assert_ne!(next.canonical_job_digest, first.canonical_job_digest);
    }

    #[test]
    fn reviewer_const_and_enum_bindings_round_trip_without_changing_coverage_paths() {
        let mut job = reviewer_job();
        job["payload"]["outputSchema"]["$defs"]["review"]["properties"]["work"] =
            json!({"type":"string","enum":["a".repeat(64)]});
        let prepared = prepare(&job, &scope("reviewer"), None).unwrap();
        let properties = &prepared.output_schema["$defs"]["review"]["properties"];
        let mut result = json!({"action":"review","review":{}});
        for field in [
            "work",
            "sourceRun",
            "sourceInvocation",
            "snapshot",
            "packetDigest",
            "answerDigest",
            "coverageDigest",
            "reviewContextDigest",
        ] {
            result["review"][field] = if field == "work" {
                properties[field]["enum"][0].clone()
            } else {
                properties[field]["const"].clone()
            };
        }
        result["review"]["assessedBlocks"] = json!(["/summary"]);
        result["review"]["assessedEvidence"] = json!(["c1", "d192"]);
        let decoded = decode_result(&result, &prepared).unwrap();
        assert_eq!(decoded["review"]["work"], "a".repeat(64));
        assert_eq!(decoded["review"]["sourceRun"], "b".repeat(32));
        assert_eq!(decoded["review"]["packetDigest"], digest('c'));
        assert_eq!(decoded["review"]["assessedBlocks"], json!(["/summary"]));
        assert_eq!(prepared.model_payload["blocks"], job["payload"]["blocks"]);
        assert_ne!(
            prepared.model_payload["source"]["sourceCheckpoint"]["checkpointDigest"],
            job["payload"]["source"]["sourceCheckpoint"]["checkpointDigest"]
        );
    }

    #[test]
    fn wrong_scope_unknown_and_wrong_domain_aliases_are_refused() {
        let job = author_job();
        let prepared = prepare(&job, &scope("author"), None).unwrap();
        let mut other_scope = scope("author");
        other_scope.run = "2".repeat(32);
        let other = prepare(&job, &other_scope, None).unwrap();
        assert_ne!(other.map.scope_tag, prepared.map.scope_tag);
        for alias in [
            other.model_payload["packet"]["packetDigest"].clone(),
            json!(format!("d9999_{}", prepared.map.scope_tag)),
            prepared.model_payload["packet"]["sourceDataContext"]["snapshot"].clone(),
            json!(digest('c')),
        ] {
            assert!(
                decode_result(
                    &json!({"action":"answer","answer":{"packetDigest":alias}}),
                    &prepared
                )
                .is_err()
            );
        }
        assert!(prepare(&job, &other_scope, Some(&prepared.map)).is_err());
        let mut reviewer = reviewer_job();
        reviewer["work"] = json!("a".repeat(64));
        assert!(prepare(&reviewer, &scope("reviewer"), Some(&prepared.map)).is_err());
        let expansion = json!({"action":"expand","selections":[{"references":["c1","d192"],"symbols":[digest('c')],"query":{"symbolContains":digest('d')}}]});
        assert_eq!(decode_result(&expansion, &prepared).unwrap(), expansion);
    }

    #[test]
    fn frozen_projection_tampering_is_refused_and_canonical_validation_runs_first() {
        let job = author_job();
        let prepared = prepare(&job, &scope("author"), None).unwrap();
        let called = Cell::new(false);
        let model = forward_model_input(&job, &prepared, |canonical| {
            called.set(true);
            assert_eq!(canonical, &job);
            Ok(())
        })
        .unwrap();
        assert!(called.get());
        assert_eq!(model.payload, prepared.model_payload);
        assert_eq!(model.output_schema, prepared.output_schema);
        assert!(model.payload.get("map").is_none());
        let mut changed = prepared.clone();
        changed.model_payload["packet"]["methodSources"][0]["text"] = json!("altered source");
        assert!(validate(&job, &changed).is_err());
        let error = forward_model_input(&job, &changed, |_| {
            Err(invalid("canonical-validator-first"))
        })
        .unwrap_err();
        assert!(error.message.contains("canonical-validator-first"));
        changed = prepared.clone();
        changed.map.entries[0].alias = "d1_foreign".into();
        assert!(validate(&job, &changed).is_err());
        changed = prepared.clone();
        changed.output_schema = json!({});
        assert!(validate(&job, &changed).is_err());
        let encoded = serde_json::to_vec(&prepared).unwrap();
        let restored: Prepared = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored, prepared);
        validate(&job, &restored).unwrap();
    }

    fn complete_delivery() -> Value {
        let pages = json!([{"work":"a".repeat(64),"snapshot":digest('f'),"receiptDigest":digest('1'),
            "items":[{"kind":"SOURCE","reference":"c1","recordDigest":digest('2'),
                "record":{"text":"def result():\n    return 7\n","file":"orders.py","textDigest":digest('3'),
                    "evidenceDigest":digest('4'),"occurrence":{"snapshot":digest('f'),"blob":digest('5'),"startByte":0,"endByte":27}}}]}]);
        let parts = json!([{"work":"a".repeat(64),"snapshot":digest('f'),"reference":"c1","text":"return 7",
            "recordDigest":digest('6'),"fragmentDigest":digest('7'),"receiptDigest":digest('8'),"startByte":18,"endByte":26,
            "source":{"textDigest":digest('3'),"evidenceDigest":digest('4'),"file":"orders.py"}}]);
        let presentation = super::super::job_context::present(
            pages.as_array().unwrap(),
            parts.as_array().unwrap(),
        );
        json!({"work":"a".repeat(64),"snapshot":digest('f'),"deliveredDigest":digest('9'),
            "pages":pages,"sourceParts":parts,
            "presentation":presentation,
            "pageReceipts":[{"resultDigest":digest('1'),"membershipDigest":digest('a'),"supplied":["c1"]}],
            "sourcePartReceipts":[{"work":"a".repeat(64),"snapshot":digest('f'),"receiptDigest":digest('8'),"reference":"c1"}],
            "citations":{"c1":"SOURCE"}})
    }

    #[test]
    fn legacy_version_retains_exact_arrays_and_frozen_records_remain_replayable() {
        let mut job = author_job();
        job["payload"]["packet"]["contextDelivery"] = complete_delivery();
        let legacy = prepare(&job, &scope("author"), None).unwrap();
        assert_eq!(legacy.version, VERSION);
        assert_eq!(legacy.map.version, VERSION);
        // Frozen 1.0 scope hashing keeps its original versioned hash preimage.
        assert_eq!(legacy.map.scope_tag, "7f2ef0d6a8f07c76");
        let delivery = &legacy.model_payload["packet"]["contextDelivery"];
        assert_eq!(delivery["pages"], delivery["presentation"]["pages"]);
        assert_eq!(
            delivery["sourceParts"],
            delivery["presentation"]["sourceParts"]
        );
        let bytes = canonical::bytes(&legacy).unwrap();
        let frozen: Prepared = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            legacy,
            prepare_with_version(&job, &scope("author"), None, VERSION).unwrap()
        );
        assert_eq!(
            legacy,
            prepare(&job, &scope("author"), Some(&frozen.map)).unwrap()
        );
        validate(&job, &frozen).unwrap();
        let forwarded = forward_model_input(&job, &frozen, |_| Ok(())).unwrap();
        assert_eq!(
            canonical::bytes(&forwarded.payload).unwrap(),
            canonical::bytes(&legacy.model_payload).unwrap()
        );
        assert_eq!(
            canonical::bytes(&forwarded.output_schema).unwrap(),
            canonical::bytes(&legacy.output_schema).unwrap()
        );
        assert!(
            !forwarded.payload["instruction"]
                .as_str()
                .unwrap()
                .contains("When raw delivery arrays are omitted")
        );
    }

    #[test]
    fn compact_version_preserves_complete_presentation_metadata_and_canonical_results() {
        for role in ["author", "reviewer"] {
            let mut job = if role == "author" {
                author_job()
            } else {
                reviewer_job()
            };
            job["payload"]["packet"]["contextDelivery"] = complete_delivery();
            if role == "reviewer" {
                job["payload"]["reviewContext"] = complete_delivery();
            }
            let unchanged = job.clone();
            let legacy = prepare(&job, &scope(role), None).unwrap();
            let compact = prepare_with_version(&job, &scope(role), None, COMPACT_VERSION).unwrap();
            assert_eq!(job, unchanged);
            assert_eq!(compact.version, COMPACT_VERSION);
            assert_eq!(compact.map.version, COMPACT_VERSION);
            for pointer in ["/packet/contextDelivery", "/reviewContext"] {
                if job["payload"].pointer(pointer).is_none() {
                    continue;
                }
                // Use the same typed projection to establish the complete
                // expected evidence, then remove only its redundant raw arrays.
                let mut expected = job["payload"].pointer(pointer).unwrap().clone();
                delivery(&mut expected, &mut compact.map.clone()).unwrap();
                expected.as_object_mut().unwrap().remove("pages");
                expected.as_object_mut().unwrap().remove("sourceParts");
                let projected = compact.model_payload.pointer(pointer).unwrap();
                assert_eq!(projected, &expected);
                assert!(projected.get("pages").is_none());
                assert!(projected.get("sourceParts").is_none());
                assert_eq!(
                    projected["presentation"]["pages"][0]["items"][0]["record"]["text"],
                    "def result():\n    return 7\n"
                );
                assert_eq!(
                    projected["presentation"]["sourceParts"][0]["text"],
                    "return 7"
                );
                assert_eq!(projected["citations"], json!({"c1":"SOURCE"}));
            }
            assert_eq!(compact.canonical_job_digest, legacy.canonical_job_digest);
            let result_key = if role == "author" { "answer" } else { "review" };
            let make_result = |prepared: &Prepared| {
                let mut result = json!({"action":result_key,result_key:{"packetDigest":prepared.model_payload["packet"]["packetDigest"],"title":"unchanged candidate"}});
                if role == "reviewer" {
                    let properties = &prepared.output_schema["$defs"]["review"]["properties"];
                    for field in [
                        "work",
                        "sourceRun",
                        "sourceInvocation",
                        "snapshot",
                        "answerDigest",
                        "coverageDigest",
                        "reviewContextDigest",
                    ] {
                        result[result_key][field] = properties[field]["const"].clone();
                    }
                }
                result
            };
            assert_eq!(
                decode_result(&make_result(&compact), &compact).unwrap(),
                decode_result(&make_result(&legacy), &legacy).unwrap()
            );
            validate(&job, &compact).unwrap();
            let forwarded = forward_model_input(&job, &compact, |_| Ok(())).unwrap();
            assert_eq!(forwarded.payload, compact.model_payload);
            assert_eq!(forwarded.output_schema, compact.output_schema);
            assert!(
                compact.model_payload["instruction"]
                    .as_str()
                    .unwrap()
                    .contains("When raw delivery arrays are omitted")
            );
        }
    }

    #[test]
    fn compact_version_refuses_incomplete_or_changed_presentations_and_retains_absent_ones() {
        for malformed in 0..5 {
            let mut job = author_job();
            let mut context = complete_delivery();
            match malformed {
                0 => {
                    context["presentation"]["pages"][0]["items"][0]["record"]["text"] =
                        json!("different source")
                }
                1 => context["presentation"]["sourceParts"] = json!([]),
                2 => {
                    context["presentation"]
                        .as_object_mut()
                        .unwrap()
                        .remove("pages");
                }
                3 => context["presentation"] = Value::Null,
                _ => context["presentation"]["callablesMeaning"] = json!("unverified metadata"),
            }
            job["payload"]["packet"]["contextDelivery"] = context;
            let unchanged = job.clone();
            assert!(prepare_with_version(&job, &scope("author"), None, COMPACT_VERSION).is_err());
            assert_eq!(job, unchanged);
            // Unsupported compaction never affects legacy decoding or replay.
            let legacy = prepare(&job, &scope("author"), None).unwrap();
            validate(&job, &legacy).unwrap();
        }
        let mut job = author_job();
        let mut context = complete_delivery();
        context.as_object_mut().unwrap().remove("presentation");
        job["payload"]["packet"]["contextDelivery"] = context;
        let compact = prepare_with_version(&job, &scope("author"), None, COMPACT_VERSION).unwrap();
        assert!(compact.model_payload["packet"]["contextDelivery"]["pages"].is_array());
        assert!(compact.model_payload["packet"]["contextDelivery"]["sourceParts"].is_array());
        validate(&job, &compact).unwrap();
    }

    #[test]
    fn compact_version_accepts_native_retained_duplicate_locations_without_rewriting_them() {
        let mut job = author_job();
        let mut context = complete_delivery();
        let coverage = json!({"kind":"COVERAGE","reference":"d192","recordDigest":digest('a'),"record":{"coverage":"PARTIAL","boundaries":["UNKNOWN_RUNTIME"]}});
        let pages = context["pages"].as_array_mut().unwrap();
        pages[0]["items"]
            .as_array_mut()
            .unwrap()
            .push(coverage.clone());
        pages.push(json!({"receiptDigest":digest('b'),"items":[coverage]}));
        context["presentation"] = super::super::job_context::present(
            context["pages"].as_array().unwrap(),
            context["sourceParts"].as_array().unwrap(),
        );
        assert_ne!(context["pages"], context["presentation"]["pages"]);
        assert_eq!(
            context["sourceParts"],
            context["presentation"]["sourceParts"]
        );
        let retained_at = context["presentation"]["pages"][1]["displayProjection"]["omittedDuplicates"][0]["retainedAt"].clone();
        assert_eq!(
            retained_at,
            json!({"pageIndex":0,"itemIndex":1,"sourceItemIndex":1})
        );
        job["payload"]["packet"]["contextDelivery"] = context.clone();
        let compact = prepare_with_version(&job, &scope("author"), None, COMPACT_VERSION).unwrap();
        let mut expected = context.clone();
        delivery(&mut expected, &mut compact.map.clone()).unwrap();
        expected.as_object_mut().unwrap().remove("pages");
        expected.as_object_mut().unwrap().remove("sourceParts");
        assert_eq!(compact.model_payload["packet"]["contextDelivery"], expected);
        assert_eq!(
            compact.model_payload["packet"]["contextDelivery"]["presentation"]["pages"][1]["displayProjection"]
                ["omittedDuplicates"][0]["retainedAt"],
            retained_at
        );
        validate(&job, &compact).unwrap();
        // A forged duplicate pointer is rejected even though the retained
        // source text and sourceParts remain otherwise unchanged.
        job["payload"]["packet"]["contextDelivery"]["presentation"]["pages"][1]["displayProjection"]
            ["omittedDuplicates"][0]["retainedAt"]["itemIndex"] = json!(0);
        assert!(prepare_with_version(&job, &scope("author"), None, COMPACT_VERSION).is_err());
    }

    #[test]
    fn representation_versions_cannot_share_maps_or_result_aliases() {
        let job = author_job();
        let scoped = scope("author");
        let legacy = prepare(&job, &scoped, None).unwrap();
        let compact = prepare_with_version(&job, &scoped, None, COMPACT_VERSION).unwrap();
        assert_ne!(legacy.map.scope_tag, compact.map.scope_tag);
        assert!(prepare_with_version(&job, &scoped, Some(&legacy.map), COMPACT_VERSION).is_err());
        assert!(prepare_with_version(&job, &scoped, Some(&compact.map), VERSION).is_err());
        for (wire, prepared) in [(&legacy, &compact), (&compact, &legacy)] {
            assert!(decode_result(&json!({"action":"answer","answer":{"packetDigest":wire.model_payload["packet"]["packetDigest"]}}), prepared).is_err());
        }
        let mut mismatched = compact.clone();
        mismatched.version = VERSION.into();
        assert!(validate(&job, &mismatched).is_err());
        assert!(decode_result(&json!({"action":"expand","selections":[]}), &mismatched).is_err());
        assert!(supported_version(VERSION));
        assert!(supported_version(COMPACT_VERSION));
        assert!(!supported_version("codeclew-model-ids/2.0"));
        assert!(prepare_with_version(&job, &scoped, None, "codeclew-model-ids/2.0").is_err());
        let mut expanded = job.clone();
        expanded["payload"]["packet"]["contextDelivery"] = complete_delivery();
        let next =
            prepare_with_version(&expanded, &scoped, Some(&compact.map), COMPACT_VERSION).unwrap();
        assert_eq!(
            &next.map.entries[..compact.map.entries.len()],
            compact.map.entries.as_slice()
        );
    }
}
