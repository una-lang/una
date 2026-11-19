//! The variable-time routines carry `_vartime` in their names, and each use outside another such
//! routine states why its operands are public (ADR 0133 rule 8, #1481). A source check: the
//! marking is a convention of names and comments, which no type enforces.

// Test code: unwrap, indexing and panics are how a test fails (clippy.toml allows them in `#[test]`
// functions only, not in their helpers).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::path::PathBuf;

/// The library code of `src/<name>`: the text before its unit tests.
fn library_source(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(name);
    let text = std::fs::read_to_string(path).unwrap();
    let end = text.find("#[cfg(test)]\nmod tests").unwrap();
    text[..end].to_owned()
}

/// The name of the function a line declares, if it declares one.
fn declared(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = rest.strip_prefix("pub(crate) ").unwrap_or(rest);
    let rest = rest.strip_prefix("pub ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
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
    // The uses.
    for (name, source) in [("mont.rs", &mont), ("ecdsa.rs", &ecdsa)] {
        let unmarked = unmarked_uses(source);
        assert!(unmarked.is_empty(), "{name}: {unmarked:#?}");
    }
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
