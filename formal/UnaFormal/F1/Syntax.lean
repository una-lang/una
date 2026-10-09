/-
The fragment F1 (`docs/plan/formal-track.md` §3): the part of Una Core IR (`spec/09`) in A-normal
form, as `una.core/1` exports it (`spec/13 §8`), in which the findings S-01 to S-12 but S-04 can
be written.

The names follow Core IR: an `Atom` is a variable or a literal, a `Rhs` is what a `let` binds,
and an `Expr` is a body. Types in the syntax are labeled at every position (`LTy`, `spec/07 §7`
rule 1); `Ty` is their structure, which the evaluator and T1 need, and labels do not change.
-/
import UnaFormal.F1.Labels

namespace Una.F1

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

/-- A type labeled at every position (`spec/07 §7` rule 1). A function type says which
effects its function may perform, the operation `op` of the one capability and the host
operation `get`, and whether it may fail (`spec/10 §5`, `uses` and `fails`). -/
inductive LTy where
  | int (l : LExpr)
  | bool (l : LExpr)
  | text (l : LExpr)
  | unit (l : LExpr)
  | list (element : LTy) (l : LExpr)
  | option (element : LTy) (l : LExpr)
  | result (value : LTy) (l : LExpr)
  | record (name : String) (l : LExpr)
  | fn (params : List LTy) (result : LTy) (op get fails : Bool) (l : LExpr)
  | var (name : String) (l : LExpr)

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
    trace (T-Op). -/
    | op (line : Atom)
    /-- The host operation, `Http.get(url)`: a sink whose failure its argument decides
    (`spec/07 §7` rule 4, N-39). -/
    | get (url : Atom)
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
rule 8), its signature with its `uses` and `fails`, and its body. -/
structure Fun where
  name : String
  tparams : List String
  params : List (String × LTy)
  result : LTy
  op : Bool
  get : Bool
  fails : Bool
  body : Expr

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

/-- A program: its declared categories, the `accepts` labels of the operation and of the host
operation, the label of the host operation's result, its records, policies and functions. -/
structure Program where
  categories : List String
  acceptsOp : Label
  acceptsGet : Label
  getResult : Label
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

end Una.F1
