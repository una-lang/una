/-
The proof of trace safety (ADR 0092, #561, stage 1): a run of a function that passes the
condition check never performs a conditioned operation where its condition is false.

One invariant covers every frame of the run. A frame runs code that is either
- **free of `o`**: it performs `o` nowhere, and it starts only functions the check deems free of
  `o` (a closed set, `closedB`); the bodies of every closure are such code in stage 1; or
- **linked**: it passed the condition check under the path knowledge `K`, `K` holds of its
  environment, and its own condition for `o`, if true, implies the top condition (`Q`).

A free frame performs nothing; a linked frame performs `o` only where its condition holds, so
where `Q` does.
-/
import UnaFormal.F1.TraceSafety
import UnaFormal.F1.InPlace

namespace Una.F1

/-! ### Conditions: values, knowledge, implication, instantiation -/

theorem Cond.under_congr {σ σ' : CAtom → Bool} : ∀ (c : Cond), (∀ a ∈ c.atoms, σ a = σ' a) →
    c.under σ = c.under σ'
  | .tt, _ | .ff, _ => rfl
  | .var x, h => by simp [Cond.under, h (.var x) (by simp [Cond.atoms])]
  | .not c, h => by simp [Cond.under, Cond.under_congr c (fun a ha => h a (by simpa [Cond.atoms] using ha))]
  | .and a b, h => by
    simp only [Cond.under]
    rw [Cond.under_congr a (fun x hx => h x (by simp [Cond.atoms, List.mem_eraseDups, hx])),
      Cond.under_congr b (fun x hx => h x (by simp [Cond.atoms, List.mem_eraseDups, hx]))]
  | .or a b, h => by
    simp only [Cond.under]
    rw [Cond.under_congr a (fun x hx => h x (by simp [Cond.atoms, List.mem_eraseDups, hx])),
      Cond.under_congr b (fun x hx => h x (by simp [Cond.atoms, List.mem_eraseDups, hx]))]
  | .eqInt a b, h => by simp [Cond.under, h (.eq a b) (by simp [Cond.atoms])]
  | .ltInt a b, h => by simp [Cond.under, h (.lt a b) (by simp [Cond.atoms])]

/-- Some assignment in the table agrees with `σ` on the atoms. -/
theorem assignments_cover (σ : CAtom → Bool) : ∀ (as : List CAtom),
    ∃ r ∈ assignments as, ∀ a ∈ as, asAssign r a = σ a
  | [] => ⟨[], by simp [assignments], by simp⟩
  | a :: as => by
    obtain ⟨r, hr, hag⟩ := assignments_cover σ as
    refine ⟨(a, σ a) :: r, ?_, ?_⟩
    · simp only [assignments, List.mem_flatMap]
      refine ⟨r, hr, ?_⟩
      cases σ a <;> simp
    · intro b hb
      simp only [List.mem_cons] at hb
      rcases hb with rfl | hb
      · simp [asAssign, List.lookup]
      · by_cases hne : b = a
        · subst hne; simp [asAssign, List.lookup]
        · have hba : (b == a) = false := by simp [hne]
          simp only [asAssign, List.lookup, hba]
          exact hag b hb

/-- **The decision procedure is sound**: an implication it decides holds in every environment. -/
theorem implies_sound {hyp goal : Cond} (h : implies hyp goal = true) (σ : CAtom → Bool)
    (hh : hyp.under σ = true) : goal.under σ = true := by
  simp only [implies, Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true] at h
  obtain ⟨_, hall⟩ := h
  obtain ⟨r, hr, hag⟩ := assignments_cover σ _
  have h1 := hall r hr
  have ehyp : hyp.under (asAssign r) = hyp.under σ :=
    Cond.under_congr hyp fun a ha => hag a (by simp [List.mem_eraseDups, ha])
  have egoal : goal.under (asAssign r) = goal.under σ :=
    Cond.under_congr goal fun a ha => hag a (by simp [List.mem_eraseDups, ha])
  rw [ehyp, egoal, hh] at h1
  simpa using h1

/-- `K` holds in `env`: each literal is the value of its variable, and each variable a `let`
bound to a comparison holds that comparison's value (#573). -/
def KHolds (K : Know) (env : Env) : Prop :=
  (∀ x b, (x, b) ∈ K.lits → CAtom.eval env (.var x) = b) ∧
    ∀ x c, (x, c) ∈ K.defs → lookup env x = some (.bool (CAtom.eval env c))

theorem KHolds.empty {env : Env} : KHolds {} env :=
  ⟨(fun _ _ h => nomatch h), (fun _ _ h => nomatch h)⟩

theorem CAtom.lit_eval {env : Env} {a : CAtom} {b : Bool} (h : CAtom.eval env a = b) :
    (CAtom.lit a b).eval env = true := by
  cases a <;> cases b <;> simp_all [CAtom.lit, Cond.eval, Cond.under]

theorem Cond.conj_eval {env : Env} :
    ∀ {cs : List Cond}, (∀ c ∈ cs, c.eval env = true) → (Cond.conj cs).eval env = true
  | [], _ => rfl
  | c :: cs, h => by
    have h1 := h c (by simp)
    have h2 := Cond.conj_eval (cs := cs) fun d hd => h d (by simp [hd])
    simp only [Cond.conj, List.foldr_cons] at h2 ⊢
    simp only [Cond.eval, Cond.under] at h1 h2 ⊢
    simp [h1, h2]

theorem knowledge_eval {K : Know} {env : Env} (h : KHolds K env) :
    (knowledge K).eval env = true := by
  unfold knowledge
  apply Cond.conj_eval
  intro c hc
  rcases List.mem_append.mp hc with hc | hc
  · obtain ⟨q, hq, rfl⟩ := List.mem_map.mp hc
    exact CAtom.lit_eval (h.1 q.1 q.2 hq)
  · obtain ⟨d, hd, hc⟩ := List.mem_flatMap.mp hc
    obtain ⟨q, hq, rfl⟩ := List.mem_map.mp hc
    obtain ⟨hq, hqd⟩ := List.mem_filter.mp hq
    have hqd : q.1 = d.1 := by simpa using hqd
    have hv := h.1 q.1 q.2 hq
    have hdv := h.2 d.1 d.2 hd
    rw [hqd] at hv
    simp only [CAtom.eval, hdv] at hv
    exact CAtom.lit_eval hv

theorem KHolds.cons_lit {K : Know} {env : Env} {x : String} {b : Bool}
    (h : KHolds K env) (hx : CAtom.eval env (.var x) = b) : KHolds (K.lit x b) env := by
  refine ⟨fun y c hy => ?_, h.2⟩
  simp only [Know.lit, List.mem_cons, Prod.mk.injEq] at hy
  rcases hy with ⟨rfl, rfl⟩ | hy
  · exact hx
  · exact h.1 y c hy

theorem KHolds.define {K : Know} {env : Env} {x : String} {c : CAtom}
    (h : KHolds K env) (hx : lookup env x = some (.bool (CAtom.eval env c))) :
    KHolds (K.define x c) env := by
  unfold Know.define
  split
  · exact h
  · refine ⟨h.1, fun y d hy => ?_⟩
    simp only [List.mem_cons, Prod.mk.injEq] at hy
    rcases hy with ⟨rfl, rfl⟩ | hy
    · exact hx
    · exact h.2 y d hy

/-- An atom's value ignores a binding of a variable it does not name. -/
theorem CAtom.eval_cons {x : String} {v : Val} {env : Env} :
    ∀ (a : CAtom), (∀ y ∈ CAtom.vars a, y ≠ x) → CAtom.eval ((x, v) :: env) a = CAtom.eval env a
  | .var y, h => by
    have := h y (by simp [CAtom.vars])
    simp [CAtom.eval, lookup, this]
  | .eq a b, h | .lt a b, h => by
    have ha : evalAtom ((x, v) :: env) a = evalAtom env a := by
      cases a <;> simp_all [evalAtom, lookup, CAtom.vars]
    have hb : evalAtom ((x, v) :: env) b = evalAtom env b := by
      cases b <;> simp_all [evalAtom, lookup, CAtom.vars]
    simp [CAtom.eval, ha, hb]

theorem KHolds.forget_cons {K : Know} {env : Env} {x : String} (v : Val)
    (h : KHolds K env) : KHolds (forget K x) ((x, v) :: env) := by
  refine ⟨fun y c hy => ?_, fun y c hy => ?_⟩
  · simp only [forget, List.mem_filter, bne_iff_ne, ne_eq] at hy
    have := h.1 y c hy.1
    simpa [CAtom.eval, lookup, hy.2] using this
  · simp only [forget, List.mem_filter, Bool.and_eq_true, bne_iff_ne, ne_eq, Bool.not_eq_true'] at hy
    obtain ⟨hm, hyx, hcx⟩ := hy
    have hnx : x ∉ c.vars := by simpa using hcx
    have hc : CAtom.eval ((x, v) :: env) c = CAtom.eval env c :=
      CAtom.eval_cons c fun z hz hzx => hnx (hzx ▸ hz)
    rw [hc]
    simpa [lookup, Ne.symm hyx, hyx] using h.2 y c hm

/-- A condition's value ignores a binding of a variable it does not name. -/
theorem Cond.eval_cons {x : String} {v : Val} {env : Env} (c : Cond) (h : x ∉ c.vars) :
    c.eval ((x, v) :: env) = c.eval env := by
  unfold Cond.eval
  apply Cond.under_congr
  intro a ha
  apply CAtom.eval_cons
  intro y hy hyx
  subst hyx
  exact h (by simp only [Cond.vars, List.mem_flatMap]; exact ⟨a, ha, hy⟩)

/-! ### Values whose closures perform no `o` -/

section Values

variable (p : Program) (o : String) (S : String → Prop)

mutual
  /-- A value whose closures run code free of `o` that starts only functions in `S`. -/
  def Val.OFree : Val → Prop
    | .list items => Val.OFreeAll items
    | .some v | .ok v | .err v => Val.OFree v
    | .record _ fs | .variant _ _ fs | .tuple fs => Val.OFreeAll fs
    | .closure _ b env =>
      (b.reach p).1.contains o = false ∧ (∀ g ∈ (b.reach p).2, S g) ∧ Val.OFreeEnv env
    | .int _ | .bool _ | .text _ | .unit | .none => True

  def Val.OFreeAll : List Val → Prop
    | [] => True
    | v :: vs => Val.OFree v ∧ Val.OFreeAll vs

  def Val.OFreeEnv : List (String × Val) → Prop
    | [] => True
    | (_, v) :: env => Val.OFree v ∧ Val.OFreeEnv env
end

variable {p o S}

theorem Val.oFreeAll_iff : ∀ (vs : List Val), Val.OFreeAll p o S vs ↔ ∀ v ∈ vs, Val.OFree p o S v
  | [] => by simp [Val.OFreeAll]
  | v :: vs => by simp [Val.OFreeAll, Val.oFreeAll_iff vs]

theorem lookup_oFree : ∀ {env : Env} {x : String} {v : Val}, Val.OFreeEnv p o S env →
    lookup env x = some v → Val.OFree p o S v
  | [], _, _, _, h => by simp [lookup] at h
  | (y, u) :: env, x, v, hf, h => by
    simp only [Val.OFreeEnv] at hf
    simp only [lookup] at h
    split at h
    · cases h; exact hf.1
    · exact lookup_oFree hf.2 h

theorem evalAtom_oFree {env : Env} (hf : Val.OFreeEnv p o S env) :
    ∀ {a : Atom} {v : Val}, evalAtom env a = some v → Val.OFree p o S v
  | .var _, _, h => lookup_oFree hf h
  | .int _, _, h | .bool _, _, h | .text _, _, h | .unit, _, h => by
    simp only [evalAtom, Option.some.injEq] at h
    subst h
    simp [Val.OFree]

theorem evalAtoms_oFree {env : Env} (hf : Val.OFreeEnv p o S env) :
    ∀ {as : List Atom} {vs : List Val}, evalAtoms env as = some vs → Val.OFreeAll p o S vs
  | [], vs, h => by
    simp only [evalAtoms, Option.some.injEq] at h
    subst h
    simp [Val.OFreeAll]
  | a :: as, vs, h => by
    simp only [evalAtoms] at h
    cases ha : evalAtom env a <;> cases has : evalAtoms env as <;> simp [ha, has] at h
    subst h
    simp only [Val.OFreeAll]
    exact ⟨evalAtom_oFree hf ha, evalAtoms_oFree hf has⟩

set_option maxHeartbeats 1000000 in
theorem evalPrim_oFree {pr : Prim} {vs : List Val} {v : Val} (h : evalPrim pr vs = some v) :
    Val.OFree p o S v := by
  cases pr
  case slice =>
    rcases vs with _ | ⟨a, _ | ⟨b, _ | ⟨c, _ | ⟨d, rest⟩⟩⟩⟩ <;> (try cases a) <;> (try cases b) <;>
      (try cases c) <;> simp [evalPrim] at h <;> (try subst h) <;> simp [Val.OFree]
  all_goals rcases vs with _ | ⟨a, _ | ⟨b, _ | ⟨c, rest⟩⟩⟩ <;> (try cases a) <;> (try cases b) <;>
    simp [evalPrim] at h <;> (try obtain ⟨_, _, rfl⟩ := h) <;> (try subst h) <;> simp [Val.OFree]

theorem zip_oFree : ∀ (xs : List String) {vs : List Val} {env : Env},
    Val.OFreeAll p o S vs → Val.OFreeEnv p o S env → Val.OFreeEnv p o S (xs.zip vs ++ env)
  | [], _, _, _, he => by simpa using he
  | _ :: _, [], _, _, he => by simpa using he
  | _ :: xs, _ :: _, _, hv, he => by
    simp only [Val.OFreeAll] at hv
    simp only [List.zip_cons_cons, List.cons_append, Val.OFreeEnv]
    exact ⟨hv.1, zip_oFree xs hv.2 he⟩

theorem bindParams_oFree {α : Type} (ps : List (String × α)) {vs : List Val} {env : Env}
    (hv : Val.OFreeAll p o S vs) (he : Val.OFreeEnv p o S env) : Val.OFreeEnv p o S (bindParams ps vs env) :=
  zip_oFree _ hv he

end Values

/-! ### Runs that perform `o` only where `Q` holds -/

section Runs

variable {H : Host} {p : Program} {o : String} {S : String → Prop} {Q : Prop}

/-- A run whose every performance of `o` implies `Q`, and whose value and failure payload are
free of `o`. -/
abbrev RunOk (p : Program) (o : String) (S : String → Prop) (Q : Prop) (r : Run H) : Prop :=
  (∀ ev ∈ r.1, PerfEv o ev → Q) ∧ (∀ v, r.2.1 = .ok v → Val.OFree p o S v) ∧
    ∀ k c ps, r.2.1 = .raised k c ps → Val.OFreeAll p o S ps

theorem RunOk.quiet {t : Trace} {r : Res} {w : World H} (ht : ∀ ev ∈ t, PerfEv o ev → Q)
    (hv : ∀ v, r = .ok v → Val.OFree p o S v)
    (hr : ∀ k c ps, r = .raised k c ps → Val.OFreeAll p o S ps) : RunOk p o S Q ((t, r, w) : Run H) :=
  ⟨ht, hv, hr⟩

theorem RunOk.stuck (w : World H) : RunOk p o S Q (stuck w) :=
  ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩

theorem RunOk.abort (w : World H) : RunOk p o S Q (abort w) :=
  ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩

theorem RunOk.done (w : World H) {v : Val} (hv : Val.OFree p o S v) : RunOk p o S Q (done w v) :=
  ⟨(fun _ h => nomatch h), fun _ h => by cases h; exact hv, (fun _ _ _ h => nomatch h)⟩

/-- A run with no events, whose value is free of `o`. -/
theorem RunOk.ret (w : World H) {r : Res} (hr : ∀ v, r = .ok v → Val.OFree p o S v)
    (hr' : ∀ k c ps, r = .raised k c ps → Val.OFreeAll p o S ps := by
      intro _ _ _ h
      first
      | (simp at h; done)
      | (simp only [Option.elim] at h; split at h <;> simp_all)
      | (split at h <;> simp_all)) :
    RunOk p o S Q (([], r, w) : Run H) :=
  ⟨(fun _ h => nomatch h), hr, hr'⟩

theorem RunOk.mono {Q' : Prop} {r : Run H} (hq : Q → Q') (h : RunOk p o S Q r) : RunOk p o S Q' r :=
  ⟨fun ev hev hp => hq (h.1 ev hev hp), h.2⟩

theorem RunOk.ghost {r : Run H} {ev : Event} (hev : PerfEv o ev → Q) (h : RunOk p o S Q r) :
    RunOk p o S Q (Run.ghost ev r) := by
  obtain ⟨tr, res, w⟩ := r
  refine ⟨fun e he hp => ?_, h.2⟩
  simp only [Run.ghost, List.mem_cons] at he
  rcases he with rfl | he
  · exact hev hp
  · exact h.1 e he hp

theorem andThen_ok {r : Run H} {k : World H → Val → Run H} (h : RunOk p o S Q r)
    (hk : ∀ w v, r.2.1 = .ok v → Val.OFree p o S v → RunOk p o S Q (k w v)) :
    RunOk p o S Q (andThen r k) := by
  obtain ⟨t, res, w⟩ := r
  cases res with
  | ok v =>
    have h2 := hk w v rfl (h.2.1 v rfl)
    simp only [andThen]
    generalize k w v = kr at h2
    obtain ⟨t', r', w'⟩ := kr
    refine ⟨fun ev hev hp => ?_, h2.2⟩
    rcases List.mem_append.mp hev with hev | hev
    · exact h.1 ev hev hp
    · exact h2.1 ev hev hp
  | raised k c ps => exact ⟨h.1, (fun _ h' => nomatch h'), h.2.2⟩
  | failed _ | stuck | outOfFuel | aborted => exact ⟨h.1, (fun _ h' => nomatch h'), (fun _ _ _ h' => nomatch h')⟩

theorem walkMap_ok {app : World H → Val → Run H}
    (happ : ∀ w v, Val.OFree p o S v → RunOk p o S Q (app w v)) :
    ∀ (w : World H) (vs : List Val), Val.OFreeAll p o S vs → RunOk p o S Q (walkMap app w vs)
  | w, [], _ => RunOk.done w (by simp [Val.OFree, Val.OFreeAll])
  | w, v :: vs, hvs => by
    simp only [Val.OFreeAll] at hvs
    simp only [walkMap]
    refine andThen_ok (happ w v hvs.1) fun w' r _ hr =>
      andThen_ok (walkMap_ok happ w' vs hvs.2) fun w'' rest _ hrest => ?_
    cases rest with
    | list rs =>
      refine RunOk.done w'' ?_
      simp only [Val.OFree, Val.OFreeAll] at hrest ⊢
      exact ⟨hr, hrest⟩
    | _ => exact RunOk.stuck w''

theorem walkFilter_ok {app : World H → Val → Run H}
    (happ : ∀ w v, Val.OFree p o S v → RunOk p o S Q (app w v)) :
    ∀ (w : World H) (vs : List Val), Val.OFreeAll p o S vs → RunOk p o S Q (walkFilter app w vs)
  | w, [], _ => RunOk.done w (by simp [Val.OFree, Val.OFreeAll])
  | w, v :: vs, hvs => by
    simp only [Val.OFreeAll] at hvs
    simp only [walkFilter]
    refine andThen_ok (happ w v hvs.1) fun w' b _ _ => ?_
    cases b with
    | bool keep =>
      refine andThen_ok (walkFilter_ok happ w' vs hvs.2) fun w'' rest _ hrest => ?_
      cases rest with
      | list rs =>
        refine RunOk.done w'' ?_
        simp only [Val.OFree] at hrest ⊢
        cases keep
        · exact hrest
        · simp only [ite_true, Val.OFreeAll]
          exact ⟨hvs.1, hrest⟩
      | _ => exact RunOk.stuck w''
    | _ => exact RunOk.stuck w'

theorem walkFirst_ok {app : World H → Val → Run H}
    (happ : ∀ w v, Val.OFree p o S v → RunOk p o S Q (app w v)) :
    ∀ (w : World H) (vs : List Val), Val.OFreeAll p o S vs → RunOk p o S Q (walkFirst app w vs)
  | w, [], _ => RunOk.done w (by simp [Val.OFree])
  | w, v :: vs, hvs => by
    simp only [Val.OFreeAll] at hvs
    simp only [walkFirst]
    refine andThen_ok (happ w v hvs.1) fun w' b _ _ => ?_
    cases b with
    | bool keep =>
      cases keep
      · exact walkFirst_ok happ w' vs hvs.2
      · exact RunOk.done w' (by simp only [Val.OFree]; exact hvs.1)
    | _ => exact RunOk.stuck w'

theorem walkFold_ok {app : World H → Val → Val → Run H}
    (happ : ∀ w a v, Val.OFree p o S a → Val.OFree p o S v → RunOk p o S Q (app w a v)) :
    ∀ (w : World H) (acc : Val) (vs : List Val), Val.OFree p o S acc → Val.OFreeAll p o S vs →
      RunOk p o S Q (walkFold app w acc vs)
  | w, acc, [], hacc, _ => RunOk.done w hacc
  | w, acc, v :: vs, hacc, hvs => by
    simp only [Val.OFreeAll] at hvs
    simp only [walkFold]
    exact andThen_ok (happ w acc v hacc hvs.1) fun w' acc' _ hacc' =>
      walkFold_ok happ w' acc' vs hacc' hvs.2

theorem walkLoop_ok {body : World H → Val → List Val → Run H}
    (hb : ∀ w v st, Val.OFree p o S v → Val.OFreeAll p o S st → RunOk p o S Q (body w v st)) :
    ∀ (w : World H) (st vs : List Val), Val.OFreeAll p o S st → Val.OFreeAll p o S vs →
      RunOk p o S Q (walkLoop body w st vs)
  | w, st, [], hst, _ => RunOk.done w (by simp only [Val.OFree]; exact hst)
  | w, st, v :: vs, hst, hvs => by
    simp only [Val.OFreeAll] at hvs
    simp only [walkLoop]
    refine andThen_ok (hb w v st hvs.1 hst) fun w' r _ hr => ?_
    cases r with
    | tuple st' =>
      simp only [Val.OFree] at hr
      simp only
      split
      · exact walkLoop_ok hb w' st' vs hr hvs.2
      · exact RunOk.stuck w'
    | _ => exact RunOk.stuck w'

theorem parseInt_ok (w : World H) (t : String) : RunOk p o S Q (parseInt w t) := by
  unfold parseInt
  cases t.toInt? with
  | some n => exact RunOk.done w (by simp [Val.OFree])
  | none => exact ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩

theorem checkRun_ok (w : World H) {v : Val} {pred : String} {code : Nat} (hv : Val.OFree p o S v) :
    RunOk p o S Q (checkRun p w v pred code) := by
  unfold checkRun
  split
  · split
    · exact RunOk.done w hv
    · exact ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  · exact RunOk.stuck w

/-- A call of a closure free of `o`: its body runs free of `o`. -/
theorem applyWith_ok {run : World H → Env → Expr → Run H}
    (hrun : ∀ w env e, Val.OFreeEnv p o S env → (e.reach p).1.contains o = false →
      (∀ g ∈ (e.reach p).2, S g) → RunOk p o S Q (run w env e))
    (w : World H) {fv : Val} {vs : List Val} (hfv : Val.OFree p o S fv) (hvs : Val.OFreeAll p o S vs) :
    RunOk p o S Q (applyWith run w fv vs) := by
  cases fv with
  | closure ps b cenv =>
    simp only [Val.OFree] at hfv
    simp only [applyWith]
    split
    · exact hrun w _ _ (bindParams_oFree ps hvs hfv.2.2) hfv.1 hfv.2.1
    · exact RunOk.stuck w
  | _ => exact RunOk.stuck w

end Runs

/-! ### Environments extended by names the conditions do not read -/

theorem lookup_prepend_of_not_mem : ∀ (pre : Env) {env : Env} {y : String}, y ∉ pre.map Prod.fst →
    lookup (pre ++ env) y = lookup env y
  | [], _, _, _ => rfl
  | (x, v) :: pre, env, y, h => by
    simp only [List.map_cons, List.mem_cons, not_or] at h
    simp only [List.cons_append, lookup]
    rw [if_neg h.1]
    exact lookup_prepend_of_not_mem pre h.2

theorem evalAtom_prepend {pre : Env} {env : Env} :
    ∀ (a : Atom), (∀ y, a = .var y → y ∉ pre.map Prod.fst) → evalAtom (pre ++ env) a = evalAtom env a
  | .var y, h => by simp only [evalAtom]; exact lookup_prepend_of_not_mem pre (h y rfl)
  | .int _, _ | .bool _, _ | .text _, _ | .unit, _ => rfl

theorem CAtom.eval_prepend {pre : Env} {env : Env} :
    ∀ (a : CAtom), (∀ y ∈ CAtom.vars a, y ∉ pre.map Prod.fst) → CAtom.eval (pre ++ env) a = CAtom.eval env a
  | .var y, h => by
    simp only [CAtom.eval]
    rw [lookup_prepend_of_not_mem pre (h y (by simp [CAtom.vars]))]
  | .eq a b, h | .lt a b, h => by
    have ha : evalAtom (pre ++ env) a = evalAtom env a :=
      evalAtom_prepend a fun y hy => h y (by subst hy; simp [CAtom.vars])
    have hb : evalAtom (pre ++ env) b = evalAtom env b :=
      evalAtom_prepend b fun y hy => h y (by subst hy; simp [CAtom.vars])
    simp only [CAtom.eval, ha, hb]

theorem Cond.eval_prepend {pre : Env} {env : Env} (c : Cond)
    (h : ∀ y ∈ c.vars, y ∉ pre.map Prod.fst) : c.eval (pre ++ env) = c.eval env := by
  unfold Cond.eval
  apply Cond.under_congr
  intro a ha
  apply CAtom.eval_prepend
  intro y hy
  exact h y (by simp only [Cond.vars, List.mem_flatMap]; exact ⟨a, ha, hy⟩)

/-- A name fresh for the conditions is not read by the condition of any operation. -/
theorem freshFor_lookup {cs : List (String × Cond)} {x o : String} (h : freshFor cs x = true) :
    x ∉ ((cs.lookup o).getD .tt).vars := by
  intro hx
  simp only [freshFor, Bool.not_eq_true', List.any_eq_false] at h
  cases hl : cs.lookup o with
  | none => simp [hl, Cond.vars, Cond.atoms] at hx
  | some c =>
    rw [hl] at hx
    have hmem : (o, c) ∈ cs := mem_of_lookup hl
    have := h (o, c) hmem
    exact this (by simpa using hx)

theorem forget_mem {K : Know} {x y : String} {b : Bool} (h : (y, b) ∈ (forget K x).lits) :
    (y, b) ∈ K.lits ∧ y ≠ x := by
  simp only [forget, List.mem_filter, bne_iff_ne, ne_eq] at h
  exact ⟨h.1, h.2⟩

theorem forgetAll_mem : ∀ (xs : List String) {K : Know} {y : String} {b : Bool},
    (y, b) ∈ (xs.foldl forget K).lits → (y, b) ∈ K.lits ∧ y ∉ xs
  | [], _, _, _, h => ⟨h, by simp⟩
  | x :: xs, K, y, b, h => by
    simp only [List.foldl_cons] at h
    obtain ⟨h1, h2⟩ := forgetAll_mem xs h
    obtain ⟨h3, h4⟩ := forget_mem h1
    exact ⟨h3, by simp [h4, h2]⟩

/-- A definition that survives forgetting `x` names neither `x` nor reads it. -/
theorem forget_defs_mem {K : Know} {x y : String} {c : CAtom} (h : (y, c) ∈ (forget K x).defs) :
    (y, c) ∈ K.defs ∧ y ≠ x ∧ x ∉ c.vars := by
  simp only [forget, List.mem_filter, Bool.and_eq_true, bne_iff_ne, ne_eq, Bool.not_eq_true'] at h
  obtain ⟨hm, hyx, hcx⟩ := h
  exact ⟨hm, hyx, by simpa using hcx⟩

theorem forgetAll_defs_mem : ∀ (xs : List String) {K : Know} {y : String} {c : CAtom},
    (y, c) ∈ (xs.foldl forget K).defs → (y, c) ∈ K.defs ∧ y ∉ xs ∧ ∀ z ∈ c.vars, z ∉ xs
  | [], _, _, _, h => ⟨h, by simp, by simp⟩
  | x :: xs, K, y, c, h => by
    simp only [List.foldl_cons] at h
    obtain ⟨h1, h2, h3⟩ := forgetAll_defs_mem xs h
    obtain ⟨h4, h5, h6⟩ := forget_defs_mem h1
    refine ⟨h4, by simp [h5, h2], fun z hz => ?_⟩
    simp only [List.mem_cons, not_or]
    exact ⟨fun hzx => h6 (hzx ▸ hz), h3 z hz⟩

/-- Knowledge that forgets the names bound in front of the environment still holds there. -/
theorem KHolds.prepend {K K' : Know} {env : Env} (pre : Env) (h : KHolds K env)
    (hK : ∀ y b, (y, b) ∈ K'.lits → (y, b) ∈ K.lits ∧ y ∉ pre.map Prod.fst)
    (hD : ∀ y c, (y, c) ∈ K'.defs →
      (y, c) ∈ K.defs ∧ y ∉ pre.map Prod.fst ∧ ∀ z ∈ c.vars, z ∉ pre.map Prod.fst) :
    KHolds K' (pre ++ env) := by
  refine ⟨fun y b hy => ?_, fun y c hy => ?_⟩
  · obtain ⟨hm, hn⟩ := hK y b hy
    have := h.1 y b hm
    simp only [CAtom.eval] at this ⊢
    rw [lookup_prepend_of_not_mem pre hn]
    exact this
  · obtain ⟨hm, hn, hv⟩ := hD y c hy
    rw [lookup_prepend_of_not_mem pre hn, CAtom.eval_prepend c hv]
    exact h.2 y c hm

/-! ### The invariant -/

section Invariant

variable {H : Host} {p : Program} {o : String} {S : String → Prop} {Q : Prop}

/-- The frame's own condition for `o`, if true, implies `Q`. -/
def Link (p : Program) (o : String) (Q : Prop) (cs : List (String × Cond)) (env : Env) : Prop :=
  ((cs.lookup o).getD .tt).eval env = true → Q

theorem Link.prepend {cs : List (String × Cond)} {env : Env} (pre : Env)
    (hf : ∀ x ∈ pre.map Prod.fst, freshFor cs x = true) (h : Link p o Q cs env) :
    Link p o Q cs (pre ++ env) := by
  intro hc
  apply h
  rw [Cond.eval_prepend] at hc
  · exact hc
  · intro y hy hmem
    exact freshFor_lookup (hf y hmem) hy

/-- Code a frame may run: free of `o`, or checked under `K` with its link. -/
abbrev GoodE (p : Program) (o : String) (S : String → Prop) (Q : Prop) (cs : List (String × Cond))
    (K : Know) (env : Env) (e : Expr) : Prop :=
  ((e.reach p).1.contains o = false ∧ ∀ g ∈ (e.reach p).2, S g) ∨
    (e.condOk p cs K = true ∧ KHolds K env ∧ Link p o Q cs env)

abbrev GoodR (p : Program) (o : String) (S : String → Prop) (Q : Prop) (cs : List (String × Cond))
    (K : Know) (env : Env) (r : Rhs) : Prop :=
  ((r.reach p).1.contains o = false ∧ ∀ g ∈ (r.reach p).2, S g) ∨
    (r.condOk p cs K = true ∧ KHolds K env ∧ Link p o Q cs env)

/-- Code free of `o` is good in any frame. -/
theorem GoodE.free {cs K env} {e : Expr} (h1 : (e.reach p).1.contains o = false)
    (h2 : ∀ g ∈ (e.reach p).2, S g) : GoodE p o S Q cs K env e := .inl ⟨h1, h2⟩

theorem contains_append_false {l₁ l₂ : List String} (h : (l₁ ++ l₂).contains o = false) :
    l₁.contains o = false ∧ l₂.contains o = false := by
  simp only [List.contains_append, Bool.or_eq_false_iff] at h
  exact h

/-- The two parts of a pair of reaches. -/
theorem pairJoin_free {a b : List String × List String}
    (h1 : (pairJoin a b).1.contains o = false) (h2 : ∀ g ∈ (pairJoin a b).2, S g) :
    (a.1.contains o = false ∧ ∀ g ∈ a.2, S g) ∧ (b.1.contains o = false ∧ ∀ g ∈ b.2, S g) := by
  simp only [pairJoin] at h1 h2
  obtain ⟨ha, hb⟩ := contains_append_false h1
  exact ⟨⟨ha, fun g hg => h2 g (List.mem_append_left _ hg)⟩,
    ⟨hb, fun g hg => h2 g (List.mem_append_right _ hg)⟩⟩

/-- A body run after binding `pre` in front of the environment, its names fresh and forgotten. -/
theorem GoodE.bind {cs : List (String × Cond)} {K K' : Know} {env : Env} {b : Expr}
    (pre : Env) (hok : b.condOk p cs K' = true) (hK : KHolds K env) (hl : Link p o Q cs env)
    (hf : ∀ x ∈ pre.map Prod.fst, freshFor cs x = true)
    (hK' : ∀ y c, (y, c) ∈ K'.lits → (y, c) ∈ K.lits ∧ y ∉ pre.map Prod.fst)
    (hD' : ∀ y c, (y, c) ∈ K'.defs →
      (y, c) ∈ K.defs ∧ y ∉ pre.map Prod.fst ∧ ∀ z ∈ c.vars, z ∉ pre.map Prod.fst) :
    GoodE p o S Q cs K' (pre ++ env) b :=
  .inr ⟨hok, KHolds.prepend pre hK hK' hD', Link.prepend pre hf hl⟩

/-! ### A callee's condition at the call -/

theorem evalAtoms_length {env : Env} : ∀ {as : List Atom} {vs : List Val},
    evalAtoms env as = some vs → vs.length = as.length
  | [], vs, h => by simp only [evalAtoms, Option.some.injEq] at h; subst h; rfl
  | a :: as, vs, h => by
    simp only [evalAtoms] at h
    cases ha : evalAtom env a <;> cases has : evalAtoms env as <;> simp [ha, has] at h
    subst h
    simp [evalAtoms_length has]

/-- The callee's environment reads a parameter as its argument reads in the caller's. -/
theorem lookup_zip_args {env : Env} :
    ∀ (names : List String) (args : List Atom) (vs : List Val) (x : String),
      evalAtoms env args = some vs → names.length = args.length →
      lookup (names.zip vs) x = ((names.zip args).lookup x).bind (evalAtom env)
  | [], _, _, x, _, _ => by simp [lookup]
  | _ :: _, [], _, _, _, h => by simp at h
  | n :: ns, a :: as, vs, x, hvs, hlen => by
    simp only [evalAtoms] at hvs
    cases ha : evalAtom env a <;> cases has : evalAtoms env as <;> simp [ha, has] at hvs
    subst hvs
    rename_i v vs'
    simp only [List.zip_cons_cons, lookup, List.lookup]
    by_cases hx : x = n
    · subst hx; simp [ha]
    · have hne : (x == n) = false := by simp [hx]
      rw [if_neg hx, hne]
      exact lookup_zip_args ns as vs' x has (by simpa using hlen)

theorem CAtom.eval_subst {env : Env} {names : List String} {args : List Atom} {vs : List Val}
    (hvs : evalAtoms env args = some vs) (hlen : names.length = args.length) :
    ∀ (c : Cond), (c.subst (names.zip args)).eval env = c.eval (names.zip vs ++ [])
  | .tt | .ff => rfl
  | .var x => by
    have hl := lookup_zip_args names args vs x hvs hlen
    simp only [List.append_nil] at *
    simp only [Cond.subst, Cond.eval]
    cases h : (names.zip args).lookup x with
    | none => simp [Cond.under, CAtom.eval, hl, h]
    | some a =>
      rw [h] at hl
      cases a with
      | var y => simp [Cond.under, CAtom.eval, hl, evalAtom]
      | bool b => cases b <;> simp [Cond.under, CAtom.eval, hl, evalAtom]
      | int _ | text _ | unit => simp [Cond.under, CAtom.eval, hl, evalAtom]
  | .not c => by
    simp only [Cond.subst, Cond.eval, Cond.under] at *
    rw [show (c.subst (names.zip args)).under (CAtom.eval env) = c.under (CAtom.eval (names.zip vs ++ [])) from
      CAtom.eval_subst hvs hlen c]
  | .and a b => by
    have ha := CAtom.eval_subst hvs hlen a
    have hb := CAtom.eval_subst hvs hlen b
    simp only [Cond.subst, Cond.eval, Cond.under] at *
    rw [ha, hb]
  | .or a b => by
    have ha := CAtom.eval_subst hvs hlen a
    have hb := CAtom.eval_subst hvs hlen b
    simp only [Cond.subst, Cond.eval, Cond.under] at *
    rw [ha, hb]
  | .eqInt a b | .ltInt a b => by
    have hat : ∀ t : Atom, (evalAtom env (substAtom (names.zip args) t)).bind (fun v => match v with
          | .int i => some i | _ => none) =
        (evalAtom (names.zip vs ++ []) t).bind (fun v => match v with | .int i => some i | _ => none) := by
      intro t
      cases t with
      | var y =>
        have hl := lookup_zip_args names args vs y hvs hlen
        simp only [List.append_nil, substAtom, evalAtom]
        cases h : (names.zip args).lookup y with
        | none => simp [h, hl, evalAtom]
        | some a' => simp only [h, hl, Option.bind_some]; cases a' <;> rfl
      | _ => simp [substAtom, evalAtom]
    have h1 := hat a
    have h2 := hat b
    simp only [Cond.subst, Cond.eval, Cond.under, CAtom.eval]
    revert h1 h2
    cases evalAtom env (substAtom (names.zip args) a) <;> cases evalAtom env (substAtom (names.zip args) b) <;>
      cases evalAtom (names.zip vs ++ []) a <;> cases evalAtom (names.zip vs ++ []) b <;>
      simp only [Option.bind] <;> (try split) <;> (try split) <;> (try split) <;> (try split) <;> simp_all

/-- A call that passes the check starts a good frame: free of `o` when the callee is deemed so,
or linked by the call's implication. -/
theorem callee_good (hS : ∀ g fn, p.find g = some fn → S g →
      (fn.body.reach p).1.contains o = false ∧ ∀ h ∈ (fn.body.reach p).2, S h)
    (hSmay : ∀ g, mayPerform p g o = false → S g)
    (hok : ∀ g fn, p.find g = some fn → fn.body.condOk p fn.conds {} = true)
    {cs : List (String × Cond)} {K : Know} {env : Env} {g : String} {fn : Fun}
    {args : List Atom} {vs : List Val}
    (hst : startOk p cs K g (some args) = true) (hK : KHolds K env) (hl : Link p o Q cs env)
    (hfd : p.find g = some fn) (hvs : evalAtoms env args = some vs) (hlen : vs.length = fn.params.length) :
    GoodE p o S Q fn.conds {} (bindParams fn.params vs []) fn.body := by
  cases hm : mayPerform p g o
  · have := hS g fn hfd (hSmay g hm)
    exact .inl this
  · refine .inr ⟨hok g fn hfd, KHolds.empty, fun hD => ?_⟩
    cases hlo : cs.lookup o with
    | none => exact hl (by simp [hlo, Cond.eval, Cond.under])
    | some c =>
      have hmem : (o, c) ∈ cs := mem_of_lookup hlo
      simp only [startOk, List.all_eq_true] at hst
      have hc := hst (o, c) hmem
      simp only [hm, Bool.not_true, Bool.false_or, hfd] at hc
      have hargs : args.length = fn.params.length := by
        rw [← hlen]; exact (evalAtoms_length hvs).symm
      rw [if_pos hargs] at hc
      have hsub := CAtom.eval_subst (names := fn.params.map Prod.fst) hvs (by simp [hargs])
        ((fn.conds.lookup o).getD .tt)
      have hD' : (((fn.conds.lookup o).getD .tt).subst ((fn.params.map Prod.fst).zip args)).eval env = true := by
        rw [hsub]; simpa [bindParams] using hD
      apply hl
      simp only [hlo, Option.getD_some]
      exact implies_sound hc (CAtom.eval env) (by
        simp only [Cond.under]
        rw [show (knowledge K).under (CAtom.eval env) = true from knowledge_eval hK]
        exact hD')

/-- A site that performs `x` directly, in a good frame: `x` is `o` only where `Q` holds. -/
theorem site_good {cs : List (String × Cond)} {K : Know} {env : Env} {r : Rhs} {x : String}
    (hg : GoodR p o S Q cs K env r) (hreach : (r.reach p).1 = [x])
    (hsite : r.condOk p cs K = true → siteOk cs K x = true) : x = o → Q := by
  intro hx
  subst hx
  rcases hg with ⟨h1, _⟩ | ⟨hc, hK, hl⟩
  · rw [hreach] at h1; simp at h1
  · have hs := hsite hc
    apply hl
    simp only [siteOk] at hs
    cases hlo : cs.lookup x with
    | none => simp [Cond.eval, Cond.under]
    | some c =>
      rw [hlo] at hs
      simp only [Option.getD_some]
      exact implies_sound hs (CAtom.eval env) (knowledge_eval hK)

/-- A closure's code in stage 1: free of every conditioned operation. -/
theorem closureFree_sound {body : Expr} (ho : o ∈ p.condOps)
    (hSmay : ∀ g, mayPerform p g o = false → S g) (h : closureFree p body = true) :
    (body.reach p).1.contains o = false ∧ ∀ g ∈ (body.reach p).2, S g := by
  simp only [closureFree, List.all_eq_true, Bool.and_eq_true, Bool.not_eq_true'] at h
  obtain ⟨h1, h2⟩ := h o ho
  exact ⟨h1, fun g hg => hSmay g (by simpa using h2 g hg)⟩

/-- Knowledge after forgetting `x`, in front of which `x` is bound. -/
theorem forget_one {K : Know} {x : String} :
    ∀ y c, (y, c) ∈ (forget K x).lits → (y, c) ∈ K.lits ∧ y ∉ [(x, (Val.unit : Val))].map Prod.fst := by
  intro y c h
  obtain ⟨h1, h2⟩ := forget_mem h
  exact ⟨h1, by simp [h2]⟩

theorem fresh_one {cs : List (String × Cond)} {x : String} (h : freshFor cs x = true) (v : Val) :
    ∀ y ∈ [(x, v)].map Prod.fst, freshFor cs y = true := by
  intro y hy; simp at hy; subst hy; exact h

/-- A body run with one more binding, in a good frame. -/
theorem GoodE.one {cs : List (String × Cond)} {K : Know} {env : Env} {b : Expr}
    {x : String} (v : Val) (hok : b.condOk p cs (forget K x) = true) (hK : KHolds K env)
    (hl : Link p o Q cs env) (hf : freshFor cs x = true) :
    GoodE p o S Q cs (forget K x) ((x, v) :: env) b := by
  have := GoodE.bind (p := p) (o := o) (S := S) (Q := Q) [(x, v)] hok hK hl (fresh_one hf v)
    (fun y c h => by obtain ⟨h1, h2⟩ := forget_mem h; exact ⟨h1, by simp [h2]⟩)
    (fun y c h => by
      obtain ⟨h1, h2, h3⟩ := forget_defs_mem h
      refine ⟨h1, by simp [h2], fun z hz => ?_⟩
      simp only [List.map_cons, List.map_nil, List.mem_cons, List.not_mem_nil, or_false]
      exact fun hzx => h3 (hzx ▸ hz))
  simpa using this

/-- What a comparison `let` binds is the comparison's value (#573): `lt` and `eqInt` give a
value only on two integers, and never abort. -/
theorem compares_value {n : Nat} {w : World H} {env : Env} {v : Rhs} {c : CAtom} {val : Val}
    (hc : v.compares = some c) (hr : (evalRhs H n p w env v).2.1 = .ok val) :
    val = .bool (CAtom.eval env c) := by
  cases n with
  | zero => simp [evalRhs] at hr
  | succ n =>
    unfold Rhs.compares at hc
    split at hc
    · rename_i a b
      simp only [Option.some.injEq] at hc
      subst hc
      simp only [evalRhs] at hr
      cases ha : evalAtom env a <;> cases hb : evalAtom env b <;>
        simp [ha, hb, evalAtoms, Prim.aborts, Option.elim] at hr
      rename_i va vb
      cases va <;> cases vb <;> simp_all [evalPrim, CAtom.eval]
    · rename_i a b
      simp only [Option.some.injEq] at hc
      subst hc
      simp only [evalRhs] at hr
      cases ha : evalAtom env a <;> cases hb : evalAtom env b <;>
        simp [ha, hb, evalAtoms, Prim.aborts, Option.elim] at hr
      rename_i va vb
      cases va <;> cases vb <;> simp_all [evalPrim, CAtom.eval]
      rw [← hr]
      exact congrArg Val.bool (Bool.eq_iff_iff.mpr (by simp [beq_iff_eq]))
    · simp at hc

/-- The body of `let x = v` in a good frame, under what the `let` teaches (#573). -/
theorem GoodE.after_let {cs : List (String × Cond)} {K : Know} {env : Env} {b : Expr} {x : String}
    {v : Rhs} (val : Val) (hok : b.condOk p cs (afterLet K x v) = true) (hK : KHolds K env)
    (hl : Link p o Q cs env) (hf : freshFor cs x = true)
    (hval : ∀ c, v.compares = some c → val = .bool (CAtom.eval env c)) :
    GoodE p o S Q cs (afterLet K x v) ((x, val) :: env) b := by
  unfold afterLet at hok ⊢
  cases hcv : v.compares with
  | none =>
    simp only [hcv] at hok ⊢
    exact GoodE.one val hok hK hl hf
  | some c =>
    simp only [hcv] at hok ⊢
    have hfk := KHolds.forget_cons (x := x) val hK
    have hlink : Link p o Q cs ((x, val) :: env) := by
      simpa using Link.prepend (p := p) (o := o) (Q := Q) [(x, val)] (fresh_one hf val) hl
    refine .inr ⟨hok, ?_, hlink⟩
    by_cases hcx : c.vars.contains x = true
    · have : (forget K x).define x c = forget K x := by
        unfold Know.define
        rw [if_pos hcx]
      rw [this]
      exact hfk
    · have hnx : x ∉ c.vars := fun hm => hcx (List.contains_iff_mem.mpr hm)
      apply KHolds.define hfk
      rw [CAtom.eval_cons c (fun z hz hzx => hnx (by rw [← hzx]; exact hz))]
      simp [lookup, hval c hcv]

/-- A branch of a good frame, under what the branch learns. -/
theorem ite_good {cs : List (String × Cond)} {K : Know} {env : Env} {c : Atom}
    {t e : Expr} {b : Bool} (hg : GoodE p o S Q cs K env (.ite c t e))
    (hc : evalAtom env c = some (.bool b)) : ∃ K', GoodE p o S Q cs K' env (if b then t else e) := by
  rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
  · obtain ⟨ht, he⟩ := pairJoin_free (S := S) (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)
    cases b
    · exact ⟨K, .inl he⟩
    · exact ⟨K, .inl ht⟩
  · cases c with
    | var x =>
      simp only [Expr.condOk, Bool.and_eq_true] at hok
      have hx : CAtom.eval env (.var x) = b := by
        simp only [evalAtom] at hc
        simp [CAtom.eval, hc]
      cases b
      · exact ⟨_, .inr ⟨hok.2, KHolds.cons_lit hK hx, hl⟩⟩
      · exact ⟨_, .inr ⟨hok.1, KHolds.cons_lit hK hx, hl⟩⟩
    | _ =>
      simp only [Expr.condOk, Bool.and_eq_true] at hok
      cases b
      · exact ⟨K, .inr ⟨hok.2, hK, hl⟩⟩
      · exact ⟨K, .inr ⟨hok.1, hK, hl⟩⟩

/-- The names of a zip are among its first list. -/
theorem zip_fst_mem {xs : List String} {vs : List Val} {y : String}
    (h : y ∈ (xs.zip vs).map Prod.fst) : y ∈ xs := by
  simp only [List.mem_map] at h
  obtain ⟨⟨a, b⟩, hab, rfl⟩ := h
  exact (List.of_mem_zip hab).1

/-- The expression half of the invariant, one step of fuel up. -/
theorem eval_good_expr
    (hS : ∀ g fn, p.find g = some fn → S g →
      (fn.body.reach p).1.contains o = false ∧ ∀ h ∈ (fn.body.reach p).2, S h)
    (n : Nat)
    (ihR : ∀ (w : World H) env cs K r, Val.OFreeEnv p o S env → GoodR p o S Q cs K env r →
      RunOk p o S Q (evalRhs H n p w env r))
    (ihE : ∀ (w : World H) env cs K e, Val.OFreeEnv p o S env → GoodE p o S Q cs K env e →
      RunOk p o S Q (eval H n p w env e)) :
    ∀ (w : World H) env cs K e, Val.OFreeEnv p o S env → GoodE p o S Q cs K env e →
      RunOk p o S Q (eval H (n + 1) p w env e) := by
  intro w env cs K e hf hg
  -- The two parts of a free pair.
  have split2 : ∀ {a b : List String × List String}, (pairJoin a b).1.contains o = false →
      (∀ g ∈ (pairJoin a b).2, S g) →
      (a.1.contains o = false ∧ ∀ g ∈ a.2, S g) ∧ (b.1.contains o = false ∧ ∀ g ∈ b.2, S g) :=
    fun h1 h2 => pairJoin_free h1 h2
  cases e with
  | atom a =>
    refine RunOk.ret w fun v hv => ?_
    cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
    subst hv
    exact evalAtom_oFree hf ha
  | letR x ann v b =>
    simp only [eval]
    have hv : GoodR p o S Q cs K env v ∧ ∀ val,
        (evalRhs H n p w env v).2.1 = .ok val → ∃ K', GoodE p o S Q cs K' ((x, val) :: env) b := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · obtain ⟨hv, hb⟩ := split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)
        exact ⟨.inl hv, fun _ _ => ⟨K, .inl hb⟩⟩
      · simp only [Expr.condOk, Bool.and_eq_true] at hok
        exact ⟨.inr ⟨hok.1.2, hK, hl⟩, fun val hres =>
          ⟨_, GoodE.after_let val hok.2 hK hl hok.1.1 fun c hc => compares_value hc hres⟩⟩
    exact andThen_ok (ihR w env cs K v hf hv.1) fun w' val hres hval => by
      obtain ⟨K', hb⟩ := hv.2 val hres
      exact ihE w' _ cs K' b (by simp only [Val.OFreeEnv]; exact ⟨hval, hf⟩) hb
  | letE x ann v b =>
    simp only [eval]
    have hv : GoodE p o S Q cs K env v ∧ ∀ val, ∃ K', GoodE p o S Q cs K' ((x, val) :: env) b := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · obtain ⟨hv, hb⟩ := split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)
        exact ⟨.inl hv, fun _ => ⟨K, .inl hb⟩⟩
      · simp only [Expr.condOk, Bool.and_eq_true] at hok
        exact ⟨.inr ⟨hok.1.2, hK, hl⟩, fun val => ⟨_, GoodE.one val hok.2 hK hl hok.1.1⟩⟩
    exact andThen_ok (ihE w env cs K v hf hv.1) fun w' val _ hval => by
      obtain ⟨K', hb⟩ := hv.2 val
      exact ihE w' _ cs K' b (by simp only [Val.OFreeEnv]; exact ⟨hval, hf⟩) hb
  | ite c t e =>
    simp only [eval]
    cases hc : evalAtom env c with
    | none => exact RunOk.stuck w
    | some v =>
      cases v with
      | bool b =>
        obtain ⟨K', hb⟩ := ite_good hg hc
        cases b
        · exact ihE w env cs K' e hf (by simpa using hb)
        · exact ihE w env cs K' t hf (by simpa using hb)
      | _ => exact RunOk.stuck w
  | caseOption sc ne x se =>
    simp only [eval]
    cases hs : evalAtom env sc with
    | none => exact RunOk.stuck w
    | some v =>
      cases v with
      | none =>
        have hn : ∃ K', GoodE p o S Q cs K' env ne := by
          rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
          · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).1⟩
          · simp only [Expr.condOk, Bool.and_eq_true] at hok
            exact ⟨K, .inr ⟨hok.1.2, hK, hl⟩⟩
        obtain ⟨K', hn⟩ := hn
        exact ihE w env cs K' ne hf hn
      | some y =>
        have hy := evalAtom_oFree hf hs
        simp only [Val.OFree] at hy
        have hsm : ∃ K', GoodE p o S Q cs K' ((x, y) :: env) se := by
          rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
          · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).2⟩
          · simp only [Expr.condOk, Bool.and_eq_true] at hok
            exact ⟨_, GoodE.one y hok.2 hK hl hok.1.1⟩
        obtain ⟨K', hsm⟩ := hsm
        exact ihE w _ cs K' se (by simp only [Val.OFreeEnv]; exact ⟨hy, hf⟩) hsm
      | _ => exact RunOk.stuck w
  | caseResult sc x oe y ee =>
    simp only [eval]
    cases hs : evalAtom env sc with
    | none => exact RunOk.stuck w
    | some v =>
      cases v with
      | ok z =>
        have hz := evalAtom_oFree hf hs
        simp only [Val.OFree] at hz
        have hb : ∃ K', GoodE p o S Q cs K' ((x, z) :: env) oe := by
          rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
          · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).1⟩
          · simp only [Expr.condOk, Bool.and_eq_true] at hok
            exact ⟨_, GoodE.one z hok.1.2 hK hl hok.1.1.1⟩
        obtain ⟨K', hb⟩ := hb
        exact ihE w _ cs K' oe (by simp only [Val.OFreeEnv]; exact ⟨hz, hf⟩) hb
      | err z =>
        have hz := evalAtom_oFree hf hs
        simp only [Val.OFree] at hz
        have hb : ∃ K', GoodE p o S Q cs K' ((y, z) :: env) ee := by
          rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
          · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).2⟩
          · simp only [Expr.condOk, Bool.and_eq_true] at hok
            exact ⟨_, GoodE.one z hok.2 hK hl hok.1.1.2⟩
        obtain ⟨K', hb⟩ := hb
        exact ihE w _ cs K' ee (by simp only [Val.OFreeEnv]; exact ⟨hz, hf⟩) hb
      | _ => exact RunOk.stuck w
  | fail c => exact ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  | raise kind code args =>
    simp only [eval]
    cases ha : evalAtoms env args with
    | none => exact RunOk.stuck w
    | some vs =>
      exact ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), fun _ _ _ h => by cases h; exact evalAtoms_oFree hf ha⟩
  | unreachable => exact ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  | withH o' f b =>
    simp only [eval]
    have hb : ∃ K', GoodE p o S Q cs K' ((markerName o', .text f) :: env) b := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).2⟩
      · simp only [Expr.condOk, Bool.and_eq_true] at hok
        exact ⟨_, GoodE.one _ hok.2 hK hl hok.1⟩
    obtain ⟨K', hb⟩ := hb
    exact ihE w _ cs K' b (by simp only [Val.OFreeEnv]; exact ⟨by simp [Val.OFree], hf⟩) hb
  | caseSum sc i xs t e =>
    simp only [eval]
    cases hs : evalAtom env sc with
    | none => exact RunOk.stuck w
    | some v =>
      cases v with
      | variant k j fs =>
        have hfs := evalAtom_oFree hf hs
        simp only [Val.OFree] at hfs
        dsimp only
        split
        · split
          · have ht : ∃ K', GoodE p o S Q cs K' (xs.zip fs ++ env) t := by
              rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
              · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).1⟩
              · simp only [Expr.condOk, Bool.and_eq_true, List.all_eq_true] at hok
                refine ⟨_, GoodE.bind (xs.zip fs) hok.1.2 hK hl (fun y hy => hok.1.1 y (zip_fst_mem hy))
                  (fun y c hyc => ?_) (fun y c hyc => ?_)⟩
                · obtain ⟨h3, h4⟩ := forgetAll_mem xs hyc
                  exact ⟨h3, fun hm => h4 (zip_fst_mem hm)⟩
                · obtain ⟨h3, h4, h5⟩ := forgetAll_defs_mem xs hyc
                  exact ⟨h3, fun hm => h4 (zip_fst_mem hm), fun z hz hm => h5 z hz (zip_fst_mem hm)⟩
            obtain ⟨K', ht⟩ := ht
            exact ihE w _ cs K' t (zip_oFree xs hfs hf) ht
          · exact RunOk.stuck w
        · have he : ∃ K', GoodE p o S Q cs K' env e := by
            rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
            · exact ⟨K, .inl (split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)).2⟩
            · simp only [Expr.condOk, Bool.and_eq_true] at hok
              exact ⟨K, .inr ⟨hok.2, hK, hl⟩⟩
          obtain ⟨K', he⟩ := he
          exact ihE w env cs K' e hf he
      | _ => exact RunOk.stuck w
  | attemptK kind b =>
    simp only [eval]
    have hb : GoodE p o S Q cs K env b := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · exact .inl ⟨by simpa [Expr.reach] using h1, by simpa [Expr.reach] using h2⟩
      · exact .inr ⟨by simpa [Expr.condOk] using hok, hK, hl⟩
    have h₂ := ihE w env cs K b hf hb
    generalize eval H n p w env b = res at h₂ ⊢
    obtain ⟨t, r, w'⟩ := res
    cases r with
    | ok x => exact ⟨h₂.1, fun _ h => by cases h; simp only [Val.OFree]; exact h₂.2.1 x rfl, (fun _ _ _ h => nomatch h)⟩
    | raised k c ps =>
      dsimp only
      split
      · exact ⟨h₂.1, fun _ h => by cases h; simp only [Val.OFree]; exact h₂.2.2 k c ps rfl, (fun _ _ _ h => nomatch h)⟩
      · exact ⟨h₂.1, (fun _ h => nomatch h), h₂.2.2⟩
    | failed c => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
    | stuck | outOfFuel | aborted => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  | handleK kind b x h =>
    simp only [eval]
    have hb : GoodE p o S Q cs K env b ∧ ∀ val, ∃ K', GoodE p o S Q cs K' ((x, val) :: env) h := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · obtain ⟨hb, hh⟩ := split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)
        exact ⟨.inl hb, fun _ => ⟨K, .inl hh⟩⟩
      · simp only [Expr.condOk, Bool.and_eq_true] at hok
        exact ⟨.inr ⟨hok.1.2, hK, hl⟩, fun val => ⟨_, GoodE.one val hok.2 hK hl hok.1.1⟩⟩
    have h₂ := ihE w env cs K b hf hb.1
    generalize eval H n p w env b = res at h₂ ⊢
    obtain ⟨t, r, w'⟩ := res
    cases r with
    | raised k c ps =>
      dsimp only
      split
      · obtain ⟨K', hh⟩ := hb.2 (.variant k c ps)
        have g₂ := ihE w' ((x, .variant k c ps) :: env) cs K' h
          (by simp only [Val.OFreeEnv, Val.OFree]; exact ⟨h₂.2.2 k c ps rfl, hf⟩) hh
        generalize eval H n p w' ((x, .variant k c ps) :: env) h = res' at g₂ ⊢
        obtain ⟨t', r', w''⟩ := res'
        refine ⟨fun ev hev hp => ?_, g₂.2⟩
        rcases List.mem_append.mp hev with hev | hev
        · exact h₂.1 ev hev hp
        · exact g₂.1 ev hev hp
      · exact ⟨h₂.1, (fun _ h => nomatch h), h₂.2.2⟩
    | ok v => exact ⟨h₂.1, fun _ h' => by cases h'; exact h₂.2.1 v rfl, (fun _ _ _ h' => nomatch h')⟩
    | failed c => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
    | stuck | outOfFuel | aborted => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  | attempt b =>
    simp only [eval]
    have hb : GoodE p o S Q cs K env b := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · exact .inl ⟨by simpa [Expr.reach] using h1, by simpa [Expr.reach] using h2⟩
      · exact .inr ⟨by simpa [Expr.condOk] using hok, hK, hl⟩
    have h₂ := ihE w env cs K b hf hb
    generalize eval H n p w env b = res at h₂ ⊢
    obtain ⟨t, r, w'⟩ := res
    cases r with
    | ok x => exact ⟨h₂.1, fun _ h => by cases h; simp only [Val.OFree]; exact h₂.2.1 x rfl, (fun _ _ _ h => nomatch h)⟩
    | failed c => exact ⟨h₂.1, fun _ h => by cases h; simp [Val.OFree], (fun _ _ _ h => nomatch h)⟩
    | raised k c ps => exact ⟨h₂.1, (fun _ h => nomatch h), h₂.2.2⟩
    | stuck | outOfFuel | aborted => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  | handle b x h =>
    simp only [eval]
    have hb : GoodE p o S Q cs K env b ∧ ∀ val, ∃ K', GoodE p o S Q cs K' ((x, val) :: env) h := by
      rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
      · obtain ⟨hb, hh⟩ := split2 (by simpa [Expr.reach] using h1) (by simpa [Expr.reach] using h2)
        exact ⟨.inl hb, fun _ => ⟨K, .inl hh⟩⟩
      · simp only [Expr.condOk, Bool.and_eq_true] at hok
        exact ⟨.inr ⟨hok.1.2, hK, hl⟩, fun val => ⟨_, GoodE.one val hok.2 hK hl hok.1.1⟩⟩
    have h₂ := ihE w env cs K b hf hb.1
    generalize eval H n p w env b = res at h₂ ⊢
    obtain ⟨t, r, w'⟩ := res
    cases r with
    | failed c =>
      obtain ⟨K', hh⟩ := hb.2 (.int c)
      have g₂ := ihE w' ((x, .int c) :: env) cs K' h
        (by simp only [Val.OFreeEnv, Val.OFree]; exact ⟨trivial, hf⟩) hh
      simp only []
      generalize eval H n p w' ((x, .int c) :: env) h = res' at g₂ ⊢
      obtain ⟨t', r', w''⟩ := res'
      refine ⟨fun ev hev hp => ?_, g₂.2⟩
      rcases List.mem_append.mp hev with hev | hev
      · exact h₂.1 ev hev hp
      · exact g₂.1 ev hev hp
    | ok v => exact ⟨h₂.1, fun _ h' => by cases h'; exact h₂.2.1 v rfl, (fun _ _ _ h' => nomatch h')⟩
    | raised k c ps => exact ⟨h₂.1, (fun _ h' => nomatch h'), h₂.2.2⟩
    | stuck | outOfFuel | aborted => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
  | letTuple bs v b =>
    simp only [eval]
    cases hv : evalAtom env v with
    | none => exact RunOk.stuck w
    | some x =>
      cases x with
      | tuple vs =>
        simp only
        split
        · have hvs := evalAtom_oFree hf hv
          simp only [Val.OFree] at hvs
          have hb : ∃ K', GoodE p o S Q cs K' (bindParams bs vs env) b := by
            rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
            · exact ⟨K, .inl ⟨by simpa [Expr.reach] using h1, by simpa [Expr.reach] using h2⟩⟩
            · simp only [Expr.condOk, Bool.and_eq_true, List.all_eq_true] at hok
              refine ⟨_, GoodE.bind _ hok.2 hK hl (fun y hy => ?_) (fun y c hyc => ?_) (fun y c hyc => ?_)⟩
              · obtain ⟨q, hq, rfl⟩ := List.mem_map.mp (zip_fst_mem hy)
                exact hok.1 q hq
              · rw [← List.foldl_map (f := Prod.fst) (g := forget)] at hyc
                obtain ⟨h3, h4⟩ := forgetAll_mem _ hyc
                exact ⟨h3, fun hm => h4 (zip_fst_mem hm)⟩
              · rw [← List.foldl_map (f := Prod.fst) (g := forget)] at hyc
                obtain ⟨h3, h4, h5⟩ := forgetAll_defs_mem _ hyc
                exact ⟨h3, fun hm => h4 (zip_fst_mem hm), fun z hz hm => h5 z hz (zip_fst_mem hm)⟩
          obtain ⟨K', hb⟩ := hb
          exact ihE w _ cs K' b (bindParams_oFree bs hvs hf) hb
        · exact RunOk.stuck w
      | _ => exact RunOk.stuck w
  | loop src x xt st init b =>
    simp only [eval]
    cases hs : sourceVals env src with
    | none => exact RunOk.stuck w
    | some items =>
      cases hi : evalAtoms env init with
      | none => exact RunOk.stuck w
      | some stv =>
        simp only
        split
        · have hitems : Val.OFreeAll p o S items := by
            cases src with
            | list xs =>
              simp only [sourceVals] at hs
              split at hs
              · rename_i its hx
                cases hs
                have := evalAtom_oFree hf hx
                simpa [Val.OFree] using this
              · cases hs
            | range lo hi' inc =>
              simp only [sourceVals] at hs
              split at hs
              · cases hs
                rw [Val.oFreeAll_iff]
                intro v hv
                simp only [rangeVals, List.mem_map] at hv
                obtain ⟨_, _, rfl⟩ := hv
                simp [Val.OFree]
              · cases hs
          have hb : ∀ v st', ∃ K', GoodE p o S Q cs K' ((x, v) :: bindParams st st' env) b := by
            intro v st'
            rcases hg with ⟨h1, h2⟩ | ⟨hok, hK, hl⟩
            · exact ⟨K, .inl ⟨by simpa [Expr.reach] using h1, by simpa [Expr.reach] using h2⟩⟩
            · simp only [Expr.condOk, Bool.and_eq_true, List.all_eq_true] at hok
              have := GoodE.bind (p := p) (o := o) (S := S) (Q := Q) ((x, v) :: (st.map Prod.fst).zip st')
                hok.2 hK hl (fun y hy => ?_) (fun y c hyc => ?_) (fun y c hyc => ?_)
              · exact ⟨_, by simpa [bindParams] using this⟩
              · simp only [List.map_cons, List.mem_cons] at hy
                rcases hy with rfl | hy
                · exact hok.1.1
                · obtain ⟨q, hq, rfl⟩ := List.mem_map.mp (zip_fst_mem hy)
                  exact hok.1.2 q hq
              · rw [← List.foldl_map (f := Prod.fst) (g := forget)] at hyc
                obtain ⟨h3, h4⟩ := forgetAll_mem _ hyc
                obtain ⟨h5, h6⟩ := forget_mem h3
                refine ⟨h5, ?_⟩
                simp only [List.map_cons, List.mem_cons, not_or]
                exact ⟨h6, fun hm => h4 (zip_fst_mem hm)⟩
              · rw [← List.foldl_map (f := Prod.fst) (g := forget)] at hyc
                obtain ⟨h3, h4, h5⟩ := forgetAll_defs_mem _ hyc
                obtain ⟨h7, h8, h9⟩ := forget_defs_mem h3
                refine ⟨h7, ?_, fun z hz => ?_⟩
                · simp only [List.map_cons, List.mem_cons, not_or]
                  exact ⟨h8, fun hm => h4 (zip_fst_mem hm)⟩
                · simp only [List.map_cons, List.mem_cons, not_or]
                  exact ⟨fun hzx => h9 (hzx ▸ hz), fun hm => h5 z hz (zip_fst_mem hm)⟩
          exact walkLoop_ok (fun w' v st' hv hst => by
              obtain ⟨K', hb'⟩ := hb v st'
              exact ihE w' _ cs K' b (by simp only [Val.OFreeEnv]; exact ⟨hv, bindParams_oFree st hst hf⟩) hb')
            w stv items (evalAtoms_oFree hf hi) hitems
        · exact RunOk.stuck w

/-- **The invariant holds along every run.** A good frame from values free of `o` performs `o`
only where `Q` holds, and gives values free of `o`. -/
theorem eval_good
    (hS : ∀ g fn, p.find g = some fn → S g →
      (fn.body.reach p).1.contains o = false ∧ ∀ h ∈ (fn.body.reach p).2, S h)
    (hSmay : ∀ g, mayPerform p g o = false → S g)
    (hok : ∀ g fn, p.find g = some fn → fn.body.condOk p fn.conds {} = true)
    (hvia : ∀ pol po, p.policy pol = some po → S po.via) (ho : o ∈ p.condOps)
    (hresp : ∀ m vs s v s', H.request m vs s = (.ok v, s') → Val.OFree p o S v) : ∀ n : Nat,
    (∀ (w : World H) env cs K r, Val.OFreeEnv p o S env → GoodR p o S Q cs K env r →
      RunOk p o S Q (evalRhs H n p w env r)) ∧
    (∀ (w : World H) env cs K e, Val.OFreeEnv p o S env → GoodE p o S Q cs K env e →
      RunOk p o S Q (eval H n p w env e))
  | 0 => ⟨fun _ _ _ _ _ _ _ => ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩,
      fun _ _ _ _ _ _ _ => ⟨(fun _ h => nomatch h), (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := eval_good hS hSmay hok hvia ho hresp n
    have hfree : ∀ (w : World H) env e, Val.OFreeEnv p o S env → (e.reach p).1.contains o = false →
        (∀ g ∈ (e.reach p).2, S g) → RunOk p o S Q (eval H n p w env e) :=
      fun w env e hf h1 h2 => ihE w env [] {} e hf (.inl ⟨h1, h2⟩)
    have happ : ∀ (w : World H) fv vs, Val.OFree p o S fv → Val.OFreeAll p o S vs →
        RunOk p o S Q (applyWith (eval H n p) w fv vs) :=
      fun w _ _ hfv hvs => applyWith_ok hfree w hfv hvs
    -- A callee run in a good frame.
    have hcall : ∀ (w : World H) env cs K r g args fn vs, Val.OFreeEnv p o S env →
        GoodR p o S Q cs K env r → ((r.reach p).2 = [g] ∨ g ∈ (r.reach p).2) →
        (r.condOk p cs K = true → startOk p cs K g (some args) = true) →
        p.find g = some fn → evalAtoms env args = some vs → vs.length = fn.params.length →
        RunOk p o S Q (eval H n p w (bindParams fn.params vs []) fn.body) := by
      intro w env cs K r g args fn vs hf hg hstart hst hfd hvs hlen
      have hfv := bindParams_oFree (p := p) (o := o) (S := S) fn.params (evalAtoms_oFree hf hvs)
        (env := []) trivial
      rcases hg with ⟨_, h2⟩ | ⟨hc, hK, hl⟩
      · have hg' : S g := h2 g (by rcases hstart with h | h <;> simp_all)
        exact ihE w _ [] {} fn.body hfv (.inl (hS g fn hfd hg'))
      · exact ihE w _ _ _ fn.body hfv (callee_good hS hSmay hok (hst hc) hK hl hfd hvs hlen)
    constructor
    · intro w env cs K r hf hg
      cases r with
      | atom a =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        exact evalAtom_oFree hf ha
      | prim pr args =>
        simp only [evalRhs]
        cases ha : evalAtoms env args with
        | none => exact RunOk.ret w fun v hv => by simp [Option.elim] at hv
        | some vs =>
          simp only [Option.map_some, Option.getD_some, Option.bind_some]
          split
          · exact RunOk.ret w fun v hv => by simp at hv
          · refine RunOk.ret w fun v hv => ?_
            cases hpr : evalPrim pr vs with
            | none => simp [hpr, Option.elim] at hv
            | some v' =>
              simp [hpr, Option.elim] at hv
              subst hv
              exact evalPrim_oFree hpr
      | call g targs args =>
        simp only [evalRhs]
        cases hfd : p.find g with
        | none => exact RunOk.stuck w
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact RunOk.stuck w
          | some vs =>
            simp only []
            split
            · rename_i hlen
              exact hcall w env cs K _ g args fn vs hf hg (.inl (by simp [Rhs.reach]))
                (fun hc => by simpa [Rhs.condOk] using hc) hfd ha hlen
            · exact RunOk.stuck w
      | invoke f args =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact RunOk.stuck w
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact RunOk.stuck w
          | some vs =>
            simp only []
            split
            · rename_i hlen
              refine RunOk.ghost (fun hp => ?_) (hcall w env cs K _ f args fn vs hf hg
                (.inl (by simp [Rhs.reach])) (fun hc => by
                  simp only [Rhs.condOk, Bool.and_eq_true] at hc; exact hc.2) hfd ha hlen)
              simp only [PerfEv] at hp
              exact site_good hg (by simp [Rhs.reach]) (fun hc => by
                simp only [Rhs.condOk, Bool.and_eq_true] at hc; exact hc.1) hp
            · exact RunOk.stuck w
      | invokeOp o' args =>
        simp only [evalRhs]
        have hmem := handlerFor_mem p o' (lookup env (markerName o'))
        cases hfd : p.find (p.handlerFor o' (lookup env (markerName o'))) with
        | none => exact RunOk.stuck w
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact RunOk.stuck w
          | some vs =>
            simp only []
            split
            · rename_i hlen
              refine RunOk.ghost (fun hp => ?_) (hcall w env cs K _ _ args fn vs hf hg
                (.inr (by simpa [Rhs.reach] using hmem)) (fun hc => by
                  simp only [Rhs.condOk, Bool.and_eq_true, List.all_eq_true] at hc
                  exact hc.2 _ hmem) hfd ha hlen)
              simp only [PerfEv] at hp
              exact site_good hg (by simp [Rhs.reach]) (fun hc => by
                simp only [Rhs.condOk, Bool.and_eq_true] at hc; exact hc.1) hp
            · exact RunOk.stuck w
      | apply f args =>
        simp only [evalRhs]
        cases hfa : evalAtom env f with
        | none => exact RunOk.stuck w
        | some fv =>
          cases ha : evalAtoms env args with
          | none => exact RunOk.stuck w
          | some vs => exact happ w fv vs (evalAtom_oFree hf hfa) (evalAtoms_oFree hf ha)
      | op line =>
        simp only [evalRhs]
        cases evalAtom env line with
        | none => exact RunOk.stuck w
        | some v =>
          dsimp only
          cases v.line with
          | none => exact RunOk.stuck w
          | some s =>
            refine ⟨fun ev hev hp => ?_, fun _ h => by cases h; simp [Val.OFree], (fun _ _ _ h => nomatch h)⟩
            simp only [List.mem_singleton] at hev
            subst hev
            simp only [PerfEv] at hp
            exact site_good hg (by simp [Rhs.reach]) (fun hc => by simpa [Rhs.condOk] using hc) hp.symm
      | read path =>
        simp only [evalRhs]
        cases evalAtom env path with
        | none => exact RunOk.stuck w
        | some v =>
          cases v with
          | text s =>
            have hev : ∀ ev ∈ [Event.read s], PerfEv o ev → Q := by
              intro ev hev hp
              simp only [List.mem_singleton] at hev
              subst hev
              simp only [PerfEv] at hp
              exact site_good hg (by simp [Rhs.reach]) (fun hc => by simpa [Rhs.condOk] using hc) hp.symm
            dsimp only
            cases H.read s w.fs with
            | some c => exact ⟨hev, fun _ h => by cases h; simp [Val.OFree], (fun _ _ _ h => nomatch h)⟩
            | none => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
          | _ => exact RunOk.stuck w
      | hmac key data =>
        simp only [evalRhs]
        cases evalAtom env key with
        | none => cases evalAtom env data <;> exact RunOk.stuck w
        | some v =>
          cases evalAtom env data with
          | none => cases v <;> exact RunOk.stuck w
          | some d =>
            cases v with
            | text k =>
              have hev : ∀ ev ∈ [Event.keyUse k], PerfEv o ev → Q := by
                intro ev hev hp
                simp only [List.mem_singleton] at hev
                subst hev
                simp only [PerfEv] at hp
                exact site_good hg (by simp [Rhs.reach]) (fun hc => by simpa [Rhs.condOk] using hc) hp.symm
              dsimp only
              cases d.line with
              | none => exact RunOk.stuck w
              | some s =>
                dsimp only
                cases H.key k with
                | some m => exact ⟨hev, fun _ h => by cases h; simp [Val.OFree], (fun _ _ _ h => nomatch h)⟩
                | none => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
            | _ => exact RunOk.stuck w
      | write path content =>
        simp only [evalRhs]
        cases evalAtom env path with
        | none => cases evalAtom env content <;> exact RunOk.stuck w
        | some v =>
          cases evalAtom env content with
          | none => cases v <;> exact RunOk.stuck w
          | some c =>
            cases v with
            | text s =>
              cases c with
              | text t =>
                have hev : ∀ ev ∈ [Event.write s t], PerfEv o ev → Q := by
                  intro ev hev hp
                  simp only [List.mem_singleton] at hev
                  subst hev
                  simp only [PerfEv] at hp
                  exact site_good hg (by simp [Rhs.reach]) (fun hc => by simpa [Rhs.condOk] using hc) hp.symm
                dsimp only
                cases H.write s t w.fs with
                | some fs' => exact ⟨hev, fun _ h => by cases h; simp [Val.OFree], (fun _ _ _ h => nomatch h)⟩
                | none => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
              | _ => exact RunOk.stuck w
            | _ => cases c <;> exact RunOk.stuck w
      | req m args =>
        simp only [evalRhs]
        cases ha : evalAtoms env args with
        | none => exact RunOk.stuck w
        | some vs =>
          have hev : ∀ ev ∈ [Event.req m vs], PerfEv o ev → Q := by
            intro ev hev hp
            simp only [List.mem_singleton] at hev
            subst hev
            simp only [PerfEv] at hp
            exact site_good hg (by simp [Rhs.reach]) (fun hc => by simpa [Rhs.condOk] using hc) hp.symm
          dsimp only
          cases hrq : H.request m vs w.srv with
          | mk res srv' =>
            cases res with
            | ok v => exact ⟨hev, fun _ h => by cases h; exact hresp m vs w.srv v srv' hrq, (fun _ _ _ h => nomatch h)⟩
            | error c => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
      | construct record fields =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtoms env fields <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtoms_oFree hf ha
      | variant kind ctor fields =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtoms env fields <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtoms_oFree hf ha
      | field value index =>
        simp only [evalRhs]
        cases hv : evalAtom env value with
        | none => exact RunOk.stuck w
        | some v =>
          cases v with
          | record r fs =>
            have hfs := evalAtom_oFree hf hv
            simp only [Val.OFree] at hfs
            refine RunOk.ret w fun x hx => ?_
            cases hi : fs[index]? <;> simp [hi, Option.elim] at hx
            subst hx
            exact (Val.oFreeAll_iff fs).mp hfs _ (List.mem_of_getElem? hi)
          | _ => exact RunOk.stuck w
      | list items =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtoms_oFree hf ha
      | none => exact RunOk.done w (by simp [Val.OFree])
      | some a =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtom_oFree hf ha
      | first a =>
        simp only [evalRhs]
        cases hv : evalAtom env a with
        | none => exact RunOk.stuck w
        | some v =>
          cases v with
          | list items =>
            have his := evalAtom_oFree hf hv
            simp only [Val.OFree] at his
            cases items with
            | nil => exact RunOk.done w (by simp [Val.OFree])
            | cons x xs => exact RunOk.done w ((Val.oFreeAll_iff _).mp his x (by simp))
          | _ => exact RunOk.stuck w
      | elemAt xs i =>
        simp only [evalRhs]
        cases hv : evalAtom env xs with
        | none => cases evalAtom env i <;> exact RunOk.stuck w
        | some v =>
          cases evalAtom env i with
          | none => cases v <;> exact RunOk.stuck w
          | some k =>
            cases v with
            | list items =>
              cases k with
              | int n =>
                have his := evalAtom_oFree hf hv
                simp only [Val.OFree] at his
                refine RunOk.done w ?_
                rcases atIndex_cases items n with h | ⟨x, hx, h⟩
                · rw [h]; simp [Val.OFree]
                · rw [h]; exact (Val.oFreeAll_iff _).mp his x hx
              | _ => exact RunOk.stuck w
            | _ => cases k <;> exact RunOk.stuck w
      | orElse o' d =>
        simp only [evalRhs]
        cases ho' : evalAtom env o' with
        | none => cases evalAtom env d <;> exact RunOk.stuck w
        | some v =>
          cases hd : evalAtom env d with
          | none => cases v <;> exact RunOk.stuck w
          | some x =>
            cases v with
            | some y =>
              have hy := evalAtom_oFree hf ho'
              simp only [Val.OFree] at hy
              exact RunOk.done w hy
            | none => exact RunOk.done w (evalAtom_oFree hf hd)
            | _ => exact RunOk.stuck w
      | isOk r =>
        simp only [evalRhs]
        cases evalAtom env r with
        | none => exact RunOk.stuck w
        | some v =>
          cases v with
          | ok _ => exact RunOk.done w (by simp [Val.OFree])
          | err _ => exact RunOk.done w (by simp [Val.OFree])
          | _ => exact RunOk.stuck w
      | mkOk a =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtom_oFree hf ha
      | mkErr a =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtom_oFree hf ha
      | lambda params body =>
        refine RunOk.done w ?_
        simp only [Val.OFree]
        rcases hg with ⟨h1, h2⟩ | ⟨hc, _, _⟩
        · exact ⟨by simpa [Rhs.reach] using h1, by simpa [Rhs.reach] using h2, hf⟩
        · have := closureFree_sound ho hSmay (by simpa [Rhs.condOk] using hc)
          exact ⟨this.1, this.2, hf⟩
      | funRef g targs =>
        simp only [evalRhs]
        cases hfd : p.find g with
        | none => exact RunOk.stuck w
        | some fn =>
          refine RunOk.done w ?_
          simp only [Val.OFree]
          have hSg : S g := by
            rcases hg with ⟨_, h2⟩ | ⟨hc, _, _⟩
            · exact h2 g (by simp [Rhs.reach])
            · simp only [Rhs.condOk, List.all_eq_true, Bool.not_eq_true'] at hc
              exact hSmay g (hc o ho)
          have hfn := hS g fn hfd hSg
          exact ⟨hfn.1, hfn.2, trivial⟩
      | declassify pol a =>
        simp only [evalRhs]
        cases hpo : p.policy pol with
        | none => exact RunOk.stuck w
        | some po =>
          cases hav : evalAtom env a with
          | none => exact RunOk.stuck w
          | some v =>
            dsimp only
            cases hfd : p.find po.via with
            | none => exact RunOk.stuck w
            | some fn =>
              simp only []
              split
              · have h₂ := hfree w (bindParams fn.params [v] []) fn.body
                  (bindParams_oFree _ (by simp only [Val.OFreeAll]; exact ⟨evalAtom_oFree hf hav, trivial⟩) trivial)
                  (hS po.via fn hfd (hvia pol po hpo)).1 (hS po.via fn hfd (hvia pol po hpo)).2
                generalize eval H n p w (bindParams fn.params [v] []) fn.body = res at h₂ ⊢
                obtain ⟨t, r, w'⟩ := res
                -- A release is no performance.
                have hrel : ∀ (x : Val), ∀ ev ∈ t ++ [Event.release pol x], PerfEv o ev → Q := by
                  intro x ev hev hp
                  rcases List.mem_append.mp hev with hev | hev
                  · exact h₂.1 ev hev hp
                  · simp only [List.mem_singleton] at hev; subst hev; exact hp.elim
                cases r with
                | ok x => exact ⟨hrel x, fun _ h => by cases h; exact h₂.2.1 x rfl, (fun _ _ _ h => nomatch h)⟩
                | failed c => exact ⟨hrel _, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
                | raised k c ps => exact ⟨h₂.1, (fun _ h => nomatch h), h₂.2.2⟩
                | stuck | outOfFuel | aborted => exact ⟨h₂.1, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
              · exact RunOk.stuck w
      | map xs f =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact RunOk.stuck w
        | some xv =>
          cases hfv : evalAtom env f with
          | none => cases xv <;> exact RunOk.stuck w
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_oFree hf hx
              simp only [Val.OFree] at hi
              exact walkMap_ok (fun w' v hv => happ w' fv [v] (evalAtom_oFree hf hfv)
                (by simp only [Val.OFreeAll]; exact ⟨hv, trivial⟩)) w items hi
            | _ => exact RunOk.stuck w
      | filter xs keep =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact RunOk.stuck w
        | some xv =>
          cases hfv : evalAtom env keep with
          | none => cases xv <;> exact RunOk.stuck w
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_oFree hf hx
              simp only [Val.OFree] at hi
              exact walkFilter_ok (fun w' v hv => happ w' fv [v] (evalAtom_oFree hf hfv)
                (by simp only [Val.OFreeAll]; exact ⟨hv, trivial⟩)) w items hi
            | _ => exact RunOk.stuck w
      | firstWhere xs keep =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact RunOk.stuck w
        | some xv =>
          cases hfv : evalAtom env keep with
          | none => cases xv <;> exact RunOk.stuck w
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_oFree hf hx
              simp only [Val.OFree] at hi
              exact walkFirst_ok (fun w' v hv => happ w' fv [v] (evalAtom_oFree hf hfv)
                (by simp only [Val.OFreeAll]; exact ⟨hv, trivial⟩)) w items hi
            | _ => exact RunOk.stuck w
      | fold xs init f =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact RunOk.stuck w
        | some xv =>
          cases hacc : evalAtom env init with
          | none => cases xv <;> exact RunOk.stuck w
          | some acc =>
            cases hfv : evalAtom env f with
            | none => cases xv <;> exact RunOk.stuck w
            | some fv =>
              cases xv with
              | list items =>
                have hi := evalAtom_oFree hf hx
                simp only [Val.OFree] at hi
                exact walkFold_ok (fun w' a v ha hv => happ w' fv [a, v] (evalAtom_oFree hf hfv)
                  (by simp only [Val.OFreeAll]; exact ⟨ha, hv, trivial⟩)) w acc items
                  (evalAtom_oFree hf hacc) hi
              | _ => exact RunOk.stuck w
      | parse s =>
        simp only [evalRhs]
        cases evalAtom env s with
        | none => exact RunOk.stuck w
        | some v =>
          cases v with
          | text t => exact parseInt_ok w t
          | _ => exact RunOk.stuck w
      | checked a pred code =>
        simp only [evalRhs]
        cases ha : evalAtom env a with
        | none => exact RunOk.stuck w
        | some v => exact checkRun_ok w (evalAtom_oFree hf ha)
      | eq a b =>
        simp only [evalRhs]
        cases evalAtom env a with
        | none => exact RunOk.stuck w
        | some va =>
          cases evalAtom env b with
          | none => exact RunOk.stuck w
          | some vb => exact RunOk.done w (by simp [Val.OFree])
      | toText a =>
        simp only [evalRhs]
        cases evalAtom env a with
        | none => exact RunOk.stuck w
        | some v => exact RunOk.done w (by simp [Val.OFree])
      | envGet name =>
        simp only [evalRhs]
        cases evalAtom env name with
        | none => exact RunOk.stuck w
        | some v =>
          cases v with
          | text s =>
            refine RunOk.done w ?_
            cases w.env s <;> simp [Option.elim, Val.OFree]
          | _ => exact RunOk.stuck w
      | tuple items =>
        refine RunOk.ret w fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.OFree]
        exact evalAtoms_oFree hf ha
      | append xs item =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact RunOk.stuck w
        | some xv =>
          cases hi : evalAtom env item with
          | none => cases xv <;> exact RunOk.stuck w
          | some v =>
            cases xv with
            | list items =>
              have hl := evalAtom_oFree hf hx
              have hv := evalAtom_oFree hf hi
              simp only [Val.OFree, Val.oFreeAll_iff] at hl ⊢
              refine RunOk.done w ?_
              simp only [Val.OFree, Val.oFreeAll_iff, List.mem_append, List.mem_singleton]
              rintro y (hy | rfl)
              · exact hl y hy
              · exact hv
            | _ => exact RunOk.stuck w
      | set xs index item =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact RunOk.stuck w
        | some xv =>
          cases evalAtom env index with
          | none => cases xv <;> exact RunOk.stuck w
          | some iv =>
            cases hi : evalAtom env item with
            | none => cases xv <;> cases iv <;> exact RunOk.stuck w
            | some v =>
              cases xv with
              | list items =>
                cases iv with
                | int i =>
                  simp only
                  split
                  · have hl := evalAtom_oFree hf hx
                    have hv := evalAtom_oFree hf hi
                    simp only [Val.OFree, Val.oFreeAll_iff] at hl
                    refine RunOk.done w ?_
                    simp only [Val.OFree, Val.OFreeAll, Val.oFreeAll_iff, and_true]
                    intro y hy
                    rcases List.mem_or_eq_of_mem_set hy with hy | rfl
                    · exact hl y hy
                    · exact hv
                  · exact RunOk.abort w
                | _ => exact RunOk.stuck w
              | _ => cases iv <;> exact RunOk.stuck w
    · intro w env cs K e hf hg
      exact eval_good_expr hS n ihR ihE w env cs K e hf hg

end Invariant

/-! ### Trace safety -/

mutual
  theorem Val.oFree_of_firstOrder {p : Program} {o : String} {S : String → Prop} :
      ∀ (v : Val), v.FirstOrder → Val.OFree p o S v
    | .closure _ _ _, h => h.elim
    | .list items, h => Val.oFreeAll_of_firstOrder items h
    | .some v, h | .ok v, h | .err v, h => Val.oFree_of_firstOrder v h
    | .record _ fs, h | .variant _ _ fs, h | .tuple fs, h => Val.oFreeAll_of_firstOrder fs h
    | .int _, _ | .bool _, _ | .text _, _ | .unit, _ | .none, _ => trivial

  theorem Val.oFreeAll_of_firstOrder {p : Program} {o : String} {S : String → Prop} :
      ∀ (vs : List Val), Val.FirstOrderAll vs → Val.OFreeAll p o S vs
    | [], _ => trivial
    | v :: vs, h => ⟨Val.oFree_of_firstOrder v h.1, Val.oFreeAll_of_firstOrder vs h.2⟩
end

/-- A response of `Http` (H4) is free of `o`: it holds no closure. -/
theorem isResponse_oFree {p : Program} {o : String} {S : String → Prop} {v : Val} (h : v.IsResponse) :
    Val.OFree p o S v := by
  unfold Val.IsResponse at h
  split at h
  · rename_i hs _
    simp only [Val.OFree, Val.OFreeAll, and_true, true_and]
    rw [Val.oFreeAll_iff]
    intro x hx
    obtain ⟨n, y, rfl⟩ := h x hx
    simp [Val.OFree, Val.OFreeAll]
  · exact h.elim

/-- **Trace safety** (ADR 0092 rule 7), for every outcome of the run: a result, a failure,
`.aborted`, `.stuck` or `.outOfFuel`. A run of a function of a program that passes the checks,
on first-order arguments where the function's condition on `o` is false, never performs `o`: not
in its body, not in a callee, a handler or a closure. -/
theorem trace_safety {H : Host} {p : Program} {f : Fun} {o : String} {C : Cond}
    (hts : p.traceSafeB = true) (h4 : H.RespTyped) (hf : f ∈ p.funs) (hc : f.conds.lookup o = some C)
    (w : World H) (n : Nat) (vs : List Val) (hvs : Val.FirstOrderAll vs)
    (hfalse : C.eval (bindParams f.params vs []) = false) :
    ¬ Performs o (eval H n p w (bindParams f.params vs []) f.body).1 := by
  simp only [Program.traceSafeB, Bool.and_eq_true, List.all_eq_true] at hts
  obtain ⟨⟨hchk, hclosed⟩, hpol⟩ := hts
  have ho : o ∈ p.condOps := by
    simp only [Program.condOps, List.mem_flatMap, List.mem_map]
    exact ⟨f, hf, (o, C), mem_of_lookup hc, rfl⟩
  have hfind : ∀ {g fn}, p.find g = some fn → fn ∈ p.funs ∧ fn.name = g := by
    intro g fn h
    unfold Program.find at h
    exact ⟨List.mem_of_find?_eq_some h, by simpa using List.find?_some h⟩
  let S : String → Prop := fun g => mayPerform p g o = false
  have hS : ∀ g fn, p.find g = some fn → S g →
      (fn.body.reach p).1.contains o = false ∧ ∀ h ∈ (fn.body.reach p).2, S h := by
    intro g fn hfd hg
    obtain ⟨hmem, hname⟩ := hfind hfd
    have hcl := hclosed o ho
    simp only [closedB, List.all_eq_true, Bool.or_eq_true, Bool.and_eq_true, Bool.not_eq_true'] at hcl
    rcases hcl fn hmem with h | ⟨h1, h2⟩
    · simp only [S] at hg; rw [hname] at h; rw [hg] at h; exact absurd h (by simp)
    · exact ⟨h1, fun h hh => by simpa [S] using h2 h hh⟩
  have hok : ∀ g fn, p.find g = some fn → fn.body.condOk p fn.conds {} = true :=
    fun _ fn hfd => hchk fn (hfind hfd).1
  have hvia : ∀ pol po, p.policy pol = some po → S po.via := by
    intro pol po hpo
    unfold Program.policy at hpo
    have := hpol po (List.mem_of_find?_eq_some hpo)
    simp only [List.all_eq_true, Bool.not_eq_true'] at this
    exact this o ho
  have hresp : ∀ m vs s v s', H.request m vs s = (.ok v, s') → Val.OFree p o S v :=
    fun m vs s v s' h => isResponse_oFree (h4 m vs s s' v h)
  let Q : Prop := C.eval (bindParams f.params vs []) = true
  have hgood : GoodE p o S Q f.conds {} (bindParams f.params vs []) f.body :=
    .inr ⟨hchk f hf, KHolds.empty, fun h => by simpa [Q, hc] using h⟩
  have hrun := (eval_good (Q := Q) hS (fun _ h => h) hok hvia ho hresp n).2 w _ f.conds {} f.body
    (bindParams_oFree f.params (Val.oFreeAll_of_firstOrder vs hvs) (by simp [Val.OFreeEnv])) hgood
  rintro ⟨ev, hev, hp⟩
  have := hrun.1 ev hev hp
  simp only [Q, hfalse] at this
  exact Bool.false_ne_true this

end Una.F1
