/-
The restrictions of option B1 (declassification, `docs/plan/formal-track-m4.md` §3) on a
program's policies, and what a pure `via` function's run is: no event and no failure.
-/
import UnaFormal.F1.LTyping

namespace Una.F1

mutual
  /-- A right-hand side that performs no event and cannot fail or call: the `via` functions of
  option B1. -/
  def Rhs.Pure : Rhs → Prop
    | .atom _ | .prim _ _ | .construct _ _ | .field _ _ | .list _ | .none | .some _ | .first _ | .elemAt _ _
    | .orElse _ _ | .isOk _ | .mkOk _
    | .mkErr _ | .eq _ _ | .variant _ _ _
    | .lambda _ _ | .funRef _ _ => True
    | _ => False

  /-- A body that performs no event, cannot fail, and calls nothing. -/
  def Expr.Pure : Expr → Prop
    | .atom _ => True
    | .letR _ _ v b => Rhs.Pure v ∧ Expr.Pure b
    | .letE _ _ v b => Expr.Pure v ∧ Expr.Pure b
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e
    | .caseSum _ _ _ t e | .handleK _ t _ e =>
      Expr.Pure t ∧ Expr.Pure e
    | .attempt b | .attemptK _ b => Expr.Pure b
    | .fail _ | .raise _ _ _ | .unreachable | .letTuple _ _ _ | .loop _ _ _ _ _ _ | .withH _ _ _ => False
end

/-- A right-hand side that performs no event and calls nothing, but may fail: a pure one, or
`checked` (#453 Phase B). -/
def Rhs.PureOrFails : Rhs → Prop
  | .checked _ _ _ => True
  | r => r.Pure

/-- A body that performs no event and calls nothing, but may fail: `Expr.Pure` with `fail` and
`checked` (#453 Phase B). -/
def Expr.PureOrFails : Expr → Prop
  | .atom _ => True
  | .letR _ _ v b => Rhs.PureOrFails v ∧ Expr.PureOrFails b
  | .letE _ _ v b => Expr.PureOrFails v ∧ Expr.PureOrFails b
  | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e
  | .caseSum _ _ _ t e | .handleK _ t _ e =>
    Expr.PureOrFails t ∧ Expr.PureOrFails e
  | .attempt b | .attemptK _ b => Expr.PureOrFails b
  | .fail _ => True
  -- A `via` function's failure is released as `Err(c)`, a code (ADR 0090): one with a payload
  -- is not a `via` function's (#512).
  | .raise _ _ _ | .unreachable | .letTuple _ _ _ | .loop _ _ _ _ _ _ | .withH _ _ _ => False

/-- **The restrictions of option B1**, on the policies. Each `from` is a scalar type, each `to`
is a scalar type with a concrete label, and each `via` function performs no operation and
calls nothing (`Expr.PureOrFails`). It may fail (#453 Phase B): its failure is released as its
value is (T-Declassify, ADR 0090). One that does not declare `fails` is `Pure`, so its run
cannot fail. `spec/07 §5` requires none of them (rule 1 gives a policy the `uses` and
`fails` of its `via` function); they are the model's. Of the corpus's 120 policies, none has a
`from` that is not a scalar, 3 have a `to` that is not a scalar (2.5%), and 4 have a `via`
function that is not `Pure` as written here (3.3%). The scalar `from` is a candidate to lift: it
only spares the proof a lemma from the labeled typing of values to their structural typing
(`LValTy` to `ValTy`), which the closures make long. -/
structure Program.B1WellFormed (p : Program) : Prop where
  src : ∀ po ∈ p.policies, po.src.erase.Scalar
  dst : ∀ po ∈ p.policies, po.dst.Ground ∧ po.dst.erase.Scalar
  via : ∀ po ∈ p.policies, ∃ fn, p.find po.via = some fn ∧ fn.op = false ∧
    -- Either the `via` performs no host operation and may only fail (#453 Phase B), or it is of
    -- ADR 0098's shape: `Keys` only, its failure decided by literal key names (#587).
    ((fn.host.isNone = true ∧ fn.body.PureOrFails ∧ (fn.fails = false → fn.body.Pure)) ∨
      fn.hmacVia = true) ∧
    -- A `via` function that fails with a payload kind (#512) is the model's restriction, not
    -- Una's (`spec/07 §5` rule 1 gives the policy the `fails` of its `via` function, of any
    -- kind): F1 releases a `via` function's failure as `Err(c)`, a code (ADR 0090).
    fn.raises = []

/-- Without policies, the restrictions hold. -/
theorem Program.B1WellFormed.of_nil {p : Program} (hnil : p.policies = []) : p.B1WellFormed where
  src := by rw [hnil]; intro _ h; nomatch h
  dst := by rw [hnil]; intro _ h; nomatch h
  via := by rw [hnil]; intro _ h; nomatch h

variable {H : Host}

/-- A run from `w` with no event, that does not fail, and leaves the world as it was. -/
def Quiet (w : World H) (r : Run H) : Prop := r.1 = [] ∧ (∀ c, r.2.1 ≠ .failed c) ∧ r.2.2 = w

theorem quiet_andThen {w : World H} {first : Run H} {rest : World H → Val → Run H}
    (h₁ : Quiet w first) (h₂ : ∀ v, Quiet w (rest w v)) : Quiet w (andThen first rest) := by
  obtain ⟨t, r, w'⟩ := first
  obtain ⟨ht, hr, hw⟩ := h₁
  simp only at ht hr hw
  subst ht hw
  cases r with
  | ok v =>
    simp only [andThen]
    have := h₂ v
    rcases hq : rest w' v with ⟨t', r', w''⟩
    rw [hq] at this
    exact ⟨by simpa using this.1, this.2.1, this.2.2⟩
  | failed c => exact absurd rfl (hr c)
  | stuck => exact ⟨rfl, fun _ h => (nomatch h), rfl⟩
  | raised _ _ _ => exact ⟨rfl, fun _ h => (nomatch h), rfl⟩
  | outOfFuel | aborted => exact ⟨rfl, fun _ h => (nomatch h), rfl⟩

theorem quiet_elim {w : World H} {o : Option Val} : Quiet w ([], o.elim .stuck .ok, w) := by
  cases o <;> exact ⟨rfl, fun _ h => (nomatch h), rfl⟩

/-- **A pure body's run** performs no event, does not fail, and leaves the world as it was, at
any fuel. -/
theorem pure_run (p : Program) (w : World H) : ∀ n : Nat,
    (∀ (env : Env) (r : Rhs), r.Pure → Quiet w (evalRhs H n p w env r)) ∧
    (∀ (env : Env) (e : Expr), e.Pure → Quiet w (eval H n p w env e))
  | 0 => ⟨fun _ _ _ => ⟨rfl, fun _ h => (nomatch h), rfl⟩, fun _ _ _ => ⟨rfl, fun _ h => (nomatch h), rfl⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := pure_run p w n
    have hstuck : Quiet w (stuck w) := ⟨rfl, fun _ h => (nomatch h), rfl⟩
    have hdone : ∀ v, Quiet w (done w v) := fun _ => ⟨rfl, fun _ h => (nomatch h), rfl⟩
    have habort : Quiet w ([], Res.aborted, w) := ⟨rfl, fun _ h => (nomatch h), rfl⟩
    constructor
    · intro env r hr
      cases r <;> simp only [Rhs.Pure] at hr <;> simp only [evalRhs]
      all_goals first
        | exact quiet_elim
        | exact hdone _
        | exact habort
        | (split <;> first | exact quiet_elim | exact hstuck | exact hdone _ | exact habort)
    · intro env e he
      cases e with
      | atom a => exact quiet_elim
      | letR x t v b =>
        exact quiet_andThen (ihR env v he.1) (fun _ => ihE _ b he.2)
      | letE x t v b =>
        exact quiet_andThen (ihE env v he.1) (fun _ => ihE _ b he.2)
      | ite c t e =>
        simp only [eval]
        split
        · exact ihE env t he.1
        · exact ihE env e he.2
        · exact hstuck
      | caseOption s nE x sE =>
        simp only [eval]
        split
        · exact ihE env nE he.1
        · exact ihE _ sE he.2
        · exact hstuck
      | caseResult s x oE y eE =>
        simp only [eval]
        split
        · exact ihE _ oE he.1
        · exact ihE _ eE he.2
        · exact hstuck
      | fail c => exact absurd he (by simp [Expr.Pure])
      | raise _ _ _ => exact absurd he (by simp [Expr.Pure])
      | unreachable => exact absurd he (by simp [Expr.Pure])
      | withH _ _ _ => exact absurd he (by simp [Expr.Pure])
      | caseSum s i xs t e =>
        simp only [eval]
        split
        · split
          · split
            · exact ihE _ t he.1
            · exact hstuck
          · exact ihE env e he.2
        · exact hstuck
      | attempt b =>
        have h := ihE env b he
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r with
        | ok v => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | failed c => exact absurd rfl (h.2.1 c)
        | stuck => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | raised _ _ _ => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | outOfFuel | aborted => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
      | attemptK k b =>
        have h := ihE env b he
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r with
        | ok v => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | failed c => exact absurd rfl (h.2.1 c)
        | stuck => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | raised _ _ _ => dsimp only; split <;> exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
        | outOfFuel | aborted => exact ⟨h.1, fun _ h => (nomatch h), h.2.2⟩
      | handle b x hd =>
        have h := ihE env b he.1
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r with
        | failed c => exact absurd rfl (h.2.1 c)
        | ok v => exact h
        | stuck => exact h
        | raised _ _ _ => exact h
        | outOfFuel | aborted => exact h
      | handleK k b x hd =>
        have h := ihE env b he.1
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r with
        | raised k' c ps =>
          obtain ⟨ht, _, hw⟩ := h
          simp only at ht hw
          subst ht hw
          dsimp only
          split
          · have := ihE ((x, .variant k' c ps) :: env) hd he.2
            simpa using this
          · exact ⟨rfl, fun _ h => (nomatch h), rfl⟩
        | failed c => exact absurd rfl (h.2.1 c)
        | ok v => exact h
        | stuck => exact h
        | outOfFuel | aborted => exact h
      | letTuple _ _ _ => exact (show False from he).elim
      | loop _ _ _ _ _ _ => exact (show False from he).elim

/-- A run from `w` with no event, that leaves the world as it was: it may fail. -/
def Silent (w : World H) (r : Run H) : Prop := r.1 = [] ∧ r.2.2 = w

theorem silent_andThen {w : World H} {first : Run H} {rest : World H → Val → Run H}
    (h₁ : Silent w first) (h₂ : ∀ v, Silent w (rest w v)) : Silent w (andThen first rest) := by
  obtain ⟨t, r, w'⟩ := first
  obtain ⟨ht, hw⟩ := h₁
  simp only at ht hw
  subst ht hw
  cases r with
  | ok v =>
    simp only [andThen]
    have := h₂ v
    rcases hq : rest w' v with ⟨t', r', w''⟩
    rw [hq] at this
    exact ⟨by simpa using this.1, this.2⟩
  | failed c => exact ⟨rfl, rfl⟩
  | stuck => exact ⟨rfl, rfl⟩
  | raised _ _ _ => exact ⟨rfl, rfl⟩
  | outOfFuel | aborted => exact ⟨rfl, rfl⟩

theorem Quiet.silent {w : World H} {r : Run H} (h : Quiet w r) : Silent w r := ⟨h.1, h.2.2⟩

/-- **A body that may only fail** performs no event and leaves the world as it was, at any
fuel (#453 Phase B). -/
theorem silent_run (p : Program) (w : World H) : ∀ n : Nat,
    (∀ (env : Env) (r : Rhs), r.PureOrFails → Silent w (evalRhs H n p w env r)) ∧
    (∀ (env : Env) (e : Expr), e.PureOrFails → Silent w (eval H n p w env e))
  | 0 => ⟨fun _ _ _ => ⟨rfl, rfl⟩, fun _ _ _ => ⟨rfl, rfl⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := silent_run p w n
    constructor
    · intro env r hr
      cases r with
      | checked a pred code =>
        simp only [evalRhs]
        split
        · simp only [checkRun]
          split
          · split <;> exact ⟨rfl, rfl⟩
          · exact ⟨rfl, rfl⟩
        · exact ⟨rfl, rfl⟩
      | _ =>
        simp only [Rhs.PureOrFails] at hr
        exact ((pure_run p w (n + 1)).1 env _ hr).silent
    · intro env e he
      cases e with
      | atom a => exact ((pure_run p w (n + 1)).2 env (.atom a) trivial).silent
      | letR x t v b =>
        exact silent_andThen (ihR env v he.1) (fun _ => ihE _ b he.2)
      | letE x t v b =>
        exact silent_andThen (ihE env v he.1) (fun _ => ihE _ b he.2)
      | ite c t e =>
        simp only [eval]
        split
        · exact ihE env t he.1
        · exact ihE env e he.2
        · exact ⟨rfl, rfl⟩
      | caseOption s nE x sE =>
        simp only [eval]
        split
        · exact ihE env nE he.1
        · exact ihE _ sE he.2
        · exact ⟨rfl, rfl⟩
      | caseResult s x oE y eE =>
        simp only [eval]
        split
        · exact ihE _ oE he.1
        · exact ihE _ eE he.2
        · exact ⟨rfl, rfl⟩
      | fail c => exact ⟨rfl, rfl⟩
      | raise _ _ _ => exact (show False from he).elim
      | unreachable => exact (show False from he).elim
      | withH _ _ _ => exact (show False from he).elim
      | caseSum s i xs t e =>
        simp only [eval]
        split
        · split
          · split
            · exact ihE _ t he.1
            · exact ⟨rfl, rfl⟩
          · exact ihE env e he.2
        · exact ⟨rfl, rfl⟩
      | attempt b =>
        have h := ihE env b he
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r <;> exact h
      | attemptK k b =>
        have h := ihE env b he
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        cases r <;> dsimp only <;> (try split) <;> exact h
      | handle b x hd =>
        have h := ihE env b he.1
        simp only [eval]
        revert h
        rcases hb : eval H n p w env b with ⟨t, r, w'⟩
        intro h
        obtain ⟨ht, hw⟩ := h
        simp only at ht hw
        subst ht hw
        cases r with
        | failed c =>
          have := ihE ((x, .int c) :: env) hd he.2
          simpa using this
        | ok v => exact ⟨rfl, rfl⟩
        | stuck => exact ⟨rfl, rfl⟩
        | raised _ _ _ => exact ⟨rfl, rfl⟩
        | outOfFuel | aborted => exact ⟨rfl, rfl⟩
      | handleK k b x hd =>
        have h := ihE env b he.1
        simp only [eval]
        revert h
        rcases eval H n p w env b with ⟨t, r, w'⟩
        intro h
        obtain ⟨ht, hw⟩ := h
        simp only at ht hw
        subst ht hw
        cases r with
        | raised k' c ps =>
          dsimp only
          split
          · have := ihE ((x, .variant k' c ps) :: env) hd he.2
            simpa using this
          · exact ⟨rfl, rfl⟩
        | failed c => exact ⟨rfl, rfl⟩
        | ok v => exact ⟨rfl, rfl⟩
        | stuck => exact ⟨rfl, rfl⟩
        | outOfFuel | aborted => exact ⟨rfl, rfl⟩
      | letTuple _ _ _ => exact (show False from he).elim
      | loop _ _ _ _ _ _ => exact (show False from he).elim

end Una.F1

namespace Una.F1

variable {H : Host}

theorem andThen_ne_raised {first : Run H} {rest : World H → Val → Run H}
    (h1 : ∀ k c ps, first.2.1 ≠ .raised k c ps) (h2 : ∀ w v k c ps, (rest w v).2.1 ≠ .raised k c ps) :
    ∀ k c ps, (andThen first rest).2.1 ≠ .raised k c ps := by
  obtain ⟨t, r, w⟩ := first
  intro k c ps
  cases r with
  | ok v =>
    simp only [andThen]
    have := h2 w v k c ps
    revert this
    rcases rest w v with ⟨t', r', w'⟩
    exact id
  | raised k' c' ps' => exact (h1 k' c' ps' rfl).elim
  | _ => simp [andThen]

/-- **A body that may only fail never fails with a payload** (#512): it has no `raise` and calls
nothing, so a `via` function's failure is always a code, which T-Declassify releases as `Err(c)`. -/
theorem noRaise_run (p : Program) (w : World H) : ∀ n : Nat,
    (∀ (env : Env) (r : Rhs), r.PureOrFails → ∀ k c ps, (evalRhs H n p w env r).2.1 ≠ .raised k c ps) ∧
    (∀ (env : Env) (e : Expr), e.PureOrFails → ∀ k c ps, (eval H n p w env e).2.1 ≠ .raised k c ps)
  | 0 => ⟨fun _ _ _ _ _ _ h => by simp [evalRhs] at h, fun _ _ _ _ _ _ h => by simp [eval] at h⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := noRaise_run p w n
    have hR : ∀ (env : Env) (r : Rhs), r.PureOrFails → ∀ k c ps,
        (evalRhs H (n + 1) p w env r).2.1 ≠ .raised k c ps := by
      intro env r hr k c ps
      cases r with
      | atom a => simp only [evalRhs]; cases evalAtom env a <;> simp [Option.elim]
      | prim pr args =>
        simp only [evalRhs]
        split
        · simp
        · cases (evalAtoms env args).bind (evalPrim pr) <;> simp [Option.elim]
      | construct r fs => simp only [evalRhs]; cases evalAtoms env fs <;> simp [Option.elim]
      | variant k' i fs => simp only [evalRhs]; cases evalAtoms env fs <;> simp [Option.elim]
      | field v i =>
        simp only [evalRhs]
        split
        · rename_i fields _
          cases fields[i]? <;> simp [Option.elim]
        · simp [stuck]
      | list xs => simp only [evalRhs]; cases evalAtoms env xs <;> simp [Option.elim]
      | none => simp [evalRhs, done]
      | some a => simp only [evalRhs]; cases evalAtom env a <;> simp [Option.elim]
      | first a => simp only [evalRhs]; split <;> simp [done, stuck]
      | elemAt xs i => simp only [evalRhs]; split <;> simp [done, stuck]
      | orElse o d => simp only [evalRhs]; split <;> simp [done, stuck]
      | isOk r => simp only [evalRhs]; split <;> simp [done, stuck]
      | mkOk a => simp only [evalRhs]; cases evalAtom env a <;> simp [Option.elim]
      | mkErr a => simp only [evalRhs]; cases evalAtom env a <;> simp [Option.elim]
      | eq a b => simp only [evalRhs]; split <;> simp [done, stuck]
      | lambda ps b => simp [evalRhs, done]
      | funRef f ts => simp only [evalRhs]; split <;> simp [done, stuck]
      | checked a pred code =>
        simp only [evalRhs]
        split
        · simp only [checkRun]
          split
          · split <;> simp [done]
          · simp [stuck]
        · simp [stuck]
      | _ => simp [Rhs.PureOrFails, Rhs.Pure] at hr
    refine ⟨hR, fun env e he k c ps => ?_⟩
    cases e with
    | atom a => simp only [eval]; cases evalAtom env a <;> simp [Option.elim]
    | letR x t v b =>
      simp only [eval]
      exact andThen_ne_raised ((noRaise_run p w n).1 env v he.1)
        (fun w' v' => (noRaise_run p w' n).2 _ b he.2) k c ps
    | letE x t v b =>
      simp only [eval]
      exact andThen_ne_raised (ihE env v he.1) (fun w' v' => (noRaise_run p w' n).2 _ b he.2) k c ps
    | ite c' t e =>
      simp only [eval]
      split
      · exact ihE env t he.1 k c ps
      · exact ihE env e he.2 k c ps
      · simp [stuck]
    | caseOption s nE x sE =>
      simp only [eval]
      split
      · exact ihE env nE he.1 k c ps
      · exact ihE _ sE he.2 k c ps
      · simp [stuck]
    | caseResult s x oE y eE =>
      simp only [eval]
      split
      · exact ihE _ oE he.1 k c ps
      · exact ihE _ eE he.2 k c ps
      · simp [stuck]
    | caseSum s i xs t e =>
      simp only [eval]
      split
      · split
        · split
          · exact ihE _ t he.1 k c ps
          · simp [stuck]
        · exact ihE env e he.2 k c ps
      · simp [stuck]
    | fail _ => simp [eval]
    | raise _ _ _ => exact (show False from he).elim
    | unreachable => exact (show False from he).elim
    | withH _ _ _ => exact (show False from he).elim
    | attempt b =>
      simp only [eval]
      have h := ihE env b he
      revert h
      rcases eval H n p w env b with ⟨t, r, w'⟩
      intro h
      cases r with
      | raised k' c' ps' => exact (h k' c' ps' rfl).elim
      | _ => simp
    | attemptK kd b =>
      simp only [eval]
      have h := ihE env b he
      revert h
      rcases eval H n p w env b with ⟨t, r, w'⟩
      intro h
      cases r with
      | raised k' c' ps' => exact (h k' c' ps' rfl).elim
      | _ => simp
    | handle b x hd =>
      simp only [eval]
      have h := ihE env b he.1
      revert h
      rcases eval H n p w env b with ⟨t, r, w'⟩
      intro h
      cases r with
      | failed code =>
        have := (noRaise_run p w' n).2 ((x, .int code) :: env) hd he.2 k c ps
        simpa using this
      | raised k' c' ps' => exact (h k' c' ps' rfl).elim
      | _ => simp
    | handleK kd b x hd =>
      simp only [eval]
      have h := ihE env b he.1
      revert h
      rcases eval H n p w env b with ⟨t, r, w'⟩
      intro h
      cases r with
      | raised k' c' ps' => exact (h k' c' ps' rfl).elim
      | _ => simp
    | letTuple _ _ _ => exact (show False from he).elim
    | loop _ _ _ _ _ _ => exact (show False from he).elim

end Una.F1
