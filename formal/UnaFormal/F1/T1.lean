/-
T1, type safety, for F1a (`spec/10 §10`, M1 of `docs/plan/formal-track.md`): a well-typed
expression of a well-formed program, in an environment of the types its context gives, never
gets stuck. It ends with a value of its type, with a failure, or runs out of fuel, which is
the big-step form of "it is a value or a `fail`, or it steps" for terminating runs.
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

theorem andThen_good {first : List String × Res} {rest : Val → List String × Res} {t s : Ty}
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

theorem sound (hwf : p.WellFormed) : ∀ n : Nat,
    (∀ {env : Env} {ctx : Ctx} {r : Rhs} {t : Ty},
      EnvTy p env ctx → RhsTy p ctx r t → Good p t (evalRhs n p env r).2) ∧
    (∀ {env : Env} {ctx : Ctx} {e : Expr} {t : Ty},
      EnvTy p env ctx → ExprTy p ctx e t → Good p t (eval n p env e).2)
  | 0 => ⟨fun _ _ => by simp [evalRhs, Good], fun _ _ => by simp [eval, Good]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := sound hwf n
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
      | call hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        have hlen : vs.length = _ := valsTy_length hvst
        simp only [List.length_map] at hlen
        have hbody := find_wellTyped hwf hf
        have henv' := bindParams_typed hvst (EnvTy.nil (p := p))
        simp only [List.append_nil] at henv'
        simp only [evalRhs, hf, hvs, hlen, if_true]
        exact ihE henv' hbody
      | apply hf has =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hf
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        cases hvt with
        | closure hcenv hbody =>
          have hlen : vs.length = _ := valsTy_length hvst
          simp only [List.length_map] at hlen
          simp only [evalRhs, hv, hvs, hlen, if_true]
          exact ihE (bindParams_typed hvst hcenv) hbody
      | op hl =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hl
        cases hvt
        simp [evalRhs, hv, Good, ValTy.unit]
      | construct hfs has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .record hfs hvst
      | field hv hfs hi =>
        obtain ⟨v, hv', hvt⟩ := atom_typed henv hv
        cases hvt with
        | record hfs' hvals =>
          rw [hfs] at hfs'
          cases hfs'
          obtain ⟨w, hw, hwt⟩ := field_typed hvals hi
          simp [evalRhs, hv', hw, Good, hwt]
      | list has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        have hlen : vs.length = _ := valsTy_length hvst
        simp only [List.length_map] at hlen
        rw [map_const_of_length (t := _) hlen.symm] at hvst
        simp [evalRhs, hvs, Good]
        exact .list hvst
      | none => simp [evalRhs, Good]; exact .none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good]
        exact .some hvt
      | lambda hb => simp [evalRhs, Good]; exact .closure henv hb
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
      | fail => simp [eval, Good]
      | attempt hb =>
        have h := ihE henv hb
        simp only [eval]
        revert h
        cases eval n p env _ with
        | mk tr r =>
          cases r <;> simp [Good]
          · exact .ok
          · exact .err
      | handle hb hh =>
        have h := ihE henv hb
        simp only [eval]
        revert h
        cases eval n p env _ with
        | mk tr r =>
          cases r with
          | failed code =>
            intro _
            have hgood := ihE (.cons (ValTy.int (n := (code : Int))) henv) hh
            simpa using hgood
          | ok v => simp
          | stuck => simp [Good]
          | outOfFuel => simp [Good]

/-- **T1 for F1a.** A well-typed expression of a well-formed program, run in an environment of
its context's types with any fuel, never gets stuck, and a value it ends with has its type. -/
theorem t1 (hwf : p.WellFormed) {env : Env} {ctx : Ctx} {e : Expr} {t : Ty}
    (henv : EnvTy p env ctx) (he : ExprTy p ctx e t) (n : Nat) :
    (eval n p env e).2 ≠ .stuck ∧ ∀ v, (eval n p env e).2 = .ok v → ValTy p v t := by
  have h := (sound hwf n).2 henv he
  constructor
  · intro hs
    rw [hs] at h
    exact h
  · intro v hv
    rw [hv] at h
    exact h

end Una.F1
