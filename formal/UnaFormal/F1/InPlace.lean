/-
The typing rules keep `ℓ_T` in place (`LTy.InPlace`): in a context whose types keep it, at a
program counter without a plain symbol, every type the labeled typing derives keeps it, and the
label of every failure has no plain symbol (`typing_inPlace`).
-/
import UnaFormal.F1.Subst

namespace Una.F1

theorem mem_of_lookup {α β : Type} [BEq α] [LawfulBEq α] {x : α} {b : β} :
    ∀ {σ : List (α × β)}, List.lookup x σ = some b → (x, b) ∈ σ
  | [], h => by simp at h
  | (k, v) :: σ, h => by
    simp only [List.lookup] at h
    split at h
    · rename_i hk
      cases h
      have : x = k := by simpa using hk
      subst this
      exact List.mem_cons_self
    · exact List.mem_cons_of_mem _ (mem_of_lookup h)

/-- No plain symbol. -/
abbrev LExpr.PlainFree (e : LExpr) : Prop := e.plain = []

theorem LExpr.plainFree_join {a b : LExpr} (ha : a.PlainFree) (hb : b.PlainFree) :
    (a.join b).PlainFree := by
  simp [LExpr.PlainFree, LExpr.join, ha, hb]

theorem LExpr.plainFree_promote (e : LExpr) : e.promote.PlainFree := rfl

theorem LExpr.plainFree_bot : LExpr.bot.PlainFree := rfl

theorem LExpr.plainFree_ofLabel (l : Label) : (LExpr.ofLabel l).PlainFree := rfl

theorem LExpr.plainFree_joinAll : ∀ {es : List LExpr}, (∀ e ∈ es, e.PlainFree) →
    (LExpr.joinAll es).PlainFree
  | [], _ => rfl
  | e :: _, h =>
    plainFree_join (h e List.mem_cons_self) (plainFree_joinAll (fun x hx => h x (List.mem_cons_of_mem _ hx)))

/-- The plain symbols an instantiation leaves: those of `e` it does not cover, when the labels it
gives have none. -/
theorem LExpr.inst_plain {σ : List (String × LExpr)} (hσ : ∀ q ∈ σ, q.2.PlainFree) (e : LExpr) :
    ∀ x ∈ (e.inst σ).plain, x ∈ e.plain := by
  intro x hx
  simp only [LExpr.inst, LExpr.join, List.mem_append] at hx
  rcases hx with hx | hx
  · have : (LExpr.joinAll ((e.plain ++ e.star).filterMap (fun x => List.lookup x σ))).PlainFree := by
      apply LExpr.plainFree_joinAll
      intro l hl
      obtain ⟨y, _, hy⟩ := List.mem_filterMap.mp hl
      exact hσ _ (mem_of_lookup hy)
    rw [this] at hx; nomatch hx
  · exact (List.mem_filter.mp hx).1

theorem LExpr.plainFree_of {e : LExpr} (h : ∀ x ∈ e.plain, False) : e.PlainFree :=
  List.eq_nil_iff_forall_not_mem.mpr h

theorem LTy.raise_inPlace {e : LExpr} (he : e.PlainFree) :
    ∀ {t : LTy}, t.InPlace → (t.raise e).InPlace
  | .var a l, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢
    intro x hx
    rcases List.mem_append.mp hx with hx | hx
    · exact h x hx
    · rw [he] at hx; nomatch hx
  | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢; rw [h, he]; rfl
  | .list _ _, h | .option _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢; exact ⟨h.1, by rw [h.2, he]; rfl⟩
  | .result _ _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢; exact ⟨h.1, h.2.1, by rw [h.2.2, he]; rfl⟩
  | .fn _ _ _ _ _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢
    exact ⟨h.1, h.2.1, by rw [h.2.2, he]; rfl⟩
  | .tuple _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢; exact ⟨h.1, by rw [h.2, he]; rfl⟩

mutual
  theorem LTy.deep_plainFree (rd : String → Label) : ∀ {t : LTy}, t.InPlace → (t.deep rd).PlainFree
    | .var _ _, _ => rfl
    | .int _, h | .bool _, h | .text _, h | .unit _, h => h
    | .record _ _, h => LExpr.plainFree_join h rfl
    | .list _ _, h | .option _ _, h =>
      LExpr.plainFree_join h.2 (LTy.deep_plainFree rd h.1)
    | .result _ _ _, h =>
      LExpr.plainFree_join h.2.2
        (LExpr.plainFree_join (LTy.deep_plainFree rd h.1) (LTy.deep_plainFree rd h.2.1))
    | .fn _ _ _ _ _ _, h =>
      LExpr.plainFree_join h.2.2
        (LExpr.plainFree_join (LTy.deeps_plainFree rd h.1) (LTy.deep_plainFree rd h.2.1))
    | .tuple _ _, h => LExpr.plainFree_join h.2 (LTy.deeps_plainFree rd h.1)

  theorem LTy.deeps_plainFree (rd : String → Label) :
      ∀ {ts : List LTy}, LTy.InPlaceAll ts → (LTy.deeps rd ts).PlainFree
    | [], _ => rfl
    | _ :: _, h => LExpr.plainFree_join (LTy.deep_plainFree rd h.1) (LTy.deeps_plainFree rd h.2)
end

mutual
  theorem LTy.Ground.inPlace : ∀ {t : LTy}, t.Ground → t.InPlace
    | .var _ _, h => h.elim
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => h.1
    | .list _ _, h | .option _ _, h => ⟨LTy.Ground.inPlace h.1, h.2.1⟩
    | .result _ _ _, h => ⟨LTy.Ground.inPlace h.1, LTy.Ground.inPlace h.2.1, h.2.2.1⟩
    | .fn _ _ _ _ _ _, h => ⟨LTy.GroundAll.inPlace h.1, LTy.Ground.inPlace h.2.1, h.2.2.1⟩
    | .tuple _ _, h => ⟨LTy.GroundAll.inPlace h.1, h.2.1⟩

  theorem LTy.GroundAll.inPlace : ∀ {ts : List LTy}, LTy.GroundAll ts → LTy.InPlaceAll ts
    | [], _ => trivial
    | _ :: _, h => ⟨LTy.Ground.inPlace h.1, LTy.GroundAll.inPlace h.2⟩
end

theorem deeps_ground_plainFree (rd : String → Label) {σ : List (String × LTy)}
    (hσ : ∀ q ∈ σ, q.2.Ground) : ∀ q ∈ σ.map (fun q => (q.1, q.2.deep rd)), q.2.PlainFree := by
  intro q hq
  obtain ⟨q', hq', rfl⟩ := List.mem_map.mp hq
  exact LTy.deep_plainFree rd (LTy.Ground.inPlace (hσ q' hq'))

mutual
  /-- Instantiation by ground type arguments keeps `ℓ_T` in place. -/
  theorem LTy.lsubst_inPlace (rd : String → Label) {σ : List (String × LTy)}
      (hσ : ∀ q ∈ σ, q.2.Ground) : ∀ {t : LTy}, t.InPlace → (t.lsubst rd σ).InPlace
    | .int l, h | .bool l, h | .text l, h | .unit l, h | .record _ l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact LExpr.plainFree_of (fun x hx => by
        have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
        rw [h] at this; nomatch this)
    | .list t l, h | .option t l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact ⟨LTy.lsubst_inPlace rd hσ h.1, LExpr.plainFree_of (fun x hx => by
        have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
        rw [h.2] at this; nomatch this)⟩
    | .result t e l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact ⟨LTy.lsubst_inPlace rd hσ h.1, LTy.lsubst_inPlace rd hσ h.2.1,
        LExpr.plainFree_of (fun x hx => by
          have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
          rw [h.2.2] at this; nomatch this)⟩
    | .fn ps r _ _ _ l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact ⟨LTy.lsubsts_inPlace rd hσ h.1, LTy.lsubst_inPlace rd hσ h.2.1,
        LExpr.plainFree_of (fun x hx => by
          have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
          rw [h.2.2] at this; nomatch this)⟩
    | .tuple ts l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact ⟨LTy.lsubsts_inPlace rd hσ h.1, LExpr.plainFree_of (fun x hx => by
        have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
        rw [h.2] at this; nomatch this)⟩
    | .var a l, h => by
      simp only [LTy.lsubst]
      cases hl : List.lookup a σ with
      | some u =>
        apply LTy.raise_inPlace _ (LTy.Ground.inPlace (hσ _ (mem_of_lookup hl)))
        exact LExpr.plainFree_of (fun x hx => by
          have h₁ := LExpr.inst_plain (deeps_ground_plainFree rd hσ) _ x hx
          obtain ⟨h₂, hne⟩ := List.mem_filter.mp h₁
          simp only [LTy.InPlace] at h
          have := h x h₂
          subst this
          simp at hne)
      | none =>
        simp only [LTy.InPlace] at h ⊢
        exact fun x hx => h x (LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx)

  theorem LTy.lsubsts_inPlace (rd : String → Label) {σ : List (String × LTy)}
      (hσ : ∀ q ∈ σ, q.2.Ground) : ∀ {ts : List LTy}, LTy.InPlaceAll ts →
        LTy.InPlaceAll (LTy.lsubsts rd σ ts)
    | [], _ => trivial
    | _ :: _, h => ⟨LTy.lsubst_inPlace rd hσ h.1, LTy.lsubsts_inPlace rd hσ h.2⟩
end

/-- Every type of a context keeps `ℓ_T` in place. -/
def LCtx.InPlace (Γ : LCtx) : Prop := ∀ q ∈ Γ, q.2.InPlace

/-- The labels of failures, and the decision label (#524), have no plain symbol. -/
def Eff.PlainFree (ε : Eff) : Prop := (∀ l, ε.fails = some l → l.PlainFree) ∧ ε.dl.PlainFree

theorem Eff.failLabel_plainFree {ε : Eff} (h : ε.PlainFree) : ε.failLabel.PlainFree := by
  cases hf : ε.fails with
  | none => simp [Eff.failLabel, hf]; rfl
  | some l => simpa [Eff.failLabel, hf] using h.1 l hf

theorem Eff.plainFree_union {ε₁ ε₂ : Eff} (h₁ : ε₁.PlainFree) (h₂ : ε₂.PlainFree) :
    (ε₁.union ε₂).PlainFree := by
  refine ⟨fun l hl => ?_, LExpr.plainFree_join h₁.2 h₂.2⟩
  cases hf₁ : ε₁.fails <;> cases hf₂ : ε₂.fails <;> simp [Eff.union, hf₁, hf₂] at hl <;>
    subst hl
  · exact h₂.1 _ hf₂
  · exact h₁.1 _ hf₁
  · exact LExpr.plainFree_join (h₁.1 _ hf₁) (h₂.1 _ hf₂)

theorem Eff.plainFree_ofFn (o : Bool) (g : HostFx) (f : Bool) : (Eff.ofFn o g f).PlainFree := by
  refine ⟨fun l hl => ?_, LExpr.plainFree_bot⟩
  cases f <;> simp [Eff.ofFn] at hl
  subst hl; rfl

theorem Eff.plainFree_ofFnR (o : Bool) (g : HostFx) (f : Bool) (rs : List String) :
    (Eff.ofFnR o g f rs).PlainFree :=
  Eff.plainFree_ofFn o g f

theorem Eff.plainFree_ofCall (fn : Fun) : (Eff.ofCall fn).PlainFree :=
  ⟨fun l hl => (Eff.plainFree_ofFn fn.op fn.host fn.fails).1 l
    (by simpa [Eff.ofCall, Eff.ofFnR] using hl), LExpr.plainFree_bot⟩

/-- An invocation's failure is labeled `pc` (#524): plain-free when `pc` is. -/
theorem Eff.plainFree_ofInvoke {pc : LExpr} (h : pc.PlainFree) (fn : Fun) :
    (Eff.ofInvoke pc fn).PlainFree := by
  refine ⟨fun l hl => ?_, LExpr.plainFree_bot⟩
  cases hf : fn.fails <;> simp [Eff.ofInvoke, hf] at hl
  subst hl; exact h

theorem Eff.plainFree_none : Eff.none.PlainFree := ⟨fun _ h => by simp [Eff.none] at h, LExpr.plainFree_bot⟩

/-- A choice decided at a plain-free label (#524). -/
theorem Eff.plainFree_decide {c : LExpr} {ε : Eff} (h : ε.PlainFree) (hc : c.PlainFree) :
    (ε.decide c).PlainFree := by
  unfold Eff.decide
  split
  · exact ⟨h.1, LExpr.plainFree_join h.2 hc⟩
  · exact h

theorem Eff.plainFree_union_decide {c : LExpr} {ε₁ ε₂ : Eff} (hc : c.PlainFree) (h₁ : ε₁.PlainFree)
    (h₂ : ε₂.PlainFree) : ((ε₁.union ε₂).decide c).PlainFree :=
  Eff.plainFree_decide (Eff.plainFree_union h₁ h₂) hc

theorem Eff.plainFree_attemptOf (ε : Eff) : (Eff.attemptOf ε).PlainFree :=
  ⟨fun _ h => by simp [Eff.attemptOf] at h, LExpr.plainFree_bot⟩

theorem Eff.plainFree_handleOf {ε₁ ε₂ : Eff} (h₁ : ε₁.PlainFree) (h₂ : ε₂.PlainFree) :
    (Eff.handleOf ε₁ ε₂).PlainFree :=
  Eff.plainFree_decide (ε := ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels,
    ε₂.codes, ε₂.raises, ε₂.dl⟩) ⟨h₂.1, h₂.2⟩ h₁.2

/-- A request's failure label is plain-free when the program counter and the arguments are. -/
theorem Program.reqFail_plainFree {p : Program} {pc : LExpr} {ts : List LTy} (hpc : pc.PlainFree)
    (hd : ∀ u ∈ ts, (p.deep u).PlainFree) : (p.reqFail pc ts).PlainFree := by
  unfold Program.reqFail
  split
  · refine LExpr.plainFree_join hpc ?_
    split
    · exact hd _ (by simp_all)
    · exact LExpr.plainFree_bot
  · refine LExpr.plainFree_join (LExpr.plainFree_join hpc ?_) (LExpr.plainFree_ofLabel _)
    refine LExpr.plainFree_joinAll fun e he => ?_
    obtain ⟨u, hu, rfl⟩ := List.mem_map.mp he
    exact hd u hu

/-- A `Files` failure's label is plain-free when what decides it is. -/
theorem Program.fsFail_plainFree {p : Program} {l : LExpr} (h : l.PlainFree) : (p.fsFail l).PlainFree := by
  unfold Program.fsFail
  split
  · exact h
  · exact LExpr.plainFree_join h (LExpr.plainFree_ofLabel _)

theorem lctxLookup_mem {x : String} {t : LTy} : ∀ {Γ : LCtx}, lctxLookup Γ x = some t → (x, t) ∈ Γ
  | [], h => by simp [lctxLookup] at h
  | (y, u) :: Γ, h => by
    simp only [lctxLookup] at h
    split at h
    · rename_i hx; cases h; subst hx; exact List.mem_cons_self
    · exact List.mem_cons_of_mem _ (lctxLookup_mem h)

theorem latom_inPlace {Γ : LCtx} {a : Atom} {t : LTy} (hΓ : Γ.InPlace) (h : LAtomTy Γ a t) :
    t.InPlace := by
  cases h with
  | var hx => exact hΓ _ (lctxLookup_mem hx)
  | _ => rfl

theorem inPlaceAll_of_mem : ∀ {ts : List LTy}, (∀ t ∈ ts, t.InPlace) → LTy.InPlaceAll ts
  | [], _ => trivial
  | _ :: _, h => ⟨h _ List.mem_cons_self, inPlaceAll_of_mem (fun t ht => h t (List.mem_cons_of_mem _ ht))⟩

theorem mem_of_inPlaceAll : ∀ {ts : List LTy}, LTy.InPlaceAll ts → ∀ t ∈ ts, t.InPlace
  | [], _ => fun _ h => nomatch h
  | _ :: _, h => fun t ht => by
    rcases List.mem_cons.mp ht with rfl | ht
    · exact h.1
    · exact mem_of_inPlaceAll h.2 t ht

theorem ground_of_mem : ∀ {ts : List LTy}, LTy.GroundAll ts → ∀ t ∈ ts, t.Ground
  | [], _ => fun _ h => nomatch h
  | _ :: _, h => fun t ht => by
    rcases List.mem_cons.mp ht with rfl | ht
    · exact h.1
    · exact ground_of_mem h.2 t ht

theorem zip_ground {xs : List String} {ts : List LTy} (h : LTy.GroundAll ts) :
    ∀ q ∈ xs.zip ts, q.2.Ground := fun _ hq => ground_of_mem h _ (List.of_mem_zip hq).2

variable {p : Program}

theorem params_inPlace (hwf : p.LWellFormed) {f : String} {fn : Fun} (hf : p.find f = some fn)
    (σ : List (String × LTy)) (hσ : ∀ q ∈ σ, q.2.Ground) :
    LTy.InPlaceAll (fn.params.map (fun q => p.lsub σ q.2)) ∧ (p.lsub σ fn.result).InPlace := by
  obtain ⟨hps, hr⟩ := hwf.inPlace.1 fn (List.mem_of_find?_eq_some hf)
  refine ⟨inPlaceAll_of_mem (fun t ht => ?_), LTy.lsubst_inPlace _ hσ hr⟩
  obtain ⟨q, hq, rfl⟩ := List.mem_map.mp ht
  exact LTy.lsubst_inPlace _ hσ (hps q hq)

theorem latoms_inPlace {Γ : LCtx} {as : List Atom} {ts : List LTy} (hΓ : Γ.InPlace)
    (h : LAtomsTy Γ as ts) : ∀ u ∈ ts, u.InPlace := by
  induction h with
  | nil => intro _ h; nomatch h
  | cons ha _ ih =>
    intro u hu
    rcases List.mem_cons.mp hu with rfl | hu
    · exact latom_inPlace hΓ ha
    · exact ih u hu

theorem LTy.top_plainFree {u : LTy} (h : u.InPlace) (hv : ∀ a l, u ≠ .var a l) :
    u.top.PlainFree := by
  cases u with
  | var a l => exact absurd rfl (hv a l)
  | int | bool | text | unit | record => exact h
  | list | option => exact h.2
  | result => exact h.2.2
  | fn => exact h.2.2
  | tuple => exact h.2

/-- The label of a primitive's result has no plain symbol. -/
theorem prim_inPlace {pr : Prim} {ts : List LTy} {t : Ty} (hts : ∀ u ∈ ts, u.InPlace)
    (hp : PrimTy pr (ts.map LTy.erase) t) : (ofScalar t (p.primLabel pr ts)).InPlace := by
  have hl : (p.primLabel pr ts).PlainFree := by
    unfold Program.primLabel
    split
    · rename_i u
      generalize hes : [u].map LTy.erase = es at hp
      cases hp <;> simp only [List.map_cons, List.map_nil, List.cons.injEq, and_true] at hes <;>
        exact LTy.top_plainFree (hts u List.mem_cons_self) (fun a l h => by
          subst h; simp [LTy.erase] at hes)
    · exact LExpr.plainFree_joinAll (fun e he => by
        obtain ⟨u, hu, rfl⟩ := List.mem_map.mp he
        exact LTy.deep_plainFree _ (hts u hu))
  unfold ofScalar; split <;> exact hl


/-- The bindings of a `case` on a sum keep `ℓ_T` in place (#512): its fields' declared types do,
raised by a plain-free label. -/
theorem caseSum_ctx_inPlace (hwf : p.LWellFormed) {Γ : LCtx} {ls : LExpr} {k : String} {sm : Sum}
    {i : Nat} {fs : List LTy} {xs : List String} (hΓ : Γ.InPlace) (hl : ls.PlainFree)
    (hsm : p.sum k = some sm) (hf : sm.variants[i]? = some fs) :
    LCtx.InPlace (xs.zip (fs.map (LTy.raise ls)) ++ Γ) := fun q hq => by
  rcases List.mem_append.mp hq with hq | hq
  · obtain ⟨a, b⟩ := q
    obtain ⟨_, hb⟩ := List.of_mem_zip hq
    obtain ⟨t, ht, rfl⟩ := List.mem_map.mp hb
    exact LTy.raise_inPlace hl
      (hwf.sums _ (List.mem_of_find?_eq_some hsm) _ (List.mem_of_getElem? hf) t ht).2.1
  · exact hΓ q hq

mutual
  /-- **The typing rules keep `ℓ_T` in place.** -/
  theorem LRhsTy.inPlace (hwf : p.LWellFormed) :
      ∀ {E : List String} {Γ : LCtx} {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff},
        Γ.InPlace → pc.PlainFree → LRhsTy p E Γ pc r t ε → t.InPlace ∧ ε.PlainFree
    | _, _, _, _, _, _, hΓ, _, .atom h => ⟨latom_inPlace hΓ h, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .prim (ts := ts) has hp =>
      ⟨prim_inPlace (latoms_inPlace hΓ has) hp, Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .call (fn := fn) hf _ _ _ _ hg _ _ _ _ =>
      ⟨(params_inPlace hwf hf _ (zip_ground hg)).2, Eff.plainFree_ofCall _⟩
    | _, _, _, _, _, _, _, hpc, .invoke (fn := fn) hf _ _ _ _ _ _ =>
      ⟨(hwf.inPlace.1 fn (List.mem_of_find?_eq_some hf)).2, Eff.plainFree_ofInvoke hpc fn⟩
    | _, _, _, _, _, _, _, hpc, .invokeOp (fn := fn) _ _ _ hf _ _ _ _ _ _ =>
      ⟨(hwf.inPlace.1 fn (List.mem_of_find?_eq_some hf)).2, Eff.plainFree_ofInvoke hpc fn⟩
    | _, _, _, _, _, _, hΓ, _, .apply hf _ _ _ => by
      have h := latom_inPlace hΓ hf
      simp only [LTy.InPlace] at h
      exact ⟨LTy.raise_inPlace (LExpr.plainFree_join h.2.2 rfl) h.2.1, Eff.plainFree_ofFn _ _ _⟩
    | _, _, _, _, _, _, _, _, .op _ _ _ _ => ⟨rfl, fun _ h => by simp at h, LExpr.plainFree_bot⟩
    | _, _, _, _, _, _, hΓ, hpc, .read hu _ _ _ =>
      ⟨rfl, fun _ h => by
        simp at h; subst h
        exact Program.fsFail_plainFree (LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hu))),
        Program.fsFail_plainFree (LTy.deep_plainFree _ (latom_inPlace hΓ hu))⟩
    | _, _, _, _, _, _, hΓ, hpc, .write hu _ _ _ _ _ _ =>
      ⟨rfl, fun _ h => by
        simp at h; subst h
        exact Program.fsFail_plainFree (LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hu))),
        Program.fsFail_plainFree (LTy.deep_plainFree _ (latom_inPlace hΓ hu))⟩
    | _, _, _, _, _, _, hΓ, hpc, .req (ts := ts) has _ _ _ _ => by
      have hts := latoms_inPlace hΓ has
      have hd : ∀ u ∈ ts, (p.deep u).PlainFree := fun u hu => LTy.deep_plainFree _ (hts u hu)
      exact ⟨rfl, fun _ h => by simp at h; subst h; exact Program.reqFail_plainFree hpc hd,
        Program.reqFail_plainFree LExpr.plainFree_bot hd⟩
    | _, _, _, _, _, _, _, _, .construct _ _ _ => ⟨rfl, Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .variant _ _ _ _ => ⟨rfl, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .field hv hrec hi => by
      have h := latom_inPlace hΓ hv
      have hrec' := hwf.inPlace.2.1 _ (List.mem_of_find?_eq_some hrec) _ (List.mem_of_getElem? hi)
      exact ⟨LTy.raise_inPlace h hrec', Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .list _ _ ht => ⟨⟨ht, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .none ht => ⟨⟨ht, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .some ha => ⟨⟨latom_inPlace hΓ ha, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .first ha => by
      have h := latom_inPlace hΓ ha
      exact ⟨h, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .mkOk ha he => ⟨⟨latom_inPlace hΓ ha, he, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .mkErr ha ht => ⟨⟨ht, latom_inPlace hΓ ha, rfl⟩, Eff.plainFree_none⟩
    | _, Γ, _, _, _, _, hΓ, _, .lambda (params := params) hb _ _ _ hps => by
      have hΓ' : LCtx.InPlace (params ++ Γ) := by
        intro q hq
        rcases List.mem_append.mp hq with hq | hq
        · exact hps q hq
        · exact hΓ q hq
      refine ⟨⟨inPlaceAll_of_mem (fun t ht => ?_), (LExprTy.inPlace hwf hΓ' rfl hb).1, rfl⟩,
        Eff.plainFree_none⟩
      obtain ⟨q, hq, rfl⟩ := List.mem_map.mp ht
      exact hps q hq
    | _, _, _, _, _, _, _, _, .funRef (fn := fn) hf _ _ _ _ hg _ _ =>
      ⟨⟨(params_inPlace hwf hf _ (zip_ground hg)).1, (params_inPlace hwf hf _ (zip_ground hg)).2, rfl⟩,
        Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, hpc, .declassify hpo _ _ _ _ _ => by
      have hmem := List.mem_of_find?_eq_some hpo
      have hd := (hwf.inPlace.2.2 _ hmem).2
      have hcl := (hwf.policies _ hmem).2.1
      have htop := LTy.top_plainFree hd (fun a l' h => by
          rw [h] at hcl
          have hc := hcl [(a, .int)]
          simp [LTy.erase, Ty.subst] at hc)
      refine ⟨hd, fun l hl => ?_, ?_⟩
      · simp only [Eff.ofPolicy] at hl
        split at hl
        · cases hl
          exact LExpr.plainFree_join hpc htop
        · cases hl
      · simp only [Eff.ofPolicy]
        split
        · exact htop
        · exact LExpr.plainFree_bot
    | _, _, _, _, _, _, hΓ, _, .map hxs hf _ _ => by
      have hx := latom_inPlace hΓ hxs
      have h := latom_inPlace hΓ hf
      simp only [LTy.InPlace] at hx h
      exact ⟨⟨LTy.raise_inPlace h.2.2 h.2.1, LExpr.plainFree_join hx.2 h.2.2⟩, Eff.plainFree_ofFn _ _ _⟩
    | _, _, _, _, _, _, hΓ, _, .filter hxs hk _ _ => by
      have hx := latom_inPlace hΓ hxs
      have h := latom_inPlace hΓ hk
      simp only [LTy.InPlace] at hx h
      exact ⟨⟨hx.1, LExpr.plainFree_join (LExpr.plainFree_join hx.2 h.2.2) h.2.1⟩,
        Eff.plainFree_ofFn _ _ _⟩
    | _, _, _, _, _, _, hΓ, _, .firstWhere hxs hk _ _ => by
      have hx := latom_inPlace hΓ hxs
      have h := latom_inPlace hΓ hk
      simp only [LTy.InPlace] at hx h
      exact ⟨⟨hx.1, LExpr.plainFree_join (LExpr.plainFree_join hx.2 h.2.2) h.2.1⟩,
        Eff.plainFree_ofFn _ _ _⟩
    | _, _, _, _, _, _, hΓ, _, .fold hxs _ hf _ _ _ _ => by
      have hx := latom_inPlace hΓ hxs
      have h := latom_inPlace hΓ hf
      simp only [LTy.InPlace, LTy.InPlaceAll] at hx h
      exact ⟨LTy.raise_inPlace (LExpr.plainFree_join hx.2 h.2.2) h.1.1, Eff.plainFree_ofFn _ _ _⟩
    | _, _, _, _, _, _, hΓ, hpc, .parse hs _ =>
      ⟨LTy.deep_plainFree _ (latom_inPlace hΓ hs), fun _ h => by
        simp at h; subst h
        exact LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hs)),
        LTy.deep_plainFree _ (latom_inPlace hΓ hs)⟩
    | _, _, _, _, _, _, hΓ, hpc, .checked hs _ _ _ =>
      ⟨latom_inPlace hΓ hs, fun _ h => by
        simp at h; subst h
        exact LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hs)),
        LTy.deep_plainFree _ (latom_inPlace hΓ hs)⟩
    | _, _, _, _, _, _, hΓ, _, .eq ha hb _ _ _ =>
      ⟨LExpr.plainFree_join (LTy.deep_plainFree _ (latom_inPlace hΓ ha))
        (LTy.deep_plainFree _ (latom_inPlace hΓ hb)), Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .toText ha _ =>
      ⟨LTy.deep_plainFree _ (latom_inPlace hΓ ha), Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .envGet _ _ _ => ⟨⟨rfl, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .tuple has =>
      ⟨⟨inPlaceAll_of_mem (latoms_inPlace hΓ has), rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .append hxs _ _ => ⟨latom_inPlace hΓ hxs, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .set hxs _ _ _ _ =>
      ⟨⟨⟨latom_inPlace hΓ hxs, rfl, trivial⟩, rfl⟩, Eff.plainFree_none⟩

  theorem LExprTy.inPlace (hwf : p.LWellFormed) :
      ∀ {E : List String} {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff},
        Γ.InPlace → pc.PlainFree → LExprTy p E Γ pc e t ε → t.InPlace ∧ ε.PlainFree
    | _, _, _, _, _, _, hΓ, _, .atom h => ⟨latom_inPlace hΓ h, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, hpc, .letR hr _ hb hann => by
      have h₁ := LRhsTy.inPlace hwf hΓ hpc hr
      have h₂ := LExprTy.inPlace hwf (Γ := (_, _) :: _) (fun q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · exact hann
          · exact hΓ q hq)
        (LExpr.plainFree_join hpc (Eff.failLabel_plainFree h₁.2)) hb
      exact ⟨h₂.1, Eff.plainFree_union h₁.2 h₂.2⟩
    | _, _, _, _, _, _, hΓ, hpc, .letE hv _ hb hann => by
      have h₁ := LExprTy.inPlace hwf hΓ hpc hv
      have h₂ := LExprTy.inPlace hwf (Γ := (_, _) :: _) (fun q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · exact hann
          · exact hΓ q hq)
        (LExpr.plainFree_join hpc (Eff.failLabel_plainFree h₁.2)) hb
      exact ⟨h₂.1, Eff.plainFree_union h₁.2 h₂.2⟩
    | _, _, _, _, _, _, hΓ, hpc, .ite hc ht he _ _ hs => by
      have hl : _ := latom_inPlace hΓ hc
      simp only [LTy.InPlace] at hl
      exact ⟨LTy.raise_inPlace hl hs, Eff.plainFree_union_decide hl
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) ht).2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) he).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseOption (x := x) (te := te) (ls := ls) hsc hn hsome _ _ hs => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ' : LCtx.InPlace ((x, te.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2 hl.1
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace hl.2 hs, Eff.plainFree_union_decide hl.2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl.2) hn).2
        (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl.2) hsome).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseResult (x := x) (y := y) (te := te) (ee := ee) (ls := ls) hsc hok herr _ _ hs => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ₁ : LCtx.InPlace ((x, te.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2.2 hl.1
        · exact hΓ q hq
      have hΓ₂ : LCtx.InPlace ((y, ee.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2.2 hl.2.1
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace hl.2.2 hs, Eff.plainFree_union_decide hl.2.2
        (LExprTy.inPlace hwf hΓ₁ (LExpr.plainFree_join hpc hl.2.2) hok).2
        (LExprTy.inPlace hwf hΓ₂ (LExpr.plainFree_join hpc hl.2.2) herr).2⟩
    | _, _, _, _, _, _, _, _, .fail _ _ hs => ⟨hs, fun _ h => by simp at h; subst h; rfl, LExpr.plainFree_bot⟩
    | _, _, _, _, _, _, _, _, .raise _ _ _ _ _ _ hs => ⟨hs, fun _ h => by simp at h; subst h; rfl, LExpr.plainFree_bot⟩
    | _, _, _, _, _, _, _, _, .unreachable hs => ⟨hs, Eff.plainFree_none⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .withH (o := o) _ hb =>
      LExprTy.inPlace hwf (Γ := (markerName o, .text .bot) :: Γ) (fun q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · rfl
          · exact hΓ q hq) hpc hb
    | _, Γ, _, _, _, _, hΓ, hpc, .caseSum (binders := xs) (fs := fs) (ls := ls) hsc hsm hf _ hth hel _ _ hs => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ' := caseSum_ctx_inPlace hwf hΓ hl hsm hf (xs := xs)
      exact ⟨LTy.raise_inPlace hl hs, Eff.plainFree_union_decide hl
        (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hth).2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) hel).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseSumT (binders := xs) (fs := fs) (ls := ls) hsc hsm hf _ hth hel _ _ hts _ => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ' := caseSum_ctx_inPlace hwf hΓ hl hsm hf (xs := xs)
      exact ⟨⟨hts, rfl⟩, Eff.plainFree_union_decide hl
        (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hth).2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) hel).2⟩
    | _, _, _, _, _, _, hΓ, hpc, .attempt hb _ => by
      have h := LExprTy.inPlace hwf hΓ hpc hb
      exact ⟨⟨h.1, rfl, h.2.2⟩, Eff.plainFree_attemptOf _⟩
    | _, _, _, _, _, _, hΓ, hpc, .attemptK hb _ _ => by
      have h := LExprTy.inPlace hwf hΓ hpc hb
      exact ⟨⟨h.1, rfl, h.2.2⟩, Eff.plainFree_attemptOf _⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .handleK (kind := kd) (x := x) (ε₁ := ε₁) hb _ _ hh _ _ hs => by
      have h₁ := LExprTy.inPlace hwf hΓ hpc hb
      have hl := Eff.failLabel_plainFree h₁.2
      have hΓ' : LCtx.InPlace ((x, .record kd ε₁.dl) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact h₁.2.2
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace h₁.2.2 hs,
        Eff.plainFree_handleOf h₁.2 (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hh).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .handleKT (kind := kd) (x := x) (ε₁ := ε₁) hb _ _ hh _ _ hts _ => by
      have h₁ := LExprTy.inPlace hwf hΓ hpc hb
      have hl := Eff.failLabel_plainFree h₁.2
      have hΓ' : LCtx.InPlace ((x, .record kd ε₁.dl) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact h₁.2.2
        · exact hΓ q hq
      exact ⟨⟨hts, rfl⟩,
        Eff.plainFree_handleOf h₁.2 (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hh).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .handle (x := x) (ε₁ := ε₁) hb _ hh _ _ hs => by
      have h₁ := LExprTy.inPlace hwf hΓ hpc hb
      have hl := Eff.failLabel_plainFree h₁.2
      have hΓ' : LCtx.InPlace ((x, .int ε₁.dl) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact h₁.2.2
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace h₁.2.2 hs,
        Eff.plainFree_handleOf h₁.2 (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hh).2⟩
    | _, _, _, _, _, _, hΓ, hpc, .iteT hc ht he _ _ hts _ => by
      have hl : _ := latom_inPlace hΓ hc
      simp only [LTy.InPlace] at hl
      exact ⟨⟨hts, rfl⟩, Eff.plainFree_union_decide hl
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) ht).2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) he).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseOptionT (x := x) (te := te) (ls := ls) hsc hn hsome _ _ hts _ => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ' : LCtx.InPlace ((x, te.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2 hl.1
        · exact hΓ q hq
      exact ⟨⟨hts, rfl⟩, Eff.plainFree_union_decide hl.2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl.2) hn).2
        (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl.2) hsome).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseResultT (x := x) (y := y) (te := te) (ee := ee) (ls := ls) hsc hok herr _ _ hts _ => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ₁ : LCtx.InPlace ((x, te.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2.2 hl.1
        · exact hΓ q hq
      have hΓ₂ : LCtx.InPlace ((y, ee.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2.2 hl.2.1
        · exact hΓ q hq
      exact ⟨⟨hts, rfl⟩, Eff.plainFree_union_decide hl.2.2
        (LExprTy.inPlace hwf hΓ₁ (LExpr.plainFree_join hpc hl.2.2) hok).2
        (LExprTy.inPlace hwf hΓ₂ (LExpr.plainFree_join hpc hl.2.2) herr).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .handleT (x := x) (ε₁ := ε₁) hb _ hh _ _ hts _ => by
      have h₁ := LExprTy.inPlace hwf hΓ hpc hb
      have hl := Eff.failLabel_plainFree h₁.2
      have hΓ' : LCtx.InPlace ((x, .int ε₁.dl) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact h₁.2.2
        · exact hΓ q hq
      exact ⟨⟨hts, rfl⟩,
        Eff.plainFree_handleOf h₁.2 (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hh).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .letTuple (binders := bs) _ _ hbs hb => by
      have hΓ' : LCtx.InPlace (bs ++ Γ) := fun q hq => by
        rcases List.mem_append.mp hq with hq | hq
        · exact hbs q hq
        · exact hΓ q hq
      exact LExprTy.inPlace hwf hΓ' hpc hb
    | _, Γ, _, _, _, _, hΓ, hpc, .loopList (x := x) (xty := xty) (state := st) hxs _ hx _ _ hst hb _ _ _ => by
      have hl := latom_inPlace hΓ hxs
      simp only [LTy.InPlace] at hl
      have hΓ' : LCtx.InPlace ((x, xty) :: (st ++ Γ)) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact hx
        · rcases List.mem_append.mp hq with hq | hq
          · exact hst q hq
          · exact hΓ q hq
      refine ⟨⟨inPlaceAll_of_mem (fun t ht => ?_), rfl⟩,
        Eff.plainFree_decide (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl.2) hb).2 hl.2⟩
      obtain ⟨q, hq, rfl⟩ := List.mem_map.mp ht
      exact hst q hq
    | _, Γ, _, _, _, _, hΓ, hpc, .loopRange (x := x) (xty := xty) (state := st) hlo hhi _ hx _ _ hst hb _ _ _ => by
      have h₁ := latom_inPlace hΓ hlo
      have h₂ := latom_inPlace hΓ hhi
      simp only [LTy.InPlace] at h₁ h₂
      have hl : LExpr.PlainFree _ := LExpr.plainFree_join h₁ h₂
      have hΓ' : LCtx.InPlace ((x, xty) :: (st ++ Γ)) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact hx
        · rcases List.mem_append.mp hq with hq | hq
          · exact hst q hq
          · exact hΓ q hq
      refine ⟨⟨inPlaceAll_of_mem (fun t ht => ?_), rfl⟩,
        Eff.plainFree_decide (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hb).2 hl⟩
      obtain ⟨q, hq, rfl⟩ := List.mem_map.mp ht
      exact hst q hq
end

end Una.F1
