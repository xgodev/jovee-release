//! Responsibility: runs every listed test alone under coverage and records which functions it executed.

use crate::args::Args;
use crate::export;
use crate::list::{self, Binary};
use crate::map::Builder;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TEST_TIMEOUT: Duration = Duration::from_secs(1200);

pub fn map_run(args: &Args) -> Result<(), String> {
    let tests = list::load(args.req("list")?)?;
    let llvm = args.req("llvm-bin")?;
    let root = args.req("root")?;
    let out = args.req("out")?;
    let jobs: usize = args.opt("jobs").and_then(|j| j.parse().ok()).unwrap_or(4);
    let exclude = args.opt("exclude");
    let work = format!("{out}.d");
    std::fs::create_dir_all(&work).map_err(|e| format!("{work}: {e}"))?;
    let lib_path = tests.library_dirs.join(":");
    let mut builder = Builder::new();
    for bin in &tests.binaries {
        let names: Vec<&String> = bin.tests.iter().filter(|t| exclude.is_none_or(|x| !t.contains(x))).collect();
        if names.is_empty() {
            continue;
        }
        let started = Instant::now();
        let covered = run_binary(bin, &names, llvm, &work, &lib_path, jobs)?;
        let merged = format!("{work}/binary.profdata");
        let parts: Vec<String> = (0..names.len()).map(|i| format!("{work}/{i}.profdata")).filter(|p| Path::new(p).exists()).collect();
        if parts.is_empty() {
            continue;
        }
        profdata(llvm, &[&["merge", "-sparse", "-o", &merged][..], &parts.iter().map(String::as_str).collect::<Vec<_>>()].concat())?;
        let places = export::places(llvm, &bin.path, &merged, root)?;
        for (test, fns) in names.iter().zip(&covered) {
            let id = format!("{}\t{}", bin.id, test);
            for f in fns {
                if let Some(p) = places.get(f) {
                    builder.add(&p.file, p.start, p.end, &id);
                }
            }
        }
        for p in parts.iter().chain(std::iter::once(&merged)) {
            let _ = std::fs::remove_file(p);
        }
        eprintln!("{}: {} tests in {:?}", bin.id, names.len(), started.elapsed());
    }
    builder.finish("").save(out)
}

/// Runs the binary's tests `jobs` at a time; returns each test's executed functions.
fn run_binary(bin: &Binary, names: &[&String], llvm: &str, work: &str, lib_path: &str, jobs: usize) -> Result<Vec<BTreeSet<String>>, String> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<BTreeSet<String>>> = Mutex::new(vec![BTreeSet::new(); names.len()]);
    let error: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= names.len() {
                    break;
                }
                match run_one(bin, names[i], i, llvm, work, lib_path) {
                    Ok(fns) => results.lock().unwrap()[i] = fns,
                    Err(e) => *error.lock().unwrap() = Some(e),
                }
            });
        }
    });
    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }
    Ok(results.into_inner().unwrap())
}

fn run_one(bin: &Binary, name: &str, i: usize, llvm: &str, work: &str, lib_path: &str) -> Result<BTreeSet<String>, String> {
    let raw_dir = format!("{work}/raw-{i}");
    std::fs::create_dir_all(&raw_dir).map_err(|e| format!("{raw_dir}: {e}"))?;
    let mut child = Command::new(&bin.path)
        .args([name, "--exact", "--test-threads", "1"])
        .current_dir(&bin.cwd)
        .env("LLVM_PROFILE_FILE", format!("{raw_dir}/%p-%m.profraw"))
        .env("CARGO_MANIFEST_DIR", &bin.cwd)
        .env("CARGO_PKG_NAME", &bin.package)
        .env("NEXTEST", "1")
        .env("LD_LIBRARY_PATH", lib_path)
        .env("DYLD_FALLBACK_LIBRARY_PATH", lib_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", bin.path))?;
    let started = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if started.elapsed() > TEST_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("timed out: {} {name}", bin.id);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let raws: Vec<String> = std::fs::read_dir(&raw_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path().to_string_lossy().into_owned()))
        .collect();
    let mut fns = BTreeSet::new();
    if !raws.is_empty() {
        let out = format!("{work}/{i}.profdata");
        profdata(llvm, &[&["merge", "-sparse", "-o", &out][..], &raws.iter().map(String::as_str).collect::<Vec<_>>()].concat())?;
        let shown = Command::new(format!("{llvm}/llvm-profdata"))
            .args(["show", "--covered", &out])
            .output()
            .map_err(|e| format!("llvm-profdata show: {e}"))?;
        fns = String::from_utf8_lossy(&shown.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    }
    let _ = std::fs::remove_dir_all(&raw_dir);
    Ok(fns)
}

fn profdata(llvm: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(format!("{llvm}/llvm-profdata")).args(args).output().map_err(|e| format!("llvm-profdata: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("llvm-profdata {}: {}", args[0], String::from_utf8_lossy(&out.stderr)))
    }
}
