//! `una.toml` (`spec/13 §7`, ADR 0021 rule 2).

use std::collections::BTreeMap;

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
}

impl Edition {
    /// Every edition, oldest first.
    pub const ALL: [Edition; 3] = [Edition::V0_1, Edition::V0_2, Edition::V0_3];

    /// The edition's name, as `una.toml` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Edition::V0_1 => "0.1",
            Edition::V0_2 => "0.2",
            Edition::V0_3 => "0.3",
        }
    }

    /// Every edition's name, quoted, as a message lists them: `"0.1", "0.2" and "0.3"`.
    fn listed() -> String {
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
    fn parse(text: &str) -> Option<Edition> {
        match text {
            "0.1" => Some(Edition::V0_1),
            "0.2" => Some(Edition::V0_2),
            "0.3" => Some(Edition::V0_3),
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
    // `[contracts]` is a table of edition 0.3 only (ADR 0092 rule 8, as amended): earlier
    // editions report it as the unknown table it always was, so they do not change.
    let locks = declared_edition(document.get_ref()) >= Edition::V0_3;
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
            "contracts" if locks => contracts(&table, &mut manifest, &mut errors),
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
fn is_upper_camel(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// A suggestion in UpperCamelCase for `name`.
fn upper_camel(name: &str) -> String {
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
