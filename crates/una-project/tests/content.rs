//! The content of a package (ADR 0122 rules 27 to 29): the files it holds, its listing and
//! digest, and its archive, with the scaling test of the digest.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::Instant;

use pretty_assertions::assert_eq;
use una_project::content::{
    ARCHIVE_MAGIC, File, archive, check_path, digest_of, is_package_file, listing, read_archive,
};

fn file(path: &str, bytes: &[u8]) -> File {
    File {
        path: path.to_owned(),
        bytes: bytes.to_vec(),
    }
}

fn package() -> Vec<File> {
    vec![
        file("src/b.una", b"module b\n"),
        file("ita.una", b"ita demo\n  version 0.1.0\n  edition 0.4\n"),
        file("src/a/c.una", b"module a.c\n"),
        file("LICENSE-MIT", b"MIT\n"),
    ]
}

#[test]
fn the_digest_depends_only_on_paths_and_bytes() {
    let mut one = package();
    let mut two = package();
    two.reverse();
    let first = digest_of(&mut one).unwrap();
    assert_eq!(first, digest_of(&mut two).unwrap());
    assert!(first.starts_with("sha256:") && first.len() == 71);
    // The listing is in ascending byte order, one entry per file.
    let listing = listing(&mut one).unwrap();
    let paths: Vec<&str> = listing
        .lines()
        .map(|line| line.split('\0').next().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["LICENSE-MIT", "ita.una", "src/a/c.una", "src/b.una"]
    );
    assert!(listing.starts_with("LICENSE-MIT\u{0}4\u{0}"));
    // A file renamed, or a byte changed, changes it.
    let mut renamed = package();
    renamed[0].path = "src/d.una".to_owned();
    assert_ne!(first, digest_of(&mut renamed).unwrap());
    let mut edited = package();
    edited[0].bytes.push(b' ');
    assert_ne!(first, digest_of(&mut edited).unwrap());
}

#[test]
fn a_package_holds_una_sources_and_its_documents_only() {
    for path in [
        "ita.una",
        "src/a.una",
        "src/a/b.una",
        "README.md",
        "CHANGELOG.md",
        "LICENSE",
        "LICENSE-MIT",
        "LICENSE.txt",
    ] {
        assert!(is_package_file(path), "{path}");
    }
    for path in [
        "ita.lock",
        "src/a.c",
        "src/lib.a",
        "build.rs",
        "src/.una",
        "docs/x.md",
        "LICENSES/x",
        "una.toml",
    ] {
        assert!(!is_package_file(path), "{path}");
    }
}

#[test]
fn paths_are_refused_by_form() {
    for path in [
        "",
        "/src/a.una",
        "src/../a.una",
        "src/./a.una",
        "src/.hidden.una",
        "src/a.",
        "src//a.una",
        "src/CON.una",
        "src/nul.txt",
        "src/a b.una",
        "src/a\\b.una",
        "src/é.una",
    ] {
        assert!(check_path(path).is_err(), "{path:?}");
    }
    assert!(check_path(&format!("src/{}.una", "a".repeat(260))).is_err());
    assert!(check_path(&format!("{}a.una", "s/".repeat(32))).is_err());
    assert!(check_path("src/console.una").is_ok());
}

#[test]
fn listings_refuse_case_collisions_and_file_directory_clashes() {
    let mut twins = vec![file("src/a.una", b""), file("src/A.una", b"")];
    assert!(listing(&mut twins).unwrap_err().contains("ignoring case"));
    let mut clash = vec![file("src/a", b""), file("src/a/b.una", b"")];
    assert!(listing(&mut clash).is_err());
    let mut clash = vec![file("src/A.una", b""), file("src/a.una/b.una", b"")];
    assert!(listing(&mut clash).unwrap_err().contains("directory"));
    let mut foreign = vec![file("src/lib.c", b"int x;")];
    assert!(
        listing(&mut foreign)
            .unwrap_err()
            .contains("not a file a package holds")
    );
}

#[test]
fn an_archive_reads_back_to_its_files_and_digest() {
    let mut files = package();
    let bytes = archive(&mut files).unwrap();
    assert!(bytes.starts_with(format!("{ARCHIVE_MAGIC}\n").as_bytes()));
    let read = read_archive(&bytes).unwrap();
    assert_eq!(read.digest, digest_of(&mut package()).unwrap());
    let back: Vec<(&str, &[u8])> = read
        .files
        .iter()
        .map(|(path, range)| (path.as_str(), &bytes[range.clone()]))
        .collect();
    assert_eq!(back[1], ("ita.una", files[1].bytes.as_slice()));
    assert_eq!(back.len(), 4);
}

#[test]
fn malformed_archives_are_refused() {
    let good = archive(&mut package()).unwrap();
    let listing_start = ARCHIVE_MAGIC.len() + 1;
    let text = String::from_utf8(good.clone()).unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("trailing bytes", [good.clone(), b"x".to_vec()].concat()),
        ("a truncated file", good[..good.len() - 1].to_vec()),
        (
            "another magic",
            text.replacen("ita-archive/1", "ita-archive/2", 1)
                .into_bytes(),
        ),
        ("a leading zero", {
            let mut b = good[..listing_start].to_vec();
            b.extend_from_slice(b"0");
            b.extend_from_slice(&good[listing_start..]);
            b
        }),
        (
            "a length past the end",
            text.replacen(
                &format!("\n{}\n", text.lines().nth(1).unwrap()),
                "\n999999\n",
                1,
            )
            .into_bytes(),
        ),
        ("a changed byte", {
            let mut b = good.clone();
            let last = b.len() - 2;
            b[last] ^= 1;
            b
        }),
        ("an empty archive", Vec::new()),
    ];
    for (what, bytes) in cases {
        assert!(read_archive(&bytes).is_err(), "{what} is refused");
    }
    // A listing out of order, or with a path rule 27 refuses, made by hand.
    for (a, b) in [
        ("src/b.una", "src/a.una"),
        ("src/a.una", "src/a.una"),
        ("src/../x.una", "src/y.una"),
        ("src/a.c", "src/b.una"),
    ] {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let listing = format!("{a}\u{0}0\u{0}{hash}\n{b}\u{0}0\u{0}{hash}\n");
        let bytes = format!("{ARCHIVE_MAGIC}\n{}\n{listing}", listing.len()).into_bytes();
        assert!(read_archive(&bytes).is_err(), "{a}, {b}");
    }
}

/// The digest and the archive reader are linear in the package's size (ADR 0122, Costs):
/// the time per byte of a 16 MiB package stays within a constant factor of that of a 1 MiB one.
/// The factor is generous, for a shared machine; quadratic work would exceed it 16 times over.
#[test]
fn the_digest_and_the_reader_scale_linearly() {
    let per_byte = |size: usize| {
        let files: Vec<File> = (0..16)
            .map(|i| file(&format!("src/m{i:02}.una"), &vec![b'x'; size / 16]))
            .collect();
        let started = Instant::now();
        let bytes = archive(&mut files.clone()).unwrap();
        let read = read_archive(&bytes).unwrap();
        assert_eq!(read.files.len(), 16);
        started.elapsed().as_secs_f64() / size as f64
    };
    // Warm up, then measure.
    per_byte(1 << 20);
    let small = per_byte(1 << 20);
    let large = per_byte(16 << 20);
    assert!(
        large < small * 4.0,
        "per-byte time grew from {small:e} to {large:e}"
    );
}
