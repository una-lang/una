//! The text of Core IR (`spec/09 §2, §4`): what `una check --dump-core` prints.
//!
//! Declarations in source order, one block each; every binder with its type,
//! label and refinements; primitives in prefix form; qualified callees.

use std::fmt::Write as _;

use crate::ir::{
    Arm, Atom, Base, BinOp, Bound, CapOp, Clause, Ctor, Decl, Expr, FloatOp, Fun, HandlerOp,
    LoopSource, Module, Prim, Rhs, Source, Term, Type, Use,
};

/// The text of a module's Core IR, newline-terminated.
pub fn print_module(module: &Module) -> String {
    let mut printer = Printer::default();
    printer.line(0, &format!("module {}", module.path));
    for decl in &module.decls {
        printer.out.push('\n');
        printer.decl(decl);
    }
    printer.out
}

/// A type as written in Core IR.
pub fn print_type(ty: &Type) -> String {
    let mut out = String::new();
    write_type(&mut out, ty);
    out
}

/// A term in the surface syntax of the refinement language (`spec/03 §3`).
pub fn print_term(term: &Term) -> String {
    print_term_with(term, &|_| None)
}

/// A term in the surface syntax, with each subterm for which `replace` gives a text printed
/// as that text, parenthesized when it is not a single name: `it` as the value it stands for.
pub fn print_term_with(term: &Term, replace: &dyn Fn(&Term) -> Option<String>) -> String {
    let mut out = String::new();
    write_term(&mut out, term, 0, replace);
    out
}

/// Whether source text is a single primary or postfix expression: no white space outside
/// brackets and text literals, and no leading minus, so it needs no parentheses.
fn atomic(text: &str) -> bool {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        if quoted {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => return false,
            _ => {}
        }
    }
    !text.is_empty() && !text.starts_with('-')
}

/// How tightly a term binds, from `implies` (1) to a primary (10), after `spec/02 §7`.
fn precedence(term: &Term) -> u8 {
    match term {
        Term::Binary { op, .. } => match op {
            BinOp::Implies => 1,
            BinOp::Or => 2,
            BinOp::And => 3,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 5,
            BinOp::Add | BinOp::Sub => 6,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 7,
        },
        Term::Not(_) => 4,
        Term::Is { .. } | Term::In { .. } => 5,
        Term::Neg(_) => 8,
        Term::Int(digits) if digits.starts_with('-') => 8,
        Term::Field { .. }
        | Term::Len(_)
        | Term::Size(_)
        | Term::Shape { .. }
        | Term::Proj { .. }
        | Term::Call { .. } => 9,
        Term::Float { op, .. } | Term::Float32 { op, .. } => match op {
            FloatOp::Eq | FloatOp::Ne | FloatOp::Lt | FloatOp::Le | FloatOp::Gt | FloatOp::Ge => 5,
            FloatOp::Add | FloatOp::Sub => 6,
            FloatOp::Mul | FloatOp::Div => 7,
            FloatOp::Neg => 8,
            _ => 9,
        },
        Term::ToF64 { .. } | Term::ToF32 { .. } => 9,
        _ => 10,
    }
}

fn write_term(
    out: &mut String,
    term: &Term,
    at_least: u8,
    replace: &dyn Fn(&Term) -> Option<String>,
) {
    if let Some(text) = replace(term) {
        if atomic(&text) || at_least <= 1 {
            out.push_str(&text);
        } else {
            let _ = write!(out, "({text})");
        }
        return;
    }
    let own = precedence(term);
    let parens = own < at_least;
    if parens {
        out.push('(');
    }
    match term {
        // A renamed binder, `x%2`, prints as its source name `x`.
        Term::Var(name) => match name.split_once('%') {
            Some((source, _)) if !source.is_empty() => out.push_str(source),
            _ => out.push_str(name),
        },
        Term::It => out.push_str("it"),
        Term::Result => out.push_str("result"),
        Term::Int(digits) => out.push_str(digits),
        Term::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Term::Text(text) => out.push_str(&quote(text)),
        Term::Unit => out.push_str("unit"),
        Term::Ctor(ctor) => out.push_str(&ctor.name),
        Term::Field { value, field, .. } => {
            write_term(out, value, 9, replace);
            let _ = write!(out, ".{field}");
        }
        Term::Len(value) => {
            write_term(out, value, 9, replace);
            out.push_str(".len");
        }
        Term::Has { map, key } => {
            write_term(out, map, 9, replace);
            out.push_str(".has(key: ");
            write_term(out, key, 0, replace);
            out.push(')');
        }
        Term::Member { set, item } => {
            write_term(out, set, 9, replace);
            out.push_str(".has(item: ");
            write_term(out, item, 0, replace);
            out.push(')');
        }
        Term::Size(value) => {
            write_term(out, value, 9, replace);
            out.push_str(".size");
        }
        Term::Shape { fun, value } => {
            write_term(out, value, 9, replace);
            let _ = write!(out, ".{}", fun.as_str());
        }
        // `a.get(index: i)`, `m.get(row: i, column: j)` (ADR 0120 rule 33).
        Term::Element { value, indices } => {
            write_term(out, value, 9, replace);
            out.push_str(".get(");
            let labels: &[&str] = match indices.len() {
                1 => &["index"],
                _ => &["row", "column"],
            };
            for (n, (index, label)) in indices.iter().zip(labels).enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "{label}: ");
                write_term(out, index, 0, replace);
            }
            out.push(')');
        }
        Term::Proj { value, index } => {
            write_term(out, value, 9, replace);
            let _ = write!(out, ".{index}");
        }
        Term::Neg(value) => {
            out.push('-');
            write_term(out, value, 8, replace);
        }
        Term::Not(value) => {
            out.push_str("not ");
            write_term(out, value, 4, replace);
        }
        Term::Binary { op, left, right } => {
            // `implies` groups to the right; comparisons do not chain.
            let (left_at, right_at) = match op {
                BinOp::Implies => (own + 1, own),
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    (own + 1, own + 1)
                }
                _ => (own, own + 1),
            };
            write_term(out, left, left_at, replace);
            let _ = write!(out, " {} ", op.as_str());
            write_term(out, right, right_at, replace);
        }
        Term::Is { value, ctor } => {
            write_term(out, value, 6, replace);
            let _ = write!(out, " is {}", ctor.name);
        }
        Term::In {
            value,
            low,
            high,
            exclusive,
        } => {
            write_term(out, value, 6, replace);
            let dots = if *exclusive { "..<" } else { ".." };
            let _ = write!(out, " in {low}{dots}{high}");
        }
        Term::Call { predicate, args } => {
            out.push_str(&predicate.name);
            out.push('(');
            for (index, arg) in args.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_term(out, arg, 1, replace);
            }
            out.push(')');
        }
        Term::F64(bits) => out.push_str(&crate::float::render(f64::from_bits(*bits))),
        Term::F32(bits) => out.push_str(&crate::float32::render(f32::from_bits(*bits))),
        Term::Float { op, args } | Term::Float32 { op, args } => {
            let symbol = match op {
                FloatOp::Add => Some("+"),
                FloatOp::Sub => Some("-"),
                FloatOp::Mul => Some("*"),
                FloatOp::Div => Some("/"),
                FloatOp::Eq => Some("=="),
                FloatOp::Ne => Some("!="),
                FloatOp::Lt => Some("<"),
                FloatOp::Le => Some("<="),
                FloatOp::Gt => Some(">"),
                FloatOp::Ge => Some(">="),
                _ => None,
            };
            match (symbol, args.as_slice(), op) {
                (Some(symbol), [left, right], _) => {
                    let level = precedence(term);
                    write_term(out, left, level, replace);
                    let _ = write!(out, " {symbol} ");
                    write_term(out, right, level + 1, replace);
                }
                (_, [value], FloatOp::Neg) => {
                    out.push('-');
                    write_term(out, value, 9, replace);
                }
                (_, [receiver, rest @ ..], op) => {
                    write_term(out, receiver, 9, replace);
                    let _ = write!(out, ".{}(", op.method().unwrap_or("?"));
                    for (index, (arg, param)) in rest.iter().zip(op.params()).enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        let _ = write!(out, "{param}: ");
                        write_term(out, arg, 1, replace);
                    }
                    out.push(')');
                }
                _ => out.push('?'),
            }
        }
        Term::ToF64 { value, .. } => {
            write_term(out, value, 9, replace);
            out.push_str(".to_f64()");
        }
        Term::ToF32 { value, .. } => {
            write_term(out, value, 9, replace);
            out.push_str(".to_f32()");
        }
    }
    if parens {
        out.push(')');
    }
}

/// What a loop or a `together_for` visits.
fn print_source(source: &LoopSource) -> String {
    match source {
        LoopSource::List(list) => print_atom(list),
        LoopSource::Range {
            low,
            high,
            inclusive,
        } => format!(
            "{}{}{}",
            print_atom(low),
            if *inclusive { ".." } else { "..<" },
            print_atom(high)
        ),
        LoopSource::Until { measure, .. } => format!("until decreases {}", measure.text),
        LoopSource::Ever { within: None } => "ever".to_owned(),
        LoopSource::Ever {
            within: Some((deadline, _)),
        } => format!("ever within {}", print_atom(deadline)),
    }
}

/// An atom as written in Core IR.
pub fn print_atom(atom: &Atom) -> String {
    match atom {
        Atom::Var(name) => name.clone(),
        Atom::Int(digits) => digits.clone(),
        Atom::F64(bits) => crate::float::render(f64::from_bits(*bits)),
        Atom::Decimal { coefficient, scale } => {
            let c = coefficient.parse().unwrap_or_default();
            format!("{}d", crate::decimal::render(&c, *scale))
        }
        Atom::F32(bits) => crate::float32::render(f32::from_bits(*bits)),
        Atom::Bool(value) => value.to_string(),
        Atom::Text(text) => quote(text),
        Atom::Unit => "unit".to_owned(),
    }
}

/// A text literal in Una syntax, with the escapes of `spec/01 §6`.
pub fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20
                || c == '\u{7f}'
                || ('\u{202a}'..='\u{202e}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c) =>
            {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_type(out: &mut String, ty: &Type) {
    match ty {
        Type::Base {
            base,
            refinements,
            label,
        } => {
            if !label.is_public() {
                let _ = write!(out, "{label} ");
            }
            if refinements.is_empty() {
                write_base(out, base);
            } else {
                out.push('{');
                write_base(out, base);
                let names: Vec<&str> = refinements.iter().map(|r| r.name.as_str()).collect();
                let _ = write!(out, " | {}}}", names.join(", "));
            }
        }
        Type::Fn {
            params,
            result,
            uses,
            fails,
            label,
        } => {
            if !label.is_public() {
                let _ = write!(out, "{label} ");
            }
            out.push_str("fn(");
            for (index, param) in params.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_type(out, param);
            }
            out.push_str(") -> ");
            write_type(out, result);
            if !uses.is_empty() {
                let _ = write!(out, " uses {}", print_uses(uses));
            }
            if let Some(fails) = fails {
                let _ = write!(out, " fails {fails}");
            }
        }
    }
}

fn write_base(out: &mut String, base: &Base) {
    match base {
        Base::Int => out.push_str("Int"),
        Base::Fixed(int) => out.push_str(int.as_str()),
        Base::F64 => out.push_str("F64"),
        Base::Decimal(scale) => out.push_str(&format!("Decimal[{scale}]")),
        Base::F32 => out.push_str("F32"),
        Base::Bool => out.push_str("Bool"),
        Base::Text => out.push_str("Text"),
        Base::Unit => out.push_str("Unit"),
        Base::Hashed => out.push_str("Hashed"),
        Base::Named { name, args, .. } => {
            out.push_str(name);
            if !args.is_empty() {
                out.push('[');
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    write_type(out, arg);
                }
                out.push(']');
            }
        }
        Base::Param(name) => out.push_str(name),
        Base::Tuple(types) => {
            out.push('(');
            for (index, ty) in types.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_type(out, ty);
            }
            out.push(')');
        }
        Base::Unknown => out.push('_'),
    }
}

/// A capability context, `{c ↦ K[O], ...}`.
fn print_uses(uses: &[Use]) -> String {
    let entries: Vec<String> = uses
        .iter()
        .map(|u| format!("{} ↦ {}[{}]", u.cap, u.capability, u.ops.join(", ")))
        .collect();
    format!("{{{}}}", entries.join(", "))
}

/// The `requires` and `ensures` clauses of a header, each with its source text.
fn clauses(requires: &[Clause], ensures: &[Clause]) -> String {
    let mut out = String::new();
    for clause in requires {
        let _ = write!(out, " requires {}", clause.text);
    }
    for clause in ensures {
        let _ = write!(out, " ensures {}", clause.text);
    }
    out
}

fn atoms(atoms: &[Atom]) -> String {
    atoms.iter().map(print_atom).collect::<Vec<_>>().join(", ")
}

fn ctor(ctor: &Ctor, args: &[Atom]) -> String {
    if args.is_empty() {
        ctor.name.clone()
    } else {
        format!("{}({})", ctor.name, atoms(args))
    }
}

#[derive(Default)]
struct Printer {
    out: String,
}

impl Printer {
    fn line(&mut self, indent: usize, text: &str) {
        for _ in 0..indent {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn params(params: &[(String, Type)]) -> String {
        params
            .iter()
            .map(|(name, ty)| format!("{name}: {}", print_type(ty)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn decl(&mut self, decl: &Decl) {
        match decl {
            Decl::Alias {
                name,
                params,
                target,
            } => {
                let params = if params.is_empty() {
                    String::new()
                } else {
                    format!("[{}]", params.join(", "))
                };
                self.line(0, &format!("type {name}{params} = {}", print_type(target)));
            }
            Decl::Data {
                name,
                params,
                variants,
                ..
            } => {
                let params = if params.is_empty() {
                    String::new()
                } else {
                    format!("[{}]", params.join(", "))
                };
                let variants: Vec<String> = variants
                    .iter()
                    .map(|variant| {
                        if variant.fields.is_empty() {
                            variant.name.clone()
                        } else {
                            format!("{}({})", variant.name, Printer::params(&variant.fields))
                        }
                    })
                    .collect();
                self.line(
                    0,
                    &format!("data {name}{params} = {}", variants.join(" | ")),
                );
            }
            Decl::Pred {
                name,
                reflect,
                params,
                body,
                ..
            } => {
                let kind = if *reflect { "reflect" } else { "opaque" };
                self.line(
                    0,
                    &format!("pred {kind} {name}({}) =", Printer::params(params)),
                );
                self.expr(1, body);
            }
            Decl::Cap { name, ops } => {
                self.line(0, &format!("cap {name} {{"));
                for op in ops {
                    self.cap_op(op);
                }
                self.line(0, "}");
            }
            Decl::Handler {
                name,
                params,
                capability,
                uses,
                ops,
                ..
            } => {
                let uses = if uses.is_empty() {
                    String::new()
                } else {
                    format!(" uses {}", print_uses(uses))
                };
                self.line(
                    0,
                    &format!(
                        "handler {name}({}) for {capability}{uses} {{",
                        Printer::params(params)
                    ),
                );
                for op in ops {
                    self.handler_op(op);
                }
                self.line(0, "}");
            }
            Decl::Fun(fun) => self.fun(fun),
            Decl::Policy {
                name,
                from,
                to,
                via,
            } => {
                self.line(
                    0,
                    &format!(
                        "policy {name} : {} ⇝ {} via {}",
                        print_type(from),
                        print_type(to),
                        via.qualified()
                    ),
                );
            }
        }
    }

    fn cap_op(&mut self, op: &CapOp) {
        let mut text = format!(
            "op {}({}) : {}",
            op.name,
            Printer::params(&op.params),
            print_type(&op.result)
        );
        if let Some(fails) = &op.fails {
            let _ = write!(text, " fails {fails}");
        }
        text.push_str(&clauses(&op.requires, &op.ensures));
        match &op.accepts {
            Some(label) => {
                let _ = write!(text, " accepts {label}");
            }
            None => text.push_str(" accepts ⊤"),
        }
        self.line(1, &text);
    }

    fn handler_op(&mut self, op: &HandlerOp) {
        self.line(1, &format!("op {}({}) =", op.name, op.params.join(", ")));
        self.expr(2, &op.body);
    }

    fn fun(&mut self, fun: &Fun) {
        let type_params = if fun.type_params.is_empty() {
            String::new()
        } else {
            format!("[{}]", fun.type_params.join(", "))
        };
        let mut header = format!(
            "fun {}{type_params}({}) : {}",
            fun.name,
            Printer::params(&fun.params),
            print_type(&fun.result)
        );
        if !fun.uses.is_empty() {
            let _ = write!(header, " uses {}", print_uses(&fun.uses));
        }
        if let Some(fails) = &fun.fails {
            let _ = write!(header, " fails {fails}");
        }
        header.push_str(&clauses(&fun.requires, &fun.ensures));
        header.push_str(" =");
        self.line(0, &header);
        self.expr(1, &fun.body);
    }

    fn expr(&mut self, indent: usize, expr: &Expr) {
        match expr {
            Expr::Atom { atom, .. } => self.line(indent, &print_atom(atom)),
            Expr::Unreachable { .. } => self.line(indent, "unreachable"),
            Expr::Loop {
                source,
                var,
                state,
                init,
                body,
                on_stop,
                ..
            } => {
                let source_ref = source.as_ref();
                let source = print_source(source);
                let state: Vec<String> = state
                    .iter()
                    .zip(init)
                    .map(|((name, ty), init)| {
                        format!("{name}: {} = {}", print_type(ty), print_atom(init))
                    })
                    .collect();
                self.line(
                    indent,
                    &format!(
                        "loop {}: {} in {source} with ({}) do",
                        var.0,
                        print_type(&var.1),
                        state.join(", ")
                    ),
                );
                // The condition of `until`, evaluated before each iteration.
                if let LoopSource::Until { cond, .. } = source_ref {
                    self.line(indent + 1, "until");
                    self.expr(indent + 2, cond);
                    self.line(indent + 1, "do");
                }
                self.expr(indent + 1, body);
                if let Some(on_stop) = on_stop {
                    let names: Vec<&str> = on_stop
                        .state
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect();
                    self.line(indent, &format!("on stop with ({}) do", names.join(", ")));
                    self.expr(indent + 1, &on_stop.body);
                }
            }
            Expr::Together { tasks, .. } => {
                self.line(indent, "together");
                for task in tasks {
                    self.line(indent + 1, "task");
                    self.expr(indent + 2, task);
                }
            }
            Expr::FirstOf { tasks, .. } => {
                self.line(indent, "first_of");
                for task in tasks {
                    self.line(indent + 1, "branch");
                    self.expr(indent + 2, task);
                }
            }
            Expr::Tasks { bound, body, .. } => {
                match bound {
                    Some((bound, _)) => {
                        self.line(indent, &format!("tasks at most {}", print_atom(bound)));
                    }
                    None => self.line(indent, "tasks"),
                }
                self.expr(indent + 1, body);
            }
            Expr::Start { task, .. } => {
                self.line(indent, "start");
                self.expr(indent + 1, task);
            }
            Expr::TogetherFor {
                source, var, task, ..
            } => {
                self.line(
                    indent,
                    &format!(
                        "together_for {}: {} in {} do",
                        var.0,
                        print_type(&var.1),
                        print_source(source)
                    ),
                );
                self.expr(indent + 1, task);
            }
            Expr::LetTuple {
                names, value, body, ..
            } => {
                let binders: Vec<String> = names
                    .iter()
                    .map(|(name, ty)| format!("{name}: {}", print_type(ty)))
                    .collect();
                self.line(
                    indent,
                    &format!("let ({}) = {} in", binders.join(", "), print_atom(value)),
                );
                self.expr(indent, body);
            }
            Expr::Let {
                name,
                ty,
                value,
                body,
                ..
            } => {
                let head = format!("let {name}: {} =", print_type(ty));
                match value.as_ref() {
                    Bound::Rhs(rhs) if !matches!(rhs, Rhs::Lambda { .. } | Rhs::Checked { .. }) => {
                        self.line(indent, &format!("{head} {} in", rhs_line(rhs)));
                    }
                    Bound::Rhs(rhs) => {
                        self.line(indent, &head);
                        self.rhs_block(indent + 1, rhs);
                        self.line(indent, "in");
                    }
                    Bound::Expr(inner) => {
                        self.line(indent, &head);
                        self.expr(indent + 1, inner);
                        self.line(indent, "in");
                    }
                }
                self.expr(indent, body);
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                self.line(indent, &format!("if {} then", print_atom(cond)));
                self.expr(indent + 1, then);
                self.line(indent, "else");
                self.expr(indent + 1, otherwise);
            }
            Expr::Case {
                scrutinee,
                arms,
                otherwise,
            } => {
                self.line(indent, &format!("case {} of", print_atom(scrutinee)));
                for arm in arms {
                    self.arm(indent + 1, arm);
                }
                if let Some(otherwise) = otherwise {
                    self.line(indent + 1, "else ⇒");
                    self.expr(indent + 2, otherwise);
                }
            }
            Expr::Fail { ctor: c, args, .. } => {
                self.line(indent, &format!("fail {}", ctor(c, args)));
            }
            Expr::Exit { tag, state, .. } => {
                self.line(indent, &format!("exit {}({})", tag.as_str(), atoms(state)));
            }
            Expr::Handle {
                body,
                error,
                handler,
            } => {
                self.line(indent, "handle");
                self.expr(indent + 1, body);
                self.line(indent, &format!("with {error} ⇒"));
                self.expr(indent + 1, handler);
            }
            Expr::Attempt { body, .. } => {
                self.line(indent, "attempt");
                self.expr(indent + 1, body);
            }
            Expr::Within {
                deadline,
                body,
                timeout,
                wrap,
                ..
            } => {
                let wrap = wrap
                    .as_ref()
                    .map_or_else(String::new, |wrap| format!(", wrap {}", wrap.name));
                self.line(
                    indent,
                    &format!(
                        "within {} (timeout {}{wrap})",
                        print_atom(deadline),
                        timeout.name
                    ),
                );
                self.expr(indent + 1, body);
            }
            Expr::With { cap, source, body } => {
                let source = match source {
                    Source::Rebind(other) => other.clone(),
                    Source::Handler {
                        handler,
                        args,
                        given,
                        ..
                    } => {
                        let given = if *given { " (given)" } else { "" };
                        format!("{}({}){given}", handler.qualified(), atoms(args))
                    }
                };
                self.line(indent, &format!("with {cap} = {source} in"));
                self.expr(indent, body);
            }
        }
    }

    fn arm(&mut self, indent: usize, arm: &Arm) {
        let binds: Vec<String> = arm
            .binds
            .iter()
            .map(|bind| bind.clone().unwrap_or_else(|| "_".to_owned()))
            .collect();
        let pattern = if binds.is_empty() {
            arm.ctor.name.clone()
        } else {
            format!("{}({})", arm.ctor.name, binds.join(", "))
        };
        self.line(indent, &format!("{pattern} ⇒"));
        self.expr(indent + 1, &arm.body);
    }

    /// A right-hand side that takes more than a line: a lambda or `checked`.
    fn rhs_block(&mut self, indent: usize, rhs: &Rhs) {
        match rhs {
            Rhs::Lambda { params, body, .. } => {
                self.line(indent, &format!("λ({}).", Printer::params(params)));
                self.expr(indent + 1, body);
            }
            Rhs::Checked {
                value,
                it,
                predicate,
                ctor: c,
                args,
                ..
            } => {
                self.line(indent, &format!("checked({}, λ({it}).", print_atom(value)));
                self.expr(indent + 1, predicate);
                self.line(indent, &format!(", {})", ctor(c, args)));
            }
            other => self.line(indent, &rhs_line(other)),
        }
    }
}

/// A right-hand side on one line.
fn rhs_line(rhs: &Rhs) -> String {
    match rhs {
        Rhs::Atom(atom) => print_atom(atom),
        Rhs::Tuple(items) => format!("({})", atoms(items)),
        Rhs::Prim {
            prim: Prim::Decimal(op),
            args,
            ..
        } => {
            // The static scales are part of a decimal operation's meaning (`spec/29 §16`).
            let scales: Vec<String> = op.scales().iter().map(u8::to_string).collect();
            format!("{}[{}]({})", op.as_str(), scales.join(", "), atoms(args))
        }
        Rhs::Prim { prim, args, .. } => format!("{}({})", prim.as_str(), atoms(args)),
        Rhs::Call {
            function,
            type_args,
            args,
            ..
        } => {
            let type_args = if type_args.is_empty() {
                String::new()
            } else {
                let shown: Vec<String> = type_args.iter().map(print_type).collect();
                format!("[{}]", shown.join(", "))
            };
            format!("{}{type_args}({})", function.qualified(), atoms(args))
        }
        Rhs::Apply { callee, args, .. } => format!("{}({})", print_atom(callee), atoms(args)),
        Rhs::Op { cap, op, args, .. } => format!("{cap}.{op}({})", atoms(args)),
        Rhs::Construct { ctor: c, args, .. } => ctor(c, args),
        Rhs::Field { value, field, .. } => format!("{}.{field}", print_atom(value)),
        Rhs::List(items) => format!("[{}]", atoms(items)),
        Rhs::Declassify { policy, arg, .. } => {
            format!("declassify[{}]({})", policy.qualified(), print_atom(arg))
        }
        Rhs::Lambda { params, .. } => format!("λ({}). ...", Printer::params(params)),
        Rhs::Checked { value, .. } => format!("checked({}, ...)", print_atom(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str) -> Box<Term> {
        Box::new(Term::Var(name.to_owned()))
    }

    fn binary(op: BinOp, left: Box<Term>, right: Box<Term>) -> Box<Term> {
        Box::new(Term::Binary { op, left, right })
    }

    #[test]
    fn terms_print_with_the_parentheses_that_precedence_needs() {
        let sum = binary(BinOp::Add, var("a"), var("b"));
        assert_eq!(
            print_term(&binary(BinOp::Mul, sum.clone(), var("c"))),
            "(a + b) * c"
        );
        assert_eq!(
            print_term(&binary(
                BinOp::Add,
                var("a"),
                binary(BinOp::Mul, var("b"), var("c"))
            )),
            "a + b * c"
        );
        assert_eq!(
            print_term(&binary(
                BinOp::Sub,
                var("a"),
                binary(BinOp::Sub, var("b"), var("c"))
            )),
            "a - (b - c)"
        );
        let implies = binary(
            BinOp::Implies,
            var("x"),
            binary(BinOp::Implies, var("y"), var("z")),
        );
        assert_eq!(print_term(&implies), "x implies y implies z");
        let grouped = binary(
            BinOp::Implies,
            binary(BinOp::Implies, var("x"), var("y")),
            var("z"),
        );
        assert_eq!(print_term(&grouped), "(x implies y) implies z");
        let not = Term::Not(binary(BinOp::And, var("a"), var("b")));
        assert_eq!(print_term(&not), "not (a and b)");
        let renamed = Term::Len(var("name%2"));
        assert_eq!(print_term(&renamed), "name.len");
        let it = Term::In {
            value: Box::new(Term::It),
            low: "0".to_owned(),
            high: "5000".to_owned(),
            exclusive: false,
        };
        let value = |term: &Term| matches!(term, Term::It).then(|| "e.remaining".to_owned());
        assert_eq!(print_term_with(&it, &value), "e.remaining in 0..5000");
        let sum_value = |term: &Term| matches!(term, Term::It).then(|| "a + 1".to_owned());
        let less = binary(BinOp::Lt, Box::new(Term::It), var("b"));
        assert_eq!(print_term_with(&less, &sum_value), "(a + 1) < b");
        let call = |term: &Term| matches!(term, Term::It).then(|| "clamp(f(x), y)".to_owned());
        assert_eq!(print_term_with(&it, &call), "clamp(f(x), y) in 0..5000");
    }
}
