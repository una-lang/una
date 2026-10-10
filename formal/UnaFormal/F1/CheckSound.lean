/-
M4 completion, part A1 (`docs/plan/formal-track-m4-completion.md`): the checker of M2
(`Check.lean`) is sound for the labeled typing. This file holds the base facts: each Boolean test
of the checker implies the proposition of `LTyping.lean` it stands for.
-/
import UnaFormal.F1.Check
import UnaFormal.F1.Order
import UnaFormal.F1.B1

namespace Una.F1.Check

theorem labelLe_sound {a b : Label} (h : labelLe a b = true) : a.le b := by
  simp only [labelLe, Bool.and_eq_true, decide_eq_true_eq, List.all_eq_true,
    List.contains_iff_mem] at h
  exact ⟨h.1, h.2⟩

theorem leB_sound {top : Label} {a b : LExpr} (h : leB top a b = true) : LExpr.le top a b := by
  simp only [leB, Bool.or_eq_true, Bool.and_eq_true, List.all_eq_true, List.contains_iff_mem]
    at h
  rcases h with h | ⟨⟨hb, hp⟩, hs⟩
  · exact .inl (labelLe_sound h)
  · exact .inr ⟨labelLe_sound hb, fun x hx => by simpa using hp x hx, fun x hx => by simpa using hs x hx⟩

theorem symFree_sound {e : LExpr} (h : symFree e = true) : e.SymFree := by
  simp only [symFree, Bool.and_eq_true, List.isEmpty_iff] at h
  exact h

mutual
  theorem tyEq_sound : ∀ (a b : Ty), tyEq a b = true → a = b
    | .int, b, h | .bool, b, h | .text, b, h | .unit, b, h => by cases b <;> simp_all [tyEq]
    | .record r, b, h | .var r, b, h => by cases b <;> simp_all [tyEq]
    | .list a, b, h => by
      cases b with
      | list b => simp only [tyEq] at h; rw [tyEq_sound a b h]
      | _ => simp [tyEq] at h
    | .option a, b, h => by
      cases b with
      | option b => simp only [tyEq] at h; rw [tyEq_sound a b h]
      | _ => simp [tyEq] at h
    | .result a, b, h => by
      cases b with
      | result b => simp only [tyEq] at h; rw [tyEq_sound a b h]
      | _ => simp [tyEq] at h
    | .fn ps r, b, h => by
      cases b with
      | fn qs s =>
        simp only [tyEq, Bool.and_eq_true] at h
        rw [tysEq_sound ps qs h.1, tyEq_sound r s h.2]
      | _ => simp [tyEq] at h
    | .tuple ps, b, h => by
      cases b with
      | tuple qs => simp only [tyEq] at h; rw [tysEq_sound ps qs h]
      | _ => simp [tyEq] at h

  theorem tysEq_sound : ∀ (as bs : List Ty), tysEq as bs = true → as = bs
    | [], [], _ => rfl
    | a :: as, b :: bs, h => by
      simp only [tysEq, Bool.and_eq_true] at h; rw [tyEq_sound a b h.1, tysEq_sound as bs h.2]
    | [], _ :: _, h | _ :: _, [], h => by simp [tysEq] at h
end

theorem httpRecordsB_sound {p : Program} (h : httpRecordsB p = true) : p.HttpRecords := by
  unfold httpRecordsB at h
  split at h
  · rename_i hr rr hh hrr
    simp only [Bool.and_eq_true] at h
    exact ⟨⟨hr, hh, tysEq_sound _ _ h.1⟩, ⟨rr, hrr, tysEq_sound _ _ h.2⟩⟩
  · simp at h

theorem hostLeB_sound {a b : HostFx} (h : hostLeB a b = true) : HostFx.Le a b := by
  simp only [hostLeB, Bool.and_eq_true, Bool.or_eq_true, Bool.not_eq_true'] at h
  obtain ⟨⟨h₁, h₂⟩, h₃⟩ := h
  refine ⟨fun h => ?_, fun h => ?_, fun h => ?_⟩
  · rcases h₁ with h' | h' <;> simp_all
  · rcases h₂ with h' | h' <;> simp_all
  · rcases h₃ with h' | h' <;> simp_all

mutual
  theorem flowB_sound {top : Label} (a b : LTy) (h : flowB top a b = true) : Flow top a b := by
    rw [flowB.eq_def] at h
    split at h <;> (try simp only [Bool.and_eq_true, beq_iff_eq, Bool.or_eq_true, Bool.not_eq_true']
      at h)
    · exact .int (leB_sound h)
    · exact .bool (leB_sound h)
    · exact .text (leB_sound h)
    · exact .unit
    · exact .list (flowB_sound _ _ h.1) (leB_sound h.2)
    · exact .option (flowB_sound _ _ h.1) (leB_sound h.2)
    · exact .result (flowB_sound _ _ h.1) (leB_sound h.2)
    · obtain ⟨rfl, h⟩ := h; exact .record (leB_sound h)
    · obtain ⟨⟨⟨⟨⟨hps, hr⟩, ho⟩, hg⟩, hf⟩, hl⟩ := h
      refine .fn (flowsB_sound _ _ hps) (flowB_sound _ _ hr) ?_ (hostLeB_sound hg) ?_
        (leB_sound hl)
      · intro h; cases ho with | inl h' => simp_all | inr h' => exact h'
      · intro h; cases hf with | inl h' => simp_all | inr h' => exact h'
    · obtain ⟨rfl, h⟩ := h; exact .var (leB_sound h)
    · exact .tuple (flowsB_sound _ _ h.1) (leB_sound h.2)
    · exact absurd h (by simp)
  termination_by sizeOf a + sizeOf b

  theorem flowsB_sound {top : Label} (as bs : List LTy) (h : flowsB top as bs = true) :
      Flows top as bs := by
    rw [flowsB.eq_def] at h
    split at h
    · exact .nil
    · simp only [Bool.and_eq_true] at h
      exact .cons (flowB_sound _ _ h.1) (flowsB_sound _ _ h.2)
    · exact absurd h (by simp)
  termination_by sizeOf as + sizeOf bs
end

mutual
  theorem inPlace_sound : ∀ {t : LTy}, inPlace t = true → t.InPlace
    | .var a l, h => by
      simp only [inPlace, List.all_eq_true, beq_iff_eq] at h
      exact h
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => by
      simpa [inPlace, LTy.InPlace] using h
    | .list _ _, h | .option _ _, h | .result _ _, h => by
      simp only [inPlace, Bool.and_eq_true, List.isEmpty_iff] at h
      exact ⟨inPlace_sound h.1, h.2⟩
    | .fn _ _ _ _ _ _, h => by
      simp only [inPlace, Bool.and_eq_true, List.isEmpty_iff] at h
      exact ⟨inPlaceAll_sound h.1.1, inPlace_sound h.1.2, h.2⟩
    | .tuple _ _, h => by
      simp only [inPlace, Bool.and_eq_true, List.isEmpty_iff] at h
      exact ⟨inPlaceAll_sound h.1, h.2⟩

  theorem inPlaceAll_sound : ∀ {ts : List LTy}, inPlaceAll ts = true → LTy.InPlaceAll ts
    | [], _ => trivial
    | _ :: _, h => by
      simp only [inPlaceAll, Bool.and_eq_true] at h
      exact ⟨inPlace_sound h.1, inPlaceAll_sound h.2⟩
end

mutual
  theorem ground_sound : ∀ {t : LTy}, ground t = true → t.Ground
    | .var _ _, h => by simp [ground] at h
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => symFree_sound h
    | .list _ _, h | .option _ _, h | .result _ _, h => by
      simp only [ground, Bool.and_eq_true] at h
      exact ⟨ground_sound h.1, symFree_sound h.2⟩
    | .fn _ _ _ _ _ _, h => by
      simp only [ground, Bool.and_eq_true] at h
      exact ⟨groundAll_sound h.1.1, ground_sound h.1.2, symFree_sound h.2⟩
    | .tuple _ _, h => by
      simp only [ground, Bool.and_eq_true] at h
      exact ⟨groundAll_sound h.1, symFree_sound h.2⟩

  theorem groundAll_sound : ∀ {ts : List LTy}, groundAll ts = true → LTy.GroundAll ts
    | [], _ => trivial
    | _ :: _, h => by
      simp only [groundAll, Bool.and_eq_true] at h
      exact ⟨ground_sound h.1, groundAll_sound h.2⟩
end

theorem symsIn_sound {xs : List String} {e : LExpr} (h : symsIn xs e = true) : e.SymsIn xs := by
  simp only [symsIn, List.all_eq_true, List.contains_iff_mem] at h
  exact fun x hx => by simpa using h x hx

mutual
  theorem scopedIn_sound {xs : List String} : ∀ {t : LTy}, scopedIn xs t = true → t.ScopedIn xs
    | .var _ _, h => by
      simp only [scopedIn, Bool.and_eq_true, List.contains_iff_mem] at h
      exact ⟨by simpa using h.1, symsIn_sound h.2⟩
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => symsIn_sound h
    | .list _ _, h | .option _ _, h | .result _ _, h => by
      simp only [scopedIn, Bool.and_eq_true] at h
      exact ⟨scopedIn_sound h.1, symsIn_sound h.2⟩
    | .fn _ _ _ _ _ _, h => by
      simp only [scopedIn, Bool.and_eq_true] at h
      exact ⟨scopedInAll_sound h.1.1, scopedIn_sound h.1.2, symsIn_sound h.2⟩
    | .tuple _ _, h => by
      simp only [scopedIn, Bool.and_eq_true] at h
      exact ⟨scopedInAll_sound h.1, symsIn_sound h.2⟩

  theorem scopedInAll_sound {xs : List String} :
      ∀ {ts : List LTy}, scopedInAll xs ts = true → LTy.ScopedInAll xs ts
    | [], _ => trivial
    | _ :: _, h => by
      simp only [scopedInAll, Bool.and_eq_true] at h
      exact ⟨scopedIn_sound h.1, scopedInAll_sound h.2⟩
end

theorem boundB_sound {p : Program} {pc : LExpr} {o : Bool} {g : HostFx} {f : Bool}
    (h : boundB p pc o g f = true) : p.Bound pc o g f := by
  simp only [boundB, Bool.and_eq_true, Bool.or_eq_true, Bool.not_eq_true'] at h
  obtain ⟨⟨⟨⟨ho, hr⟩, hw⟩, hq⟩, hf⟩ := h
  refine ⟨fun h => ?_, fun h => ?_, fun h => ?_, fun h => ?_, fun h => ?_⟩
  · rcases ho with h' | ⟨h₁, h₂⟩
    · simp_all
    · exact ⟨leB_sound h₁, symFree_sound h₂⟩
  · rcases hr with h' | ⟨h₁, h₂⟩
    · simp_all
    · exact ⟨leB_sound h₁, symFree_sound h₂⟩
  · rcases hw with h' | ⟨⟨h₁, h₂⟩, h₃⟩
    · simp_all
    · refine ⟨leB_sound h₁, fun ho => ?_, symFree_sound h₃⟩
      rcases h₂ with h₂ | h₂
      · simp_all
      · exact leB_sound h₂
  · rcases hq with h' | ⟨h₁, h₂⟩
    · simp_all
    · exact ⟨leB_sound h₁, symFree_sound h₂⟩
  · rcases hf with h' | ⟨h₁, h₂⟩
    · simp_all
    · exact ⟨leB_sound h₁, symFree_sound h₂⟩

theorem relBoundB_sound {p : Program} {pc : LExpr} {rs : List String}
    (h : relBoundB p pc rs = true) : p.RelBound pc rs := by
  intro pol hpol
  simp only [relBoundB, List.all_eq_true] at h
  have := h pol hpol
  split at this
  · rename_i po hpo
    simp only [Bool.and_eq_true] at this
    exact ⟨po, hpo, leB_sound this.1, symFree_sound this.2⟩
  · exact absurd this (by simp)

theorem atomTy_sound {Γ : LCtx} {a : Atom} {t : LTy} (h : atomTy Γ a = .ok t) : LAtomTy Γ a t := by
  cases a with
  | var x =>
    simp only [atomTy, some!] at h
    split at h
    · rename_i t' ht
      cases h
      exact .var ht
    · exact absurd h (by simp)
  | int _ => cases h; exact .int
  | bool _ => cases h; exact .bool
  | text _ => cases h; exact .text
  | unit => cases h; exact .unit

theorem atomsTy_sound {Γ : LCtx} : ∀ {as : List Atom} {ts : List LTy}, atomsTy Γ as = .ok ts →
    LAtomsTy Γ as ts
  | [], ts, h => by
    simp only [atomsTy, List.mapM_nil, pure, Except.pure] at h
    cases h
    exact .nil
  | a :: as, ts, h => by
    simp only [atomsTy, List.mapM_cons, bind, Except.bind, pure, Except.pure] at h
    split at h
    · exact absurd h (by simp)
    · rename_i t ht
      split at h
      · exact absurd h (by simp)
      · rename_i ts' hts
        cases h
        exact .cons (atomTy_sound ht) (atomsTy_sound hts)

theorem primTy_sound {pr : Prim} {ts : List Ty} {t : Ty} (h : primTy pr ts = some t) :
    PrimTy pr ts t := by
  unfold primTy at h
  split at h
  all_goals first
    | (cases h; done)
    | (cases h; constructor)
    | (split at h
       · rename_i hall
         cases h
         exact .concat fun u hu => tyEq_sound _ _ (List.all_eq_true.mp hall u hu)
       · cases h)

/-- The type `joinTy` gives both operands of `==` flows into. -/
theorem joinTy_sound {top : Label} : ∀ {t₁ t₂ t : LTy}, joinTy t₁ t₂ = some t →
    Flow top t₁ t ∧ Flow top t₂ t
  | .int _, .int _, _, h | .bool _, .bool _, _, h | .text _, .text _, _, h | .unit _, .unit _, _, h => by
    simp only [joinTy, Option.some.injEq] at h
    subst h
    first
      | exact ⟨.int (LExpr.le_join_left _ _), .int (LExpr.le_join_right _ _)⟩
      | exact ⟨.bool (LExpr.le_join_left _ _), .bool (LExpr.le_join_right _ _)⟩
      | exact ⟨.text (LExpr.le_join_left _ _), .text (LExpr.le_join_right _ _)⟩
      | exact ⟨.unit, .unit⟩
  | .list _ _, .list _ _, _, h | .option _ _, .option _ _, _, h | .result _ _, .result _ _, _, h => by
    simp only [joinTy, Option.map_eq_some_iff] at h
    obtain ⟨u, hu, rfl⟩ := h
    obtain ⟨h₁, h₂⟩ := joinTy_sound hu
    first
      | exact ⟨.list h₁ (LExpr.le_join_left _ _), .list h₂ (LExpr.le_join_right _ _)⟩
      | exact ⟨.option h₁ (LExpr.le_join_left _ _), .option h₂ (LExpr.le_join_right _ _)⟩
      | exact ⟨.result h₁ (LExpr.le_join_left _ _), .result h₂ (LExpr.le_join_right _ _)⟩
  | .record _ _, .record _ _, _, h => by
    simp only [joinTy] at h
    split at h
    · rename_i hr
      simp only [beq_iff_eq] at hr
      subst hr
      cases h
      exact ⟨.record (LExpr.le_join_left _ _), .record (LExpr.le_join_right _ _)⟩
    · cases h
  | .var _ _, .var _ _, _, h => by
    simp only [joinTy] at h
    split at h
    · rename_i hr
      simp only [beq_iff_eq] at hr
      subst hr
      cases h
      exact ⟨.var (LExpr.le_join_left _ _), .var (LExpr.le_join_right _ _)⟩
    · cases h
  | .int _, .bool _, _, h | .int _, .text _, _, h | .int _, .unit _, _, h | .int _, .list _ _, _, h
  | .int _, .option _ _, _, h | .int _, .result _ _, _, h | .int _, .record _ _, _, h
  | .int _, .fn _ _ _ _ _ _, _, h | .int _, .var _ _, _, h => by simp [joinTy] at h
  | .bool _, .int _, _, h | .bool _, .text _, _, h | .bool _, .unit _, _, h | .bool _, .list _ _, _, h
  | .bool _, .option _ _, _, h | .bool _, .result _ _, _, h | .bool _, .record _ _, _, h
  | .bool _, .fn _ _ _ _ _ _, _, h | .bool _, .var _ _, _, h => by simp [joinTy] at h
  | .text _, .int _, _, h | .text _, .bool _, _, h | .text _, .unit _, _, h | .text _, .list _ _, _, h
  | .text _, .option _ _, _, h | .text _, .result _ _, _, h | .text _, .record _ _, _, h
  | .text _, .fn _ _ _ _ _ _, _, h | .text _, .var _ _, _, h => by simp [joinTy] at h
  | .unit _, .int _, _, h | .unit _, .bool _, _, h | .unit _, .text _, _, h | .unit _, .list _ _, _, h
  | .unit _, .option _ _, _, h | .unit _, .result _ _, _, h | .unit _, .record _ _, _, h
  | .unit _, .fn _ _ _ _ _ _, _, h | .unit _, .var _ _, _, h => by simp [joinTy] at h
  | .list _ _, .int _, _, h | .list _ _, .bool _, _, h | .list _ _, .text _, _, h
  | .list _ _, .unit _, _, h | .list _ _, .option _ _, _, h | .list _ _, .result _ _, _, h
  | .list _ _, .record _ _, _, h | .list _ _, .fn _ _ _ _ _ _, _, h | .list _ _, .var _ _, _, h => by
    simp [joinTy] at h
  | .option _ _, .int _, _, h | .option _ _, .bool _, _, h | .option _ _, .text _, _, h
  | .option _ _, .unit _, _, h | .option _ _, .list _ _, _, h | .option _ _, .result _ _, _, h
  | .option _ _, .record _ _, _, h | .option _ _, .fn _ _ _ _ _ _, _, h
  | .option _ _, .var _ _, _, h => by simp [joinTy] at h
  | .result _ _, .int _, _, h | .result _ _, .bool _, _, h | .result _ _, .text _, _, h
  | .result _ _, .unit _, _, h | .result _ _, .list _ _, _, h | .result _ _, .option _ _, _, h
  | .result _ _, .record _ _, _, h | .result _ _, .fn _ _ _ _ _ _, _, h
  | .result _ _, .var _ _, _, h => by simp [joinTy] at h
  | .record _ _, .int _, _, h | .record _ _, .bool _, _, h | .record _ _, .text _, _, h
  | .record _ _, .unit _, _, h | .record _ _, .list _ _, _, h | .record _ _, .option _ _, _, h
  | .record _ _, .result _ _, _, h | .record _ _, .fn _ _ _ _ _ _, _, h
  | .record _ _, .var _ _, _, h => by simp [joinTy] at h
  | .fn _ _ _ _ _ _, _, _, h => by simp [joinTy] at h
  | .var _ _, .int _, _, h | .var _ _, .bool _, _, h | .var _ _, .text _, _, h
  | .var _ _, .unit _, _, h | .var _ _, .list _ _, _, h | .var _ _, .option _ _, _, h
  | .var _ _, .result _ _, _, h | .var _ _, .record _ _, _, h
  | .var _ _, .fn _ _ _ _ _ _, _, h => by simp [joinTy] at h

theorem isScalar_sound {t : LTy} (h : isScalar t = true) : t.erase.Scalar := by
  cases t <;> simp_all [isScalar, LTy.erase, Ty.Scalar]

theorem isText_sound {t : LTy} (h : isText t = true) : t.erase = .text := by
  cases t <;> simp_all [isText, LTy.erase]

theorem eqArgs_sound {p : Program} {E : List String} {fn : Fun} {targs : List LTy}
    (h : eqArgs p E fn targs = true) : fn.EqArgs p E targs := by
  intro q hq hE
  simp only [eqArgs, List.all_eq_true, Bool.or_eq_true, Bool.not_eq_true'] at h
  rcases h q hq with h' | h'
  · simp_all
  · exact h'

theorem isVia_false {p : Program} {f : String} (h : isVia p f = false) : ¬ p.IsVia f := by
  rintro ⟨po, hpo, rfl⟩
  simp only [isVia, List.any_eq_false, beq_iff_eq] at h
  exact h po hpo rfl

/-! ### Taking the checker's steps apart -/

theorem bind_ok {α β : Type} {x : Result α} {f : α → Result β} {b : β} :
    (x >>= f) = .ok b ↔ ∃ a, x = .ok a ∧ f a = .ok b := by
  cases x <;> simp [bind, Except.bind]

theorem need_ok {c : Bool} {m : String} {u : Unit} : need c m = .ok u ↔ c = true := by
  cases u
  cases c <;> simp [need, pure, Except.pure, throw, throwThe, MonadExceptOf.throw]

theorem some_ok {α : Type} {o : Option α} {m : String} {a : α} :
    some! o m = .ok a ↔ o = some a := by
  cases o <;> simp [some!, pure, Except.pure, throw, throwThe, MonadExceptOf.throw]

theorem flowTo_ok {p : Program} {t ann : LTy} {w : String} {u : Unit} (h : flowTo p t ann w = .ok u) :
    Flow p.top t ann :=
  flowB_sound _ _ (need_ok.mp h)

theorem le_ok {p : Program} {a b : LExpr} {w : String} {u : Unit} (h : le p a b w = .ok u) : p.le a b :=
  leB_sound (need_ok.mp h)

theorem pure_ok {α : Type} {a b : α} : (pure a : Result α) = .ok b ↔ a = b := by
  simp [pure, Except.pure]

@[simp] theorem throw_ok {α : Type} {m : String} {a : α} :
    (throw m : Result α) = .ok a ↔ False := by
  simp [throw, throwThe, MonadExceptOf.throw]

/-- The premises a call and a function reference share. -/
theorem callable_sound {p : Program} {E : List String} {f : String} {targs : List LTy} {fn : Fun}
    (h : callable p E f targs = .ok fn) :
    p.find f = some fn ∧ ¬ p.IsVia f ∧ targs.length = fn.tparams.length ∧
      LTy.firstClasses targs = true ∧ fn.EqArgs p E targs ∧ LTy.GroundAll targs := by
  simp only [callable, bind_ok, need_ok, some_ok, pure_ok, Bool.not_eq_true', beq_iff_eq] at h
  obtain ⟨fn', hf, _, hv, _, hl, _, hfc, _, he, _, hg, rfl⟩ := h
  exact ⟨hf, isVia_false hv, hl, hfc, eqArgs_sound he, groundAll_sound hg⟩

end Una.F1.Check
