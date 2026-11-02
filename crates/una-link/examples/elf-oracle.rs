//! `cargo run -p una-link --example elf-oracle -- FIXTURE.o LINKED`: the per-type oracle of the
//! ELF relocation engine (ADR 0124 rule 33, step 4b) against the static-pie executable a system
//! linker made of the fixture object (`una_link::elf_oracle`). It prints each relocation type
//! and action with the number of places where the engine wrote the linker's bytes (or made a
//! choice of the same meaning, each listed as EQUIVALENT), then each place where they differ
//! (MISMATCH), and fails if there is one. scripts/link-oracle-elf.sh runs it on
//! GNU `ld`'s outputs; tests/elf_oracle.rs runs the same comparison on `ld.lld`'s.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [object, linked] = args.as_slice() else {
        eprintln!("usage: elf-oracle FIXTURE.o LINKED");
        return ExitCode::from(2);
    };
    let read = |path: &str| {
        std::fs::read(path).map_err(|error| eprintln!("error: cannot read {path}: {error}"))
    };
    let (Ok(object), Ok(linked)) = (read(object), read(linked)) else {
        return ExitCode::from(2);
    };
    match una_link::elf_oracle(&object, &linked) {
        Ok(report) => {
            for (key, count) in &report.equal {
                println!("{key}: {count}");
            }
            for equivalent in &report.equivalent {
                println!("EQUIVALENT {equivalent}");
            }
            for mismatch in &report.mismatches {
                println!("MISMATCH {mismatch}");
            }
            if report.mismatches.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(problem) => {
            eprintln!("error: {problem}");
            ExitCode::FAILURE
        }
    }
}
