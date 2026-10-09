/-
The typing of F1a (`spec/10`): of atoms, the right-hand sides of `let`, bodies, and, for T1,
of values and environments. A context lists the types of the variables in scope in the order
the evaluator binds them, so that the innermost binding comes first in both.

Failure is a legitimate end of a run (T1: a value or a failure), so the typing of F1a does not
track which code can fail; T3 adds it.
-/
import UnaFormal.F1.Eval

namespace Una.F1

/-- A typing context: the types of the variables in scope, innermost first. -/
abbrev Ctx := List (String × Ty)

/-- The type of variable `x` in `ctx`, the innermost binding. -/
def ctxLookup (ctx : Ctx) (x : String) : Option Ty :=
  match ctx with
  | [] => none
  | (y, t) :: rest => if x = y then some t else ctxLookup rest x

/-- The field types of the record kind `r` of the program. -/
def Program.fields (p : Program) (r : String) : Option (List Ty) :=
  (p.records.find? (fun rec => rec.name = r)).map Record.fields

inductive AtomTy : Ctx → Atom → Ty → Prop where
  | var {ctx x t} : ctxLookup ctx x = some t → AtomTy ctx (.var x) t
  | int {ctx n} : AtomTy ctx (.int n) .int
  | bool {ctx b} : AtomTy ctx (.bool b) .bool
  | text {ctx s} : AtomTy ctx (.text s) .text
  | unit {ctx} : AtomTy ctx .unit .unit

inductive AtomsTy : Ctx → List Atom → List Ty → Prop where
  | nil {ctx} : AtomsTy ctx [] []
  | cons {ctx a as t ts} : AtomTy ctx a t → AtomsTy ctx as ts → AtomsTy ctx (a :: as) (t :: ts)

/-- The argument and result types of each primitive. -/
inductive PrimTy : Prim → List Ty → Ty → Prop where
  | add : PrimTy .add [.int, .int] .int
  | sub : PrimTy .sub [.int, .int] .int
  | lt : PrimTy .lt [.int, .int] .bool
  | eqInt : PrimTy .eqInt [.int, .int] .bool
  | not : PrimTy .not [.bool] .bool
  | lenList {t} : PrimTy .len [.list t] .int
  | lenText : PrimTy .len [.text] .int

mutual
  inductive RhsTy (p : Program) : Ctx → Rhs → Ty → Prop where
    | atom {ctx a t} : AtomTy ctx a t → RhsTy p ctx (.atom a) t
    | prim {ctx pr args ts t} : AtomsTy ctx args ts → PrimTy pr ts t → RhsTy p ctx (.prim pr args) t
    | call {ctx f args fn} :
        p.find f = some fn → AtomsTy ctx args (fn.params.map Prod.snd) →
        RhsTy p ctx (.call f args) fn.result
    | apply {ctx f args ts r fails} :
        AtomTy ctx f (.fn ts r fails) → AtomsTy ctx args ts → RhsTy p ctx (.apply f args) r
    | op {ctx line} : AtomTy ctx line .text → RhsTy p ctx (.op line) .unit
    | construct {ctx record fields ts} :
        p.fields record = some ts → AtomsTy ctx fields ts →
        RhsTy p ctx (.construct record fields) (.record record)
    | field {ctx value record index ts t} :
        AtomTy ctx value (.record record) → p.fields record = some ts → ts[index]? = some t →
        RhsTy p ctx (.field value index) t
    | list {ctx items t} : AtomsTy ctx items (items.map (fun _ => t)) → RhsTy p ctx (.list items) (.list t)
    | none {ctx t} : RhsTy p ctx .none (.option t)
    | some {ctx a t} : AtomTy ctx a t → RhsTy p ctx (.some a) (.option t)
    | lambda {ctx params body r fails} :
        ExprTy p (params ++ ctx) body r →
        RhsTy p ctx (.lambda params body) (.fn (params.map Prod.snd) r fails)

  inductive ExprTy (p : Program) : Ctx → Expr → Ty → Prop where
    | atom {ctx a t} : AtomTy ctx a t → ExprTy p ctx (.atom a) t
    | letR {ctx x t value body s} :
        RhsTy p ctx value t → ExprTy p ((x, t) :: ctx) body s → ExprTy p ctx (.letR x t value body) s
    | letE {ctx x t value body s} :
        ExprTy p ctx value t → ExprTy p ((x, t) :: ctx) body s → ExprTy p ctx (.letE x t value body) s
    | ite {ctx cond thenE elseE s} :
        AtomTy ctx cond .bool → ExprTy p ctx thenE s → ExprTy p ctx elseE s →
        ExprTy p ctx (.ite cond thenE elseE) s
    | caseOption {ctx scrutinee noneE x someE t s} :
        AtomTy ctx scrutinee (.option t) → ExprTy p ctx noneE s → ExprTy p ((x, t) :: ctx) someE s →
        ExprTy p ctx (.caseOption scrutinee noneE x someE) s
    | fail {ctx code s} : ExprTy p ctx (.fail code) s
    | attempt {ctx body t} : ExprTy p ctx body t → ExprTy p ctx (.attempt body) (.result t)
    | handle {ctx body x handler s} :
        ExprTy p ctx body s → ExprTy p ((x, .int) :: ctx) handler s →
        ExprTy p ctx (.handle body x handler) s
end

/-- A program is well formed when each function's body has its declared result type under its
parameters. -/
def Program.WellFormed (p : Program) : Prop :=
  ∀ fn ∈ p.funs, ExprTy p fn.params fn.body fn.result

mutual
  /-- The typing of values (for T1). A closure has a function type when its body has the
  result type under its parameters and the types of the environment it captured. -/
  inductive ValTy (p : Program) : Val → Ty → Prop where
    | int {n} : ValTy p (.int n) .int
    | bool {b} : ValTy p (.bool b) .bool
    | text {s} : ValTy p (.text s) .text
    | unit : ValTy p .unit .unit
    | list {items t} : ValsTy p items (items.map (fun _ => t)) → ValTy p (.list items) (.list t)
    | none {t} : ValTy p .none (.option t)
    | some {v t} : ValTy p v t → ValTy p (.some v) (.option t)
    | ok {v t} : ValTy p v t → ValTy p (.ok v) (.result t)
    | err {code t} : ValTy p (.err code) (.result t)
    | record {record fields ts} :
        p.fields record = some ts → ValsTy p fields ts → ValTy p (.record record fields) (.record record)
    | closure {params body env ctx r fails} :
        EnvTy p env ctx → ExprTy p (params ++ ctx) body r →
        ValTy p (.closure params body env) (.fn (params.map Prod.snd) r fails)

  inductive ValsTy (p : Program) : List Val → List Ty → Prop where
    | nil : ValsTy p [] []
    | cons {v vs t ts} : ValTy p v t → ValsTy p vs ts → ValsTy p (v :: vs) (t :: ts)

  /-- An environment has a context's types, binding by binding, in the same order. -/
  inductive EnvTy (p : Program) : Env → Ctx → Prop where
    | nil : EnvTy p [] []
    | cons {x v t env ctx} : ValTy p v t → EnvTy p env ctx → EnvTy p ((x, v) :: env) ((x, t) :: ctx)
end

end Una.F1
