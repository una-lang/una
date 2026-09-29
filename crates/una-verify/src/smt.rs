//! The SMT-LIB encoding of `spec/12 §4`: formulas of QF_UFLIA, the names of sorts and
//! symbols, and queries emitted as deterministic text (DECISION D-115).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A sort of the solver: `Int`, `Bool`, or a declared sort named after a Una type.
pub(crate) type Sort = String;

/// The sorts that SMT-LIB defines, which queries never declare.
fn builtin(sort: &str) -> bool {
    matches!(sort, "Int" | "Bool")
}

/// A term or formula of the solver's logic.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum F {
    /// An integer literal, in decimal, with an optional `-`.
    Int(String),
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
    /// A binder, `result` or a path selector.
    Plain,
    /// A text literal: all of them are distinct (`spec/12 §4`).
    Text(String),
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
    /// The variant of a value of the sum at `kind` (module, name): between 0 and `variants`.
    Tag {
        module: String,
        kind: String,
        variants: usize,
    },
    /// A predicate; a reflect one has a definition.
    Predicate,
    /// Any other function, without facts: nonlinear arithmetic, text ordering.
    Other,
}

impl F {
    /// The sort of the term.
    pub(crate) fn sort(&self) -> Sort {
        match self {
            F::Int(_) => "Int".to_owned(),
            F::Bool(_) => "Bool".to_owned(),
            F::Const(c) => c.sort.clone(),
            F::Local { sort, .. } => sort.clone(),
            F::App { fun, .. } => fun.result.clone(),
            F::Op { op, args } => match *op {
                "+" | "-" | "*" | "div" | "mod" => "Int".to_owned(),
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
    fn visit<'a>(&'a self, out: &mut Vec<&'a F>) {
        match self {
            F::App { args, .. } | F::Op { args, .. } => {
                for arg in args {
                    arg.visit(out);
                }
                out.push(self);
            }
            F::Const(_) => out.push(self),
            F::Int(_) | F::Bool(_) | F::Local { .. } => {}
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
            F::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            F::Const(c) => {
                let _ = write!(out, "|{}|", c.name);
            }
            F::Local { name, .. } => {
                let _ = write!(out, "|{name}|");
            }
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

    /// The SMT-LIB text of the term.
    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        self.render(&mut out);
        out
    }
}

/// The name of the sort of a kind declared in `module`: `Kind_<module>_<Name>`, each `_` of a
/// module segment doubled so that distinct paths give distinct names (`spec/12 §4`).
pub(crate) fn kind_sort(module: &str, name: &str, args: &[Sort]) -> Sort {
    let module: Vec<String> = module.split('.').map(|s| s.replace('_', "__")).collect();
    let mut sort = format!("Kind_{}_{name}", module.join("_"));
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
    /// The declarations and assertions as SMT-LIB text: sorts, constants and functions sorted
    /// by name, then definitions in dependency order, then the assertions, with the
    /// distinctness of text literals first among them (`spec/12 §4`).
    pub(crate) fn text(&self) -> String {
        let mut terms = Vec::new();
        for assertion in &self.assertions {
            assertion.visit(&mut terms);
        }
        for definition in self.definitions.values() {
            definition.body.visit(&mut terms);
        }
        let mut sorts: BTreeSet<Sort> = BTreeSet::new();
        let mut constants: BTreeMap<String, Sort> = BTreeMap::new();
        let mut functions: BTreeMap<String, (Vec<Sort>, Sort)> = BTreeMap::new();
        let mut texts: BTreeSet<String> = BTreeSet::new();
        for term in &terms {
            match term {
                F::Const(c) => {
                    sorts.insert(c.sort.clone());
                    constants.insert(c.name.clone(), c.sort.clone());
                    if let ConstRole::Text(_) = c.role {
                        texts.insert(c.name.clone());
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
        for (name, sort) in &constants {
            let _ = writeln!(out, "(declare-fun |{name}| () {sort})");
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
                .map(|(param, sort)| format!("(|{param}| {sort})"))
                .collect();
            let _ = writeln!(
                out,
                "(define-fun {name} ({}) Bool {})",
                params.join(" "),
                definition.body.text()
            );
        }
        if texts.len() > 1 {
            let names: Vec<String> = texts.iter().map(|name| format!("|{name}|")).collect();
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
    fn queries_declare_sorted_symbols_and_keep_assertions_in_order() {
        let event = kind_sort("events.seating", "Event", &[]);
        assert_eq!(event, "Kind_events_seating_Event");
        assert_eq!(kind_sort("a_b.c", "K", &[]), "Kind_a__b_c_K");
        assert_eq!(
            kind_sort("a.b_c", "K", &["Int".to_owned()]),
            "Kind_a_b__c_K_Int"
        );
        let remaining = F::App {
            fun: Fun {
                name: format!("remaining!{event}"),
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
            "(declare-sort Kind_events_seating_Event 0)\n\
             (declare-fun |e| () Kind_events_seating_Event)\n\
             (declare-fun remaining!Kind_events_seating_Event (Kind_events_seating_Event) Int)\n\
             (assert (<= 0 (remaining!Kind_events_seating_Event |e|)))\n\
             (assert (not (< (remaining!Kind_events_seating_Event |e|) (- 1))))\n"
        );
    }
}
