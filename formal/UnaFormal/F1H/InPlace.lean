/-
The typing rules keep `ℓ_T` in place (`LTy.InPlace`): in a context whose types keep it, at a
program counter without a plain symbol, every type the labeled typing derives keeps it, and the
label of every failure has no plain symbol (`typing_inPlace`).
-/
import UnaFormal.F1H.Subst
import UnaFormal.F1.InPlace

namespace Una.F1H

open Una.F1 (Label LExpr)

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
  | .list _ _, h | .option _ _, h | .result _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢; exact ⟨h.1, by rw [h.2, he]; rfl⟩
  | .fn _ _ _ _ _ _, h => by
    simp only [LTy.raise, LTy.InPlace, LExpr.join] at h ⊢
    exact ⟨h.1, h.2.1, by rw [h.2.2, he]; rfl⟩

mutual
  theorem LTy.deep_plainFree (rd : String → Label) : ∀ {t : LTy}, t.InPlace → (t.deep rd).PlainFree
    | .var _ _, _ => rfl
    | .int _, h | .bool _, h | .text _, h | .unit _, h => h
    | .record _ _, h => LExpr.plainFree_join h rfl
    | .list _ _, h | .option _ _, h | .result _ _, h =>
      LExpr.plainFree_join h.2 (LTy.deep_plainFree rd h.1)
    | .fn _ _ _ _ _ _, h =>
      LExpr.plainFree_join h.2.2
        (LExpr.plainFree_join (LTy.deeps_plainFree rd h.1) (LTy.deep_plainFree rd h.2.1))

  theorem LTy.deeps_plainFree (rd : String → Label) :
      ∀ {ts : List LTy}, LTy.InPlaceAll ts → (LTy.deeps rd ts).PlainFree
    | [], _ => rfl
    | _ :: _, h => LExpr.plainFree_join (LTy.deep_plainFree rd h.1) (LTy.deeps_plainFree rd h.2)
end

mutual
  theorem LTy.Ground.inPlace : ∀ {t : LTy}, t.Ground → t.InPlace
    | .var _ _, h => h.elim
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => h.1
    | .list _ _, h | .option _ _, h | .result _ _, h => ⟨LTy.Ground.inPlace h.1, h.2.1⟩
    | .fn _ _ _ _ _ _, h => ⟨LTy.GroundAll.inPlace h.1, LTy.Ground.inPlace h.2.1, h.2.2.1⟩

  theorem LTy.GroundAll.inPlace : ∀ {ts : List LTy}, LTy.GroundAll ts → LTy.InPlaceAll ts
    | [], _ => trivial
    | _ :: _, h => ⟨LTy.Ground.inPlace h.1, LTy.GroundAll.inPlace h.2⟩
end

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
    | .list t l, h | .option t l, h | .result t l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact ⟨LTy.lsubst_inPlace rd hσ h.1, LExpr.plainFree_of (fun x hx => by
        have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
        rw [h.2] at this; nomatch this)⟩
    | .fn ps r _ _ _ l, h => by
      simp only [LTy.lsubst, LTy.InPlace] at h ⊢
      exact ⟨LTy.lsubsts_inPlace rd hσ h.1, LTy.lsubst_inPlace rd hσ h.2.1,
        LExpr.plainFree_of (fun x hx => by
          have := LExpr.inst_plain (deeps_ground_plainFree rd hσ) l x hx
          rw [h.2.2] at this; nomatch this)⟩
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

/-- The labels of failures have no plain symbol. -/
def Eff.PlainFree (ε : Eff) : Prop := ∀ l, ε.fails = some l → l.PlainFree

theorem Eff.failLabel_plainFree {ε : Eff} (h : ε.PlainFree) : ε.failLabel.PlainFree := by
  cases hf : ε.fails with
  | none => simp [Eff.failLabel, hf]; rfl
  | some l => simpa [Eff.failLabel, hf] using h l hf

theorem Eff.plainFree_union {ε₁ ε₂ : Eff} (h₁ : ε₁.PlainFree) (h₂ : ε₂.PlainFree) :
    (ε₁.union ε₂).PlainFree := by
  intro l hl
  cases hf₁ : ε₁.fails <;> cases hf₂ : ε₂.fails <;> simp [Eff.union, hf₁, hf₂] at hl <;>
    subst hl
  · exact h₂ _ hf₂
  · exact h₁ _ hf₁
  · exact LExpr.plainFree_join (h₁ _ hf₁) (h₂ _ hf₂)

theorem Eff.plainFree_ofFn (o : Bool) (g : HostFx) (f : Bool) : (Eff.ofFn o g f).PlainFree := by
  intro l hl
  cases f <;> simp [Eff.ofFn] at hl
  subst hl; rfl

theorem Eff.plainFree_ofFnR (o : Bool) (g : HostFx) (f : Bool) (rs : List String) :
    (Eff.ofFnR o g f rs).PlainFree :=
  Eff.plainFree_ofFn o g f

theorem Eff.plainFree_none : Eff.none.PlainFree := fun _ h => by simp [Eff.none] at h

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
  | list | option | result => exact h.2
  | fn => exact h.2.2

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

mutual
  /-- **The typing rules keep `ℓ_T` in place.** -/
  theorem LRhsTy.inPlace (hwf : p.LWellFormed) :
      ∀ {E : List String} {Γ : LCtx} {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff},
        Γ.InPlace → pc.PlainFree → LRhsTy p E Γ pc r t ε → t.InPlace ∧ ε.PlainFree
    | _, _, _, _, _, _, hΓ, _, .atom h => ⟨latom_inPlace hΓ h, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .prim (ts := ts) has hp =>
      ⟨prim_inPlace (latoms_inPlace hΓ has) hp, Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .call (fn := fn) hf _ _ _ _ hg _ _ _ _ =>
      ⟨(params_inPlace hwf hf _ (zip_ground hg)).2, Eff.plainFree_ofFnR _ _ _ _⟩
    | _, _, _, _, _, _, hΓ, _, .apply hf _ _ _ => by
      have h := latom_inPlace hΓ hf
      simp only [LTy.InPlace] at h
      exact ⟨LTy.raise_inPlace (LExpr.plainFree_join h.2.2 rfl) h.2.1, Eff.plainFree_ofFn _ _ _⟩
    | _, _, _, _, _, _, _, _, .op _ _ _ _ => ⟨rfl, fun _ h => by simp at h⟩
    | _, _, _, _, _, _, hΓ, hpc, .read hu _ _ _ =>
      ⟨rfl, fun _ h => by
        simp at h; subst h
        exact Program.fsFail_plainFree (LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hu)))⟩
    | _, _, _, _, _, _, hΓ, hpc, .write hu _ _ _ _ _ _ =>
      ⟨rfl, fun _ h => by
        simp at h; subst h
        exact Program.fsFail_plainFree (LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hu)))⟩
    | _, _, _, _, _, _, hΓ, hpc, .req (ts := ts) has _ _ _ =>
      ⟨rfl, fun _ h => by
        simp at h; subst h
        have hts := latoms_inPlace hΓ has
        unfold Program.reqFail
        split
        · refine LExpr.plainFree_join hpc ?_
          split
          · exact LTy.deep_plainFree _ (hts _ (by simp_all))
          · exact LExpr.plainFree_bot
        · refine LExpr.plainFree_join (LExpr.plainFree_join hpc ?_) (LExpr.plainFree_ofLabel _)
          refine LExpr.plainFree_joinAll fun e he => ?_
          obtain ⟨u, hu, rfl⟩ := List.mem_map.mp he
          exact LTy.deep_plainFree _ (hts u hu)⟩
    | _, _, _, _, _, _, _, _, .construct _ _ _ => ⟨rfl, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .field hv hrec hi => by
      have h := latom_inPlace hΓ hv
      have hrec' := hwf.inPlace.2.1 _ (List.mem_of_find?_eq_some hrec) _ (List.mem_of_getElem? hi)
      exact ⟨LTy.raise_inPlace h hrec', Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .list _ _ ht => ⟨⟨ht, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .none ht => ⟨⟨ht, rfl⟩, Eff.plainFree_none⟩
    | _, _, _, _, _, _, hΓ, _, .some ha => ⟨⟨latom_inPlace hΓ ha, rfl⟩, Eff.plainFree_none⟩
    | _, Γ, _, _, _, _, hΓ, _, .lambda (params := params) hb _ _ hps => by
      have hΓ' : LCtx.InPlace (params ++ Γ) := by
        intro q hq
        rcases List.mem_append.mp hq with hq | hq
        · exact hps q hq
        · exact hΓ q hq
      refine ⟨⟨inPlaceAll_of_mem (fun t ht => ?_), (LExprTy.inPlace hwf hΓ' rfl hb).1, rfl⟩,
        Eff.plainFree_none⟩
      obtain ⟨q, hq, rfl⟩ := List.mem_map.mp ht
      exact hps q hq
    | _, _, _, _, _, _, _, _, .funRef (fn := fn) hf _ _ _ _ hg _ =>
      ⟨⟨(params_inPlace hwf hf _ (zip_ground hg)).1, (params_inPlace hwf hf _ (zip_ground hg)).2, rfl⟩,
        Eff.plainFree_none⟩
    | _, _, _, _, _, _, _, _, .declassify hpo _ _ _ _ _ =>
      ⟨(hwf.inPlace.2.2 _ (List.mem_of_find?_eq_some hpo)).2, Eff.plainFree_ofFnR _ _ _ _⟩
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
        exact LExpr.plainFree_join hpc (LTy.deep_plainFree _ (latom_inPlace hΓ hs))⟩
    | _, _, _, _, _, _, hΓ, _, .eq ha hb _ _ _ =>
      ⟨LExpr.plainFree_join (LTy.deep_plainFree _ (latom_inPlace hΓ ha))
        (LTy.deep_plainFree _ (latom_inPlace hΓ hb)), Eff.plainFree_none⟩

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
      exact ⟨LTy.raise_inPlace hl hs, Eff.plainFree_union
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) ht).2
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl) he).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseOption (x := x) (te := te) (ls := ls) hsc hn hsome _ _ hs => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ' : LCtx.InPlace ((x, te.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2 hl.1
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace hl.2 hs, Eff.plainFree_union
        (LExprTy.inPlace hwf hΓ (LExpr.plainFree_join hpc hl.2) hn).2
        (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl.2) hsome).2⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .caseResult (x := x) (y := y) (te := te) (ls := ls) hsc hok herr _ _ hs => by
      have hl := latom_inPlace hΓ hsc
      simp only [LTy.InPlace] at hl
      have hΓ₁ : LCtx.InPlace ((x, te.raise ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact LTy.raise_inPlace hl.2 hl.1
        · exact hΓ q hq
      have hΓ₂ : LCtx.InPlace ((y, .int ls) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact hl.2
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace hl.2 hs, Eff.plainFree_union
        (LExprTy.inPlace hwf hΓ₁ (LExpr.plainFree_join hpc hl.2) hok).2
        (LExprTy.inPlace hwf hΓ₂ (LExpr.plainFree_join hpc hl.2) herr).2⟩
    | _, _, _, _, _, _, _, _, .fail _ _ hs => ⟨hs, fun _ h => by simp at h; subst h; rfl⟩
    | _, _, _, _, _, _, hΓ, hpc, .attempt hb => by
      have h := LExprTy.inPlace hwf hΓ hpc hb
      exact ⟨⟨h.1, Eff.failLabel_plainFree h.2⟩, fun _ h => by simp at h⟩
    | _, Γ, _, _, _, _, hΓ, hpc, .handle (x := x) (ε₁ := ε₁) hb hh _ _ hs => by
      have h₁ := LExprTy.inPlace hwf hΓ hpc hb
      have hl := Eff.failLabel_plainFree h₁.2
      have hΓ' : LCtx.InPlace ((x, .int ε₁.failLabel) :: Γ) := fun q hq => by
        rcases List.mem_cons.mp hq with rfl | hq
        · exact hl
        · exact hΓ q hq
      exact ⟨LTy.raise_inPlace hl hs, (LExprTy.inPlace hwf hΓ' (LExpr.plainFree_join hpc hl) hh).2⟩
end

end Una.F1H
