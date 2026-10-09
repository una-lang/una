/-
The formal track of Una (`docs/plan/formal-track.md`, #46): a model of Una Core in Lean 4.
M0 sets up the toolchain, the gate and the first definitions. M1 defines the fragment F1, its
semantics, its structural and labeled typings, proves T1 for both, and states the semantic
reading of the symbolic labels (`UnaFormal/F1/Spike.lean`).
-/
import UnaFormal.Label
import UnaFormal.F1.T1
import UnaFormal.F1.Erase
import UnaFormal.F1.Spike
import UnaFormal.F1.Examples
