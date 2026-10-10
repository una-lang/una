/-
Non-vacuity of M4 completion part A3 (`Erasure.lean`, `Monomorph.lean`, `Import.finalize`):
`main` calls the generic `g[T](x: T) -> T` at `Int`. The program checked at instances, `P₁`, has
`g`'s instance `g[public Int]` in place of `g`, and `main`'s call renamed to it. It passes the
whole-program check and agrees with the document's program `P₂` up to annotations, both by
evaluation, so `t6_release_erasure` applies to `P₂`, and a run of `main` in `P₂` is, up to
annotations, its run in `P₁`. A `#guard` checks that the agreement fails when the instance's body
differs from `g`'s.
-/
import UnaFormal.F1.Import
import UnaFormal.F1.CheckTests
import UnaFormal.F1.ReleaseExample

namespace Una.F1.MonomorphExample

open Check Import CheckTests

/-- `g[T](x: T) -> T = x`. -/
def g : Fun where
  name := "g"
  tparams := ["T"]
  eqParams := []
  params := [("x", .var "T" (sym ["T"] []))]
  result := .var "T" (sym ["T"] [])
  op := false
  host := HostFx.none
  fails := false
  body := .atom (.var "x")

/-- `main() -> Int = g[Int](5)`. -/
def main : Fun where
  name := "main"
  tparams := []
  eqParams := []
  params := []
  result := .int .bot
  op := false
  host := HostFx.none
  fails := false
  body := .letR "r" (.int .bot) (.call "g" [.int .bot] [.int 5]) (.atom (.var "r"))

/-- `g`'s instance at `Int`, `g[public Int](x: Int) -> Int = x`, as the instance search makes it
(checked below). -/
def gInt : Fun where
  name := "g[public Int]"
  tparams := []
  eqParams := []
  params := [("x", .int .bot)]
  result := .int .bot
  op := false
  host := HostFx.none
  fails := false
  body := .atom (.var "x")

/-- `main` with its call renamed to the instance, as `Import.Expr.toInst` makes it (checked
below). -/
def mainMono : Fun where
  name := "main"
  tparams := []
  eqParams := []
  params := []
  result := .int .bot
  op := false
  host := HostFx.none
  fails := false
  body := .letR "r" (.int .bot) (.call "g[public Int]" [] [.int 5]) (.atom (.var "r"))

/-- The document's program. -/
def P₂ : Program :=
  mkProgram [] Label.bot [] [] [g, main]

/-- The program checked at instances: `main`'s call renamed, `g` replaced by its instance. -/
def monoFuns : List Fun := [mainMono, gInt]

def ren : List (String × String) := [("g[public Int]", "g"), ("g", "g[]")]

/-- `P₁` agrees with `P₂` up to annotations. -/
theorem sim : Sim (renOf ren) { P₂ with funs := monoFuns } P₂ := progSim_sound (by rfl)

-- `flowB` is defined by well-founded recursion: it is unsealed for the kernel to evaluate it.
unseal flowB flowsB in
/-- `P₁` passes the whole-program check, with no hypothesis. -/
theorem certified :
    ({ P₂ with funs := monoFuns } : Program).LWellFormed ∧
      ({ P₂ with funs := monoFuns } : Program).B1WellFormed ∧
      ({ P₂ with funs := monoFuns } : Program).HostWF :=
  progOk_sound_nil (by rfl)

/-- A run of `main` in the document's program is, up to annotations, its run in `P₁`. -/
theorem runs_agree {H : Host} (hai : H.AnonInvariant) (w : World H) (n : Nat) :
    runAnon (renOf ren) (eval H n { P₂ with funs := monoFuns } w [] mainMono.body) =
      runAnon id (eval H n P₂ w [] main.body) :=
  eval_erasure sim hai w n rfl (by rfl)

/-- `main`'s runs in the document's program `P₂` itself are related as `t6_release_host` says,
through `t6_release_erasure_obs`: its hypotheses hold of a real document. -/
theorem main_obs (L : Label) {H : Host} (hlaw : H.Lawful) (hai : H.AnonInvariant) (w : World H) :
    proj P₂ L (eval H 5 P₂ w [] main.body).1 = proj P₂ L (eval H 5 P₂ w [] main.body).1 ∨
      Diverged P₂ L (eval H 5 P₂ w [] main.body).1 (eval H 5 P₂ w [] main.body).1 := by
  obtain ⟨hwf, hb1, hh⟩ := certified
  rcases hwf.funs mainMono (by simp [monoFuns]) with hok | ⟨hv, _⟩
  · obtain ⟨t, ε, ht, _⟩ := hok
    have hrun : eval H 5 P₂ w [] main.body = ([], .ok (.int 5), w) := rfl
    exact t6_release_erasure_obs L sim (proj_funs P₂ monoFuns L) hwf hb1 hh hlaw hai (σ := [])
      (Γ := []) (E := []) (fun _ h => nomatch h) (fun _ h => by simp at h) (fun _ h => nomatch h)
      rfl (WorldRel.refl w) .nil .nil .nil ht
      (e' := main.body) (by rfl) rfl rfl 5 (by rw [hrun]; trivial) (by rw [hrun]; trivial)
      (by rw [hrun]; simp) (by rw [hrun]; simp)
  · obtain ⟨po, hpo, _⟩ := hv
    simp [P₂, mkProgram] at hpo

/-! ### Two runs with releases -/

section Releases

open ReleaseExample (check rev sec env Γ Γ_inPlace)

/-- `g`'s instance at `Bool`. -/
def gBool : Fun where
  name := "g[public Bool]"
  tparams := []
  eqParams := []
  params := [("x", .bool .bot)]
  result := .bool .bot
  op := false
  host := HostFx.none
  fails := false
  body := .atom (.var "x")

/-- `caller(s: secret Int) -> Bool = g[Bool](rev(s))`: it releases whether `s` is 1234 through
`reveal`, then passes the result through the generic `g`. -/
def caller : Fun where
  name := "caller"
  tparams := []
  eqParams := []
  params := [("s", .int sec)]
  result := .bool .bot
  op := false
  host := HostFx.none
  fails := false
  body := .letR "b" (.bool .bot) (.call "rev" [] [.var "s"])
    (.letR "r" (.bool .bot) (.call "g" [.bool .bot] [.var "b"]) (.atom (.var "r")))
  rels := ["reveal"]

/-- `caller` with its call renamed to the instance. -/
def callerMono : Fun where
  name := "caller"
  tparams := []
  eqParams := []
  params := [("s", .int sec)]
  result := .bool .bot
  op := false
  host := HostFx.none
  fails := false
  body := .letR "b" (.bool .bot) (.call "rev" [] [.var "s"])
    (.letR "r" (.bool .bot) (.call "g[public Bool]" [] [.var "b"]) (.atom (.var "r")))
  rels := ["reveal"]

/-- The document's program: `ReleaseExample.prog`'s policies, its `via` function `check`, `rev`,
the generic `g` and `caller`. -/
def R₂ : Program := { ReleaseExample.prog with funs := [check, rev, g, caller] }

def rFuns : List Fun := [check, rev, callerMono, gBool]

def rRen : List (String × String) := [("g[public Bool]", "g"), ("g", "g[]")]

theorem rSim : Sim (renOf rRen) { R₂ with funs := rFuns } R₂ := progSim_sound (by rfl)

unseal flowB flowsB in
theorem rCertified :
    ({ R₂ with funs := rFuns } : Program).LWellFormed ∧
      ({ R₂ with funs := rFuns } : Program).B1WellFormed ∧
      ({ R₂ with funs := rFuns } : Program).HostWF :=
  progOk_sound_nil (by rfl)

theorem sec_hidden' : ¬ Vis ({ R₂ with funs := rFuns } : Program) Label.bot (LExpr.bot.join sec) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [R₂, ReleaseExample.prog, Program.top, sec, LExpr.ofLabel, LExpr.join, LExpr.bot,
      Label.bot, Label.join, Level.join] at h <;> revert h <;> decide

/-- **Two runs of the document's `caller` on different secrets, through `t6_release_erasure_obs`.**
PIN 1234 releases `true` and PIN 1 releases `false`, both visible at `public`, and the first-order
hypothesis is discharged on the two traces: the runs diverge at the release, as
`t6_release_host` says. -/
theorem caller_obs {H : Host} (hlaw : H.Lawful) (hai : H.AnonInvariant) (w : World H) :
    proj R₂ Label.bot (eval H 8 R₂ w (env 1234) caller.body).1 =
        proj R₂ Label.bot (eval H 8 R₂ w (env 1) caller.body).1 ∨
      Diverged R₂ Label.bot (eval H 8 R₂ w (env 1234) caller.body).1
        (eval H 8 R₂ w (env 1) caller.body).1 := by
  obtain ⟨hwf, hb1, hh⟩ := rCertified
  rcases hwf.funs callerMono (by simp [rFuns]) with hok | ⟨hv, _⟩
  · obtain ⟨t, ε, ht, _⟩ := hok
    have r₁ : eval H 8 R₂ w (env 1234) caller.body =
        ([.release "reveal" (.bool true)], .ok (.bool true), w) := rfl
    have r₂ : eval H 8 R₂ w (env 1) caller.body =
        ([.release "reveal" (.bool false)], .ok (.bool false), w) := rfl
    exact t6_release_erasure_obs Label.bot rSim (proj_funs R₂ rFuns Label.bot) hwf hb1 hh hlaw hai
      (σ := []) (Γ := Γ) (E := []) (fun _ h => nomatch h) (fun _ h => by simp at h) Γ_inPlace rfl
      (WorldRel.refl w) (.cons .int .nil) (.cons .int .nil) (.cons (.hidden sec_hidden') .nil) ht
      (e' := caller.body) (by rfl) rfl rfl 8 (by rw [r₁]; trivial) (by rw [r₂]; trivial)
      (by rw [r₁]; simp [Event.FirstOrder, Val.FirstOrder])
      (by rw [r₂]; simp [Event.FirstOrder, Val.FirstOrder])
  · obtain ⟨po, hpo, hvia⟩ := hv
    simp [R₂, ReleaseExample.prog, ReleaseExample.reveal, ReleaseExample.peek] at hpo
    rcases hpo with rfl | rfl <;> simp [callerMono, caller, ReleaseExample.reveal, ReleaseExample.peek] at hvia

/-- The two runs do release different values: the conclusion is the divergence, not equality. -/
theorem caller_diverges {H : Host} (w : World H) :
    proj R₂ Label.bot (eval H 8 R₂ w (env 1234) caller.body).1 ≠
      proj R₂ Label.bot (eval H 8 R₂ w (env 1) caller.body).1 := by
  have r₁ : eval H 8 R₂ w (env 1234) caller.body =
      ([.release "reveal" (.bool true)], .ok (.bool true), w) := rfl
  have r₂ : eval H 8 R₂ w (env 1) caller.body =
      ([.release "reveal" (.bool false)], .ok (.bool false), w) := rfl
  rw [r₁, r₂]
  intro h
  have hv : ∀ v, EvVis R₂ Label.bot (Event.release "reveal" v) := fun v =>
    ⟨_, rfl, Examples.le_self _⟩
  simp [proj, hv] at h

end Releases

-- `gInt` and `mainMono` are what the re-check computes: the instance search's instance of `g` at
-- `Int` (its name, signature and body), and `main` with its call renamed.
#guard s!"g[{instKey [.int .bot]}]" == gInt.name
#guard
  let i := Check.instantiate (fun σ t => t.lsubst rd0 σ) g [.int .bot] gInt.name
  i.name == gInt.name && i.tparams.isEmpty && fmtTy i.result == fmtTy gInt.result &&
    i.params.map (fun q => (q.1, fmtTy q.2)) == gInt.params.map (fun q => (q.1, fmtTy q.2)) &&
    exprSim id i.body gInt.body
#guard exprSim id (Expr.toInst ["g"] main.body) mainMono.body &&
  match Expr.toInst ["g"] main.body with
  | .letR _ _ (.call f [] _) _ => f == "g[public Int]"
  | _ => false
-- The agreement fails when the instance's body is not `g`'s.
#guard !progSim ren { P₂ with funs := [mainMono, { gInt with body := .atom (.int 0) }] } P₂

-- `cond` is recomputed from the final members (Una-1's note on A3): `g1`, checked at its
-- instances but demoted before the certificates, leaves with its instance, so the program checked
-- at instances still agrees with the document's; built from a `cond` that still holds `g1`, it
-- would not, and every declaration of the document would lose its certificate.
#guard
  let g1Int : Fun := { gInt with name := "g1[public Int]" }
  let read : String → Option Fun := fun f => if f == "main" then some main else if f == "g" then some g else none
  let nested : String → Bool := fun f => f == "g" || f == "g1"
  let insts := [("g", gInt), ("g1", g1Int)]
  let (cond, mono, ren') := monoParts ["main", "g"] nested read insts []
  cond == ["g"] && progSim ren' { P₂ with funs := mono } P₂ &&
    -- the same parts with the stale `cond`, `g1` included
    !progSim (ren' ++ [("g1[public Int]", "g1")]) { P₂ with funs := mono ++ [g1Int] } P₂

end Una.F1.MonomorphExample
