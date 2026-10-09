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
  get := false
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
  get := false
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
  get := false
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
  get := false
  fails := false
  body := .letR "r" (.int .bot) (.call "g[public Int]" [] [.int 5]) (.atom (.var "r"))

/-- The document's program. -/
def P₂ : Program :=
  { categories := [], acceptsOp := Label.bot, acceptsGet := Label.bot, getResult := Label.bot,
    records := [], policies := [], funs := [g, main] }

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
      ({ P₂ with funs := monoFuns } : Program).le (.ofLabel Label.bot) (.ofLabel Label.bot) :=
  progOk_sound_nil (by rfl)

/-- A run of `main` in the document's program is, up to annotations, its run in `P₁`. -/
theorem runs_agree (w : World) (n : Nat) :
    runAnon (renOf ren) (eval n { P₂ with funs := monoFuns } w [] mainMono.body) =
      runAnon id (eval n P₂ w [] main.body) :=
  eval_erasure sim w n rfl (by rfl)

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

end Una.F1.MonomorphExample
