//! The glibc floor of the shipped Linux `una` (#2105): `scripts/glibc-floor.sh` reads the highest
//! `GLIBC_` symbol version an executable asks the loader for and refuses one above the floor,
//! shown here on listings a stand-in `objdump` prints; `scripts/check-release.sh` runs it with
//! the floor of `install.sh`, which is the glibc of the image the archive is built in (2.28,
//! AlmaLinux 8's; 2.39, Ubuntu 24.04's, through 0.5.1), and the check is what keeps it there.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap()
}

/// One line of `objdump -T` for an undefined symbol of `version` (`GLIBC_2.34`), weak or not.
fn line(version: &str, name: &str, weak: bool) -> String {
    let binding = if weak { "  w " } else { "    " };
    format!("0000000000000000 {binding}  DF *UND*\t0000000000000000 ({version}) {name}\n")
}

/// Runs the script with `floor` on a file whose `objdump -T` listing is `listing`, or fails
/// when `listing` is `None`; returns the exit code, stdout and stderr.
fn floor_of(dir: &Path, floor: &str, listing: Option<&str>) -> (Option<i32>, String, String) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(dir.join("una"), "an executable").unwrap();
    let body = match listing {
        Some(listing) => {
            std::fs::write(dir.join("listing"), listing).unwrap();
            format!(
                "#!/bin/sh\n[ \"$1\" = -T ] || exit 9\ncat '{}'\n",
                dir.join("listing").display()
            )
        }
        None => "#!/bin/sh\necho 'file format not recognized' >&2\nexit 1\n".to_owned(),
    };
    let objdump = dir.join("bin/objdump");
    std::fs::write(&objdump, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&objdump, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The test's own PATH, with the stand-in in front (compiler/conventions.md §1 covers libraries).
    #[allow(clippy::disallowed_methods)]
    let inherited = std::env::var("PATH").unwrap();
    let output = std::process::Command::new("sh")
        .arg(root().join("scripts/glibc-floor.sh"))
        .arg(floor)
        .arg(dir.join("una"))
        .env("PATH", format!("{}:{inherited}", dir.join("bin").display()))
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn an_executable_at_or_below_the_floor_passes_and_one_above_it_is_refused() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("glibc-floor");
    let header = "\nuna:     file format elf64-littleaarch64\n\nDYNAMIC SYMBOL TABLE:\n";
    let unwinder = "0000000000000000      DF *UND*\t0000000000000000 (GCC_3.0)    _Unwind_Resume\n";
    let base = format!(
        "{header}{}{unwinder}{}{}",
        line("GLIBC_2.17", "memmove", false),
        line("GLIBC_2.4", "an_old_one", false),
        line("GLIBC_2.34", "__libc_start_main", false),
    );

    // At the floor and below it, compared as numbers: 2.4 is before 2.35, though not as text.
    for floor in ["2.34", "2.35", "2.100", "3.0"] {
        let (code, stdout, stderr) = floor_of(&dir, floor, Some(&base));
        assert_eq!(code, Some(0), "{floor}: {stderr}");
        assert!(
            stdout.contains(&format!(
                "una: highest symbol version GLIBC_2.34, floor {floor}\n"
            )),
            "{floor}: {stdout}"
        );
    }

    // Above the floor: refused, with each symbol named and no line for the record.
    let (code, stdout, stderr) = floor_of(&dir, "2.33", Some(&base));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(
        stderr.contains("needs a glibc above the floor 2.33"),
        "{stderr}"
    );
    assert!(
        stderr.contains("__libc_start_main (GLIBC_2.34)"),
        "{stderr}"
    );
    assert!(!stderr.contains("memmove"), "{stderr}");
    let (code, _, stderr) = floor_of(&dir, "2.3", Some(&base));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("an_old_one (GLIBC_2.4)"), "{stderr}");

    // What the archive of 0.4.1 asked for: C23's entry points of glibc 2.38, and, weakly, the
    // process descriptors of 2.39. A weak reference counts too.
    let newer = format!(
        "{base}{}{}",
        line("GLIBC_2.38", "__isoc23_strtol", false),
        line("GLIBC_2.39", "pidfd_spawnp", true),
    );
    let (code, _, stderr) = floor_of(&dir, "2.35", Some(&newer));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("__isoc23_strtol (GLIBC_2.38)"), "{stderr}");
    assert!(stderr.contains("pidfd_spawnp (GLIBC_2.39)"), "{stderr}");
    let (code, stdout, _) = floor_of(&dir, "2.39", Some(&newer));
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("highest symbol version GLIBC_2.39, floor 2.39"),
        "{stdout}"
    );
    let weak_only = format!("{base}{}", line("GLIBC_2.36", "arc4random", true));
    assert_eq!(floor_of(&dir, "2.35", Some(&weak_only)).0, Some(1));

    // A version with a third number is above the floor of its first two, and a version that is
    // not numbers, glibc's private one, is refused at any floor.
    let patch = format!("{base}{}", line("GLIBC_2.35.1", "a_patch_one", false));
    let (code, _, stderr) = floor_of(&dir, "2.35", Some(&patch));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("a_patch_one (GLIBC_2.35.1)"), "{stderr}");
    assert_eq!(floor_of(&dir, "2.36", Some(&patch)).0, Some(0));
    let private = format!("{base}{}", line("GLIBC_PRIVATE", "__libc_fatal", false));
    let (code, _, stderr) = floor_of(&dir, "9.9", Some(&private));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("__libc_fatal (GLIBC_PRIVATE)"), "{stderr}");
}

#[test]
fn a_file_whose_versions_cannot_be_read_never_passes() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("glibc-floor-unread");
    // No GLIBC_ version in the listing: a static executable, or one of another C library.
    let none = "\nuna:     file format elf64-littleaarch64\n\nDYNAMIC SYMBOL TABLE:\nno symbols\n";
    let (code, stdout, stderr) = floor_of(&dir, "2.35", Some(none));
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(
        stderr.contains("names no GLIBC_ symbol version"),
        "{stderr}"
    );
    // objdump fails on the file.
    let (code, stdout, stderr) = floor_of(&dir, "2.35", None);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("objdump -T cannot read"), "{stderr}");
    // A floor that is not MAJOR.MINOR.
    for floor in ["2", "2.", ".35", "2.35.1", "2.x", "", "glibc 2.35"] {
        let (code, _, stderr) = floor_of(&dir, floor, Some(none));
        assert_eq!(code, Some(2), "{floor}: {stderr}");
        assert!(stderr.contains("is not MAJOR.MINOR"), "{floor}: {stderr}");
    }
}

#[test]
fn the_release_check_reads_the_floor_of_the_installer_and_the_image_has_that_glibc() {
    // One floor: install.sh sets it on one line, scripts/check-release.sh reads that line, and
    // the image the archive is built in has the glibc of that version, as the lock records it:
    // an executable linked there can ask for no later symbol version, and none earlier is
    // promised.
    let installer = read("install.sh");
    let floors: Vec<&str> = installer
        .lines()
        .filter_map(|line| line.strip_prefix("glibc_floor="))
        .collect();
    let [floor] = floors[..] else {
        panic!("install.sh: one glibc_floor line, found {floors:?}");
    };
    let (major, minor) = floor.split_once('.').expect("MAJOR.MINOR");
    assert!(
        major.parse::<u32>().is_ok() && minor.parse::<u32>().is_ok(),
        "{floor}"
    );
    let check = read("scripts/check-release.sh");
    let read_floor = check
        .find("glibc_floor=$(sed -n 's/^glibc_floor=")
        .expect("check-release.sh reads install.sh's floor");
    let canary = check
        .find("scripts/glibc-floor.sh 2.17 \"$una\"")
        .expect("the canary scan of una");
    let scan = check
        .find("scripts/glibc-floor.sh \"$glibc_floor\" \"$una\"")
        .expect("the scan of una");
    assert!(read_floor < canary && canary < scan);
    assert!(
        check.contains("scripts/glibc-floor.sh \"$glibc_floor\" \"$tmp/install-one/out/hello\"")
    );
    let lock = read("supply-chain/z3.lock");
    for package in ["glibc", "glibc-devel", "glibc-headers"] {
        let version = lock
            .lines()
            .find_map(|line| line.strip_prefix(&format!("\"{package}\" = \"")))
            .unwrap();
        assert!(
            version.starts_with(&format!("{floor}-")),
            "{package}: {version}, floor {floor}"
        );
    }
}
