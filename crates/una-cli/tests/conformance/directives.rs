//! Directive parsing and validation for conformance cases (`compiler/testing.md §2`, ADR 0013,
//! D2 to D4). A case whose directives are malformed fails.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use una_diag::Code;

use crate::discovery::CaseKind;

/// The directive names of `compiler/testing.md §2`.
const NAMES: [&str; 5] = ["test", "spec", "expect", "grant", "pending"];

/// A case with well-formed directives.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Case {
    /// The one-line description of `-- test:`.
    pub test: String,
    /// The rule under test, as written in `-- spec:`.
    pub spec: String,
    /// The expected diagnostics of `una check`.
    pub check: CheckExpectation,
    /// The lines `una run` prints, in order.
    pub run_output: Vec<String>,
    /// The expected outcome of `una test`.
    pub test_outcome: TestExpectation,
    /// The grants passed to `una run`.
    pub grants: Vec<String>,
    /// The milestone of `-- pending:`, if any.
    pub pending: Option<u32>,
}

/// The expectation on the diagnostics of `una check`.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum CheckExpectation {
    /// The case expects nothing of `una check` itself.
    #[default]
    Unspecified,
    /// `-- expect: ok`: no diagnostics at tier 1.
    Ok,
    /// `-- expect: tier0-ok`: no diagnostics at tier 0.
    Tier0Ok,
    /// `-- expect: error ...`: exactly this set of errors.
    Errors(BTreeSet<ExpectedError>),
}

/// The expectation on the outcome of `una test`.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum TestExpectation {
    /// The case expects nothing of `una test`.
    #[default]
    Unspecified,
    /// `-- expect: test-pass`.
    Pass,
    /// `-- expect: test-fail "<example>"`: exactly these examples fail.
    Fail(BTreeSet<String>),
}

/// One `-- expect: error <code> at <position>` line, or one reported error in the same form.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExpectedError {
    pub code: Code,
    /// The file, relative to the package root: the case file itself in a single-module case,
    /// which is its own package root (ADR 0015), or the written path in a multi-module case.
    pub file: String,
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for ExpectedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}:{}:{}",
            self.code, self.file, self.line, self.col
        )
    }
}

/// The `## N.` sections of each specification file, keyed by its number `NN`.
pub struct SpecIndex(BTreeMap<String, BTreeSet<u32>>);

impl SpecIndex {
    /// Reads the section headings of `docs/spec/NN-*.md`.
    pub fn load(dir: &Path) -> SpecIndex {
        let mut index = BTreeMap::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let Some((number, _)) = name.split_once('-') else {
                continue;
            };
            if !name.ends_with(".md") || !is_spec_number(number) {
                continue;
            }
            let text = std::fs::read_to_string(dir.join(&name)).unwrap();
            let sections = text
                .lines()
                .filter_map(|line| line.strip_prefix("## ")?.split_once(". ")?.0.parse().ok())
                .collect();
            let previous = index.insert(number.to_owned(), sections);
            assert!(
                previous.is_none(),
                "two specification files are numbered {number}"
            );
        }
        SpecIndex(index)
    }
}

/// One directive line of a file.
struct Directive {
    file: String,
    line: usize,
    name: String,
    value: String,
}

/// One parsed `-- expect:` value.
enum Expectation {
    Ok,
    Tier0Ok,
    TestPass,
    Error(ExpectedError),
    RunOutput(String),
    TestFail(String),
}

/// Parses and validates the directives of a case. `files` holds the case's files keyed by path
/// relative to the case: the case file itself for a single-module case, every file under the
/// case directory for a multi-module case.
pub fn parse_case(
    kind: CaseKind,
    files: &BTreeMap<String, Vec<u8>>,
    specs: &SpecIndex,
) -> Result<Case, Vec<String>> {
    let mut errors = Vec::new();
    let mut directives = Vec::new();
    let mut carrying = Vec::new();
    for (path, bytes) in files {
        let carries = match kind {
            CaseKind::Single => true,
            CaseKind::Directory => path.starts_with("src/") && path.ends_with(".una"),
        };
        if carries {
            let found = extract(path, bytes, &mut errors);
            if !found.is_empty() {
                carrying.push(path.as_str());
            }
            directives.extend(found);
        }
    }
    if carrying.len() > 1 {
        errors.push(format!(
            "directives appear in {}; exactly one file of a multi-module case carries them",
            carrying.join(", ")
        ));
    }
    let case = build(kind, files, specs, &directives, &mut errors);
    if errors.is_empty() {
        Ok(case)
    } else {
        Err(errors)
    }
}

/// Collects the directive block of a file (D2) and reports unknown and misplaced directives.
fn extract(file: &str, bytes: &[u8], errors: &mut Vec<String>) -> Vec<Directive> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut directives = Vec::new();
    let mut in_block = true;
    for (index, raw) in bytes.split(|&byte| byte == b'\n').enumerate() {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let number = index + 1;
        if in_block {
            if let Some((name, value)) = std::str::from_utf8(line).ok().and_then(directive_form) {
                if NAMES.contains(&name) {
                    directives.push(Directive {
                        file: file.to_owned(),
                        line: number,
                        name: name.to_owned(),
                        value: value.to_owned(),
                    });
                } else {
                    errors.push(format!("{file}:{number}: unknown directive `-- {name}:`"));
                }
                continue;
            }
            in_block = false;
        }
        let misplaced = NAMES
            .into_iter()
            .find(|name| line.starts_with(format!("-- {name}: ").as_bytes()));
        if let Some(name) = misplaced {
            let problem = if std::str::from_utf8(line).is_err() {
                format!("`-- {name}:` directive is not valid UTF-8")
            } else {
                format!("misplaced or malformed `-- {name}:` directive")
            };
            errors.push(format!("{file}:{number}: {problem}"));
        }
    }
    directives
}

/// Splits a line of the form `-- <name>: <value>`: a lowercase name, one space after `--` and
/// after the colon, and a value without leading or trailing whitespace.
fn directive_form(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.strip_prefix("-- ")?.split_once(": ")?;
    let name_ok = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let value_ok = !value.is_empty() && value.trim() == value;
    (name_ok && value_ok).then_some((name, value))
}

/// Validates the values, cardinality and exclusivity of the directives of a case (D3).
fn build(
    kind: CaseKind,
    files: &BTreeMap<String, Vec<u8>>,
    specs: &SpecIndex,
    directives: &[Directive],
    errors: &mut Vec<String>,
) -> Case {
    let mut case = Case::default();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut expected_errors = BTreeSet::new();
    let mut failing_examples = BTreeSet::new();
    for directive in directives {
        let at = format!("{}:{}", directive.file, directive.line);
        let value = directive.value.as_str();
        *counts.entry(directive.name.as_str()).or_default() += 1;
        let result = match directive.name.as_str() {
            "test" => {
                case.test = value.to_owned();
                Ok(())
            }
            "spec" => validate_spec(value, specs).map(|()| case.spec = value.to_owned()),
            "grant" => validate_grant(value).map(|()| case.grants.push(value.to_owned())),
            "pending" => parse_pending(value).map(|milestone| case.pending = Some(milestone)),
            _ => match parse_expectation(value, kind, &directive.file, files) {
                Ok(Expectation::Ok) => count(&mut counts, "ok"),
                Ok(Expectation::Tier0Ok) => count(&mut counts, "tier0-ok"),
                Ok(Expectation::TestPass) => count(&mut counts, "test-pass"),
                Ok(Expectation::Error(error)) => {
                    if expected_errors.insert(error) {
                        Ok(())
                    } else {
                        Err(format!("duplicate `-- expect: {value}`"))
                    }
                }
                Ok(Expectation::RunOutput(line)) => {
                    case.run_output.push(line);
                    Ok(())
                }
                Ok(Expectation::TestFail(example)) => {
                    if failing_examples.insert(example) {
                        Ok(())
                    } else {
                        Err(format!("duplicate `-- expect: {value}`"))
                    }
                }
                Err(error) => Err(error),
            },
        };
        if let Err(error) = result {
            errors.push(format!("{at}: {error}"));
        }
    }
    let times = |name: &str| counts.get(name).copied().unwrap_or(0);
    for (name, directive) in [("test", "-- test:"), ("spec", "-- spec:")] {
        match times(name) {
            0 => errors.push(format!("missing `{directive}` directive")),
            1 => {}
            n => errors.push(format!(
                "`{directive}` appears {n} times; exactly one is allowed"
            )),
        }
    }
    if times("expect") == 0 {
        errors.push("missing `-- expect:` directive".to_owned());
    }
    for (name, directive) in [
        ("pending", "-- pending:"),
        ("ok", "-- expect: ok"),
        ("tier0-ok", "-- expect: tier0-ok"),
        ("test-pass", "-- expect: test-pass"),
    ] {
        let n = times(name);
        if n > 1 {
            errors.push(format!(
                "`{directive}` appears {n} times; at most one is allowed"
            ));
        }
    }
    let check_forms = [
        times("ok") > 0,
        times("tier0-ok") > 0,
        !expected_errors.is_empty(),
    ];
    if check_forms.into_iter().filter(|present| *present).count() > 1 {
        errors.push("`ok`, `tier0-ok` and `error` expectations are mutually exclusive".to_owned());
    }
    if times("test-pass") > 0 && !failing_examples.is_empty() {
        errors.push("`test-pass` and `test-fail` expectations are mutually exclusive".to_owned());
    }
    case.check = if times("ok") > 0 {
        CheckExpectation::Ok
    } else if times("tier0-ok") > 0 {
        CheckExpectation::Tier0Ok
    } else if !expected_errors.is_empty() {
        CheckExpectation::Errors(expected_errors)
    } else {
        CheckExpectation::Unspecified
    };
    case.test_outcome = if times("test-pass") > 0 {
        TestExpectation::Pass
    } else if !failing_examples.is_empty() {
        TestExpectation::Fail(failing_examples)
    } else {
        TestExpectation::Unspecified
    };
    case
}

fn count<'a>(counts: &mut BTreeMap<&'a str, usize>, form: &'a str) -> Result<(), String> {
    *counts.entry(form).or_default() += 1;
    Ok(())
}

/// Validates `spec/NN §a[, §b ...]` against the specification files and their headings.
fn validate_spec(value: &str, specs: &SpecIndex) -> Result<(), String> {
    let malformed =
        || format!("expected `spec/NN §a` with optional `, §b` sections, found `{value}`");
    let (number, sections) = value
        .strip_prefix("spec/")
        .and_then(|rest| rest.split_once(' '))
        .filter(|(number, _)| is_spec_number(number))
        .ok_or_else(malformed)?;
    let known = specs
        .0
        .get(number)
        .ok_or_else(|| format!("no specification file docs/spec/{number}-*.md"))?;
    for part in sections.split(", ") {
        let section = part
            .strip_prefix('§')
            .and_then(parse_number)
            .ok_or_else(malformed)?;
        if !known.contains(&section) {
            return Err(format!("spec/{number} has no section {section}"));
        }
    }
    Ok(())
}

fn is_spec_number(text: &str) -> bool {
    text.len() == 2 && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Validates a grant spec of `spec/13 §5`.
fn validate_grant(value: &str) -> Result<(), String> {
    let valid = match value {
        "console" | "clock" | "env" => true,
        _ => {
            if let Some(keyfile) = value.strip_prefix("keys:") {
                !keyfile.is_empty()
            } else if let Some(rest) = value.strip_prefix("files:") {
                rest.rsplit_once(':').is_some_and(|(path, mode)| {
                    !path.is_empty() && matches!(mode, "read" | "write" | "read,write")
                })
            } else {
                false
            }
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid grant `{value}` (spec/13 §5)"))
    }
}

/// Parses `M1` to `M10`.
fn parse_pending(value: &str) -> Result<u32, String> {
    value
        .strip_prefix('M')
        .and_then(parse_number)
        .filter(|milestone| (1..=10).contains(milestone))
        .ok_or_else(|| format!("expected a milestone from M1 to M10, found `{value}`"))
}

/// Parses the value of `-- expect:`.
fn parse_expectation(
    value: &str,
    kind: CaseKind,
    file: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Expectation, String> {
    match value {
        "ok" => return Ok(Expectation::Ok),
        "tier0-ok" => return Ok(Expectation::Tier0Ok),
        "test-pass" => return Ok(Expectation::TestPass),
        _ => {}
    }
    if let Some(rest) = value.strip_prefix("error ") {
        return parse_error(rest, kind, file, files).map(Expectation::Error);
    }
    if let Some(rest) = value.strip_prefix("run-output ") {
        return parse_quoted(rest).map(Expectation::RunOutput);
    }
    if let Some(rest) = value.strip_prefix("test-fail ") {
        let example = parse_quoted(rest)?;
        if example.is_empty() {
            return Err("`test-fail` needs a non-empty example name".to_owned());
        }
        return Ok(Expectation::TestFail(example));
    }
    Err(format!("unknown expectation `{value}`"))
}

/// Parses `<code> at <line>:<col>`, or `<code> at <path>:<line>:<col>` in a multi-module case,
/// and checks that the position exists in its file.
fn parse_error(
    rest: &str,
    kind: CaseKind,
    file: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<ExpectedError, String> {
    let (code, position) = rest
        .split_once(" at ")
        .ok_or_else(|| format!("expected `error <code> at <position>`, found `error {rest}`"))?;
    let code: Code = code
        .parse()
        .map_err(|error| format!("{error} (spec/13 §4)"))?;
    let mut parts = position.rsplitn(3, ':');
    let col = parts.next().and_then(parse_number);
    let line = parts.next().and_then(parse_number);
    let path = parts.next();
    let (Some(line), Some(col)) = (line, col) else {
        return Err(format!(
            "expected a position with 1-based decimal line and column, found `{position}`"
        ));
    };
    let target = match (kind, path) {
        (CaseKind::Single, None) => file,
        (CaseKind::Single, Some(_)) => {
            return Err(format!(
                "a single-module case writes positions as `<line>:<col>`, found `{position}`"
            ));
        }
        (CaseKind::Directory, None) => {
            return Err(format!(
                "a multi-module case writes positions as `<path>:<line>:<col>`, found `{position}`"
            ));
        }
        (CaseKind::Directory, Some(path)) if path.is_empty() || path.contains(':') => {
            return Err(format!("invalid path `{path}` in `{position}`"));
        }
        (CaseKind::Directory, Some(path)) => path,
    };
    let bytes = files
        .get(target)
        .ok_or_else(|| format!("no file `{target}` in the case"))?;
    if !position_exists(bytes, line, col) {
        return Err(format!("position {line}:{col} is outside `{target}`"));
    }
    Ok(ExpectedError {
        code,
        file: target.to_owned(),
        line,
        col,
    })
}

/// Whether a line exists and the column is at most one past its last character, counting
/// columns in Unicode scalar values after `\r\n` becomes `\n` (`spec/01 §1`).
fn position_exists(bytes: &[u8], line: u32, col: u32) -> bool {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let (Ok(line), Ok(col)) = (usize::try_from(line), usize::try_from(col)) else {
        return false;
    };
    text.split('\n')
        .nth(line - 1)
        .is_some_and(|content| col <= content.chars().count() + 1)
}

/// Parses a quoted value whose only escapes are `\"` and `\\`.
fn parse_quoted(text: &str) -> Result<String, String> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| format!("expected a quoted value, found `{text}`"))?;
    let mut value = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(escaped @ ('"' | '\\')) => value.push(escaped),
                _ => {
                    return Err(format!(
                        "only `\\\"` and `\\\\` escapes are allowed in `{text}`"
                    ));
                }
            },
            '"' => return Err(format!("unescaped quote inside `{text}`")),
            c => value.push(c),
        }
    }
    Ok(value)
}

/// A positive decimal integer, without sign or leading zeros.
fn parse_number(text: &str) -> Option<u32> {
    let canonical = !text.is_empty()
        && !text.starts_with('0')
        && text.bytes().all(|byte| byte.is_ascii_digit());
    canonical.then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A case body whose lines start at line 4 after a three-line header: `module m` (4),
    /// blank (5), `f() -> Unit` (6), `=` (7), `  unit` (8) and an empty last line (9).
    const BODY: &str = "module m\n\nf() -> Unit\n=\n  unit\n";

    fn specs() -> SpecIndex {
        SpecIndex(BTreeMap::from([
            ("05".to_owned(), (1..=8).collect()),
            ("07".to_owned(), (1..=6).collect()),
            ("12".to_owned(), (1..=10).collect()),
        ]))
    }

    fn single(text: &str) -> Result<Case, Vec<String>> {
        let files = BTreeMap::from([("case.una".to_owned(), text.as_bytes().to_vec())]);
        parse_case(CaseKind::Single, &files, &specs())
    }

    /// A single-module case with the given directive lines followed by `BODY`.
    fn with_directives(lines: &[&str]) -> Result<Case, Vec<String>> {
        let header: String = lines.iter().map(|line| format!("{line}\n")).collect();
        single(&format!("{header}{BODY}"))
    }

    #[track_caller]
    fn assert_malformed(lines: &[&str], expected: &str) {
        let errors = with_directives(lines).expect_err("the case must be malformed");
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "{errors:?} does not mention {expected:?}"
        );
    }

    fn error(code: Code, line: u32, col: u32) -> ExpectedError {
        ExpectedError {
            code,
            file: "case.una".to_owned(),
            line,
            col,
        }
    }

    #[test]
    fn parses_the_mandatory_directives() {
        let case =
            with_directives(&["-- test: t", "-- spec: spec/05 §6", "-- expect: ok"]).unwrap();
        assert_eq!(
            case,
            Case {
                test: "t".to_owned(),
                spec: "spec/05 §6".to_owned(),
                check: CheckExpectation::Ok,
                ..Case::default()
            }
        );
    }

    #[test]
    fn parses_error_expectations_as_a_set() {
        // Four header lines: `  unit` is line 9 and `f() -> Unit` is line 7.
        let case = with_directives(&[
            "-- test: t",
            "-- spec: spec/12 §3, §6",
            "-- expect: error U5001 at 9:3",
            "-- expect: error U2001 at 7:1",
        ])
        .unwrap();
        assert_eq!(
            case.check,
            CheckExpectation::Errors(BTreeSet::from([
                error(Code::U2001, 7, 1),
                error(Code::U5001, 9, 3),
            ]))
        );
        assert_eq!(case.spec, "spec/12 §3, §6");
    }

    #[test]
    fn parses_the_other_expectations_grants_and_pending() {
        let case = with_directives(&[
            "-- test: t",
            "-- spec: spec/05 §7",
            "-- expect: tier0-ok",
            r#"-- expect: run-output "say \"hi\" \\ bye""#,
            r#"-- expect: run-output """#,
            r#"-- expect: run-output "say \"hi\" \\ bye""#,
            r#"-- expect: test-fail "second example""#,
            r#"-- expect: test-fail "first example""#,
            "-- grant: console",
            "-- grant: clock",
            "-- grant: env",
            "-- grant: keys:secrets/key.bin",
            "-- grant: files:data/in:read",
            "-- grant: files:data/out:write",
            "-- grant: files:data:read,write",
            "-- pending: M7",
        ])
        .unwrap();
        assert_eq!(case.check, CheckExpectation::Tier0Ok);
        assert_eq!(
            case.run_output,
            [r#"say "hi" \ bye"#, "", r#"say "hi" \ bye"#]
        );
        assert_eq!(
            case.test_outcome,
            TestExpectation::Fail(BTreeSet::from([
                "first example".to_owned(),
                "second example".to_owned(),
            ]))
        );
        assert_eq!(case.grants.len(), 7);
        assert_eq!(case.pending, Some(7));
    }

    #[test]
    fn parses_test_pass() {
        let case = with_directives(&["-- test: t", "-- spec: spec/05 §6", "-- expect: test-pass"])
            .unwrap();
        assert_eq!(case.test_outcome, TestExpectation::Pass);
        assert_eq!(case.check, CheckExpectation::Unspecified);
    }

    #[test]
    fn a_byte_order_mark_and_crlf_do_not_affect_directives() {
        let text = "\u{feff}-- test: t\r\n-- spec: spec/05 §6\r\n-- expect: error U0001 at 1:1\r\nmodule m\r\n";
        let case = single(text).unwrap();
        assert_eq!(
            case.check,
            CheckExpectation::Errors(BTreeSet::from([error(Code::U0001, 1, 1)]))
        );
    }

    #[test]
    fn comments_after_the_block_are_not_directives() {
        let text = "-- test: t\n-- spec: spec/05 §6\n-- expect: ok\nmodule m\n-- note: fine\n  -- expect: indented, so a comment\n";
        assert!(single(text).is_ok(), "{:?}", single(text));
    }

    #[test]
    fn positions_may_point_one_past_the_end_of_a_line_or_at_the_last_empty_line() {
        // Three header lines: `  unit` is line 8 (6 characters) and line 9 is empty.
        for position in ["8:7", "9:1"] {
            let expect = format!("-- expect: error U0101 at {position}");
            let case = with_directives(&["-- test: t", "-- spec: spec/05 §6", &expect]);
            assert!(case.is_ok(), "{position}: {case:?}");
        }
    }

    #[test]
    fn positions_count_columns_with_the_rule_of_adr_0015() {
        // Each maximal ill-formed subsequence is one column: `\xE2\x82` (a truncated three-byte
        // sequence) is one, `\xFF` and `\xFE` are one each. A byte order mark and a tab are one
        // column each, and `\r\n` ends a line without adding a column.
        for (line, width) in [
            (&b"a\xE2\x82b"[..], 3),
            (b"a\xFF\xFEb", 4),
            ("\u{feff}ab".as_bytes(), 3),
            (b"\ta\tb", 4),
            (b"ab\r\nnext", 2),
        ] {
            assert!(position_exists(line, 1, width + 1), "{line:?}");
            assert!(!position_exists(line, 1, width + 2), "{line:?}");
        }
    }

    #[test]
    fn rejects_missing_and_repeated_directives() {
        assert_malformed(
            &["-- spec: spec/05 §6", "-- expect: ok"],
            "missing `-- test:` directive",
        );
        assert_malformed(
            &["-- test: t", "-- expect: ok"],
            "missing `-- spec:` directive",
        );
        assert_malformed(
            &["-- test: t", "-- spec: spec/05 §6"],
            "missing `-- expect:` directive",
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- test: u",
                "-- spec: spec/05 §6",
                "-- expect: ok",
            ],
            "`-- test:` appears 2 times; exactly one is allowed",
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- spec: spec/05 §7",
                "-- expect: ok",
            ],
            "`-- spec:` appears 2 times; exactly one is allowed",
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expect: ok",
                "-- pending: M5",
                "-- pending: M6",
            ],
            "`-- pending:` appears 2 times; at most one is allowed",
        );
    }

    #[test]
    fn rejects_unknown_and_misplaced_directives() {
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expcet: ok",
                "-- expect: ok",
            ],
            "case.una:3: unknown directive `-- expcet:`",
        );
        assert_malformed(
            &["-- test: t", "-- spec: spec/05 §6", "", "-- expect: ok"],
            "case.una:4: misplaced or malformed `-- expect:` directive",
        );
        assert_malformed(
            &["-- test: t", "-- spec: spec/05 §6", "-- expect: ok "],
            "case.una:3: misplaced or malformed `-- expect:` directive",
        );
        assert_malformed(
            &["-- test: t", "-- spec: spec/05 §6", "-- expect:  ok"],
            "case.una:3: misplaced or malformed `-- expect:` directive",
        );
        let files = BTreeMap::from([(
            "case.una".to_owned(),
            b"-- test: t\n-- spec: spec/05 \xA76\n-- expect: ok\nmodule m\n".to_vec(),
        )]);
        let errors = parse_case(CaseKind::Single, &files, &specs()).unwrap_err();
        assert!(
            errors.contains(&"case.una:2: `-- spec:` directive is not valid UTF-8".to_owned()),
            "{errors:?}"
        );
    }

    /// Three header lines ending with `expect`; `  unit` is line 8 (6 characters).
    fn with(expect: &str) -> [&str; 3] {
        ["-- test: t", "-- spec: spec/05 §6", expect]
    }

    #[test]
    fn rejects_malformed_error_expectations() {
        assert_malformed(
            &with("-- expect: error U9999 at 8:3"),
            "unknown diagnostic code \"U9999\"",
        );
        assert_malformed(
            &with("-- expect: error U4006 at 8:3"),
            "unknown diagnostic code \"U4006\"",
        );
        assert_malformed(
            &with("-- expect: error U3005"),
            "expected `error <code> at <position>`",
        );
        assert_malformed(
            &with("-- expect: error U3005 at 0:3"),
            "1-based decimal line and column",
        );
        assert_malformed(
            &with("-- expect: error U3005 at 08:3"),
            "1-based decimal line and column",
        );
        assert_malformed(
            &with("-- expect: error U3005 at 8"),
            "1-based decimal line and column",
        );
        assert_malformed(
            &with("-- expect: error U3005 at 10:1"),
            "position 10:1 is outside `case.una`",
        );
        assert_malformed(
            &with("-- expect: error U3005 at 8:8"),
            "position 8:8 is outside `case.una`",
        );
        assert_malformed(
            &with("-- expect: error U3005 at case.una:8:3"),
            "a single-module case writes positions as `<line>:<col>`",
        );
    }

    #[test]
    fn rejects_contradictory_or_duplicate_expectations() {
        // Four header lines: `  unit` is line 9.
        let exclusive = "`ok`, `tier0-ok` and `error` expectations are mutually exclusive";
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expect: ok",
                "-- expect: error U3005 at 9:3",
            ],
            exclusive,
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expect: ok",
                "-- expect: tier0-ok",
            ],
            exclusive,
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expect: ok",
                "-- expect: ok",
            ],
            "`-- expect: ok` appears 2 times; at most one is allowed",
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expect: test-pass",
                r#"-- expect: test-fail "e""#,
            ],
            "`test-pass` and `test-fail` expectations are mutually exclusive",
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                "-- expect: error U3005 at 9:3",
                "-- expect: error U3005 at 9:3",
            ],
            "case.una:4: duplicate `-- expect: error U3005 at 9:3`",
        );
        assert_malformed(
            &[
                "-- test: t",
                "-- spec: spec/05 §6",
                r#"-- expect: test-fail "e""#,
                r#"-- expect: test-fail "e""#,
            ],
            r#"duplicate `-- expect: test-fail "e"`"#,
        );
    }

    #[test]
    fn rejects_malformed_quoted_values_and_unknown_expectations() {
        assert_malformed(
            &with("-- expect: run-output hello"),
            "expected a quoted value",
        );
        assert_malformed(
            &with(r#"-- expect: run-output "hello"#),
            "expected a quoted value",
        );
        assert_malformed(
            &with(r#"-- expect: run-output "a\nb""#),
            "escapes are allowed",
        );
        assert_malformed(
            &with(r#"-- expect: run-output "a\""#),
            "escapes are allowed",
        );
        assert_malformed(&with(r#"-- expect: run-output "a"b""#), "unescaped quote");
        assert_malformed(
            &with(r#"-- expect: test-fail """#),
            "non-empty example name",
        );
        assert_malformed(&with("-- expect: maybe"), "unknown expectation `maybe`");
    }

    #[test]
    fn rejects_invalid_grants_and_pending_milestones() {
        for grant in [
            "net",
            "keys:",
            "files:/tmp:exec",
            "files::read",
            "files:data",
            "Console",
        ] {
            assert_malformed(
                &[
                    "-- test: t",
                    "-- spec: spec/05 §6",
                    "-- expect: ok",
                    &format!("-- grant: {grant}"),
                ],
                &format!("invalid grant `{grant}` (spec/13 §5)"),
            );
        }
        for pending in ["M0", "M11", "M05", "m5", "soon", "M"] {
            assert_malformed(
                &[
                    "-- test: t",
                    "-- spec: spec/05 §6",
                    "-- expect: ok",
                    &format!("-- pending: {pending}"),
                ],
                &format!("expected a milestone from M1 to M10, found `{pending}`"),
            );
        }
    }

    #[test]
    fn rejects_spec_references_that_do_not_resolve() {
        for spec in [
            "05 §6",
            "spec/5 §6",
            "spec/05",
            "spec/05 6",
            "spec/05 §6,§7",
            "spec/05 §06",
        ] {
            assert_malformed(
                &["-- test: t", &format!("-- spec: {spec}"), "-- expect: ok"],
                "expected `spec/NN §a` with optional `, §b` sections",
            );
        }
        assert_malformed(
            &["-- test: t", "-- spec: spec/99 §1", "-- expect: ok"],
            "no specification file docs/spec/99-*.md",
        );
        assert_malformed(
            &["-- test: t", "-- spec: spec/05 §42", "-- expect: ok"],
            "spec/05 has no section 42",
        );
    }

    fn directory(files: &[(&str, &str)]) -> Result<Case, Vec<String>> {
        let files = files
            .iter()
            .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
            .collect();
        parse_case(CaseKind::Directory, &files, &specs())
    }

    #[test]
    fn parses_a_multi_module_case_with_file_positions() {
        let case = directory(&[
            ("una.toml", "[package]\nname = \"c\"\n"),
            ("src/a.una", "-- test: t\n-- spec: spec/05 §6\n-- expect: error U7002 at una.toml:2:1\n-- expect: error U1003 at src/b.una:1:1\nmodule a\n"),
            ("src/b.una", "module b\n"),
        ])
        .unwrap();
        assert_eq!(
            case.check,
            CheckExpectation::Errors(BTreeSet::from([
                ExpectedError {
                    code: Code::U1003,
                    file: "src/b.una".to_owned(),
                    line: 1,
                    col: 1,
                },
                ExpectedError {
                    code: Code::U7002,
                    file: "una.toml".to_owned(),
                    line: 2,
                    col: 1,
                },
            ]))
        );
    }

    #[test]
    fn rejects_malformed_multi_module_cases() {
        let header = "-- test: t\n-- spec: spec/05 §6\n";
        let errors = directory(&[
            ("una.toml", "[package]\n"),
            (
                "src/a.una",
                &format!("{header}-- expect: error U1003 at 1:1\nmodule a\n"),
            ),
        ])
        .unwrap_err();
        assert!(
            errors.iter().any(
                |e| e.contains("a multi-module case writes positions as `<path>:<line>:<col>`")
            ),
            "{errors:?}"
        );
        let errors = directory(&[
            ("una.toml", "[package]\n"),
            (
                "src/a.una",
                &format!("{header}-- expect: error U1003 at src/c.una:1:1\nmodule a\n"),
            ),
        ])
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("no file `src/c.una` in the case")),
            "{errors:?}"
        );
        let errors = directory(&[
            ("una.toml", "[package]\n"),
            (
                "src/a.una",
                &format!("{header}-- expect: error U1003 at a:b:1:1\nmodule a\n"),
            ),
        ])
        .unwrap_err();
        assert!(
            errors.iter().any(|e| e.contains("invalid path `a:b`")),
            "{errors:?}"
        );
        let errors = directory(&[
            ("una.toml", "[package]\n"),
            ("src/a.una", &format!("{header}-- expect: ok\nmodule a\n")),
            ("src/b.una", "-- expect: ok\nmodule b\n"),
        ])
        .unwrap_err();
        assert!(
            errors.contains(&"directives appear in src/a.una, src/b.una; exactly one file of a multi-module case carries them".to_owned()),
            "{errors:?}"
        );
    }
}
