/-
ADR 0116 rule 7 and V1 (#946): **a decreasing measure ends a `for until` loop**; and rules 13 and
14: **a host stop ends a `for ever` loop** at its stop point (the end of this file).

The measure `m` of `for until c decreases m` is never evaluated (V1): it is the verifier's, so the
theorem takes it as any function of the loop's state into the integers. Its two obligations are
hypotheses, as the verifier proves them, over every state the loop's invariant allows (the
bindings the body updates, fresh at their declared types, `spec/17 §5` rule 3) and every answer
of the host:

- **`decreases_bound`**: where the condition is `false`, so that the loop continues, `m ≥ 0`;
- **`decreases_step`**: an iteration that ends at the end of the body or at a `skip` leaves a
  state whose measure is less than the one it started from. An iteration that ends at `stop`
  carries neither obligation: it ends the loop.

Rule 9 states what "every iteration ends" rests on, outside this theorem: a run of the condition
and of the body ends, after finitely many steps, within the fuel it is given. Under these
hypotheses the loop ends within the measure's bound: the condition runs at most `m + 2` times,
which the fuel of its iterations bounds (`walkUntil`), so a run with that much fuel does not run
out of it.
-/
import UnaFormal.F1.Eval

namespace Una.F1

variable {H : Host}

/-- A run that ended, whatever its outcome: not out of fuel. -/
abbrev Ends (r : Run H) : Prop := r.2.1 ≠ .outOfFuel

theorem ends_andThen {first : Run H} {rest : World H → Val → Run H} (h₁ : Ends first)
    (h₂ : ∀ v, first.2.1 = .ok v → Ends (rest first.2.2 v)) : Ends (andThen first rest) := by
  obtain ⟨t, r, w⟩ := first
  cases r with
  | ok v =>
    have h := h₂ v rfl
    simp only [andThen]
    revert h
    obtain ⟨_, _, _⟩ := rest w v
    exact id
  | outOfFuel => exact absurd rfl h₁
  | _ => intro h; cases h

theorem ends_iter {r : Run H} (h : Ends r) : Ends (Run.iter r) := by
  obtain ⟨t, res, w⟩ := r
  cases res with
  | exit b st => cases b <;> (intro h'; cases h')
  | _ => exact h

theorem ends_endLoop (n : Nat) {r : Run H} (h : Ends r) : Ends (Run.endLoop n r) := by
  obtain ⟨t, res, w⟩ := r
  cases res with
  | exit b st =>
    cases b
    · intro h'; cases h'
    · simp only [Run.endLoop]; split <;> (intro h'; cases h')
  | _ => exact h

/-- The fuel a `for until` needs from a state of measure `μ`: when `μ < 0`, the condition holds
there (`decreases_bound`), so one check; otherwise `μ + 2` checks bound its iterations. -/
def untilNeed (μ : Int) : Nat := if μ < 0 then 1 else μ.toNat + 2

/-- **A decreasing measure ends a `for until`** (ADR 0116 V1), at the level of its iterations:
under an invariant `I` of its states, when the condition and an iteration's body end, and the
measure `m` satisfies `decreases_bound` and `decreases_step`, the loop ends with `k` checks of
its condition as soon as `k` is at least `untilNeed (m st)`, whatever the host answers. -/
theorem walkUntil_ends (m : List Val → Int) (I : List Val → Prop)
    {cond body : World H → List Val → Run H}
    (hc : ∀ w st, I st → Ends (cond w st))
    (hb : ∀ w w' st, I st → (cond w st).2.1 = .ok (.bool false) → Ends (body w' st))
    (hbound : ∀ w st, I st → (cond w st).2.1 = .ok (.bool false) → 0 ≤ m st)
    (hstep : ∀ w w' st st', I st → (cond w st).2.1 = .ok (.bool false) →
      (Run.iter (body w' st)).2.1 = .ok (.tuple st') → m st' < m st)
    (hinv : ∀ w w' st st', I st → (cond w st).2.1 = .ok (.bool false) →
      (Run.iter (body w' st)).2.1 = .ok (.tuple st') → I st') :
    ∀ (k : Nat) (w : World H) (st : List Val), I st → untilNeed (m st) ≤ k →
      Ends (walkUntil cond body k w st)
  | 0, w, st, _, hk => by
    unfold untilNeed at hk
    split at hk <;> omega
  | k + 1, w, st, hI, hk => by
    simp only [walkUntil]
    refine ends_andThen (hc w st hI) (fun v hv => ?_)
    match v, hv with
    | .bool true, _ => intro h; cases h
    | .bool false, hv =>
      have h0 := hbound w st hI hv
      refine ends_andThen (ends_iter (hb w _ st hI hv)) (fun r hr => ?_)
      match r, hr with
      | .tuple st', hr =>
        simp only
        split
        · have hlt := hstep w _ st st' hI hv hr
          refine walkUntil_ends m I hc hb hbound hstep hinv k _ st' (hinv w _ st st' hI hv hr) ?_
          unfold untilNeed at hk ⊢
          rw [if_neg (by omega)] at hk
          split <;> omega
        · intro h; cases h
      | .int _, _ | .bool _, _ | .text _, _ | .unit, _ | .list _, _ | .none, _ | .some _, _
      | .ok _, _ | .err _, _ | .record _ _, _ | .variant _ _ _, _ | .closure _ _ _, _ =>
        intro h; cases h
    | .int _, _ | .text _, _ | .unit, _ | .list _, _ | .none, _ | .some _, _ | .ok _, _
    | .err _, _ | .record _ _, _ | .variant _ _ _, _ | .closure _ _ _, _ | .tuple _, _ =>
      intro h; cases h

/-- **A decreasing measure ends a `for until`** (ADR 0116 rule 7 and V1): a run of
`loop until(c) with (b̄) = (ā) do e`, with `n + 1` fuel, does not run out of it when, over the
states its invariant `I` allows, a run of the condition and of the body with `n` fuel ends (rule
9), the obligations `decreases_bound` and `decreases_step` of the measure `m` hold, and the
measure of the initial state is below the fuel: `m(ā) + 2 ≤ n`. -/
theorem loopUntil_ends (m : List Val → Int) (I : List Val → Prop) {n : Nat} {p : Program}
    {w : World H} {env : Env} {cond body : Expr} {state : List (String × LTy)} {init : List Atom}
    {st₀ : List Val} (hinit : evalAtoms env init = some st₀) (hI₀ : I st₀)
    (hfuel : (m st₀).toNat + 2 ≤ n)
    (hc : ∀ w st, I st → Ends (eval H n p w (bindParams state st env) cond))
    (hb : ∀ w w' st, I st → (eval H n p w (bindParams state st env) cond).2.1 = .ok (.bool false) →
      Ends (eval H n p w' (bindParams state st env) body))
    (hbound : ∀ w st, I st →
      (eval H n p w (bindParams state st env) cond).2.1 = .ok (.bool false) → 0 ≤ m st)
    (hstep : ∀ w w' st st', I st →
      (eval H n p w (bindParams state st env) cond).2.1 = .ok (.bool false) →
      (Run.iter (eval H n p w' (bindParams state st env) body)).2.1 = .ok (.tuple st') →
        m st' < m st)
    (hinv : ∀ w w' st st', I st →
      (eval H n p w (bindParams state st env) cond).2.1 = .ok (.bool false) →
      (Run.iter (eval H n p w' (bindParams state st env) body)).2.1 = .ok (.tuple st') → I st') :
    Ends (eval H (n + 1) p w env (.loopUntil cond state init body)) := by
  simp only [eval, hinit]
  split
  · refine ends_endLoop _ (walkUntil_ends m I hc hb hbound hstep hinv n w st₀ hI₀ ?_)
    unfold untilNeed
    split <;> omega
  · intro h; cases h


/-! ### A host stop ends a `for ever` (ADR 0116 rules 13 and 14)

A `for ever` loop ends only by `stop`, a failure or the host. Under the host's stop schedule
(`Host.stopAt`), a loop whose schedule stops it at the start of an iteration ends there, when each
iteration before it ends; then `on stop` runs, and the loop ends when it does. -/

theorem ends_finishEver {k : World H → List Val → Run H} {st₀ : List Val} {r : Run H}
    (hr : Ends r) (hk : ∀ w st, Ends (k w st)) : Ends (finishEver k st₀ r) := by
  obtain ⟨t, res, w⟩ := r
  cases res with
  | ok v =>
    cases v with
    | tuple st =>
      have := hk w st
      simp only [finishEver]
      revert this
      obtain ⟨_, _, _⟩ := k w st
      exact id
    | _ => intro h; cases h
  | failed c =>
    have := hk w st₀
    simp only [finishEver]
    revert this
    obtain ⟨_, r', _⟩ := k w st₀
    cases r' <;> simp [Ends]
  | raised kd c ps =>
    have := hk w st₀
    simp only [finishEver]
    revert this
    obtain ⟨_, r', _⟩ := k w st₀
    cases r' <;> simp [Ends]
  | outOfFuel => exact absurd rfl hr
  | _ => exact hr

/-- **The schedule's stop point ends the loop**: when the schedule stops the loop at the start of
iteration `i + j`, and each iteration's body ends, on the states an invariant `I` allows, the loop
from iteration `i` ends with more than `j` iterations of fuel, whatever the host answers. -/
theorem walkEver_ends {stops : Nat → Bool} {body : World H → List Val → Run H}
    (I : List Val → Prop) (hb : ∀ w st, I st → Ends (body w st))
    (hinv : ∀ w st st', I st → (Run.iter (body w st)).2.1 = .ok (.tuple st') → I st') :
    ∀ (j i k : Nat) (w : World H) (st : List Val), stops (i + j) = true → I st → j < k →
      Ends (walkEver stops body k i w st)
  | _, _, 0, _, _, _, _, hk => absurd hk (Nat.not_lt_zero _)
  | j, i, k + 1, w, st, hs, hI, hk => by
    simp only [walkEver]
    split
    · intro h; cases h
    · rename_i hni
      cases j with
      | zero => exact absurd hs (by simpa using hni)
      | succ j =>
        refine ends_andThen (ends_iter (hb w st hI)) (fun r hr => ?_)
        match r, hr with
        | .tuple st', hr =>
          simp only
          split
          · exact walkEver_ends I hb hinv j (i + 1) k _ st'
              (by rw [show i + 1 + j = i + (j + 1) by omega]; exact hs) (hinv w st st' hI hr) (by omega)
          · intro h; cases h
        | .int _, _ | .bool _, _ | .text _, _ | .unit, _ | .list _, _ | .none, _ | .some _, _
        | .ok _, _ | .err _, _ | .record _ _, _ | .variant _ _ _, _ | .closure _ _ _, _ =>
          intro h; cases h

/-- **A host stop ends a `for ever` loop** (ADR 0116 rules 13 and 14): a run of
`loop ever with (b̄) = (ā) do e on stop o`, with `n + 1` fuel, does not run out of it when the
host's schedule stops the loop at the start of an iteration `j < n`, and, over the states its
invariant `I` allows, a run of the body and of `on stop` with `n` fuel ends. -/
theorem loopEver_ends (I : List Val → Prop) {n : Nat} {p : Program} {w : World H} {env : Env}
    {site : Nat} {body o : Expr} {state os : List (String × LTy)} {init : List Atom}
    {st₀ : List Val} (hinit : evalAtoms env init = some st₀) (hI₀ : I st₀) (j : Nat)
    (hstop : H.stopAt site j = true) (hj : j < n)
    (hb : ∀ w st, I st → Ends (eval H n p w (bindParams state st env) body))
    (hinv : ∀ w st st', I st →
      (Run.iter (eval H n p w (bindParams state st env) body)).2.1 = .ok (.tuple st') → I st')
    (ho : ∀ w st, Ends (eval H n p w (bindParams os st env) o)) :
    Ends (eval H (n + 1) p w env (.loopEver site state init body os o)) := by
  simp only [eval, hinit]
  split
  · refine ends_finishEver (ends_endLoop _ (walkEver_ends I hb hinv j 0 n w st₀
      (by simpa using hstop) hI₀ hj)) (fun w st => ?_)
    split
    · exact ho w st
    · intro h; cases h
  · intro h; cases h

end Una.F1
