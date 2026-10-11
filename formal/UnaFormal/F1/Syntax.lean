/-
M5 (`docs/plan/formal-track-m5.md`): **F1 with a stateful host**. Its host operations are those
of the stateful capabilities, `Files.read`, `Files.write` and a request of `Http` (its four
operations differ only in their method), and its evaluation threads the host's state. M5 staged
it beside the model with a stateless host, and #459 made it the one model.

The fragment F1 (`docs/plan/formal-track.md` §3): the part of Una Core IR (`spec/09`) in A-normal
form, as `una.core/1` exports it (`spec/13 §8`), in which the findings S-01 to S-12 but S-04 can
be written.

The names follow Core IR: an `Atom` is a variable or a literal, a `Rhs` is what a `let` binds,
and an `Expr` is a body. Types in the syntax are labeled at every position (`LTy`, `spec/07 §7`
rule 1); `Ty` is their structure, which the evaluator and T1 need, and labels do not change.
-/
import UnaFormal.F1.Labels

namespace Una.F1

/-- The structure of a type: scalars, lists, options, results with their error type, the package's
records by name, function types and type parameters. -/
inductive Ty where
  | int
  | bool
  | text
  | unit
  | list (element : Ty)
  | option (element : Ty)
  | result (value err : Ty)
  | record (name : String)
  | fn (params : List Ty) (result : Ty)
  | var (name : String)
  /-- Core's tuples (`spec/09 §2`, ADR 0056), which only the memory model's desugaring makes:
  a loop's state and an `edit` call's results (`spec/17 §4`). -/
  | tuple (items : List Ty)

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

theorem HostFx.Le.refl (a : HostFx) : HostFx.Le a a := ⟨id, id, id⟩

theorem HostFx.Le.trans {a b c : HostFx} (h₁ : HostFx.Le a b) (h₂ : HostFx.Le b c) :
    HostFx.Le a c :=
  ⟨h₂.1 ∘ h₁.1, h₂.2.1 ∘ h₁.2.1, h₂.2.2 ∘ h₁.2.2⟩

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
  | result (value err : LTy) (l : LExpr)
  | record (name : String) (l : LExpr)
  | fn (params : List LTy) (result : LTy) (op : Bool) (host : HostFx) (fails : Bool) (l : LExpr)
  | var (name : String) (l : LExpr)
  | tuple (items : List LTy) (l : LExpr)

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
  /-- `*` on integers (#526). -/
  | mul
  /-- Euclidean `div` and `mod` (`spec/11`: the remainder is non-negative, as SMT-LIB's), #526. A
  zero divisor aborts the run (`Prim.aborts`). -/
  | div
  | mod
  | lt
  | eqInt
  | not
  | len
  /-- The prelude's `sum` of a list of integers. -/
  | sum
  /-- `concat` of texts, any number of them: what interpolation desugars to (`spec/09 §3`). -/
  | concat

/-- What a loop visits (`spec/17 §4`): the elements of a list, in order, or the integers from
`low` to `high`, in increasing order, `high` included when `inclusive`. -/
inductive Source where
  | list (xs : Atom)
  | range (low high : Atom) (inclusive : Bool)

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
    arguments, of the types `httpParams` fixes: every one a sink at its `accepts`, and every one
    deciding its failure (S-15). Its response is a `Response` record. -/
    | req (method : String) (args : List Atom)
    | construct (record : String) (fields : List Atom)
    /-- `C(ā)` of a sum kind (#512): the variant `ctor` of `kind`, with the fields `ā`. -/
    | variant (kind : String) (ctor : Nat) (fields : List Atom)
    | field (value : Atom) (index : Nat)
    | list (items : List Atom)
    | none
    | some (value : Atom)
    /-- `Ok(a)` and `Err(a)`, a result built directly (#508). -/
    | mkOk (value : Atom)
    | mkErr (value : Atom)
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
    /-- `checked(a, φ) else fail C` (`spec/10 §7`, T-Checked, #453): `a`, when the predicate
    named `pred` holds of it, or the failure `code`. The predicate is abstract: its meaning is
    the program's `predSem`, so the theorems hold whatever the refinement language makes it.
    Its operand is a scalar, which the predicate reads as its text (`Val.line`). -/
    | checked (value : Atom) (pred : String) (code : Nat)
    /-- `a == b` on values of any type with equality, `T` with `Eq` included (`spec/20`):
    the comparison whose result carries the decision `ℓ*_T` of `spec/07 §7` rule 8. -/
    | eq (a b : Atom)
    /-- A tuple of Core (`spec/17 §4`). -/
    | tuple (items : List Atom)
    /-- The prelude's `append`: `xs` with `item` at its end. -/
    | append (xs item : Atom)
    /-- The prelude's `set(edit xs, index, item)` (`spec/17 §8`), which returns the tuple of
    the new list and `unit`, as its `edit` parameter desugars. -/
    | set (xs index item : Atom)
    /-- `to_text(a)`: the text of a value (`spec/09 §3`, interpolation), at a type with equality
    (M6b). -/
    | toText (a : Atom)
    /-- `Env.get(name)` of `std.host` (`spec/14 §3`): the variable, as `Option[internal Text]`, from
    the world's environment, where two runs may hold different values (#487). -/
    | envGet (name : Atom)

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
    /-- `fail C(ā)` of a payload kind (#512): the failure of kind `kind`, variant `code`, with
    the payload `args`. A kind whose variants have no fields stays a code (`fail`). -/
    | raise (kind : String) (code : Nat) (args : List Atom)
    /-- `when a is C(x₁, ...) -> t else e` on a sum kind (#512): `t` with the fields bound when
    `a` is the variant `ctor`, `e` otherwise. Una's `when` with several arms is a chain of them. -/
    | caseSum (scrutinee : Atom) (ctor : Nat) (binders : List String) (thenE elseE : Expr)
    /-- `attempt e`: the value or the failure of `e`, as a result. -/
    | attempt (body : Expr)
    /-- `e if error x -> h`: `h` runs when `e` fails, with its code bound to `x`. -/
    | handle (body : Expr) (error : String) (handler : Expr)
    /-- `attempt e` where `e` fails with the payload kind `kind` (#512): its failure, as the
    value `Err(C(ā))`. A failure of another kind is not caught; the importer emits it only for
    a body whose failures are of `kind`. -/
    | attemptK (kind : String) (body : Expr)
    /-- `e if error x -> h` where `e` fails with the payload kind `kind` (#512): `h` runs with
    the failure's value bound to `x`. A failure of another kind is not caught. -/
    | handleK (kind : String) (body : Expr) (error : String) (handler : Expr)
    /-- `let (x₁: S₁, ...) = a in e`: the components of a tuple (`spec/17 §4`). -/
    | letTuple (binders : List (String × LTy)) (value : Atom) (body : Expr)
    /-- `loop x: V in s with (b₁: S₁, ...) = (a₁, ...) do e` (`spec/17 §4`, ADR 0056): each
    iteration binds `x` and the current state, and the body's value is the tuple of the new
    state; the loop's value is the tuple of the final state. Iterations are not calls. -/
    | loop (source : Source) (x : String) (xty : LTy) (state : List (String × LTy))
        (init : List Atom) (body : Expr)
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
  /-- The payload kinds its failures may have (#512): its `fails` kind when that is a kind with
  a field, so that `attempt` and a handler know what they receive. -/
  raises : List String := []

/-- A record kind of the program: its field types, with their declared labels, in order, and
the join of every label they declare, which the checker computes over nested records (the
least solution, `spec/07 §7` rule 2) and well-formedness checks. -/
structure Record where
  name : String
  fields : List LTy
  deep : Label

/-- A sum kind of the program (#512): for each variant, in order, its field types with their
declared labels, and the join of every label they declare (`spec/07 §7` rule 3, the least
solution over nested kinds). Its values are typed by `LTy.record`, the type of a named kind,
whose name the program's records and sums never share. -/
structure Sum where
  name : String
  variants : List (List LTy)
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
  /-- M6's draft rule for loops, for the counterexample only (`LoopLeak.lean`): an iteration's
  failure not bounded by what decides whether the iteration runs. `t6_host` requires it `false`. -/
  oldLoop : Bool := false
  records : List Record
  /-- The sum kinds (#512). -/
  sums : List Sum := []
  policies : List Policy
  /-- The meaning of `checked`'s predicates (#453), by name, on the text of a scalar operand:
  any function, since the theorems quantify over the program. The refinement language that
  defines a predicate is the verifier's, and the model trusts no evaluator of it. -/
  predSem : String → String → Bool := fun _ _ => true
  funs : List Fun

/-- `⊤`: `restricted` with every declared category. -/
def Program.top (p : Program) : Label := ⟨.restricted, p.categories⟩

/-- The record kind `r` of the program. -/
def Program.record (p : Program) (r : String) : Option Record :=
  p.records.find? (fun rec => rec.name = r)

/-- The sum kind `k` of the program (#512). -/
def Program.sum (p : Program) (k : String) : Option Sum :=
  p.sums.find? (fun s => s.name = k)

/-- The join of every label the fields of the named kind `r` declare: a record's, or a sum's
over all its variants (#512). -/
def Program.recordDeep (p : Program) (r : String) : Label :=
  match p.record r with
  | some rec => rec.deep
  | none =>
    match p.sum r with
    | some s => s.deep
    | none => Label.bot

/-- The function `f` of the program. -/
def Program.find (p : Program) (f : String) : Option Fun :=
  p.funs.find? (fun fn => fn.name = f)

/-- The policy `pol` of the program. -/
def Program.policy (p : Program) (pol : String) : Option Policy :=
  p.policies.find? (fun po => po.name = pol)

/-- **`Http`'s operations, as `spec/24 §1` fixes them** (M5c, #470): the structure of each
method's parameters. `get` and `delete` take a URL, headers and an optional key name; `post` and
`put` also take a body. The re-check takes them from here, not from the export. -/
def httpParams : String → Option (List Ty)
  | "get" | "delete" => some [.text, .list (.record "prelude.Header"), .option .text]
  | "post" | "put" => some [.text, .list (.record "prelude.Header"), .text, .option .text]
  | _ => none

end Una.F1
