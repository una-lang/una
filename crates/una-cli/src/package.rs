//! Loading a package into a source database, and the diagnostics of its project files
//! (ADR 0021, ADR 0022).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use una_diag::{Code, Diagnostic};
use una_project::ita::{check_consent, check_lock_consistency, parse_ita_lock, parse_ita_manifest};
use una_project::{
    Format, Manifest, ProjectError, Role, discover, parse_lock, parse_manifest,
};
use una_syntax::ita;
use una_syntax::{FileId, SourceDb, Span};

pub use una_project::LoadError;

/// A loaded package: its files in a source database, by role.
#[derive(Debug)]
pub struct Package {
    /// Every file of the package: `una.toml`, `verification.lock`, then the modules.
    pub sources: SourceDb,
    /// Whether the package is a single file checked on its own (ADR 0021 rule 1).
    pub single: bool,
    /// Which manifest the package has (ADR 0122 rule 2).
    pub format: Format,
    /// Whether the directory holds both `ita.una` and `una.toml` (U7008).
    pub both_manifests: bool,
    /// `una.toml` or `ita.una`, for a package directory.
    pub manifest: Option<FileId>,
    /// `verification.lock` or `ita.lock`, when the package has one.
    pub lock: Option<FileId>,
    /// The modules, in path order.
    pub modules: Vec<FileId>,
    /// Where each file is on disk, for `una fmt`.
    pub disk_paths: BTreeMap<FileId, PathBuf>,
    /// The path dependencies of `ita.una`, loaded or not (ADR 0122 rule 7). Their modules are
    /// checked with the root's only once [`project`] mounts them.
    pub dependencies: Vec<Dependency>,
}

/// A path dependency of the root: the need that names it, and its files, or why they could not
/// be read.
#[derive(Debug)]
pub struct Dependency {
    /// The need's index in the root's `needs`.
    pub need: usize,
    /// The directory as `ita.una` writes it, without a trailing `/`: the prefix of its files'
    /// names in the source database.
    pub dir: String,
    /// Its `ita.una`.
    pub manifest: Option<FileId>,
    /// Its modules, in path order.
    pub modules: Vec<FileId>,
    /// Why it could not be loaded.
    pub error: Option<String>,
}

/// Loads the package at `path` (ADR 0021 rule 1).
pub fn load(path: &Path) -> Result<Package, LoadError> {
    load_with(path, &BTreeMap::new())
}

/// Loads the package at `path`, with the text of `overlay` in place of the files it names:
/// the unsaved buffers of an editor (ADR 0063 rule 4).
pub fn load_with(path: &Path, overlay: &BTreeMap<PathBuf, String>) -> Result<Package, LoadError> {
    let mut loaded = discover(path)?;
    for file in &mut loaded.files {
        if let Some(text) = overlay.get(&file.path) {
            file.bytes = text.as_bytes().to_vec();
        }
    }
    let mut package = Package {
        sources: SourceDb::new(),
        single: loaded.single,
        format: loaded.format,
        both_manifests: loaded.both_manifests,
        manifest: None,
        lock: None,
        modules: Vec::new(),
        disk_paths: BTreeMap::new(),
        dependencies: Vec::new(),
    };
    for file in loaded.files {
        let id = package.sources.add(file.name, &file.bytes);
        package.disk_paths.insert(id, file.path);
        match file.role {
            Role::Manifest => package.manifest = Some(id),
            Role::Lock => package.lock = Some(id),
            Role::Module => package.modules.push(id),
        }
    }
    if package.format == Format::Ita
        && let Some(id) = package.manifest
    {
        let text = package.sources.file(id).text().to_owned();
        load_dependencies(path, &text, overlay, &mut package);
    }
    Ok(package)
}

/// Loads the path dependencies that the root's `ita.una` names (ADR 0122 rule 7). A directory
/// that is not relative, that is the root, lies under the root's `src/`, or is named by two
/// needs is not read; nor is one that does not load. Each keeps the reason, which [`project`]
/// reports at its need. O(files of the dependencies).
fn load_dependencies(
    root: &Path,
    manifest: &str,
    overlay: &BTreeMap<PathBuf, String>,
    package: &mut Package,
) {
    let (read, _) = una_syntax::ita::parse_manifest(manifest);
    let canonical_root = std::fs::canonicalize(root).ok();
    let mut seen: Vec<PathBuf> = Vec::new();
    for (index, need) in read.needs.iter().enumerate() {
        let Some(una_syntax::ita::At {
            value: una_syntax::ita::Origin::Path(dir),
            ..
        }) = &need.origin
        else {
            continue;
        };
        let written = dir.trim_end_matches('/').to_owned();
        let mut dependency = Dependency {
            need: index,
            dir: written.clone(),
            manifest: None,
            modules: Vec::new(),
            error: None,
        };
        let refusal = (|| {
            if !una_project::ita::is_relative_dir(&written) || written == "." {
                return Some("its directory is not a relative path to another directory".to_owned());
            }
            let place = root.join(&written);
            let canonical = std::fs::canonicalize(&place).ok()?;
            if let Some(root) = &canonical_root
                && (canonical == *root || canonical.starts_with(root.join("src")))
            {
                return Some("its directory is the package itself, or inside its `src/`".to_owned());
            }
            if seen.contains(&canonical) {
                return Some("another need names the same directory".to_owned());
            }
            seen.push(canonical);
            None
        })();
        if let Some(reason) = refusal {
            dependency.error = Some(reason);
            package.dependencies.push(dependency);
            continue;
        }
        match una_project::discover_dependency(&root.join(&written), &written) {
            Ok(loaded) => {
                for mut file in loaded.files {
                    if let Some(text) = overlay.get(&file.path) {
                        file.bytes = text.as_bytes().to_vec();
                    }
                    let id = package.sources.add(file.name, &file.bytes);
                    package.disk_paths.insert(id, file.path);
                    match file.role {
                        Role::Manifest => dependency.manifest = Some(id),
                        Role::Module => dependency.modules.push(id),
                        Role::Lock => {}
                    }
                }
            }
            Err(error) => dependency.error = Some(error.to_string()),
        }
        package.dependencies.push(dependency);
    }
}

/// The settings of the package and the diagnostics of its project files. A single file has
/// the default settings and lock (ADR 0021); an invalid `una.toml` still yields its settings
/// that parsed.
pub fn project(package: &Package) -> (Manifest, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut located = |file: FileId, errors: Vec<ProjectError>| {
        for error in errors {
            let span = Span::new(file, error.range.start, error.range.end);
            diagnostics.push(Diagnostic::error(
                error.code,
                error.message,
                package.sources.location(span),
            ));
        }
    };
    if package.format == Format::Ita {
        let mut manifest = match package.manifest {
            Some(id) => {
                let (manifest, errors) = parse_ita_manifest(package.sources.file(id).text());
                located(id, errors);
                if package.both_manifests {
                    located(
                        id,
                        vec![ProjectError {
                            code: Code::U7008,
                            range: 0..0,
                            message: "the package has both ita.una and una.toml; remove una.toml, whose settings ita.una replaces".to_owned(),
                        }],
                    );
                }
                manifest
            }
            None => Manifest::default(),
        };
        let Some(manifest_id) = package.manifest else {
            return (manifest, diagnostics);
        };
        let read = manifest.ita.clone();
        let needs: &[ita::Need] = read.as_deref().map_or(&[], |read| read.needs.as_slice());
        let root_name = read
            .as_deref()
            .and_then(|read| read.name.as_ref())
            .map(|name| name.value.to_string())
            .unwrap_or_default();
        // The path dependencies: checked, then mounted with their package extents.
        let mut versions: BTreeMap<String, String> = BTreeMap::new();
        for (index, need) in needs.iter().enumerate() {
            let refuse = |message: String| ProjectError {
                code: Code::U7008,
                range: need.range.clone(),
                message,
            };
            let Some(ita::At {
                value: ita::Origin::Path(_),
                ..
            }) = &need.origin
            else {
                // Registry and workspace sources are not loaded by this release: a need of one
                // is refused, so no import reaches a package that is not checked.
                located(
                    manifest_id,
                    vec![refuse(format!(
                        "this release loads only path dependencies, so `{}` cannot be needed",
                        need.name.value
                    ))],
                );
                continue;
            };
            let Some(dependency) = package.dependencies.iter().find(|d| d.need == index) else {
                continue;
            };
            if let Some(reason) = &dependency.error {
                located(
                    manifest_id,
                    vec![refuse(format!(
                        "the path dependency `{}` cannot be loaded: {reason}",
                        dependency.dir
                    ))],
                );
                continue;
            }
            let Some(dependency_manifest) = dependency.manifest else {
                continue;
            };
            let (settings, errors) =
                parse_ita_manifest(package.sources.file(dependency_manifest).text());
            let invalid = !errors.is_empty();
            located(dependency_manifest, errors);
            let mut problems = dependency_problems(need, &settings, &manifest);
            if invalid {
                problems.push(refuse(format!(
                    "the path dependency `{}` has an invalid ita.una",
                    dependency.dir
                )));
            }
            if !problems.is_empty() {
                located(manifest_id, problems);
                continue;
            }
            if let Some(version) = &settings.version {
                versions.insert(dependency.dir.clone(), version.value.clone());
            }
            let local = una_project::ita::local_name(need);
            // Never mounted under a name that is not a local name, that is reserved or has the
            // skeleton of one, or that another need already mounts: a dependency's module must
            // never take the path of a standard module such as `std.host`, or of another
            // package's (rule 7). `ita.una` reports each as U7008 already.
            let unusable = !una_project::names::is_name_word(&local.value, 1, 64)
                || una_project::names::reserved_like(&local.value).is_some()
                || manifest.mounts.iter().any(|mount| mount.local == local.value);
            if unusable {
                located(
                    manifest_id,
                    vec![refuse(format!(
                        "`{}` cannot be the local name of a dependency",
                        local.value
                    ))],
                );
                continue;
            }
            manifest.mounts.push(una_project::Mount {
                local: local.value.clone(),
                src: format!("{}/src/", dependency.dir),
            });
            let manifest_text = package.sources.file(manifest_id).text();
            manifest.boundaries.push(una_project::Boundary {
                module: una_project::Located {
                    value: local.value.clone(),
                    range: need.name.range.clone(),
                },
                ceiling: need
                    .may
                    .iter()
                    .map(|cap| una_project::Ceiling {
                        capability: cap.capability.clone(),
                        ops: cap.ops.iter().map(|op| op.value.clone()).collect(),
                        at: una_project::Located {
                            value: manifest_text.get(cap.range.clone()).unwrap_or("").to_owned(),
                            range: cap.range.clone(),
                        },
                    })
                    .collect(),
                package: true,
            });
        }
        // The root's own settings never reach into a dependency: a dependency's module is
        // trusted only by a trust line (ADR 0122 rule 53, not loaded yet), and its boundaries,
        // contracts and entry are its own.
        let inside = |path: &str| {
            manifest.mounts.iter().any(|mount| {
                path.strip_prefix(mount.local.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
        };
        let mut reaching: Vec<ProjectError> = Vec::new();
        for entry in &manifest.trusted {
            if inside(&entry.value) {
                reaching.push(ProjectError {
                    code: Code::U7008,
                    range: entry.range.clone(),
                    message: format!(
                        "`{}` is a dependency's module, which only a trust line of the root can trust",
                        entry.value
                    ),
                });
            }
        }
        for boundary in manifest.boundaries.iter().filter(|b| !b.package) {
            if inside(&boundary.module.value) {
                reaching.push(ProjectError {
                    code: Code::U7008,
                    range: boundary.module.range.clone(),
                    message: format!(
                        "`{}` is inside a dependency, whose package extent already bounds it",
                        boundary.module.value
                    ),
                });
            }
        }
        for contract in &manifest.contracts {
            if inside(&contract.function.value) {
                reaching.push(ProjectError {
                    code: Code::U7008,
                    range: contract.function.range.clone(),
                    message: format!(
                        "`{}` is a dependency's function, which the root's contracts cannot lock",
                        contract.function.value
                    ),
                });
            }
        }
        if let Some(entry) = &manifest.entry
            && inside(&entry.value)
        {
            reaching.push(ProjectError {
                code: Code::U7008,
                range: entry.range.clone(),
                message: format!(
                    "the run entry `{}` is a dependency's module; only the root is run",
                    entry.value
                ),
            });
        }
        located(manifest_id, reaching);
        match package.lock {
            Some(id) => {
                let (lock, errors) = parse_ita_lock(package.sources.file(id).text());
                let invalid = !errors.is_empty();
                located(id, errors);
                if !invalid && let Some(read) = read.as_deref() {
                    let consistency = check_lock_consistency(read, &lock, &versions);
                    located(manifest_id, consistency.in_manifest);
                    located(id, consistency.in_lock);
                    located(manifest_id, check_consent(read, &lock, &root_name));
                }
            }
            None => located(
                manifest_id,
                vec![ProjectError {
                    code: Code::U7009,
                    range: 0..0,
                    message: "the package has no ita.lock; run `una ita lock`".to_owned(),
                }],
            ),
        }
        return (manifest, diagnostics);
    }
    let manifest = match package.manifest {
        Some(id) => {
            let (manifest, errors) = parse_manifest(package.sources.file(id).text());
            located(id, errors);
            manifest
        }
        None => Manifest::default(),
    };
    match (package.lock, package.manifest) {
        (Some(id), _) => located(id, parse_lock(package.sources.file(id).text()).1),
        // A missing lock is located at the start of the package manifest.
        (None, Some(manifest)) => diagnostics.push(Diagnostic::error(
            Code::U7004,
            "the package has no verification.lock; add one next to una.toml",
            package.sources.location(Span::at(manifest, 0)),
        )),
        (None, None) => {}
    }
    (manifest, diagnostics)
}

/// Why a loaded path dependency may not be used as the root writes it (ADR 0122 rules 42, 49
/// and 53, and what this release does not load yet), each at the root's need.
pub(crate) fn dependency_problems(
    need: &ita::Need,
    dependency: &Manifest,
    root: &Manifest,
) -> Vec<ProjectError> {
    let mut problems = Vec::new();
    let mut problem = |code: Code, message: String| {
        problems.push(ProjectError {
            code,
            range: need.range.clone(),
            message,
        });
    };
    let Some(read) = dependency.ita.as_deref() else {
        return problems;
    };
    if let Some(name) = &read.name
        && name.value != need.name.value
    {
        problem(
            Code::U7008,
            format!(
                "the package at this path is `{}`, not `{}`",
                name.value, need.name.value
            ),
        );
    }
    // Rule 42: a local package carries a name that a registry may hold (an unscoped name is
    // Itaúna's, and an owner may be any registry's, which no client can tell offline), so the
    // root accepts it by name.
    if need.accept_name.is_none() {
        problem(
            Code::U7012,
            format!(
                "`{}` is a local package with a name a registry may hold; write `accept name` under the need to use it",
                need.name.value
            ),
        );
    }
    // Rule 49: a dependency's edition is its dependent's or earlier.
    if dependency.edition > root.edition {
        problem(
            Code::U7012,
            format!(
                "`{}` is of edition {}, later than this package's {}",
                need.name.value,
                dependency.edition.name(),
                root.edition.name()
            ),
        );
    }
    let unsupported = [
        (!read.needs.is_empty(), "needs of its own"),
        (!read.labels.is_empty(), "label categories"),
        (!read.trusted.is_empty(), "trusted modules"),
        (!read.boundaries.is_empty(), "boundaries"),
        (!read.contracts.is_empty(), "contracts"),
    ];
    for (present, what) in unsupported {
        if present {
            problem(
                Code::U7008,
                format!(
                    "`{}` has {what}, which this release does not load for a dependency yet",
                    need.name.value
                ),
            );
        }
    }
    for cap in &need.may {
        if cap.capability.contains('.') {
            problem(
                Code::U7008,
                format!(
                    "the ceiling `{}` names a dependency's own capability, which this release does not load yet; a ceiling names host capabilities",
                    cap.capability
                ),
            );
        }
    }
    problems
}
