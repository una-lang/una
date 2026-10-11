/-
The unary half's theorem for the generic-free part of F1: a run keeps to its labeled typing
(`unary_sound`).
-/
import UnaFormal.F1.InstVal
import UnaFormal.F1.B1

namespace Una.F1

/-- A symbol-free label is the concrete label it holds. -/
theorem LExpr.eq_ofLabel_of_symFree {e : LExpr} (h : e.SymFree) : e = .ofLabel e.base := by
  obtain ⟨b, pl, st⟩ := e
  obtain ⟨h1, h2⟩ := h
  simp only at h1 h2
  subst h1 h2
  rfl

/-! ### At an instance -/

/-- The bindings of a `case` on a sum (#512): its fields, typed, in front of the environment. -/
theorem zip_ltyped {p : Program} {env : Env} {Γ : LCtx} (henv : LEnvTy p env Γ) :
    ∀ {xs : List String} {vs : List Val} {ts : List LTy}, LValsTy p vs ts → ts.length = xs.length →
      LEnvTy p (xs.zip vs ++ env) (xs.zip ts ++ Γ)
  | [], _, _, _, _ => by simpa using henv
  | _ :: _, _, _, .nil, h => by simp at h
  | _ :: _, _, _, .cons hv hvs, h => by
    simp only [List.zip_cons_cons, List.cons_append]
    exact .cons hv (zip_ltyped henv hvs (by simpa using h))

theorem LCtx.lsub_zip (rd : String → Label) (σ : List (String × LTy)) (xs : List String)
    (ts : List LTy) : LCtx.lsub rd σ (xs.zip ts) = xs.zip (ts.map (LTy.lsubst rd σ)) := by
  simp only [LCtx.lsub, List.zip_map_right]
  rfl

variable {p : Program} {H : Host}

theorem walkMap_uok {app : World H → Val → Run H} {A : Event → Prop} {F : FailCond}
    {a b : LTy} (l : LExpr)
    (happ : ∀ w v, LValTy p v a → UOk A (fun r => LValTy p r b) F w (app w v)) :
    ∀ {items : List Val} (w : World H), LValsAll p items a →
      UOk A (fun r => LValTy p r (.list b l)) F w (walkMap app w items)
  | [], _, _ => uok_done (.list .nil)
  | v :: vs, w, .cons hv hvs => by
    simp only [walkMap]
    refine andThen_uok (happ w v hv) (fun w r hr => ?_)
    refine andThen_uok (walkMap_uok l happ w hvs) (fun w rest hrest => ?_)
    cases hrest with
    | list hrs => exact uok_done (.list (.cons hr hrs))

theorem walkFilter_uok {app : World H → Val → Run H} {A : Event → Prop} {F : FailCond}
    {a : LTy} {lb : LExpr} (l : LExpr)
    (happ : ∀ w v, LValTy p v a → UOk A (fun r => LValTy p r (.bool lb)) F w (app w v)) :
    ∀ {items : List Val} (w : World H), LValsAll p items a →
      UOk A (fun r => LValTy p r (.list a l)) F w (walkFilter app w items)
  | [], _, _ => uok_done (.list .nil)
  | v :: vs, w, .cons hv hvs => by
    simp only [walkFilter]
    refine andThen_uok (happ w v hv) (fun w b hb => ?_)
    cases hb with
    | @bool keep =>
      refine andThen_uok (walkFilter_uok l happ w hvs) (fun w rest hrest => ?_)
      cases hrest with
      | list hrs =>
        cases keep
        · exact uok_done (.list hrs)
        · exact uok_done (.list (.cons hv hrs))

theorem walkFirst_uok {app : World H → Val → Run H} {A : Event → Prop} {F : FailCond}
    {a : LTy} {lb : LExpr} (l : LExpr)
    (happ : ∀ w v, LValTy p v a → UOk A (fun r => LValTy p r (.bool lb)) F w (app w v)) :
    ∀ {items : List Val} (w : World H), LValsAll p items a →
      UOk A (fun r => LValTy p r (.option a l)) F w (walkFirst app w items)
  | [], _, _ => uok_done .none
  | v :: vs, w, .cons hv hvs => by
    simp only [walkFirst]
    refine andThen_uok (happ w v hv) (fun w b hb => ?_)
    cases hb with
    | @bool keep =>
      cases keep
      · exact walkFirst_uok l happ w hvs
      · exact uok_done (.some hv)

theorem walkFold_uok {app : World H → Val → Val → Run H} {A : Event → Prop} {F : FailCond}
    {a b : LTy}
    (happ : ∀ w acc v, LValTy p acc b → LValTy p v a →
      UOk A (fun r => LValTy p r b) F w (app w acc v)) :
    ∀ {items : List Val} (w : World H) {acc : Val}, LValTy p acc b → LValsAll p items a →
      UOk A (fun r => LValTy p r b) F w (walkFold app w acc items)
  | [], _, _, hacc, _ => uok_done hacc
  | v :: vs, w, _, hacc, .cons hv hvs => by
    simp only [walkFold]
    exact andThen_uok (happ w _ v hacc hv) (fun w acc' hacc' => walkFold_uok happ w hacc' hvs)

/-- **A loop keeps its state typed** (T-Loop's invariant): when every iteration maps a typed state
to a tuple of the same types, so does the loop, by induction on the iterations. -/
theorem walkLoop_uok {body : World H → Val → List Val → Run H} {A : Event → Prop} {F : FailCond}
    {a : LTy} {ts : List LTy}
    (hbody : ∀ w v st, LValTy p v a → LValsTy p st ts →
      UOk A (fun r => ∃ st', r = .tuple st' ∧ LValsTy p st' ts) F w (body w v st)) :
    ∀ {items : List Val} (w : World H) {st : List Val}, LValsTy p st ts → (∀ v ∈ items, LValTy p v a) →
      UOk A (fun r => ∃ st', r = .tuple st' ∧ LValsTy p st' ts) F w (walkLoop body w st items)
  | [], _, _, hst, _ => uok_done ⟨_, rfl, hst⟩
  | v :: vs, w, st, hst, h => by
    simp only [walkLoop]
    refine andThen_uok (hbody w v st (h v List.mem_cons_self) hst) (fun w r hr => ?_)
    obtain ⟨st', rfl, hst'⟩ := hr
    simp only
    rw [ite_eq_left (by rw [LValsTy.length hst', LValsTy.length hst])]
    exact walkLoop_uok hbody w hst' (fun u hu => h u (List.mem_cons_of_mem _ hu))

theorem LValsAll.mem : ∀ {items : List Val} {t : LTy}, LValsAll p items t → ∀ v ∈ items, LValTy p v t
  | _, _, .nil => fun _ h => nomatch h
  | _, _, .cons hv hvs => fun u hu => by
    rcases List.mem_cons.mp hu with rfl | hu
    · exact hv
    · exact LValsAll.mem hvs u hu

theorem LValsAll.append_one : ∀ {items : List Val} {t : LTy} {v : Val}, LValsAll p items t →
    LValTy p v t → LValsAll p (items ++ [v]) t
  | _, _, _, .nil, hv => .cons hv .nil
  | _, _, _, .cons h hs, hv => .cons h (LValsAll.append_one hs hv)

theorem LValsAll.set : ∀ {items : List Val} {t : LTy} {v : Val} (i : Nat), LValsAll p items t →
    LValTy p v t → LValsAll p (items.set i v) t
  | _, _, _, _, .nil, _ => .nil
  | _, _, _, 0, .cons _ hs, hv => .cons hv hs
  | _, _, _, i + 1, .cons h hs, hv => .cons h (LValsAll.set i hs hv)

/-- The generic-free part of F1: no function has type parameters, and there is no policy. -/
structure Program.GenericFree (p : Program) : Prop where
  funs : ∀ fn ∈ p.funs, fn.tparams = []
  policies : p.policies = []

/-- Without policies, every function passes T-Fun. -/
theorem funOk_of_find (hwf : p.LWellFormed) (hpol : p.policies = []) {f : String} {fn : Fun}
    (h : p.find f = some fn) : p.FunOk fn := by
  have hmem := find_mem h
  rcases hwf.funs fn hmem with hok | ⟨⟨po, hpo, _⟩, _⟩
  · exact hok
  · rw [hpol] at hpo
    nomatch hpo

/-- A function that is not the `via` function of a policy passes T-Fun (option B1). -/
theorem funOk_of_nvia (hwf : p.LWellFormed) {f : String} {fn : Fun} (hnv : ¬ p.IsVia f)
    (h : p.find f = some fn) : p.FunOk fn := by
  have hmem := find_mem h
  have hname : fn.name = f := by
    have := List.find?_some h
    simpa using this
  rcases hwf.funs fn hmem with hok | ⟨hv, _⟩
  · exact hok
  · exact absurd (hname ▸ hv) hnv

section
variable {σ : List (String × LTy)}

theorem latom_ltypedσ {env : Env} {Γ : LCtx} {a : Atom} {t : LTy}
    (henv : LEnvTy p env (Γ.lsub p.recordDeep σ)) (h : LAtomTy Γ a t) :
    ∃ v, evalAtom env a = some v ∧ LValTy p v (t.lsubst p.recordDeep σ) := by
  cases h with
  | var hx => exact lookup_ltyped henv (by rw [lctxLookup_lsub, hx]; rfl)
  | int => exact ⟨_, rfl, .int⟩
  | bool => exact ⟨_, rfl, .bool⟩
  | text => exact ⟨_, rfl, .text⟩
  | unit => exact ⟨_, rfl, .unit⟩

theorem latoms_ltypedσ {env : Env} {Γ : LCtx} {as : List Atom} {ts : List LTy}
    (henv : LEnvTy p env (Γ.lsub p.recordDeep σ)) (h : LAtomsTy Γ as ts) :
    ∃ vs, evalAtoms env as = some vs ∧ LValsTy p vs (ts.map (LTy.lsubst p.recordDeep σ)) := by
  induction h with
  | nil => exact ⟨[], rfl, .nil⟩
  | cons ha _ ih =>
    obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
    obtain ⟨vs, hvs, hvst⟩ := ih
    exact ⟨v :: vs, by simp [evalAtoms, hv, hvs], .cons hvt hvst⟩

/-- A value of a type whose structure is `Text` is a text. -/
theorem LValTy.text_of {v : Val} {t : LTy} (hv : LValTy p v t) (ht : t.erase = .text) :
    ∃ s, v = .text s := by
  cases hv <;> simp_all [LTy.erase]

/-- Values of types that erase to `Text` are texts, so first-order. -/
theorem LValsTy.firstOrder_texts : ∀ {vs : List Val} {ts : List LTy}, LValsTy p vs ts →
    (∀ t ∈ ts, t.erase = .text) → Val.FirstOrderAll vs
  | _, _, .nil, _ => by simp [Val.FirstOrderAll]
  | _, _, .cons hv hvs, ht => by
    simp only [Val.FirstOrderAll]
    obtain ⟨s, rfl⟩ := hv.text_of (ht _ (by simp))
    exact ⟨by simp [Val.FirstOrder], LValsTy.firstOrder_texts hvs fun t h => ht t (by simp [h])⟩

/-- A value of a `Header` record type is first-order, in a program that holds `Http`'s kinds. -/
theorem LValTy.firstOrder_header (hr : p.HttpRecords) (hd : ∀ k, p.record k = Option.none ∨ p.sum k = Option.none)
    {v : Val} {t : LTy} (hv : LValTy p v t)
    (ht : t.erase = .record "prelude.Header") : v.FirstOrder := by
  obtain ⟨⟨hrec, hh, hhf⟩, _⟩ := hr
  cases hv with
  | @record r fs rec' l hrec' hfs =>
    simp only [LTy.erase, Ty.record.injEq] at ht
    subst ht
    rw [hh] at hrec'
    cases hrec'
    simp only [Val.FirstOrder]
    refine LValsTy.firstOrder_texts hfs fun t ht => ?_
    have := List.mem_map_of_mem (f := LTy.erase) ht
    simp only [Record.fieldTys] at hhf
    rw [hhf] at this
    simpa using this
  | variant hs _ _ =>
    simp only [LTy.erase, Ty.record.injEq] at ht
    subst ht
    rcases hd "prelude.Header" with h | h <;> simp_all
  | _ => simp [LTy.erase] at ht

/-- Values of a `Header` record type, in a list, are first-order. -/
theorem LValsAll.firstOrder_header (hr : p.HttpRecords) (hd : ∀ k, p.record k = Option.none ∨ p.sum k = Option.none) :
    ∀ {vs : List Val} {t : LTy},
    LValsAll p vs t → t.erase = .record "prelude.Header" → Val.FirstOrderAll vs
  | _, _, .nil, _ => by simp [Val.FirstOrderAll]
  | _, _, .cons hx hxs, ht => by
    simp only [Val.FirstOrderAll]
    exact ⟨LValTy.firstOrder_header hr hd hx ht, LValsAll.firstOrder_header hr hd hxs ht⟩

/-- **A request's arguments are first-order** (M5c): values of `Http`'s parameter types, `Text`,
`List[Header]` and `Option[Text]`, hold no closure. -/
theorem LValTy.firstOrder_httpArg (hr : p.HttpRecords)
    (hd : ∀ k, p.record k = Option.none ∨ p.sum k = Option.none) {v : Val} {t : LTy} (hv : LValTy p v t)
    (ht : t.erase = .text ∨ t.erase = .option .text ∨ t.erase = .list (.record "prelude.Header")) :
    v.FirstOrder := by
  rcases ht with ht | ht | ht
  · obtain ⟨s, rfl⟩ := hv.text_of ht
    simp [Val.FirstOrder]
  · cases hv with
    | none => simp [Val.FirstOrder]
    | some hv' =>
      simp only [LTy.erase, Ty.option.injEq] at ht
      obtain ⟨s, rfl⟩ := hv'.text_of ht
      simp [Val.FirstOrder]
    | _ => simp [LTy.erase] at ht
  · cases hv with
    | @list items t' l hall =>
      simp only [LTy.erase, Ty.list.injEq] at ht
      simp only [Val.FirstOrder]
      exact LValsAll.firstOrder_header hr hd hall ht
    | _ => simp [LTy.erase] at ht

theorem LValsTy.firstOrder_http (hr : p.HttpRecords)
    (hd : ∀ k, p.record k = Option.none ∨ p.sum k = Option.none) {m : String} :
    ∀ {vs : List Val} {ts : List LTy}, LValsTy p vs (ts.map (LTy.lsubst p.recordDeep σ)) →
      httpParams m = some (ts.map LTy.erase) → Val.FirstOrderAll vs := by
  intro vs ts hvs hm
  have hall : ∀ t ∈ ts, t.erase = .text ∨ t.erase = .option .text ∨
      t.erase = .list (.record "prelude.Header") := by
    intro t ht
    have hmem := List.mem_map_of_mem (f := LTy.erase) ht
    unfold httpParams at hm
    split at hm
    all_goals first
      | (simp only [Option.some.injEq] at hm; rw [← hm] at hmem; simp at hmem
         rcases hmem with h | h | h | h <;> simp [h])
      | (simp only [Option.some.injEq] at hm; rw [← hm] at hmem; simp at hmem
         rcases hmem with h | h | h <;> simp [h])
      | (cases hm)
  clear hm
  induction ts generalizing vs with
  | nil => cases hvs; simp [Val.FirstOrderAll]
  | cons t ts ih =>
    cases hvs with
    | cons hv hvs =>
      simp only [Val.FirstOrderAll]
      refine ⟨LValTy.firstOrder_httpArg hr hd hv ?_, ih hvs fun u hu => hall u (by simp [hu])⟩
      rw [LTy.erase_lsubst]
      rcases hall t (by simp) with h | h | h <;> rw [h] <;> simp [Ty.subst]

/-- A text, at any type that erases to `Text`. -/
theorem LValTy.text_erase {s : String} : ∀ {t : LTy}, t.erase = .text → LValTy p (.text s) t
  | .text _, _ => .text
  | .int _, h | .bool _, h | .unit _, h | .list _ _, h | .option _ _, h | .result _ _ _, h
  | .record _ _, h | .fn _ _ _ _ _ _, h | .var _ _, h => by simp [LTy.erase] at h

/-- A `Header`, at any type that erases to `Header`. -/
theorem LValTy.header_erase (hr : p.HttpRecords) {n x : String} :
    ∀ {t : LTy}, t.erase = .record "prelude.Header" → LValTy p (.record "prelude.Header" [.text n, .text x]) t
  | .record r _, h => by
    simp only [LTy.erase, Ty.record.injEq] at h
    subst h
    obtain ⟨⟨rec, hrec, hf⟩, _⟩ := hr
    refine .record hrec ?_
    simp only [Record.fieldTys] at hf
    rcases hfs : rec.fields with _ | ⟨g₁, _ | ⟨g₂, _ | ⟨g₃, rest⟩⟩⟩ <;> rw [hfs] at hf <;>
      simp at hf
    exact .cons (LValTy.text_erase hf.1) (.cons (LValTy.text_erase hf.2) .nil)
  | .text _, h | .int _, h | .bool _, h | .unit _, h | .list _ _, h | .option _ _, h
  | .result _ _ _, h | .fn _ _ _ _ _ _, h | .var _ _, h => by simp [LTy.erase] at h

/-- **A response is labeled-typed at `Response`** (H4), whatever its label. -/
theorem LValTy.response (hr : p.HttpRecords) {v : Val} (h : v.IsResponse) (l : LExpr) :
    LValTy p v (.record "prelude.Response" l) := by
  obtain ⟨_, ⟨rrec, hrr, hrf⟩⟩ := id hr
  unfold Val.IsResponse at h
  split at h
  · rename_i st hs body
    refine .record hrr ?_
    simp only [Record.fieldTys] at hrf
    rcases hfs : rrec.fields with _ | ⟨f₁, _ | ⟨f₂, _ | ⟨f₃, _ | ⟨f₄, rest⟩⟩⟩⟩ <;> rw [hfs] at hrf <;>
      simp at hrf
    obtain ⟨h₁, h₂, h₃⟩ := hrf
    refine .cons ?_ (.cons ?_ (.cons (LValTy.text_erase h₃) .nil))
    · cases f₁ <;> simp [LTy.erase] at h₁
      exact .int
    · cases f₂ <;> simp [LTy.erase] at h₂
      refine .list ?_
      induction hs with
      | nil => exact .nil
      | cons x xs ih =>
        obtain ⟨n, y, rfl⟩ := h x (by simp)
        exact .cons (LValTy.header_erase hr h₂) (ih fun z hz => h z (by simp [hz]))
  · exact h.elim

theorem LValsTy.flowsσ {vs : List Val} {ts us : List LTy} (h : LValsTy p vs (ts.map (LTy.lsubst p.recordDeep σ)))
    (hf : Flows p.top ts us) : LValsTy p vs (us.map (LTy.lsubst p.recordDeep σ)) := by
  have := Flows.lsubst p.recordDeep σ hf
  rw [LTy.lsubsts_eq, LTy.lsubsts_eq] at this
  exact LValsTy.flows h this

theorem LValsTy.lsubst_raise : ∀ {vs : List Val} {ps : List LTy} (e : LExpr),
    LValsTy p vs (ps.map (LTy.lsubst p.recordDeep σ)) →
      LValsTy p vs ((ps.map (LTy.raise e)).map (LTy.lsubst p.recordDeep σ))
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, _, .cons h hs => .cons (LValTy.lsubst_raise h) (LValsTy.lsubst_raise _ hs)

theorem LValsTy.lsubst_unraise : ∀ {vs : List Val} {ps : List LTy} {e : LExpr},
    LValsTy p vs ((ps.map (LTy.raise e)).map (LTy.lsubst p.recordDeep σ)) →
      LValsTy p vs (ps.map (LTy.lsubst p.recordDeep σ))
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, _, .cons h hs => .cons (LValTy.lsubst_unraise h) (LValsTy.lsubst_unraise hs)

theorem ofScalar_lsubst (rd : String → Label) (t : Ty) (l : LExpr) :
    (ofScalar t l).lsubst rd σ = ofScalar t (l.inst (σ.map (fun q => (q.1, q.2.deep rd)))) := by
  unfold ofScalar; split <;> rfl

theorem LValTy.ground_lsubst {v : Val} {u : LTy} (hu : u.Ground) :
    LValTy p v (u.lsubst p.recordDeep σ) ↔ LValTy p v u :=
  ⟨fun h => LValTy.flow h (Flow.lsubst_ground _ σ hu).1, fun h => LValTy.flow h (Flow.lsubst_ground _ σ hu).2⟩

theorem LValsTy.ground_lsubst : ∀ {vs : List Val} {us : List LTy}, (∀ u ∈ us, u.Ground) →
    LValsTy p vs (us.map (LTy.lsubst p.recordDeep σ)) → LValsTy p vs us
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, hg, .cons h hs =>
    .cons ((LValTy.ground_lsubst (hg _ List.mem_cons_self)).mp h)
      (LValsTy.ground_lsubst (fun u hu => hg u (List.mem_cons_of_mem _ hu)) hs)

/-- No name is both a record's and a sum's (#512). -/
theorem not_sum_of_record (hwf : p.LWellFormed) {k : String} {rec : Record} {sm : Sum}
    (hr : p.record k = some rec) (hs : p.sum k = some sm) : False := by
  rcases hwf.disjoint k with h | h <;> simp_all

/-- Fields of ground types, raised and instantiated, keep their values (#512). -/
theorem LValsTy.raise_lsubst_ground {l : LExpr} : ∀ {vs : List Val} {fs : List LTy},
    (∀ f ∈ fs, f.Ground) → LValsTy p vs fs →
      LValsTy p vs ((fs.map (LTy.raise l)).map (LTy.lsubst p.recordDeep σ))
  | [], [], _, .nil => .nil
  | _ :: _, _ :: _, hg, .cons h hs =>
    .cons (LValTy.lsubst_raise ((LValTy.ground_lsubst (hg _ List.mem_cons_self)).mpr h))
      (LValsTy.raise_lsubst_ground (fun f hf => hg f (List.mem_cons_of_mem _ hf)) hs)

theorem le_inst_join_left (s : List (String × LExpr)) (a b : LExpr) :
    p.le (a.inst s) ((a.join b).inst s) := LExpr.le_inst s (LExpr.le_join_left a b)

theorem Fails.union_left_inst {pc : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff} :
    Fails p pc (ε₁.inst s) → Fails p pc ((ε₁.union ε₂).inst s) := by
  refine Fails.mono (LExpr.le_refl pc) (fun l hl => ?_)
  simp only [Eff.inst, Option.map_eq_some_iff] at hl
  obtain ⟨l₁, h₁, rfl⟩ := hl
  obtain ⟨l', hl', hle⟩ := union_fails_left (p := p) (ε₂ := ε₂) l₁ h₁
  exact ⟨l'.inst s, by simp [Eff.inst, hl'], LExpr.le_inst s hle⟩

theorem Fails.union_right_inst {pc pc' : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (hpc : p.le pc pc') : Fails p pc' (ε₂.inst s) → Fails p pc ((ε₁.union ε₂).inst s) := by
  refine Fails.mono hpc (fun l hl => ?_)
  simp only [Eff.inst, Option.map_eq_some_iff] at hl
  obtain ⟨l₂, h₂, rfl⟩ := hl
  obtain ⟨l', hl', hle⟩ := union_fails_right (p := p) (ε₁ := ε₁) l₂ h₂
  exact ⟨l'.inst s, by simp [Eff.inst, hl'], LExpr.le_inst s hle⟩

theorem allowed_union_left_inst {pc : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff} :
    ∀ ev, Allowed p pc (ε₁.inst s) ev → Allowed p pc ((ε₁.union ε₂).inst s) ev :=
  Allowed.mono (LExpr.le_refl pc) union_op_left union_host_left (fun x h => union_rels_left x h)

theorem allowed_union_right_inst {pc pc' : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (hpc : p.le pc pc') : ∀ ev, Allowed p pc' (ε₂.inst s) ev → Allowed p pc ((ε₁.union ε₂).inst s) ev :=
  Allowed.mono hpc union_op_right union_host_right (fun x h => union_rels_right x h)

/-- At a call whose program counter's instance the function's bound allows. -/
theorem bound_allowed_inst {pc pc' : LExpr} {s : List (String × LExpr)} {o : Bool} {g : HostFx} {f : Bool}
    (hb : p.Bound pc' o g f) (hpc : p.le pc (pc'.inst s)) :
    ∀ ev, FnAllowed o g ev → Allowed p pc ((Eff.ofFn o g f).inst s) ev :=
  bound_allowed (hb.inst s) hpc

theorem bound_fails_inst {pc pc' : LExpr} {s : List (String × LExpr)} {o : Bool} {g : HostFx} {f : Bool}
    (hb : p.Bound pc' o g f) (hpc : p.le pc (pc'.inst s)) (hf : f = true) :
    Fails p pc ((Eff.ofFn o g f).inst s) := by
  refine ⟨LExpr.bot.inst s, by simp [Eff.ofFn, Eff.inst, hf], ?_⟩
  exact LExpr.le_trans hpc (LExpr.le_trans ((hb.inst s).2.2.2.2 hf).1
    (by rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]; exact LExpr.le_join_right _ _))

theorem bound_fails_instR {pc pc' : LExpr} {s : List (String × LExpr)} {o : Bool} {g : HostFx} {f : Bool}
    {rs : List String} (hb : p.Bound pc' o g f) (hpc : p.le pc (pc'.inst s)) (hf : f = true) :
    Fails p pc ((Eff.ofFnR o g f rs).inst s) :=
  bound_fails_inst hb hpc hf

theorem fnAllowed_of {pc : LExpr} {s : List (String × LExpr)} {ε : Eff} {o : Bool} {g : HostFx}
    (ho : ε.op = true → o = true) (hg : HostFx.Le ε.host g) (hr : ε.rels = []) :
    ∀ ev, Allowed p pc (ε.inst s) ev → FnAllowed o g ev
  | .out _, h => ho h.1
  | .read _, h => hg.1 h.1
  | .write _ _, h => hg.2.1 h.1
  | .req _ _, h => ⟨hg.2.2 h.1, h.2.2⟩
  | .release _ _, h => by have := h.1; simp [Eff.inst, hr] at this

/-- **The events of a call** (option B1): the callee's operations, which its bound allows, and
its releases, through policies it declares (`Fun.rels`), each above the program counter of the
call (`Program.RelBound`). The policies' `to` labels are concrete. -/
theorem call_allowed_inst {pc pc₀ pc' : LExpr} {s s' : List (String × LExpr)} {εb : Eff}
    {o : Bool} {g : HostFx} {f : Bool} {rs : List String} (hdst : ∀ po ∈ p.policies, po.dst.top.SymFree)
    (hb : p.Bound pc' o g f) (hrb : p.RelBound pc' rs) (hpc : p.le pc (pc'.inst s))
    (ho : εb.op = true → o = true) (hg : HostFx.Le εb.host g) (hr : ∀ x ∈ εb.rels, x ∈ rs) :
    ∀ ev, Allowed p pc₀ (εb.inst s') ev → Allowed p pc ((Eff.ofFnR o g f rs).inst s) ev
  | .out _, h => ⟨ho h.1, LExpr.le_trans hpc ((hb.inst s).1 (ho h.1)).1⟩
  | .read _, h => ⟨hg.1 h.1, LExpr.le_trans hpc ((hb.inst s).2.1 (hg.1 h.1)).1⟩
  | .write _ _, h => ⟨hg.2.1 h.1, LExpr.le_trans hpc ((hb.inst s).2.2.1 (hg.2.1 h.1)).1,
      fun ho => LExpr.le_trans hpc (((hb.inst s).2.2.1 (hg.2.1 h.1)).2.1 ho)⟩
  | .req _ _, h => ⟨hg.2.2 h.1, LExpr.le_trans hpc ((hb.inst s).2.2.2.1 (hg.2.2 h.1)).1, h.2.2⟩
  | .release pol _, ⟨hm, po, hpo, _, hv⟩ => by
    obtain ⟨po', hpo', hle, _⟩ := hrb pol (hr pol hm)
    rw [hpo] at hpo'
    cases hpo'
    refine ⟨hr pol hm, po, hpo, LExpr.le_trans hpc ?_, hv⟩
    have hsf := hdst po (List.mem_of_find?_eq_some hpo)
    rw [LExpr.eq_ofLabel_of_symFree hsf] at hle ⊢
    exact LExpr.le_of_le_inst_ofLabel (LExpr.le_inst s hle)

theorem flag_of_fails {pc : LExpr} {s : List (String × LExpr)} {ε : Eff} {f : Bool}
    (hf : ε.fails.isSome = true → f = true) : Fails p pc (ε.inst s) → f = true := by
  rintro ⟨l, hl, _⟩
  apply hf
  simp only [Eff.inst, Option.map_eq_some_iff] at hl
  obtain ⟨l', hl', _⟩ := hl
  simp [hl']

/-! ### What a failure may be (#512): a code, or a payload of a kind the effects raise -/

theorem FailsAs.union_left_inst {pc : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff} :
    (FailsAs p pc (ε₁.inst s)).Le (FailsAs p pc ((ε₁.union ε₂).inst s)) :=
  ⟨fun ⟨h, hc⟩ => ⟨Fails.union_left_inst h, by
      simp only [Eff.inst, Eff.union] at hc ⊢; simp [hc]⟩,
    fun _ _ _ ⟨h, hk, hp⟩ => ⟨Fails.union_left_inst h, by
      simp only [Eff.inst, Eff.union] at hk ⊢; exact List.mem_append_left _ hk, hp⟩⟩

theorem FailsAs.union_right_inst {pc pc' : LExpr} {s : List (String × LExpr)} {ε₁ ε₂ : Eff}
    (hpc : p.le pc pc') : (FailsAs p pc' (ε₂.inst s)).Le (FailsAs p pc ((ε₁.union ε₂).inst s)) :=
  ⟨fun ⟨h, hc⟩ => ⟨Fails.union_right_inst hpc h, by
      simp only [Eff.inst, Eff.union] at hc ⊢; simp [hc]⟩,
    fun _ _ _ ⟨h, hk, hp⟩ => ⟨Fails.union_right_inst hpc h, by
      simp only [Eff.inst, Eff.union] at hk ⊢; exact List.mem_append_right _ hk, hp⟩⟩

theorem FailsAs.mono_pc {pc pc' : LExpr} {ε : Eff} (hpc : p.le pc pc') :
    (FailsAs p pc' ε).Le (FailsAs p pc ε) :=
  ⟨fun ⟨h, hc⟩ => ⟨Fails.mono hpc (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h, hc⟩,
    fun _ _ _ ⟨h, hk, hp⟩ => ⟨Fails.mono hpc (fun l hl => ⟨l, hl, LExpr.le_refl l⟩) h, hk, hp⟩⟩

/-- A function value fails with a code only, at a call its bound allows. -/
theorem bound_failsAs_inst {pc pc' : LExpr} {s : List (String × LExpr)} {o : Bool} {g : HostFx}
    {f : Bool} (hb : p.Bound pc' o g f) (hpc : p.le pc (pc'.inst s)) :
    FailCond.Le ⟨f = true, fun _ _ _ => False⟩ (FailsAs p pc ((Eff.ofFn o g f).inst s)) :=
  ⟨fun (hf : f = true) => ⟨bound_fails_inst hb hpc hf, by simp [Eff.ofFn, Eff.inst, hf]⟩,
    fun _ _ _ h => h.elim⟩

/-- **A call fails as its callee declares**: with a code when it declares no payload kind, and with
a payload of a kind it declares (`Eff.ofCall`, T-Fun). -/
theorem call_failsAs_inst {pc pc₀ pc' : LExpr} {s s' : List (String × LExpr)} {fn : Fun} {εb : Eff}
    (hb : p.Bound pc' fn.op fn.host fn.fails) (hpc : p.le pc (pc'.inst s))
    (hfs : εb.fails.isSome = true → fn.fails = true) (hcodes : εb.codes = true → fn.raises = [])
    (hraises : ∀ k ∈ εb.raises, k ∈ fn.raises) :
    (FailsAs p pc₀ (εb.inst s')).Le (FailsAs p pc ((Eff.ofCall fn).inst s)) := by
  have hfl : ∀ {pc₁ : LExpr}, Fails p pc₁ (εb.inst s') → Fails p pc ((Eff.ofCall fn).inst s) :=
    fun h => bound_fails_instR (rs := fn.rels) hb hpc (flag_of_fails hfs h)
  refine ⟨fun ⟨h, hc⟩ => ⟨hfl h, ?_⟩, fun k _ _ ⟨h, hk, hp⟩ => ⟨hfl h, ?_, hp⟩⟩
  · have hf := flag_of_fails hfs h
    have hr := hcodes (by simpa [Eff.inst] using hc)
    simp [Eff.ofCall, Eff.ofFnR, Eff.ofFn, Eff.inst, hf, hr]
  · simpa [Eff.ofCall, Eff.inst] using hraises k (by simpa [Eff.inst] using hk)

end

/-- A ground type's top label is concrete. -/
theorem LTy.Ground.top_symFree : ∀ {t : LTy}, t.Ground → t.top.SymFree
  | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => h
  | .list _ _, h | .option _ _, h => h.2
  | .result _ _ _, h => h.2.2
  | .fn _ _ _ _ _ _, h => h.2.2
  | .tuple _ _, h => h.2
  | .var _ _, h => h.elim

/-- A value of a scalar type, labeled or not. -/
theorem lvalTy_of_scalar {v : Val} : ∀ {t : LTy}, t.erase.Scalar → ValTy p v t.erase → LValTy p v t
  | .int _, _, h | .bool _, _, h | .text _, _, h => by cases h <;> constructor
  | .unit _, hs, _ | .list _ _, hs, _ | .option _ _, hs, _ | .result _ _ _, hs, _ | .record _ _, hs, _
  | .fn _ _ _ _ _ _, hs, _ | .var _ _, hs, _ | .tuple _ _, hs, _ => by simp [LTy.erase, Ty.Scalar] at hs

theorem valTy_of_scalar {v : Val} : ∀ {t : LTy}, t.erase.Scalar → LValTy p v t → ValTy p v t.erase
  | .int _, _, h | .bool _, _, h | .text _, _, h => by cases h <;> constructor
  | .unit _, hs, _ | .list _ _, hs, _ | .option _ _, hs, _ | .result _ _ _, hs, _ | .record _ _, hs, _
  | .fn _ _ _ _ _ _, hs, _ | .var _ _, hs, _ | .tuple _ _, hs, _ => by simp [LTy.erase, Ty.Scalar] at hs

theorem Ty.Scalar.subst {t : Ty} (σ : List (String × Ty)) (h : t.Scalar) : t.subst σ = t := by
  cases t <;> simp_all [Ty.Scalar, Ty.subst]

/-- Below a concrete label, at every instance. -/
theorem le_inst_of_symFree {pc d : LExpr} (s : List (String × LExpr)) (hd : d.SymFree)
    (h : p.le pc d) : p.le (pc.inst s) d := by
  rw [LExpr.eq_ofLabel_of_symFree hd] at h ⊢
  exact LExpr.le_of_le_inst_ofLabel (LExpr.le_inst s h)

/-- **The unary half, at every ground instance.** A run of a body the labeled typing accepts,
in an environment of the instance of its context, keeps to the instance of its typing. -/
theorem unary_soundR (hwf : p.LWellFormed) (hb1 : p.B1WellFormed) (h4 : H.RespTyped) : ∀ n : Nat,
    (∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {r : Rhs}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LRhsTy p E Γ pc r t ε →
      UGood p (pc.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w (evalRhs H n p w env r)) ∧
    (∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LExprTy p E Γ pc e t ε →
      UGood p (pc.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w (eval H n p w env e))
  | 0 => ⟨fun _ _ _ => by simp [evalRhs, UGood, UOk, URes, WEff],
      fun _ _ _ => by simp [eval, UGood, UOk, URes, WEff]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := unary_soundR hwf hb1 h4 n
    have hdst : ∀ po ∈ p.policies, po.dst.top.SymFree := fun po h => (hb1.dst po h).1.top_symFree
    have hrun : ∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String}
        {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
        LExprTy p E Γ .bot e t ε →
        UGood p (LExpr.bot.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w
          (eval H n p w env e) :=
      fun hσ henv he => ihE hσ henv he
    constructor
    · intro σ w env Γ E pc r t ε hσ henv hr
      have hpc0 : ∀ x : LExpr, p.le (pc.inst (p.sd σ)) ((pc.join x).inst (p.sd σ)) :=
        fun x => le_inst_join_left _ pc x
      cases hr with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes] using hvt, WEff.self _ _⟩
      | prim has hp =>
        obtain ⟨vs, hvs, _⟩ := latoms_ltypedσ henv has
        simp only [evalRhs, hvs, Option.map_some, Option.getD_some, Option.bind_some]
        split
        · exact ⟨fun _ h => by simp at h, trivial, WEff.self _ _⟩
        · refine ⟨fun _ h => by simp at h, ?_, WEff.self _ _⟩
          cases he : evalPrim _ vs with
          | none => simp [Option.elim, URes]
          | some v =>
            simp only [Option.elim]
            show LValTy p v _
            rw [ofScalar_lsubst]
            exact prim_ltyped _ hp he
      | @call _ _ f targs args fn ts hf hnv hlen _ _ hg has hfl hb hrb =>
        obtain ⟨tb, εb, hbody, hflow, ho, hgo, hfs, _, hrs, hcd, hra⟩ := funOk_of_nvia hwf hnv hf
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        have hpg : ∀ u ∈ fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2), u.Ground := by
          intro u hu
          obtain ⟨q, hq, rfl⟩ := List.mem_map.mp hu
          exact LTy.lsubst_ground _ hτ hcov (hps q hq)
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        have ha := LValsTy.ground_lsubst hpg (LValsTy.flowsσ hvst hfl)
        have ha' : LValsTy p vs ((LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params).map Prod.snd) := by
          rw [LCtx.lsub_snd]; simpa [Program.lsub, List.map_map, Function.comp_def] using ha
        simp only [evalRhs, hf, hvs]
        rw [ite_eq_left (by rw [LValsTy.length ha, List.length_map])]
        have henv' := bindParams_ltyped ha' (LEnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        have henv'' : LEnvTy p (bindParams fn.params vs []) (LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params) := by
          simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using henv'
        refine UOk.mono (call_allowed_inst hdst hb hrb (LExpr.le_refl _) ho hgo hrs)
          (fun v hv => ?_)
          (call_failsAs_inst hb (LExpr.le_refl _) hfs hcd hra)
          (ihE hτ (by simpa using henv'') hbody)
        have hr := LTy.lsubst_ground p.recordDeep hτ hcov hres
        exact (LValTy.ground_lsubst hr).mpr (LValTy.flow hv (Flow.lsubst _ _ hflow))
      | apply hf has hfl hb =>
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hf
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        simp only [evalRhs, hfv, hvs]
        have hargs := LValsTy.lsubst_unraise (LValsTy.flowsσ hvst hfl)
        simp only [LTy.lsubst, LTy.lsubsts_eq] at hft
        refine UOk.mono (bound_allowed_inst hb (le_inst_join_left _ pc _))
          (fun v hv => LValTy.lsubst_raise hv)
          (bound_failsAs_inst hb (le_inst_join_left _ pc _))
          (applyWith_uok hrun hft hargs w)
      | @op _ _ line t hl ht hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hl
        cases t <;> simp [LTy.erase, Ty.Scalar] at ht
        all_goals
          simp only [LTy.lsubst] at hvt
          cases hvt
          simp only [evalRhs, hv, Val.line]
          refine ⟨fun ev hev => ?_, LValTy.unit, WEff.self _ _⟩
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))⟩
      | @read _ _ path t hu ht hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hu
        cases t <;> simp [LTy.erase] at ht
        simp only [LTy.lsubst] at hvt
        cases hvt
        rename_i _ s
        simp only [evalRhs, hv]
        cases H.read s w.fs with
        | some body =>
          refine ⟨fun ev hev => ?_, LValTy.text, WEff.self _ _⟩
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))⟩
        | none =>
          refine ⟨fun ev hev => ?_, ⟨⟨_, rfl, ?_⟩, rfl⟩, WEff.self _ _⟩
          · simp only [List.mem_singleton] at hev
            subst hev
            exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))⟩
          · refine LExpr.le_inst _ ?_
            unfold Program.fsFail
            split
            · exact LExpr.le_join_left _ _
            · exact LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _)
      | @write _ _ path content t c hu ht hc hct hle hst _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hu
        obtain ⟨cv, hcv, hcvt⟩ := latom_ltypedσ henv hc
        cases t <;> simp [LTy.erase] at ht
        cases c <;> simp [LTy.erase] at hct
        simp only [LTy.lsubst] at hvt hcvt
        cases hvt with
        | @text s _ =>
        cases hcvt with
        | @text cs _ =>
        simp only [evalRhs, hv, hcv]
        cases H.write s cs w.fs with
        | some fs' =>
          refine ⟨fun ev hev => ?_, LValTy.unit, fun h => by simp [Event.isWrite] at h, fun _ => rfl, rfl⟩
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _
            (LExpr.le_trans (LExpr.le_join_left _ _) hle))),
            fun ho => LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ (hst ho)))⟩
        | none =>
          refine ⟨fun ev hev => ?_, ⟨⟨_, rfl, ?_⟩, rfl⟩, WEff.self _ _⟩
          · simp only [List.mem_singleton] at hev
            subst hev
            exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _
            (LExpr.le_trans (LExpr.le_join_left _ _) hle))),
            fun ho => LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ (hst ho)))⟩
          · refine LExpr.le_inst _ ?_
            unfold Program.fsFail
            split
            · exact LExpr.le_join_left _ _
            · exact LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _)
      | @req _ _ m args ts has hts hr hle _ =>
        obtain ⟨vs, hvs, hvt⟩ := latoms_ltypedσ henv has
        have htx := LValsTy.firstOrder_http hr hwf.disjoint hvt hts
        simp only [evalRhs, hvs]
        generalize hq : H.request m vs w.srv = q
        obtain ⟨o, srv'⟩ := q
        cases o with
        | ok body =>
          refine ⟨fun ev hev => ?_, ?_, fun _ => rfl, fun h => by simp [Event.isReq] at h, rfl⟩
          rotate_left
          · simpa [LTy.lsubst] using LValTy.response hr (h4 m vs w.srv srv' body hq) _
          simp only [List.mem_singleton] at hev
          subst hev
          exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle)), htx⟩
        | error c =>
          refine ⟨fun ev hev => ?_, ⟨⟨_, rfl, ?_⟩, rfl⟩, fun _ => rfl, fun h => by simp [Event.isReq] at h, rfl⟩
          · simp only [List.mem_singleton] at hev
            subst hev
            exact ⟨rfl, LExpr.le_trans (hpc0 _) (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle)),
              htx⟩
          · refine LExpr.le_inst _ ?_
            unfold Program.reqFail
            split
            · exact LExpr.le_join_left _ _
            · exact LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _)
      | construct hrec has hfl =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        have := LValsTy.ground_lsubst (hwf.recordsGround _ (record_mem hrec)) (LValsTy.flowsσ hvst hfl)
        exact ⟨fun _ h => by simp [evalRhs] at h,
          by simpa [evalRhs, hvs, URes, LTy.lsubst] using LValTy.record hrec this, WEff.self _ _⟩
      | variant hs hf has hfl =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        have := LValsTy.ground_lsubst
          (fun u hu => (hwf.sums _ (sum_mem hs) _ (List.mem_of_getElem? hf) u hu).1)
          (LValsTy.flowsσ hvst hfl)
        exact ⟨fun _ h => by simp [evalRhs] at h,
          by simpa [evalRhs, hvs, URes, LTy.lsubst] using LValTy.variant hs hf this, WEff.self _ _⟩
      | field hv hrec hi =>
        obtain ⟨v, hv', hvt⟩ := latom_ltypedσ henv hv
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | record hrec' hvals =>
          rw [hrec] at hrec'
          cases hrec'
          obtain ⟨u, hu, hut⟩ := lfield_typed hvals hi
          have hg := hwf.recordsGround _ (record_mem hrec) _ (List.mem_of_getElem? hi)
          exact ⟨fun _ h => by simp [evalRhs, hv'] at h,
            by simpa [evalRhs, hv', hu, URes] using
              LValTy.lsubst_raise ((LValTy.ground_lsubst hg).mpr hut),
            by simp only [evalRhs, hv', hu]; exact WEff.self _ _⟩
        | variant hs _ _ => exact (not_sum_of_record hwf hrec hs).elim
      | list has hfl _ =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        refine ⟨fun _ h => by simp [evalRhs] at h, ?_, WEff.self _ _⟩
        simp only [evalRhs, hvs, Option.map, Option.elim, URes, LTy.lsubst]
        exact .list (LValsAll.of_flows hvst (fun u hu => by
          obtain ⟨u', hu', rfl⟩ := List.mem_map.mp hu
          exact Flow.lsubst _ _ (hfl u' hu')))
      | none _ => exact uok_done LValTy.none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes, LTy.lsubst] using LValTy.some hvt,
          WEff.self _ _⟩
      | first ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | list hall =>
          cases hall with
          | nil => simp only [evalRhs, hv]; exact uok_done LValTy.none
          | cons hx _ => simp only [evalRhs, hv]; exact uok_done (LValTy.some hx)
      | mkOk ha _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes, LTy.lsubst] using LValTy.ok hvt,
          WEff.self _ _⟩
      | mkErr ha _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [evalRhs] at h, by simpa [evalRhs, hv, URes, LTy.lsubst] using LValTy.err hvt,
          WEff.self _ _⟩
      | lambda hb hfl hrl hrs _ =>
        refine uok_done ?_
        simp only [LTy.lsubst]
        exact .closure hσ henv hb hfl hrl hrs (by rw [LTy.lsubsts_eq]; exact Flows.refl _) (Flow.refl _)
          id (HostFx.Le.refl _) id
      | @funRef _ _ f targs fn hf hnv hlen _ _ hg hfr hfr2 =>
        obtain ⟨tb, εb, hbody, hflow, ho, hgo, hfs, hfl, hrs, _, hra⟩ := funOk_of_nvia hwf hnv hf
        have hrl : εb.rels = [] :=
          List.eq_nil_iff_forall_not_mem.mpr (fun x hx => by simpa [hfr] using hrs x hx)
        have hrsn : εb.raises = [] :=
          List.eq_nil_iff_forall_not_mem.mpr (fun x hx => by simpa [hfr2] using hra x hx)
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        simp only [evalRhs, hf]
        refine uok_done ?_
        simp only [LTy.lsubst, Program.lsub]
        refine .closure (Γ := []) (E := fn.eqParams) hτ .nil (by simpa using hbody) hfl hrl hrsn ?_ ?_ ho hgo hfs
        · rw [LTy.lsubsts_eq]
          have : ∀ ts : List (String × LTy), (∀ q ∈ ts, q.2.ScopedIn fn.tparams) →
              Flows p.top ((ts.map (fun q => q.2.lsubst p.recordDeep (fn.tparams.zip targs))).map
                (LTy.lsubst p.recordDeep σ))
                ((ts.map Prod.snd).map (LTy.lsubst p.recordDeep (fn.tparams.zip targs))) := by
            intro ts hts
            induction ts with
            | nil => exact .nil
            | cons q qs ih =>
              exact .cons (Flow.lsubst_ground _ σ
                  (LTy.lsubst_ground _ hτ hcov (hts q List.mem_cons_self))).1
                (ih (fun q' hq' => hts q' (List.mem_cons_of_mem _ hq')))
          exact this fn.params hps
        · exact Flow.trans (Flow.lsubst _ _ hflow)
            (Flow.lsubst_ground _ σ (LTy.lsubst_ground _ hτ hcov hres)).2
      | @declassify _ _ pol a po fn t hpo hfn ha hflow hle _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        have hmem := policy_mem hpo
        obtain ⟨_, _, fn', hfn', hps, hres⟩ := hwf.wellFormed.policies _ hmem
        rw [hfn] at hfn'
        cases hfn'
        obtain ⟨fnv, hfnv, _, _, hpof, hpure, _⟩ := hb1.via po hmem
        rw [hfn] at hfnv
        cases hfnv
        obtain ⟨hdg, hds⟩ := hb1.dst po hmem
        have hsrc : (t.lsubst p.recordDeep σ).erase = po.src.erase := by
          rw [LTy.erase_lsubst, Flow.erase hflow, Ty.Scalar.subst _ (hb1.src po hmem)]
        have hlen := paramTys_length fn []
        rw [hps] at hlen
        simp only [evalRhs, hpo, hv, hfn]
        rw [ite_eq_left (by simpa using hlen)]
        have hvs : ValsTy p [v] (fn.paramTys []) := by
          rw [hps, ← hsrc]
          exact .cons (valTy_of_scalar (by rw [hsrc]; exact hb1.src po hmem) hvt) .nil
        have henv' := bindParams_typed hvs (paramTys_length fn []) (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        have hbody := body_typed hwf.wellFormed hfn []
        rw [Ty.subst_nil, hres] at hbody
        have hgood := (sound hwf.wellFormed h4 n).2 (w := w) henv' hbody
        -- The `via` body performs no event; it fails only if it declares `fails` (B1).
        have hq := (silent_run p w n).2 (bindParams fn.params [v] []) fn.body hpof
        have hnf : fn.fails = false → ∀ c,
            (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 ≠ .failed c :=
          fun hf => ((pure_run p w n).2 _ fn.body (hpure hf)).2.1
        -- A `via` body never fails with a payload (#512): it has no `raise` and calls nothing.
        have hnr := (noRaise_run p w n).2 (bindParams fn.params [v] []) fn.body hpof
        revert hgood hq hnf hnr
        generalize eval H n p w (bindParams fn.params [v] []) fn.body = run
        obtain ⟨tr, res, w'⟩ := run
        intro hgood hq hnf hnr
        obtain ⟨htr, hw⟩ := hq
        simp only at htr hw hnf
        subst htr hw
        cases res with
        | ok r =>
          simp only [Good] at hgood
          refine ⟨fun ev hev => ?_, ?_, WEff.self _ _⟩
          · simp only [List.nil_append, List.mem_singleton] at hev
            subst hev
            exact ⟨by simp [Eff.ofPolicy, Eff.inst], po, hpo,
              le_inst_of_symFree _ hdg.top_symFree hle, .inl hgood⟩
          · show LValTy p r (po.dst.lsubst p.recordDeep σ)
            refine lvalTy_of_scalar ?_ ?_
            · rw [LTy.erase_lsubst, Ty.Scalar.subst _ hds]; exact hds
            · rw [LTy.erase_lsubst, Ty.Scalar.subst _ hds]; exact hgood
        | failed c =>
          -- A failing `via` releases `Err(c)`, and the call fails at `pc ⊔ ℓ(to)`.
          refine ⟨fun ev hev => ?_, ?_, WEff.self _ _⟩
          · simp only [List.nil_append, List.mem_singleton] at hev
            subst hev
            exact ⟨by simp [Eff.ofPolicy, Eff.inst], po, hpo,
              le_inst_of_symFree _ hdg.top_symFree hle, .inr ⟨c, rfl⟩⟩
          · cases hf : fn.fails with
            | false => exact absurd rfl (hnf hf c)
            | true =>
              exact ⟨⟨(pc.join po.dst.top).inst (p.sd σ), by simp [Eff.ofPolicy, Eff.inst, hf],
                LExpr.le_inst _ (LExpr.le_join_left pc po.dst.top)⟩, by simp [Eff.ofPolicy, Eff.inst, hf]⟩
        | stuck => exact ⟨fun _ h => (nomatch h), trivial, WEff.self _ _⟩
        | outOfFuel | aborted => exact ⟨fun _ h => (nomatch h), trivial, WEff.self _ _⟩
        | raised k c ps => exact absurd rfl (hnr k c ps)
      | map hxs hf hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hf
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b : LExpr, p.le (pc.inst (p.sd σ)) (((pc.join a).join b).inst (p.sd σ)) :=
            fun _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _)) (fun v hv => by simpa [LTy.lsubst] using hv)
            (bound_failsAs_inst hb (hpc _ _)) ?_
          exact walkMap_uok _ (fun w v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) (FailCond.Le.refl _)
            (applyWith_uok hrun hft (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil) w)) w hitems
      | filter hxs hk hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b : LExpr, p.le (pc.inst (p.sd σ)) (((pc.join a).join b).inst (p.sd σ)) :=
            fun _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _)) (fun v hv => by simpa [LTy.lsubst] using hv)
            (bound_failsAs_inst hb (hpc _ _)) ?_
          exact walkFilter_uok _ (fun w v hv =>
            applyWith_uok hrun hft (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil) w) w hitems
      | firstWhere hxs hk hte hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          have hpc : ∀ a b c d : LExpr,
              p.le (pc.inst (p.sd σ)) (((((pc.join a).join b).join c).join d).inst (p.sd σ)) :=
            fun _ _ _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_trans
              (LExpr.le_join_left _ _) (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _ _ _)) (fun v hv => by simpa [LTy.lsubst] using hv)
            (bound_failsAs_inst hb (hpc _ _ _ _)) ?_
          exact walkFirst_uok _ (fun w v hv =>
            applyWith_uok hrun hft (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil) w) w hitems
      | fold hxs hi hf hte hinit hres hb =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨iv, hiv, hit⟩ := latom_ltypedσ henv hi
        obtain ⟨fv, hfv, hft⟩ := latom_ltypedσ henv hf
        simp only [LTy.lsubst, LTy.lsubsts] at hxt hft
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, hfv]
          have hpc : ∀ a b : LExpr, p.le (pc.inst (p.sd σ)) (((pc.join a).join b).inst (p.sd σ)) :=
            fun _ _ => LExpr.le_inst _ (LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_left _ _))
          refine UOk.mono (bound_allowed_inst hb (hpc _ _)) (fun v hv => LValTy.lsubst_raise hv)
            (bound_failsAs_inst hb (hpc _ _)) ?_
          exact walkFold_uok (fun w acc v hacc hv => UOk.mono (fun _ h => h)
            (fun _ h => LValTy.flow h (Flow.lsubst _ _ hres)) (FailCond.Le.refl _)
            (applyWith_uok hrun hft (.cons hacc (.cons (LValTy.flow hv (Flow.lsubst _ _ hte)) .nil)) w))
            w (LValTy.flow hit (Flow.lsubst _ _ hinit)) hitems
      | @parse _ _ s₀ t hs ht =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        cases t <;> simp [LTy.erase] at ht
        simp only [LTy.lsubst] at hvt
        cases hvt
        rename_i _ s
        simp only [evalRhs, hv]
        unfold parseInt
        split
        · exact uok_done LValTy.int
        · exact ⟨fun _ h => (nomatch h), ⟨⟨_, rfl, hpc0 _⟩, rfl⟩, WEff.self _ _⟩
      | @checked _ _ a t pred code ha hs _ _ =>
        -- T-Checked (#453): the operand when the predicate holds of it, a failure otherwise.
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        cases t <;> simp [LTy.erase, Ty.Scalar] at hs <;> simp only [LTy.lsubst] at hvt <;>
          cases hvt <;> simp only [evalRhs, hv, checkRun, Val.line] <;> split
        all_goals first
          | exact uok_done (by constructor)
          | exact ⟨fun _ h => (nomatch h), ⟨⟨_, rfl, hpc0 _⟩, rfl⟩, WEff.self _ _⟩
      | eq ha hb _ _ _ =>
        obtain ⟨va, hva, _⟩ := latom_ltypedσ henv ha
        obtain ⟨vb, hvb, _⟩ := latom_ltypedσ henv hb
        simp only [evalRhs, hva, hvb, done]
        exact uok_done LValTy.bool
      | toText ha _ =>
        obtain ⟨v, hv, _⟩ := latom_ltypedσ henv ha
        simp only [evalRhs, hv]
        exact uok_done (by simp only [LTy.lsubst]; exact .text)
      | @envGet _ _ name t ha ht _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        cases t <;> simp [LTy.erase] at ht
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | @text s _ =>
          simp only [evalRhs, hv]
          cases w.env s
          · exact uok_done (by simp only [LTy.lsubst, Option.elim]; exact .none)
          · exact uok_done (by simp only [LTy.lsubst, Option.elim]; exact .some .text)
      | tuple has =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        refine ⟨fun _ h => by simp [evalRhs] at h, ?_, WEff.self _ _⟩
        simp only [evalRhs, hvs, Option.map_some, Option.elim, URes, LTy.lsubst, LTy.lsubsts_eq]
        exact .tuple hvst
      | append hxs hi hflow =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨iv, hiv, hit⟩ := latom_ltypedσ henv hi
        simp only [LTy.lsubst] at hxt ⊢
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv]
          exact uok_done (.list (LValsAll.append_one hitems (LValTy.flow hit (Flow.lsubst _ _ hflow))))
      | set hxs hi _ hv hflow =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        obtain ⟨iv, hiv, hit⟩ := latom_ltypedσ henv hi
        obtain ⟨vv, hvv, hvt⟩ := latom_ltypedσ henv hv
        simp only [LTy.lsubst] at hxt hit
        cases hxt with
        | list hitems =>
          cases hit
          simp only [evalRhs, hxv, hiv, hvv]
          split
          · simp only [LTy.lsubst, LTy.lsubsts]
            exact uok_done (.tuple (.cons (.list (LValsAll.set _ hitems
              (LValTy.flow hvt (Flow.lsubst _ _ hflow)))) (.cons .unit .nil)))
          · exact ⟨fun _ h => by simp [abort] at h, by simp [abort, URes], WEff.self _ _⟩
    · intro σ w env Γ E pc e t ε hσ henv he
      have hpc0 : ∀ x : LExpr, p.le (pc.inst (p.sd σ)) ((pc.join x).inst (p.sd σ)) :=
        fun x => le_inst_join_left _ pc x
      cases he with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv ha
        exact ⟨fun _ h => by simp [eval] at h, by simpa [eval, hv, URes] using hvt, WEff.self _ _⟩
      | letR hr hflow hb _ =>
        simp only [eval]
        refine andThen_uok (UOk.mono allowed_union_left_inst
          (fun v hv => LValTy.flow hv (Flow.lsubst _ _ hflow)) FailsAs.union_left_inst
          (ihR hσ henv hr)) (fun w' v hv => ?_)
        exact UOk.mono (allowed_union_right_inst (hpc0 _)) (fun _ h => h)
          (FailsAs.union_right_inst (hpc0 _)) (ihE (w := w') hσ (.cons hv henv) hb)
      | letE hv hflow hb _ =>
        simp only [eval]
        refine andThen_uok (UOk.mono allowed_union_left_inst
          (fun v hv => LValTy.flow hv (Flow.lsubst _ _ hflow)) FailsAs.union_left_inst
          (ihE hσ henv hv)) (fun w' v hv => ?_)
        exact UOk.mono (allowed_union_right_inst (hpc0 _)) (fun _ h => h)
          (FailsAs.union_right_inst (hpc0 _)) (ihE (w := w') hσ (.cons hv henv) hb)
      | ite hc ht he h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hc
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | @bool b _ =>
          cases b
          · simp only [eval, hv]
            exact UOk.mono (allowed_union_right_inst (hpc0 _))
              (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
              (FailsAs.union_right_inst (hpc0 _)) (ihE hσ henv he)
          · simp only [eval, hv]
            exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
              (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
              ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
              (ihE hσ henv ht)
      | caseOption hs hn hsome h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | none =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
            ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
            (ihE hσ henv hn)
        | some hw =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right_inst (hpc0 _))
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
            (FailsAs.union_right_inst (hpc0 _))
            (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) hsome)
      | caseResult hs hok herr h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | ok hw =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
            ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
            (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) hok)
        | err hw =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right_inst (hpc0 _))
            (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
            (FailsAs.union_right_inst (hpc0 _)) (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) herr)
      | fail hpc _ _ =>
        exact ⟨fun _ h => by simp [eval] at h, ⟨⟨_, rfl,
          LExpr.le_trans (LExpr.le_inst _ hpc)
            (by rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
                exact LExpr.join_le (LExpr.le_join_left _ _) (LExpr.le_join_right _ _))⟩, rfl⟩, WEff.self _ _⟩
      | raise hs hf has hfl hpc _ _ =>
        obtain ⟨vs, hvs, hvst⟩ := latoms_ltypedσ henv has
        have hpay := LValsTy.ground_lsubst
          (fun u hu => (hwf.sums _ (sum_mem hs) _ (List.mem_of_getElem? hf) u hu).1)
          (LValsTy.flowsσ hvst hfl)
        simp only [eval, hvs]
        exact ⟨fun _ h => by simp at h, ⟨⟨_, rfl,
          LExpr.le_trans (LExpr.le_inst _ hpc)
            (by rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
                exact LExpr.join_le (LExpr.le_join_left _ _) (LExpr.le_join_right _ _))⟩,
          by simp [Eff.inst], _, _, hs, hf, hpay⟩, WEff.self _ _⟩
      | unreachable _ => exact ⟨fun _ h => by simp [eval] at h, by simp [eval, URes], WEff.self _ _⟩
      | @caseSum _ _ scrutinee kind ctor binders thenE elseE ls sm fs t₁ t₂ s ε₁ ε₂ hsc hsm hf hlen hth hel h₁ h₂ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hsc
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | record hrec _ => exact (not_sum_of_record hwf hrec hsm).elim
        | @variant _ i vs sm' fs' _ hsm' hf' hvals =>
          rw [hsm] at hsm'
          cases hsm'
          simp only [eval, hv]
          by_cases hi : i = ctor
          · subst hi
            rw [hf] at hf'
            cases hf'
            rw [if_pos rfl, if_pos (by rw [LValsTy.length hvals]; exact hlen)]
            have hg : ∀ f ∈ fs, f.Ground :=
              fun u hu => (hwf.sums _ (sum_mem hsm) _ (List.mem_of_getElem? hf) u hu).1
            have henv' := zip_ltyped (xs := binders) henv
              (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hvals) (by simp [hlen])
            rw [← LCtx.lsub_zip, ← LCtx.lsub_append] at henv'
            exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
              (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₁)))
              ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
              (ihE hσ henv' hth)
          · rw [if_neg hi]
            exact UOk.mono (allowed_union_right_inst (hpc0 _))
              (fun _ h => LValTy.lsubst_raise (LValTy.flow h (Flow.lsubst _ _ h₂)))
              (FailsAs.union_right_inst (hpc0 _)) (ihE hσ henv hel)
      | @caseSumT _ _ scrutinee kind ctor binders thenE elseE ls sm fs t₁ t₂ ts ε₁ ε₂ hsc hsm hf hlen hth hel h₁ h₂ _ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hsc
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | record hrec _ => exact (not_sum_of_record hwf hrec hsm).elim
        | @variant _ i vs sm' fs' _ hsm' hf' hvals =>
          rw [hsm] at hsm'
          cases hsm'
          simp only [eval, hv]
          by_cases hi : i = ctor
          · subst hi
            rw [hf] at hf'
            cases hf'
            rw [if_pos rfl, if_pos (by rw [LValsTy.length hvals]; exact hlen)]
            have hg : ∀ f ∈ fs, f.Ground :=
              fun u hu => (hwf.sums _ (sum_mem hsm) _ (List.mem_of_getElem? hf) u hu).1
            have henv' := zip_ltyped (xs := binders) henv
              (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hvals) (by simp [hlen])
            rw [← LCtx.lsub_zip, ← LCtx.lsub_append] at henv'
            exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
              (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁))
              ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
              (ihE hσ henv' hth)
          · rw [if_neg hi]
            exact UOk.mono (allowed_union_right_inst (hpc0 _))
              (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
              (FailsAs.union_right_inst (hpc0 _)) (ihE hσ henv hel)
      | attempt hb hr =>
        have h := ihE (w := w) hσ henv hb
        simp only [eval]
        revert h
        rcases eval H n p w env _ with ⟨tr, r, w'⟩
        intro h
        have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
            ((⟨_, _, Option.none, _, false, []⟩ : Eff).inst (p.sd σ)) ev :=
          fun ev hev => Allowed.mono (LExpr.le_refl _) id (HostFx.Le.refl _) (fun _ h => h) ev (h.1 ev hev)
        cases r with
        | ok v => exact ⟨hev, .ok h.2.1, h.2.2⟩
        | failed c => exact ⟨hev, .err .int, h.2.2⟩
        | stuck => exact ⟨hev, trivial, h.2.2⟩
        | outOfFuel | aborted => exact ⟨hev, trivial, h.2.2⟩
        | raised k c ps => exact absurd h.2.1.2.1 (by simp [Eff.inst, hr])
      | @attemptK _ _ kind body t ε hb hcd hra =>
        have h := ihE (w := w) hσ henv hb
        simp only [eval]
        revert h
        rcases eval H n p w env _ with ⟨tr, r, w'⟩
        intro h
        have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
            ((⟨_, _, Option.none, _, false, []⟩ : Eff).inst (p.sd σ)) ev :=
          fun ev hev => Allowed.mono (LExpr.le_refl _) id (HostFx.Le.refl _) (fun _ h => h) ev (h.1 ev hev)
        cases r with
        | ok v => exact ⟨hev, .ok h.2.1, h.2.2⟩
        | failed c => exact absurd h.2.1.2 (by simp [Eff.inst, hcd])
        | stuck => exact ⟨hev, trivial, h.2.2⟩
        | outOfFuel | aborted => exact ⟨hev, trivial, h.2.2⟩
        | raised k c ps =>
          obtain ⟨_, hk, sm, fs, hs, hf, hps⟩ := h.2.1
          have hkk : k = kind := hra k (by simpa [Eff.inst] using hk)
          subst hkk
          dsimp only
          rw [if_pos rfl]
          exact ⟨hev, .err (.variant hs hf hps), h.2.2⟩
      | @handleK _ _ kind body x handler _ _ _ ε₁ ε₂ hb hcd hra hh h₁ h₂ _ =>
        have h := ihE (w := w) hσ henv hb
        simp only [eval]
        revert h
        rcases eval H n p w env _ with ⟨tr, r, w₁⟩
        intro h
        have hmono : HostFx.Le ε₁.host (ε₁.host.or ε₂.host) := union_host_left (ε₂ := ε₂)
        have hmono₂ : HostFx.Le ε₂.host (ε₁.host.or ε₂.host) := union_host_right (ε₁ := ε₁)
        have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
            ((⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩ : Eff).inst (p.sd σ)) ev :=
          fun ev hev => Allowed.mono (LExpr.le_refl _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
            hmono (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (h.1 ev hev)
        cases r with
        | raised k c ps =>
          obtain ⟨_, hk, sm, fs, hs, hf, hps⟩ := h.2.1
          have hkk : k = kind := hra k (by simpa [Eff.inst] using hk)
          subst hkk
          dsimp only
          rw [if_pos rfl]
          have hh' := ihE (w := w₁) hσ (.cons (LValTy.variant (l := _) hs hf hps) henv) hh
          rcases hrh : eval H n p w₁ ((x, Val.variant k c ps) :: env) handler with ⟨tr', r', w₂⟩
          rw [hrh] at hh'
          refine ⟨fun ev hev' => ?_, ?_, WEff.trans h.2.2 hh'.2.2⟩
          · rcases List.mem_append.mp hev' with hev' | hev'
            · exact hev ev hev'
            · exact Allowed.mono (hpc0 _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
                hmono₂ (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (hh'.1 ev hev')
          · have h2 := hh'.2.1
            cases r' <;> simp only [URes] at h2 ⊢
            · exact LValTy.lsubst_raise (LValTy.flow h2 (Flow.lsubst _ _ h₂))
            · exact (FailsAs.mono_pc (hpc0 _)).1 h2
            · exact (FailsAs.mono_pc (hpc0 _)).2 _ _ _ h2
        | failed c => exact absurd h.2.1.2 (by simp [Eff.inst, hcd])
        | ok v => exact ⟨hev, LValTy.lsubst_raise (LValTy.flow h.2.1 (Flow.lsubst _ _ h₁)), h.2.2⟩
        | stuck => exact ⟨hev, trivial, h.2.2⟩
        | outOfFuel | aborted => exact ⟨hev, trivial, h.2.2⟩
      | @handleKT _ _ kind body x handler _ _ _ ε₁ ε₂ hb hcd hra hh h₁ h₂ _ _ =>
        have h := ihE (w := w) hσ henv hb
        simp only [eval]
        revert h
        rcases eval H n p w env _ with ⟨tr, r, w₁⟩
        intro h
        have hmono : HostFx.Le ε₁.host (ε₁.host.or ε₂.host) := union_host_left (ε₂ := ε₂)
        have hmono₂ : HostFx.Le ε₂.host (ε₁.host.or ε₂.host) := union_host_right (ε₁ := ε₁)
        have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
            ((⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩ : Eff).inst (p.sd σ)) ev :=
          fun ev hev => Allowed.mono (LExpr.le_refl _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
            hmono (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (h.1 ev hev)
        cases r with
        | raised k c ps =>
          obtain ⟨_, hk, sm, fs, hs, hf, hps⟩ := h.2.1
          have hkk : k = kind := hra k (by simpa [Eff.inst] using hk)
          subst hkk
          dsimp only
          rw [if_pos rfl]
          have hh' := ihE (w := w₁) hσ (.cons (LValTy.variant (l := _) hs hf hps) henv) hh
          rcases hrh : eval H n p w₁ ((x, Val.variant k c ps) :: env) handler with ⟨tr', r', w₂⟩
          rw [hrh] at hh'
          refine ⟨fun ev hev' => ?_, ?_, WEff.trans h.2.2 hh'.2.2⟩
          · rcases List.mem_append.mp hev' with hev' | hev'
            · exact hev ev hev'
            · exact Allowed.mono (hpc0 _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
                hmono₂ (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (hh'.1 ev hev')
          · have h2 := hh'.2.1
            cases r' <;> simp only [URes] at h2 ⊢
            · exact LValTy.flow h2 (Flow.lsubst _ _ h₂)
            · exact (FailsAs.mono_pc (hpc0 _)).1 h2
            · exact (FailsAs.mono_pc (hpc0 _)).2 _ _ _ h2
        | failed c => exact absurd h.2.1.2 (by simp [Eff.inst, hcd])
        | ok v => exact ⟨hev, LValTy.flow h.2.1 (Flow.lsubst _ _ h₁), h.2.2⟩
        | stuck => exact ⟨hev, trivial, h.2.2⟩
        | outOfFuel | aborted => exact ⟨hev, trivial, h.2.2⟩
      | @handle _ _ body x handler _ _ _ ε₁ ε₂ hb hr hh h₁ h₂ _ =>
        have h := ihE (w := w) hσ henv hb
        simp only [eval]
        revert h
        rcases eval H n p w env _ with ⟨tr, r, w₁⟩
        intro h
        have hmono : HostFx.Le ε₁.host (ε₁.host.or ε₂.host) := union_host_left (ε₂ := ε₂)
        have hmono₂ : HostFx.Le ε₂.host (ε₁.host.or ε₂.host) := union_host_right (ε₁ := ε₁)
        have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
            ((⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩ : Eff).inst (p.sd σ)) ev :=
          fun ev hev => Allowed.mono (LExpr.le_refl _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
            hmono (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (h.1 ev hev)
        cases r with
        | failed code =>
          have hh' := ihE (w := w₁) hσ (.cons (LValTy.int (n := (code : Int))) henv) hh
          rcases hrh : eval H n p w₁ ((x, Val.int (code : Int)) :: env) handler with ⟨tr', r', w₂⟩
          rw [hrh] at hh'
          simp only [hrh]
          refine ⟨fun ev hev' => ?_, ?_, WEff.trans h.2.2 hh'.2.2⟩
          · rcases List.mem_append.mp hev' with hev' | hev'
            · exact hev ev hev'
            · exact Allowed.mono (hpc0 _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
                hmono₂ (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (hh'.1 ev hev')
          · have h2 := hh'.2.1
            cases r' <;> simp only [URes] at h2 ⊢
            · exact LValTy.lsubst_raise (LValTy.flow h2 (Flow.lsubst _ _ h₂))
            · exact (FailsAs.mono_pc (hpc0 _)).1 h2
            · exact (FailsAs.mono_pc (hpc0 _)).2 _ _ _ h2
        | ok v => exact ⟨hev, LValTy.lsubst_raise (LValTy.flow h.2.1 (Flow.lsubst _ _ h₁)), h.2.2⟩
        | stuck => exact ⟨hev, trivial, h.2.2⟩
        | outOfFuel | aborted => exact ⟨hev, trivial, h.2.2⟩
        | raised k c ps => exact absurd h.2.1.2.1 (by simp [Eff.inst, hr])
      | iteT hc ht he h₁ h₂ _ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hc
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | @bool b _ =>
          cases b
          · simp only [eval, hv]
            exact UOk.mono (allowed_union_right_inst (hpc0 _))
              (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
              (FailsAs.union_right_inst (hpc0 _)) (ihE hσ henv he)
          · simp only [eval, hv]
            exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
              (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁))
              ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
              (ihE hσ henv ht)
      | caseOptionT hs hn hsome h₁ h₂ _ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | none =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
            (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁))
            ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
            (ihE hσ henv hn)
        | some hw =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right_inst (hpc0 _))
            (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
            (FailsAs.union_right_inst (hpc0 _))
            (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) hsome)
      | caseResultT hs hok herr h₁ h₂ _ _ =>
        obtain ⟨v, hv, hvt⟩ := latom_ltypedσ henv hs
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | ok hw =>
          simp only [eval, hv]
          exact UOk.mono (Allowed.mono (hpc0 _) union_op_left union_host_left (fun x h => union_rels_left x h))
            (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁))
            ((FailsAs.mono_pc (hpc0 _)).trans FailsAs.union_left_inst)
            (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) hok)
        | err hw =>
          simp only [eval, hv]
          exact UOk.mono (allowed_union_right_inst (hpc0 _))
            (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
            (FailsAs.union_right_inst (hpc0 _)) (ihE hσ (.cons (LValTy.lsubst_raise hw) henv) herr)
      | @handleT _ _ body x handler _ _ _ ε₁ ε₂ hb hr hh h₁ h₂ _ _ =>
        have h := ihE (w := w) hσ henv hb
        simp only [eval]
        revert h
        rcases eval H n p w env _ with ⟨tr, r, w₁⟩
        intro h
        have hmono : HostFx.Le ε₁.host (ε₁.host.or ε₂.host) := union_host_left (ε₂ := ε₂)
        have hmono₂ : HostFx.Le ε₂.host (ε₁.host.or ε₂.host) := union_host_right (ε₁ := ε₁)
        have hev : ∀ ev ∈ tr, Allowed p (pc.inst (p.sd σ))
            ((⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩ : Eff).inst (p.sd σ)) ev :=
          fun ev hev => Allowed.mono (LExpr.le_refl _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
            hmono (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (h.1 ev hev)
        cases r with
        | failed code =>
          have hh' := ihE (w := w₁) hσ (.cons (LValTy.int (n := (code : Int))) henv) hh
          rcases hrh : eval H n p w₁ ((x, Val.int (code : Int)) :: env) handler with ⟨tr', r', w₂⟩
          rw [hrh] at hh'
          simp only [hrh]
          refine ⟨fun ev hev' => ?_, ?_, WEff.trans h.2.2 hh'.2.2⟩
          · rcases List.mem_append.mp hev' with hev' | hev'
            · exact hev ev hev'
            · exact Allowed.mono (hpc0 _) (fun h => by simp only [Eff.inst] at h ⊢; simp [h])
                hmono₂ (fun x h => by simp only [Eff.inst] at h ⊢; simp [h]) ev (hh'.1 ev hev')
          · have h2 := hh'.2.1
            cases r' <;> simp only [URes] at h2 ⊢
            · exact LValTy.flow h2 (Flow.lsubst _ _ h₂)
            · exact (FailsAs.mono_pc (hpc0 _)).1 h2
            · exact (FailsAs.mono_pc (hpc0 _)).2 _ _ _ h2
        | ok v => exact ⟨hev, LValTy.flow h.2.1 (Flow.lsubst _ _ h₁), h.2.2⟩
        | stuck => exact ⟨hev, trivial, h.2.2⟩
        | outOfFuel | aborted => exact ⟨hev, trivial, h.2.2⟩
        | raised k c ps => exact absurd h.2.1.2.1 (by simp [Eff.inst, hr])
      | @letTuple _ _ bs value body ts l _ _ hv hflows _ hb =>
        obtain ⟨v, hv', hvt⟩ := latom_ltypedσ henv hv
        simp only [LTy.lsubst] at hvt
        cases hvt with
        | @tuple vs _ _ hvals =>
          rw [LTy.lsubsts_eq] at hvals
          have hbs := LValsTy.flowsσ (LValsTy.lsubst_raise l hvals) hflows
          simp only [eval, hv']
          rw [ite_eq_left (by rw [LValsTy.length hbs]; simp)]
          have henv' : LEnvTy p (bindParams bs vs env) ((bs ++ Γ).lsub p.recordDeep σ) := by
            rw [LCtx.lsub_append]
            have := bindParams_ltyped (params := LCtx.lsub p.recordDeep σ bs)
              (by rw [LCtx.lsub_snd]; exact hbs) henv
            simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
          exact ihE hσ henv' hb
      | @loopList _ _ xs x xty st init body te ls ts tb ε hxs hx _ hi hflows _ hb hbt _ =>
        obtain ⟨xv, hxv, hxt⟩ := latom_ltypedσ henv hxs
        simp only [LTy.lsubst] at hxt
        cases hxt with
        | @list items _ _ hitems =>
          obtain ⟨sv, hsv, hsvt⟩ := latoms_ltypedσ henv hi
          have hst0 := LValsTy.flowsσ hsvt hflows
          simp only [eval, sourceVals, hxv, hsv]
          rw [ite_eq_left (by rw [LValsTy.length hst0]; simp)]
          refine UOk.mono (fun _ h => h) (fun r hr => ?_) (FailCond.Le.refl _) (walkLoop_uok (a := te.lsubst p.recordDeep σ)
            (fun w v st' hv hst' => ?_) w hst0 (LValsAll.mem hitems))
          · obtain ⟨st', rfl, hst'⟩ := hr
            simp only [LTy.lsubst, LTy.lsubsts_eq]
            exact .tuple hst'
          · have henv' : LEnvTy p ((x, v) :: bindParams st st' env)
                (LCtx.lsub p.recordDeep σ ((x, xty) :: (st ++ Γ))) := by
              have hb' : LEnvTy p (bindParams st st' env) (LCtx.lsub p.recordDeep σ (st ++ Γ)) := by
                rw [LCtx.lsub_append]
                have := bindParams_ltyped (params := LCtx.lsub p.recordDeep σ st)
                  (by rw [LCtx.lsub_snd]; exact hst') henv
                simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
              exact .cons (LValTy.flow (LValTy.lsubst_raise hv) (Flow.lsubst _ _ hx)) hb'
            refine UOk.mono (Allowed.mono (hpc0 ls) id (HostFx.Le.refl _) (fun _ h => h))
              (fun r hr => ?_) (FailsAs.mono_pc (hpc0 ls))
              (ihE (w := w) hσ henv' hb)
            have := LValTy.flow hr (Flow.lsubst _ _ hbt)
            simp only [LTy.lsubst, LTy.lsubsts_eq] at this
            cases this with
            | tuple h => exact ⟨_, rfl, h⟩
      | @loopRange _ _ low high inc x xty st init body l₁ l₂ ts tb ε hlo hhi hx _ hi hflows _ hb hbt _ =>
        obtain ⟨lv, hlv, hlt⟩ := latom_ltypedσ henv hlo
        obtain ⟨hv₀, hhv, hht⟩ := latom_ltypedσ henv hhi
        simp only [LTy.lsubst] at hlt hht
        cases hlt with
        | @int a _ =>
          cases hht with
          | @int b _ =>
            obtain ⟨sv, hsv, hsvt⟩ := latoms_ltypedσ henv hi
            have hst0 := LValsTy.flowsσ hsvt hflows
            simp only [eval, sourceVals, hlv, hhv, hsv]
            rw [ite_eq_left (by rw [LValsTy.length hst0]; simp)]
            refine UOk.mono (fun _ h => h) (fun r hr => ?_) (FailCond.Le.refl _)
              (walkLoop_uok (a := (LTy.int (l₁.join l₂)).lsubst p.recordDeep σ)
                (fun w v st' hv hst' => ?_) w hst0 (fun v hv => ?_))
            · obtain ⟨st', rfl, hst'⟩ := hr
              simp only [LTy.lsubst, LTy.lsubsts_eq]
              exact .tuple hst'
            · have henv' : LEnvTy p ((x, v) :: bindParams st st' env)
                  (LCtx.lsub p.recordDeep σ ((x, xty) :: (st ++ Γ))) := by
                have hb' : LEnvTy p (bindParams st st' env) (LCtx.lsub p.recordDeep σ (st ++ Γ)) := by
                  rw [LCtx.lsub_append]
                  have := bindParams_ltyped (params := LCtx.lsub p.recordDeep σ st)
                    (by rw [LCtx.lsub_snd]; exact hst') henv
                  simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
                exact .cons (LValTy.flow hv (Flow.lsubst _ _ hx)) hb'
              refine UOk.mono (Allowed.mono (hpc0 _) id (HostFx.Le.refl _) (fun _ h => h))
                (fun r hr => ?_) (FailsAs.mono_pc (hpc0 _))
                (ihE (w := w) hσ henv' hb)
              have := LValTy.flow hr (Flow.lsubst _ _ hbt)
              simp only [LTy.lsubst, LTy.lsubsts_eq] at this
              cases this with
              | tuple h => exact ⟨_, rfl, h⟩
            · simp only [rangeVals, List.mem_map] at hv
              obtain ⟨_, _, rfl⟩ := hv
              simp only [LTy.lsubst]
              exact .int

/-- **The unary half without policies**, as M3 and M4 state it: `unary_soundR`, whose
restrictions hold when there is no policy. -/
theorem unary_sound (hwf : p.LWellFormed) (hpol : p.policies = []) (h4 : H.RespTyped) : ∀ n : Nat,
    (∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {r : Rhs}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LRhsTy p E Γ pc r t ε →
      UGood p (pc.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w (evalRhs H n p w env r)) ∧
    (∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
      {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
      LExprTy p E Γ pc e t ε →
      UGood p (pc.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w (eval H n p w env e)) :=
  unary_soundR hwf (.of_nil hpol) h4

end Una.F1
