//! The ELF relocation engine (ADR 0124 rule 24, step 4b) over the real inputs of a Linux
//! executable: every object of the pinned toolchain's musl inputs (`libc.a`, the start-up
//! objects, `libunwind.a`) and of its standard library for both musl targets is read, and every
//! relocation of every section the executable would contain is planned and applied with a
//! synthetic layout of its object. None may be refused.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::print_stdout
)]

use std::path::PathBuf;
use std::process::Command;

use una_link::{ElfCheck, check_elf};

/// The pinned toolchain's library directory for `target`, which rust-toolchain.toml installs.
fn toolchain_lib(target: &str) -> PathBuf {
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("rustc runs");
    let sysroot = String::from_utf8(output.stdout).unwrap();
    let lib = PathBuf::from(sysroot.trim())
        .join("lib/rustlib")
        .join(target)
        .join("lib");
    assert!(
        lib.join("self-contained/libc.a").is_file(),
        "{} is missing: rust-toolchain.toml lists {target}, so `rustup target add {target}` installs it",
        lib.display()
    );
    lib
}

#[test]
fn every_relocation_of_the_real_inputs_is_applied() {
    for target in ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        let lib = toolchain_lib(target);
        let mut all = ElfCheck::default();
        let mut inputs: Vec<PathBuf> = [
            "libc.a",
            "rcrt1.o",
            "crti.o",
            "crtn.o",
            "crtbeginS.o",
            "crtendS.o",
            "libunwind.a",
        ]
        .iter()
        .map(|file| lib.join("self-contained").join(file))
        .collect();
        for entry in std::fs::read_dir(&lib).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".rlib") && !name.starts_with("libprofiler_builtins-") {
                inputs.push(path);
            }
        }
        inputs.sort();
        for path in inputs {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = std::fs::read(&path).unwrap();
            let check = check_elf(&name, &bytes).unwrap_or_else(|p| panic!("{target} {name}: {p}"));
            all.relocations.extend(std::iter::empty());
            for (key, count) in check.relocations {
                *all.relocations.entry(key).or_insert(0) += count;
            }
            all.objects += check.objects;
            all.got += check.got;
            all.fixups += check.fixups;
        }
        println!(
            "{target}: {} objects, {} GOT slots, {} fixups",
            all.objects, all.got, all.fixups
        );
        for (key, count) in &all.relocations {
            println!("  {key}: {count}");
        }
        assert!(all.objects > 1000, "{target}: {} objects", all.objects);
        assert!(all.fixups > 0, "{target}");
        // Every thread-local access of the standard library is relaxed to the local-exec model.
        assert!(
            all.relocations
                .keys()
                .any(|key| key.ends_with("relaxed-tls")),
            "{target}: {:?}",
            all.relocations
        );
    }
}
