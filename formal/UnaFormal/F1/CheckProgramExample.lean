/-
Non-vacuity of M4 completion part A2 (`CheckProgramSound.lean`): a program of three functions,
the `via` function `check`, `rev`, which declassifies through `reveal`, and `caller`, which calls
`rev`, passes the whole-program check by evaluation, so it is accepted by the labeled typing and
within B1, and everything `caller` reaches passes T-Fun. A `#guard` checks that the same program
fails it when the policy and its `via` function disagree (G1).
-/
import UnaFormal.F1.CheckProgramSound
import UnaFormal.F1.ReleaseExample

namespace Una.F1.CheckProgramExample

open ReleaseExample Check

/-- `caller(s: secret Int) -> Bool = rev(s)`: it releases through `reveal`, as `rev` does. -/
def caller : Fun where
  name := "caller"
  tparams := []
  eqParams := []
  params := [("s", .int sec)]
  result := .bool .bot
  op := false
  get := false
  fails := false
  body := .letR "b" (.bool .bot) (.call "rev" [] [.var "s"]) (.atom (.var "b"))
  rels := ["reveal"]

/-- `check`, `rev` and `caller`, with `prog`'s policies `reveal` and `peek`. -/
def prog3 : Program := { prog with funs := [check, rev, caller] }

-- `flowB` is defined by well-founded recursion: it is unsealed for the kernel to evaluate it.
unseal flowB flowsB in
/-- The whole-program check accepts `prog3`, with no function checked at its instances, so its
soundness has no hypothesis left. -/
theorem prog3_certified :
    prog3.LWellFormed ∧ prog3.B1WellFormed ∧
      prog3.le (.ofLabel prog3.acceptsGet) (.ofLabel prog3.getResult) :=
  progOk_sound_nil (by rfl)

unseal flowB flowsB in
/-- Everything `caller` reaches is a function of `prog3` that passes T-Fun. -/
theorem caller_reach : ∀ f ∈ caller.body.callees, ∃ g, prog3.find f = some g ∧ prog3.FunOk g :=
  reach_closed prog3_certified.1 (checkFun_sound (by rfl))

/-- `check` returning its `Int` argument, while `reveal`'s `to` is `Bool`. -/
def checkInt : Fun := { check with result := .int .bot, body := .atom (.var "x") }

-- The whole-program check rejects the program whose policy and `via` function disagree.
#guard !progOk [] { prog3 with funs := [checkInt, rev, caller] }
#guard progOk [] prog3

end Una.F1.CheckProgramExample
