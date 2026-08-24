use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PYTHON_MODEL_SCHEMA: &str = "codeclew-python-project-model/1.0";
pub const PYTHON_GRAMMAR_AUTHORITY: &str =
    "tree-sitter-0.25.10/tree-sitter-python-0.25.0/utf8-syntax";
const SELECTOR_PREFIX: &str = "python:";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonCompilationSelector {
    pub import_root: String,
    pub source_root: String,
}

impl PythonCompilationSelector {
    pub fn parse(value: &str) -> Result<Self, ClewError> {
        if value.len() > 1024 || !value.starts_with(SELECTOR_PREFIX) {
            return Err(invalid(
                "Python compilation selector prefix or size is invalid",
            ));
        }
        let Some((import_root, source_root)) = value[SELECTOR_PREFIX.len()..].split_once('#')
        else {
            return Err(invalid("Python compilation selector is invalid"));
        };
        if source_root.contains('#')
            || !safe_relative_directory(import_root)
            || !safe_relative_directory(source_root)
            || !is_ancestor_or_equal(import_root, source_root)
        {
            return Err(invalid("Python compilation selector is invalid"));
        }
        Ok(Self {
            import_root: import_root.into(),
            source_root: source_root.into(),
        })
    }

    pub fn canonical(&self) -> String {
        format!("{SELECTOR_PREFIX}{}#{}", self.import_root, self.source_root)
    }

    pub fn contains(&self, path: &str) -> bool {
        path.ends_with(".py") && below(&self.source_root, path)
    }

    pub fn module_name(&self, path: &str) -> Result<String, ClewError> {
        if !self.contains(path) || !below(&self.import_root, path) {
            return Err(invalid("Python source is outside its selector authority"));
        }
        let relative = if self.import_root == "." {
            path
        } else {
            path.strip_prefix(&format!("{}/", self.import_root))
                .ok_or_else(|| invalid("Python module path is outside its import root"))?
        };
        let without_suffix = relative
            .strip_suffix(".py")
            .ok_or_else(|| invalid("Python module path has no source suffix"))?;
        let mut components = without_suffix.split('/').collect::<Vec<_>>();
        if components.last() == Some(&"__init__") {
            components.pop();
        }
        if components.is_empty() {
            return Ok("__root__".into());
        }
        if components
            .iter()
            .any(|component| !safe_module_component(component))
        {
            return Err(invalid("Python module identity is not representable"));
        }
        Ok(components.join("."))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonProjectModel {
    pub schema: String,
    pub model_digest: String,
    pub grammar_authority: String,
    pub selectors: Vec<PythonCompilationSelector>,
}

impl PythonProjectModel {
    pub fn create(requested: &[String]) -> Result<Self, ClewError> {
        let requested_count = requested.len();
        let selectors = requested
            .iter()
            .map(|value| PythonCompilationSelector::parse(value))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if selectors.is_empty() || selectors.len() != requested_count || selectors.len() > 32 {
            return Err(invalid(
                "Python compilation selector set is empty, duplicated or too large",
            ));
        }
        let mut model = Self {
            schema: PYTHON_MODEL_SCHEMA.into(),
            model_digest: String::new(),
            grammar_authority: PYTHON_GRAMMAR_AUTHORITY.into(),
            selectors: selectors.into_iter().collect(),
        };
        model.model_digest = canonical::hash(&model).map_err(internal)?;
        model.verify()?;
        Ok(model)
    }

    pub fn verify(&self) -> Result<(), ClewError> {
        if self.schema != PYTHON_MODEL_SCHEMA
            || self.grammar_authority != PYTHON_GRAMMAR_AUTHORITY
            || self.selectors.is_empty()
            || self.selectors.len() > 32
            || self.selectors.windows(2).any(|pair| pair[0] >= pair[1])
            || self
                .selectors
                .iter()
                .any(|selector| PythonCompilationSelector::parse(&selector.canonical()).is_err())
        {
            return Err(invalid("Python project model authority is invalid"));
        }
        let mut unsigned = self.clone();
        unsigned.model_digest.clear();
        if self.model_digest != canonical::hash(&unsigned).map_err(internal)? {
            return Err(invalid("Python project model digest is invalid"));
        }
        Ok(())
    }
}

fn safe_relative_directory(value: &str) -> bool {
    if value == "." {
        return true;
    }
    !value.is_empty()
        && value.len() <= 512
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains(['\\', '\0', '#'])
        && value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn is_ancestor_or_equal(ancestor: &str, path: &str) -> bool {
    ancestor == "." || ancestor == path || path.starts_with(&format!("{ancestor}/"))
}

fn below(directory: &str, path: &str) -> bool {
    directory == "." || path.starts_with(&format!("{directory}/"))
}

fn safe_module_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{PythonCompilationSelector, PythonProjectModel};

    #[test]
    fn selector_is_exact_and_derives_module_identity() {
        let selector = PythonCompilationSelector::parse("python:.#backend").unwrap();
        assert_eq!(selector.canonical(), "python:.#backend");
        assert!(selector.contains("backend/api.py"));
        assert!(!selector.contains("tests/test_api.py"));
        assert_eq!(
            selector.module_name("backend/api.py").unwrap(),
            "backend.api"
        );
        assert_eq!(
            selector.module_name("backend/pkg/__init__.py").unwrap(),
            "backend.pkg"
        );
    }

    #[test]
    fn selector_rejects_escape_and_non_ancestor_import_root() {
        for value in [
            "python:../x#src",
            "python:/tmp#src",
            "python:lib#src",
            "python:.#../src",
            "python:.#src#extra",
            "python:.#",
        ] {
            assert!(PythonCompilationSelector::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn project_model_is_canonical_and_rejects_duplicates() {
        let left =
            PythonProjectModel::create(&["python:.#tests".into(), "python:.#src".into()]).unwrap();
        let right =
            PythonProjectModel::create(&["python:.#src".into(), "python:.#tests".into()]).unwrap();
        assert_eq!(left, right);
        assert!(
            PythonProjectModel::create(&["python:.#src".into(), "python:.#src".into()]).is_err()
        );
    }
}
