/-
Lemmas for T1 on F1: typings are closed under substitution of type arguments; typed environments
give typed values to typed atoms; primitives keep their result types; binding parameters keeps
an environment typed.
-/
import UnaFormal.F1.Typing

namespace Una.F1

variable {p : Program}

/-! ### Substitution -/

theorem ctxLookup_subst (τ : List (String × Ty)) (x : String) :
    ∀ ctx : Ctx, ctxLookup (ctx.subst τ) x = (ctxLookup ctx x).map (Ty.subst τ)
  | [] => rfl
  | (y, t) :: ctx => by
    simp only [Ctx.subst, List.map_cons, ctxLookup]
    split
    · rfl
    · exact ctxLookup_subst τ x ctx

theorem atom_subst (τ : List (String × Ty)) {ctx : Ctx} {a : Atom} {t : Ty}
    (h : AtomTy ctx a t) : AtomTy (ctx.subst τ) a (t.subst τ) := by
  cases h with
  | var hx => exact .var (by rw [ctxLookup_subst, hx]; rfl)
  | int => exact .int
  | bool => exact .bool
  | text => exact .text
  | unit => exact .unit

theorem atoms_subst (τ : List (String × Ty)) {ctx : Ctx} {as : List Atom} {ts : List Ty}
    (h : AtomsTy ctx as ts) : AtomsTy (ctx.subst τ) as (ts.map (Ty.subst τ)) := by
  induction h with
  | nil => exact .nil
  | cons ha _ ih => exact .cons (atom_subst τ ha) ih

theorem source_subst (τ : List (String × Ty)) {ctx : Ctx} {s : Source} {t : Ty}
    (h : SourceTy ctx s t) : SourceTy (ctx.subst τ) s (t.subst τ) := by
  cases h with
  | list hx => exact .list (by simpa [Ty.subst] using atom_subst τ hx)
  | range hl hh => exact .range (atom_subst τ hl) (atom_subst τ hh)

theorem prim_subst (τ : List (String × Ty)) {pr : Prim} {ts : List Ty} {t : Ty}
    (h : PrimTy pr ts t) : PrimTy pr (ts.map (Ty.subst τ)) (t.subst τ) := by
  cases h with
  | concat hts =>
    refine .concat fun u hu => ?_
    obtain ⟨t, ht, rfl⟩ := List.mem_map.mp hu
    rw [hts t ht]; rfl
  | _ => simp only [List.map, Ty.subst]; constructor

theorem bindTys_subst {α : Type} (τ : List (String × Ty)) (params : List (String × α))
    (ts : List Ty) (ctx : Ctx) :
    Ctx.subst τ (bindTys params ts ++ ctx) = bindTys params (ts.map (Ty.subst τ)) ++ ctx.subst τ := by
  simp only [Ctx.subst, bindTys, List.map_append, List.zip_map_right]
  rfl

theorem paramTys_subst (fn : Fun) (σ τ : List (String × Ty)) :
    (fn.paramTys σ).map (Ty.subst τ) = fn.paramTys (σ.map (fun q => (q.1, q.2.subst τ)) ++ τ) := by
  simp only [Fun.paramTys, List.map_map]
  apply List.map_congr_left
  intro q _
  exact Ty.subst_subst σ τ _

theorem map_subst_closed (τ : List (String × Ty)) :
    ∀ {ts : List Ty}, (∀ t ∈ ts, t.Closed) → ts.map (Ty.subst τ) = ts
  | [], _ => rfl
  | t :: ts, h => by
    rw [List.map_cons, h t List.mem_cons_self τ,
      map_subst_closed τ (fun u hu => h u (List.mem_cons_of_mem _ hu))]

theorem record_mem {r : String} {rec : Record} (h : p.record r = some rec) : rec ∈ p.records :=
  List.mem_of_find?_eq_some h

theorem policy_mem {pol : String} {po : Policy} (h : p.policy pol = some po) :
    po ∈ p.policies :=
  List.mem_of_find?_eq_some h

theorem mem_of_getElem? {α : Type} {xs : List α} {i : Nat} {x : α} (h : xs[i]? = some x) :
    x ∈ xs := by
  obtain ⟨hlt, heq⟩ := List.getElem?_eq_some_iff.mp h
  exact heq ▸ List.getElem_mem hlt

/-- The shapes of `Http`'s parameters: `Text`, `Option[Text]` and `List[Header]`. -/
def Ty.HttpArg (t : Ty) : Prop := t = .text ∨ t = .option .text ∨ t = .list (.record "prelude.Header")

theorem httpParams_args {m : String} {ts : List Ty} (h : httpParams m = some ts) :
    ∀ t ∈ ts, t.HttpArg := by
  intro t ht
  unfold httpParams at h
  split at h
  all_goals first
    | (cases h; simp at ht; rcases ht with h | h | h | h <;> subst h <;> simp [Ty.HttpArg])
    | (cases h; simp at ht; rcases ht with h | h | h <;> subst h <;> simp [Ty.HttpArg])
    | (cases h)

theorem Ty.HttpArg.subst {t : Ty} (h : t.HttpArg) (τ : List (String × Ty)) : t.subst τ = t := by
  rcases h with rfl | rfl | rfl <;> simp [Ty.subst]

/-- `Http`'s parameter types have no type variable: substitution keeps them. -/
theorem httpParams_subst (τ : List (String × Ty)) {m : String} {ts : List Ty}
    (h : httpParams m = some ts) : ts.map (Ty.subst τ) = ts := by
  unfold httpParams at h
  split at h
  all_goals first
    | (cases h; simp [Ty.subst])
    | (cases h)

mutual
  /-- A typing of a right-hand side is closed under substitution. -/
  theorem RhsTy.subst (hwf : p.WellFormed) (τ : List (String × Ty)) :
      ∀ {ctx : Ctx} {r : Rhs} {t : Ty}, RhsTy p ctx r t → RhsTy p (ctx.subst τ) r (t.subst τ)
    | _, _, _, .atom h => .atom (atom_subst τ h)
    | _, _, _, .prim has hp => .prim (atoms_subst τ has) (prim_subst τ hp)
    | _, _, _, .call (fn := fn) (σ := σ) hf has => by
      have h := atoms_subst τ has
      rw [paramTys_subst] at h
      rw [Ty.subst_subst]
      exact .call hf h
    | _, _, _, .apply hf has => by
      have h := atom_subst τ hf
      simp only [Ty.subst, Ty.substs_eq] at h
      exact .apply h (atoms_subst τ has)
    | _, _, _, .op (t := t) h hs => by
      have h' := atom_subst τ h
      cases t <;> simp only [Ty.Scalar] at hs <;> exact .op h' (by simp [Ty.subst, Ty.Scalar])
    | _, _, _, .read h => .read (atom_subst τ h)
    | _, _, _, .write h₁ h₂ => .write (atom_subst τ h₁) (atom_subst τ h₂)
    | _, _, _, .req has hts hr => by
      have h := atoms_subst τ has
      rw [httpParams_subst τ hts] at h
      exact .req h hts hr
    | _, _, _, .construct hr has => by
      have h := atoms_subst τ has
      rw [map_subst_closed τ (hwf.records _ (record_mem hr))] at h
      exact .construct hr h
    | _, _, _, .field (rec := rec) hv hr hi => by
      have hc := hwf.records _ (record_mem hr) _ (mem_of_getElem? hi)
      rw [hc τ]
      exact .field (atom_subst τ hv) hr hi
    | _, _, _, .list has => by
      have h := atoms_subst τ has
      simp only [List.map_map] at h
      exact .list h
    | _, _, _, .none => .none
    | _, _, _, .some h => .some (atom_subst τ h)
    | _, _, _, .lambda (ts := ts) hlen hb => by
      have h := ExprTy.subst hwf τ hb
      rw [bindTys_subst] at h
      simp only [Ty.subst, Ty.substs_eq]
      exact .lambda (by simp [hlen]) h
    | _, _, _, .funRef (fn := fn) (σ := σ) hf => by
      simp only [Ty.subst, Ty.substs_eq, paramTys_subst, Ty.subst_subst]
      exact .funRef hf
    | _, _, _, .declassify hpo ha => by
      obtain ⟨hs, hd, _⟩ := hwf.policies _ (policy_mem hpo)
      have h := atom_subst τ ha
      rw [hs τ] at h
      rw [hd τ]
      exact .declassify hpo h
    | _, _, _, .map hxs hf => by
      have h1 := atom_subst τ hxs
      have h2 := atom_subst τ hf
      simp only [Ty.subst, Ty.substs] at h1 h2 ⊢
      exact .map h1 h2
    | _, _, _, .filter hxs hk => by
      have h1 := atom_subst τ hxs
      have h2 := atom_subst τ hk
      simp only [Ty.subst, Ty.substs] at h1 h2 ⊢
      exact .filter h1 h2
    | _, _, _, .firstWhere hxs hk => by
      have h1 := atom_subst τ hxs
      have h2 := atom_subst τ hk
      simp only [Ty.subst, Ty.substs] at h1 h2 ⊢
      exact .firstWhere h1 h2
    | _, _, _, .fold hxs hi hf => by
      have h1 := atom_subst τ hxs
      have h2 := atom_subst τ hi
      have h3 := atom_subst τ hf
      simp only [Ty.subst, Ty.substs] at h1 h3 ⊢
      exact .fold h1 h2 h3
    | _, _, _, .parse h => .parse (atom_subst τ h)
    | _, _, _, .eq ha hb => .eq (atom_subst τ ha) (atom_subst τ hb)
    | _, _, _, .toText ha => .toText (atom_subst τ ha)
    | _, _, _, .envGet ha => by
      have h := atom_subst τ ha
      simp only [Ty.subst] at h ⊢
      exact .envGet h
    | _, _, _, .tuple has => by
      have h := atoms_subst τ has
      simp only [Ty.subst, Ty.substs_eq]
      exact .tuple h
    | _, _, _, .append hxs hi => by
      have h1 := atom_subst τ hxs
      simp only [Ty.subst] at h1 ⊢
      exact .append h1 (atom_subst τ hi)
    | _, _, _, .set hxs hi hv => by
      have h1 := atom_subst τ hxs
      simp only [Ty.subst, Ty.substs] at h1 ⊢
      exact .set h1 (atom_subst τ hi) (atom_subst τ hv)

  /-- A typing of a body is closed under substitution. -/
  theorem ExprTy.subst (hwf : p.WellFormed) (τ : List (String × Ty)) :
      ∀ {ctx : Ctx} {e : Expr} {t : Ty}, ExprTy p ctx e t → ExprTy p (ctx.subst τ) e (t.subst τ)
    | _, _, _, .atom h => .atom (atom_subst τ h)
    | _, _, _, .letR hr hb => .letR (RhsTy.subst hwf τ hr) (ExprTy.subst hwf τ hb)
    | _, _, _, .letE hv hb => .letE (ExprTy.subst hwf τ hv) (ExprTy.subst hwf τ hb)
    | _, _, _, .ite hc ht he =>
      .ite (atom_subst τ hc) (ExprTy.subst hwf τ ht) (ExprTy.subst hwf τ he)
    | _, _, _, .caseOption hs hn hsome =>
      .caseOption (atom_subst τ hs) (ExprTy.subst hwf τ hn) (ExprTy.subst hwf τ hsome)
    | _, _, _, .caseResult hs hok herr =>
      .caseResult (atom_subst τ hs) (ExprTy.subst hwf τ hok) (ExprTy.subst hwf τ herr)
    | _, _, _, .fail => .fail
    | _, _, _, .attempt hb => .attempt (ExprTy.subst hwf τ hb)
    | _, _, _, .handle hb hh => .handle (ExprTy.subst hwf τ hb) (ExprTy.subst hwf τ hh)
    | _, _, _, .letTuple hv hlen hb => by
      have h1 := atom_subst τ hv
      have h2 := ExprTy.subst hwf τ hb
      rw [bindTys_subst] at h2
      simp only [Ty.subst, Ty.substs_eq] at h1
      exact .letTuple h1 (by simp [hlen]) h2
    | _, _, _, .loop (x := x) (t := t) (ts := ts) hs hi hlen hb => by
      have h2 : ExprTy p ((x, t.subst τ) :: Ctx.subst τ (bindTys _ ts ++ _)) _
          ((Ty.tuple ts).subst τ) := ExprTy.subst hwf τ hb
      rw [bindTys_subst] at h2
      simp only [Ty.subst, Ty.substs_eq] at h2 ⊢
      exact .loop (source_subst τ hs) (atoms_subst τ hi) (by simp [hlen]) h2
end

/-! ### Values -/

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

/-- Values typed by a list of one type each have that type. -/
theorem valsTy_const {t : Ty} :
    ∀ {vs : List Val} {xs : List Atom}, ValsTy p vs (xs.map (fun _ => t)) → ∀ v ∈ vs, ValTy p v t
  | [], _, _ => by simp
  | _ :: _, [], h => by cases h
  | _ :: _, _ :: _, .cons hv hvs => by
    intro u hu
    rcases List.mem_cons.mp hu with rfl | hu
    · exact hv
    · exact valsTy_const hvs u hu

theorem sumInts_typed : ∀ {items : List Val}, (∀ v ∈ items, ValTy p v .int) →
    ∃ n, sumInts items = some n
  | [], _ => ⟨0, rfl⟩
  | v :: rest, h => by
    have hv := h v (List.mem_cons_self ..)
    cases hv
    rename_i k
    obtain ⟨m, hm⟩ := sumInts_typed (fun u hu => h u (List.mem_cons_of_mem _ hu))
    exact ⟨k + m, by simp [sumInts, hm]⟩

theorem concatTexts_typed : ∀ {vs : List Val} {ts : List Ty}, ValsTy p vs ts →
    (∀ t ∈ ts, t = .text) → ∃ s, concatTexts vs = some s
  | [], [], .nil, _ => ⟨"", rfl⟩
  | _ :: _, t :: _, .cons hv hvs, h => by
    have ht := h t List.mem_cons_self
    subst ht
    cases hv
    rename_i s
    obtain ⟨r, hr⟩ := concatTexts_typed hvs (fun u hu => h u (List.mem_cons_of_mem _ hu))
    exact ⟨s ++ r, by simp [concatTexts, hr]⟩

theorem prim_typed {pr : Prim} {ts : List Ty} {t : Ty} {vs : List Val}
    (hp : PrimTy pr ts t) (hvs : ValsTy p vs ts) :
    ∃ v, evalPrim pr vs = some v ∧ ValTy p v t := by
  cases hp
  case sum =>
    rcases hvs with _ | ⟨hv, hvs⟩
    cases hvs
    cases hv
    rename_i hall
    obtain ⟨n, hn⟩ := sumInts_typed hall
    exact ⟨.int n, by simp [evalPrim, hn], .int⟩
  case concat hts =>
    obtain ⟨s, hs⟩ := concatTexts_typed hvs hts
    exact ⟨.text s, by simp [evalPrim, hs], .text⟩
  all_goals
  · repeat (rcases hvs with _ | ⟨hv, hvs⟩; try cases hv)
    all_goals exact ⟨_, rfl, by constructor⟩

theorem field_typed {t : Ty} :
    ∀ {fields : List Val} {ts : List Ty} {i : Nat}, ValsTy p fields ts → ts[i]? = some t →
      ∃ v, fields[i]? = some v ∧ ValTy p v t
  | _, _, _, .nil, hi => by simp at hi
  | _, _, 0, .cons hv _, hi => by simp at hi ⊢; subst hi; exact hv
  | _, _, i + 1, .cons _ hfs, hi => by simp at hi ⊢; exact field_typed hfs hi

theorem bindParams_typed {α : Type} :
    ∀ {params : List (String × α)} {vs : List Val} {ts : List Ty} {env : Env} {ctx : Ctx},
      ValsTy p vs ts → ts.length = params.length → EnvTy p env ctx →
      EnvTy p (bindParams params vs env) (bindTys params ts ++ ctx)
  | [], _, _, _, _, .nil, _, henv => by simpa [bindParams, bindTys] using henv
  | [], _, _ :: _, _, _, _, hlen, _ => by simp at hlen
  | _ :: _, _, [], _, _, .nil, hlen, _ => by simp at hlen
  | _ :: _, _, _ :: _, _, _, .cons hv hrest, hlen, henv => by
    simp only [List.length_cons, Nat.add_right_cancel_iff] at hlen
    have := bindParams_typed hrest hlen henv
    simp only [bindParams, bindTys, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
    exact .cons hv this

theorem find_mem {f : String} {fn : Fun} (h : p.find f = some fn) : fn ∈ p.funs :=
  List.mem_of_find?_eq_some h

theorem paramTys_length (fn : Fun) (σ : List (String × Ty)) :
    (fn.paramTys σ).length = fn.params.length := by
  simp [Fun.paramTys]

/-- The body of a function, at any instance of its type parameters. -/
theorem body_typed {f : String} {fn : Fun} (hwf : p.WellFormed) (h : p.find f = some fn)
    (σ : List (String × Ty)) :
    ExprTy p (bindTys fn.params (fn.paramTys σ)) fn.body (fn.result.erase.subst σ) := by
  have h0 := ExprTy.subst hwf σ (hwf.funs fn (find_mem h))
  have h1 := bindTys_subst σ fn.params (fn.paramTys []) []
  simp only [List.append_nil] at h1
  rw [h1, paramTys_subst] at h0
  simpa [Ctx.subst] using h0

end Una.F1
