/-
F1a, the first part of the fragment F1 (`docs/plan/formal-track.md` §3), for M1: the
monomorphic core of Una Core IR (`spec/09`) in A-normal form, as `una.core/1` exports it
(`spec/13 §8`). Type parameters, labels and their symbolic labels join in M3 and M4, where
noninterference needs them; type safety (T1) does not.

The names follow Core IR: an `Atom` is a variable or a literal, a `Rhs` is what a `let`
binds, and an `Expr` is a body.
-/
namespace Una.F1

/-- The types of F1a: scalars, lists, options, the results of `attempt`, the package's
records by name, and function types. A function type says whether the function can fail. -/
inductive Ty where
  | int
  | bool
  | text
  | unit
  | list (element : Ty)
  | option (element : Ty)
  | result (value : Ty)
  | record (name : String)
  | fn (params : List Ty) (result : Ty) (fails : Bool)
  deriving Repr

/-- A variable or a literal (`spec/09 §2`). -/
inductive Atom where
  | var (name : String)
  | int (n : Int)
  | bool (b : Bool)
  | text (s : String)
  | unit
  deriving Repr

/-- The primitives of F1a. -/
inductive Prim where
  | add
  | sub
  | lt
  | eqInt
  | not
  | len
  deriving Repr

mutual
  /-- What a `let` binds. -/
  inductive Rhs where
    | atom (a : Atom)
    | prim (p : Prim) (args : List Atom)
    /-- A call of the program's function `f`. -/
    | call (f : String) (args : List Atom)
    /-- A call of a function value. -/
    | apply (f : Atom) (args : List Atom)
    /-- The one operation of F1a, `@log.write(line)`: a sink, recorded in the trace. -/
    | op (line : Atom)
    | construct (record : String) (fields : List Atom)
    | field (value : Atom) (index : Nat)
    | list (items : List Atom)
    | none
    | some (value : Atom)
    | lambda (params : List (String × Ty)) (body : Expr)

  /-- A body in A-normal form. -/
  inductive Expr where
    | atom (a : Atom)
    | letR (name : String) (ty : Ty) (value : Rhs) (body : Expr)
    | letE (name : String) (ty : Ty) (value : Expr) (body : Expr)
    | ite (cond : Atom) (thenE elseE : Expr)
    /-- `when o is None -> n is Some(x) -> s`. -/
    | caseOption (scrutinee : Atom) (none : Expr) (x : String) (some : Expr)
    /-- `fail` with a failure, identified by its code. -/
    | fail (code : Nat)
    /-- `attempt e`: the value or the failure of `e`, as a result. -/
    | attempt (body : Expr)
    /-- `e if error x -> h`: `h` runs when `e` fails, with its code bound to `x`. -/
    | handle (body : Expr) (error : String) (handler : Expr)
end

/-- A function of the program. -/
structure Fun where
  name : String
  params : List (String × Ty)
  result : Ty
  fails : Bool
  body : Expr

/-- A record kind of the program: its field types, in order. -/
structure Record where
  name : String
  fields : List Ty

/-- A program: its record kinds and its functions. -/
structure Program where
  records : List Record
  funs : List Fun

end Una.F1
