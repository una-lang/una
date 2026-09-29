//! The syntax of Core IR (`spec/09 §2`).

use una_syntax::Span;

use crate::label::{Accepts, Label};

/// A binder: a parameter, a local or a temporary `%t<n>`, unique within its function
/// (`spec/09 §1` rule 3).
pub type Name = String;

/// The qualified name of a declaration: its module's path and its name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Path {
    /// The module path, for example `events.checkin`.
    pub module: String,
    /// The declaration's name.
    pub name: String,
}

impl Path {
    /// `module.name`.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
}

/// A type: a refined, labeled base, or a function type (`spec/09 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// `{ν: b | φ}^ℓ`, with the refinements of the kinds or range it comes from.
    Base {
        /// The base type.
        base: Base,
        /// The refinements, conjoined (`spec/03 §2` rule 3).
        refinements: Vec<Refinement>,
        /// The label.
        label: Label,
    },
    /// `(fn(τ, ...) -> τ uses Κ fails E)^ℓ`.
    Fn {
        /// The parameter types.
        params: Vec<Type>,
        /// The result type.
        result: Box<Type>,
        /// The capability context.
        uses: Vec<Use>,
        /// The error kind.
        fails: Option<String>,
        /// The label.
        label: Label,
    },
}

/// A base type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base {
    /// `Int`.
    Int,
    /// `Bool`.
    Bool,
    /// `Text`.
    Text,
    /// `Unit`.
    Unit,
    /// `Hashed`.
    Hashed,
    /// A kind with its type arguments, `List` and `Option` included.
    Named {
        /// The kind's name.
        name: String,
        /// Its type arguments.
        args: Vec<Type>,
    },
    /// A type parameter.
    Param(String),
    /// A type that inference left open; it only occurs where no value is ever built.
    Unknown,
}

/// One entry of a capability context, `c ↦ K[O]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    /// The reference, without `@`.
    pub cap: Name,
    /// The capability.
    pub capability: String,
    /// The operations.
    pub ops: Vec<String>,
}

/// A term of the refinement language (`spec/03 §3`): a tree over binders, `it` and
/// `result`, since formulas are not A-normal (DECISION D-111).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    /// A binder: a parameter, a local or a pattern variable.
    Var(Name),
    /// `it`, the refined value.
    It,
    /// `result`, the value a function returns.
    Result,
    /// An integer literal, in decimal, with an optional `-`.
    Int(String),
    /// `true` or `false`.
    Bool(bool),
    /// A text literal.
    Text(String),
    /// `unit`.
    Unit,
    /// A variant without fields, as a value.
    Ctor(Ctor),
    /// `a.f`, a field of a record.
    Field {
        /// The record.
        value: Box<Term>,
        /// The record's kind.
        kind: Path,
        /// The field's name.
        field: String,
        /// The field's position.
        index: usize,
    },
    /// `a.len`.
    Len(Box<Term>),
    /// `-a`.
    Neg(Box<Term>),
    /// `not a`.
    Not(Box<Term>),
    /// `a op b`.
    Binary {
        /// The operator.
        op: BinOp,
        /// The left operand.
        left: Box<Term>,
        /// The right operand.
        right: Box<Term>,
    },
    /// `a is C`.
    Is {
        /// The value tested.
        value: Box<Term>,
        /// The variant.
        ctor: Ctor,
    },
    /// `a in low..high` or `a in low..<high`.
    In {
        /// The value tested.
        value: Box<Term>,
        /// The low bound, in decimal.
        low: String,
        /// The high bound, in decimal.
        high: String,
        /// Whether the high bound is excluded.
        exclusive: bool,
    },
    /// `p(a, ...)`, a predicate.
    Call {
        /// The predicate.
        predicate: Path,
        /// The arguments, in parameter order.
        args: Vec<Term>,
    },
}

/// A binary operator of the refinement language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`, Euclidean.
    Div,
    /// `mod`, Euclidean.
    Mod,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `and`.
    And,
    /// `or`.
    Or,
    /// `implies`.
    Implies,
}

impl BinOp {
    /// How the operator is written in Una.
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "mod",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Implies => "implies",
        }
    }
}

/// A formula with its source: a `requires` or `ensures` clause, or the predicate of a
/// refinement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    /// The formula.
    pub term: Term,
    /// The formula as written.
    pub text: String,
    /// Where it is written; `None` in the standard library, whose source is part of the
    /// compiler and never reported.
    pub span: Option<Span>,
}

/// A refinement of a type: the predicate `it` satisfies (`spec/03 §2, §3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refinement {
    /// How it is written: an alias kind's name, or a range such as `in 0..5000`.
    pub name: String,
    /// The predicate over `it`, and the `where` clause or range that declares it.
    pub clause: Clause,
}

/// An atom (`spec/09 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Atom {
    /// A binder.
    Var(Name),
    /// An integer literal, in decimal, with an optional `-`.
    Int(String),
    /// `true` or `false`.
    Bool(bool),
    /// A text literal, with its escapes decoded.
    Text(String),
    /// `unit`.
    Unit,
}

/// A primitive operation (`spec/09 §2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// Euclidean `div`.
    Div,
    /// Euclidean `mod`.
    Mod,
    /// Negation.
    Neg,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `not`.
    Not,
    /// The length of a text or list.
    Len,
    /// Text concatenation.
    Concat,
    /// `Int`, `Bool` or `Text` as text.
    ToText,
}

impl Prim {
    /// How the primitive is written in Core.
    pub fn as_str(self) -> &'static str {
        match self {
            Prim::Add => "+",
            Prim::Sub => "-",
            Prim::Mul => "*",
            Prim::Div => "div",
            Prim::Mod => "mod",
            Prim::Neg => "neg",
            Prim::Eq => "==",
            Prim::Ne => "!=",
            Prim::Lt => "<",
            Prim::Le => "<=",
            Prim::Gt => ">",
            Prim::Ge => ">=",
            Prim::Not => "not",
            Prim::Len => "len",
            Prim::Concat => "concat",
            Prim::ToText => "to_text",
        }
    }
}

/// A constructor: a variant of a sum kind, or the constructor of a record kind, which has the
/// kind's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ctor {
    /// The kind.
    pub kind: Path,
    /// The variant, or the record kind's name.
    pub name: String,
    /// The variant's position in its kind; 0 for a record.
    pub index: usize,
}

/// A right-hand side (`spec/09 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rhs {
    /// An atom.
    Atom(Atom),
    /// A primitive applied to atoms.
    Prim {
        /// The primitive.
        prim: Prim,
        /// Its operands.
        args: Vec<Atom>,
        /// The source of each operand.
        arg_spans: Vec<Span>,
        /// The expression in the source.
        span: Span,
    },
    /// `f[τ*](a, ...)`: a call of a declared function or predicate.
    Call {
        /// The callee.
        function: Path,
        /// The type arguments of a generic callee.
        type_args: Vec<Type>,
        /// The arguments, in parameter order.
        args: Vec<Atom>,
        /// The source of each argument.
        arg_spans: Vec<Span>,
        /// The call in the source.
        span: Span,
    },
    /// `a(a, ...)`: a call of a function value.
    Apply {
        /// The function value.
        callee: Atom,
        /// The arguments.
        args: Vec<Atom>,
        /// The source of each argument.
        arg_spans: Vec<Span>,
        /// The call in the source.
        span: Span,
    },
    /// `c.o(a, ...)`, with the static label of each argument for trace redaction
    /// (`spec/11 §1, §5`).
    Op {
        /// The capability reference.
        cap: Name,
        /// The capability.
        capability: String,
        /// The operation.
        op: String,
        /// The arguments, in parameter order.
        args: Vec<Atom>,
        /// The source of each argument.
        arg_spans: Vec<Span>,
        /// The static labels of the arguments.
        labels: Vec<Label>,
        /// The operation in the source.
        span: Span,
    },
    /// `C(a, ...)`, with the arguments in field order.
    Construct {
        /// The constructor.
        ctor: Ctor,
        /// The fields.
        args: Vec<Atom>,
        /// The source of each field.
        arg_spans: Vec<Span>,
    },
    /// `a.f`.
    Field {
        /// The record.
        value: Atom,
        /// The field's name.
        field: String,
        /// The field's position.
        index: usize,
    },
    /// `[a, ...]`.
    List(Vec<Atom>),
    /// `λ(x: τ, ...). e`.
    Lambda {
        /// The parameters.
        params: Vec<(Name, Type)>,
        /// The body.
        body: Box<Expr>,
    },
    /// `checked(a, φ, C(a, ...))`, with `φ` as an expression over the binder `it`, which
    /// the interpreter runs, and as a term, which the verifier assumes.
    Checked {
        /// The value checked.
        value: Atom,
        /// The binder of `it`.
        it: Name,
        /// The predicate.
        predicate: Box<Expr>,
        /// The predicate as a term over `it`.
        formula: Term,
        /// The failure.
        ctor: Ctor,
        /// Its fields.
        args: Vec<Atom>,
        /// The source of each field.
        arg_spans: Vec<Span>,
        /// `checked` in the source.
        span: Span,
    },
    /// `declassify_p(a)`.
    Declassify {
        /// The policy.
        policy: Path,
        /// Its `via` function.
        via: Path,
        /// The value.
        arg: Atom,
        /// The source of the value.
        arg_span: Span,
        /// The call in the source.
        span: Span,
    },
}

/// What a `let` binds: a right-hand side, or an expression, as the value of a handled
/// statement or a branching construct is (DECISION D-098).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    /// A right-hand side.
    Rhs(Rhs),
    /// An expression.
    Expr(Expr),
}

/// An arm of `case`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    /// The variant.
    pub ctor: Ctor,
    /// The binder of each field, in field order, where one is bound.
    pub binds: Vec<Option<Name>>,
    /// The body.
    pub body: Expr,
}

/// The right-hand side of `with c = s`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `with c = d`.
    Rebind(Name),
    /// `with c = h(a, ...)`, or a `given` line of an example.
    Handler {
        /// The handler.
        handler: Path,
        /// Its arguments.
        args: Vec<Atom>,
        /// The source of each argument.
        arg_spans: Vec<Span>,
        /// Whether a `given` line installs it (`spec/11 §5`).
        given: bool,
    },
}

/// An expression (`spec/09 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// An atom, returned by the enclosing expression.
    Atom {
        /// The atom.
        atom: Atom,
        /// The returned expression in the source.
        span: Span,
    },
    /// `let x: τ = b in e`.
    Let {
        /// The binder.
        name: Name,
        /// Its type.
        ty: Type,
        /// What it binds.
        value: Box<Bound>,
        /// The rest.
        body: Box<Expr>,
        /// The source of the value.
        span: Span,
        /// Whether the source writes the binder's type, whose refinements are then an
        /// obligation (`spec/12 §3`).
        written: bool,
    },
    /// `if a then e else e`.
    If {
        /// The condition.
        cond: Atom,
        /// The branch taken when it holds.
        then: Box<Expr>,
        /// The other branch.
        otherwise: Box<Expr>,
    },
    /// `case a of { C(x, ...) ⇒ e; ... } [else ⇒ e]`.
    Case {
        /// The value examined.
        scrutinee: Atom,
        /// The arms.
        arms: Vec<Arm>,
        /// The `else` branch.
        otherwise: Option<Box<Expr>>,
    },
    /// `fail C(a, ...)`.
    Fail {
        /// The variant.
        ctor: Ctor,
        /// Its fields.
        args: Vec<Atom>,
        /// The source of each field.
        arg_spans: Vec<Span>,
        /// The failure in the source.
        span: Span,
    },
    /// `handle e with x ⇒ e`.
    Handle {
        /// The expression whose failure is handled.
        body: Box<Expr>,
        /// The binder of the failure.
        error: Name,
        /// The handler.
        handler: Box<Expr>,
    },
    /// `attempt e`.
    Attempt(Box<Expr>),
    /// `with c = s in e`.
    With {
        /// The capability reference bound.
        cap: Name,
        /// What it is bound to.
        source: Source,
        /// The expression it scopes over.
        body: Box<Expr>,
    },
    /// The branch after the last arm of a `when` over literals or ranges without `else`,
    /// which the `exhaustive` obligation proves is never taken (DECISION D-111).
    Unreachable {
        /// The `when` keyword.
        span: Span,
        /// The disjunction of the arms' conditions, as text.
        goal: String,
    },
}

/// A variant of a data declaration, or the single constructor of a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataVariant {
    /// The name.
    pub name: String,
    /// The fields, in order.
    pub fields: Vec<(String, Type)>,
}

/// An operation of a capability declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapOp {
    /// The name.
    pub name: String,
    /// The parameters.
    pub params: Vec<(Name, Type)>,
    /// The result type.
    pub result: Type,
    /// The error kind.
    pub fails: Option<String>,
    /// The `requires` clauses, over the parameters.
    pub requires: Vec<Clause>,
    /// The `ensures` clauses, over the parameters and `result`.
    pub ensures: Vec<Clause>,
    /// The sink label; `None` accepts every label.
    pub accepts: Accepts,
}

/// An operation of a handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerOp {
    /// The name.
    pub name: String,
    /// The parameters.
    pub params: Vec<Name>,
    /// The body.
    pub body: Expr,
}

/// A function declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fun {
    /// The name.
    pub name: String,
    /// The type parameters.
    pub type_params: Vec<String>,
    /// The parameters.
    pub params: Vec<(Name, Type)>,
    /// The result type.
    pub result: Type,
    /// The capability context.
    pub uses: Vec<Use>,
    /// The error kind.
    pub fails: Option<String>,
    /// The `requires` clauses, over the parameters.
    pub requires: Vec<Clause>,
    /// The `ensures` clauses, over the parameters and `result`.
    pub ensures: Vec<Clause>,
    /// The body.
    pub body: Expr,
    /// The function's name in the source.
    pub span: Span,
    /// Whether the function is `pub`.
    pub public: bool,
}

/// The contract of a function of the standard library, whose body is native: its `ensures`
/// are hypotheses for callers (`spec/14 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// The function.
    pub path: Path,
    /// The type parameters.
    pub type_params: Vec<String>,
    /// The parameters.
    pub params: Vec<(Name, Type)>,
    /// The result type.
    pub result: Type,
    /// The `requires` clauses.
    pub requires: Vec<Clause>,
    /// The `ensures` clauses.
    pub ensures: Vec<Clause>,
}

/// A declaration (`spec/09 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    /// `type T[α*] = τ`.
    Alias {
        /// The name.
        name: String,
        /// The type parameters.
        params: Vec<String>,
        /// The type it stands for.
        target: Type,
    },
    /// `data T[α*] = C(f: τ, ...) | ...`.
    Data {
        /// The name.
        name: String,
        /// The type parameters.
        params: Vec<String>,
        /// The variants; a record has one, named after it.
        variants: Vec<DataVariant>,
        /// Whether it is a record.
        record: bool,
    },
    /// `pred reflect p(x: τ, ...) = φ` or `pred opaque p(x: τ, ...) = e`.
    Pred {
        /// The name.
        name: String,
        /// Whether it is `reflect`.
        reflect: bool,
        /// The parameters.
        params: Vec<(Name, Type)>,
        /// The body, which the interpreter runs.
        body: Expr,
        /// The body of a reflect predicate as a term, which the verifier defines it by.
        formula: Option<Term>,
        /// The predicate's name in the source.
        span: Span,
    },
    /// `cap K { op ... }`.
    Cap {
        /// The name.
        name: String,
        /// The operations.
        ops: Vec<CapOp>,
    },
    /// `handler h(x: τ, ...) for K uses Κ { op o(y, ...) = e }`.
    Handler {
        /// The name.
        name: String,
        /// The parameters.
        params: Vec<(Name, Type)>,
        /// The capability it implements.
        capability: String,
        /// Its own capability context.
        uses: Vec<Use>,
        /// The operations.
        ops: Vec<HandlerOp>,
    },
    /// `fun f[α*](x: τ, ...) : τ uses Κ fails E = e`.
    Fun(Fun),
    /// `policy p : τ ⇝ τ via f`.
    Policy {
        /// The name.
        name: String,
        /// The `from` type.
        from: Type,
        /// The `to` type.
        to: Type,
        /// The `via` function.
        via: Path,
    },
}

/// An expectation of an example (`spec/08 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expectation {
    /// `expect e`, with `result` bound to the `do` line's value.
    Holds {
        /// The binder of `result`.
        result: Name,
        /// The condition.
        check: Expr,
        /// The line as written, for reports.
        text: String,
        /// Its line in the source.
        line: u32,
    },
    /// `expect fails V`.
    Fails {
        /// The variant.
        variant: String,
        /// The line as written.
        text: String,
        /// Its line in the source.
        line: u32,
    },
    /// `expect performed @x.op` or `expect not performed @x.op`.
    Performed {
        /// The capability reference.
        cap: Name,
        /// The operation.
        op: String,
        /// Whether it says `not performed`.
        negated: bool,
        /// The line as written.
        text: String,
        /// Its line in the source.
        line: u32,
    },
}

/// An example: its `given` lines around its `do` line, as nested `with` bindings, and its
/// expectations (`spec/09 §3`, `spec/11 §6`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    /// The name.
    pub name: String,
    /// The `do` line inside the `given` lines.
    pub body: Expr,
    /// The expectations, in order.
    pub expectations: Vec<Expectation>,
}

/// The Core IR of a module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// The module path.
    pub path: String,
    /// The declarations, in source order.
    pub decls: Vec<Decl>,
    /// The examples, in source order.
    pub examples: Vec<Example>,
}

/// The Core IR of a package: its modules, in path order, and the contracts of the standard
/// library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The modules.
    pub modules: Vec<Module>,
    /// The functions of the prelude and `std.testing`.
    pub library: Vec<Signature>,
}
