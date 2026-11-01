//! `cargo run -p una-link --example scan-relocations -- FILE...`: the relocation scan of ADR 0124
//! rule 24 over objects and archives, one line per (type, section, referent) with its count.
#![allow(clippy::print_stdout, clippy::print_stderr)]

fn main() -> std::process::ExitCode {
    let mut total = std::collections::BTreeMap::new();
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("scan-relocations: cannot read {path}: {error}");
                return std::process::ExitCode::FAILURE;
            }
        };
        match una_link::scan_relocations(&path, &bytes) {
            Ok(counts) => {
                for (key, count) in counts {
                    *total.entry(key).or_insert(0u64) += count;
                }
            }
            Err(problem) => {
                eprintln!("scan-relocations: {problem}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    for (key, count) in total {
        println!("{count:>9} {key}");
    }
    std::process::ExitCode::SUCCESS
}
