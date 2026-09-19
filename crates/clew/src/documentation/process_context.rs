//! Compact process views over retained evidence. Deferred handles are not marked supplied.
use super::{invalid, model::Source};
use crate::error::ClewError;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE: &str = "process-v1";

/// Offsets refer to UTF-8 bytes in another retained text, never to source-file offsets.
fn covered_text(container: &Source, part: &Source) -> Option<(usize, usize)> {
    let same_occurrence = match (&container.occurrence, &part.occurrence) {
        (Some(a), Some(b)) => a.snapshot == b.snapshot && a.blob == b.blob,
        (None, None) => container.evidence_digest == part.evidence_digest,
        _ => false,
    };
    if !same_occurrence
        || container.service != part.service
        || container.revision != part.revision
        || container.file != part.file
        || container.authority != part.authority
        || container.start_line > part.start_line
        || container.end_line < part.end_line
        || part.text.is_empty()
    {
        return None;
    }
    container
        .text
        .match_indices(&part.text)
        .find_map(|(start, _)| {
            let line = container.start_line
                + container.text[..start]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count() as u64;
            (line == part.start_line).then_some((start, start + part.text.len()))
        })
}

pub(super) fn project(rows: Vec<Value>) -> Result<Vec<Value>, ClewError> {
    let mut source_rows: Vec<_> = rows
        .iter()
        .filter(|row| row["kind"] == "SOURCE")
        .map(|row| {
            let source: Source = serde_json::from_value(row["record"].clone()).map_err(|_| {
                invalid("process context requires a complete retained source record")
            })?;
            let reference = row["reference"]
                .as_str()
                .ok_or_else(|| invalid("process source lacks a registered reference"))?
                .to_owned();
            Ok((reference, source))
        })
        .collect::<Result<_, ClewError>>()?;
    source_rows.sort_by(|a, b| b.1.text.len().cmp(&a.1.text.len()).then(a.0.cmp(&b.0)));
    let mut complete: Vec<(String, Source)> = Vec::new();
    let mut aliases = Vec::new();
    let mut body_reference = BTreeMap::new();
    for (reference, source) in source_rows {
        if let Some((container, (start, end))) =
            complete.iter().find_map(|(reference, container)| {
                covered_text(container, &source).map(|range| (reference, range))
            })
        {
            body_reference.insert(reference.clone(), container.clone());
            aliases.push(
                json!({"deferredReference":reference,"sourceId":source.id,"file":source.file,
                "startLine":source.start_line,"endLine":source.end_line,
                "textFrom":{"reference":container,"startByte":start,"endByte":end}}),
            );
        } else {
            body_reference.insert(reference.clone(), reference.clone());
            complete.push((reference, source));
        }
    }
    let kept_sources: BTreeSet<_> = complete
        .iter()
        .map(|(reference, _)| reference.as_str())
        .collect();
    let flow_references: BTreeSet<_> = rows
        .iter()
        .filter(|row| row["kind"] == "DEPENDENCY" && row["record"]["kind"] == "FLOW")
        .filter_map(|row| row["reference"].as_str())
        .collect();
    let mut projected = Vec::new();
    let mut order = Vec::new();
    let mut deferred_symbols = 0;
    for row in &rows {
        if row["kind"] == "SOURCE"
            && !kept_sources.contains(row["reference"].as_str().unwrap_or(""))
        {
            continue;
        }
        if row["kind"] == "DEPENDENCY" && row["record"]["kind"] == "SYMBOL" {
            let record = &row["record"];
            let sources: BTreeSet<_> = row["sourceReferences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|reference| body_reference.get(reference))
                .collect();
            projected.push(json!({"kind":"CALLABLE_SUMMARY","id":row["id"],"referenceRoles":[],
                "record":{"symbol":record["symbol"],"service":record["service"],"scope":record["normalized"]["scope"],
                    "name":record["normalized"]["name"],"owner":record["normalized"]["ownerIdentity"],
                    "boundaries":record["normalized"].pointer("/documentation/boundaries"),
                    "fullRecordReference":row["reference"],"sourceReferences":sources,
                    "authority":"NAVIGATION_SUMMARY_NOT_A_SUPPLIED_PROVIDER_FACT"}}));
            deferred_symbols += 1;
        } else if row["kind"] == "FLOW_STEP" {
            let references: Vec<_> = row["dependencyReferences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|reference| flow_references.contains(reference))
                .collect();
            if references.is_empty() {
                projected.push(row.clone());
            } else {
                order.push(json!({"id":row["id"],"flowReferences":references,"depth":row["record"]["depth"]}));
            }
        } else {
            projected.push(row.clone());
        }
    }
    projected.push(
        json!({"kind":"PROCESS_FLOW_ORDER","id":"process-flow-order","referenceRoles":[],
        "record":{"authority":"RECORDED_STATIC_STRUCTURE_NOT_EXECUTED_TRACE","steps":order}}),
    );
    projected.push(json!({"kind":"SOURCE_TEXT_ALIASES","id":"source-text-aliases","referenceRoles":[],
        "record":{"aliases":aliases,"authority":"EXACT_TEXT_SHARING_ONLY_ORIGINAL_SOURCE_IDENTITIES_REMAIN_DISTINCT",
            "instruction":"Alias offsets are UTF-8 bytes within a complete source record's text. An alias is not a supplied source handle. Cite the complete body reference, or expand the deferred reference to read the original source record."}}));
    projected.push(json!({"kind":"CONTEXT_PROFILE","id":"context-profile:process-v1","referenceRoles":[],
        "record":{"profile":PROFILE,"deferredSymbolCount":deferred_symbols,"completeSourceCount":complete.len(),
            "instruction":"Explain the selected process using complete retained method bodies and fully supplied FLOW facts. CALLABLE_SUMMARY is navigation only: fullRecordReference is not supplied and must be expanded before citing provider fields. Preserve control and exception branches from source; unresolved boundaries do not establish runtime execution. Use query kind SYMBOL with symbolContains to discover additional retained evidence, then expand its returned references. No source acquisition is performed.",
            "expansion":{"query":{"kind":"SYMBOL","symbolContains":""}}}}));
    Ok(projected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(id: &str, text: &str, start: u64, end: u64) -> Source {
        serde_json::from_value(json!({"id":id,"service":"svc","revision":"rev","file":"Worker.java","startLine":start,"endLine":end,"text":text,"textDigest":"hash","evidenceDigest":"hash","authority":"COMPILER"})).unwrap()
    }
    #[test]
    fn source_sharing_preserves_exact_utf8_and_occurrence_binding() {
        let body = source("body", "first\nαβ\nlast", 10, 12);
        assert_eq!(
            covered_text(&body, &source("part", "αβ", 11, 11)),
            Some((6, 10))
        );
        assert_eq!(&body.text[6..10], "αβ");
        assert_eq!(
            covered_text(&body, &source("wrong-line", "αβ", 12, 12)),
            None
        );
        let mut other = source("other", "αβ", 11, 11);
        other.revision = "other".into();
        assert_eq!(covered_text(&body, &other), None);
        assert_eq!(
            covered_text(&body, &source("different", "αβ\r", 11, 11)),
            None
        );
    }
    #[test]
    fn identical_inner_text_does_not_share_across_source_occurrences() {
        let mut body = source("body", "first\nbody\nlast", 1, 3);
        let mut part = source("part", "body", 2, 2);
        part.evidence_digest = "another-file".into();
        assert_eq!(covered_text(&body, &part), None);
        body.occurrence = Some(super::super::model::SourceOccurrence {
            snapshot: "generation-a".into(),
            blob: "blob-a".into(),
            start_byte: 0,
            end_byte: 15,
        });
        assert_eq!(covered_text(&body, &part), None);
        part.occurrence = Some(super::super::model::SourceOccurrence {
            snapshot: "generation-a".into(),
            blob: "blob-a".into(),
            start_byte: 6,
            end_byte: 10,
        });
        assert_eq!(covered_text(&body, &part), Some((6, 10)));
        part.occurrence.as_mut().unwrap().blob = "blob-b".into();
        assert_eq!(covered_text(&body, &part), None);
        part.occurrence.as_mut().unwrap().blob = "blob-a".into();
        part.occurrence.as_mut().unwrap().snapshot = "generation-b".into();
        assert_eq!(covered_text(&body, &part), None);
    }
    #[test]
    fn partial_symbol_and_text_alias_never_authorize_undelivered_handles() {
        let symbol = json!({"kind":"DEPENDENCY","id":"symbol","reference":"d1","sourceReferences":["s2"],"record":{"kind":"SYMBOL","symbol":"worker","normalized":{"name":"run","sourceTokens":"hidden","documentation":{"events":[]}}}});
        let flow = json!({"kind":"DEPENDENCY","id":"flow","reference":"d2","record":{"kind":"FLOW","normalized":{"kind":"IF","condition":"ready"}}});
        let rows = vec![
            symbol,
            flow.clone(),
            json!({"kind":"SOURCE","id":"body","reference":"s1","record":source("body","first\nbody\nlast",1,3)}),
            json!({"kind":"SOURCE","id":"part","reference":"s2","record":source("part","body",2,2)}),
            json!({"kind":"FLOW_STEP","id":"step","dependencyReferences":["d1","d2"],"record":{"depth":1}}),
        ];
        let result = project(rows).unwrap();
        assert!(
            !result
                .iter()
                .any(|row| row["reference"] == "d1" || row["reference"] == "s2")
        );
        assert!(result.contains(&flow));
        assert_eq!(
            result.iter().filter(|row| row["kind"] == "SOURCE").count(),
            1
        );
        let summary = result
            .iter()
            .find(|row| row["kind"] == "CALLABLE_SUMMARY")
            .unwrap();
        assert_eq!(summary["record"]["fullRecordReference"], "d1");
        assert_eq!(summary["record"]["sourceReferences"], json!(["s1"]));
        let order = result
            .iter()
            .find(|row| row["kind"] == "PROCESS_FLOW_ORDER")
            .unwrap();
        assert_eq!(order["record"]["steps"][0]["flowReferences"], json!(["d2"]));
    }
}
