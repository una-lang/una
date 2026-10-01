//! Discovery of conformance cases (`compiler/testing.md §2`, "Discovery"; ADR 0013, D5).

use std::collections::BTreeMap;
use std::path::Path;

/// The areas of `compiler/testing.md §2`.
pub const AREAS: [&str; 10] = [
    "syntax",
    "fmt",
    "names",
    "types",
    "authority",
    "security",
    "verification",
    "runtime",
    "examples",
    "project",
];

/// How a case is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseKind {
    /// One `.una` file, compiled as a package containing only that module.
    Single,
    /// A directory with `una.toml` and `src/`.
    Directory,
}

/// A discovered case: its path relative to the conformance root, and its layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseLocation {
    pub path: String,
    pub kind: CaseKind,
}

/// An entry of a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    File(String),
    Dir { name: String, has_una_toml: bool },
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::File(name) | Entry::Dir { name, .. } => name,
        }
    }

    /// Whether the name starts with `.`; hidden entries are ignored (ADR 0013, D6).
    fn is_hidden(&self) -> bool {
        self.name().starts_with('.')
    }
}

/// The areas listed at the conformance root, and an error for every directory that is not an
/// area. Files at the root, such as `README.md`, are not cases; hidden entries are ignored.
pub fn areas(entries: &[Entry]) -> (Vec<String>, Vec<String>) {
    let mut areas = Vec::new();
    let mut errors = Vec::new();
    for entry in entries.iter().filter(|entry| !entry.is_hidden()) {
        if let Entry::Dir { name, .. } = entry {
            if AREAS.contains(&name.as_str()) {
                areas.push(name.clone());
            } else {
                errors.push(format!("{name}/: not an area of compiler/testing.md §2"));
            }
        }
    }
    (areas, errors)
}

/// The cases of an area, and an error for every entry that is not a case: every `.una` file is
/// a single-module case and every directory containing `una.toml` is a multi-module case.
/// Hidden entries are ignored.
pub fn cases(area: &str, entries: &[Entry]) -> (Vec<CaseLocation>, Vec<String>) {
    let mut cases = Vec::new();
    let mut errors = Vec::new();
    for entry in entries.iter().filter(|entry| !entry.is_hidden()) {
        match entry {
            Entry::File(name) if name.ends_with(".una") => cases.push(CaseLocation {
                path: format!("{area}/{name}"),
                kind: CaseKind::Single,
            }),
            Entry::Dir {
                name,
                has_una_toml: true,
            } => cases.push(CaseLocation {
                path: format!("{area}/{name}"),
                kind: CaseKind::Directory,
            }),
            Entry::File(name) => errors.push(format!(
                "{area}/{name}: not a case (a case is a .una file or a directory with una.toml)"
            )),
            Entry::Dir {
                name,
                has_una_toml: false,
            } => errors.push(format!(
                "{area}/{name}/: not a case (a multi-module case needs una.toml)"
            )),
        }
    }
    (cases, errors)
}

/// Discovers the cases under the conformance root, in path order, with the entries that are not
/// cases.
pub fn discover(root: &Path) -> (Vec<CaseLocation>, Vec<String>) {
    let (areas, mut errors) = areas(&list(root));
    let mut found = Vec::new();
    for area in areas {
        let (area_cases, area_errors) = cases(&area, &list(&root.join(&area)));
        found.extend(area_cases);
        errors.extend(area_errors);
    }
    (found, errors)
}

/// The files of a case, keyed by path relative to the case: the file itself for a single-module
/// case, every file under the case directory for a multi-module case, except hidden ones.
pub fn read_case_files(root: &Path, case: &CaseLocation) -> BTreeMap<String, Vec<u8>> {
    let path = root.join(&case.path);
    let mut files = BTreeMap::new();
    match case.kind {
        CaseKind::Single => {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            files.insert(name, std::fs::read(&path).unwrap());
        }
        CaseKind::Directory => collect(&path, "", &mut files),
    }
    files
}

fn collect(dir: &Path, prefix: &str, files: &mut BTreeMap<String, Vec<u8>>) {
    for entry in list(dir).into_iter().filter(|entry| !entry.is_hidden()) {
        match entry {
            Entry::File(name) => {
                let bytes = std::fs::read(dir.join(&name)).unwrap();
                files.insert(format!("{prefix}{name}"), bytes);
            }
            Entry::Dir { name, .. } => {
                collect(&dir.join(&name), &format!("{prefix}{name}/"), files)
            }
        }
    }
}

/// The entries of a directory, sorted by name.
fn list(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let path = entry.path();
            if path.is_dir() {
                Entry::Dir {
                    has_una_toml: path.join("una.toml").is_file(),
                    name,
                }
            } else {
                Entry::File(name)
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name().cmp(b.name()));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Entry {
        Entry::File(name.to_owned())
    }

    fn dir(name: &str, has_una_toml: bool) -> Entry {
        Entry::Dir {
            name: name.to_owned(),
            has_una_toml,
        }
    }

    #[test]
    fn root_directories_must_be_areas_and_root_files_are_not_cases() {
        let (areas, errors) = areas(&[
            file("README.md"),
            dir("authority", false),
            dir("misc", false),
            dir("syntax", false),
        ]);
        assert_eq!(areas, ["authority", "syntax"]);
        assert_eq!(errors, ["misc/: not an area of compiler/testing.md §2"]);
    }

    #[test]
    fn an_area_holds_una_files_and_directories_with_una_toml() {
        let (cases, errors) = cases(
            "names",
            &[
                file("cycle.una"),
                dir("imports", true),
                dir("leftovers", false),
                file("notes.md"),
            ],
        );
        assert_eq!(
            cases,
            [
                CaseLocation {
                    path: "names/cycle.una".to_owned(),
                    kind: CaseKind::Single,
                },
                CaseLocation {
                    path: "names/imports".to_owned(),
                    kind: CaseKind::Directory,
                },
            ]
        );
        assert_eq!(
            errors,
            [
                "names/leftovers/: not a case (a multi-module case needs una.toml)",
                "names/notes.md: not a case (a case is a .una file or a directory with una.toml)",
            ]
        );
    }

    #[test]
    fn hidden_entries_are_ignored_at_the_root_and_in_areas() {
        let (areas, errors) = areas(&[
            file(".DS_Store"),
            dir(".cache", false),
            dir("syntax", false),
        ]);
        assert_eq!(areas, ["syntax"]);
        assert_eq!(errors, Vec::<String>::new());
        let (cases, errors) = cases(
            "syntax",
            &[
                file(".DS_Store"),
                file(".draft.una"),
                dir(".old", true),
                file("tab.una"),
            ],
        );
        assert_eq!(
            cases,
            [CaseLocation {
                path: "syntax/tab.una".to_owned(),
                kind: CaseKind::Single,
            }]
        );
        assert_eq!(errors, Vec::<String>::new());
    }

    #[test]
    fn hidden_files_of_a_multi_module_case_are_not_read() {
        let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hidden-files-case");
        // Temporary files stay under target/, inside the repository.
        let _ = std::fs::remove_dir_all(&root);
        let case = root.join("names/imports");
        std::fs::create_dir_all(case.join("src/.cache")).unwrap();
        for (path, text) in [
            ("una.toml", "[package]\n"),
            (".DS_Store", ""),
            ("src/a.una", "module a\n"),
            ("src/.b.una", "module b\n"),
            ("src/.cache/c.una", "module c\n"),
        ] {
            std::fs::write(case.join(path), text).unwrap();
        }
        let files = read_case_files(
            &root,
            &CaseLocation {
                path: "names/imports".to_owned(),
                kind: CaseKind::Directory,
            },
        );
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(files.keys().collect::<Vec<_>>(), ["src/a.una", "una.toml"]);
    }

    #[test]
    fn discovers_the_initial_cases() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance");
        let (cases, errors) = discover(&root);
        assert_eq!(errors, Vec::<String>::new());
        for path in [
            "authority/cap_as_value.una",
            "security/fail_under_secret_pc.una",
            "verification/postcondition_refuted.una",
        ] {
            assert!(
                cases.contains(&CaseLocation {
                    path: path.to_owned(),
                    kind: CaseKind::Single,
                }),
                "{path} is not discovered"
            );
        }
    }
}
