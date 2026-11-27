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

impl Type {
    /// The join of the labels at every position of the type that holds data: its own label,
    /// and its type arguments' deeply, the elements of a list or a map included. A function
    /// type holds no data but itself. The interpreter and the native backend redact the values
    /// of a trace, and the fields of a kind, by it (`spec/11 §5`); one definition keeps their
    /// traces alike (#415).
    pub fn deep_label(&self) -> Label {
        match self {
            Type::Base { base, label, .. } => match base {
                Base::Named { args, .. } => args
                    .iter()
                    .fold(label.clone(), |acc, arg| acc.join(&arg.deep_label())),
                _ => label.clone(),
            },
            Type::Fn { label, .. } => label.clone(),
        }
    }

    /// Whether the type is or holds a type parameter, or a type that inference left open,
    /// outside function types: its label is known only at run time, so U6001 does not show a
    /// field of such a type (`spec/04 §8`).
    pub fn has_param(&self) -> bool {
        match self {
            Type::Base { base, .. } => match base {
                Base::Param(_) | Base::Unknown => true,
                Base::Named { args, .. } => args.iter().any(Type::has_param),
                _ => false,
            },
            Type::Fn { .. } => false,
        }
    }
}

/// A fixed-width integer type of edition `0.2` (`spec/18 §1`, ADR 0057).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntType {
    /// `I8`.
    I8,
    /// `I16`.
    I16,
    /// `I32`.
    I32,
    /// `I64`.
    I64,
    /// `U8`.
    U8,
    /// `U16`.
    U16,
    /// `U32`.
    U32,
    /// `U64`, or `U32` for an `F32`.
    U64,
}

impl IntType {
    /// Every fixed-width type, in the order of `spec/18 §1`.
    pub const ALL: [IntType; 8] = [
        IntType::I8,
        IntType::I16,
        IntType::I32,
        IntType::I64,
        IntType::U8,
        IntType::U16,
        IntType::U32,
        IntType::U64,
    ];

    /// The type's name, as written.
    pub fn as_str(self) -> &'static str {
        match self {
            IntType::I8 => "I8",
            IntType::I16 => "I16",
            IntType::I32 => "I32",
            IntType::I64 => "I64",
            IntType::U8 => "U8",
            IntType::U16 => "U16",
            IntType::U32 => "U32",
            IntType::U64 => "U64",
        }
    }

    /// The type named `name`, if it is one.
    pub fn from_name(name: &str) -> Option<IntType> {
        IntType::ALL.into_iter().find(|t| t.as_str() == name)
    }

    /// The width in bits.
    pub fn bits(self) -> u32 {
        match self {
            IntType::I8 | IntType::U8 => 8,
            IntType::I16 | IntType::U16 => 16,
            IntType::I32 | IntType::U32 => 32,
            IntType::I64 | IntType::U64 => 64,
        }
    }

    /// Whether the type is signed.
    pub fn signed(self) -> bool {
        matches!(
            self,
            IntType::I8 | IntType::I16 | IntType::I32 | IntType::I64
        )
    }

    /// The least value: `-2^(n-1)` signed, `0` unsigned.
    pub fn least(self) -> i128 {
        if self.signed() {
            -(1i128 << (self.bits() - 1))
        } else {
            0
        }
    }

    /// The greatest value: `2^(n-1) - 1` signed, `2^n - 1` unsigned.
    pub fn greatest(self) -> i128 {
        if self.signed() {
            (1i128 << (self.bits() - 1)) - 1
        } else {
            (1i128 << self.bits()) - 1
        }
    }

    /// Whether every value of `self` is a value of `target`: a widening conversion
    /// (`spec/18 §4` rule 3).
    pub fn widens_to(self, target: IntType) -> bool {
        target.least() <= self.least() && self.greatest() <= target.greatest()
    }
}

/// A base type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base {
    /// `Int`.
    Int,
    /// A fixed-width integer type (`spec/18`).
    Fixed(IntType),
    /// `F64`, IEEE 754 binary64, edition 0.3 (`spec/29`, ADR 0114).
    F64,
    /// `Decimal[s]`, an exact decimal of scale s, edition 0.4 (`spec/29 §16`, ADR 0120): its
    /// value is an integer coefficient c, worth c × 10^-s.
    Decimal(u8),
    /// `F32`, IEEE 754 binary32, edition 0.4 (`spec/29`, ADR 0120 rule 5).
    F32,
    /// `Bool`.
    Bool,
    /// `Text`.
    Text,
    /// `Unit`.
    Unit,
    /// `Hashed`.
    Hashed,
    /// `ReplyRight` of `std.host`, edition 0.4 (ADR 0123 rule 1): an opaque handle that names
    /// one exchange of `HttpServer`, with equality and no constructor.
    ReplyRight,
    /// A kind with its type arguments, `List` and `Option` included.
    Named {
        /// The module that declares the kind: `prelude` for `List`, `Option` and `Result`.
        module: String,
        /// The kind's name.
        name: String,
        /// Its type arguments.
        args: Vec<Type>,
    },
    /// A type parameter.
    Param(String),
    /// A tuple, internal to the desugaring of the memory model: never surface syntax
    /// (`spec/17 §4`, `spec/09 §2`).
    Tuple(Vec<Type>),
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
    /// (Never set on a function type: `gives` is U0101 there.)
    /// The label each operation gives through the binding, `op gives l` (ADR 0094 rule 1),
    /// declared or inferred, in the order of `ops`; an operation absent here gives `public`,
    /// so the list is empty when none is labeled, as always before edition 0.3. The formal
    /// track's re-check reads a declaration with a labeled operation as not re-checked until
    /// F1 models the labels (ADR 0094 rule 8, #563).
    pub gives: Vec<(String, Label)>,
    /// The conditions written on its operations (ADR 0092), each with its operation, in the
    /// order written; empty when none is written, as always before edition 0.3.
    pub conds: Vec<(String, Clause)>,
}

/// A locked condition of a function's operation (ADR 0092 rule 8, as amended): the header's
/// condition on the operation must imply it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// The binding, without `@`.
    pub cap: Name,
    /// The operation.
    pub op: String,
    /// The condition the header writes on the operation, if any.
    pub header: Option<Clause>,
    /// The lock's condition.
    pub lock: Clause,
    /// Where the header grants the operation.
    pub at: Span,
    /// Where the lock entry is written in `una.toml`.
    pub entry: Span,
    /// The edit that writes the lock's condition back on the header's operation: its range and
    /// replacement, when conditions (a), (b) and (d) hold (ADR 0092, amendment of 2026-10-15).
    /// The verifier keeps it under (c), and the report gives it under (e).
    pub edit: Option<(Span, String)>,
}

/// A term of the refinement language (`spec/03 §3`): a tree over binders, `it` and
/// `result`, since formulas are not A-normal.
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
    /// A shape function of a dense value other than `len`: `m.rows`, `m.columns`, `t.rank`,
    /// `f.size` (`spec/29 §17`, ADR 0120 rules 33 and 38).
    Shape {
        /// Which function.
        fun: ShapeFun,
        /// The dense value.
        value: Box<Term>,
    },
    /// `a.get(index: i)`, `m.get(row: i, column: j)` or `t.get(at: [i, ...])` of a dense
    /// value, in the refinement language (`spec/29 §17`, ADR 0120 rule 33): the element at the
    /// indices, which a precondition keeps in range wherever code reads it.
    Element {
        /// The dense value.
        value: Box<Term>,
        /// The indices, one per axis.
        indices: Vec<Term>,
        /// Whether the value is a tensor, whose indices are written as one list literal.
        tensor: bool,
    },
    /// `t.dim(axis: k)` of a tensor (`spec/29 §17`, ADR 0120 rule 33): the length of axis k.
    /// A program writes k as an integer literal (U2038); the prelude's contracts also write
    /// it as a term over their parameters.
    Dim {
        /// The tensor.
        value: Box<Term>,
        /// The axis.
        axis: Box<Term>,
    },
    /// `m.has(key: k)` of a map (`spec/21 §4` rule 4), or of a `HashMap` (ADR 0119 V5).
    Has {
        /// The map.
        map: Box<Term>,
        /// The key.
        key: Box<Term>,
    },
    /// `s.has(item: x)` of a `Set`, a `HashSet` or a `BitSet` (ADR 0119 V5).
    Member {
        /// The set.
        set: Box<Term>,
        /// The item.
        item: Box<Term>,
    },
    /// `b.size` of a `BitSet` (ADR 0119 V5).
    Size(Box<Term>),
    /// The component `index` of a tuple, as `result` of a function with `edit` parameters
    /// (`spec/17 §4`).
    Proj {
        /// The tuple.
        value: Box<Term>,
        /// The component's position.
        index: usize,
    },
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
    /// A float literal, `nan()` or `infinity()`, as its bits (`spec/29 §8`).
    F64(u64),
    /// An operation of `F64` of the refinement language, its receiver first (`spec/29 §8`).
    Float {
        /// The operation.
        op: FloatOp,
        /// Its operands.
        args: Vec<Term>,
    },
    /// `n.to_f64()` of an integer term, with the integer's type: `None` for `Int`.
    ToF64 {
        /// The integer.
        value: Box<Term>,
        /// Its fixed-width type, or `None` for `Int`.
        from: Option<IntType>,
    },
    /// An `F32` literal as its bits (`spec/29`, ADR 0120 V1).
    F32(u32),
    /// An operation of `F32` of the refinement language, its receiver first; `ToF64` is
    /// `x.to_f64()` of an `F32` (ADR 0120 V1).
    Float32 {
        /// The operation.
        op: FloatOp,
        /// Its operands.
        args: Vec<Term>,
    },
    /// `n.to_f32()` of an integer term, with the integer's type: `None` for `Int`.
    ToF32 {
        /// The integer.
        value: Box<Term>,
        /// Its fixed-width type, or `None` for `Int`.
        from: Option<IntType>,
    },
    /// `complex(re: a, im: b)`, a `Complex[F64]` or a `Complex[F32]` built from its components
    /// (`spec/29 §19`, ADR 0120 V3).
    Complex {
        /// The record constructor of the complex kind.
        ctor: Ctor,
        /// The real component.
        re: Box<Term>,
        /// The imaginary component.
        im: Box<Term>,
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
    /// A float literal, as the bits of its value (`spec/29`): never a NaN.
    F64(u64),
    /// A decimal literal (`spec/29 §16`): its coefficient, in decimal with an optional `-`, at
    /// its scale.
    Decimal {
        /// The coefficient c of the value c × 10^-scale.
        coefficient: String,
        /// The scale.
        scale: u8,
    },
    /// An `F32` literal, as the bits of its value, rounded once from the decimal (ADR 0120
    /// rule 6): never a NaN.
    F32(u32),
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
    /// A conversion to the fixed-width type, or to `Int` when `None` (`spec/18 §4`).
    Convert(Option<IntType>),
    /// A built-in method of a fixed-width type (`spec/18 §5`).
    Fixed(FixedOp, IntType),
    /// A built-in method of a map (`spec/21 §2`); `set` and `remove` are rebinds to `With`
    /// and `Without`.
    Map(MapOp),
    /// A built-in method of a `Stack` or a `Queue` of `std.collections` (`spec/28 §5`, ADR 0119
    /// rules 5 and 6), both held as one double-ended sequence; `push` and `pop` are rebinds of
    /// the receiver to the new sequence.
    Seq(SeqOp),
    /// A built-in method of a `Set` of `std.collections` (`spec/28 §6`, ADR 0119 rule 7);
    /// `add` and `remove` are rebinds to `With` and `Without`.
    Set(SetOp),
    /// A built-in method of a `HashMap` of `std.collections` (`spec/28 §7`, ADR 0119 rules 8 to
    /// 12); `set` and `remove` are rebinds to `With` and `Without`, and `update` is a lookup,
    /// a call and a `With`.
    HashMap(HashMapOp),
    /// A built-in method of a `HashSet` of `std.collections` (`spec/28 §7`, ADR 0119 rules 8 to
    /// 12); `add` and `remove` are rebinds to `With` and `Without`.
    HashSet(HashSetOp),
    /// A built-in method of a `PriorityQueue` of `std.collections` (`spec/28 §8`, ADR 0119 rule
    /// 13); `push`, `pop` and `pop_ranked` are rebinds of the receiver to the new queue.
    Priority(PriorityOp),
    /// A built-in method or property of a `BitSet` of `std.collections` (`spec/28 §9`, ADR 0119
    /// rule 14); `add` and `remove` are rebinds to `With` and `Without`.
    BitSet(BitSetOp),
    /// An operator, method or conversion of `F64` (`spec/29`, ADR 0114): its own kind, so no
    /// existing primitive changes meaning.
    Float(FloatOp),
    /// A shape function of a dense value other than `len` (`spec/29 §17`, ADR 0120 rule 33).
    Shape(ShapeFun),
    /// An operator, method or conversion of `Decimal[s]` (`spec/29 §16`, ADR 0120 rules 13 to
    /// 17), with the static scales of its operands.
    Decimal(DecimalOp),
    /// An operator, method or conversion of `F32` (ADR 0120 rule 5): the operations of `F64`
    /// at binary32, its own kind, so no existing primitive changes meaning.
    Float32(FloatOp),
}

/// The operators, methods and conversions of `Decimal[s]` (`spec/29 §16`, ADR 0120 rules 13
/// to 17). Each carries the scales its meaning depends on, which are static: the operands are
/// coefficients, so `x + y` at scales 2 and 4 adds `x × 100` and `y`. Every one is exact; the
/// rounding ones take a `Rounding` operand after their other operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecimalOp {
    /// `x + y`, at scales a and b: scale max(a, b).
    Add(u8, u8),
    /// `x - y`, at scales a and b: scale max(a, b).
    Sub(u8, u8),
    /// `x * y`: the coefficients' product, at the sum of the scales.
    Mul,
    /// `-x`.
    Neg,
    /// `x == y`, at scales a and b, by value.
    Eq(u8, u8),
    /// `x != y`.
    Ne(u8, u8),
    /// `x < y`.
    Lt(u8, u8),
    /// `x <= y`.
    Le(u8, u8),
    /// `x > y`.
    Gt(u8, u8),
    /// `x >= y`.
    Ge(u8, u8),
    /// `x.widen(scale: s)`, from scale a to s ≥ a.
    Widen(u8, u8),
    /// `x.rescale(scale: s, rounding: r)`, from scale a to s < a.
    Rescale(u8, u8),
    /// `x.rescale_exact(scale: s)`, from scale a to s < a; its precondition holds.
    RescaleExact(u8, u8),
    /// `x.divide(by: y, scale: s, rounding: r)`, x at scale a and y at scale b; `y != 0`.
    Divide(u8, u8, u8),
    /// `x.remainder(by: y, scale: s)`, x at scale a and y at scale b; `y != 0`.
    Remainder(u8, u8, u8),
    /// `x.to_int(rounding: r)`, x at scale a.
    ToInt(u8),
    /// `x.abs()`.
    Abs,
    /// `x.coefficient()`: the coefficient as an `Int`.
    Coefficient,
    /// `n.to_decimal()` of an integer of any integer type: `Decimal[0]`.
    FromInt,
    /// `x.to_decimal(scale: s, rounding: r)` of an `F64`: `Option[Decimal[s]]`.
    FromF64(u8),
    /// `x.to_f64()`, x at scale a.
    ToF64(u8),
    /// The rendering of rule 16, x at scale a, as `to_text` and interpolation write it.
    ToText(u8),
    /// `parse_decimal(t, scale: s)`, which fails with `ParseError`.
    Parse(u8),
    /// `as_decimal(j, scale: s)`: `Option[Decimal[s]]`.
    FromJson(u8),
    /// `json_decimal(x)`, x at scale a: a `Json`.
    ToJson(u8),
}

impl DecimalOp {
    /// How the operation is written in Core.
    pub fn as_str(self) -> &'static str {
        match self {
            DecimalOp::Add(..) => "decimal_add",
            DecimalOp::Sub(..) => "decimal_sub",
            DecimalOp::Mul => "decimal_mul",
            DecimalOp::Neg => "decimal_neg",
            DecimalOp::Eq(..) => "decimal_eq",
            DecimalOp::Ne(..) => "decimal_ne",
            DecimalOp::Lt(..) => "decimal_lt",
            DecimalOp::Le(..) => "decimal_le",
            DecimalOp::Gt(..) => "decimal_gt",
            DecimalOp::Ge(..) => "decimal_ge",
            DecimalOp::Widen(..) => "decimal_widen",
            DecimalOp::Rescale(..) => "decimal_rescale",
            DecimalOp::RescaleExact(..) => "decimal_rescale_exact",
            DecimalOp::Divide(..) => "decimal_divide",
            DecimalOp::Remainder(..) => "decimal_remainder",
            DecimalOp::ToInt(_) => "decimal_to_int",
            DecimalOp::Abs => "decimal_abs",
            DecimalOp::Coefficient => "decimal_coefficient",
            DecimalOp::FromInt => "decimal_from_int",
            DecimalOp::FromF64(_) => "decimal_from_f64",
            DecimalOp::ToF64(_) => "decimal_to_f64",
            DecimalOp::ToText(_) => "decimal_to_text",
            DecimalOp::Parse(_) => "parse_decimal",
            DecimalOp::FromJson(_) => "as_decimal",
            DecimalOp::ToJson(_) => "json_decimal",
        }
    }

    /// The operation as two machine words, its position and its scales packed a byte each, which
    /// the native code passes to the runtime; [`DecimalOp::from_code`] reads them back.
    pub fn code(self) -> (i64, i64) {
        let index = match self {
            DecimalOp::Add(..) => 0,
            DecimalOp::Sub(..) => 1,
            DecimalOp::Mul => 2,
            DecimalOp::Neg => 3,
            DecimalOp::Eq(..) => 4,
            DecimalOp::Ne(..) => 5,
            DecimalOp::Lt(..) => 6,
            DecimalOp::Le(..) => 7,
            DecimalOp::Gt(..) => 8,
            DecimalOp::Ge(..) => 9,
            DecimalOp::Widen(..) => 10,
            DecimalOp::Rescale(..) => 11,
            DecimalOp::RescaleExact(..) => 12,
            DecimalOp::Divide(..) => 13,
            DecimalOp::Remainder(..) => 14,
            DecimalOp::ToInt(_) => 15,
            DecimalOp::Abs => 16,
            DecimalOp::Coefficient => 17,
            DecimalOp::FromInt => 18,
            DecimalOp::FromF64(_) => 19,
            DecimalOp::ToF64(_) => 20,
            DecimalOp::ToText(_) => 21,
            DecimalOp::Parse(_) => 22,
            DecimalOp::FromJson(_) => 23,
            DecimalOp::ToJson(_) => 24,
        };
        let packed = self
            .scales()
            .iter()
            .enumerate()
            .fold(0i64, |packed, (i, s)| packed | (i64::from(*s) << (8 * i)));
        (index, packed)
    }

    /// The operation that [`DecimalOp::code`] gave `index` and `packed`.
    pub fn from_code(index: i64, packed: i64) -> Option<DecimalOp> {
        let scale = |i: u32| u8::try_from((packed >> (8 * i)) & 0xFF).ok();
        let (a, b, c) = (scale(0)?, scale(1)?, scale(2)?);
        Some(match index {
            0 => DecimalOp::Add(a, b),
            1 => DecimalOp::Sub(a, b),
            2 => DecimalOp::Mul,
            3 => DecimalOp::Neg,
            4 => DecimalOp::Eq(a, b),
            5 => DecimalOp::Ne(a, b),
            6 => DecimalOp::Lt(a, b),
            7 => DecimalOp::Le(a, b),
            8 => DecimalOp::Gt(a, b),
            9 => DecimalOp::Ge(a, b),
            10 => DecimalOp::Widen(a, b),
            11 => DecimalOp::Rescale(a, b),
            12 => DecimalOp::RescaleExact(a, b),
            13 => DecimalOp::Divide(a, b, c),
            14 => DecimalOp::Remainder(a, b, c),
            15 => DecimalOp::ToInt(a),
            16 => DecimalOp::Abs,
            17 => DecimalOp::Coefficient,
            18 => DecimalOp::FromInt,
            19 => DecimalOp::FromF64(a),
            20 => DecimalOp::ToF64(a),
            21 => DecimalOp::ToText(a),
            22 => DecimalOp::Parse(a),
            23 => DecimalOp::FromJson(a),
            24 => DecimalOp::ToJson(a),
            _ => return None,
        })
    }

    /// The static scales of the operation, in the order its variant holds them.
    pub fn scales(self) -> Vec<u8> {
        match self {
            DecimalOp::Add(a, b)
            | DecimalOp::Sub(a, b)
            | DecimalOp::Eq(a, b)
            | DecimalOp::Ne(a, b)
            | DecimalOp::Lt(a, b)
            | DecimalOp::Le(a, b)
            | DecimalOp::Gt(a, b)
            | DecimalOp::Ge(a, b)
            | DecimalOp::Widen(a, b)
            | DecimalOp::Rescale(a, b)
            | DecimalOp::RescaleExact(a, b) => vec![a, b],
            DecimalOp::Divide(a, b, s) | DecimalOp::Remainder(a, b, s) => vec![a, b, s],
            DecimalOp::ToInt(a)
            | DecimalOp::FromF64(a)
            | DecimalOp::ToF64(a)
            | DecimalOp::ToText(a)
            | DecimalOp::Parse(a)
            | DecimalOp::FromJson(a)
            | DecimalOp::ToJson(a) => vec![a],
            DecimalOp::Mul
            | DecimalOp::Neg
            | DecimalOp::Abs
            | DecimalOp::Coefficient
            | DecimalOp::FromInt => Vec::new(),
        }
    }
}

/// The shape functions of a dense value beside `len`, which `Array` has as a list does, and
/// beside `dim`, which takes an axis (`spec/29 §17`, ADR 0120 rules 33 and 38). Each is an `Int`
/// of at least 0, and a rank is from 1 to 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShapeFun {
    /// `m.rows`.
    Rows,
    /// `m.columns`.
    Columns,
    /// `t.rank` of a tensor: from 1 to 8.
    Rank,
    /// The number of elements of a tensor, the product of its lengths. Only the prelude's
    /// contracts write it: it states the precondition of `reshape` linearly.
    Count,
    /// `f.size` of an `Lu[T]`.
    Size,
}

impl ShapeFun {
    /// The name of the function, as source writes it after the point.
    pub fn as_str(self) -> &'static str {
        match self {
            ShapeFun::Rows => "rows",
            ShapeFun::Columns => "columns",
            ShapeFun::Rank => "rank",
            ShapeFun::Count => "count",
            ShapeFun::Size => "size",
        }
    }
}

/// The operators, methods and conversions of `F64` (`spec/29`, ADR 0114 rules 3 to 5). Every
/// one is total and O(1), and every `F64` it returns is canonical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FloatOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// Unary `-`.
    Neg,
    /// `==`, identity of the canonical bits.
    Eq,
    /// `!=`.
    Ne,
    /// `<`, IEEE totalOrder.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `abs()`.
    Abs,
    /// `sqrt()`.
    Sqrt,
    /// `fma(times:, plus:)`.
    Fma,
    /// `rem(by:)`.
    Rem,
    /// `floor()`.
    Floor,
    /// `ceil()`.
    Ceil,
    /// `trunc()`.
    Trunc,
    /// `round_half_even()`.
    RoundHalfEven,
    /// `round_half_away()`.
    RoundHalfAway,
    /// `copy_sign(from:)`.
    CopySign,
    /// `ieee_min(other:)`.
    IeeeMin,
    /// `ieee_max(other:)`.
    IeeeMax,
    /// `clamp(lo:, hi:)`.
    Clamp,
    /// `ieee_eq(other:)`.
    IeeeEq,
    /// `ieee_ne(other:)`.
    IeeeNe,
    /// `ieee_lt(other:)`.
    IeeeLt,
    /// `ieee_le(other:)`.
    IeeeLe,
    /// `ieee_gt(other:)`.
    IeeeGt,
    /// `ieee_ge(other:)`.
    IeeeGe,
    /// `is_nan()`.
    IsNan,
    /// `is_finite()`.
    IsFinite,
    /// `is_infinite()`.
    IsInfinite,
    /// `is_sign_negative()`.
    IsSignNegative,
    /// `is_zero()`.
    IsZero,
    /// `is_positive_zero()`.
    IsPositiveZero,
    /// `is_negative_zero()`.
    IsNegativeZero,
    /// `to_bits()`, a `U64`.
    ToBits,
    /// `to_int_exact()`, an `Option[Int]`.
    ToIntExact,
    /// `to_f64()` of an integer of any integer type.
    FromInt,
    /// The rendering of `spec/29`, as `to_text` and interpolation write it.
    ToText,
    /// `x.to_f32()` of an `F64`: IEEE `convertFormat`, ties to even (ADR 0120 rule 8). Only
    /// under `Prim::Float`.
    ToF32,
    /// `x.to_f64()` of an `F32`, exact (ADR 0120 rule 8). Only under `Prim::Float32`.
    ToF64,
}

/// What a method of `F64` or `F32` returns (ADR 0114 rule 3.3, ADR 0120 rule 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatResult {
    /// The receiver's type, `F64` or `F32`.
    F64,
    /// The other float type: `to_f32()` of an `F64`, `to_f64()` of an `F32`.
    Other,
    /// `Bool`.
    Bool,
    /// `U64`.
    U64,
    /// `Option[Int]`.
    OptionInt,
}

impl FloatOp {
    /// Every operation, in declaration order: the code the native backend passes to the
    /// runtime is the position here.
    pub const ALL: [FloatOp; 43] = [
        FloatOp::Add,
        FloatOp::Sub,
        FloatOp::Mul,
        FloatOp::Div,
        FloatOp::Neg,
        FloatOp::Eq,
        FloatOp::Ne,
        FloatOp::Lt,
        FloatOp::Le,
        FloatOp::Gt,
        FloatOp::Ge,
        FloatOp::Abs,
        FloatOp::Sqrt,
        FloatOp::Fma,
        FloatOp::Rem,
        FloatOp::Floor,
        FloatOp::Ceil,
        FloatOp::Trunc,
        FloatOp::RoundHalfEven,
        FloatOp::RoundHalfAway,
        FloatOp::CopySign,
        FloatOp::IeeeMin,
        FloatOp::IeeeMax,
        FloatOp::Clamp,
        FloatOp::IeeeEq,
        FloatOp::IeeeNe,
        FloatOp::IeeeLt,
        FloatOp::IeeeLe,
        FloatOp::IeeeGt,
        FloatOp::IeeeGe,
        FloatOp::IsNan,
        FloatOp::IsFinite,
        FloatOp::IsInfinite,
        FloatOp::IsSignNegative,
        FloatOp::IsZero,
        FloatOp::IsPositiveZero,
        FloatOp::IsNegativeZero,
        FloatOp::ToBits,
        FloatOp::ToIntExact,
        FloatOp::FromInt,
        FloatOp::ToText,
        FloatOp::ToF32,
        FloatOp::ToF64,
    ];

    /// The methods of `F64`, in the order of ADR 0114's table.
    pub const METHODS: [FloatOp; 28] = [
        FloatOp::Abs,
        FloatOp::Sqrt,
        FloatOp::Fma,
        FloatOp::Rem,
        FloatOp::Floor,
        FloatOp::Ceil,
        FloatOp::Trunc,
        FloatOp::RoundHalfEven,
        FloatOp::RoundHalfAway,
        FloatOp::CopySign,
        FloatOp::IeeeMin,
        FloatOp::IeeeMax,
        FloatOp::Clamp,
        FloatOp::IeeeEq,
        FloatOp::IeeeNe,
        FloatOp::IeeeLt,
        FloatOp::IeeeLe,
        FloatOp::IeeeGt,
        FloatOp::IeeeGe,
        FloatOp::IsNan,
        FloatOp::IsFinite,
        FloatOp::IsInfinite,
        FloatOp::IsSignNegative,
        FloatOp::IsZero,
        FloatOp::IsPositiveZero,
        FloatOp::IsNegativeZero,
        FloatOp::ToBits,
        FloatOp::ToIntExact,
    ];

    /// The kind of the operation in Core and its export: `f64_` and the operation
    /// (`spec/13 §8`).
    pub fn as_str(self) -> &'static str {
        match self {
            FloatOp::Add => "f64_add",
            FloatOp::Sub => "f64_sub",
            FloatOp::Mul => "f64_mul",
            FloatOp::Div => "f64_div",
            FloatOp::Neg => "f64_neg",
            FloatOp::Eq => "f64_eq",
            FloatOp::Ne => "f64_ne",
            FloatOp::Lt => "f64_lt",
            FloatOp::Le => "f64_le",
            FloatOp::Gt => "f64_gt",
            FloatOp::Ge => "f64_ge",
            FloatOp::Abs => "f64_abs",
            FloatOp::Sqrt => "f64_sqrt",
            FloatOp::Fma => "f64_fma",
            FloatOp::Rem => "f64_rem",
            FloatOp::Floor => "f64_floor",
            FloatOp::Ceil => "f64_ceil",
            FloatOp::Trunc => "f64_trunc",
            FloatOp::RoundHalfEven => "f64_round_half_even",
            FloatOp::RoundHalfAway => "f64_round_half_away",
            FloatOp::CopySign => "f64_copy_sign",
            FloatOp::IeeeMin => "f64_ieee_min",
            FloatOp::IeeeMax => "f64_ieee_max",
            FloatOp::Clamp => "f64_clamp",
            FloatOp::IeeeEq => "f64_ieee_eq",
            FloatOp::IeeeNe => "f64_ieee_ne",
            FloatOp::IeeeLt => "f64_ieee_lt",
            FloatOp::IeeeLe => "f64_ieee_le",
            FloatOp::IeeeGt => "f64_ieee_gt",
            FloatOp::IeeeGe => "f64_ieee_ge",
            FloatOp::IsNan => "f64_is_nan",
            FloatOp::IsFinite => "f64_is_finite",
            FloatOp::IsInfinite => "f64_is_infinite",
            FloatOp::IsSignNegative => "f64_is_sign_negative",
            FloatOp::IsZero => "f64_is_zero",
            FloatOp::IsPositiveZero => "f64_is_positive_zero",
            FloatOp::IsNegativeZero => "f64_is_negative_zero",
            FloatOp::ToBits => "f64_to_bits",
            FloatOp::ToIntExact => "f64_to_int_exact",
            FloatOp::FromInt => "f64_from_int",
            FloatOp::ToText => "f64_to_text",
            FloatOp::ToF32 => "f64_to_f32",
            FloatOp::ToF64 => "f64_to_f64",
        }
    }

    /// The kind of the operation of `F32` in Core and its export: `f32_` and the operation.
    pub fn as_str_32(self) -> &'static str {
        match self {
            FloatOp::Add => "f32_add",
            FloatOp::Sub => "f32_sub",
            FloatOp::Mul => "f32_mul",
            FloatOp::Div => "f32_div",
            FloatOp::Neg => "f32_neg",
            FloatOp::Eq => "f32_eq",
            FloatOp::Ne => "f32_ne",
            FloatOp::Lt => "f32_lt",
            FloatOp::Le => "f32_le",
            FloatOp::Gt => "f32_gt",
            FloatOp::Ge => "f32_ge",
            FloatOp::Abs => "f32_abs",
            FloatOp::Sqrt => "f32_sqrt",
            FloatOp::Fma => "f32_fma",
            FloatOp::Rem => "f32_rem",
            FloatOp::Floor => "f32_floor",
            FloatOp::Ceil => "f32_ceil",
            FloatOp::Trunc => "f32_trunc",
            FloatOp::RoundHalfEven => "f32_round_half_even",
            FloatOp::RoundHalfAway => "f32_round_half_away",
            FloatOp::CopySign => "f32_copy_sign",
            FloatOp::IeeeMin => "f32_ieee_min",
            FloatOp::IeeeMax => "f32_ieee_max",
            FloatOp::Clamp => "f32_clamp",
            FloatOp::IeeeEq => "f32_ieee_eq",
            FloatOp::IeeeNe => "f32_ieee_ne",
            FloatOp::IeeeLt => "f32_ieee_lt",
            FloatOp::IeeeLe => "f32_ieee_le",
            FloatOp::IeeeGt => "f32_ieee_gt",
            FloatOp::IeeeGe => "f32_ieee_ge",
            FloatOp::IsNan => "f32_is_nan",
            FloatOp::IsFinite => "f32_is_finite",
            FloatOp::IsInfinite => "f32_is_infinite",
            FloatOp::IsSignNegative => "f32_is_sign_negative",
            FloatOp::IsZero => "f32_is_zero",
            FloatOp::IsPositiveZero => "f32_is_positive_zero",
            FloatOp::IsNegativeZero => "f32_is_negative_zero",
            FloatOp::ToBits => "f32_to_bits",
            FloatOp::ToIntExact => "f32_to_int_exact",
            FloatOp::FromInt => "f32_from_int",
            FloatOp::ToText => "f32_to_text",
            FloatOp::ToF32 => "f32_to_f32",
            FloatOp::ToF64 => "f32_to_f64",
        }
    }

    /// The method's name in the source, for a method of `F64`; `None` for an operator.
    pub fn method(self) -> Option<&'static str> {
        Some(match self {
            FloatOp::Abs => "abs",
            FloatOp::Sqrt => "sqrt",
            FloatOp::Fma => "fma",
            FloatOp::Rem => "rem",
            FloatOp::Floor => "floor",
            FloatOp::Ceil => "ceil",
            FloatOp::Trunc => "trunc",
            FloatOp::RoundHalfEven => "round_half_even",
            FloatOp::RoundHalfAway => "round_half_away",
            FloatOp::CopySign => "copy_sign",
            FloatOp::IeeeMin => "ieee_min",
            FloatOp::IeeeMax => "ieee_max",
            FloatOp::Clamp => "clamp",
            FloatOp::IeeeEq => "ieee_eq",
            FloatOp::IeeeNe => "ieee_ne",
            FloatOp::IeeeLt => "ieee_lt",
            FloatOp::IeeeLe => "ieee_le",
            FloatOp::IeeeGt => "ieee_gt",
            FloatOp::IeeeGe => "ieee_ge",
            FloatOp::IsNan => "is_nan",
            FloatOp::IsFinite => "is_finite",
            FloatOp::IsInfinite => "is_infinite",
            FloatOp::IsSignNegative => "is_sign_negative",
            FloatOp::IsZero => "is_zero",
            FloatOp::IsPositiveZero => "is_positive_zero",
            FloatOp::IsNegativeZero => "is_negative_zero",
            FloatOp::ToBits => "to_bits",
            FloatOp::ToIntExact => "to_int_exact",
            FloatOp::ToF32 => "to_f32",
            FloatOp::ToF64 => "to_f64",
            _ => return None,
        })
    }

    /// The method of `F64` named `name`.
    pub fn from_method(name: &str) -> Option<FloatOp> {
        FloatOp::METHODS
            .into_iter()
            .find(|op| op.method() == Some(name))
    }

    /// The labels of a method's arguments after the receiver, in order: each is an `F64`.
    pub fn params(self) -> &'static [&'static str] {
        match self {
            FloatOp::Fma => &["times", "plus"],
            FloatOp::Rem => &["by"],
            FloatOp::CopySign => &["from"],
            FloatOp::Clamp => &["lo", "hi"],
            FloatOp::IeeeMin
            | FloatOp::IeeeMax
            | FloatOp::IeeeEq
            | FloatOp::IeeeNe
            | FloatOp::IeeeLt
            | FloatOp::IeeeLe
            | FloatOp::IeeeGt
            | FloatOp::IeeeGe => &["other"],
            _ => &[],
        }
    }

    /// Whether the operation is in the refinement language (ADR 0114 rule 8.4): the
    /// operators, the comparisons, `abs()`, `sqrt()`, the IEEE comparisons, the
    /// classifications and `to_f64()`; the other methods are code only.
    pub fn in_refinement_language(self) -> bool {
        !matches!(
            self,
            FloatOp::Fma
                | FloatOp::Rem
                | FloatOp::Floor
                | FloatOp::Ceil
                | FloatOp::Trunc
                | FloatOp::RoundHalfEven
                | FloatOp::RoundHalfAway
                | FloatOp::CopySign
                | FloatOp::IeeeMin
                | FloatOp::IeeeMax
                | FloatOp::Clamp
                | FloatOp::ToBits
                | FloatOp::ToIntExact
                | FloatOp::ToText
        )
    }

    /// What the operation returns.
    pub fn result(self) -> FloatResult {
        match self {
            FloatOp::Eq
            | FloatOp::Ne
            | FloatOp::Lt
            | FloatOp::Le
            | FloatOp::Gt
            | FloatOp::Ge
            | FloatOp::IeeeEq
            | FloatOp::IeeeNe
            | FloatOp::IeeeLt
            | FloatOp::IeeeLe
            | FloatOp::IeeeGt
            | FloatOp::IeeeGe
            | FloatOp::IsNan
            | FloatOp::IsFinite
            | FloatOp::IsInfinite
            | FloatOp::IsSignNegative
            | FloatOp::IsZero
            | FloatOp::IsPositiveZero
            | FloatOp::IsNegativeZero => FloatResult::Bool,
            FloatOp::ToBits => FloatResult::U64,
            FloatOp::ToIntExact => FloatResult::OptionInt,
            FloatOp::ToF32 | FloatOp::ToF64 => FloatResult::Other,
            _ => FloatResult::F64,
        }
    }
}

/// The built-in methods of a map, in their order of arguments after the map (`spec/21 §2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MapOp {
    /// `has(key:)`.
    Has,
    /// `get(key:)`.
    Get,
    /// `with(key:, value:)`, and `edit set`.
    With,
    /// `without(key:)`, and `edit remove`.
    Without,
    /// `keys()`.
    Keys,
    /// `values()`.
    Values,
    /// `entries()`.
    Entries,
}

impl MapOp {
    /// The method's name in the source.
    pub fn as_str(self) -> &'static str {
        match self {
            MapOp::Has => "has",
            MapOp::Get => "get",
            MapOp::With => "with",
            MapOp::Without => "without",
            MapOp::Keys => "keys",
            MapOp::Values => "values",
            MapOp::Entries => "entries",
        }
    }
}

/// The operations of the double-ended sequence that holds a `Stack[T]` or a `Queue[T]`
/// (`spec/28 §5`, ADR 0119 rules 5, 6 and 19), each over the sequence in its first argument. A
/// stack keeps its top at the front, so both types list front first. Every operation is O(1),
/// amortized for the updates, except `Items`, O(n).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeqOp {
    /// The sequence with `item` added at the front: a stack's `push`, a queue's `push_front`.
    PushFront,
    /// The sequence with `item` added at the back: a queue's `push`.
    PushBack,
    /// The sequence without its front, the same sequence when it is empty: the update of a
    /// stack's or a queue's `pop`.
    PopFront,
    /// The sequence without its back, the same sequence when it is empty: the update of a
    /// queue's `pop_back`.
    PopBack,
    /// `Some` of the front, `None` when empty: `peek`, and the result of `pop`.
    PeekFront,
    /// `Some` of the back, `None` when empty: a queue's `peek_back`, and the result of
    /// `pop_back`.
    PeekBack,
    /// The elements, front first, as a `List`: `items()`.
    Items,
}

impl SeqOp {
    /// Every operation, in a fixed order: the native backend passes an operation to the
    /// runtime as its position here (ADR 0119 rule 19).
    pub const ALL: [SeqOp; 7] = [
        SeqOp::PushFront,
        SeqOp::PushBack,
        SeqOp::PopFront,
        SeqOp::PopBack,
        SeqOp::PeekFront,
        SeqOp::PeekBack,
        SeqOp::Items,
    ];

    /// The operation's name in Core.
    pub fn as_str(self) -> &'static str {
        match self {
            SeqOp::PushFront => "push_front",
            SeqOp::PushBack => "push_back",
            SeqOp::PopFront => "pop_front",
            SeqOp::PopBack => "pop_back",
            SeqOp::PeekFront => "peek_front",
            SeqOp::PeekBack => "peek_back",
            SeqOp::Items => "items",
        }
    }

    /// Whether the operation gives a new sequence built from its receiver, which it may update
    /// in place when the receiver is given up (`spec/28 §5`, ADR 0119 rule 1).
    pub fn rebuilds(self) -> bool {
        matches!(
            self,
            SeqOp::PushFront | SeqOp::PushBack | SeqOp::PopFront | SeqOp::PopBack
        )
    }

    /// The read that gives a removal's result, read before the removal: `PeekFront` for
    /// `PopFront` and `PeekBack` for `PopBack`; `None` for any other operation.
    pub fn removed(self) -> Option<SeqOp> {
        match self {
            SeqOp::PopFront => Some(SeqOp::PeekFront),
            SeqOp::PopBack => Some(SeqOp::PeekBack),
            _ => None,
        }
    }
}

/// The built-in methods of `Set[T]` (`spec/28 §6`, ADR 0119 rule 7), each over the set in its
/// first argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SetOp {
    /// `has(item:)`.
    Has,
    /// `with(item:)`, and `edit add`.
    With,
    /// `without(item:)`, and `edit remove`.
    Without,
    /// `first()`, the least item.
    First,
    /// `last()`, the greatest item.
    Last,
    /// `between(low:, high:)`, the items from `low` to `high`, both included.
    Between,
    /// `items()`, ascending.
    Items,
    /// `union(other:)`.
    Union,
    /// `intersection(other:)`.
    Intersection,
    /// `difference(other:)`.
    Difference,
    /// `is_subset(of:)`.
    IsSubset,
}

impl SetOp {
    /// Every operation, in a fixed order: the native backend passes an operation to the
    /// runtime as its position here (ADR 0119 rule 19).
    pub const ALL: [SetOp; 11] = [
        SetOp::Has,
        SetOp::With,
        SetOp::Without,
        SetOp::First,
        SetOp::Last,
        SetOp::Between,
        SetOp::Items,
        SetOp::Union,
        SetOp::Intersection,
        SetOp::Difference,
        SetOp::IsSubset,
    ];

    /// The method's name in the source.
    pub fn as_str(self) -> &'static str {
        match self {
            SetOp::Has => "has",
            SetOp::With => "with",
            SetOp::Without => "without",
            SetOp::First => "first",
            SetOp::Last => "last",
            SetOp::Between => "between",
            SetOp::Items => "items",
            SetOp::Union => "union",
            SetOp::Intersection => "intersection",
            SetOp::Difference => "difference",
            SetOp::IsSubset => "is_subset",
        }
    }

    /// Whether the operation gives a new set built from its receiver, which it may update in
    /// place when the receiver is given up (`spec/28 §6`, ADR 0119 rule 1).
    pub fn rebuilds(self) -> bool {
        matches!(
            self,
            SetOp::With | SetOp::Without | SetOp::Union | SetOp::Intersection | SetOp::Difference
        )
    }
}

/// The built-in methods of `HashMap[K, V]` (`spec/28 §7`, ADR 0119 rule 12), each over the
/// map in its first argument. Lookups and updates are O(|k|) expected, the listings O(n).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HashMapOp {
    /// `has(key:)`.
    Has,
    /// `get(key:)`.
    Get,
    /// `with(key:, value:)`, and `edit set`.
    With,
    /// `without(key:)`, and `edit remove`.
    Without,
    /// `keys()`, in insertion order.
    Keys,
    /// `values()`, in insertion order.
    Values,
    /// `entries()`, in insertion order.
    Entries,
    /// `same_entries(as:)`, whatever the order.
    SameEntries,
}

impl HashMapOp {
    /// Every operation, in a fixed order: the native backend passes an operation to the
    /// runtime as its position here (ADR 0119 rule 19).
    pub const ALL: [HashMapOp; 8] = [
        HashMapOp::Has,
        HashMapOp::Get,
        HashMapOp::With,
        HashMapOp::Without,
        HashMapOp::Keys,
        HashMapOp::Values,
        HashMapOp::Entries,
        HashMapOp::SameEntries,
    ];

    /// The method's name in the source.
    pub fn as_str(self) -> &'static str {
        match self {
            HashMapOp::Has => "has",
            HashMapOp::Get => "get",
            HashMapOp::With => "with",
            HashMapOp::Without => "without",
            HashMapOp::Keys => "keys",
            HashMapOp::Values => "values",
            HashMapOp::Entries => "entries",
            HashMapOp::SameEntries => "same_entries",
        }
    }

    /// Whether the operation gives a new map built from its receiver, which it may update in
    /// place when the receiver is given up (ADR 0119 rule 1).
    pub fn rebuilds(self) -> bool {
        matches!(self, HashMapOp::With | HashMapOp::Without)
    }
}

/// The built-in methods of `HashSet[T]` (`spec/28 §7`, ADR 0119 rule 12), each over the set in
/// its first argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HashSetOp {
    /// `has(item:)`.
    Has,
    /// `with(item:)`, and `edit add`.
    With,
    /// `without(item:)`, and `edit remove`.
    Without,
    /// `items()`, in insertion order.
    Items,
    /// `union(other:)`.
    Union,
    /// `intersection(other:)`.
    Intersection,
    /// `difference(other:)`.
    Difference,
    /// `is_subset(of:)`.
    IsSubset,
    /// `same_items(as:)`, whatever the order.
    SameItems,
}

impl HashSetOp {
    /// Every operation, in a fixed order: the native backend passes an operation to the
    /// runtime as its position here (ADR 0119 rule 19).
    pub const ALL: [HashSetOp; 9] = [
        HashSetOp::Has,
        HashSetOp::With,
        HashSetOp::Without,
        HashSetOp::Items,
        HashSetOp::Union,
        HashSetOp::Intersection,
        HashSetOp::Difference,
        HashSetOp::IsSubset,
        HashSetOp::SameItems,
    ];

    /// The method's name in the source.
    pub fn as_str(self) -> &'static str {
        match self {
            HashSetOp::Has => "has",
            HashSetOp::With => "with",
            HashSetOp::Without => "without",
            HashSetOp::Items => "items",
            HashSetOp::Union => "union",
            HashSetOp::Intersection => "intersection",
            HashSetOp::Difference => "difference",
            HashSetOp::IsSubset => "is_subset",
            HashSetOp::SameItems => "same_items",
        }
    }

    /// Whether the operation gives a new set built from its receiver, which it may update in
    /// place when the receiver is given up (ADR 0119 rule 1).
    pub fn rebuilds(self) -> bool {
        matches!(
            self,
            HashSetOp::With
                | HashSetOp::Without
                | HashSetOp::Union
                | HashSetOp::Intersection
                | HashSetOp::Difference
        )
    }
}

/// The operations of the stable heap that holds a `PriorityQueue[T, P]` (`spec/28 §8`, ADR 0119
/// rules 13 and 19), each over the queue in its first argument. The first item is the one of the
/// least priority for a queue made by `lowest_first`, of the greatest for one made by
/// `highest_first`, and among equal priorities the one pushed first. `Push` and `Pop` are
/// O(log n) comparisons, amortized, `Items` O(n log n), and the reads O(1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriorityOp {
    /// The queue with `item` added at `priority`: `push(item:, priority:)`.
    Push,
    /// The queue without its first item, the same queue when it is empty: the update of `pop`
    /// and of `pop_ranked`.
    Pop,
    /// `Some` of the first item, `None` when empty: `peek`, and the result of `pop`.
    Peek,
    /// `Some` of the first item's priority, `None` when empty: `peek_priority`.
    PeekPriority,
    /// `Some` of the first item with its priority, a `Ranked`, `None` when empty: the result of
    /// `pop_ranked`.
    PeekRanked,
    /// The items in the order of the pops, as a `List`: `items()`.
    Items,
}

impl PriorityOp {
    /// Every operation, in a fixed order: the native backend passes an operation to the
    /// runtime as its position here (ADR 0119 rule 19).
    pub const ALL: [PriorityOp; 6] = [
        PriorityOp::Push,
        PriorityOp::Pop,
        PriorityOp::Peek,
        PriorityOp::PeekPriority,
        PriorityOp::PeekRanked,
        PriorityOp::Items,
    ];

    /// The operation's name in Core.
    pub fn as_str(self) -> &'static str {
        match self {
            PriorityOp::Push => "push",
            PriorityOp::Pop => "pop",
            PriorityOp::Peek => "peek",
            PriorityOp::PeekPriority => "peek_priority",
            PriorityOp::PeekRanked => "peek_ranked",
            PriorityOp::Items => "items",
        }
    }

    /// Whether the operation gives a new queue built from its receiver, which it may update in
    /// place when the receiver is given up (`spec/28 §8`, ADR 0119 rule 1).
    pub fn rebuilds(self) -> bool {
        matches!(self, PriorityOp::Push | PriorityOp::Pop)
    }
}

/// The built-in methods of `BitSet` (`spec/28 §9`, ADR 0119 rule 14), each over the bit set in
/// its first argument, with its property `size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BitSetOp {
    /// `.size`, the number of bits, fixed when the bit set is made.
    Size,
    /// `has(item:)`, `false` outside the range.
    Has,
    /// `with(item:)`, and `edit add`: the item MUST be in the range.
    With,
    /// `without(item:)`, and `edit remove`: an item outside the range changes nothing.
    Without,
    /// `union(other:)`, of two bit sets of one size.
    Union,
    /// `intersection(other:)`, of two bit sets of one size.
    Intersection,
    /// `difference(other:)`, of two bit sets of one size.
    Difference,
    /// `is_subset(of:)`, of two bit sets of one size.
    IsSubset,
    /// `first()`, the least member.
    First,
    /// `last()`, the greatest member.
    Last,
    /// `items()`, ascending.
    Items,
}

impl BitSetOp {
    /// Every operation, in a fixed order: the native backend passes an operation to the
    /// runtime as its position here (ADR 0119 rule 19).
    pub const ALL: [BitSetOp; 11] = [
        BitSetOp::Size,
        BitSetOp::Has,
        BitSetOp::With,
        BitSetOp::Without,
        BitSetOp::Union,
        BitSetOp::Intersection,
        BitSetOp::Difference,
        BitSetOp::IsSubset,
        BitSetOp::First,
        BitSetOp::Last,
        BitSetOp::Items,
    ];

    /// The method's or the property's name in the source.
    pub fn as_str(self) -> &'static str {
        match self {
            BitSetOp::Size => "size",
            BitSetOp::Has => "has",
            BitSetOp::With => "with",
            BitSetOp::Without => "without",
            BitSetOp::Union => "union",
            BitSetOp::Intersection => "intersection",
            BitSetOp::Difference => "difference",
            BitSetOp::IsSubset => "is_subset",
            BitSetOp::First => "first",
            BitSetOp::Last => "last",
            BitSetOp::Items => "items",
        }
    }

    /// Whether the operation gives a new bit set built from its receiver, which it may update in
    /// place when the receiver is given up (`spec/28 §9`, ADR 0119 rule 1).
    pub fn rebuilds(self) -> bool {
        matches!(
            self,
            BitSetOp::With
                | BitSetOp::Without
                | BitSetOp::Union
                | BitSetOp::Intersection
                | BitSetOp::Difference
        )
    }

    /// Whether its other operand is a bit set that MUST have the receiver's size (ADR 0119
    /// rule 14).
    pub fn pairs(self) -> bool {
        matches!(
            self,
            BitSetOp::Union | BitSetOp::Intersection | BitSetOp::Difference | BitSetOp::IsSubset
        )
    }
}

/// The wrapping, saturating and bit methods of the fixed-width types (`spec/18 §5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FixedOp {
    /// `wrapping_add`.
    WrappingAdd,
    /// `wrapping_sub`.
    WrappingSub,
    /// `wrapping_mul`.
    WrappingMul,
    /// `saturating_add`.
    SaturatingAdd,
    /// `saturating_sub`.
    SaturatingSub,
    /// `saturating_mul`.
    SaturatingMul,
    /// `bit_and`.
    BitAnd,
    /// `bit_or`.
    BitOr,
    /// `bit_xor`.
    BitXor,
    /// `bit_not`.
    BitNot,
    /// `shift_right`.
    ShiftRight,
    /// `wrapping_shift_left`.
    WrappingShiftLeft,
}

impl FixedOp {
    /// Every method, in the order of `spec/18 §5`.
    pub const ALL: [FixedOp; 12] = [
        FixedOp::WrappingAdd,
        FixedOp::WrappingSub,
        FixedOp::WrappingMul,
        FixedOp::SaturatingAdd,
        FixedOp::SaturatingSub,
        FixedOp::SaturatingMul,
        FixedOp::BitAnd,
        FixedOp::BitOr,
        FixedOp::BitXor,
        FixedOp::BitNot,
        FixedOp::ShiftRight,
        FixedOp::WrappingShiftLeft,
    ];

    /// The method's name, as written.
    pub fn as_str(self) -> &'static str {
        match self {
            FixedOp::WrappingAdd => "wrapping_add",
            FixedOp::WrappingSub => "wrapping_sub",
            FixedOp::WrappingMul => "wrapping_mul",
            FixedOp::SaturatingAdd => "saturating_add",
            FixedOp::SaturatingSub => "saturating_sub",
            FixedOp::SaturatingMul => "saturating_mul",
            FixedOp::BitAnd => "bit_and",
            FixedOp::BitOr => "bit_or",
            FixedOp::BitXor => "bit_xor",
            FixedOp::BitNot => "bit_not",
            FixedOp::ShiftRight => "shift_right",
            FixedOp::WrappingShiftLeft => "wrapping_shift_left",
        }
    }

    /// The method named `name`, if it is one.
    pub fn from_name(name: &str) -> Option<FixedOp> {
        FixedOp::ALL.into_iter().find(|op| op.as_str() == name)
    }

    /// Whether the method takes an argument besides its receiver: every one but `bit_not`.
    pub fn binary(self) -> bool {
        self != FixedOp::BitNot
    }

    /// The label of the argument (`spec/18 §5`, charter P5): `mask` for the bit operations,
    /// `by` for the others.
    pub fn label(self) -> &'static str {
        match self {
            FixedOp::BitAnd | FixedOp::BitOr | FixedOp::BitXor => "mask",
            _ => "by",
        }
    }

    /// Whether the argument is a shift amount, an `Int` with `0 <= n < width`.
    pub fn shift(self) -> bool {
        matches!(self, FixedOp::ShiftRight | FixedOp::WrappingShiftLeft)
    }
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
            Prim::Convert(None) => "to_int",
            Prim::Convert(Some(int)) => match int {
                IntType::I8 => "to_i8",
                IntType::I16 => "to_i16",
                IntType::I32 => "to_i32",
                IntType::I64 => "to_i64",
                IntType::U8 => "to_u8",
                IntType::U16 => "to_u16",
                IntType::U32 => "to_u32",
                IntType::U64 => "to_u64",
            },
            Prim::Fixed(op, _) => op.as_str(),
            Prim::Map(op) => op.as_str(),
            Prim::Seq(op) => op.as_str(),
            Prim::Set(op) => op.as_str(),
            Prim::HashMap(op) => op.as_str(),
            Prim::HashSet(op) => op.as_str(),
            Prim::Priority(op) => op.as_str(),
            Prim::BitSet(op) => op.as_str(),
            Prim::Float(op) => op.as_str(),
            Prim::Shape(fun) => fun.as_str(),
            Prim::Decimal(op) => op.as_str(),
            Prim::Float32(op) => op.as_str_32(),
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
    /// `(a, ...)`: a tuple (`spec/17 §4`).
    Tuple(Vec<Atom>),
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
        /// The capability's declaration, by its qualified identity (`spec/10 §11` rule 8).
        capability_path: Path,
        /// The operation.
        op: String,
        /// The arguments, in parameter order.
        args: Vec<Atom>,
        /// The source of each argument.
        arg_spans: Vec<Span>,
        /// The static labels of the arguments.
        labels: Vec<Label>,
        /// The label its outcome is rendered at in a run's trace (S-45, #1552, `spec/11 §5`):
        /// the label of the decision whether it fails and of its payload, joined with the
        /// labels of its failure type and of its result. Above `public`, the event's outcome
        /// is `redacted`, its kind, variant and value hidden.
        outcome: Label,
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
        /// The declared function this lambda stands for, when a function is used as a value:
        /// its parameters keep their declared names and types, and its body calls the
        /// function (ADR 0037, `spec/12 §3`). `None` for a lambda of the source.
        function: Option<Path>,
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
/// statement or a branching construct is.
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
        /// The handler call in the source, where what installing it requires is charged
        /// (ADR 0092 rule 3).
        span: Span,
    },
}

/// What a loop visits (`spec/17 §4`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopSource {
    /// The elements of a list, in order.
    List(Atom),
    /// The integers from `low` to `high`, in increasing order.
    Range {
        /// The lower bound.
        low: Atom,
        /// The upper bound.
        high: Atom,
        /// Whether `high` is visited (`..`), or excluded (`..<`).
        inclusive: bool,
    },
    /// `until(c)`: no element; before each iteration, `cond` is evaluated with the current
    /// state bound, and the loop ends when it is `true` (`spec/26 §4`, ADR 0116 rule 20). Only
    /// in a loop, never in `together_for`.
    Until {
        /// The condition, an expression over the loop's state and the bindings before it.
        cond: Box<Expr>,
        /// The condition as a formula, when it is in the refinement language: false in the
        /// body, and true after a loop with no `stop` (ADR 0116 V1, V3).
        holds: Option<Box<Clause>>,
        /// The measure, over the loop's state, which is never evaluated (ADR 0116 V1).
        measure: Box<Clause>,
        /// The word `decreases`, where the measure's obligations are reported.
        decreases: Span,
    },
    /// `ever`: no element and no end but `stop`, a host stop at the start of an iteration, a
    /// failure of the body or a cancellation (`spec/26 §5`). Only in a loop.
    Ever {
        /// `within d`: the deadline of each iteration, in milliseconds, evaluated once before
        /// the first iteration, and the span of `d`, where its obligation `d >= 1` is reported
        /// (`spec/26 §7`, ADR 0116 rule 11).
        within: Option<(Atom, Span)>,
    },
}

/// The `on stop` block of a loop (`spec/26 §5`, ADR 0116 rule 20): its binders of the state,
/// and its body, whose value is the tuple of the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnStop {
    /// The binders of the state in the block.
    pub state: Vec<(Name, Type)>,
    /// The block.
    pub body: Expr,
}

/// How an iteration of a loop ends (`spec/26 §3`, ADR 0116 rule 20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExitTag {
    /// `skip`, or the end of the body: on to the next iteration.
    Next,
    /// `stop`: the loop ends.
    Stop,
}

impl ExitTag {
    /// The tag as Core prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            ExitTag::Next => "next",
            ExitTag::Stop => "stop",
        }
    }
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
    /// `let (x: τ, ...) = a in e`: the components of a tuple (`spec/17 §4`).
    LetTuple {
        /// The binders, one per component.
        names: Vec<(Name, Type)>,
        /// The tuple.
        value: Atom,
        /// The rest.
        body: Box<Expr>,
        /// The source of the tuple.
        span: Span,
    },
    /// `loop x in s with (b: τ, ...) = (a, ...) do e`: the `for` loop of `spec/17 §4`
    /// (ADR 0056, amendment of 2026-10-02). Each iteration binds `x` and the current values of
    /// the state `b, ...`, and the body returns their new values as a tuple; the loop's value
    /// is the tuple of their final values. Iterations are not calls.
    Loop {
        /// What the loop visits, evaluated once.
        source: Box<LoopSource>,
        /// The loop variable.
        var: Box<(Name, Type)>,
        /// The binders of the state in the body.
        state: Vec<(Name, Type)>,
        /// The values of the state before the loop.
        init: Vec<Atom>,
        /// The body, whose value is the tuple of the new state.
        body: Box<Expr>,
        /// The `on stop` block of a `for ever` (ADR 0116 rule 20), run once the loop has ended,
        /// with its final state, or after a failure of the body with the state from before the
        /// loop, which then propagates. Its value, the tuple of the state, is the loop's.
        on_stop: Option<Box<OnStop>>,
        /// The loop in the source.
        span: Span,
    },
    /// `exit_k(b, ...)`: ends the current iteration of the innermost enclosing loop with the
    /// state `b, ...`, by the exit tag `k`: `Next` goes on to the next iteration, as the end
    /// of the body does with the body's value, and `Stop` ends the loop, whose value is then
    /// that state (`spec/26 §3`, ADR 0116 rule 20). It occurs only in a loop's body, outside
    /// any lambda or task in it.
    Exit {
        /// The tag: `skip` or `stop`.
        tag: ExitTag,
        /// The values of the loop's state, in the order of its binders.
        state: Vec<Atom>,
        /// The `stop` or `skip` in the source.
        span: Span,
    },
    /// `together(e₁ ∥ ... ∥ eₙ)`: the tasks of a `together` block (`spec/19 §5`). Its value
    /// is the tuple of the tasks' results; a failure of task `i` cancels the tasks after it,
    /// and the block fails with the failure of the lowest-indexed task that failed (`spec/19
    /// §6`).
    Together {
        /// The tasks, in branch order, each closed over the bindings before the block.
        tasks: Vec<Expr>,
        /// The block in the source.
        span: Span,
    },
    /// `together_for x in s. e`: one task per element of `s` (`spec/19 §5`), whose value is
    /// the list of the tasks' results, in element order.
    TogetherFor {
        /// The elements, evaluated once.
        source: Box<LoopSource>,
        /// The variable of each task.
        var: Box<(Name, Type)>,
        /// The task of one element.
        task: Box<Expr>,
        /// The expression in the source.
        span: Span,
    },
    /// `first_of(e₁ | ... | eₙ)`: the branches of a `first of` race (`spec/27 §6`, ADR 0117
    /// rule 6), each a task closed over the bindings before it. Its value is the result of the
    /// first branch to return, and the other branches are cancelled; when every branch fails,
    /// it fails with the failure of the lowest-indexed one. The interpreter runs the branches
    /// in index order and stops at the first that returns, one legal schedule.
    FirstOf {
        /// The branches, in index order.
        tasks: Vec<Expr>,
        /// The expression in the source.
        span: Span,
    },
    /// `tasks(b) e`: the block of a `tasks` statement (`spec/27 §7`, ADR 0117 rule 7), run in
    /// the current task. The `start`s inside `e`, outside any task of it, start tasks that run
    /// beside it; the statement ends once `e` has ended and every task it started has ended,
    /// also when `e` fails, after cancelling them, or leaves by an `exit` of an enclosing loop.
    /// Its value is `e`'s: the tuple of the outer bindings `e` updates, or `unit`.
    Tasks {
        /// The bound of `at most b at once`, an `Int` bound before the statement, and where it
        /// is written, for its obligation `b >= 1`; none for the default bound.
        bound: Option<(Atom, Span)>,
        /// The block.
        body: Box<Expr>,
        /// The statement in the source.
        span: Span,
    },
    /// `start e`: starts a task for `e`, a `Unit` that cannot fail, closed over the bindings
    /// before it, and goes on; once as many tasks as its `tasks` statement's bound run, it
    /// first waits until one ends (`spec/27 §7`, ADR 0117 rule 7). Its value is `unit`.
    Start {
        /// The started expression, with its deadline and handling clauses.
        task: Box<Expr>,
        /// The bindings `task` captures whose types can hold a `ReplyRight` of `std.host`
        /// (ADR 0123 rule 4.2, IF6): a function value among them in a package that can hold
        /// reply rights, and a value of a type parameter there. Only these are walked at the
        /// start for the reply rights that move to the started task; empty in a package that
        /// holds none.
        rights: Vec<Name>,
        /// The statement in the source.
        span: Span,
    },
    /// `quiet e` (S-39, `spec/26 §6` rule 4): `e`, a `for ever` loop, a `within`, a task
    /// construct, a `start` (S-40) or a call or an operation that can run one, stands under a
    /// program counter above `public`. A host stop that ends a `for ever` loop that `e` runs, in
    /// its callees and in the tasks it starts included, appends no `stop` event to the trace, so
    /// the event, and the `seq` of the events after it, never reveal the data the program
    /// counter depends on. Its value is `e`'s.
    Quiet {
        /// The loop, or the `let` of the call that ends in its result.
        body: Box<Expr>,
        /// Whether `e` is quiet only by the timing floor of a `within` or a `first of`, at a
        /// program counter and effect floor that are `public`: the operations it performs then
        /// keep their events, whose occurrence is the reported timing channel (`spec/27 §5`
        /// rule 7). Otherwise they append none (S-42, #1515).
        timing: bool,
        /// The loop or the call in the source.
        span: Span,
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
    Attempt {
        /// The expression attempted.
        body: Box<Expr>,
        /// `attempt` in the source.
        span: Span,
    },
    /// `within(d) e`: `e` bounded by the deadline `d`, in milliseconds after `d` is evaluated
    /// (`spec/27 §5`, ADR 0117 rule 5). Its value is `e`'s when `e` returns before the deadline.
    /// Otherwise it fails with `timeout()`, a variant without fields: `e` is cancelled at its
    /// next check point once the deadline passes, and the tasks it started with it. A failure
    /// `x` of `e` in time fails as `wrap(x)` when there is a `wrap`, else as `x`. The verifier
    /// assumes the facts of `e` on success, and proves `d >= 1` at `deadline_span`.
    Within {
        /// The deadline, an `Int`, bound before the expression.
        deadline: Atom,
        /// The deadline in the source, where its obligation `d >= 1` is reported.
        deadline_span: Span,
        /// The expression bounded.
        body: Box<Expr>,
        /// The failure when the deadline passes: `Timed.TimedOut` or `Deadline.Expired` of
        /// `std.time`, or the function's own `TimedOut` under `if error pass`.
        timeout: Box<Ctor>,
        /// The variant that wraps a failure of `e` in time: `Timed.Failed`, whose one field is
        /// the failure; none under `if error pass`, or when `e` cannot fail.
        wrap: Option<Box<Ctor>>,
        /// The whole `e within d` in the source, where its time-out is located.
        span: Span,
    },
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
    /// which the `exhaustive` obligation proves is never taken.
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
    /// The error kind's declaration, by its qualified identity (`spec/09 §3`), when it is
    /// resolved: what a failure of the operation holds, which a native trace renders
    /// (`spec/16 §3`).
    pub error: Option<Box<Path>>,
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

/// The built-in constraint of a type parameter (`spec/20 §1`), written or inferred, in order:
/// each includes the ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Constraint {
    /// No constraint: the parameter takes no type descriptor (`spec/20 §7`).
    #[default]
    None,
    /// `==` and `!=`.
    Eq,
    /// Also `<`, `<=`, `>` and `>=`.
    Ord,
    /// Also `x.to_int()`.
    Integer,
}

/// A function declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fun {
    /// The name.
    pub name: String,
    /// The type parameters.
    pub type_params: Vec<String>,
    /// The constraint of each type parameter, in their order (`spec/20 §1`).
    pub constraints: Vec<Constraint>,
    /// The parameters.
    pub params: Vec<(Name, Type)>,
    /// The result type.
    pub result: Type,
    /// The capability context.
    pub uses: Vec<Use>,
    /// The parameters of function type under `uses of` (ADR 0062), whose arguments' effects
    /// run through the function's bindings: what the verifier charges where they are called
    /// or passed on (ADR 0092 rule 3). Not exported: `una core` derives nothing from it.
    pub uses_of: Vec<Name>,
    /// The locked conditions of its operations, from `una.toml`'s `[contracts]` (ADR 0092
    /// rule 8, as amended), each where the header grants the operation. Not exported: the lock
    /// is the consumer's, not the function's.
    pub locks: Vec<Lock>,
    /// The error kind.
    pub fails: Option<String>,
    /// The error kind's declaration, by its qualified identity (`spec/09 §3`), when it is
    /// resolved: what a failure of the function holds, which the native backend describes for
    /// U6001 (`spec/16 §3`).
    pub error: Option<Box<Path>>,
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
    /// The constraint of each type parameter, in their order (`spec/20 §1`).
    pub constraints: Vec<Constraint>,
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
        /// The declaration of the capability it implements, by its qualified identity
        /// (`spec/10 §11` rule 8).
        capability_path: Path,
        /// Its own capability context.
        uses: Vec<Use>,
        /// The operations.
        ops: Vec<HandlerOp>,
    },
    /// `fun f[α*](x: τ, ...) : τ uses Κ fails E = e`.
    Fun(Box<Fun>),
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
    /// The `N` of its `stop after N` line, in edition 0.4: a host stop arrives once `N`
    /// iterations of `for ever` loops have started (ADR 0116 rule 19, `spec/26 §8`).
    pub stop_after: Option<u64>,
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
    /// The checks of its module constants, edition 0.5, in source order (ADR 0126 rule 7).
    /// They are not declarations: every reference to a constant is substituted by its value
    /// (rule 8), and no engine runs a check. Only the verifier reads them, for the
    /// obligations of a constant's declared type at its value.
    pub constants: Vec<ConstantCheck>,
}

/// The check of a module constant (ADR 0126 rule 7): its value, elaborated as at a reference,
/// bound to its declared type, as an annotated `const` binds a value, so that each refinement
/// of the type, at the top or nested, is an obligation at the value, with no hypothesis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstantCheck {
    /// The constant's name.
    pub name: String,
    /// `let name: τ = value in name`, the binder written.
    pub body: Expr,
    /// The value in the source, always in a module of the package, never in a standard
    /// module, whose file ids the package's share. Every obligation that the elaboration of
    /// the value gives, at a position inside this span, is generated by this check once; a
    /// reference, which elaborates the same value with the same types, does not generate it
    /// again.
    pub span: Span,
}

/// A package's profile (ADR 0109): `app`, the language as it is, or `systems`, which only adds
/// obligations and declarations to the same program (rule 2). Both manifest formats declare it,
/// and Core IR records it (rule 6), so the verifier and the Core export read it from the
/// program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    /// The default: the language as it is.
    #[default]
    App,
    /// Stricter than or equal to `app` (ADR 0109 rule 2): each `Int` operation outside
    /// clauses carries an `overflow` obligation (rule 3).
    Systems,
}

impl Profile {
    /// The profile's name, as both manifest formats and the Core export write it.
    pub fn name(self) -> &'static str {
        match self {
            Profile::App => "app",
            Profile::Systems => "systems",
        }
    }

    /// The profile named `text`, if it is one.
    pub fn parse(text: &str) -> Option<Profile> {
        match text {
            "app" => Some(Profile::App),
            "systems" => Some(Profile::Systems),
            _ => None,
        }
    }
}

/// The Core IR of a package: its modules, in path order, and the contracts of the standard
/// library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The package's profile (ADR 0109 rules 3 and 6), set by elaboration from the manifest:
    /// in `systems`, the verifier gives each `Int` operation outside clauses an `overflow`
    /// obligation.
    pub profile: Profile,
    /// The modules.
    pub modules: Vec<Module>,
    /// The functions of the prelude and `std.testing`.
    pub library: Vec<Signature>,
    /// The capability declarations of [`HOST_MODULE`], with their `spec/14 §3` signatures.
    pub host: Vec<Decl>,
    /// The edits that `una fix` may apply for `effect_condition` obligations at calls and
    /// `with`s, in site order (ADR 0092, amendment of 2026-10-14). Not exported: `una core`
    /// derives nothing from them.
    pub condition_edits: Vec<ConditionEdit>,
    /// Whether the verifier keeps a map's other keys across its updates: the frame facts of
    /// ADR 0119 V3 on `Map`, in edition `0.5` and later only, the root package's (ADR 0069, amendment of
    /// 2026-10-19). Not exported: `una core` derives nothing from it.
    pub map_frames: bool,
}

/// The condition that a call or a `with` asks of its callee or handler, written on the callee's
/// operation: the first repair of the site's `effect_condition` obligation, as an edit (ADR
/// 0092, amendment of 2026-10-14). The checker computes it from the source when conditions (a)
/// to (e) and (g) hold; the verifier keeps it only under (f), and the report only under (h).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionEdit {
    /// The site: the call, or the handler call of the `with`.
    pub site: Span,
    /// The caller's own binding that the site passes, without `@`.
    pub binding: String,
    /// The conditioned operation.
    pub op: String,
    /// The callee or handler, by the name the verifier gives the site.
    pub callee: String,
    /// The range of the callee's header that the edit replaces.
    pub range: Span,
    /// The replacement.
    pub replacement: String,
}

/// The module that declares the host capabilities (`spec/14 §3`, `spec/10 §11` rule 8).
pub const HOST_MODULE: &str = "std.host";

/// The module path of the prelude's kinds and functions (`spec/14 §1`). No package module has
/// it: a module path whose first segment is `prelude` is reserved (U1009, `spec/10 §11`
/// rule 8), and tier 1 and the interpreter only see packages without diagnostics.
pub const PRELUDE_MODULE: &str = "prelude";

/// The kinds of the prelude that Core IR does not declare, which each engine knows itself: the
/// collections and a map's `Entry`, `Option` and `Result`, the failure kinds of `Files` and
/// `Keys`, `ParseError`, and the dense types of edition 0.4 (`spec/29 §17`). Every other kind
/// of the prelude is a [`Decl::Data`] of the module [`PRELUDE_MODULE`]. A `Named` kind that is
/// neither, nor declared by the package, is a bug of elaboration, and the verifier and the
/// native backend stop on it (#407).
pub const PRELUDE_ENGINE_KINDS: [&str; 12] = [
    "List",
    "Map",
    "Array",
    "Matrix",
    "Tensor",
    "Lu",
    "Entry",
    "Option",
    "Result",
    "FileError",
    "KeyError",
    "ParseError",
];

/// The type of a field of a variant of a kind of [`PRELUDE_ENGINE_KINDS`]: one of the kind's
/// type arguments, by position, or `Int`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreludeField {
    /// The type argument at this position.
    Arg(usize),
    /// `Int`.
    Int,
}

/// A variant of a kind of [`PRELUDE_ENGINE_KINDS`]: its name, and the names and types of its
/// fields, in order.
pub type PreludeVariant = (&'static str, &'static [(&'static str, PreludeField)]);

/// The variants of a kind of [`PRELUDE_ENGINE_KINDS`] that holds data, in the order the
/// prelude declares them (`spec/14 §1`, `spec/21 §2`, `spec/22 §4`). The interpreter names the
/// fields of these values by it and the native backend describes them by it, so that a trace
/// shows them alike (#415). None of their fields declares a label: the label of what they hold
/// is their type arguments'. `None` for the collections, which are not data, and any other name.
pub fn prelude_variants(kind: &str) -> Option<&'static [PreludeVariant]> {
    use PreludeField::{Arg, Int};
    match kind {
        "Option" => Some(&[("Some", &[("value", Arg(0))]), ("None", &[])]),
        "Result" => Some(&[
            ("Ok", &[("value", Arg(0))]),
            ("Err", &[("failure", Arg(1))]),
        ]),
        "Entry" => Some(&[("Entry", &[("key", Arg(0)), ("value", Arg(1))])]),
        "FileError" => Some(&[("NotFound", &[]), ("Denied", &[]), ("Io", &[])]),
        "KeyError" => Some(&[("UnknownKey", &[])]),
        "ParseError" => Some(&[
            ("Empty", &[]),
            ("NotADigit", &[("at", Int)]),
            ("NoDigits", &[]),
            ("TooLong", &[]),
        ]),
        _ => None,
    }
}

/// The module path of the collections of edition `0.4` (ADR 0119 rule 0): their types are
/// `Named` in it, and its kind `Ranked` is a [`Decl::Data`] of it.
pub const COLLECTIONS_MODULE: &str = "std.collections";

/// The module path of rate limits computed in code, edition `0.4` (ADR 0117 rule 9): its kind
/// `Limited` is a [`Decl::Data`] of it, its capability `Limit` a [`Decl::Cap`], and the handler
/// [`LIMITER`] is provided by each engine, so no [`Decl::Handler`] of it exists.
pub const LIMIT_MODULE: &str = "std.limit";

/// The handler of [`LIMIT_MODULE`] that each engine provides (ADR 0117 rule 9): a `with` or a
/// `given` of it is `Source::Handler` with this name in that module, and its arguments are
/// `at_most`, `per` and `waiting`, in that order.
pub const LIMITER: &str = "limiter";

/// A collection type of `std.collections`, edition `0.4` (ADR 0119). Each engine knows its
/// values itself, as it knows a list's and a map's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Coll {
    /// `Stack[T]`, last in, first out (rule 5).
    Stack,
    /// `Queue[T]`, first in, first out, with both ends open (rule 6).
    Queue,
    /// `Set[T]`, an ordered set over an `Ord` item (rule 7).
    Set,
    /// `HashMap[K, V]`, keyed by any `Eq` type, in insertion order (rules 8 to 12).
    HashMap,
    /// `HashSet[T]`, of any `Eq` item, in insertion order (rules 8 to 12).
    HashSet,
    /// `PriorityQueue[T, P]`, a stable heap over an `Ord` priority (rule 13).
    PriorityQueue,
    /// `BitSet`, the integers of a fixed range (rule 14).
    BitSet,
}

impl Coll {
    /// Every collection type, in the order of ADR 0119's export list.
    pub const ALL: [Coll; 7] = [
        Coll::Stack,
        Coll::Queue,
        Coll::Set,
        Coll::HashMap,
        Coll::HashSet,
        Coll::PriorityQueue,
        Coll::BitSet,
    ];

    /// The name of the type, as `std.collections` exports it.
    pub fn as_str(self) -> &'static str {
        match self {
            Coll::Stack => "Stack",
            Coll::Queue => "Queue",
            Coll::Set => "Set",
            Coll::HashMap => "HashMap",
            Coll::HashSet => "HashSet",
            Coll::PriorityQueue => "PriorityQueue",
            Coll::BitSet => "BitSet",
        }
    }

    /// The collection type named `name`, if any.
    pub fn from_name(name: &str) -> Option<Coll> {
        Coll::ALL.into_iter().find(|coll| coll.as_str() == name)
    }

    /// The number of its type arguments.
    pub fn arity(self) -> usize {
        match self {
            Coll::BitSet => 0,
            Coll::HashMap | Coll::PriorityQueue => 2,
            Coll::Stack | Coll::Queue | Coll::Set | Coll::HashSet => 1,
        }
    }

    /// Whether its values can satisfy `Eq` (ADR 0119 rule 2): a hash map, a hash set and a
    /// priority queue never do.
    pub fn has_eq(self) -> bool {
        !matches!(self, Coll::HashMap | Coll::HashSet | Coll::PriorityQueue)
    }

    /// The constraint that its type argument `index` must satisfy: `Ord` for a set's item and
    /// a priority, `Eq` for a hash key or item (ADR 0119 rules 7, 8 and 13).
    pub fn needs(self, index: usize) -> Constraint {
        match (self, index) {
            (Coll::Set, 0) | (Coll::PriorityQueue, 1) => Constraint::Ord,
            (Coll::HashMap | Coll::HashSet, 0) => Constraint::Eq,
            _ => Constraint::None,
        }
    }

    /// Whether its type argument `index` is part of its shape: the items of a set, the keys of
    /// a hash map and the priorities of a queue decide its size or its order, so their labels
    /// are below its top label (ADR 0119 rule 3, IF2 to IF4).
    pub fn is_shape(self, index: usize) -> bool {
        matches!(
            (self, index),
            (Coll::Set | Coll::HashMap | Coll::HashSet, 0) | (Coll::PriorityQueue, 1)
        )
    }

    /// What its type argument `index` is called in messages: `item`, `key`, `value` or
    /// `priority`.
    pub fn arg_role(self, index: usize) -> &'static str {
        match (self, index) {
            (Coll::HashMap, 0) => "key",
            (Coll::HashMap, _) => "value",
            (Coll::PriorityQueue, 1) => "priority",
            _ => "item",
        }
    }
}

#[cfg(test)]
mod float_op_tests {
    use super::FloatOp;

    /// The position of each operation in declaration order. The match is exhaustive, so a new
    /// operation does not compile until it has a position here, and the test below then
    /// requires it in `FloatOp::ALL` at that position.
    fn position(op: FloatOp) -> usize {
        match op {
            FloatOp::Add => 0,
            FloatOp::Sub => 1,
            FloatOp::Mul => 2,
            FloatOp::Div => 3,
            FloatOp::Neg => 4,
            FloatOp::Eq => 5,
            FloatOp::Ne => 6,
            FloatOp::Lt => 7,
            FloatOp::Le => 8,
            FloatOp::Gt => 9,
            FloatOp::Ge => 10,
            FloatOp::Abs => 11,
            FloatOp::Sqrt => 12,
            FloatOp::Fma => 13,
            FloatOp::Rem => 14,
            FloatOp::Floor => 15,
            FloatOp::Ceil => 16,
            FloatOp::Trunc => 17,
            FloatOp::RoundHalfEven => 18,
            FloatOp::RoundHalfAway => 19,
            FloatOp::CopySign => 20,
            FloatOp::IeeeMin => 21,
            FloatOp::IeeeMax => 22,
            FloatOp::Clamp => 23,
            FloatOp::IeeeEq => 24,
            FloatOp::IeeeNe => 25,
            FloatOp::IeeeLt => 26,
            FloatOp::IeeeLe => 27,
            FloatOp::IeeeGt => 28,
            FloatOp::IeeeGe => 29,
            FloatOp::IsNan => 30,
            FloatOp::IsFinite => 31,
            FloatOp::IsInfinite => 32,
            FloatOp::IsSignNegative => 33,
            FloatOp::IsZero => 34,
            FloatOp::IsPositiveZero => 35,
            FloatOp::IsNegativeZero => 36,
            FloatOp::ToBits => 37,
            FloatOp::ToIntExact => 38,
            FloatOp::FromInt => 39,
            FloatOp::ToText => 40,
            FloatOp::ToF32 => 41,
            FloatOp::ToF64 => 42,
        }
    }

    #[test]
    fn every_operation_has_one_code_the_backend_and_the_runtime_share() {
        // The native backend passes an operation to the runtime by its position in
        // `FloatOp::ALL` (ADR 0114 rule 9.6): each operation is there exactly once.
        for (index, op) in FloatOp::ALL.iter().enumerate() {
            assert_eq!(position(*op), index, "{op:?}");
        }
    }
}
