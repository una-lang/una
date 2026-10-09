/-
M4: labeled values at instances. A value of a type's instance keeps that type whatever its top
label (`LValTy.setTop`), a raise commutes with instantiation up to the top label, ground types
are their own instances up to flows, and a signature's types instantiated by ground type
arguments for all its type parameters are ground.
-/
import UnaFormal.F1.Unary
import UnaFormal.F1.InPlace

namespace Una.F1

/-- The type with its top label replaced. -/
def LTy.setTop (x : LExpr) : LTy → LTy
  | .int _ => .int x
  | .bool _ => .bool x
  | .text _ => .text x
  | .unit _ => .unit x
  | .list t _ => .list t x
  | .option t _ => .option t x
  | .result t _ => .result t x
  | .record r _ => .record r x
  | .fn ps r o g f _ => .fn ps r o g f x
  | .var a _ => .var a x

theorem LTy.raise_eq_setTop (e : LExpr) (u : LTy) : u.raise e = u.setTop (u.top.join e) := by
  cases u <;> rfl

theorem LTy.setTop_setTop (x y : LExpr) (u : LTy) : (u.setTop x).setTop y = u.setTop y := by
  cases u <;> rfl

/-- Raising before instantiation is the instance with another top label. -/
theorem LTy.lsubst_raise (rd : String → Label) (σ : List (String × LTy)) (e : LExpr) (t : LTy) :
    ∃ x, (t.raise e).lsubst rd σ = (t.lsubst rd σ).setTop x := by
  cases t with
  | var a l =>
    show ∃ x, (LTy.var a (l.join e)).lsubst rd σ = ((LTy.var a l).lsubst rd σ).setTop x
    simp only [LTy.lsubst]
    cases List.lookup a σ with
    | some u =>
      refine ⟨u.top.join (((l.join e).dropPlain a).inst (σ.map (fun q => (q.1, q.2.deep rd)))), ?_⟩
      dsimp only
      rw [LTy.raise_eq_setTop _ u, LTy.raise_eq_setTop _ u, LTy.setTop_setTop]
    | none => exact ⟨_, rfl⟩
  | _ => exact ⟨_, rfl⟩

variable {p : Program}

/-- A value does not depend on its type's top label. -/
theorem LValTy.setTop {v : Val} {t : LTy} (x : LExpr) (h : LValTy p v t) : LValTy p v (t.setTop x) := by
  cases h <;> simp only [LTy.setTop] <;>
    first
    | exact .int | exact .bool | exact .text | exact .unit | exact .list ‹_› | exact .none
    | exact .some ‹_› | exact .ok ‹_› | exact .err | exact .record ‹_› ‹_›
    | exact .closure ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_› ‹_›

theorem LValTy.of_setTop {v : Val} {t : LTy} {x : LExpr} (h : LValTy p v (t.setTop x)) :
    LValTy p v t := by
  have := LValTy.setTop t.top h
  cases t <;> simpa [LTy.setTop, LTy.top] using this

/-- A value of the instance of a raised type is a value of the instance of the type. -/
theorem LValTy.lsubst_unraise {σ : List (String × LTy)} {e : LExpr} {v : Val} {t : LTy}
    (h : LValTy p v ((t.raise e).lsubst p.recordDeep σ)) : LValTy p v (t.lsubst p.recordDeep σ) := by
  obtain ⟨x, hx⟩ := LTy.lsubst_raise p.recordDeep σ e t
  rw [hx] at h
  exact LValTy.of_setTop h

theorem LValTy.lsubst_raise {σ : List (String × LTy)} {e : LExpr} {v : Val} {t : LTy}
    (h : LValTy p v (t.lsubst p.recordDeep σ)) : LValTy p v ((t.raise e).lsubst p.recordDeep σ) := by
  obtain ⟨x, hx⟩ := LTy.lsubst_raise p.recordDeep σ e t
  rw [hx]
  exact LValTy.setTop x h

/-! ### Ground types -/

theorem LExpr.le_inst_symFree {top : Label} (σ : List (String × LExpr)) {l : LExpr} (h : l.SymFree) :
    LExpr.le top (l.inst σ) l ∧ LExpr.le top l (l.inst σ) := by
  rw [LExpr.inst_symFree σ h.1 h.2]
  exact ⟨LExpr.join_le (LExpr.bot_le _) (LExpr.le_refl _), LExpr.le_join_right _ _⟩

mutual
  /-- A ground type flows both ways with any of its instances. -/
  theorem Flow.lsubst_ground {top : Label} (rd : String → Label) (σ : List (String × LTy)) :
      ∀ {u : LTy}, u.Ground → Flow top (u.lsubst rd σ) u ∧ Flow top u (u.lsubst rd σ)
    | .var _ _, h => h.elim
    | .int _, h => ⟨.int (LExpr.le_inst_symFree _ h).1, .int (LExpr.le_inst_symFree _ h).2⟩
    | .bool _, h => ⟨.bool (LExpr.le_inst_symFree _ h).1, .bool (LExpr.le_inst_symFree _ h).2⟩
    | .text _, h => ⟨.text (LExpr.le_inst_symFree _ h).1, .text (LExpr.le_inst_symFree _ h).2⟩
    | .unit _, _ => ⟨.unit, .unit⟩
    | .record _ _, h => ⟨.record (LExpr.le_inst_symFree _ h).1, .record (LExpr.le_inst_symFree _ h).2⟩
    | .list _ _, h =>
      ⟨.list (Flow.lsubst_ground rd σ h.1).1 (LExpr.le_inst_symFree _ h.2).1,
        .list (Flow.lsubst_ground rd σ h.1).2 (LExpr.le_inst_symFree _ h.2).2⟩
    | .option _ _, h =>
      ⟨.option (Flow.lsubst_ground rd σ h.1).1 (LExpr.le_inst_symFree _ h.2).1,
        .option (Flow.lsubst_ground rd σ h.1).2 (LExpr.le_inst_symFree _ h.2).2⟩
    | .result _ _, h =>
      ⟨.result (Flow.lsubst_ground rd σ h.1).1 (LExpr.le_inst_symFree _ h.2).1,
        .result (Flow.lsubst_ground rd σ h.1).2 (LExpr.le_inst_symFree _ h.2).2⟩
    | .fn _ _ _ _ _ _, h =>
      ⟨.fn (Flows.lsubst_ground rd σ h.1).2 (Flow.lsubst_ground rd σ h.2.1).1 id id id
          (LExpr.le_inst_symFree _ h.2.2).1,
        .fn (Flows.lsubst_ground rd σ h.1).1 (Flow.lsubst_ground rd σ h.2.1).2 id id id
          (LExpr.le_inst_symFree _ h.2.2).2⟩

  theorem Flows.lsubst_ground {top : Label} (rd : String → Label) (σ : List (String × LTy)) :
      ∀ {us : List LTy}, LTy.GroundAll us →
        Flows top (LTy.lsubsts rd σ us) us ∧ Flows top us (LTy.lsubsts rd σ us)
    | [], _ => ⟨.nil, .nil⟩
    | _ :: _, h =>
      ⟨.cons (Flow.lsubst_ground rd σ h.1).1 (Flows.lsubst_ground rd σ h.2).1,
        .cons (Flow.lsubst_ground rd σ h.1).2 (Flows.lsubst_ground rd σ h.2).2⟩
end

theorem LExpr.symFree_join {a b : LExpr} (ha : a.SymFree) (hb : b.SymFree) : (a.join b).SymFree := by
  simp [LExpr.SymFree, LExpr.join, ha.1, ha.2, hb.1, hb.2]

theorem LExpr.symFree_joinAll : ∀ {es : List LExpr}, (∀ e ∈ es, e.SymFree) → (LExpr.joinAll es).SymFree
  | [], _ => ⟨rfl, rfl⟩
  | e :: _, h => symFree_join (h e List.mem_cons_self)
      (symFree_joinAll (fun x hx => h x (List.mem_cons_of_mem _ hx)))

mutual
  theorem LTy.Ground.deep (rd : String → Label) : ∀ {u : LTy}, u.Ground → (u.deep rd).SymFree
    | .var _ _, h => h.elim
    | .int _, h | .bool _, h | .text _, h | .unit _, h => h
    | .record _ _, h => LExpr.symFree_join h ⟨rfl, rfl⟩
    | .list _ _, h | .option _ _, h | .result _ _, h =>
      LExpr.symFree_join h.2 (LTy.Ground.deep rd h.1)
    | .fn _ _ _ _ _ _, h =>
      LExpr.symFree_join h.2.2 (LExpr.symFree_join (LTy.GroundAll.deeps rd h.1) (LTy.Ground.deep rd h.2.1))

  theorem LTy.GroundAll.deeps (rd : String → Label) : ∀ {us : List LTy}, LTy.GroundAll us →
      (LTy.deeps rd us).SymFree
    | [], _ => ⟨rfl, rfl⟩
    | _ :: _, h => LExpr.symFree_join (LTy.Ground.deep rd h.1) (LTy.GroundAll.deeps rd h.2)
end

theorem LTy.Ground.raise {e : LExpr} (he : e.SymFree) : ∀ {u : LTy}, u.Ground → (u.raise e).Ground
  | .var _ _, h => h.elim
  | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h => LExpr.symFree_join h he
  | .list _ _, h | .option _ _, h | .result _ _, h => ⟨h.1, LExpr.symFree_join h.2 he⟩
  | .fn _ _ _ _ _ _, h => ⟨h.1, h.2.1, LExpr.symFree_join h.2.2 he⟩

/-- A label whose symbols an instantiation all covers, by ground type arguments, has none left. -/
theorem LExpr.inst_symFree_of_covered (rd : String → Label) {σ : List (String × LTy)}
    (hσ : GroundSubst σ) {xs : List String} (hcov : ∀ x ∈ xs, ∃ u, List.lookup x σ = some u)
    {l : LExpr} (hl : l.SymsIn xs) : (l.inst (σ.map (fun q => (q.1, q.2.deep rd)))).SymFree := by
  have hcov' : ∀ x ∈ l.plain ++ l.star, (List.lookup x (σ.map (fun q => (q.1, q.2.deep rd)))).isSome := by
    intro x hx
    obtain ⟨u, hu⟩ := hcov x (hl x hx)
    simp [lookup_map, hu]
  have hhit : (LExpr.joinAll ((l.plain ++ l.star).filterMap
      (fun x => List.lookup x (σ.map (fun q => (q.1, q.2.deep rd)))))).SymFree := by
    apply LExpr.symFree_joinAll
    intro e he
    obtain ⟨x, _, hx⟩ := List.mem_filterMap.mp he
    rw [lookup_map] at hx
    obtain ⟨u, hu, rfl⟩ := Option.map_eq_some_iff.mp hx
    exact LTy.Ground.deep rd (hσ _ (mem_of_lookup hu))
  refine LExpr.symFree_join hhit ⟨?_, ?_⟩
  · apply List.eq_nil_iff_forall_not_mem.mpr
    intro x hx
    obtain ⟨hx, hnc⟩ := List.mem_filter.mp hx
    have := hcov' x (List.mem_append_left _ hx)
    simp_all
  · apply List.eq_nil_iff_forall_not_mem.mpr
    intro x hx
    obtain ⟨hx, hnc⟩ := List.mem_filter.mp hx
    have := hcov' x (List.mem_append_right _ hx)
    simp_all

theorem LExpr.symsIn_dropPlain {xs : List String} (a : String) {l : LExpr} (h : l.SymsIn xs) :
    (l.dropPlain a).SymsIn xs := by
  intro x hx
  simp only [LExpr.dropPlain, List.mem_append, List.mem_filter] at hx
  rcases hx with ⟨hx, _⟩ | hx
  · exact h x (List.mem_append_left _ hx)
  · exact h x (List.mem_append_right _ hx)

mutual
  /-- **A signature's type, instantiated for all its type parameters by ground type arguments,
  is ground.** -/
  theorem LTy.lsubst_ground (rd : String → Label) {σ : List (String × LTy)} (hσ : GroundSubst σ)
      {xs : List String} (hcov : ∀ x ∈ xs, ∃ u, List.lookup x σ = some u) :
      ∀ {t : LTy}, t.ScopedIn xs → (t.lsubst rd σ).Ground
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h =>
      LExpr.inst_symFree_of_covered rd hσ hcov h
    | .list _ _, h | .option _ _, h | .result _ _, h =>
      ⟨LTy.lsubst_ground rd hσ hcov h.1, LExpr.inst_symFree_of_covered rd hσ hcov h.2⟩
    | .fn _ _ _ _ _ _, h =>
      ⟨LTy.lsubsts_ground rd hσ hcov h.1, LTy.lsubst_ground rd hσ hcov h.2.1,
        LExpr.inst_symFree_of_covered rd hσ hcov h.2.2⟩
    | .var a l, h => by
      obtain ⟨u, hu⟩ := hcov a h.1
      simp only [LTy.lsubst, hu]
      exact LTy.Ground.raise
        (LExpr.inst_symFree_of_covered rd hσ hcov (LExpr.symsIn_dropPlain a h.2))
        (hσ _ (mem_of_lookup hu))

  theorem LTy.lsubsts_ground (rd : String → Label) {σ : List (String × LTy)} (hσ : GroundSubst σ)
      {xs : List String} (hcov : ∀ x ∈ xs, ∃ u, List.lookup x σ = some u) :
      ∀ {ts : List LTy}, LTy.ScopedInAll xs ts → LTy.GroundAll (LTy.lsubsts rd σ ts)
    | [], _ => trivial
    | _ :: _, h => ⟨LTy.lsubst_ground rd hσ hcov h.1, LTy.lsubsts_ground rd hσ hcov h.2⟩
end

/-- Every type parameter has its type argument. -/
theorem lookup_zip_cover : ∀ {xs : List String} {ts : List LTy}, ts.length = xs.length →
    ∀ x ∈ xs, ∃ u, List.lookup x (xs.zip ts) = some u
  | [], _, _ => fun _ h => nomatch h
  | y :: ys, t :: ts, hl => by
    intro x hx
    simp only [List.zip_cons_cons, List.lookup]
    by_cases hxy : x = y
    · exact ⟨t, by simp [hxy]⟩
    · simp only [show (x == y) = false from by simp [hxy]]
      exact lookup_zip_cover (by simpa using hl) x
        (by rcases List.mem_cons.mp hx with h | h; exact absurd h hxy; exact h)
  | _ :: _, [], hl => by simp at hl

end Una.F1
