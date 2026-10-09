/-
T1, type safety, for F1 (`spec/10 §10`, M1 of `docs/plan/formal-track.md`): a well-typed
expression of a well-formed program, in an environment of the types its context gives, never
gets stuck, whatever the host does. It ends with a value of its type, with a failure, or runs
out of fuel, which is the big-step form of "it is a value or a `fail`, or it steps" for
terminating runs.
-/
import UnaFormal.F1.Lemmas

namespace Una.F1

variable {p : Program}

/-- What T1 allows a run of type `t` to end with: a value of type `t`, a failure, or the end
of fuel; never a stuck evaluation. -/
def Good (p : Program) (t : Ty) : Res → Prop
  | .ok v => ValTy p v t
  | .failed _ => True
  | .stuck => False
  | .outOfFuel => True

theorem andThen_good {first : Trace × Res} {rest : Val → Trace × Res} {t s : Ty}
    (h1 : Good p t first.2) (h2 : ∀ v, ValTy p v t → Good p s (rest v).2) :
    Good p s (andThen first rest).2 := by
  obtain ⟨tr, r⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    have := h2 v h1
    revert this
    cases rest v
    simp
  | failed code => simp [andThen, Good]
  | stuck => simp [Good] at h1
  | outOfFuel => simp [andThen, Good]

/-- A call of a typed function value on typed arguments, with a sound evaluation of bodies. -/
theorem applyWith_good {run : Env → Expr → Trace × Res} {fv : Val} {vs : List Val}
    {ts : List Ty} {r : Ty}
    (hrun : ∀ {env : Env} {ctx : Ctx} {e : Expr} {t : Ty},
      EnvTy p env ctx → ExprTy p ctx e t → Good p t (run env e).2)
    (hf : ValTy p fv (.fn ts r)) (hvs : ValsTy p vs ts) : Good p r (applyWith run fv vs).2 := by
  cases hf with
  | closure hcenv hlen hbody =>
    rw [applyWith, ite_eq_left ((valsTy_length hvs).trans hlen)]
    exact hrun (bindParams_typed hvs hlen hcenv) hbody

theorem walkMap_good {app : Val → Trace × Res} {a b : Ty}
    (happ : ∀ v, ValTy p v a → Good p b (app v).2) :
    ∀ {items : List Val}, (∀ v ∈ items, ValTy p v a) → Good p (.list b) (walkMap app items).2
  | [], _ => by simp only [walkMap, done, Good]; exact .list (by simp)
  | v :: vs, h => by
    simp only [walkMap]
    refine andThen_good (happ v (h v List.mem_cons_self)) (fun r hr => ?_)
    refine andThen_good (walkMap_good happ (fun u hu => h u (List.mem_cons_of_mem _ hu)))
      (fun rest hrest => ?_)
    cases hrest with
    | list hrs =>
      simp only [done, Good]
      exact .list (fun u hu => by
        rcases List.mem_cons.mp hu with rfl | hu
        · exact hr
        · exact hrs u hu)

theorem walkFilter_good {app : Val → Trace × Res} {a : Ty}
    (happ : ∀ v, ValTy p v a → Good p .bool (app v).2) :
    ∀ {items : List Val}, (∀ v ∈ items, ValTy p v a) → Good p (.list a) (walkFilter app items).2
  | [], _ => by simp only [walkFilter, done, Good]; exact .list (by simp)
  | v :: vs, h => by
    simp only [walkFilter]
    refine andThen_good (happ v (h v List.mem_cons_self)) (fun b hb => ?_)
    cases hb with
    | @bool keep =>
      refine andThen_good (walkFilter_good happ (fun u hu => h u (List.mem_cons_of_mem _ hu)))
        (fun rest hrest => ?_)
      cases hrest with
      | list hrs =>
        simp only [done, Good]
        cases keep
        · exact .list hrs
        · exact .list (fun u hu => by
            rcases List.mem_cons.mp (by simpa using hu) with rfl | hu
            · exact h u List.mem_cons_self
            · exact hrs u hu)

theorem walkFirst_good {app : Val → Trace × Res} {a : Ty}
    (happ : ∀ v, ValTy p v a → Good p .bool (app v).2) :
    ∀ {items : List Val}, (∀ v ∈ items, ValTy p v a) → Good p (.option a) (walkFirst app items).2
  | [], _ => by simp only [walkFirst, done, Good]; exact .none
  | v :: vs, h => by
    simp only [walkFirst]
    refine andThen_good (happ v (h v List.mem_cons_self)) (fun b hb => ?_)
    cases hb with
    | @bool keep =>
      cases keep
      · exact walkFirst_good happ (fun u hu => h u (List.mem_cons_of_mem _ hu))
      · simp only [done, Good]
        exact .some (h v List.mem_cons_self)

theorem walkFold_good {app : Val → Val → Trace × Res} {a b : Ty}
    (happ : ∀ acc v, ValTy p acc b → ValTy p v a → Good p b (app acc v).2) :
    ∀ {items : List Val} {acc : Val}, ValTy p acc b → (∀ v ∈ items, ValTy p v a) →
      Good p b (walkFold app acc items).2
  | [], _, hacc, _ => by simpa [walkFold, done, Good] using hacc
  | v :: vs, _, hacc, h => by
    simp only [walkFold]
    exact andThen_good (happ _ v hacc (h v List.mem_cons_self))
      (fun acc' hacc' => walkFold_good happ hacc' (fun u hu => h u (List.mem_cons_of_mem _ hu)))

theorem parseInt_good (s : String) : Good p .int (parseInt s).2 := by
  unfold parseInt
  split
  · exact .int
  · trivial

theorem sound (hwf : p.WellFormed) (w : World) : ∀ n : Nat,
    (∀ {env : Env} {ctx : Ctx} {r : Rhs} {t : Ty},
      EnvTy p env ctx → RhsTy p ctx r t → Good p t (evalRhs n p w env r).2) ∧
    (∀ {env : Env} {ctx : Ctx} {e : Expr} {t : Ty},
      EnvTy p env ctx → ExprTy p ctx e t → Good p t (eval n p w env e).2)
  | 0 => ⟨fun _ _ => by simp [evalRhs, Good], fun _ _ => by simp [eval, Good]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := sound hwf w n
    constructor
    · intro env ctx r t henv hr
      cases hr with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good, hvt]
      | prim has hp =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        obtain ⟨v, hv, hvt⟩ := prim_typed hp hvst
        simp [evalRhs, hvs, hv, Good, hvt]
      | @call _ f _ _ fn σ hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        have hlen := paramTys_length fn σ
        simp only [evalRhs, hf, hvs]
        rw [ite_eq_left ((valsTy_length hvst).trans hlen)]
        have henv' := bindParams_typed hvst hlen (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        exact ihE henv' (body_typed hwf hf σ)
      | apply hf has =>
        obtain ⟨fv, hv, hvt⟩ := atom_typed henv hf
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp only [evalRhs, hv, hvs]
        exact applyWith_good ihE hvt hvst
      | op hl =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hl
        cases hvt
        simp [evalRhs, hv, Good, ValTy.unit]
      | get hu =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hu
        cases hvt with
        | @text s =>
          simp only [evalRhs, hv]
          cases w.get s
          · trivial
          · exact .text
      | construct hrec has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .record hrec hvst
      | field hv hrec hi =>
        obtain ⟨v, hv', hvt⟩ := atom_typed henv hv
        cases hvt with
        | record hrec' hvals =>
          rw [hrec] at hrec'
          cases hrec'
          obtain ⟨u, hu, hut⟩ := field_typed hvals hi
          simp [evalRhs, hv', hu, Good, hut]
      | list has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .list (valsTy_const hvst)
      | none => simp only [evalRhs, done, Good]; exact .none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good]
        exact .some hvt
      | lambda hlen hb => simp only [evalRhs, done, Good]; exact .closure henv hlen hb
      | @funRef _ f _ fn σ hf =>
        simp only [evalRhs, hf, done, Good]
        exact .closure .nil (paramTys_length fn σ) (by simpa using body_typed hwf hf σ)
      | declassify hpo ha =>
        obtain ⟨_, _, fn, hfn, hps, hres⟩ := hwf.policies _ (policy_mem hpo)
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        have hlen := paramTys_length fn []
        rw [hps] at hlen
        simp only [evalRhs, hpo, hv, hfn]
        rw [ite_eq_left (by simpa using hlen)]
        have hvs : ValsTy p [v] (fn.paramTys []) := by rw [hps]; exact .cons hvt .nil
        have henv' := bindParams_typed hvs (paramTys_length fn []) (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        have hbody := body_typed hwf hfn []
        rw [Ty.subst_nil, hres] at hbody
        exact ihE henv' hbody
      | map hxs hf =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hf
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          exact walkMap_good (fun v hv => applyWith_good ihE hft (.cons hv .nil)) hitems
      | filter hxs hk =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hk
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          exact walkFilter_good (fun v hv => applyWith_good ihE hft (.cons hv .nil)) hitems
      | firstWhere hxs hk =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hk
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          exact walkFirst_good (fun v hv => applyWith_good ihE hft (.cons hv .nil)) hitems
      | fold hxs hi hf =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨iv, hiv, hit⟩ := atom_typed henv hi
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hf
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, hfv]
          exact walkFold_good
            (fun acc v hacc hv => applyWith_good ihE hft (.cons hacc (.cons hv .nil))) hit hitems
      | parse hs =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hs
        cases hvt with
        | @text s =>
          simp only [evalRhs, hv]
          exact parseInt_good s
      | eq ha hb =>
        obtain ⟨va, hva, _⟩ := atom_typed henv ha
        obtain ⟨vb, hvb, _⟩ := atom_typed henv hb
        simp only [evalRhs, hva, hvb, done, Good]
        exact .bool
    · intro env ctx e t henv he
      cases he with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [eval, hv, Good, hvt]
      | letR hr hb =>
        simp only [eval]
        exact andThen_good (ihR henv hr) (fun v hv => ihE (.cons hv henv) hb)
      | letE hv hb =>
        simp only [eval]
        exact andThen_good (ihE henv hv) (fun v hv => ihE (.cons hv henv) hb)
      | ite hc ht he =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hc
        cases hvt with
        | @bool b =>
          cases b
          · simp only [eval, hv]; exact ihE henv he
          · simp only [eval, hv]; exact ihE henv ht
      | caseOption hs hn hsome =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hs
        cases hvt with
        | none => simp only [eval, hv]; exact ihE henv hn
        | some hw => simp only [eval, hv]; exact ihE (.cons hw henv) hsome
      | caseResult hs hok herr =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hs
        cases hvt with
        | ok hw => simp only [eval, hv]; exact ihE (.cons hw henv) hok
        | err => simp only [eval, hv]; exact ihE (.cons .int henv) herr
      | fail => simp [eval, Good]
      | attempt hb =>
        have h := ihE henv hb
        simp only [eval]
        revert h
        cases eval n p w env _ with
        | mk tr r =>
          cases r <;> simp [Good]
          · exact .ok
          · exact .err
      | handle hb hh =>
        have h := ihE henv hb
        simp only [eval]
        revert h
        cases eval n p w env _ with
        | mk tr r =>
          cases r with
          | failed code =>
            intro _
            have hgood := ihE (.cons (ValTy.int (n := (code : Int))) henv) hh
            simpa using hgood
          | ok v => simp
          | stuck => simp [Good]
          | outOfFuel => simp [Good]

/-- **T1 for F1.** A well-typed expression of a well-formed program, run in an environment of
its context's types with any fuel and any host, never gets stuck, and a value it ends with has
its type. -/
theorem t1 (hwf : p.WellFormed) (w : World) {env : Env} {ctx : Ctx} {e : Expr} {t : Ty}
    (henv : EnvTy p env ctx) (he : ExprTy p ctx e t) (n : Nat) :
    (eval n p w env e).2 ≠ .stuck ∧ ∀ v, (eval n p w env e).2 = .ok v → ValTy p v t := by
  have h := (sound hwf w n).2 henv he
  constructor
  · intro hs
    rw [hs] at h
    exact h
  · intro v hv
    rw [hv] at h
    exact h

end Una.F1
