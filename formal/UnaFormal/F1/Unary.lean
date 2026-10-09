/-
M3, the unary half of noninterference for the generic-free part of F1: a run of a body the
labeled typing accepts performs only the events its typing allows, each at a sink whose `accepts`
is above the program counter (T2 with the program counter, which confinement needs), fails only
with a failure whose label is above the program counter (T3 with labels), and ends with a value of
its labeled type.

The generic-free part has no type parameters (`fn.tparams = []`) and no policy, whose `via`
functions no label rule checks; declassification is M4's.
-/
import UnaFormal.F1.Subst
import UnaFormal.F1.Erase

namespace Una.F1

/-! ### The labeled typing of values -/

mutual
  /-- Values of a labeled type, for the unary half. Labels do not constrain a value by
  themselves; a closure's body is typed by the labeled rules, with the effects and failures its
  type allows. -/
  inductive LValTy (p : Program) : Val → LTy → Prop where
    | int {n l} : LValTy p (.int n) (.int l)
    | bool {b l} : LValTy p (.bool b) (.bool l)
    | text {s l} : LValTy p (.text s) (.text l)
    | unit {l} : LValTy p .unit (.unit l)
    | list {items t l} : LValsAll p items t → LValTy p (.list items) (.list t l)
    | none {t l} : LValTy p .none (.option t l)
    | some {v t l} : LValTy p v t → LValTy p (.some v) (.option t l)
    | ok {v t l} : LValTy p v t → LValTy p (.ok v) (.result t l)
    | err {code t l} : LValTy p (.err code) (.result t l)
    | record {record fields rec l} :
        p.record record = some rec → LValsTy p fields rec.fields →
        LValTy p (.record record fields) (.record record l)
    | closure {params body env Γ E t ε ps r o g f l σ} :
        (∀ q ∈ σ, q.2.Ground) →
        LEnvTy p env (Γ.lsub p.recordDeep σ) → LExprTy p E (params ++ Γ) .bot body t ε →
        p.le ε.failLabel .bot →
        Flows p.top ps ((params.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
        Flow p.top (t.lsubst p.recordDeep σ) r →
        (ε.op = true → o = true) → (ε.get = true → g = true) → (ε.fails.isSome = true → f = true) →
        LValTy p (.closure params body env) (.fn ps r o g f l)

  /-- Every value of a list has one type. -/
  inductive LValsAll (p : Program) : List Val → LTy → Prop where
    | nil {t} : LValsAll p [] t
    | cons {v vs t} : LValTy p v t → LValsAll p vs t → LValsAll p (v :: vs) t

  inductive LValsTy (p : Program) : List Val → List LTy → Prop where
    | nil : LValsTy p [] []
    | cons {v vs t ts} : LValTy p v t → LValsTy p vs ts → LValsTy p (v :: vs) (t :: ts)

  inductive LEnvTy (p : Program) : Env → LCtx → Prop where
    | nil : LEnvTy p [] []
    | cons {x v t env Γ} : LValTy p v t → LEnvTy p env Γ → LEnvTy p ((x, v) :: env) ((x, t) :: Γ)
end

variable {p : Program}

mutual
  /-- A value keeps its labeled type along a flow. -/
  theorem LValTy.flow : ∀ {v : Val} {t t' : LTy}, LValTy p v t → Flow p.top t t' → LValTy p v t'
    | _, _, _, .int, .int _ => .int
    | _, _, _, .bool, .bool _ => .bool
    | _, _, _, .text, .text _ => .text
    | _, _, _, .unit, .unit => .unit
    | _, _, _, .list h, .list f _ => .list (LValsAll.flow h f)
    | _, _, _, .none, .option _ _ => .none
    | _, _, _, .some h, .option f _ => .some (LValTy.flow h f)
    | _, _, _, .ok h, .result f _ => .ok (LValTy.flow h f)
    | _, _, _, .err, .result _ _ => .err
    | _, _, _, .record hr hfs, .record _ => .record hr hfs
    | _, _, _, .closure hσ henv hb hfl hps hr ho hg hf, .fn fps fr fo fg ff _ =>
      .closure hσ henv hb hfl (Flows.trans fps hps) (Flow.trans hr fr) (fo ∘ ho) (fg ∘ hg) (ff ∘ hf)

  theorem LValsAll.flow : ∀ {vs : List Val} {t t' : LTy}, LValsAll p vs t → Flow p.top t t' →
      LValsAll p vs t'
    | _, _, _, .nil, _ => .nil
    | _, _, _, .cons h hs, f => .cons (LValTy.flow h f) (LValsAll.flow hs f)
end

theorem LValsTy.flows : ∀ {vs : List Val} {ts ts' : List LTy}, LValsTy p vs ts →
    Flows p.top ts ts' → LValsTy p vs ts'
  | _, _, _, .nil, .nil => .nil
  | _, _, _, .cons h hs, .cons f fs => .cons (LValTy.flow h f) (LValsTy.flows hs fs)

/-- Labels at the top do not constrain a value. -/
theorem LValTy.unraise {v : Val} {t : LTy} {e : LExpr} (h : LValTy p v (t.raise e)) :
    LValTy p v t := by
  cases t <;> simp only [LTy.raise] at h <;> cases h <;> constructor <;> assumption

theorem LValTy.raise {v : Val} {t : LTy} (e : LExpr) (h : LValTy p v t) : LValTy p v (t.raise e) :=
  LValTy.flow h (Flow.raise e t)

theorem LValsTy.unraise : ∀ {vs : List Val} {ts : List LTy} {e : LExpr},
    LValsTy p vs (ts.map (LTy.raise e)) → LValsTy p vs ts
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, _, .cons h hs => .cons (LValTy.unraise h) (LValsTy.unraise hs)

theorem LValsTy.length : ∀ {vs : List Val} {ts : List LTy}, LValsTy p vs ts → vs.length = ts.length
  | _, _, .nil => rfl
  | _, _, .cons _ hs => by simp [LValsTy.length hs]

theorem LValsAll.of_flows : ∀ {vs : List Val} {ts : List LTy} {t : LTy},
    LValsTy p vs ts → (∀ u ∈ ts, Flow p.top u t) → LValsAll p vs t
  | _, _, _, .nil, _ => .nil
  | _, _, _, .cons h hs, hf =>
    .cons (LValTy.flow h (hf _ List.mem_cons_self))
      (LValsAll.of_flows hs (fun u hu => hf u (List.mem_cons_of_mem _ hu)))

theorem lookup_ltyped {x : String} {t : LTy} :
    ∀ {env : Env} {Γ : LCtx}, LEnvTy p env Γ → lctxLookup Γ x = some t →
      ∃ v, lookup env x = some v ∧ LValTy p v t
  | _, _, .nil, h => by simp [lctxLookup] at h
  | _, _, .cons (x := y) hv henv, h => by
    simp only [lctxLookup] at h
    simp only [lookup]
    by_cases hx : x = y
    · simp [hx] at h ⊢
      subst h
      exact hv
    · simp [hx] at h ⊢
      exact lookup_ltyped henv h

theorem latom_ltyped {env : Env} {Γ : LCtx} {a : Atom} {t : LTy}
    (henv : LEnvTy p env Γ) (h : LAtomTy Γ a t) : ∃ v, evalAtom env a = some v ∧ LValTy p v t := by
  cases h with
  | var hx => exact lookup_ltyped henv hx
  | int => exact ⟨_, rfl, .int⟩
  | bool => exact ⟨_, rfl, .bool⟩
  | text => exact ⟨_, rfl, .text⟩
  | unit => exact ⟨_, rfl, .unit⟩

theorem latoms_ltyped {env : Env} {Γ : LCtx} {as : List Atom} {ts : List LTy}
    (henv : LEnvTy p env Γ) (h : LAtomsTy Γ as ts) :
    ∃ vs, evalAtoms env as = some vs ∧ LValsTy p vs ts := by
  induction h with
  | nil => exact ⟨[], rfl, .nil⟩
  | cons ha _ ih =>
    obtain ⟨v, hv, hvt⟩ := latom_ltyped henv ha
    obtain ⟨vs, hvs, hvst⟩ := ih
    exact ⟨v :: vs, by simp [evalAtoms, hv, hvs], .cons hvt hvst⟩

theorem bindParams_ltyped :
    ∀ {params : List (String × LTy)} {vs : List Val} {env : Env} {Γ : LCtx},
      LValsTy p vs (params.map Prod.snd) → LEnvTy p env Γ →
      LEnvTy p (bindParams params vs env) (params ++ Γ)
  | [], [], _, _, .nil, henv => by simpa [bindParams] using henv
  | (x, t) :: params, v :: vs, _, _, .cons hv hvs, henv => by
    have := bindParams_ltyped hvs henv
    simp only [bindParams, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
    exact .cons hv this

theorem prim_ltyped {pr : Prim} {ts : List Ty} {t : Ty} {vs : List Val} {v : Val} (l : LExpr)
    (hp : PrimTy pr ts t) (he : evalPrim pr vs = some v) : LValTy p v (ofScalar t l) := by
  cases hp <;> unfold evalPrim at he <;> split at he <;> simp_all [ofScalar] <;> subst he <;>
    constructor

/-! ### Instantiation with no type argument -/

theorem LExpr.inst_nil (e : LExpr) : e.inst [] = LExpr.bot.join e := by
  have hf : ∀ l : List String,
      l.filter (fun x => !(List.lookup x ([] : List (String × LExpr))).isSome) = l := by
    intro l; induction l <;> simp_all [List.lookup, List.filter]
  have hm : ∀ l : List String,
      l.filterMap (fun x => List.lookup x ([] : List (String × LExpr))) = [] := by
    intro l; induction l <;> simp_all [List.lookup, List.filterMap]
  simp only [LExpr.inst, hf, hm, LExpr.joinAll]

theorem LExpr.inst_nil_le {top : Label} (e : LExpr) : LExpr.le top (e.inst []) e := by
  rw [LExpr.inst_nil]
  exact LExpr.join_le (LExpr.bot_le e) (LExpr.le_refl e)

theorem LExpr.le_inst_nil {top : Label} (e : LExpr) : LExpr.le top e (e.inst []) := by
  rw [LExpr.inst_nil]
  exact LExpr.le_join_right _ _

mutual
  /-- With no type argument, labeled instantiation flows both ways with the type. -/
  theorem Flow.lsubst_nil {top : Label} (rd : String → Label) :
      ∀ t : LTy, Flow top (t.lsubst rd []) t ∧ Flow top t (t.lsubst rd [])
    | .int l => ⟨.int (LExpr.inst_nil_le l), .int (LExpr.le_inst_nil l)⟩
    | .bool l => ⟨.bool (LExpr.inst_nil_le l), .bool (LExpr.le_inst_nil l)⟩
    | .text l => ⟨.text (LExpr.inst_nil_le l), .text (LExpr.le_inst_nil l)⟩
    | .unit _ => ⟨.unit, .unit⟩
    | .list t l =>
      ⟨.list (Flow.lsubst_nil rd t).1 (LExpr.inst_nil_le l),
        .list (Flow.lsubst_nil rd t).2 (LExpr.le_inst_nil l)⟩
    | .option t l =>
      ⟨.option (Flow.lsubst_nil rd t).1 (LExpr.inst_nil_le l),
        .option (Flow.lsubst_nil rd t).2 (LExpr.le_inst_nil l)⟩
    | .result t l =>
      ⟨.result (Flow.lsubst_nil rd t).1 (LExpr.inst_nil_le l),
        .result (Flow.lsubst_nil rd t).2 (LExpr.le_inst_nil l)⟩
    | .record _ l => ⟨.record (LExpr.inst_nil_le l), .record (LExpr.le_inst_nil l)⟩
    | .fn ps r _ _ _ l =>
      ⟨.fn (Flows.lsubst_nil rd ps).2 (Flow.lsubst_nil rd r).1 id id id (LExpr.inst_nil_le l),
        .fn (Flows.lsubst_nil rd ps).1 (Flow.lsubst_nil rd r).2 id id id (LExpr.le_inst_nil l)⟩
    | .var _ l => ⟨.var (LExpr.inst_nil_le l), .var (LExpr.le_inst_nil l)⟩

  theorem Flows.lsubst_nil {top : Label} (rd : String → Label) :
      ∀ ts : List LTy, Flows top (LTy.lsubsts rd [] ts) ts ∧ Flows top ts (LTy.lsubsts rd [] ts)
    | [] => ⟨.nil, .nil⟩
    | t :: ts =>
      ⟨.cons (Flow.lsubst_nil rd t).1 (Flows.lsubst_nil rd ts).1,
        .cons (Flow.lsubst_nil rd t).2 (Flows.lsubst_nil rd ts).2⟩
end

theorem flows_lsub_nil_params (fn : Fun) :
    Flows p.top (fn.params.map (fun q => p.lsub [] q.2)) (fn.params.map Prod.snd) ∧
      Flows p.top (fn.params.map Prod.snd) (fn.params.map (fun q => p.lsub [] q.2)) := by
  induction fn.params with
  | nil => exact ⟨.nil, .nil⟩
  | cons q qs ih =>
    exact ⟨.cons (Flow.lsubst_nil _ q.2).1 ih.1, .cons (Flow.lsubst_nil _ q.2).2 ih.2⟩

theorem Flows.length {top : Label} : ∀ {a b : List LTy}, Flows top a b → a.length = b.length
  | _, _, .nil => rfl
  | _, _, .cons _ fs => by simp [Flows.length fs]

theorem lfield_typed {t : LTy} :
    ∀ {fields : List Val} {ts : List LTy} {i : Nat}, LValsTy p fields ts → ts[i]? = some t →
      ∃ v, fields[i]? = some v ∧ LValTy p v t
  | _, _, _, .nil, hi => by simp at hi
  | _, _, 0, .cons hv _, hi => by simp at hi ⊢; subst hi; exact hv
  | _, _, i + 1, .cons _ hfs, hi => by simp at hi ⊢; exact lfield_typed hfs hi

/-! ### Runs that keep to their typing -/

/-- An outcome: `P` of a value, `F` of a failure. A stuck run is T1's, and a run out of fuel
says nothing. -/
abbrev URes (P : Val → Prop) (F : Prop) : Res → Prop
  | .ok v => P v
  | .failed _ => F
  | .stuck => True
  | .outOfFuel => True

/-- A run whose events satisfy `A` and whose outcome satisfies `P` and `F`. -/
abbrev UOk (A : Event → Prop) (P : Val → Prop) (F : Prop) (run : Trace × Res) : Prop :=
  (∀ ev ∈ run.1, A ev) ∧ URes P F run.2

theorem uok_done {A : Event → Prop} {P : Val → Prop} {F : Prop} {v : Val} (h : P v) :
    UOk A P F (done v) :=
  And.intro (fun _ h => nomatch h) h

theorem UOk.mono {A A' : Event → Prop} {P P' : Val → Prop} {F F' : Prop}
    (hA : ∀ ev, A ev → A' ev) (hP : ∀ v, P v → P' v) (hF : F → F') {run : Trace × Res}
    (h : UOk A P F run) : UOk A' P' F' run := by
  obtain ⟨tr, r⟩ := run
  refine ⟨fun ev hev => hA ev (h.1 ev hev), ?_⟩
  have h2 := h.2
  cases r <;> simp only [URes] at h2 ⊢
  · exact hP _ h2
  · exact hF h2

theorem andThen_uok {A : Event → Prop} {P Q : Val → Prop} {F : Prop} {first : Trace × Res}
    {rest : Val → Trace × Res} (h1 : UOk A P F first) (h2 : ∀ v, P v → UOk A Q F (rest v)) :
    UOk A Q F (andThen first rest) := by
  obtain ⟨tr, r⟩ := first
  cases r with
  | ok v =>
    have h := h2 v h1.2
    simp only [andThen]
    generalize rest v = rv at h
    obtain ⟨tr', r'⟩ := rv
    refine ⟨fun ev hev => ?_, h.2⟩
    rcases List.mem_append.mp hev with hev | hev
    · exact h1.1 ev hev
    · exact h.1 ev hev
  | failed c => exact h1
  | stuck => exact h1
  | outOfFuel => exact h1

/-- The events a code with these effects may perform under `pc`: each at a sink whose `accepts`
is above `pc`. -/
def Allowed (p : Program) (pc : LExpr) (ε : Eff) : Event → Prop
  | .out _ => ε.op = true ∧ p.le pc (.ofLabel p.acceptsOp)
  | .get _ => ε.get = true ∧ p.le pc (.ofLabel p.acceptsGet)

/-- A failure, whose label is above `pc`. -/
abbrev Fails (p : Program) (pc : LExpr) (ε : Eff) : Prop := ∃ l, ε.fails = some l ∧ p.le pc l

/-- A run that keeps to a typing at `pc` with type `t` and effects `ε`. -/
abbrev UGood (p : Program) (pc : LExpr) (t : LTy) (ε : Eff) (run : Trace × Res) : Prop :=
  UOk (Allowed p pc ε) (fun v => LValTy p v t) (Fails p pc ε) run

theorem Allowed.mono {pc pc' : LExpr} {ε ε' : Eff} (hpc : p.le pc pc')
    (ho : ε'.op = true → ε.op = true) (hg : ε'.get = true → ε.get = true) :
    ∀ ev, Allowed p pc' ε' ev → Allowed p pc ε ev
  | .out _, ⟨h1, h2⟩ => ⟨ho h1, LExpr.le_trans hpc h2⟩
  | .get _, ⟨h1, h2⟩ => ⟨hg h1, LExpr.le_trans hpc h2⟩

theorem Fails.mono {pc pc' : LExpr} {ε ε' : Eff} (hpc : p.le pc pc')
    (hf : ∀ l, ε'.fails = some l → ∃ l', ε.fails = some l' ∧ p.le l l') :
    Fails p pc' ε' → Fails p pc ε
  | ⟨l, hl, hle⟩ =>
    let ⟨l', hl', hle'⟩ := hf l hl
    ⟨l', hl', LExpr.le_trans hpc (LExpr.le_trans hle hle')⟩

theorem union_op_left {ε₁ ε₂ : Eff} (h : ε₁.op = true) : (ε₁.union ε₂).op = true := by
  simp [Eff.union, h]
theorem union_op_right {ε₁ ε₂ : Eff} (h : ε₂.op = true) : (ε₁.union ε₂).op = true := by
  simp [Eff.union, h]
theorem union_get_left {ε₁ ε₂ : Eff} (h : ε₁.get = true) : (ε₁.union ε₂).get = true := by
  simp [Eff.union, h]
theorem union_get_right {ε₁ ε₂ : Eff} (h : ε₂.get = true) : (ε₁.union ε₂).get = true := by
  simp [Eff.union, h]

theorem union_fails_left {ε₁ ε₂ : Eff} :
    ∀ l, ε₁.fails = some l → ∃ l', (ε₁.union ε₂).fails = some l' ∧ p.le l l' := by
  intro l h
  cases h₂ : ε₂.fails with
  | none => exact ⟨l, by simp [Eff.union, h, h₂], LExpr.le_refl l⟩
  | some l₂ => exact ⟨l.join l₂, by simp [Eff.union, h, h₂], LExpr.le_join_left _ _⟩

theorem union_fails_right {ε₁ ε₂ : Eff} :
    ∀ l, ε₂.fails = some l → ∃ l', (ε₁.union ε₂).fails = some l' ∧ p.le l l' := by
  intro l h
  cases h₁ : ε₁.fails with
  | none => exact ⟨l, by simp [Eff.union, h, h₁], LExpr.le_refl l⟩
  | some l₁ => exact ⟨l₁.join l, by simp [Eff.union, h, h₁], LExpr.le_join_right _ _⟩

/-- A union of effects allows what either allows. -/
theorem allowed_union_left {pc : LExpr} {ε₁ ε₂ : Eff} :
    ∀ ev, Allowed p pc ε₁ ev → Allowed p pc (ε₁.union ε₂) ev :=
  Allowed.mono (LExpr.le_refl pc) union_op_left union_get_left

theorem allowed_union_right {pc pc' : LExpr} {ε₁ ε₂ : Eff} (hpc : p.le pc pc') :
    ∀ ev, Allowed p pc' ε₂ ev → Allowed p pc (ε₁.union ε₂) ev :=
  Allowed.mono hpc union_op_right union_get_right

theorem fails_union_left {pc : LExpr} {ε₁ ε₂ : Eff} : Fails p pc ε₁ → Fails p pc (ε₁.union ε₂) :=
  Fails.mono (LExpr.le_refl pc) union_fails_left

theorem fails_union_right {pc pc' : LExpr} {ε₁ ε₂ : Eff} (hpc : p.le pc pc') :
    Fails p pc' ε₂ → Fails p pc (ε₁.union ε₂) :=
  Fails.mono hpc union_fails_right

/-- The events a function value with these effects may perform, wherever it is called. -/
def FnAllowed (o g : Bool) : Event → Prop
  | .out _ => o = true
  | .get _ => g = true

/-- At a call whose program counter `pc'` the function's bound allows (`pc ⊑ pc'`), its events
are allowed, and so is its failure. -/
theorem bound_allowed {pc pc' : LExpr} {o g f : Bool} (hb : p.Bound pc' o g f)
    (hpc : p.le pc pc') : ∀ ev, FnAllowed o g ev → Allowed p pc (Eff.ofFn o g f) ev
  | .out _, h => ⟨h, LExpr.le_trans hpc (hb.1 h).1⟩
  | .get _, h => ⟨h, LExpr.le_trans hpc (hb.2.1 h).1⟩

theorem bound_fails {pc pc' : LExpr} {o g f : Bool} (hb : p.Bound pc' o g f)
    (hpc : p.le pc pc') (hf : f = true) : Fails p pc (Eff.ofFn o g f) :=
  ⟨.bot, by simp [Eff.ofFn, hf], LExpr.le_trans hpc (hb.2.2 hf).1⟩

/-- What instantiation by `σ` gives the symbolic labels: `ℓ*` of each type argument. -/
def Program.sd (p : Program) (σ : List (String × LTy)) : List (String × LExpr) :=
  σ.map (fun q => (q.1, q.2.deep p.recordDeep))

/-- Effects instantiated: the labels of failures. -/
def Eff.inst (s : List (String × LExpr)) (ε : Eff) : Eff := ⟨ε.op, ε.get, ε.fails.map (·.inst s)⟩

/-- A ground instantiation. -/
abbrev GroundSubst (σ : List (String × LTy)) : Prop := ∀ q ∈ σ, q.2.Ground

theorem LCtx.lsub_append (rd : String → Label) (σ : List (String × LTy)) (a b : LCtx) :
    (a ++ b).lsub rd σ = a.lsub rd σ ++ b.lsub rd σ := List.map_append

theorem LCtx.lsub_snd (rd : String → Label) (σ : List (String × LTy)) (a : LCtx) :
    (a.lsub rd σ).map Prod.snd = (a.map Prod.snd).map (LTy.lsubst rd σ) := by
  simp [LCtx.lsub, List.map_map, Function.comp_def]

/-- A call of a typed function value on typed arguments keeps to the function's type, when every
body keeps to its typing at every ground instantiation. -/
theorem applyWith_uok {run : Env → Expr → Trace × Res} {fv : Val} {vs : List Val}
    {ps : List LTy} {r : LTy} {o g f : Bool} {lf : LExpr}
    (hrun : ∀ {σ : List (String × LTy)} {env : Env} {Γ : LCtx} {E : List String} {e : Expr}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LExprTy p E Γ .bot e t ε →
      UGood p (LExpr.bot.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) (run env e))
    (hf : LValTy p fv (.fn ps r o g f lf)) (hvs : LValsTy p vs ps) :
    UOk (FnAllowed o g) (fun v => LValTy p v r) (f = true) (applyWith run fv vs) := by
  cases hf with
  | @closure params _ cenv Γc _ _ _ _ _ _ _ _ _ σc hσ henv hb hfl hps hr ho hg hfs =>
    have hvs' := LValsTy.flows hvs hps
    rw [applyWith, ite_eq_left (by rw [LValsTy.length hvs', List.length_map, List.length_map])]
    have henv' : LEnvTy p (bindParams params vs cenv) ((params ++ Γc).lsub p.recordDeep σc) := by
      rw [LCtx.lsub_append]
      have := bindParams_ltyped (params := params.lsub p.recordDeep σc)
        (by rw [LCtx.lsub_snd]; exact hvs') henv
      simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
    refine UOk.mono ?_ (fun v hv => LValTy.flow hv hr) ?_ (hrun hσ henv' hb)
    · intro ev hev
      cases ev with
      | out _ => exact ho hev.1
      | get _ => exact hg hev.1
    · rintro ⟨l, hl, _⟩
      exact hfs (by simp [Eff.inst] at hl; obtain ⟨l', hl', _⟩ := hl; simp [hl'])

end Una.F1
