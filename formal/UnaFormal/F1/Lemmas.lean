/-
Lemmas for T1 on F1a: typed environments give typed values to typed atoms, primitives keep
their result types, binding parameters keeps an environment typed.
-/
import UnaFormal.F1.Typing

namespace Una.F1

variable {p : Program}

theorem lookup_typed {x : String} {t : Ty} :
    ∀ {env : Env} {ctx : Ctx}, EnvTy p env ctx → ctxLookup ctx x = some t →
      ∃ v, lookup env x = some v ∧ ValTy p v t
  | _, _, .nil, h => by simp [ctxLookup] at h
  | _, _, .cons (x := y) hv henv, h => by
    simp only [ctxLookup] at h
    simp only [lookup]
    by_cases hx : x = y
    · simp [hx] at h ⊢
      subst h
      exact hv
    · simp [hx] at h ⊢
      exact lookup_typed henv h

theorem atom_typed {env : Env} {ctx : Ctx} {a : Atom} {t : Ty}
    (henv : EnvTy p env ctx) (h : AtomTy ctx a t) :
    ∃ v, evalAtom env a = some v ∧ ValTy p v t := by
  cases h with
  | var hx => exact lookup_typed henv hx
  | int => exact ⟨_, rfl, .int⟩
  | bool => exact ⟨_, rfl, .bool⟩
  | text => exact ⟨_, rfl, .text⟩
  | unit => exact ⟨_, rfl, .unit⟩

theorem atoms_typed {env : Env} {ctx : Ctx} {as : List Atom} {ts : List Ty}
    (henv : EnvTy p env ctx) (h : AtomsTy ctx as ts) :
    ∃ vs, evalAtoms env as = some vs ∧ ValsTy p vs ts := by
  induction h with
  | nil => exact ⟨[], rfl, .nil⟩
  | cons ha _ ih =>
    obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
    obtain ⟨vs, hvs, hvst⟩ := ih
    exact ⟨v :: vs, by simp [evalAtoms, hv, hvs], .cons hvt hvst⟩

theorem valsTy_length : ∀ {vs : List Val} {ts : List Ty}, ValsTy p vs ts → vs.length = ts.length
  | _, _, .nil => rfl
  | _, _, .cons _ h => by simp [valsTy_length h]

theorem atomsTy_length {ctx : Ctx} {as : List Atom} {ts : List Ty} (h : AtomsTy ctx as ts) :
    as.length = ts.length := by
  induction h with
  | nil => rfl
  | cons _ _ ih => simp [ih]

theorem prim_typed {pr : Prim} {ts : List Ty} {t : Ty} {vs : List Val}
    (hp : PrimTy pr ts t) (hvs : ValsTy p vs ts) :
    ∃ v, evalPrim pr vs = some v ∧ ValTy p v t := by
  cases hp <;>
  · repeat (rcases hvs with _ | ⟨hv, hvs⟩; try cases hv)
    all_goals exact ⟨_, rfl, by constructor⟩

theorem field_typed {t : Ty} :
    ∀ {fields : List Val} {ts : List Ty} {i : Nat}, ValsTy p fields ts → ts[i]? = some t →
      ∃ v, fields[i]? = some v ∧ ValTy p v t
  | _, _, _, .nil, hi => by simp at hi
  | _, _, 0, .cons hv _, hi => by simp at hi ⊢; subst hi; exact hv
  | _, _, i + 1, .cons _ hfs, hi => by simp at hi ⊢; exact field_typed hfs hi

theorem bindParams_typed {params : List (String × Ty)} {vs : List Val} {env : Env} {ctx : Ctx}
    (hvs : ValsTy p vs (params.map Prod.snd)) (henv : EnvTy p env ctx) :
    EnvTy p (bindParams params vs env) (params ++ ctx) := by
  induction params generalizing vs with
  | nil => cases hvs; simpa [bindParams] using henv
  | cons param params ih =>
    cases hvs with
    | cons hv hrest =>
      have := ih hrest
      simp only [bindParams, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
      exact .cons hv this

theorem find_wellTyped {f : String} {fn : Fun} (hwf : p.WellFormed) (h : p.find f = some fn) :
    ExprTy p fn.params fn.body fn.result :=
  hwf fn (List.mem_of_find?_eq_some h)

/-- A list of `n` copies of `t` does not depend on what the list it maps had, only on its
length. -/
theorem map_const_of_length {α β : Type} {xs : List α} {ys : List β} {t : Ty}
    (h : xs.length = ys.length) : xs.map (fun _ => t) = ys.map (fun _ => t) := by
  induction xs generalizing ys with
  | nil => cases ys <;> simp_all
  | cons _ _ ih =>
    cases ys with
    | nil => simp at h
    | cons _ _ =>
      simp only [List.length_cons, Nat.add_right_cancel_iff] at h
      simp [ih h]

end Una.F1
