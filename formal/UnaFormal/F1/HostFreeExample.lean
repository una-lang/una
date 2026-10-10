/-
Non-vacuity of `HostFree.lean`: the re-checker's theorems on a host that breaks H1, as the real
host did before v0.3.5 (S-16). `s16` fails a read of a file whose content is not valid text, so two file
systems with the same files, one of them holding such a file, are told apart by a read
(`s16_unlawful`). `t6_release_hostFree` still applies to `ReleaseExample`'s program on `s16`,
from exactly those two file systems (`s16_guesses`), and `t6_release_erasure_obs_hostFree` to
`MonomorphExample`'s document (`s16_caller_obs`).
-/
import UnaFormal.F1.HostFree
import UnaFormal.F1.MonomorphExample

namespace Una.F1.HostFreeExample

/-- A host whose read fails on a file whose content is not valid text, here `"\xff"`, as the real
host's `Files.read` failed on content that is not valid UTF-8 before v0.3.5 (S-16). -/
def s16 : Host where
  FS := List (String × String)
  Srv := Unit
  Shape := List String
  shape := fun fs => fs.map Prod.fst
  read := fun path fs => (fs.lookup path).bind fun c => if c = "\xff" then none else some c
  write := fun path c fs => some ((path, c) :: fs)
  request := fun _ _ s => (none, s)

/-- A file system with one valid file, and one with the same file holding invalid text. -/
def fsOk : World s16 := ⟨[("f", "ok")], ()⟩
def fsBad : World s16 := ⟨[("f", "\xff")], ()⟩

/-- **`s16` breaks H1**: the two file systems have the same shape, and a read of `f` succeeds in
one and fails in the other. -/
theorem s16_unlawful : ¬ s16.Lawful := fun h => by
  have := h.read_fails (a := fsOk.fs) (b := fsBad.fs) "f" rfl
  revert this
  decide

open ReleaseExample in
/-- `ReleaseExample`'s program performs no host operation. -/
theorem prog_hostFree : prog.HostFree := Program.hostFreeB_sound rfl

open ReleaseExample in
/-- The two file systems are the same to an observer at `public`: the file-state label is
`public`, and they have the same files. -/
theorem worlds_rel : WorldRel prog Label.bot fsOk fsBad :=
  ⟨fun _ => rfl, fun h => absurd h intl_hidden⟩

open ReleaseExample in
theorem env_hostFree (n : Int) : Val.HostFreeEnv (env n) := by
  simp [env, Val.HostFreeEnvIn, Val.HostFreeIn]

open ReleaseExample in
/-- **T6 with declassification on a host that breaks H1.** PINs 1 and 2, from the two file
systems `s16` tells apart, release the same value: their runs are related, and leave file systems
the observer cannot tell apart, although `s16` is not lawful. -/
theorem s16_guesses :
    BRelR prog Label.bot (V prog Label.bot ((LTy.unit LExpr.bot).lsubst prog.recordDeep []))
      (Vis prog Label.bot (bodyEff.inst (prog.sd [])).failLabel)
      (eval s16 6 prog fsOk (env 1) body) (eval s16 6 prog fsBad (env 2) body) :=
  t6_release_hostFree ReleaseExample.prog_wf ReleaseExample.prog_b1 ReleaseExample.prog_host
    prog_hostFree (σ := []) (fun _ h => nomatch h) (fun _ h => by simp at h)
    ReleaseExample.Γ_inPlace rfl worlds_rel (ReleaseExample.envs_typed 1)
    (ReleaseExample.envs_typed 2) (ReleaseExample.envs_rel 1 2) ReleaseExample.body_typed
    (env_hostFree 1) (env_hostFree 2) rfl 6

open ReleaseExample in
/-- The runs end, and the related case holds: the same events, and the worlds they started from. -/
example : eval s16 6 prog fsOk (env 1) body =
      ([.release "reveal" (.bool false), .out "false"], .ok .unit, fsOk) ∧
    eval s16 6 prog fsBad (env 2) body =
      ([.release "reveal" (.bool false), .out "false"], .ok .unit, fsBad) := ⟨rfl, rfl⟩

open MonomorphExample ReleaseExample in
/-- **The document's runs, on a host that breaks H1.** `MonomorphExample.caller_obs` on `s16`,
from the two file systems it tells apart: the runs diverge at the release, as on a lawful host. -/
theorem s16_caller_obs :
    proj R₂ Label.bot (eval s16 8 R₂ fsOk (env 1234) caller.body).1 =
        proj R₂ Label.bot (eval s16 8 R₂ fsBad (env 1) caller.body).1 ∨
      Diverged R₂ Label.bot (eval s16 8 R₂ fsOk (env 1234) caller.body).1
        (eval s16 8 R₂ fsBad (env 1) caller.body).1 := by
  obtain ⟨hwf, hb1, hh⟩ := rCertified
  rcases hwf.funs callerMono (by simp [rFuns]) with hok | ⟨hv, _⟩
  · obtain ⟨t, ε, ht, _⟩ := hok
    have r₁ : eval s16 8 R₂ fsOk (env 1234) caller.body =
        ([.release "reveal" (.bool true)], .ok (.bool true), fsOk) := rfl
    have r₂ : eval s16 8 R₂ fsBad (env 1) caller.body =
        ([.release "reveal" (.bool false)], .ok (.bool false), fsBad) := rfl
    exact t6_release_erasure_obs_hostFree Label.bot rSim (proj_funs R₂ rFuns Label.bot) hwf hb1 hh
      (Program.hostFreeB_sound rfl) (σ := []) (Γ := Γ) (E := []) (fun _ h => nomatch h)
      (fun _ h => by simp at h) Γ_inPlace rfl (.cons .int .nil) (.cons .int .nil)
      (.cons (.hidden sec_hidden') .nil) ht (e' := caller.body) (by rfl) rfl rfl rfl
      (env_hostFree 1234) (env_hostFree 1) 8 (by rw [r₁]; trivial) (by rw [r₂]; trivial)
  · obtain ⟨po, hpo, hvia⟩ := hv
    simp [R₂, ReleaseExample.prog, ReleaseExample.reveal, ReleaseExample.peek] at hpo
    rcases hpo with rfl | rfl <;> simp [callerMono] at hvia

end Una.F1.HostFreeExample
