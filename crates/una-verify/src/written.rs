//! The goal text of a clause over decimals, as the source writes it (#1041, `spec/29 §16`
//! rule 11).
//!
//! In the refinement language a decimal is its coefficient (ADR 0120 V2): `it >= 0.00` of a
//! `Decimal[2]` is the term `it >= 0`, a literal is aligned by a power of ten, and
//! `x.coefficient()` is the term `x`. Printed from its term, such a goal shows coefficients
//! that the source never wrote, and its repairs are no valid code. The clause keeps its text
//! as written, so a goal over decimals is that text with each binder the obligation names
//! replaced, token by token; only the shown text changes, never the query.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use una_core::{Clause, Term, atomic, print_term_with};
use una_syntax::{Interner, Keyword, KeywordSet, SourceDb, TokenKind, lex_in};

/// The text of `clause` with the binders `replace` names replaced: its written text when the
/// clause has a decimal literal or `coefficient()`, else its term printed by
/// [`print_term_with`], as before (so every other goal, and its id, is unchanged).
pub(crate) fn shown(clause: &Clause, replace: &dyn Fn(&Term) -> Option<String>) -> String {
    written(clause, replace).unwrap_or_else(|| print_term_with(&clause.term, replace))
}

/// The written text of a clause over decimals with its binders replaced, or `None` when the
/// clause has no decimal, or when the text and the term do not agree on its binders. One
/// lexing pass and one walk of the term: O(n) in the clause's size.
fn written(clause: &Clause, replace: &dyn Fn(&Term) -> Option<String>) -> Option<String> {
    let text = clause.text.as_str();
    // The binders of the term by their source names, each with its replacement, and the float
    // literals of the term. A name with two different replacements is not decided by its text.
    let names: RefCell<BTreeMap<String, Option<String>>> = RefCell::new(BTreeMap::new());
    let ambiguous = Cell::new(false);
    let floats = Cell::new(0usize);
    print_term_with(&clause.term, &|term| {
        let name = match term {
            Term::It => "it",
            Term::Result => "result",
            // A renamed binder, `x%2`, is written `x`.
            Term::Var(name) => match name.split_once('%') {
                Some((source, _)) if !source.is_empty() => source,
                _ => name.as_str(),
            },
            Term::F64(_) | Term::F32(_) => {
                floats.set(floats.get() + 1);
                return None;
            }
            _ => return None,
        };
        let shown = replace(term);
        let mut names = names.borrow_mut();
        match names.get(name) {
            Some(previous) if *previous != shown => ambiguous.set(true),
            Some(_) => {}
            None => {
                names.insert(name.to_owned(), shown.clone());
            }
        }
        shown
    });
    if ambiguous.get() {
        return None;
    }
    let names = names.into_inner();

    let mut sources = SourceDb::new();
    let file = sources.add("<clause-text>", text.as_bytes());
    let mut interner = Interner::new();
    let lexed = lex_in(sources.file(file), file, &mut interner, KeywordSet::V0_5);
    // A comment inside the clause would be copied into the goal, its message and its repairs,
    // where it reads as hiding the rest of the formula: the printed term instead.
    if !lexed.errors.is_empty() || !lexed.comments.is_empty() {
        return None;
    }
    let tokens = &lexed.tokens;
    // A float literal of the text that is no `F64` or `F32` of the term is a decimal literal,
    // which the term holds as its coefficient (`spec/29 §16` rule 2).
    let decimal_literal = tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::Float(_)))
        .count()
        > floats.get();
    let coefficient = tokens.windows(3).any(|window| match window {
        [dot, name, open] => {
            dot.kind == TokenKind::Dot
                && open.kind == TokenKind::LParen
                && matches!(name.kind, TokenKind::LowerIdent(symbol) if interner.resolve(symbol) == "coefficient")
        }
        _ => false,
    });
    if !decimal_literal && !coefficient {
        return None;
    }

    let mut out = String::with_capacity(text.len());
    let mut previous = 0;
    let mut used = BTreeMap::new();
    for (index, token) in tokens.iter().enumerate() {
        let name = match &token.kind {
            TokenKind::Keyword(Keyword::It) => "it",
            TokenKind::Keyword(Keyword::Result) => "result",
            TokenKind::LowerIdent(symbol) => interner.resolve(*symbol),
            _ => continue,
        };
        // A field or method after `.` and an argument label before `:` are no binders.
        let after_dot = index
            .checked_sub(1)
            .and_then(|before| tokens.get(before))
            .is_some_and(|before| before.kind == TokenKind::Dot);
        let label = tokens
            .get(index + 1)
            .is_some_and(|after| after.kind == TokenKind::Colon);
        if after_dot || label {
            continue;
        }
        let Some(Some(replacement)) = names.get(name) else {
            continue;
        };
        // A number that lands as a receiver, `2.5.coefficient()`, is no valid code, and a bare
        // float literal is an `F64`: the printed term instead.
        let receiver = tokens
            .get(index + 1)
            .is_some_and(|after| after.kind == TokenKind::Dot);
        if receiver && starts_with_number(replacement) {
            return None;
        }
        out.push_str(text.get(previous..token.span.start)?);
        let whole = token.span.start == 0 && token.span.end == text.len();
        if whole || atomic(replacement) {
            out.push_str(replacement);
        } else {
            out.push('(');
            out.push_str(replacement);
            out.push(')');
        }
        previous = token.span.end;
        used.insert(name.to_owned(), ());
    }
    out.push_str(text.get(previous..)?);
    // Every binder the term replaces is written in the text; were one not, the text would
    // show a name the obligation does not mean.
    let complete = names
        .iter()
        .all(|(name, shown)| shown.is_none() || used.contains_key(name));
    complete.then_some(out)
}

/// Whether `text` starts with a number, under any `(` and `-`: a literal argument.
fn starts_with_number(text: &str) -> bool {
    text.trim_start_matches(['(', '-', ' '])
        .starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use una_core::{BinOp, Clause, Term};

    use super::shown;

    fn clause(term: Term, text: &str) -> Clause {
        Clause {
            term,
            text: text.to_owned(),
            span: None,
        }
    }

    fn binary(op: BinOp, left: Term, right: Term) -> Term {
        Term::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn int(digits: &str) -> Term {
        Term::Int(digits.to_owned())
    }

    fn var(name: &str) -> Term {
        Term::Var(name.to_owned())
    }

    fn it_is(value: &str) -> impl Fn(&Term) -> Option<String> + '_ {
        move |term: &Term| matches!(term, Term::It).then(|| value.to_owned())
    }

    #[test]
    fn a_decimal_goal_shows_its_literals_at_their_scale() {
        // `it >= 0.00` of a `Decimal[2]` is the term `it >= 0` (ADR 0120 V2).
        let nonnegative = clause(binary(BinOp::Ge, Term::It, int("0")), "it >= 0.00");
        assert_eq!(shown(&nonnegative, &it_is("result")), "result >= 0.00");
        // A value that is no single primary is parenthesized, as the term printer does.
        assert_eq!(shown(&nonnegative, &it_is("a - b")), "(a - b) >= 0.00");
        // An aligned literal shows as written, not as its aligned coefficient.
        let aligned = clause(
            binary(
                BinOp::Lt,
                Term::It,
                binary(BinOp::Mul, int("1000"), int("25")),
            ),
            "it < 2.5",
        );
        assert_eq!(shown(&aligned, &it_is("result")), "result < 2.5");
    }

    #[test]
    fn a_coefficient_goal_names_the_method_and_keeps_fields_and_labels() {
        // `x.coefficient()` is the term `x`; its goal names the method.
        let coefficient = clause(binary(BinOp::Gt, var("x"), int("3")), "x.coefficient() > 3");
        let args = |term: &Term| match term {
            Term::Var(name) if name == "x" => Some("price".to_owned()),
            _ => None,
        };
        assert_eq!(shown(&coefficient, &args), "price.coefficient() > 3");
        // A field named as a binder after `.`, and a label before `:`, stay.
        let field = clause(
            binary(BinOp::Ge, var("x"), int("0")),
            "x >= 0.00 and x.coefficient() >= 0",
        );
        assert_eq!(
            shown(&field, &args),
            "price >= 0.00 and price.coefficient() >= 0"
        );
    }

    #[test]
    fn every_other_goal_is_its_printed_term() {
        // No decimal: the term printer, so the goal and its id are unchanged.
        let integer = clause(binary(BinOp::Ge, Term::It, int("0")), "it >= 0");
        assert_eq!(shown(&integer, &it_is("result")), "result >= 0");
        let float = clause(
            binary(BinOp::Ge, Term::It, Term::F64(0.5f64.to_bits())),
            "it  >=  0.5",
        );
        assert_eq!(shown(&float, &it_is("result")), "result >= 0.5");
        // A replaced binder the text does not write: the printed term.
        let missing = clause(binary(BinOp::Ge, var("y"), int("0")), "z >= 0.00");
        let rename = |term: &Term| matches!(term, Term::Var(_)).then(|| "w".to_owned());
        assert_eq!(shown(&missing, &rename), "w >= 0");
    }

    #[test]
    fn a_goal_whose_text_cannot_be_trusted_is_its_printed_term() {
        let rename = |term: &Term| match term {
            Term::Var(name) if name == "x%1" => Some("a".to_owned()),
            Term::Var(name) if name == "x%2" => Some("b".to_owned()),
            Term::Var(name) if name == "x" => Some("price".to_owned()),
            _ => None,
        };
        // One source name with two replacements: the text cannot say which is which.
        let ambiguous = clause(
            binary(
                BinOp::And,
                binary(BinOp::Gt, var("x%1"), int("0")),
                binary(BinOp::Gt, var("x%2"), int("100")),
            ),
            "x > 0.00 and x > 1.00",
        );
        assert_eq!(shown(&ambiguous, &rename), "a > 0 and b > 100");
        // A text that does not lex.
        let unlexed = clause(binary(BinOp::Gt, var("x"), int("0")), "x > 0.00 ¤");
        assert_eq!(shown(&unlexed, &rename), "price > 0");
        // A comment inside a wrapped clause would leak into the goal and its repairs (#2228).
        let commented = clause(
            binary(
                BinOp::And,
                binary(BinOp::Gt, var("x"), int("0")),
                binary(BinOp::Gt, var("x"), int("100")),
            ),
            "(x > 0.00 -- the price\n    and x > 1.00)",
        );
        assert_eq!(shown(&commented, &rename), "price > 0 and price > 100");
        // A literal argument as the receiver of `coefficient()`: `2.5.coefficient()` is no
        // valid code.
        let literal = |term: &Term| matches!(term, Term::Var(_)).then(|| "2.5".to_owned());
        let receiver = clause(
            binary(BinOp::Lt, var("x"), int("100")),
            "x.coefficient() < 100",
        );
        assert_eq!(shown(&receiver, &literal), "2.5 < 100");
        let negative = |term: &Term| matches!(term, Term::Var(_)).then(|| "-2.5".to_owned());
        assert_eq!(shown(&receiver, &negative), "(-2.5) < 100");
    }
}
