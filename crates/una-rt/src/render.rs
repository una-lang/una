//! The Una form of a value whose type `una build` describes, as the interpreter renders it: the
//! failure of `main`, which U6001 shows with its `public` fields (`spec/04 §8`), and the
//! arguments, results and failures of the operations a trace records (`spec/11 §5`).
//!
//! A word does not say its type, so the build embeds the types the program renders: each one
//! a shape, with the variants of a kind, the names and types of their fields, and the label of
//! each field's declared type.

use serde_json::Value;
use una_diag::Location;
use una_http::serve::trace::TraceKey;

use crate::values;

/// The shape of a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `Int`.
    Int,
    /// `Decimal[s]`, its coefficient written with s digits after the point (`spec/29 §16`).
    Decimal(u8),
    /// `Bool`.
    Bool,
    /// `Text`.
    Text,
    /// `Unit`.
    Unit,
    /// `Hashed`.
    Hashed,
    /// `ReplyRight` of `std.host` (ADR 0123 rule 1): written `<reply-right>` everywhere.
    Right,
    /// `F64`, by its shortest round-trip text (`spec/29 §7`).
    F64,
    /// `F32`, by its shortest round-trip text (ADR 0120 rule 5).
    F32,
    /// A function.
    Fn,
    /// `List` of the type at this index.
    List(usize),
    /// `Map` of the key and value types at these indices (`spec/21 §2` rule 4).
    Map(usize, usize),
    /// `Array`, `Matrix`, `Tensor` or `Lu` of the element type at this index (`spec/29 §17`):
    /// the value says which, and its shape.
    Dense(usize),
    /// `Stack` or `Queue` of the type at this index, written as a list, front first, or
    /// `PriorityQueue` of items of that type, written as a list in the order of its pops
    /// (`spec/28 §4`).
    Seq(usize),
    /// `Set` of the type at this index, written in braces, ascending (`spec/28 §4`).
    Set(usize),
    /// `BitSet`, its members written in braces, ascending (`spec/28 §4`).
    Bits,
    /// `HashMap` of the key and value types at these indices, written as a map in the order its
    /// keys were first added, never in the order of their hashes (`spec/28 §4`).
    HashMap(usize, usize),
    /// `HashSet` of the type at this index, written in braces in insertion order.
    HashSet(usize),
    /// A kind, by its variants in declaration order.
    Data(Vec<Variant>),
    /// The type parameter at this position of a composite descriptor, which says how to write
    /// its values (`spec/20 §7`, ADR 0068 D6).
    Param(usize),
    /// A type the build cannot describe: its values render as `?`.
    Other,
}

/// A variant of a kind, or the constructor of a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    /// Its name.
    pub name: String,
    /// Its fields, in order.
    pub fields: Vec<Field>,
    /// The name of its kind when it is a kind of the server of `std.host` (`Request`, `Reply`,
    /// `Header`, `Param`), whose fields an event of the host's `HttpServer` writes in the
    /// reduced form of ADR 0123 rule 1.
    pub server: Option<String>,
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
    /// The driven module whose `init` this is, for a driven program (`spec/26 §2` rule 2):
    /// U6001 then names `init` of that module, not `main` (#1809).
    pub driven: Option<String>,
}

/// What U6001 says of a value whose text does not fit the run's memory budget (`spec/11 §7`,
/// #1223): it is named rather than written, as the interpreter names it.
pub const OVER_BUDGET: &str = "<a value whose text exceeds the memory budget>";

/// The types the program renders, by index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Types(pub Vec<Shape>);

impl Types {
    /// The types of the embedded description, its `types` array.
    pub fn parse(value: &Value) -> Option<Types> {
        value
            .as_array()?
            .iter()
            .map(shape)
            .collect::<Option<Vec<_>>>()
            .map(Types)
    }

    /// The variant of `word`, a value of the kind at `ty`.
    fn variant(&self, ty: usize, word: i64) -> Option<&Variant> {
        let Some(Shape::Data(variants)) = self.0.get(ty) else {
            return None;
        };
        usize::try_from(values::tag(word))
            .ok()
            .and_then(|tag| variants.get(tag))
    }

    /// The name of the variant of `word`, a value of the kind at `ty`.
    pub fn variant_name(&self, ty: usize, word: i64) -> String {
        self.variant(ty, word)
            .map_or_else(|| "?".to_owned(), |variant| variant.name.clone())
    }

    /// The failure `word`, of the kind at `ty`, as U6001 shows it: its variant, and its fields
    /// that are `public`.
    /// A field whose text does not fit the memory budget is named, [`OVER_BUDGET`], as the
    /// interpreter names it (`spec/11 §7`, #1223).
    pub fn shown(&self, ty: usize, word: i64) -> String {
        let Some(variant) = self.variant(ty, word) else {
            return "?".to_owned();
        };
        let fields: Vec<String> = variant
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.shown)
            .map(|(index, field)| {
                let value = field_word(word, index);
                let text = self
                    .render(field.ty, value)
                    .unwrap_or_else(|| OVER_BUDGET.to_owned());
                format!("{}: {}", field.name, text)
            })
            .collect();
        if fields.is_empty() {
            variant.name.clone()
        } else {
            format!("{}({})", variant.name, fields.join(", "))
        }
    }

    /// `word`, a value of the type at `ty`, in its Una form, or `<redacted:level>` when
    /// `redacted` names a level; `unit` carries no information and is never redacted.
    pub fn redacted(&self, ty: usize, word: i64, redacted: Option<&str>) -> Option<String> {
        let unit = matches!(self.0.get(ty), Some(Shape::Unit));
        match redacted {
            Some(level) if !unit => Some(format!("<redacted:{level}>")),
            _ => self.render(ty, word),
        }
    }

    /// The Una form of `word`, a value of the type at `ty`, with the fields labeled above
    /// `public` redacted, as traces and diagnostics show it. An `Int` of any size is written,
    /// in O(M(d) log d) for its d digits, once the run has room for its text ([`int_written`]):
    /// `None` when it has not, and the caller aborts the run with U6003 (`spec/11 §7`, #1223).
    pub fn render(&self, ty: usize, word: i64) -> Option<String> {
        render(&self.0, ty, word, true, &[], None)
    }

    /// `word` in an event of the host's `HttpServer`: `<redacted:level>` when `redacted` names a
    /// level, as [`Types::redacted`] writes it, and otherwise its Una form with the fields of
    /// the server's kinds in the reduced form of ADR 0123 rule 1 under `key`
    /// (`una_interp::serve_trace`).
    pub fn server_redacted(
        &self,
        ty: usize,
        word: i64,
        redacted: Option<&str>,
        key: &TraceKey,
    ) -> Option<String> {
        let unit = matches!(self.0.get(ty), Some(Shape::Unit));
        match redacted {
            Some(level) if !unit => Some(format!("<redacted:{level}>")),
            _ => render(&self.0, ty, word, true, &[], Some(key)),
        }
    }

    /// `word` in an event of the host's `HttpServer` with every field, in the reduced form under
    /// `key`, without the redaction of labels: for the toolchain's tests only, which compare
    /// the reduced form of both engines (`una_http::serve::program::Port::reveals_trace`).
    pub fn server_written(&self, ty: usize, word: i64, key: &TraceKey) -> Option<String> {
        render(&self.0, ty, word, false, &[], Some(key))
    }

    /// The Una form of `word` with every field, as interpolation writes it: the text carries
    /// the value's labels (`spec/11 §5`, ADR 0068). `params` are the descriptors of the type
    /// parameters that the type at `ty` holds, by position.
    pub fn written(&self, ty: usize, word: i64, params: &[i64]) -> Option<String> {
        render(&self.0, ty, word, false, params, None)
    }
}

/// What remains to render: a value of a type, with the descriptors of the type parameters it
/// holds, or text as it is.
enum Task {
    Value(usize, i64, std::rc::Rc<Vec<i64>>),
    Text(String),
}

/// The Una form of `word`, a value of the type at `ty` in `types`, from a stack of tasks rather
/// than the native stack: a value can nest deeper than any call (ADR 0060, draft). With
/// `redact`, a field of a kind whose declared type is labeled above `public` is
/// `<redacted:level>`.
/// With `reduce`, the fields of the server's kinds are in the reduced form of ADR 0123 rule 1
/// under that key.
fn render(
    types: &[Shape],
    ty: usize,
    word: i64,
    redact: bool,
    params: &[i64],
    reduce: Option<&TraceKey>,
) -> Option<String> {
    let mut out = String::new();
    let mut tasks = vec![Task::Value(ty, word, std::rc::Rc::new(params.to_vec()))];
    while let Some(task) = tasks.pop() {
        let (ty, word, params) = match task {
            Task::Text(text) => {
                put(&mut out, &text)?;
                continue;
            }
            Task::Value(ty, word, params) => (ty, word, params),
        };
        // The parts of a list or a variant, in order; pushed in reverse onto the stack.
        let mut parts = Vec::new();
        match types.get(ty) {
            Some(Shape::Int) => put(&mut out, &int_written(word)?)?,
            Some(Shape::Decimal(scale)) => put(
                &mut out,
                &decimal_written(&values::int_value(word), *scale)?,
            )?,
            Some(Shape::Bool) => put(&mut out, if word != 0 { "true" } else { "false" })?,
            Some(Shape::Text) => put(&mut out, &una_core::quote(&values::text_value(word)))?,
            Some(Shape::Unit) => put(&mut out, "unit")?,
            // Interpolation writes no bytes of a hash (ADR 0068, amendment of 2026-10-06).
            Some(Shape::F64) => {
                put(&mut out, &una_core::float::render(values::f64_value(word)))?;
            }
            Some(Shape::F32) => {
                put(
                    &mut out,
                    &una_core::float32::render(values::f32_value(word)),
                )?;
            }
            Some(Shape::Hashed) if !redact => put(&mut out, "<hashed>")?,
            Some(Shape::Hashed) => {
                put(&mut out, "hashed:")?;
                put(&mut out, &una_interp::hex(&values::bytes(word)))?;
            }
            Some(Shape::Fn) => put(&mut out, "<function>")?,
            // Nothing of a reply right is ever written (ADR 0123 rule 1).
            Some(Shape::Right) => put(&mut out, "<reply-right>")?,
            Some(Shape::List(item)) => {
                put(&mut out, "[")?;
                for index in 0..values::count(word) {
                    if index > 0 {
                        parts.push(Task::Text(", ".to_owned()));
                    }
                    parts.push(Task::Value(
                        *item,
                        values::field(word, index),
                        std::rc::Rc::clone(&params),
                    ));
                }
                parts.push(Task::Text("]".to_owned()));
            }
            // A stack or a queue front first, and a priority queue in the order of its pops,
            // as lists; a set ascending, in braces (`spec/28 §4`).
            Some(Shape::Seq(item) | Shape::Set(item)) => {
                let set = matches!(types.get(ty), Some(Shape::Set(_)));
                put(&mut out, if set { "{" } else { "[" })?;
                for (index, word) in values::collection_words(word).into_iter().enumerate() {
                    if index > 0 {
                        parts.push(Task::Text(", ".to_owned()));
                    }
                    parts.push(Task::Value(*item, word, std::rc::Rc::clone(&params)));
                }
                parts.push(Task::Text(if set { "}" } else { "]" }.to_owned()));
            }
            // A bit set's members, integers written at once.
            Some(Shape::Bits) => {
                let members: Vec<String> = values::bit_set_members(word)
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                put(&mut out, &format!("{{{}}}", members.join(", ")))?;
            }
            // `{k: v, ...}` and `{a, b}` in insertion order, `{}` when empty (ADR 0119 rule 10).
            Some(Shape::HashMap(key, value)) => {
                put(&mut out, "{")?;
                for (index, (k, v)) in values::hash_entries(word).into_iter().enumerate() {
                    if index > 0 {
                        parts.push(Task::Text(", ".to_owned()));
                    }
                    parts.push(Task::Value(*key, k, std::rc::Rc::clone(&params)));
                    parts.push(Task::Text(": ".to_owned()));
                    parts.push(Task::Value(*value, v, std::rc::Rc::clone(&params)));
                }
                parts.push(Task::Text("}".to_owned()));
            }
            Some(Shape::HashSet(item)) => {
                put(&mut out, "{")?;
                for (index, (k, _)) in values::hash_entries(word).into_iter().enumerate() {
                    if index > 0 {
                        parts.push(Task::Text(", ".to_owned()));
                    }
                    parts.push(Task::Value(*item, k, std::rc::Rc::clone(&params)));
                }
                parts.push(Task::Text("}".to_owned()));
            }
            // `{k: v, ...}` in key order, `{}` when empty.
            Some(Shape::Map(key, value)) => {
                put(&mut out, "{")?;
                for (index, (k, v)) in values::map_words(word).into_iter().enumerate() {
                    if index > 0 {
                        parts.push(Task::Text(", ".to_owned()));
                    }
                    parts.push(Task::Value(*key, k, std::rc::Rc::clone(&params)));
                    parts.push(Task::Text(": ".to_owned()));
                    parts.push(Task::Value(*value, v, std::rc::Rc::clone(&params)));
                }
                parts.push(Task::Text("}".to_owned()));
            }
            // An `Lu[T]` renders as `<lu size n>`, which is not source, as `<hashed>` is not
            // (ADR 0120 rule 38); any other dense value as the construction call that rebuilds
            // it, its elements in row-major order (rule 41), as the interpreter renders it.
            Some(Shape::Dense(element)) => {
                let Some(dense) = values::dense::parts(word) else {
                    put(&mut out, "?")?;
                    continue;
                };
                let layout = dense.layout;
                let shape = layout.shape.as_slice();
                match (layout.kind, shape) {
                    (values::dense::DenseKind::Lu, _) => {
                        let size = shape.get(1).copied().unwrap_or(0);
                        put(&mut out, &format!("<lu size {size}>"))?;
                        continue;
                    }
                    (values::dense::DenseKind::Matrix, [rows, columns]) => put(
                        &mut out,
                        &format!("matrix(rows: {rows}, columns: {columns}, elements: ["),
                    )?,
                    (values::dense::DenseKind::Tensor, shape) => {
                        let lengths: Vec<String> = shape.iter().map(ToString::to_string).collect();
                        put(
                            &mut out,
                            &format!("tensor(shape: [{}], elements: [", lengths.join(", ")),
                        )?;
                    }
                    _ => put(&mut out, "array([")?,
                }
                for (index, item) in dense.elements.into_iter().enumerate() {
                    if index > 0 {
                        parts.push(Task::Text(", ".to_owned()));
                    }
                    parts.push(match item {
                        values::dense::Element::Raw(x) => Task::Text(una_core::float::render(x)),
                        values::dense::Element::Word(item) => {
                            Task::Value(*element, item, std::rc::Rc::clone(&params))
                        }
                    });
                }
                parts.push(Task::Text("])".to_owned()));
            }
            Some(Shape::Data(variants)) => {
                let variant = usize::try_from(values::tag(word))
                    .ok()
                    .and_then(|tag| variants.get(tag));
                let Some(variant) = variant else {
                    put(&mut out, "?")?;
                    continue;
                };
                put(&mut out, &variant.name)?;
                if !variant.fields.is_empty() {
                    put(&mut out, "(")?;
                    // One field is shown bare, several with their names.
                    let named = variant.fields.len() > 1;
                    // In an event of the host's `HttpServer`, the fields of its kinds that the
                    // reduced form of ADR 0123 rule 1 changes (`una_interp::serve_trace`).
                    let server = reduce.zip(variant.server.as_deref());
                    let header = server
                        .and_then(|_| variant.fields.first())
                        .filter(|field| matches!(types.get(field.ty), Some(Shape::Text)))
                        .map(|_| values::text_value(field_word(word, 0)));
                    for (index, field) in variant.fields.iter().enumerate() {
                        let reduced = server.and_then(|(key, kind)| {
                            let how =
                                una_interp::serve_trace::field(kind, index, header.as_deref())?;
                            Some(match types.get(field.ty) {
                                Some(Shape::Text) => una_interp::serve_trace::write(
                                    how,
                                    key,
                                    &values::text_value(field_word(word, index)),
                                ),
                                // These fields are `Text`; any other value is written as
                                // nothing of it, never in clear.
                                _ => "?".to_owned(),
                            })
                        });
                        if index > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        if named {
                            parts.push(Task::Text(format!("{}: ", field.name)));
                        }
                        let unit = matches!(types.get(field.ty), Some(Shape::Unit));
                        parts.push(match (&field.redacted, reduced) {
                            // `unit` carries no information and is never redacted.
                            (Some(level), _) if redact && !unit => {
                                Task::Text(format!("<redacted:{level}>"))
                            }
                            (_, Some(text)) => Task::Text(text),
                            _ => Task::Value(
                                field.ty,
                                field_word(word, index),
                                std::rc::Rc::clone(&params),
                            ),
                        });
                    }
                    parts.push(Task::Text(")".to_owned()));
                }
            }
            // A value of a type parameter, by its descriptor: an integer in decimal, a text
            // quoted, anything else by the type description the descriptor carries.
            Some(Shape::Param(position)) => {
                match params
                    .get(*position)
                    .and_then(|desc| crate::generic::shape_of(*desc))
                {
                    Some(crate::generic::Written::Int) => {
                        put(&mut out, &int_written(word)?)?;
                    }
                    Some(crate::generic::Written::Text) => {
                        put(&mut out, &una_core::quote(&values::text_value(word)))?;
                    }
                    Some(crate::generic::Written::F64) => {
                        put(&mut out, &una_core::float::render(values::f64_value(word)))?;
                    }
                    Some(crate::generic::Written::F32) => {
                        put(
                            &mut out,
                            &una_core::float32::render(values::f32_value(word)),
                        )?;
                    }
                    Some(crate::generic::Written::Typed(ty, inner)) => {
                        parts.push(Task::Value(ty, word, std::rc::Rc::new(inner)));
                    }
                    None => put(&mut out, "?")?,
                }
            }
            Some(Shape::Other) | None => put(&mut out, "?")?,
        }
        tasks.extend(parts.into_iter().rev());
    }
    Some(out)
}

/// Appends `piece` to `out`, which first grows, when it must, within the memory budget: a
/// growth to at least [`una_core::INT_TEXT_RESERVED_FROM`] bytes reserves the new buffer and
/// the piece against the budget (`spec/11 §7`, #1223), so a text built of many small parts is
/// checked as it grows, at O(1) amortized per part. `None` when they do not fit: the caller
/// aborts the run with U6003.
fn put(out: &mut String, piece: &str) -> Option<()> {
    let need = out.len().saturating_add(piece.len());
    if need > out.capacity() {
        let grown = need.max(out.capacity().saturating_mul(2));
        let held = grown.saturating_add(piece.len());
        let reserved = if held >= una_core::INT_TEXT_RESERVED_FROM {
            Some(crate::runtime::try_reserve(held)?)
        } else {
            None
        };
        out.reserve_exact(grown - out.len());
        drop(reserved);
    }
    out.push_str(piece);
    Some(())
}

/// The decimal text of the `Int` `word`, once the run has room for it: a large one first
/// reserves the bytes its text and the conversion's temporaries hold, known from its bit length
/// in O(1), against the memory budget (`spec/11 §7`, #1223). `None` when they do not fit: the
/// caller aborts the run with U6003, and no conversion starts, so a write never passes the
/// allocator's hard ceiling.
pub(crate) fn int_written(word: i64) -> Option<String> {
    written_within(&values::int_value(word), |n| n.to_string())
}

/// The text of a `Decimal` of coefficient `c` at `scale`, once the run has room for it, as for
/// an `Int` of that coefficient ([`int_written`]).
pub(crate) fn decimal_written(c: &una_core::bigint::BigInt, scale: u8) -> Option<String> {
    written_within(c, |c| una_core::decimal::render(c, scale))
}

/// `write(n)`, the text of `n` or of a value with `n` as coefficient, with the bytes of
/// [`una_core::int_text_room`] reserved while it converts.
fn written_within(
    n: &una_core::bigint::BigInt,
    write: impl FnOnce(&una_core::bigint::BigInt) -> String,
) -> Option<String> {
    let reserved = match una_core::int_text_room(n) {
        Some(bytes) => Some(crate::runtime::try_reserve(bytes)?),
        None => None,
    };
    let text = write(n);
    drop(reserved);
    Some(text)
}

fn field_word(word: i64, index: usize) -> i64 {
    values::field(word, i64::try_from(index).unwrap_or(i64::MAX))
}

/// The value at `key` of `value` as an index.
pub(crate) fn index(value: &Value, key: &str) -> Option<usize> {
    usize::try_from(value.get(key)?.as_u64()?).ok()
}

fn shape(value: &Value) -> Option<Shape> {
    Some(match value.get("shape")?.as_str()? {
        "int" => Shape::Int,
        "decimal" => Shape::Decimal(
            u8::try_from(index(value, "scale")?)
                .ok()
                .filter(|s| *s <= una_core::decimal::MAX_SCALE)?,
        ),
        "bool" => Shape::Bool,
        "text" => Shape::Text,
        "unit" => Shape::Unit,
        "hashed" => Shape::Hashed,
        "reply-right" => Shape::Right,
        "f64" => Shape::F64,
        "f32" => Shape::F32,
        "fn" => Shape::Fn,
        "list" => Shape::List(index(value, "item")?),
        "map" => Shape::Map(index(value, "key")?, index(value, "value")?),
        "dense" => Shape::Dense(index(value, "element")?),
        "seq" => Shape::Seq(index(value, "item")?),
        "set" => Shape::Set(index(value, "item")?),
        "bits" => Shape::Bits,
        "hashmap" => Shape::HashMap(index(value, "key")?, index(value, "value")?),
        "hashset" => Shape::HashSet(index(value, "item")?),
        "param" => Shape::Param(index(value, "index")?),
        "data" => Shape::Data(
            value
                .get("variants")?
                .as_array()?
                .iter()
                .map(|variant| {
                    Some(Variant {
                        server: value
                            .get("server")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
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

    #[test]
    fn a_failure_shows_its_public_fields_and_redacts_labeled_ones_inside() {
        // kind Stop = | Halt(reason: Text, code: secret Int, inner: Option[Pair])
        // kind Pair: name: Text, pin: secret Int
        let description = serde_json::json!([
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
        ]);
        let types = Types::parse(&description).unwrap();
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
            types.shown(0, halt),
            r#"Halt(reason: "maintenance", inner: Some(Pair(name: "ana", pin: <redacted:secret>)))"#
        );
        assert_eq!(
            types.render(0, halt).unwrap(),
            r#"Halt(reason: "maintenance", code: <redacted:secret>, inner: Some(Pair(name: "ana", pin: <redacted:secret>)))"#
        );
        assert_eq!(
            types.redacted(1, 0, Some("internal")).unwrap(),
            "<redacted:internal>"
        );
        let none = values::data(0);
        values::push(none, values::text_word("x".to_owned()));
        values::push(none, 1);
        values::push(none, 3);
        assert_eq!(types.shown(0, none), r#"Halt(reason: "x", inner: None)"#);
        assert_eq!(types.variant_name(0, none), "Halt");
    }

    #[test]
    fn a_value_deeper_than_any_call_renders_without_recursion() {
        // `Some(Some(... None))`, a million deep (ADR 0060, draft).
        let types = Types(vec![
            Shape::Data(vec![
                Variant {
                    name: "Some".to_owned(),
                    fields: vec![Field {
                        name: "value".to_owned(),
                        ty: 0,
                        redacted: None,
                        shown: false,
                    }],
                    server: None,
                },
                Variant {
                    name: "None".to_owned(),
                    fields: Vec::new(),
                    server: None,
                },
            ]),
            Shape::List(0),
        ]);
        // Under Miri (#766), which runs each step slowly, the same walk at a smaller depth.
        let depth = if cfg!(miri) { 1_000 } else { 1_000_000 };
        let mut value = (1 << 1) | 1;
        for _ in 0..depth {
            let cell = values::data(0);
            values::push(cell, value);
            values::release(value);
            value = cell;
        }
        let list = values::list();
        values::push(list, value);
        let rendered = types.render(1, list).unwrap();
        assert_eq!(
            rendered.len(),
            "[]".len() + depth * "Some()".len() + "None".len()
        );
        let innermost = "[".len() + depth * "Some(".len();
        assert_eq!(
            rendered.get(innermost - 5..innermost + 5),
            Some("Some(None)")
        );
        assert!(rendered.ends_with(")))]"));
        values::release(value);
        values::release(list);
    }
}
