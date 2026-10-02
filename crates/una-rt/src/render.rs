//! The Una form of a value whose type `una build` describes: the failure of `main`, which
//! U6001 shows with its `public` fields (`spec/04 §8`), as the interpreter renders it.
//!
//! A word does not say its type, so the build embeds the types the failure can hold: each one
//! a shape, with the variants of a kind, the names and types of their fields, and the label of
//! each field's declared type.

use serde_json::Value;
use una_diag::Location;

use crate::values;

/// The shape of a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `Int`.
    Int,
    /// `Bool`.
    Bool,
    /// `Text`.
    Text,
    /// `Unit`.
    Unit,
    /// A function.
    Fn,
    /// `List` of the type at this index.
    List(usize),
    /// A kind, by its variants in declaration order.
    Data(Vec<Variant>),
    /// A type the build cannot describe.
    Other,
}

/// A variant of a kind, or the constructor of a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    /// Its name.
    pub name: String,
    /// Its fields, in order.
    pub fields: Vec<Field>,
}

/// A field of a variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Its name.
    pub name: String,
    /// The index of its type.
    pub ty: usize,
    /// The level of its declared type's label when it is above `public`: the field renders as
    /// `<redacted:level>`.
    pub redacted: Option<String>,
    /// Whether U6001 shows it: its declared type is `public` and has no type parameter.
    pub shown: bool,
}

/// What U6001 needs to report a failure of `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// Where `main` is named.
    pub location: Location,
    /// The index of `main`'s error kind.
    pub root: usize,
    /// The types, by index.
    pub types: Vec<Shape>,
}

impl Failure {
    /// The description of `value`, the `failure` of the embedded description.
    pub fn parse(value: &Value, location: Location) -> Option<Failure> {
        let root = usize::try_from(value.get("root")?.as_u64()?).ok()?;
        let types = value
            .get("types")?
            .as_array()?
            .iter()
            .map(shape)
            .collect::<Option<Vec<_>>>()?;
        Some(Failure {
            location,
            root,
            types,
        })
    }

    /// The failure `word` as U6001 shows it: its variant, and its fields that are `public`.
    pub fn shown(&self, word: i64) -> String {
        let Some(Shape::Data(variants)) = self.types.get(self.root) else {
            return "?".to_owned();
        };
        let Some(variant) = usize::try_from(values::tag(word))
            .ok()
            .and_then(|tag| variants.get(tag))
        else {
            return "?".to_owned();
        };
        let fields: Vec<String> = variant
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.shown)
            .map(|(index, field)| {
                let value = field_word(word, index);
                format!("{}: {}", field.name, self.render(field.ty, value))
            })
            .collect();
        if fields.is_empty() {
            variant.name.clone()
        } else {
            format!("{}({})", variant.name, fields.join(", "))
        }
    }

    /// The Una form of `word`, a value of the type at `ty`.
    fn render(&self, ty: usize, word: i64) -> String {
        match self.types.get(ty) {
            Some(Shape::Int) => values::int_value(word).to_string(),
            Some(Shape::Bool) => (word != 0).to_string(),
            Some(Shape::Text) => una_core::quote(&values::text_value(word)),
            Some(Shape::Unit) => "unit".to_owned(),
            Some(Shape::Fn) => "<function>".to_owned(),
            Some(Shape::List(item)) => {
                let items: Vec<String> = (0..values::count(word))
                    .map(|index| self.render(*item, values::field(word, index)))
                    .collect();
                format!("[{}]", items.join(", "))
            }
            Some(Shape::Data(variants)) => {
                let Some(variant) = usize::try_from(values::tag(word))
                    .ok()
                    .and_then(|tag| variants.get(tag))
                else {
                    return "?".to_owned();
                };
                let fields: Vec<(&str, String)> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| {
                        let unit = matches!(self.types.get(field.ty), Some(Shape::Unit));
                        let value = match &field.redacted {
                            // `unit` carries no information and is never redacted.
                            Some(level) if !unit => format!("<redacted:{level}>"),
                            _ => self.render(field.ty, field_word(word, index)),
                        };
                        (field.name.as_str(), value)
                    })
                    .collect();
                match fields.as_slice() {
                    [] => variant.name.clone(),
                    [(_, only)] => format!("{}({only})", variant.name),
                    _ => {
                        let fields: Vec<String> = fields
                            .iter()
                            .map(|(name, value)| format!("{name}: {value}"))
                            .collect();
                        format!("{}({})", variant.name, fields.join(", "))
                    }
                }
            }
            Some(Shape::Other) | None => "?".to_owned(),
        }
    }
}

fn field_word(word: i64, index: usize) -> i64 {
    values::field(word, i64::try_from(index).unwrap_or(i64::MAX))
}

fn index(value: &Value, key: &str) -> Option<usize> {
    usize::try_from(value.get(key)?.as_u64()?).ok()
}

fn shape(value: &Value) -> Option<Shape> {
    Some(match value.get("shape")?.as_str()? {
        "int" => Shape::Int,
        "bool" => Shape::Bool,
        "text" => Shape::Text,
        "unit" => Shape::Unit,
        "fn" => Shape::Fn,
        "list" => Shape::List(index(value, "item")?),
        "data" => Shape::Data(
            value
                .get("variants")?
                .as_array()?
                .iter()
                .map(|variant| {
                    Some(Variant {
                        name: variant.get("name")?.as_str()?.to_owned(),
                        fields: variant
                            .get("fields")?
                            .as_array()?
                            .iter()
                            .map(|field| {
                                Some(Field {
                                    name: field.get("name")?.as_str()?.to_owned(),
                                    ty: index(field, "type")?,
                                    redacted: field
                                        .get("redacted")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned),
                                    shown: field.get("shown")?.as_bool()?,
                                })
                            })
                            .collect::<Option<Vec<_>>>()?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => Shape::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use una_diag::Position;

    fn location() -> Location {
        Location {
            file: "src/main.una".to_owned(),
            start: Position { line: 1, col: 1 },
            end: Position { line: 1, col: 5 },
        }
    }

    #[test]
    fn a_failure_shows_its_public_fields_and_redacts_labeled_ones_inside() {
        // kind Stop = | Halt(reason: Text, code: secret Int, inner: Option[Pair])
        // kind Pair: name: Text, pin: secret Int
        let description = serde_json::json!({
            "root": 0,
            "types": [
                {"shape": "data", "variants": [{"name": "Halt", "fields": [
                    {"name": "reason", "type": 1, "redacted": null, "shown": true},
                    {"name": "code", "type": 2, "redacted": "secret", "shown": false},
                    {"name": "inner", "type": 3, "redacted": null, "shown": true},
                ]}]},
                {"shape": "text"},
                {"shape": "int"},
                {"shape": "data", "variants": [
                    {"name": "Some", "fields": [
                        {"name": "value", "type": 4, "redacted": null, "shown": false},
                    ]},
                    {"name": "None", "fields": []},
                ]},
                {"shape": "data", "variants": [{"name": "Pair", "fields": [
                    {"name": "name", "type": 1, "redacted": null, "shown": true},
                    {"name": "pin", "type": 2, "redacted": "secret", "shown": false},
                ]}]},
            ],
        });
        let failure = Failure::parse(&description, location()).unwrap();
        let pair = values::data(0);
        values::push(pair, values::text_word("ana".to_owned()));
        values::push(pair, 85);
        let some = values::data(0);
        values::push(some, pair);
        let halt = values::data(0);
        values::push(halt, values::text_word("maintenance".to_owned()));
        values::push(halt, 8485);
        values::push(halt, some);
        assert_eq!(
            failure.shown(halt),
            r#"Halt(reason: "maintenance", inner: Some(Pair(name: "ana", pin: <redacted:secret>)))"#
        );
        let none = values::data(0);
        values::push(none, values::text_word("x".to_owned()));
        values::push(none, 1);
        values::push(none, 3);
        assert_eq!(failure.shown(none), r#"Halt(reason: "x", inner: None)"#);
    }
}
