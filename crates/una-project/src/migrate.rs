//! `una migrate` (ADR 0122 rules 63 and 64): the text of `ita.una` and `ita.lock` for a package
//! of editions `0.1` to `0.4` described by `una.toml` and `verification.lock`.
//!
//! Pure: it reads texts and returns texts, and the command writes them. O(n) in the files'
//! size, and O(c log c) for the c comments it carries.

use std::collections::{BTreeMap, BTreeSet};

use crate::ita::{check_package_name, write_lock};
use crate::lock::{Lock, parse_lock};
use crate::manifest::{Edition, Manifest, parse_manifest};
use crate::version::Version;
use una_syntax::ita::PkgName;

/// The result of a migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    /// The text of `ita.una`.
    pub manifest: String,
    /// The text of `ita.lock`.
    pub lock: String,
    /// The package's name in `una.toml`, and in `ita.una` when it changed.
    pub name: (String, String),
    /// The edition `una.toml` named.
    pub edition: Edition,
    /// The comments of `una.toml` that precede no key or table, as written, by line: listed in
    /// the command's output, since `ita.una` has no place for them.
    pub comments_left: Vec<(usize, String)>,
    /// Whether `verification.lock` was missing, so the default solver section was written.
    pub default_lock: bool,
}

/// Why a package cannot be migrated: one sentence each.
pub type Refusal = Vec<String>;

/// Migrates `una.toml` (`toml`) and `verification.lock` (`lock`, when the package has one).
/// `name` replaces the package's name when given. Refused when either file is invalid, when the
/// name is not a package name after `-` becomes `_`, or when the version is not a full one.
pub fn migrate(toml: &str, lock: Option<&str>, name: Option<&str>) -> Result<Migration, Refusal> {
    let mut refusal = Vec::new();
    let (manifest, errors) = parse_manifest(toml);
    // Edition 0.4 in `una.toml` is the one invalid setting migration exists for.
    for error in errors.iter().filter(|e| !e.message.contains("run `una migrate`")) {
        refusal.push(format!("una.toml: {} ({})", error.message, error.code));
    }
    let solver = match lock {
        Some(text) => {
            let (lock, errors) = parse_lock(text);
            for error in &errors {
                refusal.push(format!("verification.lock: {} ({})", error.message, error.code));
            }
            lock
        }
        None => Lock::default(),
    };
    let written = manifest.name.as_ref().map(|n| n.value.clone()).unwrap_or_default();
    let target = match name {
        Some(name) => name.to_owned(),
        None => written.replace('-', "_"),
    };
    let parsed = match target.split_once('/') {
        Some((owner, name)) => PkgName {
            owner: Some(owner.to_owned()),
            name: name.to_owned(),
        },
        None => PkgName {
            owner: None,
            name: target.clone(),
        },
    };
    if let Err(message) = check_package_name(&parsed) {
        refusal.push(format!("{message}; give the package a name with `--name`"));
    }
    let version = manifest.version.as_ref().map(|v| v.value.clone()).unwrap_or_default();
    if let Err(message) = Version::parse_full(&version) {
        refusal.push(format!("una.toml: [package] version {message}"));
    }
    if !refusal.is_empty() {
        return Err(refusal);
    }
    let comments = Comments::read(toml);
    let mut out = Writer::default();
    out.comments(&comments.before_table("package"));
    out.comments(&comments.before_key("package", "name"));
    out.line(0, &format!("ita {target}"));
    out.comments_at(1, &comments.before_key("package", "version"));
    out.line(1, &format!("version {version}"));
    out.comments_at(1, &comments.before_key("package", "edition"));
    out.line(1, "edition 0.4");
    sections(&manifest, &comments, &mut out);
    Ok(Migration {
        manifest: out.text,
        lock: write_lock(&solver),
        name: (written, target),
        edition: manifest.edition,
        comments_left: comments.left(&out.used),
        default_lock: lock.is_none(),
    })
}

fn sections(manifest: &Manifest, comments: &Comments, out: &mut Writer) {
    if !manifest.categories.is_empty() {
        out.section(comments, "labels", "labels");
        for category in &manifest.categories {
            out.line(1, &category.value);
        }
    }
    if !manifest.trusted.is_empty() {
        out.section(comments, "trusted", "trusted");
        for module in &manifest.trusted {
            out.line(1, &module.value);
        }
    }
    if manifest.checked_per_module > 0 {
        out.section(comments, "escape_budget", "escapes");
        out.line(1, &format!("checked per module {}", manifest.checked_per_module));
    }
    if !manifest.boundaries.is_empty() {
        out.section(comments, "boundaries", "boundaries");
        for boundary in &manifest.boundaries {
            out.comments_at(1, &comments.before_key("boundaries", &boundary.module.value));
            out.line(1, &boundary.module.value);
            for ceiling in &boundary.ceiling {
                out.line(2, &format!("may {}[{}]", ceiling.capability, ceiling.ops.join(", ")));
            }
        }
    }
    if !manifest.contracts.is_empty() {
        out.section(comments, "contracts", "contracts");
        for contract in &manifest.contracts {
            out.comments_at(1, &comments.before_key("contracts", &contract.function.value));
            out.line(1, &contract.function.value);
            for clause in &contract.clauses {
                out.line(2, clause.value.trim());
            }
        }
    }
    if manifest.entry.is_some() || !manifest.grants.is_empty() {
        out.section(comments, "run", "run");
        if let Some(entry) = &manifest.entry {
            out.comments_at(1, &comments.before_key("run", "entry"));
            out.line(1, &format!("entry {}", entry.value));
        }
        if !manifest.grants.is_empty() {
            out.comments_at(1, &comments.before_key("run", "grants"));
            out.line(1, "grants");
            for grant in &manifest.grants {
                out.line(2, &grant_line(&grant.value));
            }
        }
    }
}

/// The `ita.una` line of a grant spec of `spec/13 §5`: `files:data/:read` becomes
/// `files "data/" read`. The spec was checked by `una.toml`'s reader.
pub fn grant_line(spec: &str) -> String {
    if let Some(file) = spec.strip_prefix("keys:") {
        return format!("keys {}", text(file));
    }
    if let Some(rest) = spec.strip_prefix("files:")
        && let Some((path, modes)) = rest.rsplit_once(':')
    {
        return format!("files {} {}", text(path), modes.replace(',', ", "));
    }
    if let Some(rest) = spec.strip_prefix("http:") {
        // The methods, when written, are the last `:` part, of letters and commas.
        if let Some((origin, methods)) = rest.rsplit_once(':')
            && !methods.is_empty()
            && methods.bytes().all(|b| b.is_ascii_alphabetic() || b == b',')
            && origin.contains("//")
        {
            return format!("http {} {}", text(origin), methods.replace(',', ", "));
        }
        return format!("http {}", text(rest));
    }
    spec.to_owned()
}

/// A text literal of `ita.una` for `value`: quotes, backslashes and braces escaped, and every
/// control or bidirectional character written as `\u{...}` (ADR 0122 rule 4).
pub fn text(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
            {
                out.push_str(&format!("\\u{{{:X}}}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text being written, with the comments it carried.
#[derive(Default)]
struct Writer {
    text: String,
    /// The lines of `una.toml` whose comments were carried.
    used: BTreeSet<usize>,
}

impl Writer {
    fn line(&mut self, depth: usize, line: &str) {
        self.text.push_str(&"  ".repeat(depth));
        self.text.push_str(line);
        self.text.push('\n');
    }

    fn comments(&mut self, comments: &[(usize, String)]) {
        self.comments_at(0, comments);
    }

    fn comments_at(&mut self, depth: usize, comments: &[(usize, String)]) {
        for (line, comment) in comments {
            self.used.insert(*line);
            self.line(depth, &format!("--{}", comment.strip_prefix('#').unwrap_or(comment)));
        }
    }

    /// A blank line, the comments before the table, and the section's header.
    fn section(&mut self, comments: &Comments, table: &str, name: &str) {
        self.text.push('\n');
        self.comments(&comments.before_table(table));
        self.line(0, name);
    }
}

/// The comments of `una.toml`, by what follows them.
struct Comments {
    /// Each comment line: its index and text from `#`.
    all: Vec<(usize, String)>,
    /// The comment lines just before a table header, by table.
    tables: BTreeMap<String, Vec<(usize, String)>>,
    /// The comment lines just before a key, by table and key.
    keys: BTreeMap<(String, String), Vec<(usize, String)>>,
}

impl Comments {
    /// Reads the comments line by line: a run of comment lines belongs to the table header or
    /// key line that follows it directly. A blank line ends a run. O(n).
    fn read(toml: &str) -> Comments {
        let mut comments = Comments {
            all: Vec::new(),
            tables: BTreeMap::new(),
            keys: BTreeMap::new(),
        };
        let mut run: Vec<(usize, String)> = Vec::new();
        let mut table = String::new();
        for (index, line) in toml.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(at) = trimmed.find('#').filter(|_| trimmed.starts_with('#')) {
                let comment = trimmed.get(at..).unwrap_or("").to_owned();
                comments.all.push((index, comment.clone()));
                run.push((index, comment));
                continue;
            }
            if trimmed.is_empty() {
                run.clear();
                continue;
            }
            if let Some(comment) = trailing_comment(trimmed) {
                comments.all.push((index, comment));
            }
            if let Some(name) = trimmed
                .strip_prefix('[')
                .and_then(|rest| rest.split(']').next())
            {
                table = name.trim().to_owned();
                comments
                    .tables
                    .insert(table.clone(), std::mem::take(&mut run));
            } else if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim().trim_matches('"').to_owned();
                comments
                    .keys
                    .insert((table.clone(), key), std::mem::take(&mut run));
            } else {
                run.clear();
            }
        }
        comments
    }

    fn before_table(&self, table: &str) -> Vec<(usize, String)> {
        self.tables.get(table).cloned().unwrap_or_default()
    }

    fn before_key(&self, table: &str, key: &str) -> Vec<(usize, String)> {
        self.keys
            .get(&(table.to_owned(), key.to_owned()))
            .cloned()
            .unwrap_or_default()
    }

    /// The comments that were not carried, in line order.
    fn left(&self, used: &BTreeSet<usize>) -> Vec<(usize, String)> {
        self.all
            .iter()
            .filter(|(line, _)| !used.contains(line))
            .map(|(line, text)| (line + 1, text.clone()))
            .collect()
    }
}

/// The comment after a value on the same line, outside any string: `key = "a#b" # note`.
fn trailing_comment(line: &str) -> Option<String> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, c) in line.char_indices() {
        match (quote, c) {
            (Some('"'), '\\') if !escaped => {
                escaped = true;
                continue;
            }
            (Some(q), c) if c == q && !escaped => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') => return line.get(index..).map(str::to_owned),
            _ => {}
        }
        escaped = false;
    }
    None
}
