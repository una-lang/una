/-
M5 (`docs/plan/formal-track-m5.md`): **F1 with a stateful host**, F1H. It is F1 with the host
operation `get` replaced by the operations of the stateful capabilities, `Files.read`,
`Files.write` and a request of `Http` (its four operations differ only in their method), and with
an evaluation that threads the host's state. Until `t6_host` is proven over it, F1H is staged
beside F1; this file is F1's `Syntax.lean` with those changes.

The fragment F1 (`docs/plan/formal-track.md` §3): the part of Una Core IR (`spec/09`) in A-normal
form, as `una.core/1` exports it (`spec/13 §8`), in which the findings S-01 to S-12 but S-04 can
be written.

The names follow Core IR: an `Atom` is a variable or a literal, a `Rhs` is what a `let` binds,
and an `Expr` is a body. Types in the syntax are labeled at every position (`LTy`, `spec/07 §7`
rule 1); `Ty` is their structure, which the evaluator and T1 need, and labels do not change.
-/
import UnaFormal.F1.Labels

namespace Una.F1H

open Una.F1 (Label LExpr)

/-- The structure of a type: scalars, lists, options, the results of `attempt`, the package's
records by name, function types and type parameters. -/
inductive Ty where
  | int
  | bool
  | text
  | unit
  | list (element : Ty)
  | option (element : Ty)
  | result (value : Ty)
  | record (name : String)
  | fn (params : List Ty) (result : Ty)
  | var (name : String)

/-- The host effects a code may perform: read a file, write a file, send a request. -/
structure HostFx where
  rd : Bool := false
  wr : Bool := false
  rq : Bool := false

/-- No host effect. -/
def HostFx.none : HostFx := {}

/-- Both codes' host effects. -/
def HostFx.or (a b : HostFx) : HostFx := ⟨a.rd || b.rd, a.wr || b.wr, a.rq || b.rq⟩

/-- `a`'s host effects are among `b`'s. -/
def HostFx.Le (a b : HostFx) : Prop :=
  (a.rd = true → b.rd = true) ∧ (a.wr = true → b.wr = true) ∧ (a.rq = true → b.rq = true)

/-- No host effect, as a Boolean. -/
def HostFx.isNone (a : HostFx) : Bool := !a.rd && !a.wr && !a.rq

/-- A type labeled at every position (`spec/07 §7` rule 1). A function type says which
effects its function may perform, the operation `op` of the one capability and the host
effects `host`, and whether it may fail (`spec/10 §5`, `uses` and `fails`). -/
inductive LTy where
  | int (l : LExpr)
  | bool (l : LExpr)
  | text (l : LExpr)
  | unit (l : LExpr)
  | list (element : LTy) (l : LExpr)
  | option (element : LTy) (l : LExpr)
  | result (value : LTy) (l : LExpr)
  | record (name : String) (l : LExpr)
  | fn (params : List LTy) (result : LTy) (op : Bool) (host : HostFx) (fails : Bool) (l : LExpr)
  | var (name : String) (l : LExpr)

/-- The scalar types, which the operation writes. -/
def Ty.Scalar : Ty → Prop
  | .int | .bool | .text => True
  | _ => False

/-- A variable or a literal (`spec/09 §2`). -/
inductive Atom where
  | var (name : String)
  | int (n : Int)
  | bool (b : Bool)
  | text (s : String)
  | unit

/-- The primitives of F1. -/
inductive Prim where
  | add
  | sub
  | lt
  | eqInt
  | not
  | len
  /-- The prelude's `sum` of a list of integers. -/
  | sum

mutual
  /-- What a `let` binds. -/
  inductive Rhs where
    | atom (a : Atom)
    | prim (p : Prim) (args : List Atom)
    /-- A call of the program's function `f`, with its type arguments. -/
    | call (f : String) (targs : List LTy) (args : List Atom)
    /-- A call of a function value (T-App). -/
    | apply (f : Atom) (args : List Atom)
    /-- The one operation of the one capability, `@log.write(line)`: a sink, recorded in the
    trace (T-Op). Its argument is a scalar, an `Int`, a `Bool` or a `Text`, written as its
    text. -/
    | op (line : Atom)
    /-- `Files.read(path)`: a sink at its `accepts`, whose failure the path and the file system's
    shape decide, and whose result is the file's content (`spec/14 §3`). -/
    | read (path : Atom)
    /-- `Files.write(path, content)`: a sink at its `accepts`, which changes the file system;
    whether it fails, and the shape it leaves, depend on the path and the shape, not the
    content (ADR 0085 decision 3). -/
    | write (path content : Atom)
    /-- A request of `Http`, `get`, `post`, `put` or `delete` (`spec/24 §1`), with its
    arguments: every one a sink at its `accepts`, and every one deciding its failure (S-15). -/
    | req (method : String) (args : List Atom)
    | construct (record : String) (fields : List Atom)
    | field (value : Atom) (index : Nat)
    | list (items : List Atom)
    | none
    | some (value : Atom)
    | lambda (params : List (String × LTy)) (body : Expr)
    /-- The program's function `f` as a value. -/
    | funRef (f : String) (targs : List LTy)
    /-- A call of the policy `pol` (`spec/07 §5`, T-Declassify). -/
    | declassify (pol : String) (value : Atom)
    /-- The prelude (`spec/14 §1`), as axioms: `xs.map(each: f)`. -/
    | map (xs f : Atom)
    | filter (xs keep : Atom)
    | firstWhere (xs keep : Atom)
    /-- `xs.fold(init, f)`, with `f(acc, x)`. -/
    | fold (xs init f : Atom)
    /-- The partial primitive: the integer a text spells, or a failure. -/
    | parse (s : Atom)
    /-- `a == b` on values of any type with equality, `T` with `Eq` included (`spec/20`):
    the comparison whose result carries the decision `ℓ*_T` of `spec/07 §7` rule 8. -/
    | eq (a b : Atom)

  /-- A body in A-normal form. -/
  inductive Expr where
    | atom (a : Atom)
    | letR (name : String) (ty : LTy) (value : Rhs) (body : Expr)
    | letE (name : String) (ty : LTy) (value : Expr) (body : Expr)
    | ite (cond : Atom) (thenE elseE : Expr)
    /-- `when o is None -> n is Some(x) -> s`. -/
    | caseOption (scrutinee : Atom) (none : Expr) (x : String) (some : Expr)
    /-- `when r is Ok(x) -> o is Err(y) -> e`. -/
    | caseResult (scrutinee : Atom) (x : String) (ok : Expr) (y : String) (err : Expr)
    /-- `fail` with a failure, identified by its code. -/
    | fail (code : Nat)
    /-- `attempt e`: the value or the failure of `e`, as a result. -/
    | attempt (body : Expr)
    /-- `e if error x -> h`: `h` runs when `e` fails, with its code bound to `x`. -/
    | handle (body : Expr) (error : String) (handler : Expr)
end

/-- A function of the program: its type parameters (one level of generics, `spec/07 §7`
rule 8), those of them constrained by `Eq` (`spec/20`), its signature with its `uses` and
`fails`, and its body. -/
structure Fun where
  name : String
  tparams : List String
  eqParams : List String
  params : List (String × LTy)
  result : LTy
  op : Bool
  host : HostFx
  fails : Bool
  body : Expr
  /-- The policies whose release a call may perform (option B1): its body's declassifications
  and those of the functions it calls. -/
  rels : List String := []

/-- A record kind of the program: its field types, with their declared labels, in order, and
the join of every label they declare, which the checker computes over nested records (the
least solution, `spec/07 §7` rule 2) and well-formedness checks. -/
structure Record where
  name : String
  fields : List LTy
  deep : Label

/-- A declassification policy (`spec/07 §5`): `from`, `to` and its `via` function. -/
structure Policy where
  name : String
  src : LTy
  dst : LTy
  via : String

/-- A program: its declared categories, the `accepts` labels of the operation and of the host's
stateful operations, the labels of their results and failures, its records, policies and
functions. -/
structure Program where
  categories : List String
  acceptsOp : Label
  /-- What `Files.read` accepts for its path (`public`, `spec/14 §3`). -/
  acceptsRead : Label
  /-- What `Files.write` accepts for its path and content (`internal`). -/
  acceptsWrite : Label
  /-- What a request of `Http` accepts for every argument (`internal`, `spec/24 §1`). -/
  acceptsReq : Label
  /-- The label of a read's content (`internal`). -/
  readResult : Label
  /-- The label of a request's response (`internal`). -/
  reqResult : Label
  /-- The least label of a request's failure: what its headers, whose value `Header` declares
  `internal`, always add (S-15, ADR 0085 decision 7). -/
  reqFloor : Label
  /-- The file-state label of the package (S-14, ADR 0085 decision 1). -/
  fileState : Label
  /-- The rules before ADR 0085, for the counterexamples only: a `Files` failure labeled by its
  path alone, no condition on the file-state label at a write, and a request's failure labeled
  by its URL alone. `t6_host` requires it `false`. -/
  oldHost : Bool := false
  records : List Record
  policies : List Policy
  funs : List Fun

/-- `⊤`: `restricted` with every declared category. -/
def Program.top (p : Program) : Label := ⟨.restricted, p.categories⟩

/-- The record kind `r` of the program. -/
def Program.record (p : Program) (r : String) : Option Record :=
  p.records.find? (fun rec => rec.name = r)

/-- The join of every label the fields of record kind `r` declare. -/
def Program.recordDeep (p : Program) (r : String) : Label :=
  match p.record r with
  | some rec => rec.deep
  | none => Label.bot

/-- The function `f` of the program. -/
def Program.find (p : Program) (f : String) : Option Fun :=
  p.funs.find? (fun fn => fn.name = f)

/-- The policy `pol` of the program. -/
def Program.policy (p : Program) (pol : String) : Option Policy :=
  p.policies.find? (fun po => po.name = pol)

end Una.F1H
