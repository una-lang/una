//! The types that a native executable renders, as `una-rt`'s `render` reads them: the failure
//! of `main` (U6001, `spec/04 §8`), and the arguments, results and failures of the operations
//! a trace records (`spec/11 §5`). A word does not say its type, so each one is described: its
//! shape, with the variants of a kind, the names and types of their fields, and the label of
//! each field's declared type, which redacts it as the interpreter does.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use una_core::{Base, Decl, Label, Level, PRELUDE_MODULE, Program, Type};

/// The types described so far, by index, and the index of each by its name.
pub(crate) struct Types<'p> {
    program: &'p Program,
    index: BTreeMap<String, usize>,
    shapes: Vec<Value>,
    /// Whether a kind had no declaration to describe.
    unknown: bool,
}

impl<'p> Types<'p> {
    /// No type described yet.
    pub(crate) fn new(program: &'p Program) -> Types<'p> {
        Types {
            program,
            index: BTreeMap::new(),
            shapes: Vec::new(),
            unknown: false,
        }
    }

    /// The descriptions, by index.
    pub(crate) fn shapes(&self) -> &[Value] {
        &self.shapes
    }

    /// Whether a kind had no declaration to describe: its values render as `?`.
    pub(crate) fn unknown(&self) -> bool {
        self.unknown
    }

    /// The index of `ty`, described once; a kind that holds itself refers to its own index.
    pub(crate) fn of(&mut self, ty: &Type) -> usize {
        let key = key(ty);
        if let Some(index) = self.index.get(&key) {
            return *index;
        }
        let index = self.shapes.len();
        self.index.insert(key, index);
        self.shapes.push(Value::Null);
        let shape = self.shape(ty);
        if let Some(slot) = self.shapes.get_mut(index) {
            *slot = shape;
        }
        index
    }

    fn shape(&mut self, ty: &Type) -> Value {
        let Type::Base { base, .. } = ty else {
            return json!({ "shape": "fn" });
        };
        match base {
            Base::Int | Base::Fixed(_) => json!({ "shape": "int" }),
            Base::Bool => json!({ "shape": "bool" }),
            Base::Text => json!({ "shape": "text" }),
            Base::Unit => json!({ "shape": "unit" }),
            Base::Hashed => json!({ "shape": "hashed" }),
            // Internal to the memory model: a tuple is never a value a trace or U6001 shows.
            Base::Tuple(_) => json!({ "shape": "other" }),
            Base::Named { module, name, args } if module == PRELUDE_MODULE => {
                let arg = |i: usize| args.get(i).cloned().unwrap_or_else(unknown);
                match name.as_str() {
                    "List" => {
                        let item = self.of(&arg(0));
                        json!({ "shape": "list", "item": item })
                    }
                    "Map" => {
                        let (key, value) = (self.of(&arg(0)), self.of(&arg(1)));
                        json!({ "shape": "map", "key": key, "value": value })
                    }
                    "Entry" => {
                        let (key, value) = (self.of(&arg(0)), self.of(&arg(1)));
                        data(vec![variant(
                            "Entry",
                            vec![
                                field("key", key, None, false),
                                field("value", value, None, false),
                            ],
                        )])
                    }
                    // The fields of the prelude's variants have no declared type that the
                    // interpreter reads: they are never redacted, and U6001 never shows them.
                    "Option" => {
                        let value = self.of(&arg(0));
                        data(vec![
                            variant("Some", vec![field("value", value, None, false)]),
                            variant("None", Vec::new()),
                        ])
                    }
                    "Result" => {
                        let (value, failure) = (self.of(&arg(0)), self.of(&arg(1)));
                        data(vec![
                            variant("Ok", vec![field("value", value, None, false)]),
                            variant("Err", vec![field("failure", failure, None, false)]),
                        ])
                    }
                    "FileError" => data(
                        ["NotFound", "Denied", "Io"]
                            .into_iter()
                            .map(|name| variant(name, Vec::new()))
                            .collect(),
                    ),
                    "KeyError" => data(vec![variant("UnknownKey", Vec::new())]),
                    // JSON (`spec/23 §1`): `Json` holds itself through its arrays and objects,
                    // which refer to its own index.
                    "Json" => {
                        let named = |name: &str, args: Vec<Type>| Type::Base {
                            base: Base::Named {
                                module: PRELUDE_MODULE.to_owned(),
                                name: name.to_owned(),
                                args,
                            },
                            refinements: Vec::new(),
                            label: Label::public(),
                        };
                        let base = |base: Base| Type::Base {
                            base,
                            refinements: Vec::new(),
                            label: Label::public(),
                        };
                        let json = named("Json", Vec::new());
                        let (boolean, int, text) = (
                            self.of(&base(Base::Bool)),
                            self.of(&base(Base::Int)),
                            self.of(&base(Base::Text)),
                        );
                        let items = self.of(&named("List", vec![json.clone()]));
                        let fields = self.of(&named("Map", vec![base(Base::Text), json]));
                        data(vec![
                            variant("Null", Vec::new()),
                            variant("Boolean", vec![field("value", boolean, None, false)]),
                            variant("Number", vec![field("value", int, None, false)]),
                            variant("Decimal", vec![field("text", text, None, false)]),
                            variant("Str", vec![field("value", text, None, false)]),
                            variant("Array", vec![field("items", items, None, false)]),
                            variant("Object", vec![field("fields", fields, None, false)]),
                        ])
                    }
                    "JsonError" => {
                        let int = self.of(&Type::Base {
                            base: Base::Int,
                            refinements: Vec::new(),
                            label: Label::public(),
                        });
                        data(
                            ["Syntax", "TooDeep", "DuplicateKey", "NumberTooLong"]
                                .into_iter()
                                .map(|name| variant(name, vec![field("at", int, None, false)]))
                                .collect(),
                        )
                    }
                    _ => {
                        self.unknown = true;
                        json!({ "shape": "other" })
                    }
                }
            }
            Base::Named { module, name, args } => {
                let declared = self
                    .program
                    .modules
                    .iter()
                    .filter(|m| m.path == *module)
                    .flat_map(|m| &m.decls)
                    .find_map(|decl| match decl {
                        Decl::Data {
                            name: declared,
                            params,
                            variants,
                            ..
                        } if declared == name => Some((params, variants)),
                        _ => None,
                    });
                let Some((params, variants)) = declared else {
                    self.unknown = true;
                    return json!({ "shape": "other" });
                };
                let variants = variants
                    .iter()
                    .map(|v| {
                        let fields = v
                            .fields
                            .iter()
                            .map(|(name, declared)| {
                                let label = deep_label(declared);
                                let ty = self.of(&substitute(declared, params, args));
                                let redacted = (!label.is_public()).then(|| level(label.level));
                                let shown = !has_param(declared) && label.is_public();
                                field(name, ty, redacted, shown)
                            })
                            .collect();
                        variant(&v.name, fields)
                    })
                    .collect();
                data(variants)
            }
            // A slot of a composite descriptor, `#<position>` (ADR 0068 D6).
            Base::Param(name) if let Some(position) = slot(name) => {
                json!({ "shape": "param", "index": position })
            }
            Base::Param(_) | Base::Unknown => json!({ "shape": "other" }),
        }
    }
}

fn data(variants: Vec<Value>) -> Value {
    json!({ "shape": "data", "variants": variants })
}

fn variant(name: &str, fields: Vec<Value>) -> Value {
    json!({ "name": name, "fields": fields })
}

fn field(name: &str, ty: usize, redacted: Option<&str>, shown: bool) -> Value {
    json!({ "name": name, "type": ty, "redacted": redacted, "shown": shown })
}

fn unknown() -> Type {
    Type::Base {
        base: Base::Unknown,
        refinements: Vec::new(),
        label: Label::public(),
    }
}

/// The name of a type, without labels or refinements, which select its description.
fn key(ty: &Type) -> String {
    match ty {
        Type::Base { base, .. } => match base {
            Base::Named { module, name, args } => {
                let args: Vec<String> = args.iter().map(key).collect();
                format!("{module}.{name}[{}]", args.join(","))
            }
            Base::Fixed(int) => int.as_str().to_owned(),
            Base::Param(name) => format!("'{name}"),
            other => format!("{other:?}"),
        },
        Type::Fn { .. } => "fn".to_owned(),
    }
}

/// `ty` with each type parameter of `params` replaced by its argument in `args`.
fn substitute(ty: &Type, params: &[String], args: &[Type]) -> Type {
    match ty {
        Type::Base {
            base: Base::Param(name),
            ..
        } => params
            .iter()
            .position(|p| p == name)
            .and_then(|i| args.get(i))
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        Type::Base {
            base:
                Base::Named {
                    module,
                    name,
                    args: inner,
                },
            refinements,
            label,
        } => Type::Base {
            base: Base::Named {
                module: module.clone(),
                name: name.clone(),
                args: inner.iter().map(|t| substitute(t, params, args)).collect(),
            },
            refinements: refinements.clone(),
            label: label.clone(),
        },
        other => other.clone(),
    }
}

/// The join of the labels at every position of `ty` that holds data, as the interpreter reads a
/// field's declared type.
pub(crate) fn deep_label(ty: &Type) -> Label {
    match ty {
        Type::Base { base, label, .. } => match base {
            Base::Named { args, .. } => args
                .iter()
                .fold(label.clone(), |acc, arg| acc.join(&deep_label(arg))),
            _ => label.clone(),
        },
        Type::Fn { label, .. } => label.clone(),
    }
}

fn has_param(ty: &Type) -> bool {
    match ty {
        Type::Base { base, .. } => match base {
            Base::Param(_) | Base::Unknown => true,
            Base::Named { args, .. } => args.iter().any(has_param),
            _ => false,
        },
        Type::Fn { .. } => false,
    }
}

/// The name of a level, as `<redacted:level>` shows it.
pub(crate) fn level(level: Level) -> &'static str {
    match level {
        Level::Public => "public",
        Level::Internal => "internal",
        Level::Confidential => "confidential",
        Level::Secret => "secret",
        Level::Restricted => "restricted",
    }
}

/// The name of the type parameter at `position` of a composite descriptor: `#<position>`, which
/// no source name can be.
pub(crate) fn slot_name(position: usize) -> String {
    format!("#{position}")
}

/// The position of a composite descriptor's type parameter named by [`slot_name`].
fn slot(name: &str) -> Option<usize> {
    name.strip_prefix('#')?.parse().ok()
}
