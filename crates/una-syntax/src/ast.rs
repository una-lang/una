//! The abstract syntax tree of `spec/02`, with spans on every node and comments as trivia.
//!
//! The tree mirrors the grammar productions closely, so the formatter (M2) can reprint any file
//! and the checker (M3 to M6) can cite the rule each node comes from.

use crate::intern::Symbol;
use crate::source::Span;
use crate::token::Comment;

/// A name and where it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ident {
    /// The interned name.
    pub name: Symbol,
    /// Where it is written.
    pub span: Span,
}

/// A source file: `module_header { import } { decl }` (`spec/02 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// The module header; absent only when the file does not start with one.
    pub module: Option<ModuleHeader>,
    /// The imports, in source order.
    pub imports: Vec<Import>,
    /// The declarations that parsed, in source order.
    pub decls: Vec<Decl>,
    /// Every comment of the file, in source order (`spec/01 §3`).
    pub comments: Vec<Comment>,
}

/// `[ "trusted" ] "module" module_path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleHeader {
    /// Whether the module is declared `trusted` (`spec/07 §6`).
    pub trusted: bool,
    /// The module path.
    pub path: ModulePath,
    /// The whole header line.
    pub span: Span,
}

/// `lower_ident { "." lower_ident }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModulePath {
    /// The segments, in order.
    pub segments: Vec<Ident>,
    /// The whole path.
    pub span: Span,
}

/// `"import" module_path "(" import_name { "," import_name } ")"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// The imported module.
    pub module: ModulePath,
    /// The imported names, lower or upper case.
    pub names: Vec<Ident>,
    /// The whole import line.
    pub span: Span,
}

/// A declaration with its doc comments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    /// The doc comments (`---`) attached to the declaration (`spec/01 §3`).
    pub docs: Vec<Comment>,
    /// The declaration.
    pub kind: DeclKind,
    /// From its first token to its last.
    pub span: Span,
}

/// The declarations of `spec/02 §1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclKind {
    /// `kind`.
    Kind(KindDecl),
    /// `predicate`.
    Predicate(PredicateDecl),
    /// A function.
    Function(FunctionDecl),
    /// `capability`.
    Capability(CapabilityDecl),
    /// `handler`.
    Handler(HandlerDecl),
    /// `policy`.
    Policy(PolicyDecl),
    /// `requirement`.
    Requirement(RequirementDecl),
    /// `example`.
    Example(ExampleDecl),
    /// `measure`, edition 0.3 (ADR 0104 rule 1).
    Measure(MeasureDecl),
    /// A module constant, `[pub] const name: type = value`, edition 0.5 (ADR 0126 rule 1).
    Const(ConstDecl),
}

/// `constant_decl = [ "pub" ] "const" lower_ident ":" type "=" line_expr NEWLINE`, edition 0.5
/// (ADR 0126 rule 1): a name bound, for the module, to a value the compiler computes at check
/// time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstDecl {
    /// Whether the constant is `pub`.
    pub public: bool,
    /// The constant's name.
    pub name: Ident,
    /// Its declared type, which the declaration MUST write.
    pub ty: Type,
    /// Its value, a constant expression (ADR 0126 rule 3).
    pub value: Expr,
}

/// `[ "pub" ] "measure" upper_ident [ "=" unit_expr ]`, edition 0.3 (ADR 0104 rule 1): a base
/// unit without a body, a derived unit with one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeasureDecl {
    /// Whether the unit is `pub`.
    pub public: bool,
    /// The unit's name.
    pub name: Ident,
    /// The unit expression of a derived unit.
    pub unit: Option<UnitExpr>,
}

/// A unit expression (ADR 0104 rule 2): `unit_term { ( "*" | "/" ) unit_term }`, left to right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitExpr {
    /// A declared unit, by name.
    Name(Ident),
    /// An integer factor, as written.
    Factor {
        /// Its decimal digits.
        digits: String,
        /// Where it is written.
        span: Span,
    },
    /// A product.
    Mul(Box<UnitExpr>, Box<UnitExpr>),
    /// A quotient.
    Div(Box<UnitExpr>, Box<UnitExpr>),
    /// A parenthesized unit expression.
    Paren(Box<UnitExpr>, Span),
}

impl UnitExpr {
    /// From its first token to its last.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            UnitExpr::Name(name) => name.span,
            UnitExpr::Factor { span, .. } | UnitExpr::Paren(_, span) => *span,
            UnitExpr::Mul(a, b) | UnitExpr::Div(a, b) => a.span().to(b.span()),
        }
    }
}

/// `[ "pub" ] "kind" upper_ident [ type_params ] kind_body` (`spec/02 §2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindDecl {
    /// Whether the kind is `pub`.
    pub public: bool,
    /// The kind's name.
    pub name: Ident,
    /// The type parameters.
    pub params: Vec<Ident>,
    /// The body.
    pub body: KindBody,
}

/// The three bodies of a kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindBody {
    /// `"=" type [ "where" line_expr ]`: an alias or a refinement.
    Alias {
        /// The base type.
        ty: Box<Type>,
        /// The `where` predicate.
        refinement: Option<Box<Expr>>,
    },
    /// `"=" variant "|" variant ...`: a sum.
    Sum(Vec<Variant>),
    /// An indented list of fields: a record.
    Record(Vec<Field>),
}

/// `upper_ident [ "(" field_sig { "," field_sig } ")" ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    /// The variant's name.
    pub name: Ident,
    /// The payload fields.
    pub fields: Vec<Field>,
    /// The whole variant.
    pub span: Span,
}

/// `lower_ident ":" type`: a record field or a variant payload field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The field's name.
    pub name: Ident,
    /// The field's type.
    pub ty: Type,
    /// The whole field.
    pub span: Span,
}

/// `[ label ] type_core [ "in" range ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    /// The information-flow label (`spec/03 §5`).
    pub label: Option<Label>,
    /// The type itself.
    pub core: TypeCore,
    /// The unit of a measured type, after `of` (ADR 0104 rule 2), edition 0.3.
    pub unit: Option<UnitExpr>,
    /// The range sugar (`spec/03 §3`).
    pub range: Option<Range>,
    /// The whole type.
    pub span: Span,
}

/// `level [ "[" upper_ident { "," upper_ident } "]" ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The level.
    pub level: Level,
    /// The categories.
    pub categories: Vec<Ident>,
    /// The whole label.
    pub span: Span,
}

/// The levels of `spec/07 §2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// `public`
    Public,
    /// `internal`
    Internal,
    /// `confidential`
    Confidential,
    /// `secret`
    Secret,
    /// `restricted`
    Restricted,
}

/// `type_core` of `spec/02 §2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeCore {
    /// `upper_ident [ "[" type { "," type } "]" ]`.
    Named {
        /// The type's name.
        name: Ident,
        /// The type arguments.
        args: Vec<Type>,
    },
    /// `"[" type "]"`: a list.
    List(Box<Type>),
    /// `"Decimal" "[" scale "]"`, edition 0.4 (`spec/29 §16`, ADR 0120 rule 2): an exact
    /// decimal type whose scale is an integer literal.
    Decimal {
        /// The word `Decimal`.
        name: Ident,
        /// The scale's digits, as its integer token holds them.
        scale: String,
        /// The scale's span.
        scale_span: Span,
    },
    /// `"fn" "(" [ types ] ")" "->" type { fn_type_clause }`.
    Function {
        /// The parameter types.
        params: Vec<Type>,
        /// The result type.
        result: Box<Type>,
        /// The `uses` and `fails` of the function type.
        clauses: Vec<FnTypeClause>,
    },
}

/// `"uses" cap_ref ":" cap_type | "fails" upper_ident` in a function type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FnTypeClause {
    /// `uses @c: C[ops]`.
    Uses(UsesClause),
    /// `fails E`.
    Fails(Ident),
}

/// `cap_ref ":" cap_type`, after `uses`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsesClause {
    /// The capability reference, without `@`.
    pub cap: Ident,
    /// The capability type.
    pub cap_type: CapType,
    /// From `uses` to the end of the capability type.
    pub span: Span,
}

/// `upper_ident [ "[" cap_op { "," cap_op } "]" ]`, where `cap_op` is `lower_ident [ "where"
/// line_expr ]` (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapType {
    /// The capability's name.
    pub name: Ident,
    /// The operations granted.
    pub ops: Vec<Ident>,
    /// The condition of each operation in `ops`, in their order: `Some` after `where`, which
    /// only the `uses` of a function or a handler in edition 0.3 may write (ADR 0092 rule 1).
    pub conditions: Vec<Option<Expr>>,
    /// The label each operation in `ops` gives, in their order: `Some` after `gives`, which
    /// only the `uses` of a function or a handler in edition 0.3 may write, before the
    /// condition (ADR 0094 rule 1).
    pub gives: Vec<Option<Label>>,
    /// The whole capability type.
    pub span: Span,
}

/// `signed_int ( ".." | "..<" ) signed_int`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    /// The lower bound.
    pub low: SignedInt,
    /// The upper bound.
    pub high: SignedInt,
    /// Whether the upper bound is excluded (`..<`).
    pub exclusive: bool,
    /// The whole range.
    pub span: Span,
}

/// `[ "-" ] int_lit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInt {
    /// Whether it has a leading `-`.
    pub negative: bool,
    /// The decimal digits, without underscores or leading zeros; empty for a bound that names
    /// a constant.
    pub digits: String,
    /// The module constant a bound of a type's range names, edition 0.5 (ADR 0126 rule 9),
    /// which the checker substitutes before the range is read.
    pub constant: Option<Box<Ident>>,
    /// The whole integer.
    pub span: Span,
}

/// `[ "pub" ] "predicate" ( "reflect" | "opaque" ) lower_ident "(" [ params ] ")" "->" type "="
/// inline_or_block` (`spec/02 §3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredicateDecl {
    /// Whether the predicate is `pub`.
    pub public: bool,
    /// Whether the predicate is `reflect` (visible to the solver) or `opaque`.
    pub reflect: bool,
    /// The predicate's name.
    pub name: Ident,
    /// The parameters.
    pub params: Vec<Param>,
    /// The result type.
    pub result: Type,
    /// The body.
    pub body: Body,
}

/// A function, in short or long form (`spec/02 §3`, `spec/04 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDecl {
    /// Whether the function is `pub`.
    pub public: bool,
    /// The function's name.
    pub name: Ident,
    /// The type parameters.
    pub type_params: Vec<Ident>,
    /// The constraint written on each type parameter, as `Ord` in `[T: Ord]`, from edition
    /// 0.2 (`spec/20 §2`); as long as `type_params`.
    pub constraints: Vec<Option<Ident>>,
    /// The parameters.
    pub params: Vec<Param>,
    /// The result type.
    pub result: Type,
    /// The header clauses, in source order.
    pub clauses: Vec<Clause>,
    /// The body: a line in the short form, a block in the long form.
    pub body: Body,
}

/// `lower_ident ":" type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// Its convention, `edit` or `take`, from edition 0.2 (`spec/17 §3`).
    pub convention: Option<Convention>,
    /// The parameter's name.
    pub name: Ident,
    /// The parameter's type.
    pub ty: Type,
    /// The whole parameter.
    pub span: Span,
}

/// A parameter convention, and its call-site marker (`spec/17 §3`, ADR 0056).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Convention {
    /// `edit`: passed in and out.
    Edit,
    /// `take`: consumed.
    Take,
}

impl Convention {
    /// The convention as written.
    pub fn as_str(self) -> &'static str {
        match self {
            Convention::Edit => "edit",
            Convention::Take => "take",
        }
    }
}

/// A header clause of a function (`spec/02 §3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    /// The clause.
    pub kind: ClauseKind,
    /// The clause keyword, where U0102 points.
    pub keyword: Span,
    /// The whole clause.
    pub span: Span,
}

/// The clauses of a function header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClauseKind {
    /// `requires line_expr`.
    Requires(Expr),
    /// `ensures line_expr`.
    Ensures(Expr),
    /// `uses cap_ref: cap_type`.
    Uses(UsesClause),
    /// `fails upper_ident`.
    Fails(Ident),
    /// `uses of lower_ident`: the effects of a parameter of function type, from edition 0.2
    /// (ADR 0062).
    UsesOf(Ident),
    /// `fails of lower_ident`: the failures of a parameter of function type, from edition 0.2
    /// (ADR 0062).
    FailsOf(Ident),
    /// `derived from upper_ident { "," upper_ident }`.
    DerivedFrom(Vec<Ident>),
    /// `diverges`, in edition 0.4: the function may run forever (`spec/26 §5`).
    Diverges,
}

/// A body: a single line, or an indented block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// `line_expr` on the declaration's line.
    Line(Expr),
    /// An indented block.
    Block(Block),
}

/// `[ "pub" ] "capability" upper_ident` and its operations (`spec/02 §4`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityDecl {
    /// Whether the capability is `pub`.
    pub public: bool,
    /// The capability's name.
    pub name: Ident,
    /// The operations.
    pub ops: Vec<OpDecl>,
}

/// `"op" lower_ident "(" [ params ] ")" "->" type` and its clauses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpDecl {
    /// The operation's name.
    pub name: Ident,
    /// The parameters.
    pub params: Vec<Param>,
    /// The result type.
    pub result: Type,
    /// The clauses, in source order.
    pub clauses: Vec<OpClause>,
    /// The whole operation.
    pub span: Span,
}

/// A clause of a capability operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpClause {
    /// The clause.
    pub kind: OpClauseKind,
    /// The clause keyword, where U0102 points.
    pub keyword: Span,
    /// The whole clause.
    pub span: Span,
}

/// The clauses of a capability operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpClauseKind {
    /// `requires line_expr`.
    Requires(Expr),
    /// `ensures line_expr`.
    Ensures(Expr),
    /// `fails upper_ident`.
    Fails(Ident),
    /// `accepts label`.
    Accepts(Label),
}

/// `[ "pub" ] "handler" lower_ident "(" [ params ] ")" "for" upper_ident` and its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerDecl {
    /// Whether the handler is `pub`.
    pub public: bool,
    /// The handler's name.
    pub name: Ident,
    /// The parameters.
    pub params: Vec<Param>,
    /// The capability it implements.
    pub capability: Ident,
    /// The capabilities it uses.
    pub uses: Vec<UsesClause>,
    /// The operations it implements.
    pub ops: Vec<HandlerOp>,
}

/// `"op" lower_ident "(" [ lower_ident { "," lower_ident } ] ")" "=" inline_or_block`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerOp {
    /// The operation's name.
    pub name: Ident,
    /// The parameter names.
    pub params: Vec<Ident>,
    /// The body.
    pub body: Body,
    /// The whole operation.
    pub span: Span,
}

/// `[ "pub" ] "policy" lower_ident` and its five lines (`spec/02 §4`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyDecl {
    /// Whether the policy is `pub`.
    pub public: bool,
    /// The policy's name.
    pub name: Ident,
    /// `from type`.
    pub from: Type,
    /// `to type`.
    pub to: Type,
    /// `via lower_ident`.
    pub via: Ident,
    /// `purpose text_lit`.
    pub purpose: TextLit,
    /// `owner lower_ident`.
    pub owner: Ident,
}

/// `"requirement" upper_ident` and its four lines (`spec/02 §5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequirementDecl {
    /// The requirement's identifier.
    pub id: Ident,
    /// `text text_lit`.
    pub text: TextLit,
    /// `source ...`.
    pub source: RequirementSource,
    /// `owner lower_ident`.
    pub owner: Ident,
    /// `status proposed | active`.
    pub active: bool,
    /// Where `proposed` or `active` is written.
    pub status: Span,
}

/// `source = "human" | ( "policy" | "regulation" | "generated" ) text_lit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequirementSource {
    /// `human`.
    Human,
    /// `policy "<document>"`.
    Policy(TextLit),
    /// `regulation "<document>"`.
    Regulation(TextLit),
    /// `generated "<task reference>"`.
    Generated(TextLit),
}

/// `"example" text_lit` and its lines (`spec/02 §5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleDecl {
    /// The example's name.
    pub name: TextLit,
    /// The `given` lines.
    pub givens: Vec<Given>,
    /// The `do` expression.
    pub action: Expr,
    /// The `expect` lines.
    pub expectations: Vec<Expectation>,
    /// The final `stop after N` line, in edition 0.4 (ADR 0116 rule 19, `spec/26 §8`).
    pub stop_after: Option<StopAfter>,
}

/// `"stop" "after" integer_literal`: the stop schedule of an example, whose `do` line may then
/// run a `for ever` loop (ADR 0116 rule 19, `spec/26 §8`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopAfter {
    /// `N`: a host stop arrives once `N` iterations of `for ever` loops have started, from 1.
    pub iterations: u64,
    /// The whole line.
    pub span: Span,
}

/// `"given" cap_ref "=" handler_call`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given {
    /// The capability reference, without `@`.
    pub cap: Ident,
    /// The handler installed for it.
    pub handler: HandlerCall,
    /// The whole line.
    pub span: Span,
}

/// `lower_ident "(" [ args ] ")"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerCall {
    /// The handler's name.
    pub name: Ident,
    /// The arguments.
    pub args: Vec<Arg>,
    /// The whole call.
    pub span: Span,
}

/// An `expect` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expectation {
    /// The expectation.
    pub kind: ExpectationKind,
    /// The whole line.
    pub span: Span,
}

/// The forms of an `expect` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectationKind {
    /// `expect fails V`.
    Fails(Ident),
    /// `expect [not] performed @x.op`.
    Performed {
        /// Whether it is `not performed`.
        negated: bool,
        /// The capability reference, without `@`.
        cap: Ident,
        /// The operation.
        op: Ident,
    },
    /// `expect line_expr`.
    Holds(Expr),
}

/// `{ statement } final`: the statements of a block, the last of which is its final
/// expression (`spec/02 §6`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The statements; the last one is the block's value.
    pub stmts: Vec<Stmt>,
    /// The whole block.
    pub span: Span,
}

/// A statement of a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `let_stmt`.
    Let(Box<LetStmt>),
    /// `place = value`, from edition 0.2 (`spec/17 §2`).
    Assign(Box<AssignStmt>),
    /// `if` with a block and no `else`, a `Unit` statement, from edition 0.2 (`spec/17 §1`).
    IfThen(Box<IfThenStmt>),
    /// `for x in source` with a block, a `Unit` statement, from edition 0.2 (`spec/17 §1`).
    For(Box<ForStmt>),
    /// `together` with its branches, from edition 0.2 (`spec/19 §1`).
    Together(Box<TogetherStmt>),
    /// `handled`.
    Handled(Handled),
    /// `block_expr`, only as the final statement.
    BlockExpr(BlockExpr),
    /// `stop [when c]`, in edition 0.4: ends the innermost loop (`spec/26 §3`).
    Stop(Box<StopStmt>),
    /// `skip`, in edition 0.4: ends the iteration (`spec/26 §3`). The span of the word.
    Skip(Span),
    /// `for until c decreases m` with a block, in edition 0.4 (`spec/26 §4`).
    Until(Box<UntilStmt>),
    /// `for ever` with a block and an optional `on stop`, in edition 0.4 (`spec/26 §5`).
    Ever(Box<EverStmt>),
    /// `on stop` with a block that follows a loop other than `for ever`, which U2034 reports
    /// (`spec/26 §5`).
    OnStop(Box<OnStop>),
    /// `tasks [at most n at once]` with a block, in edition 0.4 (`spec/27 §7`).
    Tasks(Box<TasksStmt>),
    /// `start` with a handled expression, in edition 0.4 (`spec/27 §7`).
    Start(Box<StartStmt>),
}

/// `"tasks" [ "at" "most" add_expr "at" "once" ] NEWLINE INDENT block DEDENT` (`spec/27 §7`,
/// ADR 0117 rule 7): a block that runs in the current task and may start tasks with `start`,
/// all of which end before the statement does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TasksStmt {
    /// The bound after `at most`, when there is one.
    pub bound: Option<Expr>,
    /// The block.
    pub block: Block,
    /// The word `tasks`.
    pub keyword: Span,
    /// The whole statement.
    pub span: Span,
}

/// `"start" line_expr [ "within" add_expr ] NEWLINE [ INDENT handling { handling } DEDENT ]`
/// (`spec/27 §7`, ADR 0117 rule 7): starts a task for the expression and goes on. A `with`
/// clause parses, so that the checker reports it (U2035).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartStmt {
    /// The started expression, with its deadline and handling clauses.
    pub handled: Handled,
    /// The word `start`.
    pub keyword: Span,
    /// The whole statement.
    pub span: Span,
}

/// `"for" "ever" [ "within" add_expr ] NEWLINE INDENT block DEDENT [ on_stop ]` (`spec/26 §5`,
/// §7, ADR 0116 rules 10 and 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EverStmt {
    /// The deadline of each iteration, after `within`, when there is one (`spec/26 §7`).
    pub within: Option<Expr>,
    /// The body.
    pub block: Block,
    /// The `on stop` block, when there is one.
    pub on_stop: Option<OnStop>,
    /// The whole statement.
    pub span: Span,
}

/// `"on" "stop" NEWLINE INDENT block DEDENT` (`spec/26 §5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnStop {
    /// The block.
    pub block: Block,
    /// The words `on stop`.
    pub keyword: Span,
    /// The whole `on stop` and its block.
    pub span: Span,
}

/// `"for" "until" line_expr "decreases" add_expr NEWLINE INDENT block DEDENT` (`spec/26 §4`,
/// ADR 0116 rule 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UntilStmt {
    /// `c`: the loop ends at the start of an iteration where it holds.
    pub condition: Expr,
    /// The word `decreases`, where the measure's obligations are reported.
    pub decreases: Span,
    /// `m`: the measure, for the verifier only.
    pub measure: Expr,
    /// The body.
    pub block: Block,
    /// The whole statement.
    pub span: Span,
}

/// `"stop" [ "when" line_expr ]` (`spec/26 §3`, ADR 0116 rule 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopStmt {
    /// The condition, when the statement is `stop when c`.
    pub condition: Option<Expr>,
    /// The word `stop`.
    pub keyword: Span,
    /// The whole statement.
    pub span: Span,
}

/// `place "=" ( handled | block_expr )` (`spec/17 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignStmt {
    /// The place: a name, or a field path of one.
    pub place: Expr,
    /// The value.
    pub value: LetValue,
    /// The whole statement.
    pub span: Span,
}

/// `"if" line_expr NEWLINE INDENT block DEDENT`, with no `else` (`spec/17 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfThenStmt {
    /// The condition.
    pub condition: Expr,
    /// The block run when it holds.
    pub block: Block,
    /// The whole statement.
    pub span: Span,
}

/// `"for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT`
/// (`spec/17 §1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForStmt {
    /// The loop variable.
    pub var: Ident,
    /// What the loop visits.
    pub source: ForSource,
    /// The body.
    pub block: Block,
    /// The whole statement.
    pub span: Span,
}

/// `"together" NEWLINE INDENT let_stmt let_stmt { let_stmt } DEDENT` (`spec/19 §1`). The
/// body is kept as parsed; the checker reports a statement other than `let`, or fewer than
/// two branches (U2024).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TogetherStmt {
    /// The statements of the body, each `let` a branch.
    pub body: Vec<Stmt>,
    /// The `together` keyword, where U2024 points.
    pub keyword: Span,
    /// The whole statement.
    pub span: Span,
}

/// `"together" "for" lower_ident "in" ( line_expr | range_expr ) NEWLINE INDENT block DEDENT`
/// (`spec/19 §1`): one task per element, whose results make a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TogetherFor {
    /// The variable of each task.
    pub var: Ident,
    /// The elements.
    pub source: ForSource,
    /// The body, whose final expression is the value of an element's task.
    pub block: Block,
    /// The `together` keyword.
    pub keyword: Span,
    /// The whole expression.
    pub span: Span,
}

/// `"first" "of" NEWLINE INDENT handled handled { handled } DEDENT` (`spec/27 §6`, ADR 0117
/// rule 6): a race of its branches, whose value is the first branch to return. The body is
/// kept as parsed; the checker reports a line that is not a `handled` expression, or fewer
/// than two branches (U2035).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirstOf {
    /// The statements of the body, each `handled` expression a branch.
    pub body: Vec<Stmt>,
    /// The words `first of`, where U2035 points.
    pub keyword: Span,
    /// The whole expression.
    pub span: Span,
}

/// What a `for` loop visits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForSource {
    /// The elements of a list, in order.
    List(Expr),
    /// The integers of `low..high` or `low..<high`, in increasing order.
    Range {
        /// The lower bound.
        low: Expr,
        /// The upper bound.
        high: Expr,
        /// Whether the upper bound is excluded (`..<`).
        exclusive: bool,
    },
}

/// `"let" [ "edit" ] lower_ident [ ":" type ] "=" ( handled | block_expr )`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LetStmt {
    /// Whether the binding is declared `let edit`, from edition 0.2 (`spec/17 §2`), or `var`
    /// in edition 0.5 (ADR 0137 rule 2).
    pub editable: bool,
    /// The bound name.
    pub name: Ident,
    /// The annotated type.
    pub ty: Option<Type>,
    /// The value.
    pub value: LetValue,
    /// The whole statement.
    pub span: Span,
}

/// The value of a `let`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LetValue {
    /// A line expression with its handling clauses.
    Handled(Handled),
    /// A `when` or block `if`.
    BlockExpr(BlockExpr),
}

/// `line_expr [ with_clause ] [ "within" add_expr ]` and its handling clauses; the deadline
/// from edition 0.4 (`spec/27 §5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handled {
    /// The expression.
    pub expr: Expr,
    /// The `with` bindings.
    pub with: Vec<CapBinding>,
    /// The deadline after `within`, when there is one (ADR 0117 rule 5).
    pub within: Option<Expr>,
    /// The handling clauses (`if error ...`, `if none ...`).
    pub handlings: Vec<Handling>,
    /// The whole statement.
    pub span: Span,
}

/// `cap_ref "=" ( cap_ref [ "[" lower_ident { "," lower_ident } "]" ] | handler_call )`; the
/// brackets only in edition 0.3 (ADR 0093 rule 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapBinding {
    /// The bound capability reference, without `@`.
    pub cap: Ident,
    /// What it is bound to.
    pub value: CapBindingValue,
    /// The whole binding.
    pub span: Span,
}

/// The right-hand side of a capability binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapBindingValue {
    /// Another capability reference, without `@`.
    Cap(Ident),
    /// `@y[ops]`: another capability reference restricted to some of its operations, in
    /// edition 0.3 (ADR 0093 rule 4).
    Narrowed {
        /// The capability reference, without `@`.
        target: Ident,
        /// The operations it is restricted to, at least one.
        ops: Vec<Ident>,
    },
    /// A handler instance.
    Handler(HandlerCall),
}

/// A handling clause (`spec/06 §3`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handling {
    /// The clause.
    pub kind: HandlingKind,
    /// The `if` keyword, where U0102 points.
    pub keyword: Span,
    /// The whole line.
    pub span: Span,
}

/// The handling clauses of `spec/02 §6`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandlingKind {
    /// `if error fail V`.
    ErrorFail(VariantExpr),
    /// `if error pass`.
    ErrorPass,
    /// `if error use line_expr`.
    ErrorUse(Expr),
    /// `if none fail V`.
    NoneFail(VariantExpr),
    /// `if none use line_expr`.
    NoneUse(Expr),
}

/// `upper_ident [ call_args ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantExpr {
    /// The variant's name.
    pub name: Ident,
    /// The payload arguments.
    pub args: Vec<Arg>,
    /// The whole expression.
    pub span: Span,
}

/// `when_expr | if_block`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockExpr {
    /// `when`.
    When(WhenExpr),
    /// A block `if`.
    If(IfBlock),
    /// `together for`, from edition 0.2 (`spec/19 §1`).
    TogetherFor(Box<TogetherFor>),
    /// `first of`, in edition 0.4 (`spec/27 §6`, ADR 0117 rule 6).
    FirstOf(Box<FirstOf>),
}

/// `"when" line_expr` and its arms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhenExpr {
    /// The value examined.
    pub scrutinee: Expr,
    /// The arms, in order.
    pub arms: Vec<Arm>,
    /// The `when` keyword, where U2007 and exhaustiveness obligations point.
    pub keyword: Span,
    /// The whole expression.
    pub span: Span,
}

/// `pattern "->" ( line_expr | block )`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    /// The pattern.
    pub pattern: Pattern,
    /// The arm's value.
    pub body: Body,
    /// The whole arm.
    pub span: Span,
}

/// A `when` pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// The pattern.
    pub kind: PatternKind,
    /// The whole pattern.
    pub span: Span,
}

/// The patterns of `spec/02 §6`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    /// `is V( binds )`.
    Variant {
        /// The variant's name.
        name: Ident,
        /// The bound fields.
        binds: Vec<Bind>,
    },
    /// `is literal`.
    Literal(Literal),
    /// `in range`.
    Range(Range),
    /// `else`.
    Else,
}

/// `lower_ident [ ":" lower_ident ]`: a field, or a field and its local name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    /// The field.
    pub field: Ident,
    /// The local name, when it differs from the field.
    pub local: Option<Ident>,
}

/// `"if" line_expr` with a block, then `else` with a block or another block `if`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfBlock {
    /// The condition.
    pub condition: Expr,
    /// The block run when it holds.
    pub then_block: Block,
    /// The `else` branch.
    pub otherwise: Box<ElseBranch>,
    /// The whole expression.
    pub span: Span,
}

/// The `else` of a block `if`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElseBranch {
    /// `else` and a block.
    Block(Block),
    /// `else if ...`.
    If(Box<IfBlock>),
}

/// An expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// The expression.
    pub kind: ExprKind,
    /// The whole expression.
    pub span: Span,
}

/// The expressions of `spec/02 §7`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// `if c then a else b`.
    IfLine {
        /// The condition.
        condition: Box<Expr>,
        /// The value when it holds.
        then: Box<Expr>,
        /// The value otherwise.
        otherwise: Box<Expr>,
    },
    /// `fn(params) -> body`.
    Lambda {
        /// The parameters.
        params: Vec<LambdaParam>,
        /// The body.
        body: Box<Expr>,
    },
    /// `checked(value, predicate) else fail V`.
    Checked {
        /// The value checked.
        value: Box<Expr>,
        /// The predicate, over `it`.
        predicate: Box<Expr>,
        /// The variant raised when the check fails.
        failure: VariantExpr,
    },
    /// `attempt postfix [ "within" add_expr ]`; the deadline from edition 0.4 (`spec/27 §5`).
    Attempt {
        /// The expression attempted.
        operand: Box<Expr>,
        /// The deadline after `within`, when there is one.
        within: Option<Box<Expr>>,
    },
    /// `fail V`.
    Fail(VariantExpr),
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        left: Box<Expr>,
        /// The right operand.
        right: Box<Expr>,
    },
    /// A unary operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: Box<Expr>,
    },
    /// `e is V` or `e is literal`.
    Is {
        /// The value tested.
        value: Box<Expr>,
        /// The variant or literal.
        target: IsTarget,
    },
    /// `e in range`.
    In {
        /// The value tested.
        value: Box<Expr>,
        /// The range.
        range: Range,
    },
    /// `e.name`: a field, or a one-parameter function (`spec/04 §3`).
    Field {
        /// The value.
        target: Box<Expr>,
        /// The name after the dot.
        name: Ident,
    },
    /// `old(e)` in an `ensures` clause, from edition 0.2 (`spec/17 §3`).
    Old(Box<Expr>),
    /// `e.name(args)`: a method call (`spec/04 §3`).
    MethodCall {
        /// The receiver's call-site marker, as in `edit total.record(e)` (`spec/17 §3`).
        receiver: Option<Convention>,
        /// The receiver.
        target: Box<Expr>,
        /// The method's name.
        name: Ident,
        /// The arguments.
        args: Vec<Arg>,
    },
    /// `e(args)`: a call.
    Call {
        /// The function or constructor called.
        callee: Box<Expr>,
        /// The arguments.
        args: Vec<Arg>,
    },
    /// A lower-case name.
    Name(Ident),
    /// An upper-case name: a constructor, variant or kind.
    UpperName(Ident),
    /// `@c.op(args)`: a capability operation.
    CapOp {
        /// The capability reference, without `@`.
        cap: Ident,
        /// The operation.
        op: Ident,
        /// The arguments.
        args: Vec<Arg>,
    },
    /// A bare `@c`, accepted only to report U3005 (`spec/02 §7`).
    CapRef(Ident),
    /// `( e )`.
    Paren(Box<Expr>),
    /// `[ a, b, c ]`.
    List(Vec<Expr>),
    /// `result`.
    Result,
    /// `it`.
    It,
    /// `error`.
    Error,
    /// A literal.
    Literal(Literal),
}

/// What `is` tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IsTarget {
    /// A variant.
    Variant(Ident),
    /// A literal.
    Literal(Literal),
}

/// `lower_ident [ ":" type ]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LambdaParam {
    /// The parameter's name.
    pub name: Ident,
    /// The annotated type.
    pub ty: Option<Type>,
}

/// `[ lower_ident ":" ] line_expr`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arg {
    /// The label.
    pub label: Option<Ident>,
    /// The call-site marker, `edit` or `take`, from edition 0.2 (`spec/17 §3`).
    pub marker: Option<Convention>,
    /// The value.
    pub value: Expr,
    /// The whole argument.
    pub span: Span,
}

/// The binary operators of `spec/01 §7`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinaryOp {
    /// `implies`
    Implies,
    /// `or`
    Or,
    /// `and`
    And,
    /// `==`
    Eq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `mod`
    Mod,
}

/// The unary operators of `spec/01 §7`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnaryOp {
    /// `-`
    Neg,
    /// `not`
    Not,
}

/// `literal` of `spec/02 §7`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Literal {
    /// An integer: its decimal digits.
    Int(String),
    /// A float literal of edition 0.3, as written (`spec/29`).
    Float(String),
    /// A text literal.
    Text(TextLit),
    /// `true` or `false`.
    Bool(bool),
    /// `unit`.
    Unit,
}

/// A text literal and its interpolations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLit {
    /// The literal text and the interpolated expressions, in order.
    pub parts: Vec<TextPart>,
    /// The whole literal, quotes included.
    pub span: Span,
}

/// A part of a text literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPart {
    /// Literal text, with escapes decoded.
    Text(String),
    /// `{ line_expr }`.
    Interpolation(Expr),
}
