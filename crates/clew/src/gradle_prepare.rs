use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReceipt {
    pub task: String,
    execution: Execution,
    selection: Selection,
    outputs: Vec<Output>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Execution {
    Executed,
    UpToDate,
    NotExecuted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Selection {
    Selected,
    NoSelectedOutputs,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    path: String,
    kind: OutputKind,
    files: Vec<OutputFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum OutputKind {
    File,
    Directory,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputFile {
    path: String,
    digest: String,
    selection: FileSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FileSelection {
    Source,
    Classpath,
    NotSelected,
    ChangedAfterTask,
}

fn relative(path: &str) -> bool {
    path.len() <= 4096
        && !path.contains('\\')
        && !path.bytes().any(|byte| byte.is_ascii_control())
        && path
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".." | ".git"))
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

pub(crate) fn invalid_receipt() -> ClewError {
    ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "GRADLE_PREPARE_RECEIPT_INVALID: prepared outputs do not match the selected compilation",
    )
}

pub(crate) fn read(value: &Value) -> Result<Vec<TaskReceipt>, ClewError> {
    let rows =
        serde_json::from_value::<Vec<TaskReceipt>>(value.clone()).map_err(|_| invalid_receipt())?;
    if !valid(&rows) {
        return Err(invalid_receipt());
    }
    Ok(rows)
}

pub(crate) fn valid(rows: &[TaskReceipt]) -> bool {
    let mut tasks = BTreeSet::new();
    let mut file_count = 0usize;
    rows.len() <= crate::project_config::MAX_PREPARE_TASKS
        && rows.iter().all(|row| {
            if !crate::project_config::valid_prepare_task(&row.task)
                || !tasks.insert(&row.task)
                || row.outputs.len() > 256
                || row
                    .outputs
                    .windows(2)
                    .any(|pair| pair[0].path >= pair[1].path)
            {
                return false;
            }
            let mut selected = false;
            for output in &row.outputs {
                file_count += output.files.len();
                if file_count > 2048
                    || !relative(&output.path)
                    || output
                        .files
                        .windows(2)
                        .any(|pair| pair[0].path >= pair[1].path)
                    || (output.kind == OutputKind::Missing && !output.files.is_empty())
                    || (output.kind == OutputKind::File && output.files.len() != 1)
                {
                    return false;
                }
                for file in &output.files {
                    let inside = match output.kind {
                        OutputKind::File => file.path == output.path,
                        OutputKind::Directory => file
                            .path
                            .strip_prefix(&output.path)
                            .is_some_and(|suffix| suffix.starts_with('/')),
                        OutputKind::Missing => false,
                    };
                    if !inside || !relative(&file.path) || !digest(&file.digest) {
                        return false;
                    }
                    selected |= matches!(
                        file.selection,
                        FileSelection::Source | FileSelection::Classpath
                    );
                }
            }
            match row.execution {
                Execution::NotExecuted => {
                    row.selection == Selection::Unavailable && row.outputs.is_empty()
                }
                _ if selected => row.selection == Selection::Selected,
                _ => row.selection == Selection::NoSelectedOutputs,
            }
        })
}

pub(crate) fn verify_sources<'a>(
    rows: &[TaskReceipt],
    mut lookup: impl FnMut(&str) -> Option<&'a str>,
) -> Result<(), ClewError> {
    for file in rows
        .iter()
        .flat_map(|row| &row.outputs)
        .flat_map(|output| &output.files)
    {
        let selected_digest = lookup(&file.path);
        if (file.selection == FileSelection::Source
            && selected_digest != Some(file.digest.as_str()))
            || (selected_digest == Some(file.digest.as_str())
                && file.selection != FileSelection::Source)
        {
            return Err(invalid_receipt());
        }
    }
    Ok(())
}

pub(crate) fn failure_message(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "TASK_NOT_FOUND" => {
            "GRADLE_PREPARE_TASK_NOT_FOUND: check the absolute task paths in gradle.prepareTasks"
        }
        "COMPILATION_TASK_DEPENDENCY" => {
            "GRADLE_PREPARE_COMPILATION_TASK_DEPENDENCY: select the generator instead of the compiler or a task that depends on it"
        }
        "TASK_GRAPH_BUDGET" => {
            "GRADLE_PREPARE_TASK_GRAPH_BUDGET: select preparation tasks with fewer Gradle dependencies"
        }
        "OUTPUT_OUTSIDE_CHECKOUT" => {
            "GRADLE_PREPARE_OUTPUT_OUTSIDE_CHECKOUT: declare preparation outputs inside the isolated checkout"
        }
        "OUTPUT_OBSERVATION_BUDGET" => {
            "GRADLE_PREPARE_OUTPUT_OBSERVATION_BUDGET: declare a narrower output directory or select fewer tasks"
        }
        "OUTPUT_NOT_REGULAR" => {
            "GRADLE_PREPARE_OUTPUT_NOT_REGULAR: declare only regular files or directories without symlinks"
        }
        "COMPILATION_INPUTS_UNAVAILABLE" => {
            "GRADLE_PREPARE_COMPILATION_INPUTS_UNAVAILABLE: Codeclew could not read the selected compilation inputs"
        }
        "OUTPUT_CHANGED_DURING_COMPILATION" => {
            "GRADLE_PREPARE_OUTPUT_CHANGED_DURING_COMPILATION: ensure no compiler or later task modifies a prepared input"
        }
        "TASK_FAILED" => {
            "GRADLE_PREPARE_TASK_FAILED: run the configured task with the same Gradle properties and inspect its first failure locally"
        }
        "OBSERVATION_UNAVAILABLE" => {
            "GRADLE_PREPARE_OBSERVATION_UNAVAILABLE: Gradle did not return observations for the configured preparation tasks"
        }
        "RECEIPT_INVALID" => {
            "GRADLE_PREPARE_RECEIPT_INVALID: prepared outputs do not match the selected compilation"
        }
        _ => return None,
    })
}

pub(crate) fn public_failure_code(message: &str) -> Option<&str> {
    let (code, _) = message.split_once(':')?;
    let known = failure_message(code.strip_prefix("GRADLE_PREPARE_")?)?;
    (known == message).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selected_output_requires_exact_compiler_input_bytes() {
        let path = "build/generated/Owned.java";
        let hash = format!("sha256:{}", "1".repeat(64));
        let mut data = json!([{
            "task":":prepare", "execution":"EXECUTED", "selection":"SELECTED",
            "outputs":[{"path":"build/generated","kind":"DIRECTORY","files":[
                {"path":path,"digest":hash,"selection":"SOURCE"}
            ]}]
        }]);
        let rows = read(&data).unwrap();
        assert!(verify_sources(&rows, |file| (file == path).then_some(hash.as_str())).is_ok());
        assert!(verify_sources(&rows, |_| None).is_err());
        data[0]["outputs"][0]["files"][0]["selection"] = json!("NOT_SELECTED");
        assert!(read(&data).is_err());
        data[0]["selection"] = json!("NO_SELECTED_OUTPUTS");
        let rows = read(&data).unwrap();
        assert!(verify_sources(&rows, |_| None).is_ok());
        assert!(verify_sources(&rows, |_| Some(hash.as_str())).is_err());
        data[0]["outputs"][0]["files"][0]["path"] = json!("build/generated/../Owned.java");
        assert!(read(&data).is_err());
    }

    #[test]
    fn skipped_task_cannot_claim_selected_outputs() {
        let mut data = json!([{
            "task":":prepare",
            "execution":"NOT_EXECUTED", "selection":"UNAVAILABLE", "outputs":[]
        }]);
        assert!(read(&data).is_ok());
        data[0]["selection"] = json!("NO_SELECTED_OUTPUTS");
        assert!(read(&data).is_err());
    }
}
