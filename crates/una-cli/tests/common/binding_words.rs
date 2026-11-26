//! The binding words of edition `0.5` in the Una sources of the tests (ADR 0137): a test that
//! writes a program in edition `0.4` words and checks it in edition `0.5` rewrites it with
//! [`of_0_5`], as `una fix` rewrites a package that moves to edition `0.5`.

/// `source` with each binding statement that starts its line, after the indentation, written
/// with the words of edition `0.5`: `let edit x` becomes `var x`, and `let x` becomes
/// `const x`. Nothing else changes.
#[allow(dead_code)]
pub fn of_0_5(source: &str) -> String {
    source
        .split_inclusive('\n')
        .map(|line| {
            let content = line.trim_start_matches(' ');
            let indent = &line[..line.len() - content.len()];
            if let Some(rest) = content.strip_prefix("let edit ") {
                format!("{indent}var {rest}")
            } else if let Some(rest) = content.strip_prefix("let ") {
                format!("{indent}const {rest}")
            } else {
                line.to_owned()
            }
        })
        .collect()
}

/// The column, in the source rewritten by [`of_0_5`], of `col` on `line` of `source`: the
/// columns after a rewritten keyword move by the change of its length.
#[allow(dead_code)]
pub fn column_in_0_5(source: &str, line: u64, col: u64) -> u64 {
    let Some(text) = source.lines().nth(line as usize - 1) else {
        return col;
    };
    let content = text.trim_start_matches(' ');
    let indent = (text.len() - content.len()) as u64;
    let delta: i64 = if content.starts_with("let edit ") {
        3 - 8
    } else if content.starts_with("let ") {
        5 - 3
    } else {
        0
    };
    if col > indent + 1 {
        (col as i64 + delta) as u64
    } else {
        col
    }
}
