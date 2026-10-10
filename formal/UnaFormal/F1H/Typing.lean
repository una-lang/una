/-
The structural typing of F1 (`spec/10` without labels, effects and failure kinds), which T1 needs,
and the typing of values and environments. The labeled typing of `spec/07` and `spec/10`
(`LTyping.lean`) refines it: a program it accepts, this one accepts (`Erase.lean`).

The rules read types from the data, as a checker that has inferred them would, and not from the
annotations of the syntax, which the labeled typing checks: so a typing is closed under
substitution of type arguments (`Lemmas.lean`, `ExprTy.subst`), which the call of a generic
function needs. A context lists the types of the variables in scope in the order the
evaluator binds them, so that the innermost binding comes first in both.

Failure is a legitimate end of a run (T1: a value or a failure), so this typing does not track
which code can fail; the labeled one does.
-/
import UnaFormal.F1H.Eval

namespace Una.F1H

open Una.F1 (Label LExpr)

/-- A typing context: the types of the variables in scope, innermost first. -/
abbrev Ctx := List (String × Ty)

/-- The type of variable `x` in `ctx`, the innermost binding. -/
def ctxLookup (ctx : Ctx) (x : String) : Option Ty :=
  match ctx with
  | [] => none
  | (y, t) :: rest => if x = y then some t else ctxLookup rest x

/-- A context with a substitution applied to every type. -/
def Ctx.subst (σ : List (String × Ty)) (ctx : Ctx) : Ctx := ctx.map (fun q => (q.1, q.2.subst σ))

/-- The parameters named with the types `ts`, in front. -/
def bindTys {α : Type} (params : List (String × α)) (ts : List Ty) : Ctx :=
  (params.map Prod.fst).zip ts

/-- The structural types of a function's parameters under a substitution. -/
def Fun.paramTys (fn : Fun) (σ : List (String × Ty)) : List Ty :=
  fn.params.map (fun q => q.2.erase.subst σ)

/-- The structural types of the fields of a record kind. -/
def Record.fieldTys (rec : Record) : List Ty := rec.fields.map LTy.erase

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
  | sum : PrimTy .sum [.list .int] .int

mutual
  inductive RhsTy (p : Program) : Ctx → Rhs → Ty → Prop where
    | atom {ctx a t} : AtomTy ctx a t → RhsTy p ctx (.atom a) t
    | prim {ctx pr args ts t} : AtomsTy ctx args ts → PrimTy pr ts t → RhsTy p ctx (.prim pr args) t
    | call {ctx f targs args fn σ} :
        p.find f = some fn → AtomsTy ctx args (fn.paramTys σ) →
        RhsTy p ctx (.call f targs args) (fn.result.erase.subst σ)
    | apply {ctx f args ts r} :
        AtomTy ctx f (.fn ts r) → AtomsTy ctx args ts → RhsTy p ctx (.apply f args) r
    | op {ctx line t} : AtomTy ctx line t → t.Scalar → RhsTy p ctx (.op line) .unit
    | read {ctx path} : AtomTy ctx path .text → RhsTy p ctx (.read path) .text
    | write {ctx path content} :
        AtomTy ctx path .text → AtomTy ctx content .text → RhsTy p ctx (.write path content) .unit
    | req {ctx m args ts} : AtomsTy ctx args ts → (∀ t ∈ ts, t = .text) → RhsTy p ctx (.req m args) .text
    | construct {ctx record fields rec} :
        p.record record = some rec → AtomsTy ctx fields rec.fieldTys →
        RhsTy p ctx (.construct record fields) (.record record)
    | field {ctx value record index rec t} :
        AtomTy ctx value (.record record) → p.record record = some rec →
        rec.fieldTys[index]? = some t → RhsTy p ctx (.field value index) t
    | list {ctx items t} :
        AtomsTy ctx items (items.map (fun _ => t)) → RhsTy p ctx (.list items) (.list t)
    | none {ctx t} : RhsTy p ctx .none (.option t)
    | some {ctx a t} : AtomTy ctx a t → RhsTy p ctx (.some a) (.option t)
    | lambda {ctx params body ts r} :
        ts.length = params.length → ExprTy p (bindTys params ts ++ ctx) body r →
        RhsTy p ctx (.lambda params body) (.fn ts r)
    | funRef {ctx f targs fn σ} :
        p.find f = some fn →
        RhsTy p ctx (.funRef f targs) (.fn (fn.paramTys σ) (fn.result.erase.subst σ))
    | declassify {ctx pol a po} :
        p.policy pol = some po → AtomTy ctx a po.src.erase →
        RhsTy p ctx (.declassify pol a) po.dst.erase
    | map {ctx xs f a b} :
        AtomTy ctx xs (.list a) → AtomTy ctx f (.fn [a] b) → RhsTy p ctx (.map xs f) (.list b)
    | filter {ctx xs keep a} :
        AtomTy ctx xs (.list a) → AtomTy ctx keep (.fn [a] .bool) →
        RhsTy p ctx (.filter xs keep) (.list a)
    | firstWhere {ctx xs keep a} :
        AtomTy ctx xs (.list a) → AtomTy ctx keep (.fn [a] .bool) →
        RhsTy p ctx (.firstWhere xs keep) (.option a)
    | fold {ctx xs init f a b} :
        AtomTy ctx xs (.list a) → AtomTy ctx init b → AtomTy ctx f (.fn [b, a] b) →
        RhsTy p ctx (.fold xs init f) b
    | parse {ctx s} : AtomTy ctx s .text → RhsTy p ctx (.parse s) .int
    | eq {ctx a b t} : AtomTy ctx a t → AtomTy ctx b t → RhsTy p ctx (.eq a b) .bool

  inductive ExprTy (p : Program) : Ctx → Expr → Ty → Prop where
    | atom {ctx a t} : AtomTy ctx a t → ExprTy p ctx (.atom a) t
    | letR {ctx x ann value body t s} :
        RhsTy p ctx value t → ExprTy p ((x, t) :: ctx) body s →
        ExprTy p ctx (.letR x ann value body) s
    | letE {ctx x ann value body t s} :
        ExprTy p ctx value t → ExprTy p ((x, t) :: ctx) body s →
        ExprTy p ctx (.letE x ann value body) s
    | ite {ctx cond thenE elseE s} :
        AtomTy ctx cond .bool → ExprTy p ctx thenE s → ExprTy p ctx elseE s →
        ExprTy p ctx (.ite cond thenE elseE) s
    | caseOption {ctx scrutinee noneE x someE t s} :
        AtomTy ctx scrutinee (.option t) → ExprTy p ctx noneE s →
        ExprTy p ((x, t) :: ctx) someE s →
        ExprTy p ctx (.caseOption scrutinee noneE x someE) s
    | caseResult {ctx scrutinee x okE y errE t s} :
        AtomTy ctx scrutinee (.result t) → ExprTy p ((x, t) :: ctx) okE s →
        ExprTy p ((y, .int) :: ctx) errE s →
        ExprTy p ctx (.caseResult scrutinee x okE y errE) s
    | fail {ctx code s} : ExprTy p ctx (.fail code) s
    | attempt {ctx body t} : ExprTy p ctx body t → ExprTy p ctx (.attempt body) (.result t)
    | handle {ctx body x handler s} :
        ExprTy p ctx body s → ExprTy p ((x, .int) :: ctx) handler s →
        ExprTy p ctx (.handle body x handler) s
end

/-- A program is well formed when each function's body has its declared result type under its
parameters, with its type parameters abstract; the fields of records and the types of
policies have no type parameters; and the `via` function of each policy exists and has the
policy's structure. -/
structure Program.WellFormed (p : Program) : Prop where
  funs : ∀ fn ∈ p.funs, ExprTy p (bindTys fn.params (fn.paramTys [])) fn.body fn.result.erase
  records : ∀ rec ∈ p.records, ∀ t ∈ rec.fieldTys, t.Closed
  policies : ∀ po ∈ p.policies, po.src.erase.Closed ∧ po.dst.erase.Closed ∧
    ∃ fn, p.find po.via = some fn ∧ fn.paramTys [] = [po.src.erase] ∧
      fn.result.erase = po.dst.erase

mutual
  /-- The typing of values (for T1). A closure has a function type when its body has the
  result type under its parameters and the types of the environment it captured. -/
  inductive ValTy (p : Program) : Val → Ty → Prop where
    | int {n} : ValTy p (.int n) .int
    | bool {b} : ValTy p (.bool b) .bool
    | text {s} : ValTy p (.text s) .text
    | unit : ValTy p .unit .unit
    | list {items t} : (∀ v ∈ items, ValTy p v t) → ValTy p (.list items) (.list t)
    | none {t} : ValTy p .none (.option t)
    | some {v t} : ValTy p v t → ValTy p (.some v) (.option t)
    | ok {v t} : ValTy p v t → ValTy p (.ok v) (.result t)
    | err {code t} : ValTy p (.err code) (.result t)
    | record {record fields rec} :
        p.record record = some rec → ValsTy p fields rec.fieldTys →
        ValTy p (.record record fields) (.record record)
    | closure {params body env ctx ts r} :
        EnvTy p env ctx → ts.length = params.length →
        ExprTy p (bindTys params ts ++ ctx) body r →
        ValTy p (.closure params body env) (.fn ts r)

  inductive ValsTy (p : Program) : List Val → List Ty → Prop where
    | nil : ValsTy p [] []
    | cons {v vs t ts} : ValTy p v t → ValsTy p vs ts → ValsTy p (v :: vs) (t :: ts)

  /-- An environment has a context's types, binding by binding, in the same order. -/
  inductive EnvTy (p : Program) : Env → Ctx → Prop where
    | nil : EnvTy p [] []
    | cons {x v t env ctx} : ValTy p v t → EnvTy p env ctx → EnvTy p ((x, v) :: env) ((x, t) :: ctx)
end

end Una.F1H
