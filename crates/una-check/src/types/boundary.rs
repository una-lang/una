//! Edition 0.3: the boundaries of `una.toml` (ADR 0093 rules 1 and 9).
//!
//! The manifest parser checks a `[boundaries]` entry's form and its conflicts with the other
//! tables. This pass checks it against the package: each key names a module, and each ceiling
//! names a capability that the key's module resolves, once, with operations it declares. It also
//! maps every module to the boundary whose extent holds it, once, so that a call's crossing is one
//! lookup. Each module costs one lookup per segment of its path (ADR 0093 rule 9), so the pass is
//! O(m·depth + b + c) for m modules, b boundaries and c ceiling entries.

use std::collections::BTreeMap;

use una_project::Manifest;
use una_syntax::{FileId, Span};

use super::Report;
use super::env::{Entity, Env};
use super::ty::CapId;

/// A boundary of the package, resolved.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    /// The key, as `una.toml` writes it.
    pub(crate) key: String,
    /// The ceiling: each capability, with the operations it allows.
    pub(crate) ceiling: BTreeMap<CapId, Vec<String>>,
    /// Whether it is a dependency's package extent (ADR 0122 rule 51).
    pub(crate) package: bool,
}

/// The boundaries of the package, and the boundary that holds each module, by index.
#[derive(Clone, Debug, Default)]
pub(crate) struct Boundaries {
    pub(crate) list: Vec<Resolved>,
    /// For each package module, the index in `list` of the boundary whose extent holds it.
    pub(crate) of_module: Vec<Option<usize>>,
    /// Whether the package or a module it imports declares a policy: without one, nothing
    /// releases, so a function value whose origin is not known cannot either (rule 8).
    pub(crate) any_policy: bool,
    /// For each kind, whether a value of it may hold a function value (rule 8).
    pub(crate) function_kinds: Vec<bool>,
}

impl Boundaries {
    /// The boundary whose extent holds module `module`, if any; none for a standard module.
    pub(crate) fn extent(&self, module: usize) -> Option<usize> {
        self.of_module.get(module).copied().flatten()
    }

    /// The boundary that a reference from module `from` to a declaration of module `into`
    /// crosses into: the extent of `into`, unless `from` is in the same one (rules 3, 9 and 11).
    pub(crate) fn crossing(&self, from: usize, into: usize) -> Option<usize> {
        let boundary = self.extent(into)?;
        (self.extent(from) != Some(boundary)).then_some(boundary)
    }
}

/// Checks `[boundaries]` against the package's `modules` (indices `0..modules`) and maps each
/// module to its boundary. U7002 at the entry for a key that names no module of the package, a
/// capability that the key's module does not resolve, a capability that the ceiling names twice
/// under two names, and an operation that the capability does not declare.
pub(crate) fn resolve(
    env: &Env<'_>,
    settings: &Manifest,
    manifest: Option<FileId>,
    modules: usize,
    report: &mut Report<'_>,
) -> Boundaries {
    let mut boundaries = Boundaries {
        list: Vec::new(),
        of_module: vec![None; modules],
        ..Boundaries::default()
    };
    if settings.boundaries.is_empty() {
        return boundaries;
    }
    let mut by_path: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, module) in env.modules.iter().take(modules).enumerate() {
        by_path.entry(module.path.as_str()).or_insert(index);
    }
    let at = |range: &std::ops::Range<usize>| {
        manifest.map(|file| Span::new(file, range.start, range.end))
    };
    // The index in `list` of each boundary of `settings`, or `None` for one whose key names no
    // module.
    let mut listed: Vec<Option<usize>> = Vec::with_capacity(settings.boundaries.len());
    for boundary in &settings.boundaries {
        let key = &boundary.module;
        // A dependency's package extent has no module of its own: its ceiling, the root's `may`
        // lines, names host capabilities, resolved where `std.host` declares them (ADR 0122
        // rule 51).
        let home = if boundary.package {
            Some(env.prelude)
        } else {
            by_path.get(key.value.as_str()).copied()
        };
        let Some(home) = home else {
            if let Some(span) = at(&key.range) {
                report.error(
                    settings.format.code(),
                    span,
                    format!(
                        "[boundaries] lists `{}`, which is not a module of this package",
                        key.value
                    ),
                );
            }
            listed.push(None);
            continue;
        };
        let mut ceiling: BTreeMap<CapId, Vec<String>> = BTreeMap::new();
        // The name under which each capability is first written, for a second name of it.
        let mut written: BTreeMap<CapId, &str> = BTreeMap::new();
        for entry in &boundary.ceiling {
            let capability = env
                .upper(home, &entry.capability)
                .into_iter()
                .find_map(|entity| match entity {
                    Entity::Capability(id) => Some(id),
                    _ => None,
                });
            let Some(id) = capability else {
                if let Some(span) = at(&entry.at.range) {
                    report.error(
                        settings.format.code(),
                        span,
                        format!(
                            "the ceiling of `{}` names `{}`, which is not a capability that {} sees",
                            key.value,
                            entry.capability,
                            if boundary.package {
                                "the standard module `std.host`".to_owned()
                            } else {
                                format!("module `{}`", key.value)
                            }
                        ),
                    );
                }
                continue;
            };
            // Identity is the declaration (ADR 0093 rule 12): one capability under two names is
            // named twice, never merged.
            if let Some(first) = written.get(&id) {
                if let Some(span) = at(&entry.at.range) {
                    report.error(
                        settings.format.code(),
                        span,
                        format!(
                            "the ceiling of `{}` names one capability twice, as `{first}` and `{}`",
                            key.value, entry.capability
                        ),
                    );
                }
                continue;
            }
            written.insert(id, entry.capability.as_str());
            let declared = env.capabilities.get(id);
            let ops = ceiling.entry(id).or_default();
            for op in &entry.ops {
                if declared.is_some_and(|c| !c.ops.iter().any(|o| o.name == op.as_str())) {
                    if let Some(span) = at(&entry.at.range) {
                        report.error(
                            settings.format.code(),
                            span,
                            format!(
                                "the ceiling of `{}` allows `{}.{op}`, which `{}` does not declare",
                                key.value, entry.capability, entry.capability
                            ),
                        );
                    }
                    continue;
                }
                ops.push(op.clone());
            }
        }
        listed.push(Some(boundaries.list.len()));
        boundaries.list.push(Resolved {
            key: key.value.clone(),
            ceiling,
            package: boundary.package,
        });
    }
    // ADR 0093 rule 9: one lookup per segment of each module's path.
    let extents = settings.extents();
    for (module, slot) in env
        .modules
        .iter()
        .take(modules)
        .zip(boundaries.of_module.iter_mut())
    {
        *slot = extents
            .of(&module.path)
            .and_then(|index| listed.get(index).copied().flatten());
    }
    boundaries
}

#[cfg(test)]
mod tests {
    //! ADR 0093 rule 12: identity is the declaration. Una has no alias of a capability today
    //! (an import names it by its own name), so this builds the environment directly and gives
    //! the boundary module a second name for its capability: the guard must hold if one ever
    //! lands.

    use una_project::{Edition, parse_manifest};
    use una_syntax::{Interner, SourceDb, lex, parse};

    use super::super::env::{Entity, Env, Source};
    use super::super::{Report, Standard};
    use super::resolve;

    const MODULE: &str = "module gallery.thumbnailer\n\npub capability Files\n  op read(path: Text) -> Text\n  op write(path: Text, data: Text) -> Unit\n";

    #[test]
    fn a_ceiling_that_names_one_capability_under_two_names_is_u7002() {
        let toml = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[boundaries]\n\"gallery.thumbnailer\" = [\"Files[read]\", \"Store[write]\"]\n";
        let (settings, errors) = parse_manifest(toml);
        assert!(errors.is_empty(), "{errors:?}");
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let manifest = sources.add("una.toml", toml.as_bytes());
        let id = sources.add("src/gallery/thumbnailer.una", MODULE.as_bytes());
        let lexed = lex(sources.file(id), id, &mut interner);
        let parsed = parse(&lexed, id, &interner);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let standard = Standard::load(Edition::V0_3, false, false);
        let mut report = Report::new(&sources);
        let mut env = Env::build(
            vec![Source {
                tree: &parsed.file,
                interner: &interner,
                reports: true,
                mount: None,
                edition: None,
                file: Some(id),
            }],
            standard.source(&standard.prelude),
            standard.after_prelude(),
            None,
            None,
            None,
            Vec::new(),
            Edition::V0_3,
            una_project::Profile::App,
            &mut report,
        );
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        let files = env.upper(0, "Files");
        assert!(matches!(files.as_slice(), [Entity::Capability(_)]));
        env.modules[0].imports.insert("Store", files[0]);

        let boundaries = resolve(&env, &settings, Some(manifest), 1, &mut report);
        let found: Vec<String> = report
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect();
        assert_eq!(
            found,
            [
                "U7002 the ceiling of `gallery.thumbnailer` names one capability twice, as `Files` and `Store`"
            ]
        );
        // The second name grants nothing: the ceiling keeps only what the first allows.
        let ceiling: Vec<&Vec<String>> = boundaries.list[0].ceiling.values().collect();
        assert_eq!(ceiling, [&vec!["read".to_owned()]]);
        assert_eq!(boundaries.of_module, [Some(0)]);
    }
}
