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
    | ok {v t e l} : LValTy p v t → LValTy p (.ok v) (.result t e l)
    | err {v t e l} : LValTy p v e → LValTy p (.err v) (.result t e l)
    | record {record fields rec l} :
        p.record record = some rec → LValsTy p fields rec.fields →
        LValTy p (.record record fields) (.record record l)
    | variant {kind ctor fields sm fs l} :
        p.sum kind = some sm → sm.variants[ctor]? = some fs → LValsTy p fields fs →
        LValTy p (.variant kind ctor fields) (.record kind l)
    | closure {params body env Γ E t ε ps r o g f l σ} :
        (∀ q ∈ σ, q.2.Ground) →
        LEnvTy p env (Γ.lsub p.recordDeep σ) → LExprTy p E (params ++ Γ) .bot body t ε →
        p.le ε.failLabel .bot → ε.rels = [] → ε.raises = [] →
        Flows p.top ps ((params.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
        Flow p.top (t.lsubst p.recordDeep σ) r →
        (ε.emits p = true → o = true) → HostFx.Le ε.host g → (ε.fails.isSome = true → f = true) →
        LValTy p (.closure params body env) (.fn ps r o g f l)
    | tuple {items ts l} : LValsTy p items ts → LValTy p (.tuple items) (.tuple ts l)

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

variable {p : Program} {H : Host}

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
    | _, _, _, .ok h, .result f _ _ => .ok (LValTy.flow h f)
    | _, _, _, .err h, .result _ f _ => .err (LValTy.flow h f)
    | _, _, _, .record hr hfs, .record _ => .record hr hfs
    | _, _, _, .variant hs hf hfs, .record _ => .variant hs hf hfs
    | _, _, _, .closure hσ henv hb hfl hrl hrs hps hr ho hg hf, .fn fps fr fo fg ff _ =>
      .closure hσ henv hb hfl hrl hrs (Flows.trans fps hps) (Flow.trans hr fr) (fo ∘ ho) (hg.trans fg) (ff ∘ hf)
    | _, _, _, .tuple h, .tuple fs _ => .tuple (LValsTy.flows h fs)

  theorem LValsAll.flow : ∀ {vs : List Val} {t t' : LTy}, LValsAll p vs t → Flow p.top t t' →
      LValsAll p vs t'
    | _, _, _, .nil, _ => .nil
    | _, _, _, .cons h hs, f => .cons (LValTy.flow h f) (LValsAll.flow hs f)

  theorem LValsTy.flows : ∀ {vs : List Val} {ts ts' : List LTy}, LValsTy p vs ts →
      Flows p.top ts ts' → LValsTy p vs ts'
    | _, _, _, .nil, .nil => .nil
    | _, _, _, .cons h hs, .cons f fs => .cons (LValTy.flow h f) (LValsTy.flows hs fs)
end

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
  cases hp <;> unfold evalPrim at he <;> split at he <;> simp_all [ofScalar]
  all_goals (try obtain ⟨_, _, rfl⟩ := he)
  all_goals (try subst he)
  all_goals constructor

/-! ### Instantiation with no type argument -/

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
    | .result t e l =>
      ⟨.result (Flow.lsubst_nil rd t).1 (Flow.lsubst_nil rd e).1 (LExpr.inst_nil_le l),
        .result (Flow.lsubst_nil rd t).2 (Flow.lsubst_nil rd e).2 (LExpr.le_inst_nil l)⟩
    | .record _ l => ⟨.record (LExpr.inst_nil_le l), .record (LExpr.le_inst_nil l)⟩
    | .fn ps r _ _ _ l =>
      ⟨.fn (Flows.lsubst_nil rd ps).2 (Flow.lsubst_nil rd r).1 id (HostFx.Le.refl _) id (LExpr.inst_nil_le l),
        .fn (Flows.lsubst_nil rd ps).1 (Flow.lsubst_nil rd r).2 id (HostFx.Le.refl _) id (LExpr.le_inst_nil l)⟩
    | .var _ l => ⟨.var (LExpr.inst_nil_le l), .var (LExpr.le_inst_nil l)⟩
    | .tuple ts l =>
      ⟨.tuple (Flows.lsubst_nil rd ts).1 (LExpr.inst_nil_le l),
        .tuple (Flows.lsubst_nil rd ts).2 (LExpr.le_inst_nil l)⟩

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

/-- What a run's failure must satisfy (#512): a code failure `code`, and a failure with a
payload `raised`, of its kind, variant and payload; and what an exit of a loop's iteration must
satisfy (ADR 0116 rule 20), of its tag and state. -/
structure FailCond where
  code : Prop
  raised : String → Nat → List Val → Prop
  exit : Bool → List Val → Prop

/-- One condition on failures implies another, for both forms of failure, and for exits. -/
def FailCond.Le (F F' : FailCond) : Prop :=
  (F.code → F'.code) ∧ (∀ k c ps, F.raised k c ps → F'.raised k c ps) ∧
    ∀ b vs, F.exit b vs → F'.exit b vs

theorem FailCond.Le.refl (F : FailCond) : F.Le F := ⟨id, fun _ _ _ => id, fun _ _ => id⟩

theorem FailCond.Le.trans {F G K : FailCond} (h₁ : F.Le G) (h₂ : G.Le K) : F.Le K :=
  ⟨fun h => h₂.1 (h₁.1 h), fun k c ps h => h₂.2.1 k c ps (h₁.2.1 k c ps h),
    fun b vs h => h₂.2.2 b vs (h₁.2.2 b vs h)⟩

/-- An outcome: `P` of a value, `F` of a failure or an exit. A stuck run is T1's, and a run out
of fuel says nothing. -/
abbrev URes (P : Val → Prop) (F : FailCond) : Res → Prop
  | .ok v => P v
  | .failed _ => F.code
  | .raised k c ps => F.raised k c ps
  | .exit b vs => F.exit b vs
  | .stuck => True
  | .outOfFuel | .aborted => True

/-- A write event. -/
def Event.isWrite : Event → Bool
  | .write _ _ => true
  | _ => false

/-- A request event. -/
def Event.isReq : Event → Bool
  | .req _ _ => true
  | _ => false

/-- What a run with trace `tr` may do to the world it starts from, `w` (M5): change the file
system only if it wrote, and the servers only if it sent a request, and never the environment
(#487). -/
def WEff (tr : Trace) (w w' : World H) : Prop :=
  (tr.any Event.isWrite = false → w'.fs = w.fs) ∧ (tr.any Event.isReq = false → w'.srv = w.srv) ∧
    w'.env = w.env

theorem WEff.self (tr : Trace) (w : World H) : WEff tr w w := ⟨fun _ => rfl, fun _ => rfl, rfl⟩

theorem WEff.trans {t₁ t₂ : Trace} {w₁ w₂ w₃ : World H} (h₁ : WEff t₁ w₁ w₂) (h₂ : WEff t₂ w₂ w₃) :
    WEff (t₁ ++ t₂) w₁ w₃ := by
  refine ⟨fun h => ?_, fun h => ?_, h₂.2.2.trans h₁.2.2⟩
  · simp only [List.any_append, Bool.or_eq_false_iff] at h
    exact (h₂.1 h.2).trans (h₁.1 h.1)
  · simp only [List.any_append, Bool.or_eq_false_iff] at h
    exact (h₂.2.1 h.2).trans (h₁.2.1 h.1)

/-- A run from `w` whose events satisfy `A`, whose outcome satisfies `P` and `F`, and whose
changes to the world are those its events allow. -/
abbrev UOk (A : Event → Prop) (P : Val → Prop) (F : FailCond) (w : World H) (run : Run H) : Prop :=
  (∀ ev ∈ run.1, A ev) ∧ URes P F run.2.1 ∧ WEff run.1 w run.2.2

theorem uok_done {A : Event → Prop} {P : Val → Prop} {F : FailCond} {w : World H}
    {v : Val} (h : P v) : UOk A P F w (done w v) :=
  ⟨fun _ h' => (nomatch h'), h, WEff.self _ w⟩

theorem uok_stuck {A : Event → Prop} {P : Val → Prop} {F : FailCond} {w : World H} :
    UOk A P F w (stuck w) :=
  ⟨fun _ h' => (nomatch h'), trivial, WEff.self _ w⟩

theorem UOk.mono {A A' : Event → Prop} {P P' : Val → Prop} {F F' : FailCond}
    {w : World H} (hA : ∀ ev, A ev → A' ev) (hP : ∀ v, P v → P' v) (hF : F.Le F')
    {run : Run H} (h : UOk A P F w run) : UOk A' P' F' w run := by
  obtain ⟨tr, r, w'⟩ := run
  refine ⟨fun ev hev => hA ev (h.1 ev hev), ?_, h.2.2⟩
  have h2 := h.2.1
  cases r <;> simp only [URes] at h2 ⊢
  · exact hP _ h2
  · exact hF.1 h2
  · exact hF.2.1 _ _ _ h2
  · exact hF.2.2 _ _ h2

/-- A ghost in front of a run that keeps to `A` (#561): the ghost is allowed, and it changes
nothing in the world. -/
theorem UOk.ghost {A : Event → Prop} {P : Val → Prop} {F : FailCond} {w : World H} {run : Run H}
    {ev : Event} (hw : ev.isWrite = false) (hr : ev.isReq = false) (hA : A ev) (h : UOk A P F w run) :
    UOk A P F w (Run.ghost ev run) := by
  obtain ⟨tr, r, w'⟩ := run
  refine ⟨fun ev hev => ?_, h.2.1, ?_⟩
  · simp only [Run.ghost, List.mem_cons] at hev
    rcases hev with rfl | hev
    · exact hA
    · exact h.1 ev hev
  · have h3 := h.2.2
    simpa [Run.ghost, WEff, List.any_cons, hw, hr] using h3

theorem andThen_uok {A : Event → Prop} {P Q : Val → Prop} {F : FailCond} {w : World H}
    {first : Run H} {rest : World H → Val → Run H} (h1 : UOk A P F w first)
    (h2 : ∀ w' v, P v → UOk A Q F w' (rest w' v)) :
    UOk A Q F w (andThen first rest) := by
  obtain ⟨tr, r, w₁⟩ := first
  cases r with
  | ok v =>
    have h := h2 w₁ v h1.2.1
    simp only [andThen]
    generalize rest w₁ v = rv at h
    obtain ⟨tr', r', w₂⟩ := rv
    refine ⟨fun ev hev => ?_, h.2.1, WEff.trans h1.2.2 h.2.2⟩
    rcases List.mem_append.mp hev with hev | hev
    · exact h1.1 ev hev
    · exact h.1 ev hev
  | failed c => exact h1
  | raised _ _ _ => exact h1
  | exit _ _ => exact h1
  | stuck => exact h1
  | outOfFuel | aborted => exact h1

/-- The events a code with these effects may perform under `pc`: each at a sink whose `accepts`
is above `pc`. -/
def Allowed (p : Program) (pc : LExpr) (ε : Eff) : Event → Prop
  | .out _ => ε.emits p = true ∧ p.le pc (.ofLabel p.acceptsOp)
  | .read _ => ε.host.rd = true ∧ p.le pc (.ofLabel p.acceptsRead)
  | .write _ _ => ε.host.wr = true ∧ p.le pc (.ofLabel p.acceptsWrite) ∧
      (p.oldHost = false → p.le pc (.ofLabel p.fileState))
  | .req _ vs => ε.host.rq = true ∧ p.le pc (.ofLabel p.acceptsReq) ∧ Val.FirstOrderAll vs
  | .release pol v => pol ∈ ε.rels ∧ ∃ po, p.policy pol = some po ∧ p.le pc po.dst.top ∧
      (ValTy p v po.dst.erase ∨ ∃ c : Nat, v = .err (.int c))
  -- A ghost of a performance (#561): no event of an observer, allowed everywhere.
  | .perf _ => True
  | .perfFn _ => True
  | .keyUse _ => True

/-- A failure, whose label is above `pc`. -/
abbrev Fails (p : Program) (pc : LExpr) (ε : Eff) : Prop := ∃ l, ε.fails = some l ∧ p.le pc l

/-- A payload of the variant `c` of the sum kind `k`, at the fields' declared types (#512). -/
def PayTy (p : Program) (k : String) (c : Nat) (ps : List Val) : Prop :=
  ∃ sm fs, p.sum k = some sm ∧ sm.variants[c]? = some fs ∧ LValsTy p ps fs

/-- What instantiation by `σ` gives the symbolic labels: `ℓ*` of each type argument. -/
def Program.sd (p : Program) (σ : List (String × LTy)) : List (String × LExpr) :=
  σ.map (fun q => (q.1, q.2.deep p.recordDeep))

/-- The loop of a body's exits (ADR 0116 rule 20), as the unary and binary halves read it: the
state's declared types, and the ground instance of the typing. -/
abbrev ExitCtx := Option (List LTy × List (String × LTy))

/-- The loop `X` of a typing at the ground instance `σ`. -/
def Program.xsub (_ : Program) (σ : List (String × LTy)) (X : Option (List LTy)) : ExitCtx :=
  X.map fun S => (S, σ)

/-- What an exit must satisfy (ADR 0116 rule 20): in the body of a loop whose state has the
declared types `S`, at the ground instance `σ`, it runs at the public program counter, and it
carries a state of the types' instances; outside a loop's body (`none`), no exit happens. -/
def ExitAs (p : Program) (pc : LExpr) (X : ExitCtx) (vs : List Val) : Prop :=
  ∃ S σ, X = some (S, σ) ∧ p.le pc (LExpr.bot.inst (p.sd σ)) ∧
    LValsTy p vs (S.map (LTy.lsubst p.recordDeep σ))

theorem ExitAs.none {pc : LExpr} {vs : List Val} : ¬ ExitAs p pc none vs :=
  fun ⟨_, _, h, _⟩ => nomatch h

theorem ExitAs.mono {pc pc' : LExpr} {X : ExitCtx} {vs : List Val} (hpc : p.le pc pc')
    (h : ExitAs p pc' X vs) : ExitAs p pc X vs :=
  let ⟨S, σ, hX, hle, hvs⟩ := h
  ⟨S, σ, hX, LExpr.le_trans hpc hle, hvs⟩

/-- The failures a typing with effects `ε` allows at `pc`, in the body of the loop `X` (#512):
above `pc`, a code only when `ε` may fail with one, and a payload only of a kind `ε` raises, typed
by its variant; and exits as `ExitAs` says (ADR 0116 rule 20). -/
def FailsAsX (p : Program) (pc : LExpr) (ε : Eff) (X : ExitCtx) : FailCond :=
  ⟨Fails p pc ε ∧ ε.codes = true, fun k c ps => Fails p pc ε ∧ k ∈ ε.raises ∧ PayTy p k c ps,
    fun _ vs => ExitAs p pc X vs⟩

/-- The failures a typing with effects `ε` allows at `pc` outside a loop's body: no exit. -/
abbrev FailsAs (p : Program) (pc : LExpr) (ε : Eff) : FailCond := FailsAsX p pc ε none

/-- A run from `w` that keeps to a typing at `pc` with type `t` and effects `ε`, in the body of
the loop `X`. -/
abbrev UGoodX (p : Program) (pc : LExpr) (t : LTy) (ε : Eff) (X : ExitCtx) (w : World H)
    (run : Run H) : Prop :=
  UOk (Allowed p pc ε) (fun v => LValTy p v t) (FailsAsX p pc ε X) w run

/-- A run from `w` that keeps to a typing at `pc` with type `t` and effects `ε`. -/
abbrev UGood (p : Program) (pc : LExpr) (t : LTy) (ε : Eff) (w : World H) (run : Run H) : Prop :=
  UGoodX p pc t ε none w run

/-- A run outside a loop's body keeps to its typing in one. -/
theorem FailsAs.le_x {pc : LExpr} {ε : Eff} (X : ExitCtx) :
    (FailsAs p pc ε).Le (FailsAsX p pc ε X) :=
  ⟨id, fun _ _ _ => id, fun _ _ h => (ExitAs.none h).elim⟩

theorem Allowed.mono {pc pc' : LExpr} {ε ε' : Eff} (hpc : p.le pc pc')
    (ho : ε'.emits p = true → ε.emits p = true) (hg : HostFx.Le ε'.host ε.host)
    (hr : ∀ x ∈ ε'.rels, x ∈ ε.rels) :
    ∀ ev, Allowed p pc' ε' ev → Allowed p pc ε ev
  | .out _, ⟨h1, h2⟩ => ⟨ho h1, LExpr.le_trans hpc h2⟩
  | .read _, ⟨h1, h2⟩ => ⟨hg.1 h1, LExpr.le_trans hpc h2⟩
  | .write _ _, ⟨h1, h2, h3⟩ => ⟨hg.2.1 h1, LExpr.le_trans hpc h2, fun h => LExpr.le_trans hpc (h3 h)⟩
  | .req _ _, ⟨h1, h2, h3⟩ => ⟨hg.2.2.1 h1, LExpr.le_trans hpc h2, h3⟩
  | .release _ _, ⟨h1, po, hpo, h2, h3⟩ => ⟨hr _ h1, po, hpo, LExpr.le_trans hpc h2, h3⟩
  | .perf _, _ => trivial
  | .perfFn _, _ => trivial
  | .keyUse _, _ => trivial

theorem Fails.mono {pc pc' : LExpr} {ε ε' : Eff} (hpc : p.le pc pc')
    (hf : ∀ l, ε'.fails = some l → ∃ l', ε.fails = some l' ∧ p.le l l') :
    Fails p pc' ε' → Fails p pc ε
  | ⟨l, hl, hle⟩ =>
    let ⟨l', hl', hle'⟩ := hf l hl
    ⟨l', hl', LExpr.le_trans hpc (LExpr.le_trans hle hle')⟩

theorem union_op_left {ε₁ ε₂ : Eff} (h : ε₁.emits p = true) : (ε₁.union ε₂).emits p = true := by
  simp only [Eff.emits, Eff.union, Bool.or_eq_true, List.any_append] at h ⊢
  rcases h with h | h <;> simp [h]
theorem union_op_right {ε₁ ε₂ : Eff} (h : ε₂.emits p = true) : (ε₁.union ε₂).emits p = true := by
  simp only [Eff.emits, Eff.union, Bool.or_eq_true, List.any_append] at h ⊢
  rcases h with h | h <;> simp [h]
theorem union_host_left {ε₁ ε₂ : Eff} : HostFx.Le ε₁.host (ε₁.union ε₂).host :=
  ⟨fun h => by simp [Eff.union, HostFx.or, h], fun h => by simp [Eff.union, HostFx.or, h],
    fun h => by simp [Eff.union, HostFx.or, h], fun h => by simp [Eff.union, HostFx.or, h]⟩
theorem union_host_right {ε₁ ε₂ : Eff} : HostFx.Le ε₂.host (ε₁.union ε₂).host :=
  ⟨fun h => by simp [Eff.union, HostFx.or, h], fun h => by simp [Eff.union, HostFx.or, h],
    fun h => by simp [Eff.union, HostFx.or, h], fun h => by simp [Eff.union, HostFx.or, h]⟩

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

theorem union_rels_left {ε₁ ε₂ : Eff} : ∀ x ∈ ε₁.rels, x ∈ (ε₁.union ε₂).rels := by
  intro x hx; simp [Eff.union, hx]
theorem union_rels_right {ε₁ ε₂ : Eff} : ∀ x ∈ ε₂.rels, x ∈ (ε₁.union ε₂).rels := by
  intro x hx; simp [Eff.union, hx]

/-- A union of effects allows what either allows. -/
theorem allowed_union_left {pc : LExpr} {ε₁ ε₂ : Eff} :
    ∀ ev, Allowed p pc ε₁ ev → Allowed p pc (ε₁.union ε₂) ev :=
  Allowed.mono (LExpr.le_refl pc) union_op_left union_host_left union_rels_left

theorem allowed_union_right {pc pc' : LExpr} {ε₁ ε₂ : Eff} (hpc : p.le pc pc') :
    ∀ ev, Allowed p pc' ε₂ ev → Allowed p pc (ε₁.union ε₂) ev :=
  Allowed.mono hpc union_op_right union_host_right union_rels_right

theorem fails_union_left {pc : LExpr} {ε₁ ε₂ : Eff} : Fails p pc ε₁ → Fails p pc (ε₁.union ε₂) :=
  Fails.mono (LExpr.le_refl pc) union_fails_left

theorem fails_union_right {pc pc' : LExpr} {ε₁ ε₂ : Eff} (hpc : p.le pc pc') :
    Fails p pc' ε₂ → Fails p pc (ε₁.union ε₂) :=
  Fails.mono hpc union_fails_right

/-- The events a function value with these effects may perform, wherever it is called. -/
def FnAllowed (o : Bool) (g : HostFx) : Event → Prop
  | .out _ => o = true
  | .read _ => g.rd = true
  | .write _ _ => g.wr = true
  | .req _ vs => g.rq = true ∧ Val.FirstOrderAll vs
  | .release _ _ => False
  | .perf _ => True
  | .perfFn _ => True
  | .keyUse _ => True

/-- At a call whose program counter `pc'` the function's bound allows (`pc ⊑ pc'`), its events
are allowed, and so is its failure. -/
theorem bound_allowed {pc pc' : LExpr} {o : Bool} {g : HostFx} {f : Bool} (hb : p.Bound pc' o g f)
    (hpc : p.le pc pc') : ∀ ev, FnAllowed o g ev → Allowed p pc (Eff.ofFn o g f) ev
  | .out _, h => ⟨by simpa [Eff.emits, Eff.ofFn, FnAllowed] using h, LExpr.le_trans hpc (hb.1 h).1⟩
  | .read _, h => ⟨h, LExpr.le_trans hpc (hb.2.1 h).1⟩
  | .write _ _, h => ⟨h, LExpr.le_trans hpc (hb.2.2.1 h).1, fun ho => LExpr.le_trans hpc ((hb.2.2.1 h).2.1 ho)⟩
  | .req _ _, h => ⟨h.1, LExpr.le_trans hpc (hb.2.2.2.1 h.1).1, h.2⟩
  | .release _ _, h => h.elim
  | .perf _, _ => trivial
  | .perfFn _, _ => trivial
  | .keyUse _, _ => trivial

theorem bound_fails {pc pc' : LExpr} {o : Bool} {g : HostFx} {f : Bool} (hb : p.Bound pc' o g f)
    (hpc : p.le pc pc') (hf : f = true) : Fails p pc (Eff.ofFn o g f) :=
  ⟨.bot, by simp [Eff.ofFn, hf], LExpr.le_trans hpc (hb.2.2.2.2 hf).1⟩


/-- Effects instantiated: the labels of failures. -/
def Eff.inst (s : List (String × LExpr)) (ε : Eff) : Eff :=
  ⟨ε.op, ε.host, ε.fails.map (·.inst s), ε.rels, ε.codes, ε.raises, ε.dl.inst s, ε.invokes⟩

/-- A ground instantiation. -/
abbrev GroundSubst (σ : List (String × LTy)) : Prop := ∀ q ∈ σ, q.2.Ground

theorem LCtx.lsub_append (rd : String → Label) (σ : List (String × LTy)) (a b : LCtx) :
    (a ++ b).lsub rd σ = a.lsub rd σ ++ b.lsub rd σ := List.map_append

theorem LCtx.lsub_snd (rd : String → Label) (σ : List (String × LTy)) (a : LCtx) :
    (a.lsub rd σ).map Prod.snd = (a.map Prod.snd).map (LTy.lsubst rd σ) := by
  simp [LCtx.lsub, List.map_map, Function.comp_def]

/-- A call of a typed function value on typed arguments keeps to the function's type, when every
body keeps to its typing at every ground instantiation. -/
theorem applyWith_uok {run : World H → Env → Expr → Run H} {fv : Val} {vs : List Val}
    {ps : List LTy} {r : LTy} {o : Bool} {g : HostFx} {f : Bool} {lf : LExpr}
    (hrun : ∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String}
      {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LExprTy p E Γ .bot e t ε →
      UGood p (LExpr.bot.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w (run w env e))
    (hf : LValTy p fv (.fn ps r o g f lf)) (hvs : LValsTy p vs ps) (w : World H) :
    UOk (FnAllowed o g) (fun v => LValTy p v r) ⟨f = true, fun _ _ _ => False, fun _ _ => False⟩ w
      (applyWith run w fv vs) := by
  cases hf with
  | @closure params _ cenv Γc _ _ _ _ _ _ _ _ _ σc hσ henv hb hfl hrl hrs hps hr ho hg hfs =>
    have hvs' := LValsTy.flows hvs hps
    rw [applyWith, ite_eq_left (by rw [LValsTy.length hvs', List.length_map, List.length_map])]
    have henv' : LEnvTy p (bindParams params vs cenv) ((params ++ Γc).lsub p.recordDeep σc) := by
      rw [LCtx.lsub_append]
      have := bindParams_ltyped (params := params.lsub p.recordDeep σc)
        (by rw [LCtx.lsub_snd]; exact hvs') henv
      simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
    refine UOk.mono ?_ (fun v hv => LValTy.flow hv hr) ?_ (hrun (w := w) hσ henv' hb)
    · intro ev hev
      cases ev with
      | out _ => exact ho hev.1
      | read _ => exact hg.1 hev.1
      | write _ _ => exact hg.2.1 hev.1
      | req _ _ => exact ⟨hg.2.2.1 hev.1, hev.2.2⟩
      | release _ _ => have h := hev.1; simp [Eff.inst, hrl] at h
      | perf _ => trivial
      | perfFn _ => trivial
      | keyUse _ => trivial
    · -- A function value fails with a code only (#512): its body raises no payload.
      refine ⟨fun ⟨⟨l, hl, _⟩, _⟩ => hfs (by simp [Eff.inst] at hl; obtain ⟨l', hl', _⟩ := hl; simp [hl']),
        fun _ _ _ ⟨_, hk, _⟩ => ?_, fun _ _ h => (ExitAs.none h).elim⟩
      simp [Eff.inst, hrs] at hk

end Una.F1
