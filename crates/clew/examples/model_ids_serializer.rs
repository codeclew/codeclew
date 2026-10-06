//! A compatible driver's serializer boundary; this example makes no model call.
use clew::{
    canonical,
    documentation::model_ids,
    error::{ClewError, ErrorCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Carrier {
    schema: String,
    canonical_job: Value,
    prepared_model: model_ids::Prepared,
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

// These checks illustrate the existing canonical request boundary. A provider
// driver must also keep its own policy, role, budget and contract validations.
// Never perform packet validation against the projected model payload.
fn validate_canonical(job: &Value) -> Result<(), ClewError> {
    let object = job
        .as_object()
        .ok_or_else(|| invalid("job must be an object"))?;
    let required = [
        "schema",
        "invocation",
        "role",
        "model",
        "work",
        "cap",
        "payload",
    ];
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "expansionBudget")
        || job["schema"] != "codeclew-documentation-agent-job/1.0"
        || !matches!(job["role"].as_str(), Some("author" | "reviewer"))
        || job["invocation"].as_str().is_none_or(|id| {
            id.len() != 32
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        || job["work"].as_str().is_none_or(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        || !job["cap"].is_object()
        || job["model"].as_str().is_none_or(str::is_empty)
        || !job["payload"]["outputSchema"].is_object()
    {
        return Err(invalid("canonical job envelope differs"));
    }
    let mut packet = job["payload"]["packet"].clone();
    let expected = packet
        .as_object_mut()
        .and_then(|packet| packet.remove("packetDigest"))
        .ok_or_else(|| invalid("canonical packet digest is absent"))?;
    let actual =
        canonical::hash(&packet).map_err(|_| invalid("canonical packet cannot be hashed"))?;
    if expected != actual {
        return Err(invalid("packet content binding differs"));
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "carrier exceeds example input bound"
    );
    let value = canonical::parse_json_strict(&bytes)?;
    let carrier: Carrier = serde_json::from_value(value)?;
    anyhow::ensure!(
        carrier.schema == model_ids::VERSION,
        "unsupported model representation"
    );
    let input = model_ids::forward_model_input(
        &carrier.canonical_job,
        &carrier.prepared_model,
        validate_canonical,
    )?;
    // This is the exact model input, not a final documentation-agent Reply.
    // Integrate these values with the driver's existing provider serialization.
    println!(
        "{}",
        serde_json::to_string(
            &json!({"payload":input.payload,"outputSchema":input.output_schema})
        )?
    );
    Ok(())
}
