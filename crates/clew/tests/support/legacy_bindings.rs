//! Fixed historical storage fixture for read compatibility tests.
//! No source-derived document is generated: the HTML is a static marker.
use clew::documentation::{check::Check, model::Narrative, retained_validation};
use std::{collections::BTreeMap, fs, path::Path};

pub fn retain(root: &Path, checked: &Check, narratives: Vec<Narrative>) -> String {
    let mut binding = retained_validation::make_bindings(
        checked,
        narratives
            .into_iter()
            .map(|n| (n.subject.clone(), n))
            .collect(),
    )
    .unwrap();
    let id = clew::canonical::hash_bytes(&serde_json::to_vec(&binding).unwrap())[7..].to_owned();
    let overview =
        format!("<!-- codeclew-bundle {id} -->\n<main>Fixed historical bindings fixture</main>");
    binding.output_hashes = BTreeMap::from([(
        "root-overview.html".into(),
        clew::canonical::hash_bytes(overview.as_bytes()),
    )]);
    let directory = root.join("docs/generated").join(&id);
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("bindings.json"),
        serde_json::to_vec(&binding).unwrap(),
    )
    .unwrap();
    fs::write(directory.join("root-overview.html"), overview.as_bytes()).unwrap();
    fs::write(root.join("docs/index.html"), overview.as_bytes()).unwrap();
    id
}
