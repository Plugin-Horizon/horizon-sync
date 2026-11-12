//! `cargo run --example check_original -- <path to original file>...`
//!
//! Loads HuskSync's original files (`config.yml`, `server.yml`, `messages-<language>.yml`) with the
//! port's own loader and prints a redacted summary: the key paths that were read, the key paths
//! of the file that were not read, and parse errors. No value is printed, and no file path beyond
//! the file name. A `config.yml` is loaded with `Config::from_huskysync` from its own folder, so
//! its `server.yml` and locale file are used when they sit next to it.

use horizonsync_core::{config, messages, yaml};
use std::path::Path;

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: check_original <file>...");
        std::process::exit(2);
    }
    let mut failed = false;
    for file in &files {
        let path = Path::new(file);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_owned();
        println!("== {name}");
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                println!("  read error");
                failed = true;
                continue;
            }
        };
        let node = match yaml::parse(&text) {
            Ok(n) => n,
            Err(e) => {
                println!("  parse error at line {}: {}", e.line, e.message);
                failed = true;
                continue;
            }
        };
        let (read, loader): (Vec<String>, Option<Result<(), String>>) = match name.as_str() {
            "config.yml" => {
                let dir = path.parent().unwrap_or(Path::new("."));
                let loaded = config::Config::from_huskysync(dir).map(|_| ()).map_err(|_| String::new());
                (read_keys(config::CONFIG_READ_KEYS), Some(loaded))
            }
            "server.yml" => (read_keys(config::SERVER_READ_KEYS), None),
            n if n.starts_with("messages-") => {
                let keys: Vec<String> = messages::used_keys().map(|k| format!("locales.{k}")).collect();
                let loaded = messages::Messages::from_yaml(&text).map(|_| ()).map_err(|_| String::new());
                (keys, Some(loaded))
            }
            _ => {
                println!("  not a HuskSync file this port reads");
                failed = true;
                continue;
            }
        };
        let refs: Vec<&str> = read.iter().map(String::as_str).collect();
        let report = config::key_report(&node, &refs);
        println!("  read ({}): {}", report.read.len(), report.read.join(" "));
        println!("  not read ({}): {}", report.not_read.len(), report.not_read.join(" "));
        match loader {
            // A loader error may echo a line of the file, so only the fact is printed.
            Some(Err(_)) => {
                println!("  loader: failed (details withheld, they can echo values)");
                failed = true;
            }
            Some(Ok(())) => println!("  loader: ok"),
            None => {}
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// `CONFIG_READ_KEYS`-style lists to owned strings for the report helper.
fn read_keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| (*k).to_owned()).collect()
}
