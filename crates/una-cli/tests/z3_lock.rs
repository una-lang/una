//! The pin of the Z3 built into `una` (ADR 0124 rule 1, #1206): `supply-chain/z3.lock` names the
//! release tag's commit, its tree, the digests of the source tarball and its mirror, and
//! `supply-chain/z3/` holds the recipe. The pin must agree with `verification.lock`, which names
//! the solver version a package is checked with, and the recipe must touch only the shell front
//! end, which is what `shell-library.patch` is for. The test reads files, and runs nothing but the
//! reader of the shipped targets, `scripts/release-targets.sh` (#1903).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::BTreeMap;
use std::fs;

use sha2::Digest;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The `key = "value"` lines of a lock, up to its first table header.
fn settings(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .take_while(|line| !line.starts_with('['))
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            Some((key.trim().to_string(), value.to_string()))
        })
        .collect()
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[test]
fn the_z3_pin_agrees_with_verification_lock_and_names_its_source() {
    let lock = settings(&read("supply-chain/z3.lock"));
    let solver = settings(&read("verification.lock"));
    assert_eq!(solver["solver"], "z3");
    let version = &lock["version"];
    assert_eq!(
        version, &solver["solver_version"],
        "z3.lock and verification.lock pin one version"
    );
    assert_eq!(lock["tag"], format!("z3-{version}"));
    assert!(is_hex(&lock["commit"], 40), "commit is a git hash");
    assert!(is_hex(&lock["tree"], 40), "tree is a git tree hash");
    assert_ne!(lock["commit"], lock["tree"]);
    assert_eq!(lock["source"], format!("z3-{version}.tar.gz"));
    assert!(is_hex(&lock["source_sha256"], 64), "the tarball's digest");
    assert!(
        is_hex(&lock["source_tar_sha256"], 64),
        "the uncompressed tar's digest"
    );
    assert_ne!(lock["source_sha256"], lock["source_tar_sha256"]);
    // The mirror is a release asset of this repository, named by the version, so a tarball for
    // another version can never be read as this one (the digest is the security boundary).
    assert_eq!(
        lock["mirror"],
        format!(
            "https://github.com/una-lang/una/releases/download/mirror/z3-{version}/z3-{version}.tar.gz"
        )
    );
    // Only the installer still downloads the Z3 project's own binaries, until step 1 ends.
    assert!(
        read("supply-chain/z3.lock").contains("[library]"),
        "the per-target library digests"
    );
}

/// The targets whose `una` embeds the pinned library, each of which needs a `[library]` line,
/// since `scripts/release.sh` refuses a target without one (exit 2): the shipped targets of
/// `scripts/release-targets.txt`, read by the one reader the release scripts use (#1903), which
/// fails on a list that cannot be read or lists no target.
fn library_targets() -> Vec<String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#". "$1/scripts/release-targets.sh" && release_targets "$1/scripts/release-targets.txt""#)
        .arg("sh")
        .arg(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "scripts/release-targets.txt: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let targets: Vec<String> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert!(!targets.is_empty(), "no shipped target");
    targets
}

#[test]
fn the_lock_pins_a_library_digest_for_every_target() {
    let lock = read("supply-chain/z3.lock");
    let table = lock
        .split("\n[library]\n")
        .nth(1)
        .expect("a [library] table");
    let mut pins: BTreeMap<&str, &str> = BTreeMap::new();
    let mut described: Vec<&str> = Vec::new();
    for line in table.lines().take_while(|line| !line.starts_with('[')) {
        if let Some(comment) = line.strip_prefix("# ") {
            if let Some((target, _)) = comment.split_once(": ") {
                described.push(target);
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        // The exact form `scripts/release.sh` reads: `"<triple>" = "<sha256>"`.
        let (target, sha) = line
            .split_once(" = ")
            .unwrap_or_else(|| panic!("not a library line: {line}"));
        let target = target
            .strip_prefix('"')
            .and_then(|t| t.strip_suffix('"'))
            .unwrap_or_else(|| panic!("a quoted target: {line}"));
        let sha = sha
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or_else(|| panic!("a quoted digest: {line}"));
        assert!(is_hex(sha, 64), "{target}: a SHA-256 digest");
        assert!(
            described.contains(&target),
            "{target}: its line follows a comment naming the toolchain and the two cold builds"
        );
        assert!(
            pins.insert(target, sha).is_none(),
            "{target}: one line per target"
        );
    }
    let targets = library_targets();
    for target in &targets {
        assert!(
            pins.contains_key(target.as_str()),
            "supply-chain/z3.lock has no [library] line for {target}"
        );
    }
    for target in pins.keys() {
        assert!(
            targets.iter().any(|t| t == target),
            "{target}: a pinned target that no release builds"
        );
    }
    // Two toolchains never give the same archive; equal digests would mean a copied line.
    let mut digests: Vec<&str> = pins.values().copied().collect();
    digests.sort_unstable();
    digests.dedup();
    assert_eq!(digests.len(), pins.len(), "a digest per target");
}

#[test]
fn the_recipe_builds_a_static_library_without_gmp_and_patches_only_the_shell() {
    let options: Vec<String> = read("supply-chain/z3/cmake-options.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    for required in [
        "CMAKE_BUILD_TYPE=Release",
        "Z3_BUILD_LIBZ3_SHARED=OFF",
        "Z3_USE_LIB_GMP=OFF",
        "Z3_INCLUDE_GIT_HASH=OFF",
        "Z3_INCLUDE_GIT_DESCRIBE=OFF",
        "Z3_BUILD_PYTHON_BINDINGS=OFF",
        "Z3_ENABLE_EXAMPLE_TARGETS=OFF",
        "Z3_BUILD_DOCUMENTATION=OFF",
    ] {
        assert!(
            options.iter().any(|o| o == required),
            "cmake-options.txt lacks {required}"
        );
    }
    for option in &options {
        let (key, value) = option.split_once('=').expect("an option is KEY=VALUE");
        assert!(!key.is_empty() && !value.is_empty(), "{option}");
        assert!(!key.contains(char::is_whitespace), "{option}");
        assert!(
            !(key.contains("BINDINGS") && value == "ON"),
            "{option}: no bindings are built"
        );
        assert!(
            !(key == "Z3_USE_LIB_GMP" && value == "ON"),
            "{option}: nothing under the LGPL is linked"
        );
    }
    // Every patch is listed in the lock with its digest, which is the file's.
    let lock = read("supply-chain/z3.lock");
    let table = lock.split("[patches]\n").nth(1).expect("a [patches] table");
    let patches: Vec<(&str, &str)> = table
        .lines()
        .take_while(|line| !line.starts_with('['))
        .filter_map(|line| {
            let (name, sha) = line.split_once('=')?;
            Some((name.trim().trim_matches('"'), sha.trim().trim_matches('"')))
        })
        .collect();
    assert_eq!(patches.len(), 1, "one patch");
    for (name, sha) in &patches {
        assert!(is_hex(sha, 64), "{name}: a digest");
        let bytes = fs::read(root().join("supply-chain/z3").join(name)).unwrap();
        let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
        assert_eq!(&actual, sha, "{name}: the lock's digest is the file's");
    }
    assert_eq!(patches[0].0, "shell-library.patch");
    // The patch adds the library target and the renamed entry, and touches only src/shell/.
    let patch = read("supply-chain/z3/shell-library.patch");
    let files: Vec<&str> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .collect();
    assert_eq!(files, ["src/shell/CMakeLists.txt", "src/shell/main.cpp"]);
    assert!(
        patch.contains("add_library(shell_lib STATIC"),
        "the library target"
    );
    assert!(patch.contains("OUTPUT_NAME z3shell"), "libz3shell.a");
    assert!(
        patch.contains("extern \"C\" int una_z3_shell_main(int argc, char ** argv)"),
        "the renamed entry"
    );
    assert!(
        patch.contains("#ifdef UNA_Z3_SHELL_LIBRARY"),
        "the entry is renamed only for the library"
    );
    assert!(
        !patch
            .lines()
            .any(|line| line.starts_with('-') && !line.starts_with("---")),
        "the patch removes nothing"
    );
}

#[test]
fn the_build_script_reads_the_lock_and_the_recipe() {
    let script = read("scripts/build-z3.sh");
    for needed in [
        "supply-chain/z3.lock",
        "cmake-options.txt",
        "[patches]",
        "--reproducible",
        "libz3shell.a",
        "-ffile-prefix-map=",
    ] {
        assert!(
            script.contains(needed),
            "scripts/build-z3.sh does not mention {needed}"
        );
    }
    // The digest is checked before anything is extracted or compiled (rule 1).
    let check = script.find("SHA-256 mismatch for $source").unwrap();
    let patch_check = script
        .find("SHA-256 mismatch for supply-chain/z3/$name")
        .unwrap();
    let extract = script.find("tar -xzf").unwrap();
    let apply = script
        .find("git -C \"$src\" apply \"$recipe/$name\"")
        .unwrap();
    assert!(
        check < extract && patch_check < extract && extract < apply,
        "the digest checks come before the extraction, which comes before the patches"
    );
    assert!(!script.contains("-ffp-contract"), "not adopted (rule 1)");
    let exec = Path::new(&root()).join("scripts/build-z3.sh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(
            fs::metadata(exec).unwrap().permissions().mode() & 0o111 != 0,
            "executable"
        );
    }
}

/// The `"key" = "value"` lines of the lock's table `[name]`, in order, up to the next header.
fn table<'a>(lock: &'a str, name: &str) -> Vec<(&'a str, &'a str)> {
    let header = format!("\n[{name}]\n");
    let Some(body) = lock.split(header.as_str()).nth(1) else {
        return Vec::new();
    };
    body.lines()
        .take_while(|line| !line.starts_with('['))
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            // The exact form scripts/check-toolchain.sh reads: `"<name>" = "<value>"`.
            let (key, value) = line
                .split_once(" = ")
                .unwrap_or_else(|| panic!("[{name}]: not a `\"key\" = \"value\"` line: {line}"));
            let key = key
                .strip_prefix('"')
                .and_then(|k| k.strip_suffix('"'))
                .unwrap_or_else(|| panic!("[{name}]: a quoted key: {line}"));
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or_else(|| panic!("[{name}]: a quoted value: {line}"));
            assert!(
                !key.contains('"') && !value.contains('"') && !value.contains(char::is_whitespace),
                "[{name}]: {line}"
            );
            (key, value)
        })
        .collect()
}

/// The static inputs that the linker takes from the image into a Linux `una`: GNU's C++ runtime
/// and GCC's runtime (`-static-libstdc++ -static-libgcc`), and glibc's start-up objects and
/// libc_nonshared.a, as `ld --trace` lists them in the image (#1958).
const TOOLCHAIN_FILES: [&str; 9] = [
    "libstdc++.a",
    "libgcc.a",
    "libgcc_eh.a",
    "crtbeginS.o",
    "crtendS.o",
    "Scrt1.o",
    "crti.o",
    "crtn.o",
    "libc_nonshared.a",
];

#[test]
fn the_lock_records_the_linux_toolchain_of_every_linux_target() {
    // A Linux target's library was built in an image whose packages the Dockerfile does not
    // pin, and its `una` embeds that image's libstdc++.a and libgcc.a, so the lock records the
    // packages' versions and those files' digests, which scripts/check-toolchain.sh checks
    // before the release builds `una` (#1926, #1958).
    let lock = read("supply-chain/z3.lock");
    let targets = library_targets();
    for target in &targets {
        let packages = table(&lock, &format!("toolchain.{target}"));
        let files = table(&lock, &format!("toolchain-files.{target}"));
        if !target.contains("-linux-") {
            assert!(
                packages.is_empty() && files.is_empty(),
                "{target}: not Linux"
            );
            continue;
        }
        let names: Vec<&str> = packages.iter().map(|(name, _)| *name).collect();
        for needed in [
            "gcc-toolset-13-binutils",
            "cmake",
            "make",
            "gcc-toolset-13-gcc",
            "gcc-toolset-13-gcc-c++",
            "gcc-toolset-13-libstdc++-devel",
            "glibc",
            "glibc-devel",
            "glibc-headers",
            "kernel-headers",
            "gcc-toolset-13-runtime",
        ] {
            assert!(
                names.contains(&needed),
                "{target}: the lock records no version of {needed}"
            );
        }
        for (name, version) in &packages {
            assert!(!version.is_empty(), "{target}: {name} has a version");
        }
        let pinned: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            pinned, TOOLCHAIN_FILES,
            "{target}: the linked toolchain files"
        );
        for (name, sha) in &files {
            assert!(is_hex(sha, 64), "{target}: {name} has a SHA-256 digest");
        }
        for (name, _) in packages.iter().chain(&files) {
            // scripts/release.sh reads the [library] table to the end of the file, matching a
            // line by its target triple, so no later key may be one.
            assert!(
                !targets.iter().any(|t| t == name),
                "{target}: {name} is a target triple"
            );
        }
    }
    // scripts/build-z3.sh reads `key = "value"` anywhere in the file; every line of the new
    // tables starts with a quote, so none can be read as one of its settings.
    let top = settings(&lock);
    for key in [
        "version",
        "tag",
        "commit",
        "tree",
        "source",
        "source_sha256",
        "source_tar_sha256",
        "mirror",
    ] {
        let unquoted = lock
            .lines()
            .filter(|line| line.split_once('=').is_some_and(|(k, _)| k.trim() == key))
            .count();
        assert_eq!(unquoted, 1, "{key}: one setting line");
        assert!(top.contains_key(key), "{key}: before the first table");
    }
}

#[test]
fn the_release_checks_the_linux_toolchain_before_it_builds_una() {
    let script = read("scripts/release.sh");
    let musl = script.find("UNA_EMBED_MUSL_SHA256S=$musl_digests").unwrap();
    let check = script
        .find("*-linux-*) scripts/check-toolchain.sh \"$target\" || exit 2 ;;")
        .expect("release.sh runs scripts/check-toolchain.sh for a Linux target");
    let una = script
        .find("--package una-cli")
        .expect("release.sh builds una-cli");
    assert!(
        musl < check && check < una,
        "the check comes before `una` is built"
    );
}

/// Runs scripts/check-toolchain.sh for `target` in a scratch tree at `dir` with `lock` as its
/// lock, and fakes of `rpm` (the versions of `installed`, `<package> <version>` lines)
/// and `cc` (`-print-file-name=<file>` names `dir`/files/<file> when it exists, else the bare
/// name, as GCC does) first on PATH, with `files` (name, bytes) in `dir`/files.
fn check_toolchain(
    dir: &Path,
    target: &str,
    lock: &str,
    installed: &str,
    files: &[(&str, &[u8])],
) -> std::process::Output {
    let _ = fs::remove_dir_all(dir);
    for sub in ["scripts", "supply-chain", "bin", "files"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::copy(
        root().join("scripts/check-toolchain.sh"),
        dir.join("scripts/check-toolchain.sh"),
    )
    .unwrap();
    fs::write(dir.join("supply-chain/z3.lock"), lock).unwrap();
    fs::write(dir.join("installed"), installed).unwrap();
    for (name, bytes) in files {
        fs::write(dir.join("files").join(name), bytes).unwrap();
    }
    let rpm = format!(
        "#!/bin/sh\nfor last; do :; done\nv=$(sed -n \"s/^$last //p\" '{}')\n[ -n \"$v\" ] || exit 1\nprintf '%s' \"$v\"\n",
        dir.join("installed").display()
    );
    let cc = format!(
        "#!/bin/sh\nf=${{1#-print-file-name=}}\nif [ -f '{0}'/\"$f\" ]; then printf '%s\\n' '{0}'/\"$f\"; else printf '%s\\n' \"$f\"; fi\n",
        dir.join("files").display()
    );
    for (name, body) in [("rpm", rpm), ("cc", cc)] {
        fs::write(dir.join("bin").join(name), body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                dir.join("bin").join(name),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    }
    // The test's own PATH, with the fakes in front (compiler/conventions.md §1 covers libraries).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    std::process::Command::new("sh")
        .arg(dir.join("scripts/check-toolchain.sh"))
        .arg(target)
        .env("PATH", format!("{}:{inherited}", dir.join("bin").display()))
        .output()
        .unwrap()
}

#[test]
fn check_toolchain_accepts_the_recorded_image_and_refuses_any_drift() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("check-toolchain");
    let target = "aarch64-unknown-linux-gnu";
    let a = b"libstdc++ bytes".as_slice();
    let b = b"crt1 bytes".as_slice();
    let sha = |bytes: &[u8]| format!("{:x}", sha2::Sha256::digest(bytes));
    let lock = |version: &str, digest: &str| {
        format!(
            "version = \"5.1.0\"\n[library]\n\"{target}\" = \"{}\"\n\n# packages\n[toolchain.{target}]\n\"gcc-toolset-13-gcc-c++\" = \"13.3.1-2.2.el8_10\"\n\"glibc-devel\" = \"{version}\"\n\n[toolchain-files.{target}]\n\"libstdc++.a\" = \"{digest}\"\n\"Scrt1.o\" = \"{}\"\n",
            "ab".repeat(32),
            sha(b)
        )
    };
    let installed = "gcc-toolset-13-gcc-c++ 13.3.1-2.2.el8_10\nglibc-devel 2.28-251.el8_10.53\n";
    let run = |lock: &str, installed: &str, files: &[(&str, &[u8])]| {
        let output = check_toolchain(&dir, target, lock, installed, files);
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let both: [(&str, &[u8]); 2] = [("libstdc++.a", a), ("Scrt1.o", b)];
    let good = lock("2.28-251.el8_10.53", &sha(a));

    // The image as recorded: every package and file is printed for the record.
    let (code, stdout, stderr) = run(&good, installed, &both);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains(&format!(
        "toolchain: {target}: glibc-devel 2.28-251.el8_10.53\n"
    )));
    assert!(stdout.contains(&format!(
        "toolchain: {target}: libstdc++.a sha256 {}\n",
        sha(a)
    )));
    assert!(stdout.contains(&format!("toolchain: {target}: Scrt1.o sha256 {}\n", sha(b))));

    // An AlmaLinux point update of a package.
    let (code, _, stderr) = run(&lock("2.28-251.el8_10.54", &sha(a)), installed, &both);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("the package glibc-devel is 2.28-251.el8_10.53 installed, but supply-chain/z3.lock records 2.28-251.el8_10.54"),
        "{stderr}"
    );

    // A package that the image lacks.
    let (code, _, stderr) = run(&good, "gcc-toolset-13-gcc-c++ 13.3.1-2.2.el8_10\n", &both);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("the package glibc-devel is not installed"),
        "{stderr}"
    );

    // A linked file whose bytes differ.
    let (code, _, stderr) = run(
        &lock("2.28-251.el8_10.53", &"cd".repeat(32)),
        installed,
        &both,
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "has SHA-256 {}, but supply-chain/z3.lock pins",
            sha(a)
        )),
        "{stderr}"
    );

    // A linked file that `cc` cannot find.
    let (code, _, stderr) = run(&good, installed, &both[..1]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("cc finds no Scrt1.o"), "{stderr}");

    // No table for the target, or a digest table with no digest line.
    for broken in [
        good.replace(&format!("[toolchain.{target}]"), "[toolchain.other]"),
        good.replace(
            &format!("[toolchain-files.{target}]"),
            "[toolchain-files.other]",
        ),
    ] {
        let (code, _, stderr) = run(&broken, installed, &both);
        assert_eq!(code, Some(2), "{stderr}");
        assert!(
            stderr.contains("supply-chain/z3.lock records no [toolchain"),
            "{stderr}"
        );
    }

    // A target that is not a triple never reaches the lock's patterns.
    let output = check_toolchain(&dir, "a.*", &good, installed, &both);
    assert_eq!(output.status.code(), Some(2));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_release_image_installs_the_toolchain_the_lock_records() {
    // scripts/local-ci/Dockerfile.release installs each package of the lock's toolchain table at
    // the version the table records (#2105, review L2 of #2265), so a rebuild gives the same
    // toolchain or fails at install, before scripts/check-toolchain.sh would refuse it.
    let lock = read("supply-chain/z3.lock");
    let dockerfile = read("scripts/local-ci/Dockerfile.release");
    let installed: Vec<&str> = dockerfile
        .lines()
        .map(|line| line.trim().trim_end_matches('\\').trim())
        .collect();
    // One image for both architectures (#2106): each Linux target's table is checked.
    for target in ["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"] {
        let packages = table(&lock, &format!("toolchain.{target}"));
        assert!(!packages.is_empty(), "{target}");
        for (name, version) in packages {
            let pinned = format!("{name}-{version}");
            assert!(
                installed.contains(&pinned.as_str()),
                "Dockerfile.release does not install {pinned} ({target})"
            );
        }
    }
}
