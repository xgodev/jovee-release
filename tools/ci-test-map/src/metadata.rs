//! Responsibility: knows the workspace crates from `cargo metadata`: which crate owns a file and which crates depend on a crate.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Raw {
    workspace_root: String,
    workspace_members: Vec<String>,
    packages: Vec<Package>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: String,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dep>,
}

#[derive(Deserialize)]
struct Dep {
    pkg: String,
}

pub struct Workspace {
    /// Crate directory ("crates/engine/") to crate name.
    dirs: BTreeMap<String, String>,
    /// Crate name to the workspace crates that depend on it.
    dependents: BTreeMap<String, BTreeSet<String>>,
}

impl Workspace {
    pub fn load(path: &str) -> Result<Workspace, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let raw: Raw = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let root = format!("{}/", raw.workspace_root.trim_end_matches('/'));
        let members: BTreeSet<&String> = raw.workspace_members.iter().collect();
        let names: BTreeMap<&String, &String> = raw.packages.iter().map(|p| (&p.id, &p.name)).collect();
        let mut dirs = BTreeMap::new();
        for p in raw.packages.iter().filter(|p| members.contains(&p.id)) {
            let dir = p.manifest_path.strip_prefix(&root).unwrap_or(&p.manifest_path);
            let dir = dir.strip_suffix("Cargo.toml").unwrap_or(dir);
            dirs.insert(dir.to_string(), p.name.clone());
        }
        let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for n in raw.resolve.nodes.iter().filter(|n| members.contains(&n.id)) {
            for d in n.deps.iter().filter(|d| members.contains(&d.pkg)) {
                dependents.entry(names[&d.pkg].clone()).or_default().insert(names[&n.id].clone());
            }
        }
        Ok(Workspace { dirs, dependents })
    }

    /// The crate whose directory holds the file (the deepest one).
    pub fn owner(&self, file: &str) -> Option<&str> {
        self.dirs.iter().filter(|(d, _)| file.starts_with(d.as_str())).max_by_key(|(d, _)| d.len()).map(|(_, n)| n.as_str())
    }

    /// The crates plus every workspace crate that depends on them.
    pub fn with_dependents(&self, crates: &BTreeSet<String>) -> BTreeSet<String> {
        let mut seen = crates.clone();
        let mut todo: Vec<String> = crates.iter().cloned().collect();
        while let Some(c) = todo.pop() {
            for d in self.dependents.get(&c).into_iter().flatten() {
                if seen.insert(d.clone()) {
                    todo.push(d.clone());
                }
            }
        }
        seen
    }
}
