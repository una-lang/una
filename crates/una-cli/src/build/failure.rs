//! What a native executable needs to report a failure of `main` as `una run` does (U6001,
//! `spec/04 §8`): the types the failure can hold, as `una-rt`'s `render` reads them.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use una_core::{Base, Decl, Expr, Fun, Label, Level, PRELUDE_MODULE, Path, Program, Type};

/// The description of `main`'s error kind and the types it holds, with `location`, where `main`
/// is named: `None` when `main` cannot fail.
pub(super) fn describe(
    program: &Program,
    entry: &str,
    main: &Fun,
    location: Value,
) -> Option<Value> {
    let name = main.fails.as_ref()?;
    let kind = error_kind(program, entry, main, name);
    let mut types = Types {
        program,
        index: BTreeMap::new(),
        shapes: Vec::new(),
    };
    let root = types.of(&Type::Base {
        base: Base::Named {
            module: kind.module,
            name: kind.name,
            args: Vec::new(),
        },
        refinements: Vec::new(),
        label: Label::public(),
    });
    Some(json!({ "location": location, "root": root, "types": types.shapes }))
}

/// The declaration of `main`'s error kind, which Core names without its module: the kind of a
/// `fail` in `main`, else the kind of that name in the entry module, in another module, or in the
/// prelude.
fn error_kind(program: &Program, entry: &str, main: &Fun, name: &str) -> Path {
    if let Some(kind) = failed_kind(&main.body) {
        return kind;
    }
    let declares = |module: &&una_core::Module| {
        module
            .decls
            .iter()
            .any(|decl| matches!(decl, Decl::Data { name: declared, .. } if declared == name))
    };
    let module = program
        .modules
        .iter()
        .filter(|module| module.path == entry)
        .find(declares)
        .or_else(|| program.modules.iter().find(declares))
        .map_or_else(|| PRELUDE_MODULE.to_owned(), |module| module.path.clone());
    Path {
        module,
        name: name.to_owned(),
    }
}

/// The kind of the first `fail` of `expr`, outside handling clauses.
fn failed_kind(expr: &Expr) -> Option<Path> {
    match expr {
        Expr::Fail { ctor, .. } => Some(ctor.kind.clone()),
        Expr::Let { value, body, .. } => {
            let inner = match value.as_ref() {
                una_core::Bound::Expr(inner) => failed_kind(inner),
                una_core::Bound::Rhs(_) => None,
            };
            inner.or_else(|| failed_kind(body))
        }
        Expr::If {
            then, otherwise, ..
        } => failed_kind(then).or_else(|| failed_kind(otherwise)),
        Expr::Case {
            arms, otherwise, ..
        } => arms
            .iter()
            .find_map(|arm| failed_kind(&arm.body))
            .or_else(|| otherwise.as_deref().and_then(failed_kind)),
        Expr::Handle { handler, .. } => failed_kind(handler),
        Expr::With { body, .. } => failed_kind(body),
        Expr::Atom { .. } | Expr::Attempt { .. } | Expr::Unreachable { .. } => None,
    }
}

/// The types described so far, by index, and the index of each by its name.
struct Types<'p> {
    program: &'p Program,
    index: BTreeMap<String, usize>,
    shapes: Vec<Value>,
}

impl Types<'_> {
    /// The index of `ty`, described once; a kind that holds itself refers to its own index.
    fn of(&mut self, ty: &Type) -> usize {
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
            Base::Named { module, name, args } if module == PRELUDE_MODULE => {
                let arg = |i: usize| args.get(i).cloned().unwrap_or_else(unknown);
                match name.as_str() {
                    "List" => {
                        let item = self.of(&arg(0));
                        json!({ "shape": "list", "item": item })
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
                    _ => json!({ "shape": "other" }),
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
            Base::Hashed | Base::Param(_) | Base::Unknown => json!({ "shape": "other" }),
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
fn deep_label(ty: &Type) -> Label {
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

fn level(level: Level) -> &'static str {
    match level {
        Level::Public => "public",
        Level::Internal => "internal",
        Level::Confidential => "confidential",
        Level::Secret => "secret",
        Level::Restricted => "restricted",
    }
}
