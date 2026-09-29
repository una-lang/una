//! The text of Core IR (`spec/09 §2, §4`): what `una check --dump-core` prints.
//!
//! DECISION D-099: declarations in source order, one block each; every binder with its type,
//! label and refinements; primitives in prefix form; qualified callees.

use std::fmt::Write as _;

use crate::ir::{
    Arm, Atom, Base, Bound, CapOp, Ctor, Decl, Expr, Fun, HandlerOp, Module, Rhs, Source, Type, Use,
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

/// An atom as written in Core IR.
pub fn print_atom(atom: &Atom) -> String {
    match atom {
        Atom::Var(name) => name.clone(),
        Atom::Int(digits) => digits.clone(),
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
                let _ = write!(out, " | {}}}", refinements.join(", "));
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
        Base::Bool => out.push_str("Bool"),
        Base::Text => out.push_str("Text"),
        Base::Unit => out.push_str("Unit"),
        Base::Hashed => out.push_str("Hashed"),
        Base::Named { name, args } => {
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
        header.push_str(" =");
        self.line(0, &header);
        self.expr(1, &fun.body);
    }

    fn expr(&mut self, indent: usize, expr: &Expr) {
        match expr {
            Expr::Atom(atom) => self.line(indent, &print_atom(atom)),
            Expr::Let {
                name,
                ty,
                value,
                body,
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
            Expr::Attempt(body) => {
                self.line(indent, "attempt");
                self.expr(indent + 1, body);
            }
            Expr::With { cap, source, body } => {
                let source = match source {
                    Source::Rebind(other) => other.clone(),
                    Source::Handler {
                        handler,
                        args,
                        given,
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
            Rhs::Lambda { params, body } => {
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
        Rhs::Prim { prim, args } => format!("{}({})", prim.as_str(), atoms(args)),
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
        Rhs::Construct { ctor: c, args } => ctor(c, args),
        Rhs::Field { value, field, .. } => format!("{}.{field}", print_atom(value)),
        Rhs::List(items) => format!("[{}]", atoms(items)),
        Rhs::Declassify { policy, arg, .. } => {
            format!("declassify[{}]({})", policy.qualified(), print_atom(arg))
        }
        Rhs::Lambda { params, .. } => format!("λ({}). ...", Printer::params(params)),
        Rhs::Checked { value, .. } => format!("checked({}, ...)", print_atom(value)),
    }
}
