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

/-- The symbols of a label are among `xs`. -/
def LExpr.SymsIn (xs : List String) (e : LExpr) : Prop := ∀ x ∈ e.plain ++ e.star, x ∈ xs

/-- The categories of a label are among `cats`. -/
def LExpr.CatsIn (cats : List String) (e : LExpr) : Prop := ∀ c ∈ e.base.cats, c ∈ cats

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
    | result {t₁ t₂ e₁ e₂ l₁ l₂} :
        Flow top t₁ t₂ → Flow top e₁ e₂ → LExpr.le top l₁ l₂ →
        Flow top (.result t₁ e₁ l₁) (.result t₂ e₂ l₂)
    | record {r l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.record r l₁) (.record r l₂)
    | fn {ps₁ ps₂ r₁ r₂ o₁ o₂ g₁ g₂ f₁ f₂ l₁ l₂} :
        Flows top ps₂ ps₁ → Flow top r₁ r₂ → (o₁ = true → o₂ = true) →
        HostFx.Le g₁ g₂ → (f₁ = true → f₂ = true) → LExpr.le top l₁ l₂ →
        Flow top (.fn ps₁ r₁ o₁ g₁ f₁ l₁) (.fn ps₂ r₂ o₂ g₂ f₂ l₂)
    | var {a l₁ l₂} : LExpr.le top l₁ l₂ → Flow top (.var a l₁) (.var a l₂)
    | tuple {ts₁ ts₂ l₁ l₂} :
        Flows top ts₁ ts₂ → LExpr.le top l₁ l₂ → Flow top (.tuple ts₁ l₁) (.tuple ts₂ l₂)

  inductive Flows (top : Label) : List LTy → List LTy → Prop where
    | nil : Flows top [] []
    | cons {t₁ t₂ ts₁ ts₂} : Flow top t₁ t₂ → Flows top ts₁ ts₂ → Flows top (t₁ :: ts₁) (t₂ :: ts₂)
end

mutual
  /-- A function type with effects or failures is second-class: no type argument is one or
  holds one (`spec/10 §5`, generic instantiation rule 1, U3016). -/
  def LTy.firstClass : LTy → Bool
    | .list t _ | .option t _ => t.firstClass
    | .result t e _ => t.firstClass && e.firstClass
    | .fn ps r o g f _ => !o && g.isNone && !f && LTy.firstClasses ps && r.firstClass
    | .tuple ts _ => LTy.firstClasses ts
    | _ => true

  def LTy.firstClasses : List LTy → Bool
    | [] => true
    | t :: ts => t.firstClass && LTy.firstClasses ts
end

/-- What a typing says a code may do: perform the operation, perform the host operation, and
fail, with the label of what decides whether it fails. -/
structure Eff where
  op : Bool
  host : HostFx
  fails : Option LExpr
  /-- The policies whose release the code may perform (option B1). -/
  rels : List String
  /-- Whether it may fail with a code, a failure without a payload (#512). -/
  codes : Bool
  /-- The payload kinds it may fail with (#512): `attempt` and a handler of a code catch none,
  and those of a payload kind catch that kind only. -/
  raises : List String

namespace Eff

def none : Eff := ⟨false, HostFx.none, Option.none, [], false, []⟩

def union (a b : Eff) : Eff :=
  ⟨a.op || b.op, a.host.or b.host,
    match a.fails, b.fails with
    | some l₁, some l₂ => some (l₁.join l₂)
    | some l, Option.none | Option.none, some l => some l
    | Option.none, Option.none => Option.none, a.rels ++ b.rels, a.codes || b.codes,
    a.raises ++ b.raises⟩

/-- The label of what decides whether the code fails, `⊥` when it cannot. -/
def failLabel (e : Eff) : LExpr := e.fails.getD LExpr.bot

/-- The effects of a call of a function with these `uses` and `fails`. A function's failure is
decided at its public program counter (T-Fail, T-Fun), so its label is `⊥`. -/
def ofFn (o : Bool) (g : HostFx) (f : Bool) : Eff :=
  ⟨o, g, if f then some LExpr.bot else Option.none, [], f, []⟩

/-- The effects of a call of a function that may also release through the policies `rs`. -/
def ofFnR (o : Bool) (g : HostFx) (f : Bool) (rs : List String) : Eff := { ofFn o g f with rels := rs }

/-- The effects of a call of `fn` (#512): those of `ofFnR`, with the payload kinds it raises; it
fails with a code when it fails and raises none. -/
def ofCall (fn : Fun) : Eff :=
  { ofFnR fn.op fn.host fn.fails fn.rels with
    codes := fn.fails && fn.raises.isEmpty, raises := fn.raises }

/-- The effects of a call of the policy `pol`, whose `via` function has these `uses` and `fails`
and whose `to` has the top label `dst` (T-Declassify): the release, and a failure the `via`
function decides from the `from` value, so labeled where the release is visible, `pc ⊔ dst`
(#453 Phase B, ADR 0090). -/
def ofPolicy (o : Bool) (g : HostFx) (f : Bool) (pc dst : LExpr) (pol : String) : Eff :=
  ⟨o, g, if f then some (pc.join dst) else Option.none, [pol], f, []⟩

end Eff

/-- `pc ⊑ bound(f)` (`spec/07 §3` rule 4, `spec/10 §5`): the meet of the `accepts` labels of
the effects, met with `⊥` when it can fail, written as one premise per effect. Effects and
failures need a program counter without symbolic labels (`spec/07 §7` rule 8). -/
def Program.Bound (p : Program) (pc : LExpr) (o : Bool) (g : HostFx) (f : Bool) : Prop :=
  (o = true → p.le pc (.ofLabel p.acceptsOp) ∧ pc.SymFree) ∧
  (g.rd = true → p.le pc (.ofLabel p.acceptsRead) ∧ pc.SymFree) ∧
  -- A write runs where it is entered: the program counter of the call is below the file-state
  -- label (ADR 0085 decision 1, its declarative form).
  (g.wr = true → p.le pc (.ofLabel p.acceptsWrite) ∧
    (p.oldHost = false → p.le pc (.ofLabel p.fileState)) ∧ pc.SymFree) ∧
  (g.rq = true → p.le pc (.ofLabel p.acceptsReq) ∧ pc.SymFree) ∧
  (f = true → p.le pc LExpr.bot ∧ pc.SymFree)

/-- The label of a `Files` failure decided at `l`: with the file-state label (ADR 0085), or, under
the old rules, by `l` alone. -/
def Program.fsFail (p : Program) (l : LExpr) : LExpr :=
  if p.oldHost then l else l.join (.ofLabel p.fileState)

/-- The label of a request's failure: every argument, and the headers' `internal` values (ADR 0085
decision 7), or, under the old rules, the URL alone (the first argument). -/
def Program.reqFail (p : Program) (pc : LExpr) (ts : List LTy) : LExpr :=
  if p.oldHost then pc.join (match ts with | t :: _ => p.deep t | [] => LExpr.bot)
  else (pc.join (LExpr.joinAll (ts.map p.deep))).join (.ofLabel p.reqFloor)

/-- A call of a function that may release through the policies `rs` (option B1): each release
happens at the label of its policy's `to`, so the program counter is below it, as at the
declassification itself (`spec/07 §5` rule 2), and has no symbolic label. -/
def Program.RelBound (p : Program) (pc : LExpr) (rs : List String) : Prop :=
  ∀ pol ∈ rs, ∃ po, p.policy pol = some po ∧ p.le pc po.dst.top ∧ pc.SymFree

/-- A scalar type with a label: the result of a primitive. -/
def ofScalar (t : Ty) (l : LExpr) : LTy :=
  match t with
  | .bool => .bool l
  | .text => .text l
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
    | .list t l | .option t l => t.InPlace ∧ l.plain = []
    | .result t e l => t.InPlace ∧ e.InPlace ∧ l.plain = []
    | .fn ps r _ _ _ l => LTy.InPlaceAll ps ∧ r.InPlace ∧ l.plain = []
    | .tuple ts l => LTy.InPlaceAll ts ∧ l.plain = []

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
    | .list t l | .option t l => t.Ground ∧ l.SymFree
    | .result t e l => t.Ground ∧ e.Ground ∧ l.SymFree
    | .fn ps r _ _ _ l => LTy.GroundAll ps ∧ r.Ground ∧ l.SymFree
    | .tuple ts l => LTy.GroundAll ts ∧ l.SymFree

  def LTy.GroundAll : List LTy → Prop
    | [] => True
    | t :: ts => t.Ground ∧ LTy.GroundAll ts
end

-- (`LExpr`'s predicate is F1's, shared.)

mutual
  /-- A type of a signature mentions only the function's own type parameters `xs`, as types
  and in its labels. -/
  def LTy.ScopedIn (xs : List String) : LTy → Prop
    | .var a l => a ∈ xs ∧ l.SymsIn xs
    | .int l | .bool l | .text l | .unit l | .record _ l => l.SymsIn xs
    | .list t l | .option t l => t.ScopedIn xs ∧ l.SymsIn xs
    | .result t e l => t.ScopedIn xs ∧ e.ScopedIn xs ∧ l.SymsIn xs
    | .fn ps r _ _ _ l => LTy.ScopedInAll xs ps ∧ r.ScopedIn xs ∧ l.SymsIn xs
    | .tuple ts l => LTy.ScopedInAll xs ts ∧ l.SymsIn xs

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
  | .list t _ | .option t _ => t.eqIn E recEq
  | .result t e _ => t.eqIn E recEq && e.eqIn E recEq
  | .fn _ _ _ _ _ _ | .tuple _ _ => false
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
        p.Bound pc fn.op fn.host fn.fails → p.RelBound pc fn.rels →
        LRhsTy p E Γ pc (.call f targs args) (p.lsub (fn.tparams.zip targs) fn.result)
          (.ofCall fn)
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
        LRhsTy p E Γ pc (.op line) (.unit .bot) ⟨true, HostFx.none, Option.none, [], false, []⟩
    /-- `Files.read`: its path is a sink, and its failure is decided by the path, by the program
    counter, which decides whether it runs, and by the file system's shape, which the package's
    writes decide: the file-state label (ADR 0085 decision 2). -/
    | read {Γ pc path t} :
        LAtomTy Γ path t → t.erase = .text →
        p.le (pc.join (p.deep t)) (.ofLabel p.acceptsRead) → pc.SymFree →
        LRhsTy p E Γ pc (.read path) (.text (.ofLabel p.readResult))
          ⟨false, { rd := true }, some (p.fsFail (pc.join (p.deep t))), [], true, []⟩
    /-- `Files.write`: its path and content are sinks; where it lands, and whether it runs, are
    below the file-state label (decision 1); its failure is labeled as a read's. -/
    | write {Γ pc path content t c} :
        LAtomTy Γ path t → t.erase = .text → LAtomTy Γ content c → c.erase = .text →
        p.le ((pc.join (p.deep t)).join (p.deep c)) (.ofLabel p.acceptsWrite) →
        (p.oldHost = false → p.le (pc.join (p.deep t)) (.ofLabel p.fileState)) → pc.SymFree →
        LRhsTy p E Γ pc (.write path content) (.unit .bot)
          ⟨false, { wr := true }, some (p.fsFail (pc.join (p.deep t))), [], true, []⟩
    /-- A request of `Http`: every argument is a sink, and every argument, with the headers'
    `internal` values, decides its failure (S-15, ADR 0085 decision 7). -/
    | req {Γ pc m args ts} :
        LAtomsTy Γ args ts → httpParams m = some (ts.map LTy.erase) → p.HttpRecords →
        p.le (pc.join (LExpr.joinAll (ts.map p.deep))) (.ofLabel p.acceptsReq) → pc.SymFree →
        LRhsTy p E Γ pc (.req m args) (.record "prelude.Response" (.ofLabel p.reqResult))
          ⟨false, { rq := true }, some (p.reqFail pc ts), [], true, []⟩
    | construct {Γ pc record fields rec ts} :
        p.record record = some rec → LAtomsTy Γ fields ts → Flows p.top ts rec.fields →
        LRhsTy p E Γ pc (.construct record fields) (.record record .bot) .none
    /-- T-Variant (#512): each argument flows into its field's declared type, and which variant
    is built is decided by the program counter, which the rules of branches raise the value by. -/
    | variant {Γ pc kind ctor fields sm fs ts} :
        p.sum kind = some sm → sm.variants[ctor]? = some fs → LAtomsTy Γ fields ts → Flows p.top ts fs →
        LRhsTy p E Γ pc (.variant kind ctor fields) (.record kind .bot) .none
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
    /-- `Ok(a)` and `Err(a)` (#508): the other side holds no value, so any type in place will
    do, and the flow into the declared type decides it. Which constructor is built is decided
    by the program counter, which the rules of branches raise the value by. -/
    | mkOk {Γ pc a t e} : LAtomTy Γ a t → e.InPlace → LRhsTy p E Γ pc (.mkOk a) (.result t e .bot) .none
    | mkErr {Γ pc a t e} : LAtomTy Γ a e → t.InPlace → LRhsTy p E Γ pc (.mkErr a) (.result t e .bot) .none
    /-- A lambda's body is checked at the public program counter (ADR 0031 rule 4), and, as a
    function's, its failures must be decided there. -/
    | lambda {Γ pc params body t ε} :
        LExprTy p E (params ++ Γ) .bot body t ε → p.le ε.failLabel .bot → ε.rels = [] →
        ε.raises = [] → (∀ q ∈ params, q.2.InPlace) →
        LRhsTy p E Γ pc (.lambda params body)
          (.fn (params.map Prod.snd) t ε.op ε.host ε.fails.isSome .bot) .none
    | funRef {Γ pc f targs fn} :
        p.find f = some fn → ¬ p.IsVia f → targs.length = fn.tparams.length →
        LTy.firstClasses targs = true → fn.EqArgs p E targs → LTy.GroundAll targs → fn.rels = [] →
        fn.raises = [] →
        LRhsTy p E Γ pc (.funRef f targs)
          (.fn (fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2))
            (p.lsub (fn.tparams.zip targs) fn.result) fn.op fn.host fn.fails .bot) .none
    /-- T-Declassify (`spec/07 §5` rule 2): the argument flows into `from`, and the program
    counter is below the label of `to`. The call releases its value through the policy (option
    B1), an effect a function that declassifies declares (`Fun.rels`). Its failure is the `via`
    function's, labeled `pc ⊔ ℓ(to)` (#453 Phase B, ADR 0090). -/
    | declassify {Γ pc pol a po fn t} :
        p.policy pol = some po → p.find po.via = some fn → LAtomTy Γ a t → Flow p.top t po.src →
        p.le pc po.dst.top → pc.SymFree →
        LRhsTy p E Γ pc (.declassify pol a) po.dst (.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol)
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
        LRhsTy p E Γ pc (.parse s) (.int (p.deep t))
          ⟨false, HostFx.none, some (pc.join (p.deep t)), [], true, []⟩
    /-- T-Checked (`spec/10 §7`, #453): the operand is a public scalar, under a public program
    counter (U4002 otherwise), so whether the predicate holds is public; the value keeps its type,
    and the failure is labeled as `parse`'s, which the premise makes `⊥`. -/
    | checked {Γ pc a t pred code} :
        LAtomTy Γ a t → t.erase.Scalar → p.le (pc.join (p.deep t)) .bot → pc.SymFree →
        LRhsTy p E Γ pc (.checked a pred code) t
          ⟨false, HostFx.none, some (pc.join (p.deep t)), [], true, []⟩
    /-- `==` (`spec/07 §7` rule 3): both operands flow into one type with equality, and the
    result carries every label of both. Over values of `T`, that is `ℓ*_T`, the decision of
    rule 8. -/
    | eq {Γ pc a b t₁ t₂ t} :
        LAtomTy Γ a t₁ → LAtomTy Γ b t₂ → Flow p.top t₁ t → Flow p.top t₂ t → p.eqIn E t = true →
        LRhsTy p E Γ pc (.eq a b) (.bool ((p.deep t₁).join (p.deep t₂))) .none
    /-- A tuple of Core (`spec/17 §4`): the components' types. -/
    | tuple {Γ pc items ts} : LAtomsTy Γ items ts → LRhsTy p E Γ pc (.tuple items) (.tuple ts .bot) .none
    /-- `to_text` (`spec/10 §5`, `spec/07 §7` rule 3): the text carries every label of the value,
    at a type with equality, where related values the observer fully sees are equal (as `==`). -/
    | toText {Γ pc a t} :
        LAtomTy Γ a t → p.eqIn E t = true → LRhsTy p E Γ pc (.toText a) (.text (p.deep t)) .none
    /-- `Env.get` (`spec/14 §3`): its name is `public`, its `accepts`; whether the variable exists
    is `public`, and its value `internal`. The environment is the world's: its values may differ
    between two runs, and the variables that exist may not (#487). -/
    | envGet {Γ pc name t} :
        LAtomTy Γ name t → t.erase = .text → p.le (p.deep t) .bot →
        LRhsTy p E Γ pc (.envGet name) (.option (.text (.ofLabel ⟨.internal, []⟩)) .bot) .none
    /-- `append`: the item flows into the element type; the list keeps its top label, its length's. -/
    | append {Γ pc xs item te lx t} :
        LAtomTy Γ xs (.list te lx) → LAtomTy Γ item t → Flow p.top t te →
        LRhsTy p E Γ pc (.append xs item) (.list te lx) .none
    /-- `set`: the index is `public` (its parameter's label in the prelude), and the item flows
    into the element type. Whether the index is in range is decided by the index and the length;
    out of it the run aborts (N-79), which is termination. -/
    | set {Γ pc xs index item te lx li t} :
        LAtomTy Γ xs (.list te lx) → LAtomTy Γ index (.int li) → p.le li .bot →
        LAtomTy Γ item t → Flow p.top t te →
        LRhsTy p E Γ pc (.set xs index item) (.tuple [.list te lx, .unit .bot] .bot) .none

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
    /-- T-If at a tuple type, Core's join of the bindings an `if` updates (`spec/17 §4`): a tuple
    carries no label of its own, so the condition's label goes into every component, which the
    premise asks to be at least it, as U4001 asks of a rebind under a raised program counter. A
    `Unit` component is exempt only because any two units are related (`V.unit`, `Flow.unit`). -/
    | iteT {Γ pc cond thenE elseE lc t₁ t₂ ts ε₁ ε₂} :
        LAtomTy Γ cond (.bool lc) → LExprTy p E Γ (pc.join lc) thenE t₁ ε₁ →
        LExprTy p E Γ (pc.join lc) elseE t₂ ε₂ → Flow p.top t₁ (.tuple ts .bot) →
        Flow p.top t₂ (.tuple ts .bot) → LTy.InPlaceAll ts → Flows p.top (ts.map (LTy.raise lc)) ts →
        LExprTy p E Γ pc (.ite cond thenE elseE) (.tuple ts .bot) (ε₁.union ε₂)
    /-- T-Case on an option: the pattern binding takes the element type raised by the
    scrutinee's top label. -/
    | caseOption {Γ pc scrutinee noneE x someE te ls t₁ t₂ s ε₁ ε₂} :
        LAtomTy Γ scrutinee (.option te ls) → LExprTy p E Γ (pc.join ls) noneE t₁ ε₁ →
        LExprTy p E ((x, te.raise ls) :: Γ) (pc.join ls) someE t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.caseOption scrutinee noneE x someE) (s.raise ls) (ε₁.union ε₂)
    /-- T-Case on an option at a tuple type: as `iteT`. -/
    | caseOptionT {Γ pc scrutinee noneE x someE te ls t₁ t₂ ts ε₁ ε₂} :
        LAtomTy Γ scrutinee (.option te ls) → LExprTy p E Γ (pc.join ls) noneE t₁ ε₁ →
        LExprTy p E ((x, te.raise ls) :: Γ) (pc.join ls) someE t₂ ε₂ →
        Flow p.top t₁ (.tuple ts .bot) → Flow p.top t₂ (.tuple ts .bot) → LTy.InPlaceAll ts →
        Flows p.top (ts.map (LTy.raise ls)) ts →
        LExprTy p E Γ pc (.caseOption scrutinee noneE x someE) (.tuple ts .bot) (ε₁.union ε₂)
    | caseResult {Γ pc scrutinee x okE y errE te ee ls t₁ t₂ s ε₁ ε₂} :
        LAtomTy Γ scrutinee (.result te ee ls) →
        LExprTy p E ((x, te.raise ls) :: Γ) (pc.join ls) okE t₁ ε₁ →
        LExprTy p E ((y, ee.raise ls) :: Γ) (pc.join ls) errE t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.caseResult scrutinee x okE y errE) (s.raise ls) (ε₁.union ε₂)
    /-- T-Case on a result at a tuple type: as `iteT`. -/
    | caseResultT {Γ pc scrutinee x okE y errE te ee ls t₁ t₂ ts ε₁ ε₂} :
        LAtomTy Γ scrutinee (.result te ee ls) →
        LExprTy p E ((x, te.raise ls) :: Γ) (pc.join ls) okE t₁ ε₁ →
        LExprTy p E ((y, ee.raise ls) :: Γ) (pc.join ls) errE t₂ ε₂ →
        Flow p.top t₁ (.tuple ts .bot) → Flow p.top t₂ (.tuple ts .bot) → LTy.InPlaceAll ts →
        Flows p.top (ts.map (LTy.raise ls)) ts →
        LExprTy p E Γ pc (.caseResult scrutinee x okE y errE) (.tuple ts .bot) (ε₁.union ε₂)
    /-- T-Fail: only at the public program counter (U4002). -/
    | fail {Γ pc code s} :
        p.le pc .bot → pc.SymFree → s.InPlace →
        LExprTy p E Γ pc (.fail code) s ⟨false, HostFx.none, some .bot, [], true, []⟩
    /-- T-Fail with a payload (#512): only at the public program counter, and each argument flows
    into its field's declared type. -/
    | raise {Γ pc kind code args sm fs ts s} :
        p.sum kind = some sm → sm.variants[code]? = some fs → LAtomsTy Γ args ts → Flows p.top ts fs →
        p.le pc .bot → pc.SymFree → s.InPlace →
        LExprTy p E Γ pc (.raise kind code args) s ⟨false, HostFx.none, some .bot, [], false, [kind]⟩
    /-- T-Case on a sum (#512), one arm against the rest: both run under the scrutinee's top
    label, which decides which runs, the bindings take their fields' declared types raised by it,
    and the value is raised by it. A chain of them is the `when` of several arms: each later arm
    is under every earlier test's label. -/
    | caseSum {Γ pc scrutinee kind ctor binders thenE elseE ls sm fs t₁ t₂ s ε₁ ε₂} :
        LAtomTy Γ scrutinee (.record kind ls) → p.sum kind = some sm → sm.variants[ctor]? = some fs →
        fs.length = binders.length →
        LExprTy p E (binders.zip (fs.map (LTy.raise ls)) ++ Γ) (pc.join ls) thenE t₁ ε₁ →
        LExprTy p E Γ (pc.join ls) elseE t₂ ε₂ → Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.caseSum scrutinee ctor binders thenE elseE) (s.raise ls) (ε₁.union ε₂)
    /-- The `else` of an exhaustive `when` on a sum (#512): no effect, at any type. A run that
    reached it aborts, which T6, termination-insensitive, does not observe. -/
    | unreachable {Γ pc s} : s.InPlace → LExprTy p E Γ pc .unreachable s .none
    /-- T-Case on a sum at a tuple type: as `iteT`. -/
    | caseSumT {Γ pc scrutinee kind ctor binders thenE elseE ls sm fs t₁ t₂ ts ε₁ ε₂} :
        LAtomTy Γ scrutinee (.record kind ls) → p.sum kind = some sm → sm.variants[ctor]? = some fs →
        fs.length = binders.length →
        LExprTy p E (binders.zip (fs.map (LTy.raise ls)) ++ Γ) (pc.join ls) thenE t₁ ε₁ →
        LExprTy p E Γ (pc.join ls) elseE t₂ ε₂ →
        Flow p.top t₁ (.tuple ts .bot) → Flow p.top t₂ (.tuple ts .bot) → LTy.InPlaceAll ts →
        Flows p.top (ts.map (LTy.raise ls)) ts →
        LExprTy p E Γ pc (.caseSum scrutinee ctor binders thenE elseE) (.tuple ts .bot) (ε₁.union ε₂)
    /-- T-Attempt: the result carries the label of what decides the failure, and its error is
    the failure's code, which that label decides too. -/
    | attempt {Γ pc body t ε} :
        LExprTy p E Γ pc body t ε → ε.raises = [] →
        LExprTy p E Γ pc (.attempt body) (.result t (.int .bot) ε.failLabel)
          ⟨ε.op, ε.host, Option.none, ε.rels, false, []⟩
    /-- T-Attempt of a payload kind (#512): the body fails with `kind` only, and the error is the
    failure's value, whose variant and fields that label decides. -/
    | attemptK {Γ pc kind body t ε} :
        LExprTy p E Γ pc body t ε → ε.codes = false → (∀ k ∈ ε.raises, k = kind) →
        LExprTy p E Γ pc (.attemptK kind body) (.result t (.record kind .bot) ε.failLabel)
          ⟨ε.op, ε.host, Option.none, ε.rels, false, []⟩
    /-- T-Handle: the handler runs under the label of what decides the failure, which binds its
    code, and the value is raised by it, since it decides which branch gives the value. -/
    | handle {Γ pc body x handler t₁ t₂ s ε₁ ε₂} :
        LExprTy p E Γ pc body t₁ ε₁ → ε₁.raises = [] →
        LExprTy p E ((x, .int ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) handler t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.handle body x handler) (s.raise ε₁.failLabel)
          ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
    /-- T-Handle of a payload kind (#512): the body fails with `kind` only, and the handler binds
    the failure's value at the label of what decides it (`spec/10`), so every field read from it
    is raised by that label. -/
    | handleK {Γ pc kind body x handler t₁ t₂ s ε₁ ε₂} :
        LExprTy p E Γ pc body t₁ ε₁ → ε₁.codes = false → (∀ k ∈ ε₁.raises, k = kind) →
        LExprTy p E ((x, .record kind ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) handler t₂ ε₂ →
        Flow p.top t₁ s → Flow p.top t₂ s → s.InPlace →
        LExprTy p E Γ pc (.handleK kind body x handler) (s.raise ε₁.failLabel)
          ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
    /-- T-Handle at a tuple type: as `iteT`, with what decides the failure. -/
    | handleT {Γ pc body x handler t₁ t₂ ts ε₁ ε₂} :
        LExprTy p E Γ pc body t₁ ε₁ → ε₁.raises = [] →
        LExprTy p E ((x, .int ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) handler t₂ ε₂ →
        Flow p.top t₁ (.tuple ts .bot) → Flow p.top t₂ (.tuple ts .bot) → LTy.InPlaceAll ts →
        Flows p.top (ts.map (LTy.raise ε₁.failLabel)) ts →
        LExprTy p E Γ pc (.handle body x handler) (.tuple ts .bot)
          ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
    /-- T-Handle of a payload kind at a tuple type: as `iteT`, with what decides the failure. -/
    | handleKT {Γ pc kind body x handler t₁ t₂ ts ε₁ ε₂} :
        LExprTy p E Γ pc body t₁ ε₁ → ε₁.codes = false → (∀ k ∈ ε₁.raises, k = kind) →
        LExprTy p E ((x, .record kind ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) handler t₂ ε₂ →
        Flow p.top t₁ (.tuple ts .bot) → Flow p.top t₂ (.tuple ts .bot) → LTy.InPlaceAll ts →
        Flows p.top (ts.map (LTy.raise ε₁.failLabel)) ts →
        LExprTy p E Γ pc (.handleK kind body x handler) (.tuple ts .bot)
          ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
    /-- T-LetTuple: each component, raised by the tuple's top label, flows into its binder's
    declared type. -/
    | letTuple {Γ pc binders value body ts l s ε} :
        LAtomTy Γ value (.tuple ts l) → Flows p.top (ts.map (LTy.raise l)) (binders.map Prod.snd) →
        (∀ q ∈ binders, q.2.InPlace) → LExprTy p E (binders ++ Γ) pc body s ε →
        LExprTy p E Γ pc (.letTuple binders value body) s ε
    /-- T-Loop over a list (`spec/17 §6` rule 3, ADR 0075): the declared state types are an
    invariant. The initial values flow into them, and the body, run under the list's top label,
    which decides how many iterations run, maps the state at those types to a tuple that flows
    into them. The variable carries the elements' labels and the list's top label. The loop's
    value is the state, a tuple, which carries no label of its own: the list's top label goes
    into every component, which the premise asks to be at least it (M6b), as U4001 asks of a rebind
    under a raised program counter; a `Unit` component is exempt only because any two units are
    related. An iteration's failure decides
    whether the later iterations run, as a failure decides T-Let's continuation, so it is
    decided by no more than what decides whether the iteration runs (M6, a finding of the
    proof; `una check` gives U4002 to any failure a loop body passes on that is not public). -/
    | loopList {Γ pc xs x xty state init body te ls ts tb ε} :
        LAtomTy Γ xs (.list te ls) → Flow p.top (te.raise ls) xty → xty.InPlace →
        LAtomsTy Γ init ts → Flows p.top ts (state.map Prod.snd) → (∀ q ∈ state, q.2.InPlace) →
        LExprTy p E ((x, xty) :: (state ++ Γ)) (pc.join ls) body tb ε →
        Flow p.top tb (.tuple (state.map Prod.snd) .bot) → (p.oldLoop = false → p.le ε.failLabel (pc.join ls)) →
        Flows p.top ((state.map Prod.snd).map (LTy.raise ls)) (state.map Prod.snd) →
        LExprTy p E Γ pc (.loop (.list xs) x xty state init body) (.tuple (state.map Prod.snd) .bot) ε
    /-- T-Loop over a range: as over a list, with the bounds' labels, which decide the variable's
    values and how many iterations run. -/
    | loopRange {Γ pc low high inclusive x xty state init body l₁ l₂ ts tb ε} :
        LAtomTy Γ low (.int l₁) → LAtomTy Γ high (.int l₂) → Flow p.top (.int (l₁.join l₂)) xty →
        xty.InPlace →
        LAtomsTy Γ init ts → Flows p.top ts (state.map Prod.snd) → (∀ q ∈ state, q.2.InPlace) →
        LExprTy p E ((x, xty) :: (state ++ Γ)) (pc.join (l₁.join l₂)) body tb ε →
        Flow p.top tb (.tuple (state.map Prod.snd) .bot) → (p.oldLoop = false → p.le ε.failLabel (pc.join (l₁.join l₂))) →
        Flows p.top ((state.map Prod.snd).map (LTy.raise (l₁.join l₂))) (state.map Prod.snd) →
        LExprTy p E Γ pc (.loop (.range low high inclusive) x xty state init body)
          (.tuple (state.map Prod.snd) .bot) ε
end

/-- T-Fun: the body, at the public program counter, flows into the declared result, which may
carry the symbolic labels of the type parameters (`spec/07 §7` rule 8); its effects are
declared, and its failures are decided at the public program counter (U4002). -/
def Program.FunOk (p : Program) (fn : Fun) : Prop :=
  ∃ t ε, LExprTy p fn.eqParams fn.params .bot fn.body t ε ∧ Flow p.top t fn.result ∧
    (ε.op = true → fn.op = true) ∧ HostFx.Le ε.host fn.host ∧
    (ε.fails.isSome = true → fn.fails = true) ∧ p.le ε.failLabel .bot ∧ (∀ x ∈ ε.rels, x ∈ fn.rels) ∧
    (ε.codes = true → fn.raises = []) ∧ (∀ k ∈ ε.raises, k ∈ fn.raises)

-- (`LExpr`'s predicate is F1's, shared.)

mutual
  /-- Every label of a type uses only categories among `cats`. -/
  def LTy.CatsIn (cats : List String) : LTy → Prop
    | .int l | .bool l | .text l | .unit l | .record _ l | .var _ l => l.CatsIn cats
    | .list t l | .option t l => t.CatsIn cats ∧ l.CatsIn cats
    | .result t e l => t.CatsIn cats ∧ e.CatsIn cats ∧ l.CatsIn cats
    | .fn ps r _ _ _ l => LTy.CatsInAll cats ps ∧ r.CatsIn cats ∧ l.CatsIn cats
    | .tuple ts l => LTy.CatsInAll cats ts ∧ l.CatsIn cats

  def LTy.CatsInAll (cats : List String) : List LTy → Prop
    | [] => True
    | t :: ts => t.CatsIn cats ∧ LTy.CatsInAll cats ts
end

/-- Every label the program declares uses only its declared categories (U4007, `spec/07 §7`
rule 7): the `accepts` labels, the response label, and the types of functions, records and
policies. The annotations inside bodies are the checker's, which reports U4007 at each; the model
needs the declared ones, against which `⊤` is defined. -/
def Program.CatsDeclared (p : Program) : Prop :=
  (∀ c ∈ p.acceptsOp.cats, c ∈ p.categories) ∧ (∀ c ∈ p.acceptsRead.cats, c ∈ p.categories) ∧
  (∀ c ∈ p.acceptsWrite.cats, c ∈ p.categories) ∧ (∀ c ∈ p.acceptsReq.cats, c ∈ p.categories) ∧
  (∀ c ∈ p.readResult.cats, c ∈ p.categories) ∧ (∀ c ∈ p.reqResult.cats, c ∈ p.categories) ∧
  (∀ c ∈ p.reqFloor.cats, c ∈ p.categories) ∧ (∀ c ∈ p.fileState.cats, c ∈ p.categories) ∧
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
  /-- Sums (#512): their fields are ground and keep `ℓ_T` in place, use the declared
  categories, and the declared deep label bounds them; no name is both a record's and a sum's. -/
  sums : ∀ sm ∈ p.sums, ∀ fs ∈ sm.variants, ∀ t ∈ fs,
    t.Ground ∧ t.InPlace ∧ t.CatsIn p.categories ∧ p.le (p.deep t) (.ofLabel sm.deep) ∧
      t.erase.Closed
  disjoint : ∀ k, p.record k = none ∨ p.sum k = none
  /-- Signatures and declared types keep `ℓ_T` in place. -/
  inPlace : (∀ fn ∈ p.funs, (∀ q ∈ fn.params, q.2.InPlace) ∧ fn.result.InPlace) ∧
    (∀ rec ∈ p.records, ∀ t ∈ rec.fields, t.InPlace) ∧
    (∀ po ∈ p.policies, po.src.InPlace ∧ po.dst.InPlace)

end Una.F1
