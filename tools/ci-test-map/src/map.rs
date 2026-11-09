//! Responsibility: holds the test map (which tests run each function's lines), merges partial maps and stores them.

use crate::args::Args;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One function: first line, last line, indexes into `tests`.
pub type Span = (u32, u32, Vec<u32>);

#[derive(Serialize, Deserialize, Default)]
pub struct Map {
    /// The jovee commit the line numbers belong to.
    pub sha: String,
    /// "binary id" + '\t' + "test name".
    pub tests: Vec<String>,
    /// Workspace-relative file to its functions.
    pub files: BTreeMap<String, Vec<Span>>,
}

impl Map {
    pub fn load(path: &str) -> Result<Map, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }

    pub fn save(&self, path: &str) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))
    }
}

/// Builds a map from (place → test ids) pairs; spans of one place are joined.
pub struct Builder {
    tests: Vec<String>,
    index: BTreeMap<String, u32>,
    spans: BTreeMap<(String, u32, u32), BTreeSet<u32>>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { tests: Vec::new(), index: BTreeMap::new(), spans: BTreeMap::new() }
    }

    pub fn add(&mut self, file: &str, start: u32, end: u32, test: &str) {
        let next = self.tests.len() as u32;
        let i = *self.index.entry(test.to_string()).or_insert_with(|| next);
        if i == next {
            self.tests.push(test.to_string());
        }
        self.spans.entry((file.to_string(), start, end)).or_default().insert(i);
    }

    pub fn absorb(&mut self, map: &Map) {
        for (file, spans) in &map.files {
            for (s, e, ts) in spans {
                for t in ts {
                    self.add(file, *s, *e, &map.tests[*t as usize]);
                }
            }
        }
    }

    pub fn finish(self, sha: &str) -> Map {
        let mut files: BTreeMap<String, Vec<Span>> = BTreeMap::new();
        for ((file, s, e), ts) in self.spans {
            files.entry(file).or_default().push((s, e, ts.into_iter().collect()));
        }
        Map { sha: sha.to_string(), tests: self.tests, files }
    }
}

pub fn merge_cmd(args: &Args) -> Result<(), String> {
    let mut b = Builder::new();
    for part in &args.positional {
        b.absorb(&Map::load(part)?);
    }
    let map = b.finish(args.req("sha")?);
    eprintln!("map: {} tests, {} files", map.tests.len(), map.files.len());
    map.save(args.req("out")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_two_parts_joins_the_tests_of_one_function() {
        let mut a = Builder::new();
        a.add("src/a.rs", 3, 9, "x\tt1");
        let mut b = Builder::new();
        b.add("src/a.rs", 3, 9, "y\tt2");
        b.add("src/b.rs", 1, 2, "y\tt2");
        let mut m = Builder::new();
        m.absorb(&a.finish(""));
        m.absorb(&b.finish(""));
        let map = m.finish("abc");
        assert_eq!(map.tests, vec!["x\tt1", "y\tt2"]);
        assert_eq!(map.files["src/a.rs"], vec![(3, 9, vec![0, 1])]);
        assert_eq!(map.files["src/b.rs"], vec![(1, 2, vec![1])]);
    }
}
