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
import UnaFormal.F1.Inst
import UnaFormal.F1.UnarySound
import UnaFormal.F1.T6
import UnaFormal.F1.InstVal
import UnaFormal.F1.BinaryInst
import UnaFormal.F1.Mono
import UnaFormal.F1.T6Example
import UnaFormal.F1.Subst
import UnaFormal.F1.InPlace
import UnaFormal.F1.Decision
import UnaFormal.F1.Check
import UnaFormal.F1.Import
import UnaFormal.F1.CheckTests
import UnaFormal.F1.ImportTests
import UnaFormal.F1.SumImportTests
import UnaFormal.F1.FirstImportTests
import UnaFormal.F1.Release
import UnaFormal.F1.ReleaseExample
import UnaFormal.F1.CheckSound
import UnaFormal.F1.CheckSoundCore
import UnaFormal.F1.CheckSoundExample
import UnaFormal.F1.CheckProgram
import UnaFormal.F1.CheckProgramSound
import UnaFormal.F1.CheckProgramExample
import UnaFormal.F1.FailingReleaseExample
import UnaFormal.F1.Erasure
import UnaFormal.F1.Monomorph
import UnaFormal.F1.MonomorphExample
import UnaFormal.F1.OldLeaks
import UnaFormal.F1.Untyped
import UnaFormal.F1.NewRules
import UnaFormal.F1.Runs
import UnaFormal.F1.HostExamples
import UnaFormal.F1.HostFree
import UnaFormal.F1.HostFreeReach
import UnaFormal.F1.HostFreeExample
import UnaFormal.F1.FilesCertExample
import UnaFormal.F1.HttpExample
import UnaFormal.F1.EnvExample
import UnaFormal.F1.ResultExample
import UnaFormal.F1.RecordExample
import UnaFormal.F1.ArithExample
import UnaFormal.F1.SumExample
import UnaFormal.F1.HandlerExample
import UnaFormal.F1.HandlerImportTests
import UnaFormal.F1.WithImportTests
import UnaFormal.F1.DecisionExample
import UnaFormal.F1.FirstExample
import UnaFormal.F1.LoopLeak
import UnaFormal.F1.LoopExample
