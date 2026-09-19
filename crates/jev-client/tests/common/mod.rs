//! Fixture loading shared by the integration tests.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// A JSON fixture: its file name, and its parsed content.
pub(crate) struct Fixture {
    pub(crate) name: String,
    pub(crate) json: Value,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every fixture in `group` whose file name starts with `prefix`, in a stable order.
///
/// Panics when nothing matches, so a moved or emptied directory cannot make a test pass vacuously.
pub(crate) fn load(group: &str, prefix: &str) -> Vec<Fixture> {
    let dir = fixtures_dir().join(group);
    let mut fixtures: Vec<Fixture> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_owned();
            name.starts_with(prefix).then(|| {
                let text = fs::read_to_string(&path).unwrap();
                let json = serde_json::from_str(&text)
                    .unwrap_or_else(|error| panic!("{name} is not valid JSON: {error}"));
                Fixture { name, json }
            })
        })
        .collect();
    fixtures.sort_by(|a, b| a.name.cmp(&b.name));

    assert!(
        !fixtures.is_empty(),
        "no `{prefix}*.json` fixtures in {}",
        dir.display()
    );
    fixtures
}
