//! Link existing service explanations without synthesizing their internal narrative.
//! A preview is a declared composition, never a runtime routing or call-graph proof.
#[cfg(test)]
use super::{invalid, io_error, store};
#[cfg(test)]
use crate::{canonical::hash_bytes, error::ClewError};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(not(test))]
use std::path::PathBuf;
#[cfg(test)]
use std::{
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Process {
    pub id: String,
    pub title: String,
    pub service: String,
    pub revision: String,
    pub binding: Binding,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Binding {
    CataloguePage {
        directory: PathBuf,
        export_digest: String,
        route: String,
        page_digest: String,
        operation: String,
        operation_digest: String,
    },
    NativeScenarioPage {
        directory: PathBuf,
        publication_digest: String,
        scenario: String,
        scenario_digest: String,
    },
}

#[cfg(test)]
struct Linked {
    href: String,
    proof: Value,
}
/// Native threads reuse the same frozen-page pins as the linked preview.
/// The page stays at its original address; its narrative is never regenerated.
#[cfg(test)]
pub(super) fn project_link(process: &Process, budget: &mut u64) -> Result<Value, ClewError> {
    if !store::valid_id(&process.id)
        || !store::valid_id(&process.service)
        || !bounded(&process.title)
        || !bounded(&process.revision)
    {
        return Err(invalid("invalid frozen process page identity"));
    }
    let linked = match &process.binding {
        Binding::CataloguePage { .. } => catalogue_page(process, &process.binding, budget)?,
        Binding::NativeScenarioPage { .. } => native_scenario(process, &process.binding, budget)?,
    };
    Ok(
        json!({"id":process.id,"title":process.title,"service":process.service,
        "revision":process.revision,"href":linked.href,"proof":linked.proof,
        "internalNarrativeGeneration":false}),
    )
}
#[cfg(test)]
fn read(path: &Path, budget: &mut u64) -> Result<Vec<u8>, ClewError> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(invalid("preview input must be a regular file"));
    }
    let mut raw = Vec::new();
    file.by_ref()
        .take(*budget + 1)
        .read_to_end(&mut raw)
        .map_err(io_error)?;
    if raw.len() as u64 > *budget {
        return Err(invalid("linked preview exceeds its 64 MiB input budget"));
    }
    *budget -= raw.len() as u64;
    Ok(raw)
}
#[cfg(test)]
fn pinned(path: &Path, expected: &str, budget: &mut u64) -> Result<Vec<u8>, ClewError> {
    let raw = read(path, budget)?;
    if hash_bytes(&raw) != expected {
        return Err(invalid(
            "linked preview input changed from its selected digest",
        ));
    }
    Ok(raw)
}
#[cfg(test)]
fn decode(raw: &[u8]) -> Result<Value, ClewError> {
    serde_json::from_slice(raw).map_err(|_| invalid("invalid preview reference JSON"))
}
#[cfg(test)]
fn bounded(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096
}

#[cfg(test)]
fn file_href(path: &Path) -> Result<String, ClewError> {
    let path = fs::canonicalize(path).map_err(io_error)?;
    let path = path
        .to_str()
        .ok_or_else(|| invalid("preview page path is not UTF-8"))?;
    let mut href = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            href.push(byte as char);
        } else {
            href.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(href)
}
#[cfg(test)]
fn catalogue_page(
    process: &Process,
    binding: &Binding,
    budget: &mut u64,
) -> Result<Linked, ClewError> {
    let Binding::CataloguePage {
        directory,
        export_digest,
        route,
        page_digest,
        operation,
        operation_digest,
    } = binding
    else {
        unreachable!()
    };
    if !route.starts_with("processes/")
        || !route.ends_with(".html")
        || Path::new(route)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || Path::new(route).components().count() != 2
    {
        return Err(invalid(
            "preview route must select one existing process page",
        ));
    }
    let report = decode(&pinned(
        &directory.join("export.json"),
        export_digest,
        budget,
    )?)?;
    if report["schema"] != "codeclew-documentation-catalogue-export/1.0" {
        return Err(invalid("unsupported process export reference"));
    }
    let rows: Vec<_> = report["processes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["route"] == *route)
        .collect();
    let [row] = rows.as_slice() else {
        return Err(invalid(
            "canonical process reference is absent or ambiguous",
        ));
    };
    if row["subject"] != format!("service:{}", process.service)
        || row["operation"] != *operation
        || row["operationDigest"] != *operation_digest
        || row["acceptedVersion"]["sourceRevisions"][&process.service] != process.revision
        || !matches!(
            row["acceptedVersion"]["verification"].as_str(),
            Some("VERIFIED" | "VERIFIED_WITH_LIMITATIONS")
        )
    {
        return Err(invalid(
            "canonical process reference disagrees with its accepted service/revision/operation",
        ));
    }
    let path = directory.join(route);
    String::from_utf8(pinned(&path, page_digest, budget)?)
        .map_err(|_| invalid("canonical page is not UTF-8"))?;
    Ok(Linked {
        href: file_href(&path)?,
        proof: json!({"kind":"EXISTING_CANONICAL_PAGE","path":path,"pageDigest":page_digest,"exportDigest":export_digest,"operation":operation,"operationDigest":operation_digest,"acceptedVersion":row["acceptedVersion"]}),
    })
}
#[cfg(test)]
fn native_scenario(
    process: &Process,
    binding: &Binding,
    budget: &mut u64,
) -> Result<Linked, ClewError> {
    let Binding::NativeScenarioPage {
        directory,
        publication_digest,
        scenario,
        scenario_digest,
    } = binding
    else {
        unreachable!()
    };
    if !store::valid_id(scenario) {
        return Err(invalid("invalid native process scenario identity"));
    }
    let publication = decode(&pinned(
        &directory.join("publication.json"),
        publication_digest,
        budget,
    )?)?;
    let route = format!("scenarios/{scenario}.html");
    let json_route = format!("scenarios/{scenario}.json");
    if publication["files"][&json_route] != *scenario_digest {
        return Err(invalid(
            "native scenario differs from its publication manifest",
        ));
    }
    let page_digest = publication["files"][&route]
        .as_str()
        .ok_or_else(|| invalid("native process page is absent from publication"))?;
    let data = decode(&pinned(
        &directory.join(&json_route),
        scenario_digest,
        budget,
    )?)?;
    let thread = &data["processThread"];
    let root = thread["rootMethod"]
        .as_str()
        .ok_or_else(|| invalid("native process root is unavailable"))?;
    if data["subject"] != format!("scenario:{scenario}")
        || data["revisions"][&process.service] != process.revision
        || thread["status"] != "SOURCE_BOUND_DECLARATION"
        || thread["methods"][root]["source"]["service"] != process.service
        || thread["methods"][root]["source"]["revision"] != process.revision
    {
        return Err(invalid(
            "native process page differs from its selected root/service/revision",
        ));
    }
    let path = directory.join(&route);
    String::from_utf8(pinned(&path, page_digest, budget)?)
        .map_err(|_| invalid("native process page is not UTF-8"))?;
    Ok(Linked {
        href: file_href(&path)?,
        proof: json!({"kind":"EXISTING_NATIVE_PROCESS_PAGE","path":path,"pageDigest":page_digest,"scenarioDigest":scenario_digest,"publicationDigest":publication_digest,"pageDigest":page_digest,"authority":thread["authority"],"internalNarrativeGeneration":false}),
    })
}
