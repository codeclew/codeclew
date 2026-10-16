//! Optional editorial reading plan over immutable retained publication evidence.
use super::super::{check::Check, invalid, io_error, store};
use crate::error::ClewError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    schema: String,
    publication: String,
    subject: String,
    title: String,
    purpose: String,
    #[serde(default)]
    presentation: Option<String>,
    #[serde(default)]
    language: Option<String>,
    sections: Vec<Section>,
    conclusion: Block,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Section {
    id: String,
    title: String,
    #[serde(default)]
    nav_title: Option<String>,
    #[serde(default)]
    number: Option<u8>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    route: Option<String>,
    blocks: Vec<Block>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Block {
    title: String,
    #[serde(default)]
    nav_title: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    tone: Option<String>,
    #[serde(default)]
    disclosure: Option<String>,
    #[serde(default)]
    preview: Option<String>,
    text: String,
    evidence: Vec<Reference>,
    #[serde(default)]
    links: Vec<Link>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Link {
    subject: String,
    operation: String,
    label: String,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
enum Reference {
    #[serde(rename_all = "camelCase")]
    Retained {
        subject: String,
        operation: String,
        fragment_id: String,
        text_digest: String,
    },
    #[serde(rename_all = "camelCase")]
    Source {
        service: String,
        source_id: String,
        text_digest: String,
        start_line: u64,
        end_line: u64,
    },
}
fn bounded(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit
}
fn page(files: &BTreeMap<String, Vec<u8>>, subject: &str) -> Result<Value, ClewError> {
    let (kind, id) = subject
        .split_once(':')
        .ok_or_else(|| invalid("reading-plan subject is invalid"))?;
    let directory = match kind {
        "scenario" => "scenarios",
        "service" => "services",
        _ => return Err(invalid("reading-plan subject kind is invalid")),
    };
    if !store::valid_id(id) {
        return Err(invalid("reading-plan subject identity is invalid"));
    }
    let bytes = files
        .get(&format!("{directory}/{id}.json"))
        .ok_or_else(|| invalid("reading-plan subject is absent from publication"))?;
    serde_json::from_slice(bytes).map_err(io_error)
}
fn operation<'a>(page: &'a Value, id: &str) -> Result<&'a Value, ClewError> {
    page["operations"]
        .as_array()
        .and_then(|rows| rows.iter().find(|op| op["id"] == id))
        .ok_or_else(|| invalid("reading-plan operation is absent from publication"))
}
fn source_record(source: &Value, bytes: &mut usize) -> Result<(), ClewError> {
    let text = source["text"]
        .as_str()
        .ok_or_else(|| invalid("reading-plan source is unavailable"))?;
    if source["textDigest"] != crate::canonical::hash_bytes(text.as_bytes()) {
        return Err(invalid("reading-plan source digest differs"));
    }
    *bytes += text.len();
    if *bytes > 2 * 1024 * 1024 {
        return Err(invalid("reading-plan evidence exceeds 2 MiB"));
    }
    Ok(())
}
fn project_block(
    block: Block,
    files: &BTreeMap<String, Vec<u8>>,
    checked: &Check,
    bytes: &mut usize,
) -> Result<Value, ClewError> {
    if !bounded(&block.title, 256)
        || !bounded(&block.text, 2048)
        || block.evidence.is_empty()
        || block.evidence.len() > 8
        || block.links.len() > 8
        || block.nav_title.as_deref().is_some_and(|s| !bounded(s, 256))
        || block.id.as_deref().is_some_and(|s| !store::valid_id(s))
        || block
            .tone
            .as_deref()
            .is_some_and(|s| !matches!(s, "NORMAL" | "ERROR" | "WARNING" | "INFO"))
        || block
            .disclosure
            .as_deref()
            .is_some_and(|s| !bounded(s, 256))
        || block.preview.as_deref().is_some_and(|s| !bounded(s, 512))
    {
        return Err(invalid("reading-plan block bounds or evidence are invalid"));
    }
    let mut evidence = Vec::new();
    for reference in block.evidence {
        match reference {
            Reference::Retained {
                subject,
                operation: owner,
                fragment_id,
                text_digest,
            } => {
                let p = page(files, &subject)?;
                let op = operation(&p, &owner)?;
                let fragments = std::iter::once(&op["summary"]).chain(
                    ["events", "explanation", "findings"]
                        .into_iter()
                        .flat_map(|key| op[key].as_array().into_iter().flatten()),
                );
                let matches: Vec<_> = fragments.filter(|f| f["id"] == fragment_id).collect();
                let [fragment] = matches.as_slice() else {
                    return Err(invalid(
                        "reading-plan fragment must identify one retained claim",
                    ));
                };
                let text = fragment["text"]
                    .as_str()
                    .ok_or_else(|| invalid("reading-plan retained text is absent"))?;
                if crate::canonical::hash_bytes(text.as_bytes()) != text_digest {
                    return Err(invalid("reading-plan claim text changed"));
                }
                let mut sources = BTreeMap::new();
                for id in fragment["sourceIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    let source = &p["operationSources"][&owner][id];
                    source_record(source, bytes)?;
                    sources.insert(id.to_owned(), source.clone());
                }
                if sources.is_empty() {
                    return Err(invalid("reading-plan retained claim has no exact source"));
                }
                evidence.push(json!({"kind":"RETAINED_CLAIM","subject":subject,"operation":owner,"title":op["title"],"fragment":fragment,"sources":sources,"state":p["operationStates"][&owner],"sourceContext":{"publication":p["frozenPublication"],"revisions":p["revisions"],"boundaries":op["boundaries"]}}));
            }
            Reference::Source {
                service,
                source_id,
                text_digest,
                start_line,
                end_line,
            } => {
                let source = checked
                    .services
                    .get(&service)
                    .and_then(|s| s.sources.get(&source_id))
                    .ok_or_else(|| invalid("reading-plan selected source is unavailable"))?;
                if source.text_digest != text_digest
                    || start_line < source.start_line
                    || end_line > source.end_line
                    || end_line < start_line
                {
                    return Err(invalid("reading-plan source range or digest changed"));
                }
                let record = json!(source);
                source_record(&record, bytes)?;
                evidence.push(json!({"kind":"SELECTED_SOURCE","source":record,"startLine":start_line,"endLine":end_line,"authority":"SELECTED_SAVED_SOURCE_NOT_RUNTIME_PROOF"}));
            }
        }
    }
    let mut links = Vec::new();
    for link in block.links {
        let p = page(files, &link.subject)?;
        operation(&p, &link.operation)?;
        if !bounded(&link.label, 256) {
            return Err(invalid("reading-plan link label is invalid"));
        }
        let (kind, id) = link.subject.split_once(':').unwrap();
        let directory = if kind == "service" {
            "services"
        } else {
            "scenarios"
        };
        links.push(json!({"href":format!("../{directory}/{id}.html#{}",link.operation),"label":link.label,"subject":link.subject,"operation":link.operation}));
    }
    Ok(
        json!({"id":block.id,"navTitle":block.nav_title,"tone":block.tone,"disclosure":block.disclosure,"preview":block.preview,"title":block.title,"text":block.text,"evidence":evidence,"links":links,"authority":"EDITORIAL_READING_GUIDE_NOT_ACCEPTED_PROSE"}),
    )
}
pub(super) fn load(
    path: Option<&Path>,
    publication: &str,
    files: &BTreeMap<String, Vec<u8>>,
    checked: &Check,
) -> Result<BTreeMap<String, Value>, ClewError> {
    let Some(path) = path else {
        return Ok(BTreeMap::new());
    };
    let plan: Plan = store::read(path, 128 * 1024)?;
    if plan.schema != "codeclew-question-reading-plan/1.0"
        || plan.publication != publication
        || !plan.subject.starts_with("scenario:")
        || !bounded(&plan.title, 256)
        || !bounded(&plan.purpose, 512)
        || !(2..=12).contains(&plan.sections.len())
        || plan
            .presentation
            .as_deref()
            .is_some_and(|s| !matches!(s, "QUESTIONS" | "SEQUENTIAL"))
        || plan
            .language
            .as_deref()
            .is_some_and(|s| !matches!(s, "en" | "ru"))
    {
        return Err(invalid(
            "reading-plan must bind a publication and bounded process reading path",
        ));
    }
    if plan.presentation.as_deref() == Some("SEQUENTIAL") {
        let (last, steps) = plan.sections.split_last().unwrap();
        if last.id != "confidence"
            || last.number.is_some()
            || steps.iter().any(|step| step.number.is_none())
        {
            return Err(invalid(
                "sequential reading-plan requires numbered steps followed by confidence",
            ));
        }
    }
    let owner = page(files, &plan.subject)?;
    if owner["process"].is_null() {
        return Err(invalid("reading-plan requires a saved process"));
    }
    let mut sections = Vec::new();
    let mut ids = BTreeSet::new();
    let mut bytes = 0;
    let mut next_step = 1;
    for section in plan.sections {
        if !store::valid_id(&section.id)
            || !ids.insert(section.id.clone())
            || !bounded(&section.title, 256)
            || section.blocks.is_empty()
            || section.blocks.len() > 16
            || section
                .nav_title
                .as_deref()
                .is_some_and(|s| !bounded(s, 256))
            || section.route.as_deref().is_some_and(|s| !bounded(s, 256))
            || section
                .icon
                .as_deref()
                .is_some_and(|s| !matches!(s, "arrow-right" | "database"))
        {
            return Err(invalid(
                "reading-plan section bounds or identity are invalid",
            ));
        }
        if let Some(number) = section.number {
            if number != next_step {
                return Err(invalid(
                    "reading-plan step numbers must follow reading order",
                ));
            }
            next_step += 1;
        }
        for block in &section.blocks {
            if let Some(id) = &block.id {
                if !ids.insert(id.clone()) {
                    return Err(invalid("reading-plan anchor identity must be unique"));
                }
            }
        }
        let blocks = section
            .blocks
            .into_iter()
            .map(|b| project_block(b, files, checked, &mut bytes))
            .collect::<Result<Vec<_>, _>>()?;
        sections.push(json!({"id":section.id,"title":section.title,"navTitle":section.nav_title,"number":section.number,"icon":section.icon,"route":section.route,"blocks":blocks}));
    }
    let conclusion = project_block(plan.conclusion, files, checked, &mut bytes)?;
    Ok(BTreeMap::from([(
        plan.subject.clone(),
        json!({"schema":plan.schema,"presentation":plan.presentation,"language":plan.language.unwrap_or_else(|| "en".into()),"subject":plan.subject,"title":plan.title,"purpose":plan.purpose,"sections":sections,"conclusion":conclusion,"authority":"EDITORIAL_READING_GUIDE_NOT_ACCEPTED_PROSE","publication":publication}),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check() -> Check {
        serde_json::from_value(json!({"schema":"codeclew-documentation-check/1.0","inputDigest":"input","contextDigest":"context","services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap()
    }
    fn fixture() -> (Value, BTreeMap<String, Vec<u8>>) {
        let source = json!({"text":"saved bytes", "textDigest":crate::canonical::hash_bytes(b"saved bytes"),"revision":"old-revision"});
        let fragment =
            json!({"id":"claim","text":"Retained explanation", "sourceIds":["saved-source"]});
        let page = json!({"subject":"scenario:example","process":{},"operations":[{"id":"saved-operation","summary":fragment,"events":[],"explanation":[],"findings":[]}],"operationSources":{"saved-operation":{"saved-source":source}},"operationStates":{"saved-operation":{"freshness":"STALE","verification":"UNASSESSED"}}});
        let evidence = json!({"kind":"RETAINED","subject":"scenario:example","operation":"saved-operation","fragmentId":"claim","textDigest":crate::canonical::hash_bytes(b"Retained explanation")});
        let block = json!({"title":"What happens","text":"Editorial description","evidence":[evidence],"links":[{"subject":"scenario:example","operation":"saved-operation","label":"Original account"}]});
        let plan = json!({"schema":"codeclew-question-reading-plan/1.0","publication":"frozen-publication","subject":"scenario:example","title":"Question reader","purpose":"A bounded guide", "sections":[{"id":"story","title":"What happens?","blocks":[block]},{"id":"confidence","title":"Source and confidence","blocks":[block]}],"conclusion":block});
        (
            plan,
            BTreeMap::from([(
                "scenarios/example.json".to_owned(),
                serde_json::to_vec(&page).unwrap(),
            )]),
        )
    }
    fn load_plan(
        plan: &Value,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<BTreeMap<String, Value>, ClewError> {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reading-plan.json");
        std::fs::write(&path, serde_json::to_vec(plan).unwrap()).unwrap();
        load(Some(&path), "frozen-publication", files, &check())
    }
    #[test]
    fn projects_exact_retained_claims_without_mutating_publication() {
        let (plan, files) = fixture();
        let original = files.clone();
        let result = load_plan(&plan, &files).unwrap();
        let guide = &result["scenario:example"];
        assert_eq!(
            guide["authority"],
            "EDITORIAL_READING_GUIDE_NOT_ACCEPTED_PROSE"
        );
        let evidence = &guide["sections"][0]["blocks"][0]["evidence"][0];
        assert_eq!(evidence["fragment"]["text"], "Retained explanation");
        assert_eq!(evidence["state"]["freshness"], "STALE");
        assert_eq!(
            evidence["sources"]["saved-source"]["revision"],
            "old-revision"
        );
        assert_eq!(
            guide["sections"][0]["blocks"][0]["links"][0]["href"],
            "../scenarios/example.html#saved-operation"
        );
        assert_eq!(original, files);
        assert!(
            load(None, "other-publication", &files, &check())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn rejects_changed_claim_source_publication_and_unsafe_navigation() {
        let (plan, files) = fixture();
        for (pointer, replacement) in [
            ("/publication", json!("different-publication")),
            (
                "/sections/0/blocks/0/evidence/0/textDigest",
                json!("changed"),
            ),
            (
                "/sections/0/blocks/0/evidence/0/fragmentId",
                json!("missing"),
            ),
            (
                "/sections/0/blocks/0/links/0/subject",
                json!("service:../../escape"),
            ),
            ("/sections/0/blocks/0/links/0/operation", json!("missing")),
            ("/sections/1/id", json!("story")),
            ("/sections/0/blocks/0/evidence", json!([])),
        ] {
            let mut changed = plan.clone();
            *changed.pointer_mut(pointer).unwrap() = replacement;
            assert!(load_plan(&changed, &files).is_err(), "{pointer}");
        }
        let mut missing_source = files.clone();
        let mut page: Value =
            serde_json::from_slice(&missing_source["scenarios/example.json"]).unwrap();
        page["operationSources"]["saved-operation"] = json!({});
        missing_source.insert(
            "scenarios/example.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        assert!(load_plan(&plan, &missing_source).is_err());
        let mut changed_source = files.clone();
        let mut page: Value =
            serde_json::from_slice(&changed_source["scenarios/example.json"]).unwrap();
        page["operationSources"]["saved-operation"]["saved-source"]["text"] =
            json!("different bytes");
        changed_source.insert(
            "scenarios/example.json".into(),
            serde_json::to_vec(&page).unwrap(),
        );
        assert!(load_plan(&plan, &changed_source).is_err());
    }
    #[test]
    fn sequential_plan_retains_editorial_locale_and_rejects_broken_structure() {
        let (mut plan, files) = fixture();
        plan["presentation"] = json!("SEQUENTIAL");
        plan["language"] = json!("ru");
        plan["sections"][0]["number"] = json!(1);
        plan["sections"][0]["icon"] = json!("database");
        plan["sections"][0]["blocks"][0]["id"] = json!("exchange-error");
        plan["sections"][0]["blocks"][0]["tone"] = json!("ERROR");
        let projected = load_plan(&plan, &files).unwrap();
        assert_eq!(projected["scenario:example"]["language"], "ru");
        assert_eq!(
            projected["scenario:example"]["sections"][0]["blocks"][0]["id"],
            "exchange-error"
        );
        for (pointer, replacement) in [
            ("/presentation", json!("CUSTOM_HTML")),
            ("/language", json!("unsupported")),
            ("/sections/0/number", json!(2)),
            ("/sections/0/icon", json!("<svg/>")),
            ("/sections/0/blocks/0/id", json!("confidence")),
            ("/sections/0/blocks/0/tone", json!("<script>")),
            ("/sections/1/id", json!("no-source-destination")),
        ] {
            let mut changed = plan.clone();
            *changed.pointer_mut(pointer).unwrap() = replacement;
            assert!(load_plan(&changed, &files).is_err(), "{pointer}");
        }
    }
}
