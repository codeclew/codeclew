//! Selected questions and imported prose retain independent language provenance.
use super::{model::BundleProjection, prose_language};
use crate::{documentation::language, error::ClewError};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Presentation {
    pub gaps: Vec<Value>,
    pub files: BTreeMap<String, Vec<u8>>,
    notices: BTreeMap<(String, String), String>,
}

impl Presentation {
    pub fn new(projection: &BundleProjection, requested: Option<&str>) -> Result<Self, ClewError> {
        language::validate(requested)?;
        let mut result = Self::default();
        for page in &projection.pages {
            language::validate(page.selection.documentation_language.as_deref())?;
            for paragraph in &page.authored_paragraphs {
                language::validate(paragraph.documentation_language.as_deref())?;
            }
            if requested.is_none() {
                continue;
            }
            if page.selection.question.is_some() {
                result.add(
                    &page.id,
                    if page
                        .projection_kind
                        .is_some_and(super::model::ProjectionKind::is_declaration_view)
                    {
                        &["overview", "diagnostic"]
                    } else {
                        &["diagnostic"]
                    },
                    "SELECTED_QUESTION_TRANSLATION",
                    page.selection.documentation_language.as_deref(),
                    requested,
                    format!("{}.question.original.json", page.id),
                    json!({"selection":page.selection}),
                )?;
            }
            for (index, paragraph) in page.authored_paragraphs.iter().enumerate() {
                result.add(
                    &page.id,
                    &["overview"],
                    "SELECTED_PARAGRAPH_TRANSLATION",
                    paragraph.documentation_language.as_deref(),
                    requested,
                    format!("{}.paragraph-{index}.original.json", page.id),
                    serde_json::to_value(paragraph).map_err(crate::documentation::io_error)?,
                )?;
            }
            for (index, note) in page.human_instructions.iter().enumerate() {
                result.add(
                    &page.id,
                    &["overview"],
                    "SELECTED_NOTE_TRANSLATION",
                    None,
                    requested,
                    format!("{}.note-{index}.original.json", page.id),
                    serde_json::to_value(note).map_err(crate::documentation::io_error)?,
                )?;
            }
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        page: &str,
        views: &[&str],
        kind: &str,
        available: Option<&str>,
        requested: Option<&str>,
        path: String,
        original: Value,
    ) -> Result<(), ClewError> {
        self.files.insert(
            path.clone(),
            serde_json::to_vec_pretty(&original).map_err(crate::documentation::io_error)?,
        );
        if let Some(mut gap) = prose_language::gap(&json!(available), requested, kind, &path)? {
            gap["processId"] = json!(page);
            for view in views {
                self.notices
                    .entry((page.into(), (*view).into()))
                    .or_default()
                    .push_str(&prose_language::html(&gap, &path));
            }
            self.gaps.push(gap);
        }
        Ok(())
    }

    pub fn notice(&self, page: &str, view: &str) -> &str {
        self.notices
            .get(&(page.into(), view.into()))
            .map(String::as_str)
            .unwrap_or("")
    }
}
