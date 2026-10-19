//! The types that a native executable renders, as `una-rt`'s `render` reads them: the failure
//! of `main` (U6001, `spec/04 §8`), and the arguments, results and failures of the operations
//! a trace records (`spec/11 §5`). A word does not say its type, so each one is described: its
//! shape, with the variants of a kind, the names and types of their fields, and the label of
//! each field's declared type, which redacts it as the interpreter does.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use una_core::{
    Base, Decl, Label, Level, PRELUDE_MODULE, PreludeField, Program, Type, prelude_variants,
};

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
            // No `F64` value reaches native code yet (`spec/29`): an empty list of them may.
            Base::F64 => json!({ "shape": "other" }),
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
                    // The kinds that Core leaves to the engines, from the table by which the
                    // interpreter names their fields (#415): none of their fields declares a
                    // label, and U6001 never shows them. Each field is described in the
                    // table's order, which numbers the descriptions.
                    other if let Some(variants) = prelude_variants(other) => {
                        let mut described = Vec::new();
                        for (name, fields) in variants {
                            let mut shown = Vec::new();
                            for (field_name, of) in *fields {
                                let ty = match of {
                                    PreludeField::Arg(position) => self.of(&arg(*position)),
                                    PreludeField::Int => self.of(&Type::Base {
                                        base: Base::Int,
                                        refinements: Vec::new(),
                                        label: Label::public(),
                                    }),
                                };
                                shown.push(field(field_name, ty, None, false));
                            }
                            described.push(variant(name, shown));
                        }
                        data(described)
                    }
                    // The other kinds of the prelude that Core holds, as `Response`
                    // (`spec/24 §1`), by their declarations.
                    _ => match self.declared_shape(module, name, args) {
                        Some(shape) => shape,
                        None => {
                            self.unknown = true;
                            json!({ "shape": "other" })
                        }
                    },
                }
            }
            Base::Named { module, name, args } => match self.declared_shape(module, name, args) {
                Some(shape) => shape,
                None => {
                    self.unknown = true;
                    json!({ "shape": "other" })
                }
            },
            // A slot of a composite descriptor, `#<position>` (ADR 0068 D6).
            Base::Param(name) if let Some(position) = slot(name) => {
                json!({ "shape": "param", "index": position })
            }
            Base::Param(_) | Base::Unknown => json!({ "shape": "other" }),
        }
    }
    /// The shape of the kind `name` of `module` as the program declares it, with its type
    /// parameters replaced by `args`; `None` when the program declares no such kind.
    fn declared_shape(&mut self, module: &str, name: &str, args: &[Type]) -> Option<Value> {
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
        let (params, variants) = declared?;
        let variants = variants
            .iter()
            .map(|v| {
                let fields = v
                    .fields
                    .iter()
                    .map(|(name, declared)| {
                        let label = declared.deep_label();
                        let ty = self.of(&substitute(declared, params, args));
                        let redacted = (!label.is_public()).then(|| level(label.level));
                        let shown = !declared.has_param() && label.is_public();
                        field(name, ty, redacted, shown)
                    })
                    .collect();
                variant(&v.name, fields)
            })
            .collect();
        Some(data(variants))
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

#[cfg(test)]
mod tests {
    use una_core::{
        Base, Label, PRELUDE_ENGINE_KINDS, PRELUDE_MODULE, Program, Type, prelude_variants,
    };

    use super::Types;

    #[test]
    fn the_prelude_kinds_core_leaves_to_the_engines_are_described_by_the_shared_table() {
        // #415: each variant and field, in the table's order, none of them redacted, as the
        // interpreter names them from the same table.
        let program = Program::default();
        let int = Type::Base {
            base: Base::Int,
            refinements: Vec::new(),
            label: Label::public(),
        };
        for kind in PRELUDE_ENGINE_KINDS {
            let Some(variants) = prelude_variants(kind) else {
                continue;
            };
            let mut types = Types::new(&program);
            let root = types.of(&named(PRELUDE_MODULE, kind, vec![int.clone(), int.clone()]));
            let shape = &types.shapes()[root];
            assert_eq!(shape["shape"], "data", "{kind}");
            let described: Vec<(String, Vec<(String, bool)>)> = shape["variants"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    let fields = v["fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|f| {
                            (
                                f["name"].as_str().unwrap().to_owned(),
                                f["redacted"].is_null(),
                            )
                        })
                        .collect();
                    (v["name"].as_str().unwrap().to_owned(), fields)
                })
                .collect();
            let tabled: Vec<(String, Vec<(String, bool)>)> = variants
                .iter()
                .map(|(variant, fields)| {
                    let fields = fields
                        .iter()
                        .map(|(f, _)| ((*f).to_owned(), true))
                        .collect();
                    ((*variant).to_owned(), fields)
                })
                .collect();
            assert_eq!(described, tabled, "{kind}");
        }
    }

    fn named(module: &str, name: &str, args: Vec<Type>) -> Type {
        Type::Base {
            base: Base::Named {
                module: module.to_owned(),
                name: name.to_owned(),
                args,
            },
            refinements: Vec::new(),
            label: Label::public(),
        }
    }

    #[test]
    fn every_kind_core_leaves_to_the_engines_is_described_and_no_other() {
        // #407: the build stops when a described kind has no declaration, so each kind of the
        // prelude that Core does not declare must be described here.
        let program = Program::default();
        for kind in PRELUDE_ENGINE_KINDS {
            let mut types = Types::new(&program);
            let int = Type::Base {
                base: Base::Int,
                refinements: Vec::new(),
                label: Label::public(),
            };
            types.of(&named(PRELUDE_MODULE, kind, vec![int.clone(), int]));
            assert!(!types.unknown(), "`{kind}` is described");
        }
        for (module, kind) in [(PRELUDE_MODULE, "Gone"), ("app", "Gone")] {
            let mut types = Types::new(&program);
            types.of(&named(module, kind, Vec::new()));
            assert!(types.unknown(), "`{module}.{kind}` has no declaration");
        }
    }
}
