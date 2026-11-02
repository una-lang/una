//! `cargo run -p una-link --example scan-elf -- [--std] FILE...`: the scan of the ELF inputs of a
//! Linux executable (ADR 0124 rules 12 and 24, step 4a) over objects and archives, one line per
//! (type, section kind, referent) with its count, then each common symbol, the number of
//! archive symbol-table entries checked against their members, and musl's version.
//! Files after `--std` are the Rust standard library's, which must define no common symbol.
//! `--root DIR`, first, names the inputs by their path under DIR, so that the output holds no
//! path of the machine (such as its home directory).
#![allow(clippy::print_stdout, clippy::print_stderr)]

fn main() -> std::process::ExitCode {
    let mut total = una_link::ElfScan::default();
    let mut std_common = Vec::new();
    let mut std_files = false;
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let root = if args.first().map(String::as_str) == Some("--root") && args.len() >= 2 {
        let root = args.remove(1);
        args.remove(0);
        Some(format!("{}/", root.trim_end_matches('/')))
    } else {
        None
    };
    for path in args {
        if path == "--std" {
            std_files = true;
            continue;
        }
        let shown = root
            .as_deref()
            .and_then(|root| path.strip_prefix(root))
            .unwrap_or(&path)
            .to_owned();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("scan-elf: cannot read {path}: {error}");
                return std::process::ExitCode::FAILURE;
            }
        };
        match una_link::scan_elf(&shown, &bytes) {
            Ok(scan) => {
                if std_files {
                    std_common.extend(scan.common.iter().cloned());
                }
                total.merge(scan);
            }
            Err(problem) => {
                eprintln!("scan-elf: {problem}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    for (key, count) in &total.counts {
        println!("{count:>9} {key}");
    }
    for symbol in &total.common {
        println!("common {symbol}");
    }
    println!("symbol table entries checked {}", total.indexed);
    match &total.musl_version {
        Some(version) => println!("musl {version}"),
        None => println!("musl: no __libc_version in the inputs"),
    }
    if !std_common.is_empty() {
        eprintln!(
            "scan-elf: the standard library defines common symbols, which the linker does not allocate for it: {}",
            std_common.join(", ")
        );
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
