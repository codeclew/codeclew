use clew::runtime::RuntimeAuthority;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

const REQUEST_LIMIT: usize = 64 * 1024;

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let response = match line {
            Ok(line) if line.len() <= REQUEST_LIMIT => dispatch(&line),
            Ok(_) => error(Value::Null, "REQUEST_TOO_LARGE", "request exceeds 64 KiB"),
            Err(value) => error(Value::Null, "IO_ERROR", &value.to_string()),
        };
        if writeln!(stdout, "{}", response).is_err() || stdout.flush().is_err() {
            break;
        }
        if response
            .get("result")
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            == Some("SHUTTING_DOWN")
        {
            break;
        }
    }
}

fn dispatch(line: &str) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(value) => return error(Value::Null, "INVALID_JSON", &value.to_string()),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str);
    let result = match method {
        Some("health") => {
            let runtime = match RuntimeAuthority::from_environment() {
                Ok(Some(runtime)) => runtime,
                Ok(None) => {
                    return error(id, "RUNTIME_REQUIRED", "semanticd must run from a capsule");
                }
                Err(value) => return error(id, "RUNTIME_INVALID", &value.to_string()),
            };
            json!({
                "status":"OK",
                "service":"semanticd",
                "protocol":"codeclew-managed/2.0",
                "runtimeKey":runtime.runtime_key,
                "runtimeMode":runtime.mode,
            })
        }
        Some("shutdown") => json!({"status":"SHUTTING_DOWN"}),
        Some(_) => {
            return error(
                id,
                "METHOD_REMOVED",
                "semantic operations are available only through ./clew managed sessions",
            );
        }
        None => return error(id, "INVALID_REQUEST", "request has no method"),
    };
    json!({"schema":"codeclew-semanticd-response/2.0","id":id,"result":result})
}

fn error(id: Value, code: &str, message: &str) -> Value {
    json!({
        "schema":"codeclew-semanticd-response/2.0",
        "id":id,
        "error":{"code":code,"message":message},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_legacy_methods_are_not_dispatchable() {
        let response = dispatch(r#"{"id":1,"method":"project.inspect"}"#);
        assert_eq!(response["error"]["code"], "METHOD_REMOVED");
    }

    #[test]
    fn shutdown_is_explicit() {
        let response = dispatch(r#"{"id":"x","method":"shutdown"}"#);
        assert_eq!(response["result"]["status"], "SHUTTING_DOWN");
    }
}
