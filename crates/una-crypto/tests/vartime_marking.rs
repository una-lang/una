//! The variable-time routines carry `_vartime` in their names, and each use outside another such
//! routine states why its operands are public (ADR 0133 rule 8, #1481). A source check over
//! every file of `src/`: the marking is a convention of names and comments, which no type
//! enforces. Its limits: it reads the word "public" in the comment and not its sense, it sees a
//! call and not a routine passed as a value, and it cannot tell that a routine without the suffix
//! is variable time. The name at the call site is the guard; this check is a second line.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::path::PathBuf;

/// The directory `src/` of the crate.
fn src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The library code of `text`, a source file: the text before its unit tests, if it has any.
fn library_part(text: &str) -> &str {
    let end = text.find("#[cfg(test)]\nmod tests").unwrap_or(text.len());
    &text[..end]
}

/// The library code of `src/<name>`.
fn library_source(name: &str) -> String {
    let text = std::fs::read_to_string(src().join(name)).unwrap();
    library_part(&text).to_owned()
}

/// Every `.rs` file of `src/`, by name, with its library code. `src/` holds files only: a
/// directory there fails the check, which would not read it.
fn library_sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(src()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        assert!(
            entry.file_type().unwrap().is_file(),
            "src/{name} is not a file: this check reads the files of src/ only"
        );
        if name.ends_with(".rs") {
            found.push((name.clone(), library_source(&name)));
        }
    }
    found.sort();
    found
}

/// The name of the function a line declares, if it declares one: the identifier after the
/// keyword `fn`, whatever comes before it (`pub(super)`, `const`, `unsafe`, `async`, `extern`),
/// when a parameter list or a generic list follows it.
fn declared(line: &str) -> Option<&str> {
    let code = line.trim_start();
    if code.starts_with("//") {
        return None;
    }
    let at = if code.starts_with("fn ") {
        0
    } else {
        code.find(" fn ")? + 1
    };
    let rest = &code[at + 3..];
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    (end > 0 && (rest[end..].starts_with('(') || rest[end..].starts_with('<')))
        .then(|| &rest[..end])
}

/// The lines of `source` that call a `_vartime` routine from a function that is not one itself,
/// without a comment directly above that says the operands are public.
fn unmarked_uses(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut inside = "";
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if let Some(name) = declared(line) {
            inside = name;
            continue;
        }
        let code = line.trim_start();
        if code.starts_with("//") || !code.contains("_vartime(") || inside.ends_with("_vartime") {
            continue;
        }
        let comment: String = lines[..at]
            .iter()
            .rev()
            .take_while(|above| above.trim_start().starts_with("//"))
            .map(|above| above.to_lowercase())
            .collect();
        if !comment.contains("public") {
            found.push(format!("in `{inside}`: {code}"));
        }
    }
    found
}

#[test]
fn the_variable_time_routines_are_named_and_their_uses_say_public() {
    let mont = library_source("mont.rs");
    let ecdsa = library_source("ecdsa.rs");
    // The names: the table-indexed exponentiation and the group law with branches.
    assert!(mont.contains("fn pow_vartime("));
    assert!(!mont.contains("fn pow("));
    for name in [
        "is_infinity",
        "on_curve",
        "double",
        "add",
        "joint_mul",
        "affine_x",
    ] {
        assert!(ecdsa.contains(&format!("fn {name}_vartime(")), "{name}");
        assert!(!ecdsa.contains(&format!("fn {name}(")), "{name}");
    }
    // No call keeps an unmarked name.
    assert!(!ecdsa.contains(".pow("));
    // The uses, in every file of the crate: a new module that calls one is read too.
    let sources = library_sources();
    for expected in ["mont.rs", "ecdsa.rs", "ed25519.rs", "fe25519.rs"] {
        assert!(
            sources.iter().any(|(name, _)| name == expected),
            "{expected}"
        );
    }
    for (name, source) in &sources {
        let unmarked = unmarked_uses(source);
        assert!(unmarked.is_empty(), "{name}: {unmarked:#?}");
    }
}

#[test]
fn a_declaration_is_read_whatever_its_qualifiers() {
    for (line, name) in [
        ("fn verify(&self) -> bool {", "verify"),
        ("    pub fn digest(m: &[u8]) -> [u8; 32] {", "digest"),
        (
            "    pub(crate) fn pow_vartime(&self, a: &Limbs<N>) -> Limbs<N> {",
            "pow_vartime",
        ),
        ("    pub(super) fn scalar(k: &[u8; 32]) {", "scalar"),
        ("    pub const fn scalar_len(self) -> usize {", "scalar_len"),
        ("    const fn one() -> Self {", "one"),
        ("    pub(crate) unsafe fn raw(p: *const u8) {", "raw"),
        ("fn sub_raw<const N: usize>(a: &Limbs<N>) {", "sub_raw"),
    ] {
        assert_eq!(declared(line), Some(name), "{line}");
    }
    for line in [
        "    // The fn pointer of the table.",
        "    let f: fn(u8) -> u8 = step;",
        "    acc = self.double_vartime(&acc);",
        "    /// See fn verify(x) for the checks.",
    ] {
        assert_eq!(declared(line), None, "{line}");
    }
    // A use in a function declared with qualifiers, after a `_vartime` routine, is not read as
    // inside that routine.
    let unmarked = unmarked_uses(
        "fn add_vartime() {\n    self.double_vartime(p)\n}\n\
         pub(super) const fn ladder() {\n    let y = f.pow_vartime(&a, &secret);\n}\n",
    );
    assert_eq!(
        unmarked,
        ["in `ladder`: let y = f.pow_vartime(&a, &secret);"]
    );
}

#[test]
fn the_check_finds_an_unmarked_use() {
    let unmarked = unmarked_uses(
        "fn verify() {\n    // Checks the key.\n    let x = f.pow_vartime(&a, &e);\n}\n\
         fn add_vartime() {\n    self.double_vartime(p)\n}\n\
         fn other() {\n    // Public exponent: a constant.\n    let y = f.pow_vartime(&a, &e);\n}\n",
    );
    assert_eq!(unmarked, ["in `verify`: let x = f.pow_vartime(&a, &e);"]);
}
