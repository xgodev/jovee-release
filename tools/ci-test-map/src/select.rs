//! Responsibility: picks the tests that run the lines changed between base and HEAD, from the test map.

use crate::args::Args;
use crate::diff::{self, FileDiff, Hunk};
use crate::list;
use crate::map::Map;
use crate::metadata::Workspace;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// Changes here no build or test reads.
const INERT: &[&str] = &["docs/", "site/", ".github/", ".claude/", "packaging/", "dist/", "wix/", "docker/", "hardware/", "scripts/", "tools/"];

/// Above this the filter names whole test binaries instead of single tests
/// (one argument may not pass 128 KiB on Linux).
const MAX_FILTER: usize = 100_000;

#[derive(Default, Debug, PartialEq)]
pub struct Plan {
    /// A change every crate may read (Cargo.lock, assets...): test everything.
    pub everything: bool,
    /// Crates whose every test runs.
    pub whole: BTreeSet<String>,
    /// Crates with changed files (their new tests run too).
    pub owners: BTreeSet<String>,
    /// Map test indexes that run a changed line.
    pub tests: BTreeSet<u32>,
}

/// `translate(path)` gives the map-to-base hunks of a file, or None when the
/// file did not exist at the map's commit.
pub fn plan(changes: &[FileDiff], ws: &Workspace, map: Option<&Map>, translate: &mut dyn FnMut(&str) -> Option<Vec<Hunk>>) -> Plan {
    let mut p = Plan::default();
    for f in changes {
        let Some(path) = f.old.as_deref().or(f.new.as_deref()) else { continue };
        if INERT.iter().any(|d| path.starts_with(d)) || (!path.contains('/') && path.ends_with(".md")) {
            continue;
        }
        let Some(owner) = ws.owner(path).or_else(|| f.new.as_deref().and_then(|n| ws.owner(n))) else {
            p.everything = true;
            continue;
        };
        p.owners.insert(owner.to_string());
        let (Some(map), Some(old), true) = (map, f.old.as_deref(), path.ends_with(".rs")) else {
            p.whole.insert(owner.to_string());
            continue;
        };
        let Some(hunks) = translate(old) else {
            p.whole.insert(owner.to_string());
            continue;
        };
        let Some(spans) = map.files.get(old) else { continue };
        for line in f.hunks.iter().flat_map(diff::old_lines) {
            for ml in diff::to_old(&hunks, line) {
                let inside: Vec<_> = spans.iter().filter(|(s, e, _)| *s <= ml && ml <= *e).collect();
                // A line outside every function (a const, a struct): every test of the file.
                let hit = if inside.is_empty() { spans.iter().collect() } else { inside };
                p.tests.extend(hit.into_iter().flat_map(|(_, _, ts)| ts.iter().copied()));
            }
        }
    }
    if map.is_none() {
        p.whole = ws.with_dependents(&p.whole);
    }
    p
}

pub fn select_cmd(args: &Args) -> Result<(), String> {
    let ws = Workspace::load(args.req("metadata")?)?;
    let base = args.req("base")?;
    let out = args.req("out")?;
    let map = match args.opt("map") {
        Some(m) if std::path::Path::new(m).exists() => Some(Map::load(m)?),
        _ => None,
    };
    let changes = diff::git_diff(base, "HEAD", &[])?;
    let sha = map.as_ref().map(|m| m.sha.clone()).unwrap_or_default();
    let mut translate = |path: &str| -> Option<Vec<Hunk>> {
        let exists = Command::new("git").args(["cat-file", "-e", &format!("{sha}:{path}")]).status().is_ok_and(|s| s.success());
        if !exists {
            return None;
        }
        diff::git_diff(&sha, base, &[path]).ok().map(|f| f.into_iter().flat_map(|f| f.hunks).collect())
    };
    let p = plan(&changes, &ws, map.as_ref(), &mut translate);
    let tests: Vec<(&str, &str)> = map
        .as_ref()
        .map(|m| p.tests.iter().filter_map(|i| m.tests[*i as usize].split_once('\t')).collect())
        .unwrap_or_default();
    eprintln!(
        "changed files: {}; map: {}; everything: {}; whole crates: {:?}; mapped tests: {}",
        changes.len(),
        if map.is_some() { sha.as_str() } else { "none" },
        p.everything,
        p.whole,
        tests.len()
    );
    std::fs::create_dir_all(out).map_err(|e| format!("{out}: {e}"))?;
    let mut crates: BTreeSet<String> = p.whole.iter().chain(&p.owners).cloned().collect();
    crates.extend(tests.iter().map(|(b, _)| b.split("::").next().unwrap_or(b).to_string()));
    let packages = if p.everything { "--workspace".to_string() } else { crates.iter().map(|c| format!("-p {c}")).collect::<Vec<_>>().join(" ") };
    write(out, "packages", &packages)?;
    let docs = if p.everything { "--workspace".to_string() } else { p.whole.iter().chain(&p.owners).collect::<BTreeSet<_>>().iter().map(|c| format!("-p {c}")).collect::<Vec<_>>().join(" ") };
    write(out, "doc-packages", &docs)?;
    if let Some(list_path) = args.opt("list") {
        let listed = list::load(list_path)?;
        let filter = filter(&p, &tests, map.as_ref(), &listed);
        write(out, "filter", &filter)?;
    }
    Ok(())
}

fn filter(p: &Plan, tests: &[(&str, &str)], map: Option<&Map>, listed: &list::TestList) -> String {
    const NO_E2E: &str = "not test(/e2e_ui/)";
    if p.everything {
        return NO_E2E.to_string();
    }
    let known: BTreeSet<&str> = map.map(|m| m.tests.iter().map(String::as_str).collect()).unwrap_or_default();
    let present: BTreeSet<(&str, &str)> = listed.binaries.iter().flat_map(|b| b.tests.iter().map(move |t| (b.id.as_str(), t.as_str()))).collect();
    let mut chosen: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (b, t) in tests.iter().filter(|bt| present.contains(*bt)) {
        chosen.entry(b).or_default().insert(t);
    }
    // Tests the map has never seen (new ones) in the changed crates.
    for b in listed.binaries.iter().filter(|b| p.owners.contains(&b.package)) {
        for t in &b.tests {
            if !known.contains(format!("{}\t{t}", b.id).as_str()) {
                chosen.entry(&b.id).or_default().insert(t);
            }
        }
    }
    let mut parts: Vec<String> = p.whole.iter().map(|c| format!("package(={c})")).collect();
    let single: Vec<String> = chosen.iter().flat_map(|(b, ts)| ts.iter().map(move |t| format!("(binary_id(={b}) & test(={t}))"))).collect();
    if single.iter().map(|s| s.len() + 3).sum::<usize>() > MAX_FILTER {
        parts.extend(chosen.keys().map(|b| format!("binary_id(={b})")));
    } else {
        parts.extend(single);
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("({}) & {NO_E2E}", parts.join(" | "))
}

fn write(dir: &str, name: &str, text: &str) -> Result<(), String> {
    std::fs::write(format!("{dir}/{name}"), text).map_err(|e| format!("{dir}/{name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> Workspace {
        let dir = std::env::temp_dir().join(format!("ci-test-map-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("metadata.json");
        std::fs::write(
            &path,
            r#"{"workspace_root":"/w","workspace_members":["a","b"],
               "packages":[{"id":"a","name":"engine","manifest_path":"/w/crates/engine/Cargo.toml"},
                           {"id":"b","name":"gui","manifest_path":"/w/crates/gui/Cargo.toml"}],
               "resolve":{"nodes":[{"id":"b","deps":[{"pkg":"a"}]},{"id":"a","deps":[]}]}}"#,
        )
        .unwrap();
        Workspace::load(path.to_str().unwrap()).unwrap()
    }

    fn map() -> Map {
        let mut files = BTreeMap::new();
        files.insert("crates/engine/src/lib.rs".to_string(), vec![(1, 10, vec![0]), (20, 30, vec![1])]);
        Map { sha: "m".into(), tests: vec!["engine\tt_one".into(), "gui\tt_two".into()], files }
    }

    fn change(path: &str, old_start: u32, old_len: u32) -> FileDiff {
        FileDiff { old: Some(path.into()), new: Some(path.into()), hunks: vec![Hunk { old_start, old_len, new_start: old_start, new_len: old_len }] }
    }

    #[test]
    fn a_changed_line_picks_only_the_tests_that_ran_it() {
        let p = plan(&[change("crates/engine/src/lib.rs", 25, 1)], &ws(), Some(&map()), &mut |_| Some(vec![]));
        assert_eq!(p.tests, BTreeSet::from([1]));
        assert!(p.whole.is_empty() && !p.everything);
    }

    #[test]
    fn lines_moved_since_the_map_are_followed() {
        // Three lines added at line 2 since the map: base line 28 was map line 25.
        let moved = vec![Hunk { old_start: 1, old_len: 0, new_start: 2, new_len: 3 }];
        let p = plan(&[change("crates/engine/src/lib.rs", 28, 1)], &ws(), Some(&map()), &mut |_| Some(moved.clone()));
        assert_eq!(p.tests, BTreeSet::from([1]));
    }

    #[test]
    fn a_line_outside_every_function_picks_every_test_of_the_file() {
        let p = plan(&[change("crates/engine/src/lib.rs", 15, 1)], &ws(), Some(&map()), &mut |_| Some(vec![]));
        assert_eq!(p.tests, BTreeSet::from([0, 1]));
    }

    #[test]
    fn a_non_rust_file_tests_its_whole_crate() {
        let p = plan(&[change("crates/gui/ui/main.slint", 1, 1)], &ws(), Some(&map()), &mut |_| Some(vec![]));
        assert_eq!(p.whole, BTreeSet::from(["gui".to_string()]));
    }

    #[test]
    fn docs_test_nothing_and_the_lock_file_tests_everything() {
        let p = plan(&[change("docs/x.md", 1, 1), change("README.md", 1, 1)], &ws(), Some(&map()), &mut |_| Some(vec![]));
        assert_eq!(p, Plan::default());
        let p = plan(&[change("Cargo.lock", 1, 1)], &ws(), Some(&map()), &mut |_| Some(vec![]));
        assert!(p.everything);
    }

    #[test]
    fn without_a_map_the_changed_crate_and_its_dependents_run() {
        let p = plan(&[change("crates/engine/src/lib.rs", 25, 1)], &ws(), None, &mut |_| None);
        assert_eq!(p.whole, BTreeSet::from(["engine".to_string(), "gui".to_string()]));
    }

    #[test]
    fn the_filter_names_the_mapped_tests_and_the_new_ones() {
        let p = Plan { owners: BTreeSet::from(["engine".to_string()]), tests: BTreeSet::from([1]), ..Plan::default() };
        let listed = list::TestList {
            binaries: vec![
                list::Binary { id: "engine".into(), package: "engine".into(), path: String::new(), cwd: String::new(), tests: vec!["t_one".into(), "t_new".into()] },
                list::Binary { id: "gui".into(), package: "gui".into(), path: String::new(), cwd: String::new(), tests: vec!["t_two".into()] },
            ],
            library_dirs: vec![],
        };
        let f = filter(&p, &[("gui", "t_two")], Some(&map()), &listed);
        assert_eq!(f, "((binary_id(=engine) & test(=t_new)) | (binary_id(=gui) & test(=t_two))) & not test(/e2e_ui/)");
    }
}
