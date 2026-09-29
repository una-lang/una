//! Reading TOML documents with the position of every key and value.

use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};
use una_diag::Code;

use crate::{Located, ProjectError};

/// A table and where its name is written, for errors about the table itself.
pub(crate) struct Table<'t, 'i> {
    pub(crate) name: String,
    pub(crate) at: Range<usize>,
    pub(crate) entries: &'t DeTable<'i>,
}

/// Parses `text`, or reports its syntax error with `code`.
pub(crate) fn parse<'i>(
    text: &'i str,
    code: Code,
    errors: &mut Vec<ProjectError>,
) -> Option<Spanned<DeTable<'i>>> {
    match DeTable::parse(text) {
        Ok(table) => Some(table),
        Err(error) => {
            let range = error.span().unwrap_or(0..0);
            let message = error
                .message()
                .lines()
                .next()
                .unwrap_or("invalid TOML")
                .trim()
                .trim_end_matches('.')
                .to_owned();
            errors.push(ProjectError::new(
                code,
                range,
                format!("invalid TOML: {}", lowercase_start(&message)),
            ));
            None
        }
    }
}

fn lowercase_start(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The name of a value's type, for messages.
fn type_name(value: &DeValue<'_>) -> &'static str {
    match value {
        DeValue::String(_) => "a string",
        DeValue::Integer(_) => "an integer",
        DeValue::Float(_) => "a float",
        DeValue::Boolean(_) => "a boolean",
        DeValue::Datetime(_) => "a date",
        DeValue::Array(_) => "an array",
        DeValue::Table(_) => "a table",
    }
}

impl<'t, 'i> Table<'t, 'i> {
    /// The value `key` of this table as a table, or an error with `code`.
    pub(crate) fn of(
        key: &Spanned<std::borrow::Cow<'i, str>>,
        value: &'t Spanned<DeValue<'i>>,
        code: Code,
        errors: &mut Vec<ProjectError>,
    ) -> Option<Table<'t, 'i>> {
        match value.get_ref() {
            DeValue::Table(entries) => Some(Table {
                name: key.get_ref().to_string(),
                at: key.span(),
                entries,
            }),
            other => {
                errors.push(ProjectError::new(
                    code,
                    value.span(),
                    format!(
                        "`{}` must be a table, not {}",
                        key.get_ref(),
                        type_name(other)
                    ),
                ));
                None
            }
        }
    }

    /// An unknown key of this table.
    pub(crate) fn unknown(
        &self,
        key: &Spanned<std::borrow::Cow<'i, str>>,
        code: Code,
    ) -> ProjectError {
        ProjectError::new(
            code,
            key.span(),
            format!("unknown key `{}` in [{}]", key.get_ref(), self.name),
        )
    }

    /// A required key that this table lacks.
    pub(crate) fn missing(&self, key: &str, code: Code) -> ProjectError {
        ProjectError::new(
            code,
            self.at.clone(),
            format!("[{}] has no `{key}`", self.name),
        )
    }
}

/// The string value of `key`, or an error with `code`.
pub(crate) fn string(
    key: &str,
    value: &Spanned<DeValue<'_>>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) -> Option<Located> {
    match value.get_ref() {
        DeValue::String(text) => Some(Located {
            value: text.to_string(),
            range: value.span(),
        }),
        other => {
            errors.push(ProjectError::new(
                code,
                value.span(),
                format!("`{key}` must be a string, not {}", type_name(other)),
            ));
            None
        }
    }
}

/// The integer value of `key`, or an error with `code`.
pub(crate) fn integer(
    key: &str,
    value: &Spanned<DeValue<'_>>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) -> Option<i128> {
    match value.get_ref() {
        DeValue::Integer(number) => {
            let parsed = i128::from_str_radix(number.as_str(), number.radix()).ok();
            if parsed.is_none() {
                errors.push(ProjectError::new(
                    code,
                    value.span(),
                    format!("`{key}` is too large"),
                ));
            }
            parsed
        }
        other => {
            errors.push(ProjectError::new(
                code,
                value.span(),
                format!("`{key}` must be an integer, not {}", type_name(other)),
            ));
            None
        }
    }
}

/// The strings of the array value of `key`, or errors with `code`.
pub(crate) fn strings(
    key: &str,
    value: &Spanned<DeValue<'_>>,
    code: Code,
    errors: &mut Vec<ProjectError>,
) -> Vec<Located> {
    match value.get_ref() {
        DeValue::Array(items) => items
            .iter()
            .filter_map(|item| string(key, item, code, errors))
            .collect(),
        other => {
            errors.push(ProjectError::new(
                code,
                value.span(),
                format!(
                    "`{key}` must be an array of strings, not {}",
                    type_name(other)
                ),
            ));
            Vec::new()
        }
    }
}
