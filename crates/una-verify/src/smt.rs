//! The SMT-LIB encoding of `spec/12 §4`: formulas of QF_UFLIA, the names of sorts and
//! symbols, and queries emitted as deterministic text.
//!
//! Namespaces (ADR 0038 D8): every symbol derived from user content starts with `u!` and every
//! symbol the encoder introduces with `i!`; only `Int`, `Bool` and the operators of SMT-LIB are
//! unprefixed. User names never contain `!`, and every symbol is built from a prefix and
//! `!`-separated parts, so the two namespaces cannot collide.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A sort of the solver: `Int`, `Bool`, or a declared sort named after a Una type.
pub(crate) type Sort = String;

/// The sorts that SMT-LIB defines, which queries never declare.
fn builtin(sort: &str) -> bool {
    matches!(sort, "Int" | "Bool" | FLOAT64 | FLOAT32)
}

/// The sort of `F64`: `(_ FloatingPoint 11 53)` of the FloatingPoint theory (`spec/29 §8`).
pub(crate) const FLOAT64: &str = "Float64";

/// The sort of `F32`: `(_ FloatingPoint 8 24)` (ADR 0120 V1).
pub(crate) const FLOAT32: &str = "Float32";

/// Whether `sort` is a float sort, of `F64` or `F32`.
pub(crate) fn is_float_sort(sort: &str) -> bool {
    sort == FLOAT64 || sort == FLOAT32
}

/// The rounded operators of the FloatingPoint theory, whose result has the sort of their
/// operand after the rounding mode.
const ROUNDED_OPS: [&str; 5] = ["fp.add", "fp.sub", "fp.mul", "fp.div", "fp.sqrt"];

/// The sort of `Text`.
pub(crate) const TEXT: &str = "i!Text";

/// The sort of `Unit`.
pub(crate) const UNIT: &str = "i!Unit";

/// The sort of `Hashed`.
pub(crate) const HASHED: &str = "i!Hashed";

/// The prefix of the sorts of `List[T]`.
pub(crate) const LIST: &str = "i!List_";
/// The prefix of the sorts of maps, `i!Map_<K>_<V>` (`spec/21 §4`).
pub(crate) const MAP: &str = "i!Map_";
/// The prefix of the sorts of arrays, `i!Array_<T>` (`spec/29 §17`, ADR 0120 V4).
pub(crate) const ARRAY: &str = "i!Array_";
/// The prefix of the sorts of matrices, `i!Matrix_<T>` (`spec/29 §17`, ADR 0120 V4).
pub(crate) const MATRIX: &str = "i!Matrix_";
/// The prefix of the sorts of tensors, `i!Tensor_<T>` (`spec/29 §17`, ADR 0120 V4).
pub(crate) const TENSOR: &str = "i!Tensor_";
/// The prefix of the sorts of LU factorizations, `i!Lu_<T>` (`spec/29 §17`, ADR 0120 V4).
pub(crate) const LU: &str = "i!Lu_";

/// A term or formula of the solver's logic.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum F {
    /// An integer literal, in decimal, with an optional `-`.
    Int(String),
    /// An `F64` literal, as its bits: `(fp #b<sign> #b<exponent> #b<significand>)`.
    Float(u64),
    /// An `F32` literal, as its bits (ADR 0120 V1).
    Float32(u32),
    /// `true` or `false`.
    Bool(bool),
    /// A declared constant.
    Const(Const),
    /// A variable bound by a definition, which is never declared.
    Local { name: String, sort: Sort },
    /// An uninterpreted function applied to its arguments.
    App { fun: Fun, args: Vec<F> },
    /// An operator of SMT-LIB applied to its arguments.
    Op { op: &'static str, args: Vec<F> },
}

/// A declared constant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Const {
    /// The symbol, without quotes.
    pub(crate) name: String,
    /// The sort.
    pub(crate) sort: Sort,
    /// What it stands for.
    pub(crate) role: ConstRole,
}

/// What a constant stands for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ConstRole {
    /// A binder of the program: a parameter, `result`, a `let`, a pattern variable or a
    /// temporary of Core IR, `u!v!<name>`.
    Plain,
    /// A binder the encoder introduces: a path selector or `unit`, `i!v!<name>`.
    Internal,
    /// A text literal, `i!lit!<name>` with its bytes in hex as the name: all of them are
    /// distinct (`spec/12 §4`).
    Text(String),
}

impl Const {
    /// The quoted symbol of the constant, in its namespace.
    pub(crate) fn symbol(&self) -> String {
        match self.role {
            ConstRole::Plain => user_binder(&self.name),
            ConstRole::Internal => format!("|i!v!{}|", self.name),
            ConstRole::Text(_) => format!("|i!lit!{}|", self.name),
        }
    }
}

/// The path selector `number` of a return obligation.
pub(crate) fn path_selector(number: usize) -> F {
    F::Const(Const {
        name: format!("path!{number}"),
        sort: "Bool".to_owned(),
        role: ConstRole::Internal,
    })
}

/// An uninterpreted function.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Fun {
    /// The symbol.
    pub(crate) name: String,
    /// The sorts of the arguments.
    pub(crate) args: Vec<Sort>,
    /// The sort of the result.
    pub(crate) result: Sort,
    /// What it stands for, for the ground facts of its terms.
    pub(crate) role: FunRole,
}

/// What an uninterpreted function stands for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FunRole {
    /// Field `index` of variant `variant` of the kind at `kind` (module, name), which has
    /// `variants` variants.
    Field {
        module: String,
        kind: String,
        variant: usize,
        index: usize,
        variants: usize,
    },
    /// `.len`: at least 0.
    Len,
    /// A shape function of a dense value beside `len`: `rows`, `columns`, `rank`, `count`,
    /// `size` and `dim`, at least 0, and a rank from 1 to 8 (ADR 0120 V4), or the `size` of a
    /// bit set (ADR 0119 V1): at least 0.
    Shape,
    /// The element of a dense value at its indices, `i!get!<Sort>`, uninterpreted (ADR 0120
    /// V4).
    Element,
    /// The variant of a value of the sum at `kind` (module, name): between 0 and `variants`.
    Tag {
        module: String,
        kind: String,
        variants: usize,
    },
    /// `m.has(key: k)` of a map: a map that has a key is not empty (`spec/21 §4`).
    Has,
    /// `m.get(key: k)` of a map, uninterpreted.
    Get,
    /// A read of a collection of ADR 0119 that gives an `Option` of an item, `i!<read>!<Sort>`,
    /// uninterpreted (V1): `peek` and `peekback` of a `Stack`, a `Queue` or a
    /// `PriorityQueue`, `first` and `last` of a `Set` or a `BitSet`. It is `None` exactly when
    /// the collection is empty (V2).
    Peek,
    /// Another function of a collection of ADR 0119 (V1), uninterpreted and without facts of
    /// its own: `below` of a `Stack`, the stack under its top, and `same` of `same_entries`
    /// and `same_items`. It shows as a call.
    Collection,
    /// The record that the collection `s'` is the collection `s` updated at the key `x` only,
    /// `i!frame!<Sort>(s, s', x)` (ADR 0119 V3), asserted where the update happens. For each
    /// term `y` of the key's sort in a query, it gives `y != x => has(s', y) = has(s, y)`, and
    /// for a map, whose `get` has the option sort `get`, the same of `get`. Every run makes it
    /// true, so each fact is an instance of a valid law, and no quantifier enters the query.
    Frame {
        /// The sort of `get`'s result, for a map; `None` for a set.
        get: Option<Sort>,
    },
    /// The record that a set is the union, intersection or difference of two others, or that
    /// one set is a subset of another (ADR 0119 V4), over the items of the sort `item`. Each
    /// term `y` of that sort in a query gives its membership law at `y`.
    Combine {
        /// Which relation.
        op: Combine,
        /// The sort of the items.
        item: Sort,
    },
    /// A predicate; a reflect one has a definition.
    Predicate,
    /// Any other function, without facts: nonlinear arithmetic, text ordering.
    Other,
}

/// A relation between sets of ADR 0119 V4, the role of `i!<op>!<Sort>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Combine {
    /// `union(s, t, r)`: `has(r, y) = has(s, y) or has(t, y)`.
    Union,
    /// `intersection(s, t, r)`: `has(r, y) = has(s, y) and has(t, y)`.
    Intersection,
    /// `difference(s, t, r)`: `has(r, y) = has(s, y) and not has(t, y)`.
    Difference,
    /// `subset(s, t)`: `has(s, y) => has(t, y)`.
    Subset,
}

impl Combine {
    /// The relation's name in its symbol.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Combine::Union => "union",
            Combine::Intersection => "intersection",
            Combine::Difference => "difference",
            Combine::Subset => "subset",
        }
    }
}

impl F {
    /// The sort of the term.
    pub(crate) fn sort(&self) -> Sort {
        match self {
            F::Int(_) => "Int".to_owned(),
            F::Float(_) => FLOAT64.to_owned(),
            F::Float32(_) => FLOAT32.to_owned(),
            F::Bool(_) => "Bool".to_owned(),
            F::Const(c) => c.sort.clone(),
            F::Local { sort, .. } => sort.clone(),
            F::App { fun, .. } => fun.result.clone(),
            F::Op { op, args } => match *op {
                "+" | "-" | "*" | "div" | "mod" => "Int".to_owned(),
                op if ROUNDED_OPS.contains(&op) => {
                    args.get(1).map_or_else(|| FLOAT64.to_owned(), F::sort)
                }
                "fp.neg" | "fp.abs" => args.first().map_or_else(|| FLOAT64.to_owned(), F::sort),
                "(_ to_fp 11 53)" | "(_ to_fp_unsigned 11 53)" => FLOAT64.to_owned(),
                "(_ to_fp 8 24)" | "(_ to_fp_unsigned 8 24)" => FLOAT32.to_owned(),
                // The rounding mode and the bit-vectors of a conversion have sorts of their
                // own, so nothing reads them as Booleans (the abstraction of `spec/29 §8`).
                "RNE" => "RoundingMode".to_owned(),
                op if op.starts_with("(_ int2bv ") => {
                    let width = op.trim_start_matches("(_ int2bv ").trim_end_matches(')');
                    format!("(_ BitVec {width})")
                }
                "ite" => args.get(1).map_or_else(|| "Bool".to_owned(), F::sort),
                _ => "Bool".to_owned(),
            },
        }
    }

    pub(crate) fn op(op: &'static str, args: Vec<F>) -> F {
        F::Op { op, args }
    }

    pub(crate) fn eq(left: F, right: F) -> F {
        F::op("=", vec![left, right])
    }

    pub(crate) fn not(f: F) -> F {
        match f {
            F::Bool(b) => F::Bool(!b),
            F::Op {
                op: "not",
                mut args,
            } if args.len() == 1 => args.pop().unwrap_or(F::Bool(false)),
            other => F::op("not", vec![other]),
        }
    }

    /// The conjunction of `parts`: `true` when there are none.
    pub(crate) fn and(parts: Vec<F>) -> F {
        match parts.len() {
            0 => F::Bool(true),
            1 => parts.into_iter().next().unwrap_or(F::Bool(true)),
            _ => F::op("and", parts),
        }
    }

    /// The disjunction of `parts`: `false` when there are none.
    pub(crate) fn or(parts: Vec<F>) -> F {
        match parts.len() {
            0 => F::Bool(false),
            1 => parts.into_iter().next().unwrap_or(F::Bool(false)),
            _ => F::op("or", parts),
        }
    }

    /// Every function application in the term, and every constant, depth first.
    pub(crate) fn visit<'a>(&'a self, out: &mut Vec<&'a F>) {
        match self {
            F::App { args, .. } | F::Op { args, .. } => {
                for arg in args {
                    arg.visit(out);
                }
                out.push(self);
            }
            F::Const(_) => out.push(self),
            F::Int(_) | F::Float(_) | F::Float32(_) | F::Bool(_) | F::Local { .. } => {}
        }
    }

    /// The SMT-LIB text of the term.
    pub(crate) fn render(&self, out: &mut String) {
        match self {
            F::Int(digits) => match digits.strip_prefix('-') {
                Some(positive) => {
                    let _ = write!(out, "(- {positive})");
                }
                None => out.push_str(digits),
            },
            F::Float(bits) => {
                let _ = write!(
                    out,
                    "(fp #b{:01b} #b{:011b} #b{:052b})",
                    bits >> 63,
                    (bits >> 52) & 0x7FF,
                    bits & ((1 << 52) - 1)
                );
            }
            F::Float32(bits) => {
                let _ = write!(
                    out,
                    "(fp #b{:01b} #b{:08b} #b{:023b})",
                    bits >> 31,
                    (bits >> 23) & 0xFF,
                    bits & ((1 << 23) - 1)
                );
            }
            F::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            F::Const(c) => out.push_str(&c.symbol()),
            F::Local { name, .. } => out.push_str(&user_binder(name)),
            F::App { fun, args } => {
                let _ = write!(out, "({}", fun.name);
                for arg in args {
                    out.push(' ');
                    arg.render(out);
                }
                out.push(')');
            }
            F::Op { op, args } => {
                if args.is_empty() {
                    out.push_str(match *op {
                        "and" => "true",
                        "or" => "false",
                        other => other,
                    });
                    return;
                }
                let _ = write!(out, "({op}");
                for arg in args {
                    out.push(' ');
                    arg.render(out);
                }
                out.push(')');
            }
        }
    }

    /// Whether the term has an `F64` term (`spec/29 §8`): a float literal, a constant or an
    /// application of the sort `Float64`, or an operator of the FloatingPoint theory.
    pub(crate) fn mentions_float(&self) -> bool {
        match self {
            F::Float(_) | F::Float32(_) => true,
            F::Int(_) | F::Bool(_) => false,
            F::Const(c) => is_float_sort(&c.sort),
            F::Local { sort, .. } => is_float_sort(sort),
            F::App { fun, args } => {
                is_float_sort(&fun.result)
                    || fun.args.iter().any(|sort| is_float_sort(sort))
                    || args.iter().any(F::mentions_float)
            }
            F::Op { op, args } => {
                op.starts_with("fp.") || op.contains("to_fp") || args.iter().any(F::mentions_float)
            }
        }
    }

    /// The SMT-LIB text of the term.
    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        self.render(&mut out);
        out
    }
}

/// The name of the sort of a kind declared in `module`: `u!Kind_<module>_<Name>`, each `_` of
/// a module segment doubled so that distinct paths give distinct names (`spec/12 §4`).
pub(crate) fn kind_sort(module: &str, name: &str, args: &[Sort]) -> Sort {
    let module: Vec<String> = module.split('.').map(|s| s.replace('_', "__")).collect();
    let mut sort = format!("u!Kind_{}_{name}", module.join("_"));
    for arg in args {
        sort.push('_');
        sort.push_str(arg);
    }
    sort
}

/// A reflect predicate's definition: its parameters and body.
#[derive(Clone, Debug)]
pub(crate) struct Definition {
    pub(crate) params: Vec<(String, Sort)>,
    pub(crate) body: F,
}

/// A query: the assertions of `Π ∧ ¬goal`, the definitions they use, and the terms whose
/// values a model shows.
#[derive(Clone, Debug, Default)]
pub(crate) struct Query {
    /// The assertions, in generation order.
    pub(crate) assertions: Vec<F>,
    /// The definitions of reflect predicates, by name.
    pub(crate) definitions: BTreeMap<String, Definition>,
}

impl Query {
    /// Whether the query has an `F64` term: it is then sent in the logic `ALL`, with the fixed
    /// step budget of the encoding (`spec/29 §8`, ADR 0114 rule 8.2).
    pub(crate) fn mentions_float(&self) -> bool {
        self.assertions.iter().any(F::mentions_float)
            || self
                .definitions
                .values()
                .any(|d| d.body.mentions_float() || d.params.iter().any(|(_, s)| is_float_sort(s)))
    }

    /// The declarations and assertions as SMT-LIB text: sorts, constants and functions sorted
    /// by name, then definitions in dependency order, then the assertions, with the
    /// distinctness of text literals first among them (`spec/12 §4`).
    pub(crate) fn text(&self) -> String {
        self.text_with(&[])
    }

    /// [`Query::text`] whose declarations also cover the symbols of `requested`, the terms a
    /// `(get-value ...)` asks for after the query: a requested term may apply a symbol that no
    /// assertion uses, such as the tag of a shown value of a sum kind, which the solver would
    /// otherwise not know (#855). Only declarations come from them, never an assertion.
    pub(crate) fn text_with(&self, requested: &[F]) -> String {
        let mut terms = Vec::new();
        for assertion in &self.assertions {
            assertion.visit(&mut terms);
        }
        for term in requested {
            term.visit(&mut terms);
        }
        for definition in self.definitions.values() {
            definition.body.visit(&mut terms);
        }
        let mut sorts: BTreeSet<Sort> = BTreeSet::new();
        // By symbol, which names the namespace too.
        let mut constants: BTreeMap<String, Sort> = BTreeMap::new();
        let mut functions: BTreeMap<String, (Vec<Sort>, Sort)> = BTreeMap::new();
        let mut texts: BTreeSet<String> = BTreeSet::new();
        for term in &terms {
            match term {
                F::Const(c) => {
                    sorts.insert(c.sort.clone());
                    constants.insert(c.symbol(), c.sort.clone());
                    if let ConstRole::Text(_) = c.role {
                        texts.insert(c.symbol());
                    }
                }
                F::App { fun, .. } => {
                    sorts.extend(fun.args.iter().cloned());
                    sorts.insert(fun.result.clone());
                    if !self.definitions.contains_key(&fun.name) {
                        functions.insert(fun.name.clone(), (fun.args.clone(), fun.result.clone()));
                    }
                }
                _ => {}
            }
        }
        for definition in self.definitions.values() {
            sorts.extend(definition.params.iter().map(|(_, sort)| sort.clone()));
        }
        let mut out = String::new();
        for sort in sorts.iter().filter(|sort| !builtin(sort)) {
            let _ = writeln!(out, "(declare-sort {sort} 0)");
        }
        for (symbol, sort) in &constants {
            let _ = writeln!(out, "(declare-fun {symbol} () {sort})");
        }
        for (name, (args, result)) in &functions {
            let _ = writeln!(out, "(declare-fun {name} ({}) {result})", args.join(" "));
        }
        for name in self.definition_order() {
            let Some(definition) = self.definitions.get(&name) else {
                continue;
            };
            let params: Vec<String> = definition
                .params
                .iter()
                .map(|(param, sort)| format!("({} {sort})", user_binder(param)))
                .collect();
            let _ = writeln!(
                out,
                "(define-fun {name} ({}) Bool {})",
                params.join(" "),
                definition.body.text()
            );
        }
        if texts.len() > 1 {
            let names: Vec<&str> = texts.iter().map(String::as_str).collect();
            let _ = writeln!(out, "(assert (distinct {}))", names.join(" "));
        }
        for assertion in self.assertions.iter().filter(|a| **a != F::Bool(true)) {
            let _ = writeln!(out, "(assert {})", assertion.text());
        }
        out
    }

    /// The definitions, each after those its body uses, ties by name.
    fn definition_order(&self) -> Vec<String> {
        let mut done: Vec<String> = Vec::new();
        let mut pending: Vec<&String> = self.definitions.keys().collect();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|name| {
                let uses: Vec<String> = self
                    .definitions
                    .get(*name)
                    .map(|definition| {
                        let mut terms = Vec::new();
                        definition.body.visit(&mut terms);
                        terms
                            .iter()
                            .filter_map(|term| match term {
                                F::App { fun, .. } if fun.name != **name => Some(fun.name.clone()),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let ready = uses
                    .iter()
                    .all(|used| !self.definitions.contains_key(used) || done.contains(used));
                if ready {
                    done.push((*name).clone());
                }
                !ready
            });
            if pending.len() == before {
                // Reflect predicates are not recursive (U2015); emit the rest by name.
                done.extend(pending.iter().map(|name| (*name).clone()));
                break;
            }
        }
        done
    }

    /// Every function application that the assertions contain, for ground facts.
    pub(crate) fn applications(&self) -> BTreeSet<F> {
        let mut terms = Vec::new();
        for assertion in &self.assertions {
            assertion.visit(&mut terms);
        }
        terms
            .into_iter()
            .filter(|term| matches!(term, F::App { .. } | F::Const(_)))
            .cloned()
            .collect()
    }
}

/// The quoted symbol of a binder of the program, `u!v!<name>`. The prefix also keeps the
/// names that Z3 5.1.0 rejects even quoted, `_`, `as`, `xor` and `distinct` (ADR 0033 rule 5),
/// from ever being a whole symbol.
fn user_binder(name: &str) -> String {
    format!("|u!v!{name}|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant(name: &str, sort: &str) -> F {
        F::Const(Const {
            name: name.to_owned(),
            sort: sort.to_owned(),
            role: ConstRole::Plain,
        })
    }

    #[test]
    fn requested_terms_are_declared_without_being_asserted() {
        // #855: a `(get-value ...)` term may apply a symbol that no assertion uses, such as
        // the tag of a shown value of a sum kind; the script declares it and asserts nothing.
        let measure = kind_sort("pantry.stock", "Measure", &[]);
        let tag = F::App {
            fun: Fun {
                name: format!("i!tag!{measure}"),
                args: vec![measure.clone()],
                result: "Int".to_owned(),
                role: FunRole::Other,
            },
            args: vec![constant("m", &measure)],
        };
        let query = Query {
            assertions: vec![F::op(
                "=",
                vec![constant("n", "Int"), F::Int("1".to_owned())],
            )],
            definitions: BTreeMap::new(),
        };
        let plain = query.text();
        assert!(!plain.contains("i!tag!"), "{plain}");
        let with = query.text_with(&[tag]);
        assert!(
            with.contains(&format!("(declare-sort {measure} 0)")),
            "{with}"
        );
        assert!(
            with.contains(&format!("(declare-fun |u!v!m| () {measure})")),
            "{with}"
        );
        assert!(
            with.contains(&format!("(declare-fun i!tag!{measure} ({measure}) Int)")),
            "{with}"
        );
        let asserted = |text: &str| text.lines().filter(|l| l.starts_with("(assert")).count();
        assert_eq!(asserted(&with), asserted(&plain));
        assert!(!with.contains("(assert (= (i!tag!"), "{with}");
    }

    #[test]
    fn binders_are_in_the_user_namespace() {
        // ADR 0038 D8; ADR 0033 rule 5: Z3 5.1.0 rejects these four as whole symbols, and the
        // prefix keeps them from being one.
        for name in ["_", "as", "xor", "distinct", "e", "%t0", "x%2"] {
            assert_eq!(user_binder(name), format!("|u!v!{name}|"));
        }
        assert_eq!(path_selector(1).text(), "|i!v!path!1|");
    }

    #[test]
    fn queries_declare_sorted_symbols_and_keep_assertions_in_order() {
        let event = kind_sort("events.seating", "Event", &[]);
        assert_eq!(event, "u!Kind_events_seating_Event");
        assert_eq!(kind_sort("a_b.c", "K", &[]), "u!Kind_a__b_c_K");
        assert_eq!(
            kind_sort("a.b_c", "K", &["Int".to_owned()]),
            "u!Kind_a_b__c_K_Int"
        );
        let remaining = F::App {
            fun: Fun {
                name: format!("u!f!remaining!{event}"),
                args: vec![event.clone()],
                result: "Int".to_owned(),
                role: FunRole::Other,
            },
            args: vec![constant("e", &event)],
        };
        let query = Query {
            assertions: vec![
                F::op("<=", vec![F::Int("0".to_owned()), remaining.clone()]),
                F::not(F::op("<", vec![remaining, F::Int("-1".to_owned())])),
            ],
            definitions: BTreeMap::new(),
        };
        assert_eq!(
            query.text(),
            "(declare-sort u!Kind_events_seating_Event 0)\n\
             (declare-fun |u!v!e| () u!Kind_events_seating_Event)\n\
             (declare-fun u!f!remaining!u!Kind_events_seating_Event (u!Kind_events_seating_Event) Int)\n\
             (assert (<= 0 (u!f!remaining!u!Kind_events_seating_Event |u!v!e|)))\n\
             (assert (not (< (u!f!remaining!u!Kind_events_seating_Event |u!v!e|) (- 1))))\n"
        );
    }
}
