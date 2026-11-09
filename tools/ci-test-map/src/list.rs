//! Responsibility: reads the test binaries and test names out of `cargo nextest list --message-format json`.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "rust-build-meta")]
    build_meta: BuildMeta,
    #[serde(rename = "rust-suites")]
    suites: BTreeMap<String, Suite>,
}

#[derive(Deserialize)]
struct BuildMeta {
    #[serde(rename = "target-directory")]
    target_directory: String,
    #[serde(rename = "base-output-directories", default)]
    base_output_directories: Vec<String>,
    #[serde(rename = "linked-paths", default)]
    linked_paths: Vec<String>,
}

#[derive(Deserialize)]
struct Suite {
    #[serde(rename = "package-name")]
    package_name: String,
    #[serde(rename = "binary-path")]
    binary_path: String,
    cwd: String,
    testcases: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
struct Case {
    ignored: bool,
}

pub struct Binary {
    pub id: String,
    pub package: String,
    pub path: String,
    pub cwd: String,
    pub tests: Vec<String>,
}

pub struct TestList {
    pub binaries: Vec<Binary>,
    /// Directories the test binaries load shared libraries from.
    pub library_dirs: Vec<String>,
}

pub fn load(path: &str) -> Result<TestList, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    // `cargo nextest list` may print build lines before the JSON document.
    let start = text.find('{').ok_or_else(|| format!("{path}: no JSON"))?;
    let raw: Raw = serde_json::from_str(&text[start..]).map_err(|e| format!("{path}: {e}"))?;
    let target = raw.build_meta.target_directory;
    let mut library_dirs: Vec<String> = raw
        .build_meta
        .base_output_directories
        .iter()
        .flat_map(|d| [format!("{target}/{d}"), format!("{target}/{d}/deps")])
        .collect();
    library_dirs.extend(raw.build_meta.linked_paths.iter().map(|p| {
        if p.starts_with('/') { p.clone() } else { format!("{target}/{p}") }
    }));
    let binaries = raw
        .suites
        .into_iter()
        .map(|(id, s)| Binary {
            id,
            package: s.package_name,
            path: s.binary_path,
            cwd: s.cwd,
            tests: s.testcases.into_iter().filter(|(_, c)| !c.ignored).map(|(n, _)| n).collect(),
        })
        .collect();
    Ok(TestList { binaries, library_dirs })
}
