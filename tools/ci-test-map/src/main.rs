//! Responsibility: dispatches the command line to the map builder, the map merger or the test selector.
//!
//!   ci-test-map map-run   --list list.json --llvm-bin DIR --root DIR --out part.json [--jobs N] [--exclude S]
//!   ci-test-map map-merge --sha SHA --out map.json part.json...
//!   ci-test-map select    --metadata m.json --base REV [--map map.json] [--list list.json] --out DIR

mod args;
mod diff;
mod export;
mod list;
mod map;
mod metadata;
mod runner;
mod select;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let result = match argv.first().map(String::as_str) {
        Some("map-run") => runner::map_run(&args::Args::parse(&argv[1..])),
        Some("map-merge") => map::merge_cmd(&args::Args::parse(&argv[1..])),
        Some("select") => select::select_cmd(&args::Args::parse(&argv[1..])),
        _ => Err("usage: ci-test-map map-run|map-merge|select ...".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ci-test-map: {e}");
            ExitCode::FAILURE
        }
    }
}
