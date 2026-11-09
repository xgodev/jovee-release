//! Responsibility: reads where each instrumented function lives (file and line range) out of `llvm-cov export`.

use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;

#[derive(Deserialize)]
struct Export {
    data: Vec<Data>,
}

#[derive(Deserialize)]
struct Data {
    functions: Vec<Function>,
}

#[derive(Deserialize)]
struct Function {
    name: String,
    filenames: Vec<String>,
    // [line start, column start, line end, column end, count, file id, expanded file id, kind]
    regions: Vec<Vec<u64>>,
}

/// Where a function's body is: workspace-relative file, first and last line.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Place {
    pub file: String,
    pub start: u32,
    pub end: u32,
}

/// Function name (as `llvm-profdata show` prints it) to its place, for the
/// functions of `root`'s own files.
pub fn places(llvm_bin: &str, binary: &str, profdata: &str, root: &str) -> Result<HashMap<String, Place>, String> {
    let out = Command::new(format!("{llvm_bin}/llvm-cov"))
        .args(["export", "-format=text", "-skip-expansions", "-instr-profile", profdata, binary])
        .output()
        .map_err(|e| format!("llvm-cov export: {e}"))?;
    if !out.status.success() {
        return Err(format!("llvm-cov export {binary}: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let export: Export = serde_json::from_slice(&out.stdout).map_err(|e| format!("llvm-cov export {binary}: {e}"))?;
    let prefix = format!("{}/", root.trim_end_matches('/'));
    let mut places = HashMap::new();
    for f in export.data.into_iter().flat_map(|d| d.functions) {
        let own: Vec<&Vec<u64>> = f.regions.iter().filter(|r| r.len() > 5 && r[5] == 0).collect();
        let (Some(file), false) = (f.filenames.first(), own.is_empty()) else { continue };
        let Some(rel) = file.strip_prefix(&prefix) else { continue };
        let start = own.iter().map(|r| r[0]).min().unwrap_or(0) as u32;
        let end = own.iter().map(|r| r[2]).max().unwrap_or(0) as u32;
        places.insert(f.name, Place { file: rel.to_string(), start, end });
    }
    Ok(places)
}
