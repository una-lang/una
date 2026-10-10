/-
M5: **across runs** (ADR 0085 decision 4, `docs/plan/formal-track-m5.md` §4). A package's runs
follow one another: each starts from the world the previous one left. `t6_host` relates the
worlds two runs leave, so by induction a sequence of runs of a program, from worlds the observer
cannot tell apart, keeps them so, and every run of it shows the observer the same events: an
`internal` write of one run cannot be observed by a later run's public read.
-/
import UnaFormal.F1H.T6

namespace Una.F1H

open Una.F1 (Label LExpr)

variable {p : Program} {L : Label} {H : Host}

/-- The world after `k` runs of the body `e` with `n` fuel each, from `w`, when every run ended. -/
def seqWorld (H : Host) (n : Nat) (p : Program) (e : Expr) : Nat → World H → Option (World H)
  | 0, w => some w
  | k + 1, w =>
    match seqWorld H n p e k w with
    | some w' =>
      match (eval H n p w' [] e).2.1 with
      | .ok _ | .failed _ => some (eval H n p w' [] e).2.2
      | _ => none
    | none => none

theorem seqWorld_succ {n : Nat} {e : Expr} {k : Nat} {w a : World H}
    (h : seqWorld H n p e (k + 1) w = some a) :
    ∃ a', seqWorld H n p e k w = some a' ∧ Term (eval H n p a' [] e).2.1 ∧ a = (eval H n p a' [] e).2.2 := by
  simp only [seqWorld] at h
  split at h
  · rename_i a' ha'
    refine ⟨a', ha', ?_⟩
    split at h <;> simp_all [Term]
  · simp at h

/-- **Across runs**: the worlds that `k` runs of a program leave, from worlds the observer cannot
tell apart, are worlds it cannot tell apart. -/
theorem across_runs (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF) (hlaw : H.Lawful)
    {e : Expr} {t : LTy} {ε : Eff} (he : LExprTy p [] [] .bot e t ε) (n : Nat) :
    ∀ (k : Nat) {w₁ w₂ a b : World H}, WorldRel p L w₁ w₂ →
      seqWorld H n p e k w₁ = some a → seqWorld H n p e k w₂ = some b → WorldRel p L a b
  | 0, _, _, _, _, hw, h₁, h₂ => by
    simp only [seqWorld, Option.some.injEq] at h₁ h₂
    subst h₁ h₂
    exact hw
  | k + 1, _, _, _, _, hw, h₁, h₂ => by
    obtain ⟨a', ha', ht₁, rfl⟩ := seqWorld_succ h₁
    obtain ⟨b', hb', ht₂, rfl⟩ := seqWorld_succ h₂
    have hw' := across_runs hwf hpol hh hlaw he n k hw ha' hb'
    exact (t6_host_ground (Γ := []) hwf hpol hh hlaw (fun _ h => nomatch h) rfl hw' .nil .nil .nil he n
      ht₁ ht₂).2.2

/-- **And every later run shows the observer the same events.** -/
theorem across_runs_events (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF)
    (hlaw : H.Lawful) {e : Expr} {t : LTy} {ε : Eff} (he : LExprTy p [] [] .bot e t ε) (n k : Nat)
    {w₁ w₂ a b : World H} (hw : WorldRel p L w₁ w₂) (h₁ : seqWorld H n p e k w₁ = some a)
    (h₂ : seqWorld H n p e k w₂ = some b) :
    Term (eval H n p a [] e).2.1 → Term (eval H n p b [] e).2.1 →
      proj p L (eval H n p a [] e).1 = proj p L (eval H n p b [] e).1 :=
  fun ht₁ ht₂ => (t6_host_ground (Γ := []) hwf hpol hh hlaw (fun _ h => nomatch h) rfl
    (across_runs hwf hpol hh hlaw he n k hw h₁ h₂) .nil .nil .nil he n ht₁ ht₂).1

end Una.F1H
