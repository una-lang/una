/-
The labeled typing of F1: the judgment `Γ; pc ⊢ e : τ ! ε` of `spec/10 §1` over the labeled
types of `spec/07 §7`, edition 0.2, with the one capability's operation and the host operation
as the effects, and failures with the label of what decides them (`spec/07 §7` rule 4). T6
(M3 and M4) is stated over this judgment; T1 holds for it because it refines the structural
typing (`Erase.lean`).

Where the rules below read the specification in a way it does not spell out, the comment of
the rule says so; those readings are the input of M3, whose proof confirms or changes them.
-/
import UnaFormal.F1.Typing

namespace Una.F1

/-- A labeled typing context, innermost first. -/
abbrev LCtx := List (String × LTy)

def lctxLookup (Γ : LCtx) (x : String) : Option LTy :=
  match Γ with
  | [] => none
  | (y, t) :: rest => if x = y then some t else lctxLookup rest x

/-- The structure of a labeled context. -/
def LCtx.erase (Γ : LCtx) : Ctx := Γ.map (fun q => (q.1, q.2.erase))

namespace Program

variable (p : Program)

/-- The static order of labels, relative to the program's `⊤` (`spec/07 §7` rule 8). -/
def le (a b : LExpr) : Prop := LExpr.le p.top a b

/-- `ℓ*` of a type, the labels its records declare included. -/
def deep (t : LTy) : LExpr := t.deep p.recordDeep

/-- Labeled instantiation of the type parameters `σ` covers. -/
def lsub (σ : List (String × LTy)) (t : LTy) : LTy := t.lsubst p.recordDeep σ

end Program

mutual
  /-- The flow of a value of one type into a declared type (`spec/07 §7` rule 2, S-Refine and
  S-Fun): the same structure, every position labeled below the declared label at that position,
  covariantly through lists, options and results and function results, contravariantly through
  function parameters. A function type flows into one that allows at least its effects and
  failures. A `Unit` position carries no information and never fails a flow. -/
  inductive Flow (top : Label) : LTy → LTy → Prop where
    | int {l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.int l₁) (.int l₂)
    | bool {l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.bool l₁) (.bool l₂)
    | text {l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.text l₁) (.text l₂)
    | unit {l₁ l₂} : Flow top (.unit l₁) (.unit l₂)
    | list {t₁ t₂ l₁ l₂} : Flow top t₁ t₂ → LExpr.le top l₁ l₂ → Flow top (.list t₁ l₁) (.list t₂ l₂)
    | option {t₁ t₂ l₁ l₂} :
        Flow top t₁ t₂ → LExpr.le top l₁ l₂ → Flow top (.option t₁ l₁) (.option t₂ l₂)
    | result {t₁ t₂ l₁ l₂} :
        Flow top t₁ t₂ → LExpr.le top l₁ l₂ → Flow top (.result t₁ l₁) (.result t₂ l₂)
    | record {r l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.record r l₁) (.record r l₂)
    | fn {ps₁ ps₂ r₁ r₂ o₁ o₂ g₁ g₂ f₁ f₂ l₁ l₂} :
        Flows top ps₂ ps₁ → Flow top r₁ r₂ → (o₁ = true → o₂ = true) →
        (g₁ = true → g₂ = true) → (f₁ = true → f₂ = true) → LExpr.le top l₁ l₂ →
        Flow top (.fn ps₁ r₁ o₁ g₁ f₁ l₁) (.fn ps₂ r₂ o₂ g₂ f₂ l₂)
    | var {a l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.var a l₁) (.var a l₂)

  inductive Flows (top : Label) : List LTy → List LTy → Prop where
    | nil : Flows top [] []
    | cons {t₁ t₂ ts₁ ts₂} : Flow top t₁ t₂ → Flows top ts₁ ts₂ → Flows top (t₁ :: ts₁) (t₂ :: ts₂)
end

mutual
  /-- A function type with effects or failures is second-class: no type argument is one or
  holds one (`spec/10 §5`, generic instantiation rule 1, U3016). -/
  def LTy.firstClass : LTy → Bool
    | .list t _ | .option t _ | .result t _ => t.firstClass
    | .fn ps r o g f _ => !o && !g && !f && LTy.firstClasses ps && r.firstClass
    | _ => true

  def LTy.firstClasses : List LTy → Bool
    | [] => true
    | t :: ts => t.firstClass && LTy.firstClasses ts
end

/-- What a typing says a code may do: perform the operation, perform the host operation, and
fail, with the label of what decides whether it fails. -/
structure Eff where
  op : Bool
  get : Bool
  fails : Option LExpr

namespace Eff

def none : Eff := ⟨false, false, Option.none⟩

def union (a b : Eff) : Eff :=
  ⟨a.op || b.op, a.get || b.get,
    match a.fails, b.fails with
    | some l₁, some l₂ => some (l₁.join l₂)
    | some l, Option.none | Option.none, some l => some l
    | Option.none, Option.none => Option.none⟩

/-- The label of what decides whether the code fails, `⊥` when it cannot. -/
def failLabel (e : Eff) : LExpr := e.fails.getD LExpr.bot

/-- The effects of a call of a function with these `uses` and `fails`. A function's failure is
decided at its public program counter (T-Fail, T-Fun), so its label is `⊥`. -/
def ofFn (o g f : Bool) : Eff := ⟨o, g, if f then some LExpr.bot else Option.none⟩

end Eff

/-- `pc ⊑ bound(f)` (`spec/07 §3` rule 4, `spec/10 §5`): the meet of the `accepts` labels of
the effects, met with `⊥` when it can fail, written as one premise per effect. Effects and
failures need a program counter without symbolic labels (`spec/07 §7` rule 8). -/
def Program.Bound (p : Program) (pc : LExpr) (o g f : Bool) : Prop :=
  (o = true → p.le pc (.ofLabel p.acceptsOp) ∧ pc.SymFree) ∧
  (g = true → p.le pc (.ofLabel p.acceptsGet) ∧ pc.SymFree) ∧
  (f = true → p.le pc LExpr.bot ∧ pc.SymFree)

/-- A scalar type with a label: the result of a primitive. -/
def ofScalar (t : Ty) (l : LExpr) : LTy :=
  match t with
  | .bool => .bool l
  | _ => .int l

/-- The label of a primitive's result (`spec/07 §7` rule 3): `.len` takes the top label of its
operand, the others the join of every label of their operands. -/
def Program.primLabel (p : Program) (pr : Prim) (ts : List LTy) : LExpr :=
  match pr, ts with
  | .len, [t] => t.top
  | _, ts => LExpr.joinAll (ts.map p.deep)

inductive LAtomTy : LCtx → Atom → LTy → Prop where
  | var {Γ x t} : lctxLookup Γ x = some t → LAtomTy Γ (.var x) t
  | int {Γ n} : LAtomTy Γ (.int n) (.int .bot)
  | bool {Γ b} : LAtomTy Γ (.bool b) (.bool .bot)
  | text {Γ s} : LAtomTy Γ (.text s) (.text .bot)
  | unit {Γ} : LAtomTy Γ .unit (.unit .bot)

inductive LAtomsTy : LCtx → List Atom → List LTy → Prop where
  | nil {Γ} : LAtomsTy Γ [] []
  | cons {Γ a as t ts} : LAtomTy Γ a t → LAtomsTy Γ as ts → LAtomsTy Γ (a :: as) (t :: ts)

mutual
  /-- **`ℓ_T` only in place** (`spec/07 §7` rule 8): a plain symbolic label `ℓ_T` stands for the
  labels a value of `T` was passed with, so it labels only positions of type `T`. Everywhere
  else the labels have no plain symbol; `ℓ*_T` may appear anywhere. The reading of `ℓ_T` at a
  position of type `T` as the type argument's own labels needs it: an `ℓ_T` joined onto such a
  position from elsewhere, such as a condition of type `Bool` labeled `ℓ_T`, would be dropped
  by instantiation, and a branch on it would leak. The checker places plain symbols only on type
  parameters (`crates/una-check/src/types/ty.rs`), and the model's rules keep the invariant. -/
  def LTy.InPlace : LTy → Prop
    | .var a l => ∀ x ∈ l.plain, x = a
    | .int l | .bool l | .text l | .unit l | .record _ l => l.plain = []
    | .list t l | .option t l | .result t l => t.InPlace ∧ l.plain = []
    | .fn ps r _ _ _ l => LTy.InPlaceAll ps ∧ r.InPlace ∧ l.plain = []

  def LTy.InPlaceAll : List LTy → Prop
    | [] => True
    | t :: ts => t.InPlace ∧ LTy.InPlaceAll ts
end

mutual
  /-- A ground type: no type parameter and no symbolic label. F1 has one level of generic
  functions (`docs/plan/formal-track.md` §3), so the type arguments of a call are ground. -/
  def LTy.Ground : LTy → Prop
    | .var _ _ => False
    | .int l | .bool l | .text l | .unit l | .record _ l => l.SymFree
    | .list t l | .option t l | .result t l => t.Ground ∧ l.SymFree
    | .fn ps r _ _ _ l => LTy.GroundAll ps ∧ r.Ground ∧ l.SymFree

  def LTy.GroundAll : List LTy → Prop
    | [] => True
    | t :: ts => t.Ground ∧ LTy.GroundAll ts
end

/-- The symbols of a label are among `xs`. -/
def LExpr.SymsIn (xs : List String) (e : LExpr) : Prop := ∀ x ∈ e.plain ++ e.star, x ∈ xs

mutual
  /-- A type of a signature mentions only the function's own type parameters `xs`, as types
  and in its labels. -/
  def LTy.ScopedIn (xs : List String) : LTy → Prop
    | .var a l => a ∈ xs ∧ l.SymsIn xs
    | .int l | .bool l | .text l | .unit l | .record _ l => l.SymsIn xs
    | .list t l | .option t l | .result t l => t.ScopedIn xs ∧ l.SymsIn xs
    | .fn ps r _ _ _ l => LTy.ScopedInAll xs ps ∧ r.ScopedIn xs ∧ l.SymsIn xs

  def LTy.ScopedInAll (xs : List String) : List LTy → Prop
    | [] => True
    | t :: ts => t.ScopedIn xs ∧ LTy.ScopedInAll xs ts
end

/-- A symbolic decision `ℓ*_T` for each name. -/
def decision (d : List String) : LExpr := ⟨Label.bot, [], d⟩

/-- The decisions the arguments of a call carry at their top: the `ℓ*_T` that a comparison of
`T` values, a condition on them or another deep join put there (`spec/07 §7` rules 3 and 8). -/
def decisionsOf (ts : List LTy) : List String := (ts.flatMap (fun t => t.top.star)).eraseDups

/-- A type with equality (`Eq`, `spec/20`): no function type at any position, a type parameter
only when it is among `E`, those the enclosing function constrains by `Eq` (U2026), and a record
when `recEq` says so of its kind. -/
def LTy.eqIn (E : List String) (recEq : String → Bool) : LTy → Bool
  | .list t _ | .option t _ | .result t _ => t.eqIn E recEq
  | .fn _ _ _ _ _ _ => false
  | .var a _ => E.contains a
  | .record r _ => recEq r
  | _ => true

/-- Whether the fields of record kind `r` have equality, records within records included, up to
`n` levels; the program's records are not generic, so no type parameter is constrained. -/
def Program.recordEq (p : Program) : Nat → String → Bool
  | 0, _ => false
  | n + 1, r =>
    match p.record r with
    | some rec => rec.fields.all (LTy.eqIn [] (p.recordEq n))
    | none => false

/-- A type with equality in a function whose `Eq` parameters are `E`. -/
def Program.eqIn (p : Program) (E : List String) (t : LTy) : Bool :=
  t.eqIn E (p.recordEq (p.records.length + 1))

/-- A function is the `via` function of a policy, whose body no label rule checks
(`spec/07 §5` rule 3), and which only its policy may call (`spec/07 §7` rule 5, U4005). -/
def Program.IsVia (p : Program) (f : String) : Prop := ∃ po ∈ p.policies, po.via = f

/-- The type arguments of the parameters a function constrains by `Eq` have equality. -/
def Fun.EqArgs (p : Program) (E : List String) (fn : Fun) (targs : List LTy) : Prop :=
  ∀ q ∈ fn.tparams.zip targs, q.1 ∈ fn.eqParams → p.eqIn E q.2 = true

mutual
  inductive LRhsTy (p : Program) (E : List String) : LCtx → LExpr → Rhs → LTy → Eff → Prop where
    | atom {Γ pc a t} : LAtomTy Γ a t → LRhsTy p E Γ pc (.atom a) t .none
    /-- T-Arith and its kin. -/
    | prim {Γ pc pr args ts t} :
        LAtomsTy Γ args ts → PrimTy pr (ts.map LTy.erase) t →
        LRhsTy p E Γ pc (.prim pr args) (ofScalar t (p.primLabel pr ts)) .none
    /-- T-Call with generic instantiation: the arguments flow into the parameters instantiated,
    and the result is the declared result instantiated, whose symbolic labels are the result
    summary of ADR 0067 at each position. A `via` function is not callable but through its
    policy (U4005). -/
    | call {Γ pc f targs args fn ts} :
        p.find f = some fn → ¬ p.IsVia f → targs.length = fn.tparams.length →
        LTy.firstClasses targs = true → fn.EqArgs p E targs → LTy.GroundAll targs →
        LAtomsTy Γ args ts →
        Flows p.top ts (fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2)) →
        p.Bound pc fn.op fn.get fn.fails →
        LRhsTy p E Γ pc (.call f targs args) (p.lsub (fn.tparams.zip targs) fn.result)
          (.ofFn fn.op fn.get fn.fails)
    /-- T-App. An argument that carries decisions `ℓ*_T` its parameter's type does not accept
    raises the call's result and program counter instead, as the value's label does
    (`spec/07 §7` rule 8, "decisions given to functions"). The decisions are those the
    arguments carry at their top, which only a deep join, such as a comparison of `T` values,
    puts there (`decisionsOf`). -/
    | apply {Γ pc f args ps r o g fl lf ts} :
        LAtomTy Γ f (.fn ps r o g fl lf) → LAtomsTy Γ args ts →
        Flows p.top ts (ps.map (LTy.raise (decision (decisionsOf ts)))) →
        p.Bound (pc.join (lf.join (decision (decisionsOf ts)))) o g fl →
        LRhsTy p E Γ pc (.apply f args) (r.raise (lf.join (decision (decisionsOf ts))))
          (.ofFn o g fl)
    /-- T-Op: the argument is a sink. F1's arguments are texts, for which `ℓ⁺`, the labels at
    covariant positions of rule 4, is every label, so the model writes `deep`; an argument with
    a function type would need `ℓ⁺`. -/
    | op {Γ pc line t} :
        LAtomTy Γ line t → t.erase.Scalar →
        p.le (pc.join (p.deep t)) (.ofLabel p.acceptsOp) → pc.SymFree →
        LRhsTy p E Γ pc (.op line) (.unit .bot) ⟨true, false, Option.none⟩
    /-- The host operation: a sink whose failure its argument decides, and which the program
    counter decides too, since it decides whether the operation runs. -/
    | get {Γ pc url t} :
        LAtomTy Γ url t → t.erase = .text →
        p.le (pc.join (p.deep t)) (.ofLabel p.acceptsGet) → pc.SymFree →
        LRhsTy p E Γ pc (.get url) (.text (.ofLabel p.getResult))
          ⟨false, true, some (pc.join (p.deep t))⟩
    | construct {Γ pc record fields rec ts} :
        p.record record = some rec → LAtomsTy Γ fields ts → Flows p.top ts rec.fields →
        LRhsTy p E Γ pc (.construct record fields) (.record record .bot) .none
    /-- A field takes its declared type, with its top label raised by the record's. -/
    | field {Γ pc value record index rec l t} :
        LAtomTy Γ value (.record record l) → p.record record = some rec →
        rec.fields[index]? = some t → LRhsTy p E Γ pc (.field value index) (t.raise l) .none
    /-- A list literal: each element flows into the element type. -/
    | list {Γ pc items ts t} :
        LAtomsTy Γ items ts → (∀ u ∈ ts, Flow p.top u t) → t.InPlace →
        LRhsTy p E Γ pc (.list items) (.list t .bot) .none
    | none {Γ pc t} : t.InPlace → LRhsTy p E Γ pc .none (.option t .bot) .none
    | some {Γ pc a t} : LAtomTy Γ a t → LRhsTy p E Γ pc (.some a) (.option t .bot) .none
    /-- A lambda's body is checked at the public program counter (ADR 0031 rule 4), and, as a
    function's, its failures must be decided there. -/
    | lambda {Γ pc params body t ε} :
        LExprTy p E (params ++ Γ) .bot body t ε → p.le ε.failLabel .bot →
        (∀ q ∈ params, q.2.InPlace) →
        LRhsTy p E Γ pc (.lambda params body)
          (.fn (params.map Prod.snd) t ε.op ε.get ε.fails.isSome .bot) .none
    | funRef {Γ pc f targs fn} :
        p.find f = some fn → ¬ p.IsVia f → targs.length = fn.tparams.length →
        LTy.firstClasses targs = true → fn.EqArgs p E targs → LTy.GroundAll targs →
        LRhsTy p E Γ pc (.funRef f targs)
          (.fn (fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2))
            (p.lsub (fn.tparams.zip targs) fn.result) fn.op fn.get fn.fails .bot) .none
    /-- T-Declassify. -/
    | declassify {Γ pc pol a po fn t} :
        p.policy pol = some po → p.find po.via = some fn → LAtomTy Γ a t → Flow p.top t po.src →
        p.le pc po.dst.top →
        LRhsTy p E Γ pc (.declassify pol a) po.dst (.ofFn fn.op fn.get fn.fails)
    /-- `map` (`spec/14 §1`, ADR 0051): the function runs under the top labels of `xs` and of
    itself (`spec/07 §7` rule 9); the result's top label is theirs, and each element is the
    function's result raised by its label (T-App). -/
    | map {Γ pc xs f te lx tp tr o g fl lf} :
        LAtomTy Γ xs (.list te lx) → LAtomTy Γ f (.fn [tp] tr o g fl lf) → Flow p.top te tp →
        p.Bound ((pc.join lx).join lf) o g fl →
        LRhsTy p E Γ pc (.map xs f) (.list (tr.raise lf) (lx.join lf)) (.ofFn o g fl)
    /-- `filter`: which elements remain depends on the predicate's result, whose label joins the
    result's top label (edition 0.2, ADR 0067). -/
    | filter {Γ pc xs keep te lx tp lb o g fl lf} :
        LAtomTy Γ xs (.list te lx) → LAtomTy Γ keep (.fn [tp] (.bool lb) o g fl lf) →
        Flow p.top te tp → p.Bound ((pc.join lx).join lf) o g fl →
        LRhsTy p E Γ pc (.filter xs keep) (.list te ((lx.join lf).join lb)) (.ofFn o g fl)
    /-- `first_where` calls its predicate only up to the first accepted element, so the
    elements and the predicate's results decide which calls happen (`spec/07 §7` rule 9). -/
    | firstWhere {Γ pc xs keep te lx tp lb o g fl lf} :
        LAtomTy Γ xs (.list te lx) → LAtomTy Γ keep (.fn [tp] (.bool lb) o g fl lf) →
        Flow p.top te tp →
        p.Bound ((((pc.join lx).join lf).join (p.deep te)).join lb) o g fl →
        LRhsTy p E Γ pc (.firstWhere xs keep) (.option te ((lx.join lf).join lb)) (.ofFn o g fl)
    /-- `fold`: the accumulator type, raised by the top labels of `xs` and of the function. -/
    | fold {Γ pc xs init f te lx ti ta tp tr o g fl lf} :
        LAtomTy Γ xs (.list te lx) → LAtomTy Γ init ti → LAtomTy Γ f (.fn [ta, tp] tr o g fl lf) →
        Flow p.top te tp → Flow p.top ti ta → Flow p.top tr ta →
        p.Bound ((pc.join lx).join lf) o g fl →
        LRhsTy p E Γ pc (.fold xs init f) (ta.raise (lx.join lf)) (.ofFn o g fl)
    /-- The partial primitive: its result and its failure carry every label of its input. -/
    | parse {Γ pc s t} :
        LAtomTy Γ s t → t.erase = .text →
        LRhsTy p E Γ pc (.parse s) (.int (p.deep t)) ⟨false, false, some (pc.join (p.deep t))⟩
    /-- `==` (`spec/07 §7` rule 3): both operands flow into one type with equality, and the
    result carries every label of both. Over values of `T`, that is `ℓ*_T`, the decision of
    rule 8. -/
    | eq {Γ pc a b t₁ t₂ t} :
        LAtomTy Γ a t₁ → LAtomTy Γ b t₂ → Flow p.top t₁ t → Flow p.top t₂ t → p.eqIn E t = true →
        LRhsTy p E Γ pc (.eq a b) (.bool ((p.deep t₁).join (p.deep t₂))) .none

  inductive LExprTy (p : Program) (E : List String) : LCtx → LExpr → Expr → LTy → Eff → Prop where
    | atom {Γ pc a t} : LAtomTy Γ a t → LExprTy p E Γ pc (.atom a) t .none
    /-- T-Let. The body runs only when the bound code did not fail, so it runs under the label
    of what decides that: the model's reading of U4002 for `if error pass` (`spec/07 §7`
    rule 4), whose failure reaches the function's boundary, where it must be public. -/
    | letR {Γ pc x ann value body t s ε₁ ε₂} :
        LRhsTy p E Γ pc value t ε₁ → Flow p.top t ann →
        LExprTy p E ((x, ann) :: Γ) (pc.join ε₁.failLabel) body s ε₂ → ann.InPlace →
        LExprTy p E Γ pc (.letR x ann value body) s (ε₁.union ε₂)
    | letE {Γ pc x ann value body t s ε₁ ε₂} :
        LExprTy p E Γ pc value t ε₁ → Flow p.top t ann →
        LExprTy p E ((x, ann) :: Γ) (pc.join ε₁.failLabel) body s ε₂ → ann.InPlace →
        LExprTy p E Γ pc (.letE x ann value body) s (ε₁.union ε₂)
    /-- T-If: the branches run under the condition's label, flow into the joined type, and the
    value is raised by it. -/
    | ite {Γ pc cond thenE elseE lc t₁ t₂ s ε₁ ε₂} :
        LAtomTy Γ cond (.bool lc) → LExprTy p E Γ (pc.join lc) thenE t₁ ε₁ →
        LExprTy p E Γ (pc.join lc) elseE t₂ ε₂ → Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.ite cond thenE elseE) (s.raise lc) (ε₁.union ε₂)
    /-- T-Case on an option: the pattern binding takes the element type raised by the
    scrutinee's top label. -/
    | caseOption {Γ pc scrutinee noneE x someE te ls t₁ t₂ s ε₁ ε₂} :
        LAtomTy Γ scrutinee (.option te ls) → LExprTy p E Γ (pc.join ls) noneE t₁ ε₁ →
        LExprTy p E ((x, te.raise ls) :: Γ) (pc.join ls) someE t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.caseOption scrutinee noneE x someE) (s.raise ls) (ε₁.union ε₂)
    | caseResult {Γ pc scrutinee x okE y errE te ls t₁ t₂ s ε₁ ε₂} :
        LAtomTy Γ scrutinee (.result te ls) →
        LExprTy p E ((x, te.raise ls) :: Γ) (pc.join ls) okE t₁ ε₁ →
        LExprTy p E ((y, .int ls) :: Γ) (pc.join ls) errE t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.caseResult scrutinee x okE y errE) (s.raise ls) (ε₁.union ε₂)
    /-- T-Fail: only at the public program counter (U4002). -/
    | fail {Γ pc code s} :
        p.le pc .bot → pc.SymFree → s.InPlace → LExprTy p E Γ pc (.fail code) s ⟨false, false, some .bot⟩
    /-- T-Attempt: the result carries the label of what decides the failure. -/
    | attempt {Γ pc body t ε} :
        LExprTy p E Γ pc body t ε →
        LExprTy p E Γ pc (.attempt body) (.result t ε.failLabel) ⟨ε.op, ε.get, Option.none⟩
    /-- T-Handle: the handler runs under the label of what decides the failure, which binds its
    code, and the value is raised by it, since it decides which branch gives the value. -/
    | handle {Γ pc body x handler t₁ t₂ s ε₁ ε₂} :
        LExprTy p E Γ pc body t₁ ε₁ →
        LExprTy p E ((x, .int ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) handler t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.handle body x handler) (s.raise ε₁.failLabel)
          ⟨ε₁.op || ε₂.op, ε₁.get || ε₂.get, ε₂.fails⟩
end

/-- T-Fun: the body, at the public program counter, flows into the declared result, which may
carry the symbolic labels of the type parameters (`spec/07 §7` rule 8); its effects are
declared, and its failures are decided at the public program counter (U4002). -/
def Program.FunOk (p : Program) (fn : Fun) : Prop :=
  ∃ t ε, LExprTy p fn.eqParams fn.params .bot fn.body t ε ∧ Flow p.top t fn.result ∧
    (ε.op = true → fn.op = true) ∧ (ε.get = true → fn.get = true) ∧
    (ε.fails.isSome = true → fn.fails = true) ∧ p.le ε.failLabel .bot

/-- The categories of a label are among `cats`. -/
def LExpr.CatsIn (cats : List String) (e : LExpr) : Prop := ∀ c ∈ e.base.cats, c ∈ cats

mutual
  /-- Every label of a type uses only categories among `cats`. -/
  def LTy.CatsIn (cats : List String) : LTy → Prop
    | .int l | .bool l | .text l | .unit l | .record _ l | .var _ l => l.CatsIn cats
    | .list t l | .option t l | .result t l => t.CatsIn cats ∧ l.CatsIn cats
    | .fn ps r _ _ _ l => LTy.CatsInAll cats ps ∧ r.CatsIn cats ∧ l.CatsIn cats

  def LTy.CatsInAll (cats : List String) : List LTy → Prop
    | [] => True
    | t :: ts => t.CatsIn cats ∧ LTy.CatsInAll cats ts
end

/-- Every label the program declares uses only its declared categories (U4007, `spec/07 §7`
rule 7): the `accepts` labels, the response label, and the types of functions, records and
policies. The annotations inside bodies are the checker's, which reports U4007 at each; the model
needs the declared ones, against which `⊤` is defined. -/
def Program.CatsDeclared (p : Program) : Prop :=
  (∀ c ∈ p.acceptsOp.cats, c ∈ p.categories) ∧ (∀ c ∈ p.acceptsGet.cats, c ∈ p.categories) ∧
  (∀ c ∈ p.getResult.cats, c ∈ p.categories) ∧
  (∀ fn ∈ p.funs, (∀ q ∈ fn.params, q.2.CatsIn p.categories) ∧ fn.result.CatsIn p.categories) ∧
  (∀ rec ∈ p.records, ∀ t ∈ rec.fields, t.CatsIn p.categories) ∧
  (∀ po ∈ p.policies, po.src.CatsIn p.categories ∧ po.dst.CatsIn p.categories)

/-- A program the labeled typing accepts: every function but a `via` function passes T-Fun, a
`via` function has its structural type, the declared deep labels of records bound their
fields', and records and policies are as structural well-formedness requires. -/
structure Program.LWellFormed (p : Program) : Prop where
  funs : ∀ fn ∈ p.funs, p.FunOk fn ∨
    (p.IsVia fn.name ∧ ExprTy p (bindTys fn.params (fn.paramTys [])) fn.body fn.result.erase)
  records : ∀ rec ∈ p.records, ∀ t ∈ rec.fieldTys, t.Closed
  recordDeep : ∀ rec ∈ p.records, ∀ t ∈ rec.fields, p.le (p.deep t) (.ofLabel rec.deep)
  policies : ∀ po ∈ p.policies, po.src.erase.Closed ∧ po.dst.erase.Closed ∧
    ∃ fn, p.find po.via = some fn ∧ fn.paramTys [] = [po.src.erase] ∧
      fn.result.erase = po.dst.erase
  categories : p.CatsDeclared
  /-- Records declare ground field types. -/
  recordsGround : ∀ rec ∈ p.records, ∀ t ∈ rec.fields, t.Ground
  /-- A function constrains by `Eq` only its own type parameters. -/
  eqScoped : ∀ fn ∈ p.funs, ∀ a ∈ fn.eqParams, a ∈ fn.tparams
  /-- A signature mentions only its own type parameters. -/
  sigsScoped : ∀ fn ∈ p.funs, (∀ q ∈ fn.params, q.2.ScopedIn fn.tparams) ∧
    fn.result.ScopedIn fn.tparams
  /-- Signatures and declared types keep `ℓ_T` in place. -/
  inPlace : (∀ fn ∈ p.funs, (∀ q ∈ fn.params, q.2.InPlace) ∧ fn.result.InPlace) ∧
    (∀ rec ∈ p.records, ∀ t ∈ rec.fields, t.InPlace) ∧
    (∀ po ∈ p.policies, po.src.InPlace ∧ po.dst.InPlace)

end Una.F1
