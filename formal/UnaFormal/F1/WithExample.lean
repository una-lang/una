/-
Non-vacuity of #538 and of the coordinator's ruling E: **a `with` scope charges what the handler
it installs does where it installs it.**

The program has one operation of a `with` scope, `Bell.ring(n: Int) -> Unit`, which accepts
`public`; its one handler, `m.noisy.ring`, prints at `Console`, which accepts `public`. The function
`m.chime` invokes it, with the marker `"@Bell.ring"` its callers pass. The secret is the
environment's `SECRET`, `internal`, and the flag `c` is `SECRET == "1"`.

- **(a)** `if c then (chime() with @bell = noisy()) else ()`: the call of `chime` under the secret
  emits, through the printing handler, so it is rejected; it leaks (the line `ring` is printed only
  when `c` holds), and it has no typing.
- **(b)** `chime() with @bell = noisy()` at a public program counter: certified, and it runs; the
  handler's events are at its floor.
- **(c)** `chime()` where no scope installs `Bell.ring`'s handler: no typing, and the check
  rejects it, since its marker is not bound.
-/
import UnaFormal.F1.Import

namespace Una.F1.WithExample

open Check

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `print("t")`, then `k`. -/
def printLit (t : String) (k : Expr) : Expr :=
  .letR "%t" (.text .bot) (.atom (.text t)) (.letR "%u" (.unit .bot) (.op (.var "%t")) k)

/-- The handler `noisy` of `Bell.ring`: it prints. -/
def noisy : Fun where
  name := "m.noisy.ring"
  tparams := []
  eqParams := []
  params := [("n", .int .bot)]
  op := true
  host := HostFx.none
  result := .unit .bot
  fails := false
  body := printLit "ring" (.atom .unit)

/-- `chime()`: `@bell.ring(1)`, with the marker its callers pass. -/
def chime : Fun where
  name := "m.chime"
  tparams := []
  eqParams := []
  params := [(markerName "Bell.ring", .text .bot)]
  op := false
  host := HostFx.none
  result := .unit .bot
  fails := false
  body := .letR "u" (.unit .bot) (.invokeOp "Bell.ring" [.int 1]) (.atom (.var "u"))
  invokes := ["Bell.ring"]

/-- `Console` accepts `public`; `Bell.ring` accepts `public`, its one handler `noisy`. -/
def prog : Program :=
  Import.mkProgram [] Label.bot labels [] [] [noisy, chime] [] [("m.noisy.ring", Label.bot)]
    [("m.noisy.ring", "Bell.ring")] [("Bell.ring", ["m.noisy.ring"])]

def wSet (v : String) : World Host.unit := ⟨(), (), fun x => if x = "SECRET" then some v else none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

theorem set_rel (a b : String) : WorldRel prog Label.bot (wSet a) (wSet b) :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun x => by simp only [wSet]; split <;> rfl⟩

unseal flowB flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

theorem out_vis (s : String) : EvVis prog Label.bot (.out s) := LExpr.le_refl _

theorem same_lines {e : Expr} {t : LTy} {ε : Eff} (ht : LExprTy prog [] [] .bot e t ε) (n : Nat)
    (h₁ : Term (eval Host.unit n prog (wSet "1") [] e).2.1)
    (h₂ : Term (eval Host.unit n prog (wSet "22") [] e).2.1) :
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] e).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "22") [] e).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "22") .nil .nil .nil ht n h₁ h₂).1

/-- `when Env.get("SECRET") is None -> () is Some(s) -> let c = s == "1" in k`. -/
def withFlag (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s"
      (.letR "c" (.bool intl) (.eq (.var "s") (.text "1")) k))

/-- `chime()`, passing the marker. -/
def callChime : Expr :=
  .letR "v" (.unit .bot) (.call "m.chime" [] [.var (markerName "Bell.ring")]) (.atom (.var "v"))

/-- `chime() with @bell = noisy()`. -/
def installed : Expr := .withH "Bell.ring" "m.noisy.ring" callChime

/-! ### (a) A printing handler installed under a secret program counter -/

def underSecret : Expr :=
  withFlag (.letE "b" (.unit .bot) (.ite (.var "c") installed (.atom .unit)) (.atom .unit))

unseal flowB flowsB in
theorem underSecret_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) underSecret = .error msg := ⟨_, rfl⟩

theorem underSecret_leaks : (eval Host.unit 20 prog (wSet "1") [] underSecret).1 = [.out "ring"] ∧
    (eval Host.unit 20 prog (wSet "22") [] underSecret).1 = [] := ⟨rfl, rfl⟩

theorem underSecret_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot underSecret t ε := by
  intro ht
  have h := same_lines ht 20 (by exact True.intro) (by exact True.intro)
  rw [underSecret_leaks.1, underSecret_leaks.2, proj_single_vis (out_vis _)] at h
  simp [proj] at h

/-! ### (b) The same `with` at a public program counter -/

unseal flowB flowsB in
theorem installed_checked : ∃ ε, checkExpr prog [] [] .bot (.unit .bot) installed = .ok ε := ⟨_, rfl⟩

theorem installed_runs : (eval Host.unit 20 prog (wSet "1") [] installed).1 = [.out "ring"] := rfl

/-! ### (c) An invocation no scope installs -/

unseal flowB flowsB in
theorem unbound_rejected :
    ∃ msg, checkExpr prog [] [] .bot (.unit .bot) callChime = .error msg := ⟨_, rfl⟩

theorem unbound_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot callChime t ε := by
  intro ht
  cases ht with
  | letR hr _ _ _ =>
    cases hr with
    | call _ _ _ _ _ _ has _ _ _ =>
      cases has with
      | cons ha _ =>
        cases ha with
        | var hl => simp [lctxLookup] at hl

end Una.F1.WithExample
