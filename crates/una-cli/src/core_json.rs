//! `una.core/1` (ADR 0083, `spec/13 §8`): the Core IR of a package as one JSON document, for
//! programs outside the compiler, the formal track's checker first (#46).
//!
//! The encoding is mechanical. A value of an enum of `una_core::ir` is an object whose first key
//! is `kind`, the variant's name in snake case, then the variant's fields under their Rust names
//! in declaration order; a tuple variant's one field is `value`, its several fields `values`. A
//! struct is an object of its fields in declaration order; a variant's own field named `kind` is
//! written `kind_of`, since `kind` names the variant. A tuple is an array, an `Option` its
//! value or `null`, a `Box` its content. Labels, spans and paths have the forms of `spec/13 §8`.
//! Every `match` below is exhaustive, so a new form of Core does not compile until it is encoded,
//! and every `kind` comes from the variant's own name through `stringify!`.

use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use una_check::Header;
use una_core::{
    Arm, Atom, Base, BinOp, Bound, CapOp, Clause, Constraint, Ctor, DataVariant, DecimalOp, Decl,
    Example, Expectation, Expr, FixedOp, Fun, HandlerOp, IntType, Label, LoopSource, MapOp, Module,
    Path, Prim, Program, Refinement, Rhs, Signature, Source, Term, Type, Use,
};
use una_project::Edition;
use una_syntax::{SourceDb, Span};

/// The schema of the document (ADR 0083).
pub(crate) const SCHEMA: &str = "una.core/1";

/// A JSON value whose objects keep their keys in the order they are written.
pub(crate) enum J {
    Null,
    Bool(bool),
    Num(u64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(&'static str, J)>),
}

impl Serialize for J {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            J::Null => serializer.serialize_unit(),
            J::Bool(b) => serializer.serialize_bool(*b),
            J::Num(n) => serializer.serialize_u64(*n),
            J::Str(s) => serializer.serialize_str(s),
            J::Arr(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            J::Obj(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

/// `CamelCase` as `snake_case`: the `kind` of a variant.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A node: `kind` from the variant's identifier, then its fields.
macro_rules! node {
    ($variant:ident $(, $key:literal => $value:expr)* $(,)?) => {
        J::Obj(vec![("kind", J::Str(snake(stringify!($variant)))) $(, ($key, $value))*])
    };
}

fn s(text: &str) -> J {
    J::Str(text.to_owned())
}

fn n(value: usize) -> J {
    J::Num(u64::try_from(value).unwrap_or(u64::MAX))
}

fn list<T>(items: &[T], f: impl Fn(&T) -> J) -> J {
    J::Arr(items.iter().map(f).collect())
}

fn opt<T>(value: Option<&T>, f: impl Fn(&T) -> J) -> J {
    value.map_or(J::Null, f)
}

fn strings(items: &[String]) -> J {
    list(items, |item| s(item))
}

/// The writer of one document: the sources, for the spans.
struct Writer<'s> {
    sources: &'s SourceDb,
    /// The headers of the package's functions after inference, for the facts Core does not
    /// hold: `of` clauses and result summaries (#443).
    headers: &'s [Header],
    /// The lambdas given to `of` parameters, by span: whether each performs effects and whether
    /// it can fail (ADR 0062, #443).
    of_lambdas: &'s BTreeMap<Span, (bool, bool)>,
}

impl Writer<'_> {
    fn span(&self, span: Span) -> J {
        let location = self.sources.location(span);
        J::Obj(vec![
            ("file", J::Str(location.file)),
            (
                "start",
                J::Arr(vec![
                    n(location.start.line as usize),
                    n(location.start.col as usize),
                ]),
            ),
            (
                "end",
                J::Arr(vec![
                    n(location.end.line as usize),
                    n(location.end.col as usize),
                ]),
            ),
        ])
    }

    fn spans(&self, spans: &[Span]) -> J {
        list(spans, |span| self.span(*span))
    }

    fn label(&self, label: &Label) -> J {
        J::Obj(vec![
            ("level", s(level(label))),
            (
                "categories",
                J::Arr(label.categories.iter().map(|c| s(c)).collect()),
            ),
            (
                "symbols",
                J::Arr(label.symbols.iter().map(|c| s(c)).collect()),
            ),
        ])
    }

    fn path(&self, path: &Path) -> J {
        J::Obj(vec![("module", s(&path.module)), ("name", s(&path.name))])
    }

    fn ty(&self, ty: &Type) -> J {
        match ty {
            Type::Base {
                base,
                refinements,
                label,
            } => node!(Base,
                "base" => self.base(base),
                "refinements" => list(refinements, |r| self.refinement(r)),
                "label" => self.label(label),
            ),
            Type::Fn {
                params,
                result,
                uses,
                fails,
                label,
            } => node!(Fn,
                "params" => list(params, |p| self.ty(p)),
                "result" => self.ty(result),
                "uses" => list(uses, |u| self.use_(u)),
                "fails" => opt(fails.as_ref(), |f| s(f)),
                "label" => self.label(label),
            ),
        }
    }

    fn binder(&self, (name, ty): &(String, Type)) -> J {
        J::Arr(vec![s(name), self.ty(ty)])
    }

    fn binders(&self, binders: &[(String, Type)]) -> J {
        list(binders, |b| self.binder(b))
    }

    fn int_type(&self, int: IntType) -> J {
        match int {
            IntType::I8 => node!(I8),
            IntType::I16 => node!(I16),
            IntType::I32 => node!(I32),
            IntType::I64 => node!(I64),
            IntType::U8 => node!(U8),
            IntType::U16 => node!(U16),
            IntType::U32 => node!(U32),
            IntType::U64 => node!(U64),
        }
    }

    fn base(&self, base: &Base) -> J {
        match base {
            Base::Int => node!(Int),
            Base::Fixed(int) => node!(Fixed, "value" => self.int_type(*int)),
            Base::Bool => node!(Bool),
            Base::Text => node!(Text),
            Base::Unit => node!(Unit),
            Base::Hashed => node!(Hashed),
            Base::ReplyRight => node!(ReplyRight),
            // ADR 0114 rule 17: the kind `f64`.
            Base::F64 => J::Obj(vec![("kind", s("f64"))]),
            // ADR 0120: the kind `decimal`, with its scale.
            Base::Decimal(scale) => node!(Decimal, "scale" => n(usize::from(*scale))),
            // ADR 0120 rule 5: the kind `f32`.
            Base::F32 => J::Obj(vec![("kind", s("f32"))]),
            Base::Named { module, name, args } => node!(Named,
                "module" => s(module),
                "name" => s(name),
                "args" => list(args, |a| self.ty(a)),
            ),
            Base::Param(name) => node!(Param, "value" => s(name)),
            Base::Tuple(items) => node!(Tuple, "value" => list(items, |t| self.ty(t))),
            Base::Unknown => node!(Unknown),
        }
    }

    fn use_(&self, u: &Use) -> J {
        let mut fields = vec![
            ("cap", s(&u.cap)),
            ("capability", s(&u.capability)),
            ("ops", strings(&u.ops)),
        ];
        // ADR 0094 rule 8 (#1518): an additive key, present only when an operation of the
        // binding gives a label, declared or inferred, so a document without one is unchanged.
        // It may only raise what a consumer reads: a consumer that does not model it reports
        // the declaration as not re-checked (`spec/13 §8`).
        if !u.gives.is_empty() {
            fields.push((
                "gives",
                list(&u.gives, |(op, label)| {
                    J::Obj(vec![("op", s(op)), ("label", self.label(label))])
                }),
            ));
        }
        // ADR 0092: an additive key, present only when a condition is written (ADR 0076
        // rule 1), so a document without one is unchanged.
        if !u.conds.is_empty() {
            fields.push((
                "conds",
                list(&u.conds, |(op, clause)| {
                    J::Obj(vec![("op", s(op)), ("cond", self.clause(clause))])
                }),
            ));
        }
        J::Obj(fields)
    }

    fn ctor(&self, ctor: &Ctor) -> J {
        J::Obj(vec![
            ("kind", self.path(&ctor.kind)),
            ("name", s(&ctor.name)),
            ("index", n(ctor.index)),
        ])
    }

    fn term(&self, term: &Term) -> J {
        match term {
            Term::Var(name) => node!(Var, "value" => s(name)),
            Term::It => node!(It),
            Term::Result => node!(Result),
            Term::Int(digits) => node!(Int, "value" => s(digits)),
            Term::Bool(b) => node!(Bool, "value" => J::Bool(*b)),
            Term::Text(text) => node!(Text, "value" => s(text)),
            Term::Unit => node!(Unit),
            Term::Ctor(ctor) => node!(Ctor, "value" => self.ctor(ctor)),
            Term::Field {
                value,
                kind,
                field,
                index,
            } => node!(Field,
                "value" => self.term(value),
                "kind_of" => self.path(kind),
                "field" => s(field),
                "index" => n(*index),
            ),
            Term::Len(value) => node!(Len, "value" => self.term(value)),
            Term::Has { map, key } => node!(Has, "map" => self.term(map), "key" => self.term(key)),
            // ADR 0119 V5: membership of a set and the size of a bit set.
            Term::Member { set, item } => {
                node!(Member, "set" => self.term(set), "item" => self.term(item))
            }
            Term::Size(value) => node!(Size, "value" => self.term(value)),
            // ADR 0120 rule 33: the shape functions and the elements of a dense value.
            Term::Shape { fun, value } => {
                node!(Shape, "fun" => s(fun.as_str()), "value" => self.term(value))
            }
            Term::Element {
                value,
                indices,
                tensor,
            } => node!(Element,
                "value" => self.term(value),
                "indices" => J::Arr(indices.iter().map(|index| self.term(index)).collect()),
                "tensor" => J::Bool(*tensor),
            ),
            Term::Dim { value, axis } => {
                node!(Dim, "value" => self.term(value), "axis" => self.term(axis))
            }
            Term::Proj { value, index } => {
                node!(Proj, "value" => self.term(value), "index" => n(*index))
            }
            Term::Neg(value) => node!(Neg, "value" => self.term(value)),
            Term::Not(value) => node!(Not, "value" => self.term(value)),
            Term::Binary { op, left, right } => node!(Binary,
                "op" => self.bin_op(*op),
                "left" => self.term(left),
                "right" => self.term(right),
            ),
            Term::Is { value, ctor } => {
                node!(Is, "value" => self.term(value), "ctor" => self.ctor(ctor))
            }
            Term::In {
                value,
                low,
                high,
                exclusive,
            } => node!(In,
                "value" => self.term(value),
                "low" => s(low),
                "high" => s(high),
                "exclusive" => J::Bool(*exclusive),
            ),
            Term::Call { predicate, args } => node!(Call,
                "predicate" => self.path(predicate),
                "args" => list(args, |a| self.term(a)),
            ),
            // ADR 0114 rule 17: a float literal by its bits, an operation by its `FloatOp`.
            Term::F64(bits) => J::Obj(vec![
                ("kind", s("f64")),
                ("bits", J::Str(format!("{bits:016x}"))),
            ]),
            Term::Float { op, args } => node!(Float,
                "op" => J::Obj(vec![(
                    "kind",
                    s(op.as_str().strip_prefix("f64_").unwrap_or(op.as_str()))
                )]),
                "args" => list(args, |a| self.term(a)),
            ),
            Term::ToF64 { value, from } => node!(ToF64,
                "value" => self.term(value),
                "from" => opt(from.as_ref(), |i| self.int_type(*i)),
            ),
            // ADR 0120 rule 5: `F32` by its bits as 8 lowercase hex digits, its operations
            // under `float32`.
            Term::F32(bits) => J::Obj(vec![
                ("kind", s("f32")),
                ("bits", J::Str(format!("{bits:08x}"))),
            ]),
            Term::Float32 { op, args } => node!(Float32,
                "op" => J::Obj(vec![(
                    "kind",
                    s(op.as_str().strip_prefix("f64_").unwrap_or(op.as_str()))
                )]),
                "args" => list(args, |a| self.term(a)),
            ),
            Term::ToF32 { value, from } => node!(ToF32,
                "value" => self.term(value),
                "from" => opt(from.as_ref(), |i| self.int_type(*i)),
            ),
            // ADR 0120 V3: `complex(re:, im:)` with the record constructor of its kind.
            Term::Complex { ctor, re, im } => node!(Complex,
                "ctor" => self.ctor(ctor),
                "re" => self.term(re),
                "im" => self.term(im),
            ),
        }
    }

    fn bin_op(&self, op: BinOp) -> J {
        match op {
            BinOp::Add => node!(Add),
            BinOp::Sub => node!(Sub),
            BinOp::Mul => node!(Mul),
            BinOp::Div => node!(Div),
            BinOp::Mod => node!(Mod),
            BinOp::Eq => node!(Eq),
            BinOp::Ne => node!(Ne),
            BinOp::Lt => node!(Lt),
            BinOp::Le => node!(Le),
            BinOp::Gt => node!(Gt),
            BinOp::Ge => node!(Ge),
            BinOp::And => node!(And),
            BinOp::Or => node!(Or),
            BinOp::Implies => node!(Implies),
        }
    }

    fn clause(&self, clause: &Clause) -> J {
        J::Obj(vec![
            ("term", self.term(&clause.term)),
            ("text", s(&clause.text)),
            ("span", opt(clause.span.as_ref(), |span| self.span(*span))),
        ])
    }

    fn clauses(&self, clauses: &[Clause]) -> J {
        list(clauses, |c| self.clause(c))
    }

    fn refinement(&self, refinement: &Refinement) -> J {
        J::Obj(vec![
            ("name", s(&refinement.name)),
            ("clause", self.clause(&refinement.clause)),
        ])
    }

    fn atom(&self, atom: &Atom) -> J {
        match atom {
            Atom::Var(name) => node!(Var, "value" => s(name)),
            Atom::Int(digits) => node!(Int, "value" => s(digits)),
            // ADR 0114 rule 17: its bits as 16 lowercase hex digits.
            Atom::F64(bits) => J::Obj(vec![
                ("kind", s("f64")),
                ("bits", J::Str(format!("{bits:016x}"))),
            ]),
            // ADR 0120: a decimal literal, its coefficient and its scale.
            Atom::Decimal { coefficient, scale } => node!(Decimal,
                "coefficient" => s(coefficient),
                "scale" => n(usize::from(*scale)),
            ),
            Atom::F32(bits) => J::Obj(vec![
                ("kind", s("f32")),
                ("bits", J::Str(format!("{bits:08x}"))),
            ]),
            Atom::Bool(b) => node!(Bool, "value" => J::Bool(*b)),
            Atom::Text(text) => node!(Text, "value" => s(text)),
            Atom::Unit => node!(Unit),
        }
    }

    fn atoms(&self, atoms: &[Atom]) -> J {
        list(atoms, |a| self.atom(a))
    }

    fn prim(&self, prim: &Prim) -> J {
        match prim {
            Prim::Add => node!(Add),
            Prim::Sub => node!(Sub),
            Prim::Mul => node!(Mul),
            Prim::Div => node!(Div),
            Prim::Mod => node!(Mod),
            Prim::Neg => node!(Neg),
            Prim::Eq => node!(Eq),
            Prim::Ne => node!(Ne),
            Prim::Lt => node!(Lt),
            Prim::Le => node!(Le),
            Prim::Gt => node!(Gt),
            Prim::Ge => node!(Ge),
            Prim::Not => node!(Not),
            Prim::Len => node!(Len),
            Prim::Concat => node!(Concat),
            Prim::ToText => node!(ToText),
            Prim::Convert(int) => {
                node!(Convert, "value" => opt(int.as_ref(), |i| self.int_type(*i)))
            }
            Prim::Fixed(op, int) => node!(Fixed,
                "values" => J::Arr(vec![self.fixed_op(*op), self.int_type(*int)]),
            ),
            Prim::Map(op) => node!(Map, "value" => self.map_op(*op)),
            // ADR 0119 rules 5 and 6: the operations of the sequence that holds a `Stack` or a
            // `Queue`, by their Core names.
            Prim::Seq(op) => node!(Seq, "value" => J::Obj(vec![("kind", s(op.as_str()))])),
            // ADR 0119 rule 7: the methods of `Set`, by their source names.
            Prim::Set(op) => node!(Set, "value" => J::Obj(vec![("kind", s(op.as_str()))])),
            // ADR 0119 rule 12: the methods of `HashMap` and `HashSet`, by their source names.
            Prim::HashMap(op) => {
                node!(HashMap, "value" => J::Obj(vec![("kind", s(op.as_str()))]))
            }
            Prim::HashSet(op) => {
                node!(HashSet, "value" => J::Obj(vec![("kind", s(op.as_str()))]))
            }
            // ADR 0119 rule 13: the operations of the heap that holds a `PriorityQueue`, by
            // their Core names.
            Prim::Priority(op) => {
                node!(Priority, "value" => J::Obj(vec![("kind", s(op.as_str()))]))
            }
            // ADR 0119 rule 14: the methods and the property `size` of `BitSet`, by their
            // source names.
            Prim::BitSet(op) => node!(BitSet, "value" => J::Obj(vec![("kind", s(op.as_str()))])),
            // ADR 0114 rule 17: the operations of `F64` are kinds of their own, under `float`.
            Prim::Float(op) => node!(Float, "value" => J::Obj(vec![(
                "kind",
                s(op.as_str().strip_prefix("f64_").unwrap_or(op.as_str()))
            )])),
            Prim::Shape(fun) => node!(Shape, "fun" => s(fun.as_str())),
            // ADR 0120: an operation of `Decimal`, its `DecimalOp` with its static scales.
            Prim::Decimal(op) => node!(Decimal, "value" => self.decimal_op(*op)),
            // ADR 0120 rule 5: the operations of `F32`, under `float32`, by the same names.
            Prim::Float32(op) => node!(Float32, "value" => J::Obj(vec![(
                "kind",
                s(op.as_str().strip_prefix("f64_").unwrap_or(op.as_str()))
            )])),
        }
    }

    fn decimal_op(&self, op: DecimalOp) -> J {
        let kind = match op {
            DecimalOp::Add(..) => "add",
            DecimalOp::Sub(..) => "sub",
            DecimalOp::Mul => "mul",
            DecimalOp::Neg => "neg",
            DecimalOp::Eq(..) => "eq",
            DecimalOp::Ne(..) => "ne",
            DecimalOp::Lt(..) => "lt",
            DecimalOp::Le(..) => "le",
            DecimalOp::Gt(..) => "gt",
            DecimalOp::Ge(..) => "ge",
            DecimalOp::Widen(..) => "widen",
            DecimalOp::Rescale(..) => "rescale",
            DecimalOp::RescaleExact(..) => "rescale_exact",
            DecimalOp::Divide(..) => "divide",
            DecimalOp::Remainder(..) => "remainder",
            DecimalOp::ToInt(_) => "to_int",
            DecimalOp::Abs => "abs",
            DecimalOp::Coefficient => "coefficient",
            DecimalOp::FromInt => "from_int",
            DecimalOp::FromF64(_) => "from_f64",
            DecimalOp::ToF64(_) => "to_f64",
            DecimalOp::ToText(_) => "to_text",
            DecimalOp::Parse(_) => "parse",
            DecimalOp::FromJson(_) => "from_json",
            DecimalOp::ToJson(_) => "to_json",
        };
        J::Obj(vec![
            ("kind", s(kind)),
            (
                "scales",
                J::Arr(op.scales().into_iter().map(|x| n(usize::from(x))).collect()),
            ),
        ])
    }

    fn map_op(&self, op: MapOp) -> J {
        match op {
            MapOp::Has => node!(Has),
            MapOp::Get => node!(Get),
            MapOp::With => node!(With),
            MapOp::Without => node!(Without),
            MapOp::Keys => node!(Keys),
            MapOp::Values => node!(Values),
            MapOp::Entries => node!(Entries),
        }
    }

    fn fixed_op(&self, op: FixedOp) -> J {
        match op {
            FixedOp::WrappingAdd => node!(WrappingAdd),
            FixedOp::WrappingSub => node!(WrappingSub),
            FixedOp::WrappingMul => node!(WrappingMul),
            FixedOp::SaturatingAdd => node!(SaturatingAdd),
            FixedOp::SaturatingSub => node!(SaturatingSub),
            FixedOp::SaturatingMul => node!(SaturatingMul),
            FixedOp::BitAnd => node!(BitAnd),
            FixedOp::BitOr => node!(BitOr),
            FixedOp::BitXor => node!(BitXor),
            FixedOp::BitNot => node!(BitNot),
            FixedOp::ShiftRight => node!(ShiftRight),
            FixedOp::WrappingShiftLeft => node!(WrappingShiftLeft),
        }
    }

    fn rhs(&self, rhs: &Rhs) -> J {
        match rhs {
            Rhs::Atom(atom) => node!(Atom, "value" => self.atom(atom)),
            Rhs::Tuple(atoms) => node!(Tuple, "value" => self.atoms(atoms)),
            Rhs::Prim {
                prim,
                args,
                arg_spans,
                span,
            } => node!(Prim,
                "prim" => self.prim(prim),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "span" => self.span(*span),
            ),
            Rhs::Call {
                function,
                type_args,
                args,
                arg_spans,
                span,
            } => node!(Call,
                "function" => self.path(function),
                "type_args" => list(type_args, |t| self.ty(t)),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "span" => self.span(*span),
            ),
            Rhs::Apply {
                callee,
                args,
                arg_spans,
                span,
            } => node!(Apply,
                "callee" => self.atom(callee),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "span" => self.span(*span),
            ),
            Rhs::Op {
                cap,
                capability,
                capability_path,
                op,
                args,
                arg_spans,
                labels,
                outcome,
                span,
            } => node!(Op,
                "cap" => s(cap),
                "capability" => s(capability),
                "capability_path" => self.path(capability_path),
                "op" => s(op),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "labels" => list(labels, |l| self.label(l)),
                "outcome" => self.label(outcome),
                "span" => self.span(*span),
            ),
            Rhs::Construct {
                ctor,
                args,
                arg_spans,
            } => node!(Construct,
                "ctor" => self.ctor(ctor),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
            ),
            Rhs::Field {
                value,
                field,
                index,
            } => node!(Field,
                "value" => self.atom(value),
                "field" => s(field),
                "index" => n(*index),
            ),
            Rhs::List(atoms) => node!(List, "value" => self.atoms(atoms)),
            Rhs::Lambda {
                params,
                body,
                function,
            } => node!(Lambda,
                "params" => self.binders(params),
                "body" => self.expr(body),
                "function" => opt(function.as_ref(), |p| self.path(p)),
            ),
            Rhs::Checked {
                value,
                it,
                predicate,
                formula,
                ctor,
                args,
                arg_spans,
                span,
            } => node!(Checked,
                "value" => self.atom(value),
                "it" => s(it),
                "predicate" => self.expr(predicate),
                "formula" => self.term(formula),
                "ctor" => self.ctor(ctor),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "span" => self.span(*span),
            ),
            Rhs::Declassify {
                policy,
                via,
                arg,
                arg_span,
                span,
            } => node!(Declassify,
                "policy" => self.path(policy),
                "via" => self.path(via),
                "arg" => self.atom(arg),
                "arg_span" => self.span(*arg_span),
                "span" => self.span(*span),
            ),
        }
    }

    fn bound(&self, bound: &Bound) -> J {
        match bound {
            Bound::Rhs(rhs) => node!(Rhs, "value" => self.rhs(rhs)),
            Bound::Expr(expr) => node!(Expr, "value" => self.expr(expr)),
        }
    }

    fn arm(&self, arm: &Arm) -> J {
        J::Obj(vec![
            ("ctor", self.ctor(&arm.ctor)),
            (
                "binds",
                list(&arm.binds, |b| opt(b.as_ref(), |name| s(name))),
            ),
            ("body", self.expr(&arm.body)),
        ])
    }

    fn source(&self, source: &Source) -> J {
        match source {
            Source::Rebind(name) => node!(Rebind, "value" => s(name)),
            Source::Handler {
                handler,
                args,
                arg_spans,
                given,
                ..
            } => node!(Handler,
                "handler" => self.path(handler),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "given" => J::Bool(*given),
            ),
        }
    }

    fn loop_source(&self, source: &LoopSource) -> J {
        match source {
            LoopSource::List(atom) => node!(List, "value" => self.atom(atom)),
            LoopSource::Range {
                low,
                high,
                inclusive,
            } => node!(Range,
                "low" => self.atom(low),
                "high" => self.atom(high),
                "inclusive" => J::Bool(*inclusive),
            ),
            LoopSource::Until {
                cond,
                holds,
                measure,
                decreases,
            } => node!(Until,
                "cond" => self.expr(cond),
                "holds" => opt(holds.as_ref(), |c| self.clause(c)),
                "measure" => self.clause(measure),
                "decreases" => self.span(*decreases),
            ),
            LoopSource::Ever { within: None } => node!(Ever),
            LoopSource::Ever {
                within: Some((deadline, span)),
            } => node!(Ever,
                "within" => self.atom(deadline),
                "within_span" => self.span(*span),
            ),
        }
    }

    /// The node of `expr`. A chain of `let`s, which is what makes Core IR deep (a statement
    /// or an intermediate value is one `let` whose body is the rest), is walked in a loop and
    /// its nodes are folded from the innermost body out, so the stack this takes does not grow
    /// with the chain (#2339): the frame of [`Writer::expr_node`] is about 10 KiB in an
    /// unoptimized build, which a recursion per `let` would take at every level.
    fn expr(&self, expr: &Expr) -> J {
        let mut chain = Vec::new();
        let mut inner = expr;
        while let Expr::Let { body, .. } = inner {
            chain.push(inner);
            inner = body;
        }
        let mut node = self.expr_node(inner);
        for outer in chain.into_iter().rev() {
            node = self.let_node(outer, node);
        }
        node
    }

    /// The node of a `let`, whose body's node is `body`; any other expression unchanged.
    fn let_node(&self, expr: &Expr, body: J) -> J {
        let Expr::Let {
            name,
            ty,
            value,
            span,
            written,
            ..
        } = expr
        else {
            return body;
        };
        let mut node = node!(Let,
            "name" => s(name),
            "ty" => self.ty(ty),
            "value" => self.bound(value),
            "body" => body,
            "span" => self.span(*span),
            "written" => J::Bool(*written),
        );
        // A lambda given to an `of` parameter has a function type without its effects,
        // which are its caller's (ADR 0062): the let that binds it says what they are.
        if let (Bound::Rhs(Rhs::Lambda { .. }), Some(&(effects, fails)), J::Obj(fields)) =
            (value.as_ref(), self.of_lambdas.get(span), &mut node)
        {
            fields.push((
                "of_lambda",
                J::Obj(vec![
                    ("effects", J::Bool(effects)),
                    ("fails", J::Bool(fails)),
                ]),
            ));
        }
        node
    }

    /// The node of `expr`, by its kind; [`Writer::expr`] gives a `let` its node.
    fn expr_node(&self, expr: &Expr) -> J {
        match expr {
            Expr::Atom { atom, span } => {
                node!(Atom, "atom" => self.atom(atom), "span" => self.span(*span))
            }
            Expr::Let { body, .. } => {
                let body = self.expr(body);
                self.let_node(expr, body)
            }
            Expr::LetTuple {
                names,
                value,
                body,
                span,
            } => node!(LetTuple,
                "names" => self.binders(names),
                "value" => self.atom(value),
                "body" => self.expr(body),
                "span" => self.span(*span),
            ),
            Expr::Loop {
                source,
                var,
                state,
                init,
                body,
                on_stop,
                span,
            } => {
                let mut node = node!(Loop,
                    "source" => self.loop_source(source),
                    "var" => self.binder(var),
                    "state" => self.binders(state),
                    "init" => self.atoms(init),
                    "body" => self.expr(body),
                    "span" => self.span(*span),
                );
                // The `on stop` of a `for ever`, an additive key (ADR 0116 rule 20).
                if let (Some(on_stop), J::Obj(fields)) = (on_stop, &mut node) {
                    fields.push((
                        "on_stop",
                        J::Obj(vec![
                            ("state", self.binders(&on_stop.state)),
                            ("body", self.expr(&on_stop.body)),
                        ]),
                    ));
                }
                node
            }
            Expr::Together { tasks, span } => node!(Together,
                "tasks" => list(tasks, |t| self.expr(t)),
                "span" => self.span(*span),
            ),
            Expr::FirstOf { tasks, span } => node!(FirstOf,
                "tasks" => list(tasks, |t| self.expr(t)),
                "span" => self.span(*span),
            ),
            Expr::Tasks { bound, body, span } => node!(Tasks,
                "bound" => opt(bound.as_ref(), |(b, _)| self.atom(b)),
                "bound_span" => opt(bound.as_ref(), |(_, at)| self.span(*at)),
                "body" => self.expr(body),
                "span" => self.span(*span),
            ),
            Expr::Start { task, rights, span } => node!(Start,
                "task" => self.expr(task),
                "rights" => list(rights, |name| s(name)),
                "span" => self.span(*span),
            ),
            Expr::Quiet { body, timing, span } => node!(Quiet,
                "body" => self.expr(body),
                "timing" => J::Bool(*timing),
                "span" => self.span(*span),
            ),
            Expr::TogetherFor {
                source,
                var,
                task,
                span,
            } => node!(TogetherFor,
                "source" => self.loop_source(source),
                "var" => self.binder(var),
                "task" => self.expr(task),
                "span" => self.span(*span),
            ),
            Expr::If {
                cond,
                then,
                otherwise,
            } => node!(If,
                "cond" => self.atom(cond),
                "then" => self.expr(then),
                "otherwise" => self.expr(otherwise),
            ),
            Expr::Case {
                scrutinee,
                arms,
                otherwise,
            } => node!(Case,
                "scrutinee" => self.atom(scrutinee),
                "arms" => list(arms, |a| self.arm(a)),
                "otherwise" => opt(otherwise.as_ref(), |e| self.expr(e)),
            ),
            Expr::Fail {
                ctor,
                args,
                arg_spans,
                span,
            } => node!(Fail,
                "ctor" => self.ctor(ctor),
                "args" => self.atoms(args),
                "arg_spans" => self.spans(arg_spans),
                "span" => self.span(*span),
            ),
            Expr::Exit { tag, state, span } => node!(Exit,
                "tag" => s(tag.as_str()),
                "state" => self.atoms(state),
                "span" => self.span(*span),
            ),
            Expr::Handle {
                body,
                error,
                handler,
            } => node!(Handle,
                "body" => self.expr(body),
                "error" => s(error),
                "handler" => self.expr(handler),
            ),
            Expr::Attempt { body, span } => {
                node!(Attempt, "body" => self.expr(body), "span" => self.span(*span))
            }
            Expr::Within {
                deadline,
                deadline_span,
                body,
                timeout,
                wrap,
                span,
            } => node!(Within,
                "deadline" => self.atom(deadline),
                "deadline_span" => self.span(*deadline_span),
                "body" => self.expr(body),
                "timeout" => self.ctor(timeout),
                "wrap" => opt(wrap.as_deref(), |c| self.ctor(c)),
                "span" => self.span(*span),
            ),
            Expr::With { cap, source, body } => node!(With,
                "cap" => s(cap),
                "source" => self.source(source),
                "body" => self.expr(body),
            ),
            Expr::Unreachable { span, goal } => {
                node!(Unreachable, "span" => self.span(*span), "goal" => s(goal))
            }
        }
    }

    fn constraint(&self, constraint: Constraint) -> J {
        match constraint {
            Constraint::None => node!(None),
            Constraint::Eq => node!(Eq),
            Constraint::Ord => node!(Ord),
            Constraint::Integer => node!(Integer),
        }
    }

    fn constraints(&self, constraints: &[Constraint]) -> J {
        list(constraints, |c| self.constraint(*c))
    }

    fn data_variant(&self, variant: &DataVariant) -> J {
        J::Obj(vec![
            ("name", s(&variant.name)),
            ("fields", self.binders(&variant.fields)),
        ])
    }

    fn cap_op(&self, op: &CapOp) -> J {
        J::Obj(vec![
            ("name", s(&op.name)),
            ("params", self.binders(&op.params)),
            ("result", self.ty(&op.result)),
            ("fails", opt(op.fails.as_ref(), |f| s(f))),
            ("error", opt(op.error.as_deref(), |p| self.path(p))),
            ("requires", self.clauses(&op.requires)),
            ("ensures", self.clauses(&op.ensures)),
            ("accepts", opt(op.accepts.as_ref(), |l| self.label(l))),
        ])
    }

    fn handler_op(&self, op: &HandlerOp) -> J {
        J::Obj(vec![
            ("name", s(&op.name)),
            ("params", strings(&op.params)),
            ("body", self.expr(&op.body)),
        ])
    }

    fn fun(&self, fun: &Fun) -> J {
        J::Obj(vec![
            ("name", s(&fun.name)),
            ("type_params", strings(&fun.type_params)),
            ("constraints", self.constraints(&fun.constraints)),
            ("params", self.binders(&fun.params)),
            ("result", self.ty(&fun.result)),
            ("uses", list(&fun.uses, |u| self.use_(u))),
            ("fails", opt(fun.fails.as_ref(), |f| s(f))),
            ("error", opt(fun.error.as_deref(), |p| self.path(p))),
            ("requires", self.clauses(&fun.requires)),
            ("ensures", self.clauses(&fun.ensures)),
            ("body", self.expr(&fun.body)),
            ("span", self.span(fun.span)),
            ("public", J::Bool(fun.public)),
        ])
    }

    /// A function of the module `module`, with what its header adds after inference (#443):
    /// its `of` clauses, and its result summary and `of` raises (ADR 0062, ADR 0067).
    fn fun_in(&self, module: &str, fun: &Fun) -> J {
        let mut node = self.fun(fun);
        let header = self
            .headers
            .iter()
            .find(|h| h.module == module && h.name == fun.name);
        if let (Some(header), J::Obj(fields)) = (header, &mut node) {
            let pairs = |items: &[(String, Vec<String>)], key: &'static str| {
                list(items, |(name, from)| {
                    J::Obj(vec![(key, s(name)), ("from", strings(from))])
                })
            };
            fields.push(("uses_of", strings(&header.uses_of)));
            fields.push(("fails_of", opt(header.fails_of.as_ref(), |f| s(f))));
            fields.push(("result_labels", pairs(&header.result_labels, "position")));
            fields.push(("of_raises", pairs(&header.of_raises, "param")));
        }
        node
    }

    fn signature(&self, signature: &Signature) -> J {
        J::Obj(vec![
            ("path", self.path(&signature.path)),
            ("type_params", strings(&signature.type_params)),
            ("constraints", self.constraints(&signature.constraints)),
            ("params", self.binders(&signature.params)),
            ("result", self.ty(&signature.result)),
            ("requires", self.clauses(&signature.requires)),
            ("ensures", self.clauses(&signature.ensures)),
        ])
    }

    fn decl(&self, decl: &Decl) -> J {
        match decl {
            Decl::Alias {
                name,
                params,
                target,
            } => node!(Alias,
                "name" => s(name),
                "params" => strings(params),
                "target" => self.ty(target),
            ),
            Decl::Data {
                name,
                params,
                variants,
                record,
            } => node!(Data,
                "name" => s(name),
                "params" => strings(params),
                "variants" => list(variants, |v| self.data_variant(v)),
                "record" => J::Bool(*record),
            ),
            Decl::Pred {
                name,
                reflect,
                params,
                body,
                formula,
                span,
            } => node!(Pred,
                "name" => s(name),
                "reflect" => J::Bool(*reflect),
                "params" => self.binders(params),
                "body" => self.expr(body),
                "formula" => opt(formula.as_ref(), |t| self.term(t)),
                "span" => self.span(*span),
            ),
            Decl::Cap { name, ops } => node!(Cap,
                "name" => s(name),
                "ops" => list(ops, |o| self.cap_op(o)),
            ),
            Decl::Handler {
                name,
                params,
                capability,
                capability_path,
                uses,
                ops,
            } => node!(Handler,
                "name" => s(name),
                "params" => self.binders(params),
                "capability" => s(capability),
                "capability_path" => self.path(capability_path),
                "uses" => list(uses, |u| self.use_(u)),
                "ops" => list(ops, |o| self.handler_op(o)),
            ),
            Decl::Fun(fun) => node!(Fun, "value" => self.fun(fun)),
            Decl::Policy {
                name,
                from,
                to,
                via,
            } => node!(Policy,
                "name" => s(name),
                "from" => self.ty(from),
                "to" => self.ty(to),
                "via" => self.path(via),
            ),
        }
    }

    fn expectation(&self, expectation: &Expectation) -> J {
        match expectation {
            Expectation::Holds {
                result,
                check,
                text,
                line,
            } => node!(Holds,
                "result" => s(result),
                "check" => self.expr(check),
                "text" => s(text),
                "line" => J::Num(u64::from(*line)),
            ),
            Expectation::Fails {
                variant,
                text,
                line,
            } => node!(Fails,
                "variant" => s(variant),
                "text" => s(text),
                "line" => J::Num(u64::from(*line)),
            ),
            Expectation::Performed {
                cap,
                op,
                negated,
                text,
                line,
            } => node!(Performed,
                "cap" => s(cap),
                "op" => s(op),
                "negated" => J::Bool(*negated),
                "text" => s(text),
                "line" => J::Num(u64::from(*line)),
            ),
        }
    }

    fn example(&self, example: &Example) -> J {
        let mut fields = vec![
            ("name", s(&example.name)),
            ("body", self.expr(&example.body)),
            (
                "expectations",
                list(&example.expectations, |e| self.expectation(e)),
            ),
        ];
        // Only an example with a `stop after` line has the key (ADR 0116 rule 19), so the
        // export of every other example is unchanged.
        if let Some(iterations) = example.stop_after {
            fields.push(("stop_after", J::Num(iterations)));
        }
        J::Obj(fields)
    }

    /// A declaration of the module `module`: a function with what its header adds (`fun_in`).
    /// `modules` and `rejected` (ADR 0091) both write declarations this way, so a rejected
    /// function carries the same keys as an accepted one (#514).
    fn decl_in(&self, module: &str, decl: &Decl) -> J {
        match decl {
            Decl::Fun(fun) => node!(Fun, "value" => self.fun_in(module, fun)),
            d => self.decl(d),
        }
    }

    fn module(&self, module: &Module) -> J {
        J::Obj(vec![
            ("path", s(&module.path)),
            (
                "decls",
                list(&module.decls, |d| self.decl_in(&module.path, d)),
            ),
            ("examples", list(&module.examples, |e| self.example(e))),
        ])
    }
}

/// The name of a label's level, as `una.toml` and `spec/07 §2` write it.
fn level(label: &Label) -> &'static str {
    use una_core::Level;
    match label.level {
        Level::Public => "public",
        Level::Internal => "internal",
        Level::Confidential => "confidential",
        Level::Secret => "secret",
        Level::Restricted => "restricted",
    }
}

/// A location as a span is written (`spec/13 §8` rule 5).
fn location(location: &una_diag::Location) -> J {
    J::Obj(vec![
        ("file", J::Str(location.file.clone())),
        (
            "start",
            J::Arr(vec![
                n(location.start.line as usize),
                n(location.start.col as usize),
            ]),
        ),
        (
            "end",
            J::Arr(vec![
                n(location.end.line as usize),
                n(location.end.col as usize),
            ]),
        ),
    ])
}

/// The `una.core/1` document of `program` (ADR 0083): `complete` when the package checked clean,
/// so that Core holds every declaration. `file_state` is the package's file-state label
/// (ADR 0085), an additive key (#467); the header's `profile` is the one the program records
/// (ADR 0109 rule 6).
/// `rejected`, given by `--with-rejected` only, adds the declarations rejected for information
/// flow, apart from Core (ADR 0091).
// Each argument is one part of the document, in its key order (ADR 0083, 0085, 0091).
#[allow(clippy::too_many_arguments)]
pub(crate) fn document(
    program: &Program,
    sources: &SourceDb,
    headers: &[Header],
    of_lambdas: &[(Span, bool, bool)],
    file_state: &Label,
    edition: Edition,
    complete: bool,
    rejected: Option<&[(una_check::Rejected, Vec<&una_diag::Diagnostic>)]>,
) -> Result<String, serde_json::Error> {
    let of_lambdas: BTreeMap<Span, (bool, bool)> = of_lambdas
        .iter()
        .map(|&(span, effects, fails)| (span, (effects, fails)))
        .collect();
    let writer = Writer {
        sources,
        headers,
        of_lambdas: &of_lambdas,
    };
    let mut keys = vec![
        ("schema", s(SCHEMA)),
        ("edition", s(edition.name())),
        // ADR 0109 rule 6: the profile, as the program records it, an additive key under ADR
        // 0076.
        ("profile", s(program.profile.name())),
        ("complete", J::Bool(complete)),
        ("modules", list(&program.modules, |m| writer.module(m))),
        (
            "library",
            list(&program.library, |sig| writer.signature(sig)),
        ),
        ("host", list(&program.host, |d| writer.decl(d))),
        ("file_state", writer.label(file_state)),
    ];
    // ADR 0091: the declarations rejected for information flow, apart from Core, only with
    // `--with-rejected`; without it the document is unchanged.
    if let Some(rejected) = rejected {
        keys.push((
            "rejected",
            list(rejected, |(r, diagnostics)| {
                J::Obj(vec![
                    ("module", s(&r.module)),
                    ("decl", writer.decl_in(&r.module, &r.decl)),
                    (
                        "diagnostics",
                        list(diagnostics, |d| {
                            J::Obj(vec![
                                ("code", s(d.code.as_str())),
                                ("span", location(&d.primary)),
                            ])
                        }),
                    ),
                ])
            }),
        ));
    }
    serde_json::to_string(&J::Obj(keys))
}

/// The `kind`s of each enum of `una_core::ir`, as the source declares its variants: what
/// `spec/13 §8` lists and the tests compare (ADR 0083). Parsed from the source text of
/// `ir.rs`, so a variant added there shows here without a change to this function.
#[cfg(test)]
fn declared_kinds(source: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut kinds = std::collections::BTreeMap::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("pub enum ") {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            current = Some((name, Vec::new()));
            continue;
        }
        if line == "}" {
            if let Some((name, variants)) = current.take() {
                kinds.insert(name, variants);
            }
            continue;
        }
        if let Some((_, variants)) = current.as_mut()
            && let Some(rest) = line.strip_prefix("    ")
            && rest.starts_with(|c: char| c.is_ascii_uppercase())
        {
            let variant: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            variants.push(snake(&variant));
        }
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::{declared_kinds, snake};

    #[test]
    fn a_variant_s_kind_is_its_name_in_snake_case() {
        assert_eq!(snake("LetTuple"), "let_tuple");
        assert_eq!(snake("TogetherFor"), "together_for");
        assert_eq!(snake("WrappingShiftLeft"), "wrapping_shift_left");
        assert_eq!(snake("I8"), "i8");
        assert_eq!(snake("U64"), "u64");
    }

    #[test]
    fn spec_13_lists_every_kind_of_core_ir() {
        // ADR 0083: `spec/13 §8` lists, for each enum of `una_core::ir`, the kinds of its
        // variants. A variant added to Core without its row there fails here.
        let declared = declared_kinds(include_str!("../../una-core/src/ir.rs"));
        let spec = include_str!("../../../docs/spec/13-diagnostics-and-abi.md");
        let section = spec
            .split("## 8. Core export")
            .nth(1)
            .expect("spec/13 has §8")
            .split("\n## ")
            .next()
            .unwrap_or_default();
        let mut listed = std::collections::BTreeMap::new();
        for line in section.lines() {
            // `| Expr | `atom`, `let`, ... |`
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() == 4 && cells[1].starts_with('`') && !cells[1].contains("kind") {
                let name = cells[1].trim_matches('`').to_owned();
                let kinds: Vec<String> = cells[2]
                    .split(',')
                    .map(|k| k.trim().trim_matches('`').to_owned())
                    .filter(|k| !k.is_empty())
                    .collect();
                listed.insert(name, kinds);
            }
        }
        // Enums that are not nodes of Core: the prelude's field types (#415), and what an
        // operation of `F64` returns.
        let mut declared = declared;
        declared.remove("PreludeField");
        declared.remove("FloatResult");
        assert_eq!(listed, declared);
    }
}
