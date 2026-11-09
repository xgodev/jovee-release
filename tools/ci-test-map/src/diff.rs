//! Responsibility: reads `git diff -U0` hunks and turns line numbers of one side into the other's.

use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
}

#[derive(Debug, Default)]
pub struct FileDiff {
    /// None for a file the diff adds.
    pub old: Option<String>,
    /// None for a file the diff deletes.
    pub new: Option<String>,
    pub hunks: Vec<Hunk>,
}

pub fn git_diff(from: &str, to: &str, paths: &[&str]) -> Result<Vec<FileDiff>, String> {
    let out = Command::new("git")
        .args(["diff", "-U0", "--no-renames", "--no-color", from, to, "--"])
        .args(paths)
        .output()
        .map_err(|e| format!("git diff: {e}"))?;
    if !out.status.success() {
        return Err(format!("git diff {from} {to}: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in text.lines() {
        if let Some(paths) = line.strip_prefix("diff --git a/") {
            // A mode-only or binary change has no ---/+++ lines: both sides exist.
            let (old, new) = paths.split_once(" b/").unwrap_or((paths, paths));
            files.push(FileDiff { old: Some(old.to_string()), new: Some(new.to_string()), hunks: Vec::new() });
        } else if let Some(f) = files.last_mut() {
            if let Some(p) = line.strip_prefix("--- ") {
                f.old = p.strip_prefix("a/").map(String::from);
            } else if let Some(p) = line.strip_prefix("+++ ") {
                f.new = p.strip_prefix("b/").map(String::from);
            } else if let Some(h) = line.strip_prefix("@@ -") {
                if let Some(h) = hunk(h) {
                    f.hunks.push(h);
                }
            }
        }
    }
    files
}

fn hunk(h: &str) -> Option<Hunk> {
    let mut parts = h.split_whitespace();
    let (os, ol) = range(parts.next()?)?;
    let (ns, nl) = range(parts.next()?.strip_prefix('+')?)?;
    Some(Hunk { old_start: os, old_len: ol, new_start: ns, new_len: nl })
}

fn range(r: &str) -> Option<(u32, u32)> {
    match r.split_once(',') {
        Some((s, l)) => Some((s.parse().ok()?, l.parse().ok()?)),
        None => Some((r.parse().ok()?, 1)),
    }
}

/// Old-side lines a hunk touches; a pure insertion touches the lines around it.
pub fn old_lines(h: &Hunk) -> Vec<u32> {
    if h.old_len == 0 {
        vec![h.old_start, h.old_start + 1]
    } else {
        (h.old_start..h.old_start + h.old_len).collect()
    }
}

/// The old-side lines a new-side line came from, given the file's hunks.
pub fn to_old(hunks: &[Hunk], line: u32) -> Vec<u32> {
    let mut offset: i64 = 0;
    for h in hunks {
        let changed_here = h.new_len > 0 && line >= h.new_start && line < h.new_start + h.new_len;
        if changed_here {
            return old_lines(h);
        }
        let after = if h.new_len == 0 { line > h.new_start } else { line >= h.new_start + h.new_len };
        if !after {
            break;
        }
        offset += h.old_len as i64 - h.new_len as i64;
    }
    vec![(line as i64 + offset).max(1) as u32]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,0 +4,2 @@ fn x\n+a\n+b\n@@ -10,2 +12,1 @@\n-c\n-d\n+e\ndiff --git a/src/new.rs b/src/new.rs\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1 @@\n+z\n";

    #[test]
    fn parses_files_and_hunks() {
        let f = parse(DIFF);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].old.as_deref(), Some("src/a.rs"));
        assert_eq!(f[0].hunks[0], Hunk { old_start: 3, old_len: 0, new_start: 4, new_len: 2 });
        assert_eq!(f[0].hunks[1], Hunk { old_start: 10, old_len: 2, new_start: 12, new_len: 1 });
        assert_eq!(f[1].old, None);
        assert_eq!(f[1].new.as_deref(), Some("src/new.rs"));
    }

    #[test]
    fn maps_new_lines_back_to_old_lines() {
        let h = &parse(DIFF)[0].hunks;
        assert_eq!(to_old(h, 2), vec![2]);
        assert_eq!(to_old(h, 4), vec![3, 4]);
        assert_eq!(to_old(h, 8), vec![6]);
        assert_eq!(to_old(h, 12), vec![10, 11]);
        assert_eq!(to_old(h, 20), vec![19]);
    }

    #[test]
    fn an_insertion_touches_the_lines_around_it() {
        assert_eq!(old_lines(&Hunk { old_start: 3, old_len: 0, new_start: 4, new_len: 2 }), vec![3, 4]);
    }
}
