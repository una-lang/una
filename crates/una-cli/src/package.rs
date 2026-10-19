//! Loading a package into a source database, and the diagnostics of its project files
//! (ADR 0021, ADR 0022).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use una_diag::{Code, Diagnostic};
use una_project::ita::{check_lock_consistency, parse_ita_lock, parse_ita_manifest};
use una_project::{
    Format, Manifest, ProjectError, Role, discover, parse_lock, parse_manifest,
};
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
    Ok(package)
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
        let manifest = match package.manifest {
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
        let read = manifest.ita.as_deref();
        match (package.lock, package.manifest) {
            (Some(id), _) => {
                let (lock, errors) = parse_ita_lock(package.sources.file(id).text());
                let invalid = !errors.is_empty();
                located(id, errors);
                if !invalid
                    && let (Some(read), Some(manifest_id)) = (read, package.manifest)
                {
                    let consistency = check_lock_consistency(read, &lock);
                    located(manifest_id, consistency.in_manifest);
                    located(id, consistency.in_lock);
                }
            }
            (None, Some(id)) => located(
                id,
                vec![ProjectError {
                    code: Code::U7009,
                    range: 0..0,
                    message: "the package has no ita.lock; run `una ita lock`".to_owned(),
                }],
            ),
            (None, None) => {}
        }
        // Dependencies are read and checked, but this release loads none: a need is refused
        // rather than left unresolved, so no import can reach a package that is not checked.
        if let (Some(read), Some(id)) = (read, package.manifest) {
            located(
                id,
                read.needs
                    .iter()
                    .map(|need| ProjectError {
                        code: Code::U7008,
                        range: need.range.clone(),
                        message: format!(
                            "this release does not load dependencies yet, so `{}` cannot be needed",
                            need.name.value
                        ),
                    })
                    .collect(),
            );
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
