/-
Non-vacuity of M4 completion part A1 (`CheckSoundCore.lean`): the checker accepts `rev` and the
policies of `ReleaseExample`, so the soundness theorems give their typings, and it
rejects the call of a declassifying function under a `secret` condition, which has no typing.
-/
import UnaFormal.F1.CheckSoundCore
import UnaFormal.F1.ReleaseExample

namespace Una.F1.CheckSoundExample

open ReleaseExample Check

-- `flowB` is defined by well-founded recursion: it is unsealed for the kernel to evaluate it.
unseal flowB flowsB in
/-- The checker accepts `rev`, which declassifies through `reveal`, so it passes T-Fun. -/
theorem rev_ok : prog2.FunOk rev := checkFun_sound (by rfl)

/-- The checker accepts `prog`'s two policies, so the program is within B1's restrictions. -/
theorem prog_b1_checked : prog.B1WellFormed := checkB1_sound (by rfl)

-- The checker rejects `underSecret`, as `release_under_secret_untyped` says it must.
#guard match checkExpr prog2 [] Γc .bot (.bool .bot) underSecret with
  | .ok _ => false
  | .error _ => true

end Una.F1.CheckSoundExample
