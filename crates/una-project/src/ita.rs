//! The project files of edition 0.4 (ADR 0122): `ita.una` read into the one [`Manifest`] of
//! every edition, `ita.lock` read into its solver pin and its packages, and both checked by
//! the rules that need no other file. The grammar is the syntax crate's
//! ([`una_syntax::ita`]); every problem of `ita.una` and `itaty.una` is U7008, and every
//! problem of `ita.lock` is U7009, at its line (rules 13 and 17).

use std::collections::{BTreeMap, BTreeSet};

use una_diag::Code;
use una_syntax::ita::{self, At, CapType, LockOrigin, Origin, PkgName};

use crate::lock::Lock;
use crate::manifest::{
    Boundary, Ceiling, Contract, Edition, Format, Manifest, boundary_conflicts, is_grant,
    is_module_path, is_upper_camel, upper_camel,
};
use crate::names::{is_name_word, reserved_like};
use crate::version::{Line, Version};
use crate::{Located, ProjectError};

/// The largest owner name and the smallest (ADR 0122 rule 44).
const OWNER_BYTES: (usize, usize) = (2, 39);
/// The largest package name (ADR 0122 rules 6 and 44).
const NAME_BYTES: usize = 64;

/// The predeclared source.
pub const ITATY: &str = "itaty";

/// The source names reserved for origins (ADR 0122 rule 8).
const RESERVED_SOURCES: [&str; 4] = ["itaty", "path", "git", "workspace"];

fn error(code: Code, range: std::ops::Range<usize>, message: impl Into<String>) -> ProjectError {
    ProjectError::new(code, range, message)
}

fn located(at: &At<String>) -> Located {
    Located {
        value: at.value.clone(),
        range: at.range.clone(),
    }
}

/// Whether `name` is a valid package name: `name`, or `owner/name` (ADR 0122 rules 6, 44).
/// `Err` says why not.
pub fn check_package_name(name: &PkgName) -> Result<(), String> {
    if let Some(owner) = &name.owner
        && !is_name_word(owner, OWNER_BYTES.0, OWNER_BYTES.1)
    {
        return Err(format!(
            "`{owner}` is not an owner name: lower-case letters, digits and `_`, from {} to {} bytes, starting with a letter, with no `__` and no trailing `_`",
            OWNER_BYTES.0, OWNER_BYTES.1
        ));
    }
    if !is_name_word(&name.name, 1, NAME_BYTES) {
        return Err(format!(
            "`{}` is not a package name: lower-case letters, digits and `_`, at most {NAME_BYTES} bytes, starting with a letter, with no `__` and no trailing `_`",
            name.name
        ));
    }
    Ok(())
}

/// The local name of a need: its `as`, or the last segment of its name (ADR 0122 rule 47).
pub fn local_name(need: &ita::Need) -> At<String> {
    need.local.clone().unwrap_or_else(|| At {
        value: need.name.value.name.clone(),
        range: need.name.range.clone(),
    })
}

/// Reads `ita.una` (ADR 0122 rules 5 to 13): its settings in a [`Manifest`] of format
/// [`Format::Ita`], whose `ita` holds what only `ita.una` writes, and every problem as U7008
/// (U1005 and U1002 for categories, as `una.toml` reports them). O(n) in the file's size, and
/// O(k log k) for the k entries of a section that are compared with each other.
pub fn parse_ita_manifest(text: &str) -> (Manifest, Vec<ProjectError>) {
    let code = Code::U7008;
    let (read, syntax) = ita::parse_manifest(text);
    let mut errors: Vec<ProjectError> = syntax
        .into_iter()
        .map(|e| error(code, e.range, e.message))
        .collect();
    let mut manifest = Manifest {
        format: Format::Ita,
        edition: Edition::V0_4,
        ..Manifest::default()
    };
    if let Some(name) = &read.name {
        if let Err(message) = check_package_name(&name.value) {
            errors.push(error(code, name.range.clone(), message));
        }
        manifest.name = Some(Located {
            value: name.value.to_string(),
            range: name.range.clone(),
        });
    }
    if let Some(version) = &read.version {
        if let Err(message) = Version::parse_full(&version.value) {
            errors.push(error(code, version.range.clone(), message));
        }
        manifest.version = Some(located(version));
    }
    if let Some(edition) = &read.edition {
        match Edition::parse(&edition.value) {
            Some(found) if found >= Edition::V0_4 => manifest.edition = found,
            Some(found) => errors.push(error(
                code,
                edition.range.clone(),
                format!(
                    "ita.una exists from edition 0.4; a package of edition {} keeps una.toml",
                    found.name()
                ),
            )),
            None => errors.push(error(
                code,
                edition.range.clone(),
                format!(
                    "edition `{}` is not supported; ita.una names edition 0.4 or later of {}",
                    edition.value,
                    Edition::listed()
                ),
            )),
        }
    }
    if let Some(profile) = &read.profile {
        match profile.value.as_str() {
            "app" => {}
            // Profiles are not implemented by this release. Checking a `systems` package as
            // `app` would leave out the obligations that profile adds (ADR 0109 rule 2), so it
            // is refused rather than ignored.
            "systems" => errors.push(error(
                code,
                profile.range.clone(),
                "the profile `systems` is not supported by this release; remove the line to check the package as `app`",
            )),
            other => errors.push(error(
                code,
                profile.range.clone(),
                format!("unknown profile `{other}`; the profiles are `app` and `systems`"),
            )),
        }
    }
    for (key, value) in [("about", &read.about), ("license", &read.license)] {
        if let Some(value) = value
            && value.value.trim().is_empty()
        {
            errors.push(error(
                code,
                value.range.clone(),
                format!("`{key}` must not be empty"),
            ));
        }
    }
    if let Some(home) = &read.home
        && !is_https_url(&home.value)
    {
        errors.push(error(
            code,
            home.range.clone(),
            "`home` is an `https://` URL",
        ));
    }
    let sources = check_sources(&read.sources, code, &mut errors);
    if let Some(target) = &read.publish_to
        && target.value != ITATY
        && !sources.contains_key(target.value.as_str())
    {
        errors.push(error(
            code,
            target.range.clone(),
            format!(
                "`publish to {}` names no source; the sources are `itaty` and those of `sources`",
                target.value
            ),
        ));
    }
    check_needs(&read, &sources, code, &mut errors);
    check_replace(&read.replace, &sources, code, &mut errors);
    check_trust(&read, &sources, code, &mut errors);
    labels(&read.labels, &mut manifest, &mut errors);
    for module in &read.trusted {
        manifest.trusted.push(located(module));
    }
    if let Some(budget) = &read.escapes {
        manifest.checked_per_module = budget.value;
    }
    for entry in &read.boundaries {
        let mut ceiling: Vec<Ceiling> = Vec::new();
        for cap in &entry.may {
            if let Some(problem) = ops_problem(cap) {
                errors.push(error(code, cap.range.clone(), problem));
                continue;
            }
            if ceiling.iter().any(|c| c.capability == cap.capability) {
                errors.push(error(
                    code,
                    cap.range.clone(),
                    format!(
                        "the ceiling of `{}` names `{}` twice",
                        entry.module.value, cap.capability
                    ),
                ));
                continue;
            }
            ceiling.push(Ceiling {
                capability: cap.capability.clone(),
                ops: cap.ops.iter().map(|op| op.value.clone()).collect(),
                at: Located {
                    value: text.get(cap.range.clone()).unwrap_or("").to_owned(),
                    range: cap.range.clone(),
                },
            });
        }
        manifest.boundaries.push(Boundary {
            module: located(&entry.module),
            ceiling,
        });
    }
    for entry in &read.contracts {
        manifest.contracts.push(Contract {
            function: located(&entry.function),
            clauses: entry.clauses.iter().map(located).collect(),
        });
    }
    if let Some(run) = &read.run {
        if let Some(entry) = &run.entry {
            manifest.entry = Some(located(entry));
        }
        for grant in &run.grants {
            if !is_grant(&grant.spec) {
                errors.push(error(
                    code,
                    grant.range.clone(),
                    format!("`{}` is not a valid grant", grant.spec),
                ));
                continue;
            }
            // ADR 0117's limits are not implemented by this release. A grant read without the
            // limit written below it would allow more than the manifest says, so it is refused.
            if let Some(limit) = grant.limits.first() {
                errors.push(error(
                    code,
                    limit.clone(),
                    "limits on grants are not supported by this release; a grant without its limit would allow more than written",
                ));
                continue;
            }
            manifest.grants.push(Located {
                value: grant.spec.clone(),
                range: grant.range.clone(),
            });
        }
    }
    if let Some(drive) = &read.drive {
        // ADR 0116's driver is not implemented by this release, so the section is refused
        // rather than ignored.
        errors.push(error(
            code,
            drive.range.clone(),
            "the `drive` section is not supported by this release",
        ));
    }
    if !manifest.boundaries.is_empty() {
        boundary_conflicts(&manifest, &mut errors);
    }
    manifest.ita = Some(Box::new(read));
    (manifest, errors)
}

fn is_https_url(text: &str) -> bool {
    text.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty() && !rest.starts_with('/') && rest.chars().all(|c| c.is_ascii_graphic())
    })
}

/// Whether `text` is a SHA-256 digest as the project files write it: `sha256:` and 64
/// lowercase hexadecimal digits (ADR 0122 rule 16).
pub fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Checks `sources` (rule 8) and returns them by name.
fn check_sources<'a>(
    sources: &'a [ita::Source],
    code: Code,
    errors: &mut Vec<ProjectError>,
) -> BTreeMap<&'a str, &'a ita::Source> {
    let mut by_name: BTreeMap<&str, &ita::Source> = BTreeMap::new();
    let mut owners: BTreeMap<&str, &str> = BTreeMap::new();
    for source in sources {
        let name = source.name.value.as_str();
        if RESERVED_SOURCES.contains(&name) {
            errors.push(error(
                code,
                source.name.range.clone(),
                format!("`{name}` is reserved and cannot name a source"),
            ));
            continue;
        }
        if !is_name_word(name, 1, NAME_BYTES) {
            errors.push(error(
                code,
                source.name.range.clone(),
                format!("`{name}` is not a source name"),
            ));
            continue;
        }
        if by_name.contains_key(name) {
            errors.push(error(
                code,
                source.name.range.clone(),
                format!("the source `{name}` is declared twice"),
            ));
            continue;
        }
        if !is_https_url(&source.url.value) {
            errors.push(error(
                code,
                source.url.range.clone(),
                "a source's base URL is an `https://` URL",
            ));
        }
        if let Some(root) = &source.root
            && !is_digest(&root.value)
        {
            errors.push(error(
                code,
                root.range.clone(),
                "`root` is the digest of the source's root metadata, `sha256:` and 64 lowercase hexadecimal digits",
            ));
        }
        for owner in &source.owners {
            if !is_name_word(&owner.value, OWNER_BYTES.0, OWNER_BYTES.1) {
                errors.push(error(
                    code,
                    owner.range.clone(),
                    format!("`{}` is not an owner name", owner.value),
                ));
            } else if let Some(other) = owners.insert(owner.value.as_str(), name) {
                errors.push(error(
                    code,
                    owner.range.clone(),
                    format!(
                        "the owner `{}` is served by the sources `{other}` and `{name}`; an owner belongs to one source",
                        owner.value
                    ),
                ));
            }
        }
        by_name.insert(name, source);
    }
    by_name
}

/// The source that serves `owner`, if a source of `sources` lists it (rule 8).
fn owner_source<'a>(sources: &BTreeMap<&'a str, &'a ita::Source>, owner: &str) -> Option<&'a str> {
    sources
        .iter()
        .find(|(_, source)| source.owners.iter().any(|o| o.value == owner))
        .map(|(name, _)| *name)
}

/// Checks the origin of a need or a replacement against the root's sources (rules 7, 8, 42).
fn check_origin(
    name: &At<PkgName>,
    origin: Option<&At<Origin>>,
    sources: &BTreeMap<&str, &ita::Source>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) {
    if let Some(At {
        value: Origin::Source(source),
        range,
    }) = origin
        && source != ITATY
        && !sources.contains_key(source.as_str())
    {
        errors.push(error(
            code,
            range.clone(),
            format!(
                "`from {source}` names no source; the sources are `itaty` and those of `sources`"
            ),
        ));
        return;
    }
    if let Some(At {
        value: Origin::Path(dir),
        range,
    }) = origin
        && !is_relative_dir(dir)
    {
        errors.push(error(
            code,
            range.clone(),
            "a path dependency's directory is relative to the package, with `/` separators",
        ));
        return;
    }
    let registry = match origin.map(|o| &o.value) {
        None => Some(ITATY),
        Some(Origin::Source(source)) => Some(source.as_str()),
        _ => None,
    };
    if let (Some(registry), Some(owner)) = (registry, &name.value.owner)
        && let Some(serving) = owner_source(sources, owner)
        && serving != registry
    {
        errors.push(error(
            code,
            origin.map_or(name.range.clone(), |o| o.range.clone()),
            format!(
                "the owner `{owner}` is served by the source `{serving}`; write `from {serving}`"
            ),
        ));
    }
}

/// Whether `dir` is a relative directory: not empty, no leading `/`, no `\`, no drive.
fn is_relative_dir(dir: &str) -> bool {
    !dir.is_empty()
        && !dir.starts_with('/')
        && !dir.contains('\\')
        && !dir.contains(':')
        && !dir.chars().any(char::is_control)
}

/// The operations of a capability, each once (ADR 0093 rule 1).
fn ops_problem(cap: &CapType) -> Option<String> {
    let mut seen = BTreeSet::new();
    for op in &cap.ops {
        if !seen.insert(op.value.as_str()) {
            return Some(format!(
                "`{}` names the operation `{}` twice",
                cap.capability, op.value
            ));
        }
    }
    None
}

fn check_ceiling(caps: &[CapType], code: Code, errors: &mut Vec<ProjectError>) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for cap in caps {
        if let Some(problem) = ops_problem(cap) {
            errors.push(error(code, cap.range.clone(), problem));
        } else if !seen.insert(cap.capability.as_str()) {
            errors.push(error(
                code,
                cap.range.clone(),
                format!("the ceiling names `{}` twice", cap.capability),
            ));
        }
    }
}

fn check_needs(
    read: &ita::Manifest,
    sources: &BTreeMap<&str, &ita::Source>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) {
    let mut locals: BTreeMap<String, String> = BTreeMap::new();
    let mut identities: BTreeSet<String> = BTreeSet::new();
    for need in &read.needs {
        if let Err(message) = check_package_name(&need.name.value) {
            errors.push(error(code, need.name.range.clone(), message));
            continue;
        }
        let local = local_name(need);
        if let Some(local) = &need.local
            && !is_name_word(&local.value, 1, NAME_BYTES)
        {
            errors.push(error(
                code,
                local.range.clone(),
                format!("`{}` is not a local name", local.value),
            ));
        }
        for word in [local.value.as_str(), need.name.value.name.as_str()] {
            if let Some(reserved) = reserved_like(word) {
                let how = if word == reserved {
                    format!("`{word}` is reserved")
                } else {
                    format!("`{word}` has the skeleton of the reserved name `{reserved}`")
                };
                errors.push(error(
                    code,
                    local.range.clone(),
                    format!(
                        "{how} and cannot be a package's local name; give it another with `as`"
                    ),
                ));
                break;
            }
        }
        if let Some(first) = locals.insert(local.value.clone(), need.name.value.to_string()) {
            errors.push(error(
                code,
                local.range.clone(),
                format!(
                    "the local name `{}` is already the one of `{first}`; give one another with `as`",
                    local.value
                ),
            ));
        }
        let local_origin = matches!(
            need.origin.as_ref().map(|o| &o.value),
            Some(Origin::Path(_) | Origin::Workspace)
        );
        match (&need.version, local_origin) {
            (Some(version), true) => errors.push(error(
                code,
                version.range.clone(),
                "a path or workspace need takes its version from the dependency's own `ita.una`; remove it",
            )),
            (None, false) => errors.push(error(
                code,
                need.name.range.clone(),
                format!(
                    "the need of `{}` names no version; write the least version it was tested with, such as `{} 1.0`",
                    need.name.value, need.name.value
                ),
            )),
            (Some(version), false) => match Version::parse(&version.value) {
                Ok(written) => {
                    let identity = format!(
                        "{} {} {}",
                        need.origin.as_ref().map_or(ITATY.to_owned(), |o| match &o.value {
                            Origin::Source(s) => s.clone(),
                            _ => String::new(),
                        }),
                        need.name.value,
                        written.version.line()
                    );
                    if !identities.insert(identity) {
                        errors.push(error(
                            code,
                            need.range.clone(),
                            format!(
                                "`{}` is needed twice on the major line {}",
                                need.name.value,
                                written.version.line()
                            ),
                        ));
                    }
                }
                Err(message) => errors.push(error(code, version.range.clone(), message)),
            },
            (None, true) => {}
        }
        check_origin(&need.name, need.origin.as_ref(), sources, code, errors);
        check_ceiling(&need.may, code, errors);
        for avoid in &need.avoid {
            if let Err(message) = Version::parse_full(&avoid.value) {
                errors.push(error(code, avoid.range.clone(), message));
            }
        }
        if let Some(range) = &need.accept_name
            && !matches!(
                need.origin.as_ref().map(|o| &o.value),
                Some(Origin::Path(_))
            )
        {
            errors.push(error(
                code,
                range.clone(),
                "`accept name` is written only under a path need",
            ));
        }
    }
}

fn check_replace(
    entries: &[ita::Replace],
    sources: &BTreeMap<&str, &ita::Source>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for entry in entries {
        if let Err(message) = check_package_name(&entry.name.value) {
            errors.push(error(code, entry.name.range.clone(), message));
        }
        if let Err(message) = Version::parse_full(&entry.version.value) {
            errors.push(error(code, entry.version.range.clone(), message));
        }
        if !seen.insert((entry.name.value.to_string(), entry.version.value.clone())) {
            errors.push(error(
                code,
                entry.name.range.clone(),
                format!(
                    "`{} {}` is replaced twice",
                    entry.name.value, entry.version.value
                ),
            ));
        }
        if let Origin::Path(dir) = &entry.origin.value
            && !is_relative_dir(dir)
        {
            errors.push(error(
                code,
                entry.origin.range.clone(),
                "a replacement's directory is relative to the package, with `/` separators",
            ));
        }
        if let Origin::Source(source) = &entry.origin.value
            && source != ITATY
            && !sources.contains_key(source.as_str())
        {
            errors.push(error(
                code,
                entry.origin.range.clone(),
                format!("`from {source}` names no source"),
            ));
        }
    }
}

fn check_trust(
    read: &ita::Manifest,
    sources: &BTreeMap<&str, &ita::Source>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) {
    for line in &read.trust {
        if let Err(message) = check_package_name(&line.name.value) {
            errors.push(error(code, line.name.range.clone(), message));
            continue;
        }
        if let Some(major) = &line.major
            && Line::parse(&major.value).is_none()
        {
            errors.push(error(
                code,
                major.range.clone(),
                format!(
                    "`{}` is not a major line; write `2` for the line 2, or `0.3` for the line 0.3",
                    major.value
                ),
            ));
        }
        if let Some(from) = &line.from
            && from.value != ITATY
            && !sources.contains_key(from.value.as_str())
        {
            errors.push(error(
                code,
                from.range.clone(),
                format!("`from {}` names no source", from.value),
            ));
        }
        // The major and the source may be left out only for a package the root needs
        // directly from a registry (rule 10).
        if line.major.is_none() || line.from.is_none() {
            let direct = read.needs.iter().any(|need| {
                need.name.value == line.name.value
                    && !matches!(
                        need.origin.as_ref().map(|o| &o.value),
                        Some(Origin::Path(_) | Origin::Workspace)
                    )
            });
            if !direct {
                errors.push(error(
                    code,
                    line.name.range.clone(),
                    format!(
                        "`{}` is not needed directly from a registry, so its trust line names its major line and source: `{} <major> from <source> modules ...`",
                        line.name.value, line.name.value
                    ),
                ));
            }
        }
        for module in &line.modules {
            if !is_module_path(&module.value) {
                errors.push(error(
                    code,
                    module.range.clone(),
                    format!("`{}` is not a module path", module.value),
                ));
            }
        }
        check_ceiling(&line.may, code, errors);
    }
}

fn labels(read: &[At<String>], manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    let mut first: BTreeSet<&str> = BTreeSet::new();
    for label in read {
        if !is_upper_camel(&label.value) {
            errors.push(error(
                Code::U1005,
                label.range.clone(),
                format!(
                    "label category `{}` must be UpperCamelCase, as in `{}`",
                    label.value,
                    upper_camel(&label.value)
                ),
            ));
        }
        if !first.insert(label.value.as_str()) {
            errors.push(error(
                Code::U1002,
                label.range.clone(),
                format!("label category `{}` is declared twice", label.value),
            ));
        }
        manifest.categories.push(located(label));
    }
}

// ---------------------------------------------------------------------------------------------
// `ita.lock`
// ---------------------------------------------------------------------------------------------

/// `ita.lock`, as read: the solver pin, and the registries and packages as written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItaLock {
    /// The solver pin, or the default when the section is invalid.
    pub solver: Lock,
    /// The file as the grammar reads it.
    pub read: ita::Lock,
}

/// Reads `ita.lock` (ADR 0122 rules 16 and 18): every problem is U7009, a solver of another
/// version excepted, which is U5003 where the solver runs. O(n), and O(p log p) for the order
/// of its p packages.
pub fn parse_ita_lock(text: &str) -> (ItaLock, Vec<ProjectError>) {
    let code = Code::U7009;
    let (read, syntax) = ita::parse_lock(text);
    let mut errors: Vec<ProjectError> = syntax
        .into_iter()
        .map(|e| error(code, e.range, e.message))
        .collect();
    let mut solver = Lock::default();
    if let Some(section) = &read.solver {
        for (key, value, expected) in [
            ("encoding", &section.encoding, "una-smt-1"),
            ("name", &section.name, "z3"),
            ("logic", &section.logic, "QF_UFLIA"),
        ] {
            if value.value != expected {
                errors.push(error(
                    code,
                    value.range.clone(),
                    format!("`{key}` must be \"{expected}\", not \"{}\"", value.value),
                ));
            }
        }
        if section.version.value.trim().is_empty() {
            errors.push(error(
                code,
                section.version.range.clone(),
                "the solver's `version` must not be empty",
            ));
        } else {
            solver.solver_version = section.version.value.clone();
        }
        if section.steps.value == 0 {
            errors.push(error(
                code,
                section.steps.range.clone(),
                "`steps` must be 1 or more",
            ));
        } else {
            solver.step_budget = section.steps.value;
        }
        solver.random_seed = section.seed.value;
    }
    let mut registries = BTreeSet::new();
    for registry in &read.registries {
        if !registries.insert(registry.source.value.as_str()) {
            errors.push(error(
                code,
                registry.source.range.clone(),
                format!("the registry `{}` is locked twice", registry.source.value),
            ));
        }
        if RESERVED_SOURCES[1..].contains(&registry.source.value.as_str()) {
            errors.push(error(
                code,
                registry.source.range.clone(),
                format!("`{}` is not a registry", registry.source.value),
            ));
        }
        for digest in [&registry.root, &registry.checkpoint_hash] {
            if !is_digest(&digest.value) {
                errors.push(error(
                    code,
                    digest.range.clone(),
                    "a digest is `sha256:` and 64 lowercase hexadecimal digits",
                ));
            }
        }
    }
    let mut previous: Option<(String, Version)> = None;
    for package in &read.packages {
        if let Err(message) = check_package_name(&package.name.value) {
            errors.push(error(code, package.name.range.clone(), message));
        }
        let version = match Version::parse_full(&package.version.value) {
            Ok(version) => version,
            Err(message) => {
                errors.push(error(code, package.version.range.clone(), message));
                continue;
            }
        };
        let key = (package.name.value.to_string(), version);
        if let Some(previous) = &previous
            && *previous >= key
        {
            errors.push(error(
                code,
                package.range.clone(),
                "the packages of ita.lock are sorted by name, then version, each once; run `una ita lock`",
            ));
        }
        previous = Some(key);
        for digest in [&package.digest, &package.consented].into_iter().flatten() {
            if !is_digest(&digest.value) {
                errors.push(error(
                    code,
                    digest.range.clone(),
                    "a digest is `sha256:` and 64 lowercase hexadecimal digits",
                ));
            }
        }
        if let Some(At {
            value: LockOrigin::Source(source),
            range,
        }) = &package.from
            && source != ITATY
            && !registries.contains(source.as_str())
        {
            errors.push(error(
                code,
                range.clone(),
                format!("`from {source}` names no registry of the lock"),
            ));
        }
        let mut needs: Vec<String> = Vec::new();
        for need in &package.needs {
            if let Err(message) = Version::parse_full(&need.version.value) {
                errors.push(error(code, need.version.range.clone(), message));
            }
            let name = need.name.value.to_string();
            if needs.last().is_some_and(|last| *last >= name) {
                errors.push(error(
                    code,
                    need.name.range.clone(),
                    "the `needs` lines of a package are sorted by name, each once",
                ));
            }
            needs.push(name);
        }
    }
    (ItaLock { solver, read }, errors)
}

/// The canonical text of `ita.lock` for a solver pin and no package: what `una ita lock`
/// writes for a package with no dependency (ADR 0122 rules 14 to 16).
pub fn write_lock(solver: &Lock) -> String {
    format!(
        "-- Written by `una ita`; do not edit.\nlock {}\n\nsolver\n  encoding \"{}\"\n  name \"{}\"\n  version \"{}\"\n  logic \"{}\"\n  steps {}\n  seed {}\n",
        ita::LOCK_FORMAT,
        solver.encoding_version,
        solver.solver,
        solver.solver_version,
        solver.logic,
        solver.step_budget,
        solver.random_seed
    )
}

/// The canonical text of the lock's `solver` section, the part of the lock that the proof
/// cache's key covers (ADR 0122 rule 48).
pub fn solver_text(solver: &Lock) -> String {
    format!(
        "solver\n  encoding \"{}\"\n  name \"{}\"\n  version \"{}\"\n  logic \"{}\"\n  steps {}\n  seed {}\n",
        solver.encoding_version,
        solver.solver,
        solver.solver_version,
        solver.logic,
        solver.step_budget,
        solver.random_seed
    )
}

/// The consistency of `ita.lock` with `ita.una` (ADR 0122 rule 17), as far as one package's
/// files can tell: every need of the root, and every `needs` line of a locked package, is met
/// by a locked package of its name on its major line at a version at least the one required;
/// each package is locked once on its major line; and every locked package is reachable from
/// the root. Each problem is U7009, at the need or at the package. O((n + p + r) log p) for n
/// needs, p packages and r `needs` lines, by a breadth-first walk from the root.
pub fn check_lock_consistency(manifest: &ita::Manifest, lock: &ItaLock) -> Consistency {
    let code = Code::U7009;
    let mut errors = Vec::new();
    let mut at_needs = Vec::new();
    // Each locked package by its name and major line, with its index.
    let mut locked: BTreeMap<(String, Line), (Version, usize)> = BTreeMap::new();
    for (index, package) in lock.read.packages.iter().enumerate() {
        let Ok(version) = Version::parse_full(&package.version.value) else {
            continue;
        };
        let key = (package.name.value.to_string(), version.line());
        if locked.insert(key, (version, index)).is_some() {
            errors.push(error(
                code,
                package.range.clone(),
                format!(
                    "`{}` is locked twice on one major line; run `una ita lock`",
                    package.name.value
                ),
            ));
        }
    }
    let mut reached = vec![false; lock.read.packages.len()];
    let mut work: Vec<usize> = Vec::new();
    // The root's needs, each met by a locked package.
    for need in &manifest.needs {
        let Some(version) = &need.version else {
            continue;
        };
        let Ok(written) = Version::parse(&version.value) else {
            continue;
        };
        let key = (need.name.value.to_string(), written.version.line());
        match locked.get(&key) {
            Some((found, index)) if *found >= written.version => {
                if let Some(seen) = reached.get_mut(*index)
                    && !*seen
                {
                    *seen = true;
                    work.push(*index);
                }
            }
            Some((found, _)) => at_needs.push(error(
                code,
                need.range.clone(),
                format!(
                    "ita.lock holds `{} {found}`, below the {} this need asks for; run `una ita lock`",
                    need.name.value, version.value
                ),
            )),
            None => at_needs.push(error(
                code,
                need.range.clone(),
                format!(
                    "ita.lock holds no version of `{}` for this need; run `una ita lock`",
                    need.name.value
                ),
            )),
        }
    }
    // The locked packages' own needs, walked from the root's.
    while let Some(index) = work.pop() {
        let Some(package) = lock.read.packages.get(index) else {
            continue;
        };
        for need in &package.needs {
            let Ok(version) = Version::parse_full(&need.version.value) else {
                continue;
            };
            let key = (need.name.value.to_string(), version.line());
            match locked.get(&key) {
                Some((found, next)) if *found >= version => {
                    if let Some(seen) = reached.get_mut(*next)
                        && !*seen
                    {
                        *seen = true;
                        work.push(*next);
                    }
                }
                _ => errors.push(error(
                    code,
                    need.name.range.clone(),
                    format!(
                        "`{}` needs `{} {}`, which ita.lock does not meet; run `una ita lock`",
                        package.name.value, need.name.value, need.version.value
                    ),
                )),
            }
        }
    }
    for (package, reached) in lock.read.packages.iter().zip(&reached) {
        if !reached {
            errors.push(error(
                code,
                package.range.clone(),
                format!(
                    "`{} {}` is locked but no need reaches it; run `una ita lock`",
                    package.name.value, package.version.value
                ),
            ));
        }
    }
    Consistency {
        in_manifest: at_needs,
        in_lock: errors,
    }
}

/// The problems of [`check_lock_consistency`], by the file they are located in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Consistency {
    /// At the needs of `ita.una`.
    pub in_manifest: Vec<ProjectError>,
    /// At the packages of `ita.lock`.
    pub in_lock: Vec<ProjectError>,
}

// ---------------------------------------------------------------------------------------------
// `itaty.una`
// ---------------------------------------------------------------------------------------------

/// Reads `itaty.una` (ADR 0122 rule 19): every problem is U7008. A member is a relative
/// directory without `..`, listed once.
pub fn parse_workspace(text: &str) -> (ita::Workspace, Vec<ProjectError>) {
    let code = Code::U7008;
    let (read, syntax) = ita::parse_workspace(text);
    let mut errors: Vec<ProjectError> = syntax
        .into_iter()
        .map(|e| error(code, e.range, e.message))
        .collect();
    let mut seen = BTreeSet::new();
    for member in &read.members {
        let dir = member.value.trim_end_matches('/');
        let valid = is_relative_dir(dir)
            && dir
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if !valid {
            errors.push(error(
                code,
                member.range.clone(),
                "a member is a directory relative to the workspace, without `.` or `..`",
            ));
        } else if !seen.insert(dir) {
            errors.push(error(
                code,
                member.range.clone(),
                format!("the member `{dir}` is listed twice"),
            ));
        }
    }
    let sources = check_sources(&read.sources, code, &mut errors);
    check_replace(&read.replace, &sources, code, &mut errors);
    for line in &read.trust {
        if line.major.is_none() || line.from.is_none() {
            errors.push(error(
                code,
                line.name.range.clone(),
                "a trust line of itaty.una names the package's major line and source",
            ));
        }
    }
    (read, errors)
}
