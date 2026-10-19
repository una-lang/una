//! `una.toml` (`spec/13 §7`, ADR 0021 rule 2).

use std::collections::{BTreeMap, HashMap};

use una_diag::Code;

use crate::document::{Table, integer, parse, string, strings};
use crate::{Located, ProjectError};

/// The settings of a package's `una.toml`, or their defaults (ADR 0021 rule 2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// `[package] name`.
    pub name: Option<Located>,
    /// `[package] version`.
    pub version: Option<Located>,
    /// `[labels] categories`: the label categories of the package (`spec/07 §2`).
    pub categories: Vec<Located>,
    /// `[trusted] modules`: the module paths declared trusted (`spec/07 §6`).
    pub trusted: Vec<Located>,
    /// `[escape_budget] checked_per_module` (`spec/06 §6`).
    pub checked_per_module: u64,
    /// `[run] entry`: the module whose `main` `una run` runs.
    pub entry: Option<Located>,
    /// `[run] grants`: the capabilities `una run` grants (`spec/13 §5`).
    pub grants: Vec<Located>,
    /// `[package] edition`: `0.1`, the default, or `0.2` (`spec/13 §7`, ADR 0056).
    pub edition: Edition,
    /// `[contracts]`, edition `0.3`: the locked `uses` clauses of public functions (ADR 0092
    /// rule 8, as amended).
    pub contracts: Vec<Contract>,
    /// `[boundaries]`: the boundary modules and their ceilings, in edition `0.3` (ADR 0093).
    pub boundaries: Vec<Boundary>,
    /// The file the settings were read from: `una.toml`, or `ita.una` from edition `0.4`.
    pub format: Format,
    /// The path dependencies mounted into the program (ADR 0122 rule 47): each one's local
    /// name, under which its modules are named, and where its files are. Filled when the
    /// dependencies are loaded, empty otherwise.
    pub mounts: Vec<Mount>,
    /// For `ita.una`, the file as read, with what only it writes: the head's `about`,
    /// `license`, `home` and `publish to`, and the sections `sources`, `needs`, `replace` and
    /// `trust` (ADR 0122).
    pub ita: Option<Box<una_syntax::ita::Manifest>>,
}

/// A dependency mounted into the program: its modules `m` are the program's modules
/// `<local>.m` (ADR 0122 rule 47).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// The local name, the first segment of its modules' paths in the program.
    pub local: String,
    /// The prefix of its module files' paths in the source database, ending with `src/`, such
    /// as `../util/src/`.
    pub src: String,
}

impl Manifest {
    /// The mount whose module files include the file at `path`, if any: the file is a module
    /// of that dependency. O(mounts).
    pub fn mount_of(&self, path: &str) -> Option<&Mount> {
        self.mounts
            .iter()
            .find(|mount| path.starts_with(&mount.src))
    }
}

/// The file a package's settings are read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// `una.toml`, editions `0.1` to `0.3` (ADR 0021).
    #[default]
    Toml,
    /// `ita.una`, from edition `0.4` (ADR 0122).
    Ita,
}

impl Format {
    /// The code of an invalid setting: U7002 in `una.toml`, U7008 in `ita.una` (ADR 0122
    /// rule 13).
    pub fn code(self) -> Code {
        match self {
            Format::Toml => Code::U7002,
            Format::Ita => Code::U7008,
        }
    }

    /// How a message names a table of `una.toml` or its section of `ita.una`:
    /// `[boundaries]` or `` `boundaries` ``; `[run] entry` or `` `run entry` ``.
    pub fn section(self, toml: &str) -> String {
        match self {
            Format::Toml => toml.to_owned(),
            Format::Ita => {
                let plain: String = toml.chars().filter(|c| !matches!(c, '[' | ']')).collect();
                let plain = match plain.as_str() {
                    "escape_budget checked_per_module" => "escapes checked per module".to_owned(),
                    "labels categories" => "labels".to_owned(),
                    "trusted modules" => "trusted".to_owned(),
                    _ => plain,
                };
                format!("`{plain}`")
            }
        }
    }

    /// The file's name.
    pub fn file(self) -> &'static str {
        match self {
            Format::Toml => "una.toml",
            Format::Ita => "ita.una",
        }
    }
}

/// A locked function of `[contracts]` (ADR 0092 rule 8, as amended): its qualified name and
/// its locked `uses` clauses, each as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    /// The function's qualified name, located at its key.
    pub function: Located,
    /// The `uses` clauses. A clause's `range` covers its string literal, quotes included.
    pub clauses: Vec<Located>,
}

/// One entry of `[boundaries]` (ADR 0093 rules 1 and 9): a module path, which covers the module
/// and every module below it, and its ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boundary {
    /// The key: the module path.
    pub module: Located,
    /// The ceiling: the most authority a call into the boundary may grant.
    pub ceiling: Vec<Ceiling>,
    /// Whether the boundary is a dependency's package extent (ADR 0122 rule 51): its key is a
    /// local name, the prefix of the dependency's modules, not a module, and its ceiling, the
    /// root's `may` lines, names host capabilities.
    pub package: bool,
}

/// One `C[ops]` of a ceiling: a capability's name and the operations it allows, names only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ceiling {
    /// The capability's name, as the boundary module would write it.
    pub capability: String,
    /// The operations, in the order written.
    pub ops: Vec<String>,
    /// The whole string.
    pub at: Located,
}

impl Manifest {
    /// The boundaries by key, to find the one whose extent holds a module (ADR 0093 rule 9).
    /// Built once in O(b); each query is then O(depth) lookups.
    pub fn extents(&self) -> Extents<'_> {
        Extents {
            by_key: self
                .boundaries
                .iter()
                .enumerate()
                .map(|(index, boundary)| (boundary.module.value.as_str(), index))
                .collect(),
        }
    }
}

/// The boundaries of a manifest by key (ADR 0093 rule 9), from [`Manifest::extents`].
#[derive(Clone, Debug)]
pub struct Extents<'a> {
    /// Each key, with its index in [`Manifest::boundaries`]. Only looked up, never iterated.
    by_key: HashMap<&'a str, usize>,
}

impl Extents<'_> {
    /// The index in [`Manifest::boundaries`] of the boundary whose extent holds `module`: the
    /// longest key that is `module` or a prefix of it by whole path segments. Nested keys are
    /// U7002, so a valid manifest has at most one. One lookup per segment, O(depth).
    pub fn of(&self, module: &str) -> Option<usize> {
        module_prefixes(module).find_map(|prefix| self.by_key.get(prefix).copied())
    }
}

/// `module` and each module path it lies below, longest first: `a.b.c`, `a.b`, `a`.
fn module_prefixes(module: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(module), |path| {
        path.rsplit_once('.').map(|(head, _)| head)
    })
}

impl Boundary {
    /// Whether the boundary's extent holds `module`: the key itself, or a module below it by
    /// whole path segments (ADR 0093 rule 9).
    pub fn covers(&self, module: &str) -> bool {
        covers(&self.module.value, module)
    }
}

/// Whether `key` is `module` or a prefix of it by whole path segments.
fn covers(key: &str, module: &str) -> bool {
    module
        .strip_prefix(key)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// The language edition of a package (`spec/13 §7`, ADR 0056). A single file, and a package
/// whose `una.toml` is invalid, has the default, edition `0.1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Edition {
    /// Edition `0.1`: the language of v0.1.
    #[default]
    V0_1,
    /// Edition `0.2`: adds the constructs of `spec/17` and `spec/18`.
    V0_2,
    /// Edition `0.3`, a preview until ADRs 0092 to 0094 land: every construct of edition
    /// `0.2`, and the rules that target edition `0.3` (`spec/13 §7`).
    V0_3,
    /// Edition `0.4`, a preview until the rules that target it land: every construct of
    /// edition `0.3`, and those of ADR 0116 (`spec/26`) as they land.
    V0_4,
}

impl Edition {
    /// Every edition, oldest first.
    pub const ALL: [Edition; 4] = [Edition::V0_1, Edition::V0_2, Edition::V0_3, Edition::V0_4];

    /// The edition's name, as `una.toml` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Edition::V0_1 => "0.1",
            Edition::V0_2 => "0.2",
            Edition::V0_3 => "0.3",
            Edition::V0_4 => "0.4",
        }
    }

    /// Every edition's name, quoted, as a message lists them: `"0.1", "0.2", "0.3" and "0.4"`.
    pub(crate) fn listed() -> String {
        let names: Vec<String> = Edition::ALL
            .iter()
            .map(|edition| format!("\"{}\"", edition.name()))
            .collect();
        match names.split_last() {
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        }
    }

    /// The edition named `text` in `una.toml`, if it is one.
    pub(crate) fn parse(text: &str) -> Option<Edition> {
        match text {
            "0.1" => Some(Edition::V0_1),
            "0.2" => Some(Edition::V0_2),
            "0.3" => Some(Edition::V0_3),
            "0.4" => Some(Edition::V0_4),
            _ => None,
        }
    }
}

/// Parses `una.toml`. the tables and keys of ADR 0021 rule 2 and no others;
/// every problem is U7002, except a category's naming convention (U1005) and a repeated
/// category (U1002).
pub fn parse_manifest(text: &str) -> (Manifest, Vec<ProjectError>) {
    let mut manifest = Manifest::default();
    let mut errors = Vec::new();
    let Some(document) = parse(text, Code::U7002, &mut errors) else {
        return (manifest, errors);
    };
    let mut package = false;
    // `[contracts]` and `[boundaries]` are tables of edition 0.3 only (ADR 0092 rule 8, as
    // amended, and ADR 0093): earlier editions report them as the unknown tables they always
    // were, so their output does not change.
    let edition_0_3 = declared_edition(document.get_ref()) >= Edition::V0_3;
    for (key, value) in document.get_ref().iter() {
        let Some(table) = Table::of(key, value, Code::U7002, &mut errors) else {
            continue;
        };
        match key.get_ref().as_ref() {
            "package" => {
                package = true;
                package_table(&table, &mut manifest, &mut errors);
            }
            "labels" => labels(&table, &mut manifest, &mut errors),
            "trusted" => trusted(&table, &mut manifest, &mut errors),
            "escape_budget" => escape_budget(&table, &mut manifest, &mut errors),
            "run" => run(&table, &mut manifest, &mut errors),
            "contracts" if edition_0_3 => contracts(&table, &mut manifest, &mut errors),
            "boundaries" if edition_0_3 => boundaries(&table, &mut manifest, &mut errors),
            other => errors.push(ProjectError::new(
                Code::U7002,
                key.span(),
                format!("unknown table [{other}] in una.toml"),
            )),
        }
    }
    if !package {
        errors.push(ProjectError::new(
            Code::U7002,
            0..0,
            "una.toml has no [package] table",
        ));
    }
    if !manifest.boundaries.is_empty() {
        boundary_conflicts(&manifest, &mut errors);
    }
    (manifest, errors)
}

/// The edition that `[package] edition` names, or the default when it names none.
fn declared_edition(document: &toml::de::DeTable<'_>) -> Edition {
    document
        .get("package")
        .and_then(|package| match package.get_ref() {
            toml::de::DeValue::Table(table) => table.get("edition"),
            _ => None,
        })
        .and_then(|edition| match edition.get_ref() {
            toml::de::DeValue::String(text) => Edition::parse(text),
            _ => None,
        })
        .unwrap_or_default()
}

/// `[contracts]`: each key a function's qualified name, each value an array of `uses` clauses
/// (ADR 0092 rule 8, as amended). The clauses are read by the checker, which reports them.
fn contracts(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    for (key, value) in table.entries.iter() {
        let name = key.get_ref().to_string();
        let clauses = strings(&name, value, Code::U7002, errors);
        manifest.contracts.push(Contract {
            function: Located {
                value: name,
                range: key.span(),
            },
            clauses,
        });
    }
}

fn package_table(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    let mut seen = Vec::new();
    for (key, value) in table.entries.iter() {
        let name = key.get_ref().as_ref();
        match name {
            "name" | "version" | "edition" => {
                seen.push(name.to_owned());
                let Some(located) = string(name, value, Code::U7002, errors) else {
                    continue;
                };
                if located.value.trim().is_empty() {
                    errors.push(ProjectError::new(
                        Code::U7002,
                        located.range,
                        format!("`{name}` must not be empty"),
                    ));
                } else if name == "edition" {
                    match Edition::parse(&located.value) {
                        // Edition 0.4 is described by `ita.una` (ADR 0122 Question 8.1): the
                        // package is still checked in edition 0.4, and `una migrate` moves it.
                        Some(Edition::V0_4) => {
                            manifest.edition = Edition::V0_4;
                            errors.push(ProjectError::new(
                                Code::U7002,
                                located.range,
                                "edition 0.4 is described by ita.una, not una.toml; run `una migrate`",
                            ));
                        }
                        Some(edition) => manifest.edition = edition,
                        None => errors.push(ProjectError::new(
                            Code::U7002,
                            located.range,
                            format!(
                                "edition `{}` is not supported; the editions are {}",
                                located.value,
                                Edition::listed()
                            ),
                        )),
                    }
                } else if name == "name" {
                    manifest.name = Some(located);
                } else if name == "version" {
                    manifest.version = Some(located);
                }
            }
            _ => errors.push(table.unknown(key, Code::U7002)),
        }
    }
    for required in ["name", "version", "edition"] {
        if !seen.iter().any(|name| name == required) {
            errors.push(table.missing(required, Code::U7002));
        }
    }
}

fn labels(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    for (key, value) in table.entries.iter() {
        if key.get_ref().as_ref() != "categories" {
            errors.push(table.unknown(key, Code::U7002));
            continue;
        }
        let mut first: BTreeMap<String, usize> = BTreeMap::new();
        for category in strings("categories", value, Code::U7002, errors) {
            if !is_upper_camel(&category.value) {
                errors.push(ProjectError::new(
                    Code::U1005,
                    category.range.clone(),
                    format!(
                        "label category `{}` must be UpperCamelCase, as in `{}`",
                        category.value,
                        upper_camel(&category.value)
                    ),
                ));
            }
            if first.contains_key(&category.value) {
                errors.push(ProjectError::new(
                    Code::U1002,
                    category.range.clone(),
                    format!("label category `{}` is declared twice", category.value),
                ));
            } else {
                first.insert(category.value.clone(), category.range.start);
            }
            manifest.categories.push(category);
        }
    }
}

fn trusted(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    for (key, value) in table.entries.iter() {
        if key.get_ref().as_ref() != "modules" {
            errors.push(table.unknown(key, Code::U7002));
            continue;
        }
        for module in strings("modules", value, Code::U7002, errors) {
            if is_module_path(&module.value) {
                manifest.trusted.push(module);
            } else {
                errors.push(ProjectError::new(
                    Code::U7002,
                    module.range,
                    format!("`{}` is not a module path", module.value),
                ));
            }
        }
    }
}

fn escape_budget(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    for (key, value) in table.entries.iter() {
        if key.get_ref().as_ref() != "checked_per_module" {
            errors.push(table.unknown(key, Code::U7002));
            continue;
        }
        if let Some(budget) = integer("checked_per_module", value, Code::U7002, errors) {
            match u64::try_from(budget) {
                Ok(budget) => manifest.checked_per_module = budget,
                Err(_) => errors.push(ProjectError::new(
                    Code::U7002,
                    value.span(),
                    "`checked_per_module` must be 0 or more",
                )),
            }
        }
    }
}

fn run(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    for (key, value) in table.entries.iter() {
        match key.get_ref().as_ref() {
            "entry" => {
                let Some(entry) = string("entry", value, Code::U7002, errors) else {
                    continue;
                };
                if is_module_path(&entry.value) {
                    manifest.entry = Some(entry);
                } else {
                    errors.push(ProjectError::new(
                        Code::U7002,
                        entry.range,
                        format!("`{}` is not a module path", entry.value),
                    ));
                }
            }
            "grants" => {
                for grant in strings("grants", value, Code::U7002, errors) {
                    if is_grant(&grant.value) {
                        manifest.grants.push(grant);
                    } else {
                        errors.push(ProjectError::new(
                            Code::U7002,
                            grant.range,
                            format!(
                                "`{}` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]",
                                grant.value
                            ),
                        ));
                    }
                }
            }
            _ => errors.push(table.unknown(key, Code::U7002)),
        }
    }
}

/// `[boundaries]`: each key a module path, each value an array of names-only `C[ops]`
/// strings (ADR 0093 rule 1).
fn boundaries(table: &Table<'_, '_>, manifest: &mut Manifest, errors: &mut Vec<ProjectError>) {
    for (key, value) in table.entries.iter() {
        let path = key.get_ref().as_ref();
        let module = Located {
            value: path.to_owned(),
            range: key.span(),
        };
        if !is_module_path(path) {
            errors.push(ProjectError::new(
                Code::U7002,
                module.range,
                format!("`{path}` in [boundaries] is not a module path"),
            ));
            continue;
        }
        let mut ceiling: Vec<Ceiling> = Vec::new();
        for entry in strings(path, value, Code::U7002, errors) {
            match parse_ceiling(&entry.value) {
                Some((capability, ops)) => {
                    if ceiling.iter().any(|c| c.capability == capability) {
                        errors.push(ProjectError::new(
                            Code::U7002,
                            entry.range,
                            format!("the ceiling of `{path}` names `{capability}` twice"),
                        ));
                        continue;
                    }
                    ceiling.push(Ceiling {
                        capability,
                        ops,
                        at: entry,
                    });
                }
                None => errors.push(ProjectError::new(
                    Code::U7002,
                    entry.range.clone(),
                    format!(
                        "`{}` in the ceiling of `{path}` is not a capability with its operations, such as `Files[read]`: names only, each operation once",
                        entry.value
                    ),
                )),
            }
        }
        manifest.boundaries.push(Boundary {
            module,
            ceiling,
            package: false,
        });
    }
}

/// `Name[op, op]`, names only, with at least one operation and none twice; anything else, a
/// condition or a label included, is `None`.
fn parse_ceiling(text: &str) -> Option<(String, Vec<String>)> {
    let (name, rest) = text.trim().split_once('[')?;
    let inside = rest.strip_suffix(']')?;
    if !is_upper_camel(name) {
        return None;
    }
    let mut ops: Vec<String> = Vec::new();
    for op in inside.split(',') {
        let op = op.trim();
        let mut chars = op.chars();
        let ident = chars.next().is_some_and(|first| first.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ident || ops.iter().any(|seen| seen == op) {
            return None;
        }
        ops.push(op.to_owned());
    }
    Some((name.to_owned(), ops))
}

/// U7002 for keys whose extents overlap, a boundary that is also trusted, and a `[run]` entry
/// inside a boundary (ADR 0093 rules 8 to 10). The keys are sorted, so a key and a key below it
/// are found in one pass: O(k log k).
pub(crate) fn boundary_conflicts(manifest: &Manifest, errors: &mut Vec<ProjectError>) {
    let code = manifest.format.code();
    let boundaries = manifest.format.section("[boundaries]");
    let trusted_section = manifest.format.section("[trusted] modules");
    let entry_section = manifest.format.section("[run] entry");
    let mut keys: Vec<&Boundary> = manifest.boundaries.iter().collect();
    keys.sort_by(|a, b| a.module.value.cmp(&b.module.value));
    let mut open: Vec<&Boundary> = Vec::new();
    for boundary in keys {
        while open
            .last()
            .is_some_and(|outer| !outer.covers(&boundary.module.value))
        {
            open.pop();
        }
        if let Some(outer) = open.last() {
            errors.push(ProjectError::new(
                code,
                boundary.module.range.clone(),
                format!(
                    "{boundaries} lists `{}` inside `{}`, which already covers it; a module has one ceiling",
                    boundary.module.value, outer.module.value
                ),
            ));
        }
        open.push(boundary);
    }
    let extents = manifest.extents();
    let boundary_of = |module: &str| {
        extents
            .of(module)
            .and_then(|index| manifest.boundaries.get(index))
    };
    for trusted in &manifest.trusted {
        if let Some(boundary) = boundary_of(&trusted.value) {
            errors.push(ProjectError::new(
                code,
                trusted.range.clone(),
                format!(
                    "`{}` is in {trusted_section} and inside the boundary `{}`; a boundary module never releases",
                    trusted.value, boundary.module.value
                ),
            ));
        }
    }
    if let Some(entry) = &manifest.entry
        && let Some(boundary) = boundary_of(&entry.value)
    {
        errors.push(ProjectError::new(
            code,
            entry.range.clone(),
            format!(
                "the {entry_section} `{}` is inside the boundary `{}`, whose grants would bypass its ceiling",
                entry.value, boundary.module.value
            ),
        ));
    }
}

/// Whether `text` is a module path: lower-case identifiers joined by dots (`spec/02 §1`).
pub fn is_module_path(text: &str) -> bool {
    !text.is_empty()
        && text.split('.').all(|segment| {
            let mut chars = segment.chars();
            chars.next().is_some_and(|first| first.is_ascii_lowercase())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// Whether `text` is a grant spec of `spec/13 §5`: `console`, `clock`, `env`, `keys:<keyfile>`,
/// `files:<path>:read`, `:write` or `:read,write`, or `http:<origin>[:<methods>]` (`spec/24 §2`).
pub fn is_grant(text: &str) -> bool {
    match text {
        "console" | "clock" | "env" => true,
        _ => {
            if text.starts_with("http:") {
                return una_core::http::Grant::parse(text).is_some();
            }
            if let Some(file) = text.strip_prefix("keys:") {
                return !file.is_empty();
            }
            let Some(rest) = text.strip_prefix("files:") else {
                return false;
            };
            match rest.rsplit_once(':') {
                Some((path, access)) => {
                    !path.is_empty() && matches!(access, "read" | "write" | "read,write")
                }
                None => false,
            }
        }
    }
}

/// Whether `name` is UpperCamelCase (`spec/01 §4`).
pub(crate) fn is_upper_camel(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// A suggestion in UpperCamelCase for `name`.
pub(crate) fn upper_camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
            upper = false;
        } else {
            upper = true;
        }
    }
    out
}
