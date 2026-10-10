/-
Worked instances of T6 with declassification (option B1, `Release.lean`): a program whose
policy releases whether a secret PIN is 1234, run on different secrets.

- Different releases: the runs diverge at the release (`pin_diverges`).
- A `via` that is not injective: two different secrets release the same value, and the
  corollary gives the full conclusion of `t6_host` (`guesses_agree`). The theorem tracks the values
  released, not the secrets.
- A hidden release, to `internal`: an observer at `public` sees none, and the runs are related
  although they release different values (`hidden_release_related`).
- A call of a function that declassifies, under a `secret` condition, has no typing
  (`release_under_secret_untyped`).
-/
import UnaFormal.F1.Release
import UnaFormal.F1.Examples
import UnaFormal.F1.CheckProgramSound

namespace Una.F1.ReleaseExample

open Examples (le_self)

def sec : LExpr := .ofLabel ⟨.secret, []⟩
def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `check(x: secret Int) -> Bool = x == 1234`, the `via` function. -/
def check : Fun where
  name := "check"
  tparams := []
  eqParams := []
  params := [("x", .int sec)]
  result := .bool .bot
  op := false
  host := HostFx.none
  fails := false
  body := .letR "r" (.bool sec) (.eq (.var "x") (.int 1234)) (.atom (.var "r"))

/-- `policy reveal from secret Int to public Bool via check`. -/
def reveal : Policy := ⟨"reveal", .int sec, .bool .bot, "check"⟩

/-- `policy peek from secret Int to internal Bool via check`. -/
def peek : Policy := ⟨"peek", .int sec, .bool intl, "check"⟩

def prog : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := Label.bot
  acceptsWrite := Label.bot
  acceptsReq := Label.bot
  readResult := ⟨.internal, []⟩
  reqResult := ⟨.internal, []⟩
  reqFloor := ⟨.internal, []⟩
  fileState := Label.bot
  records := []
  policies := [reveal, peek]
  funs := [check]

theorem check_typed : ExprTy prog (bindTys check.params (check.paramTys [])) check.body
    check.result.erase :=
  .letR (.eq (.var rfl) .int) (.atom (.var rfl))

/-- The labeled typing accepts the program: `check` is the `via` function of both policies. -/
theorem prog_wf : prog.LWellFormed where
  funs fn h := by
    simp only [prog, List.mem_singleton] at h
    subst h
    exact .inr ⟨⟨reveal, by simp [prog], rfl⟩, check_typed⟩
  records rec h := by simp [prog] at h
  recordDeep rec h := by simp [prog] at h
  policies po h := by
    simp only [prog, List.mem_cons, List.not_mem_nil, or_false] at h
    rcases h with rfl | rfl <;>
      exact ⟨fun _ => rfl, fun _ => rfl, check, rfl, rfl, rfl⟩
  categories := by
    simp [Program.CatsDeclared, prog, check, reveal, peek, sec, intl, Label.bot, LTy.CatsIn,
      LExpr.CatsIn, LExpr.bot, LExpr.ofLabel]
  recordsGround _ h := by simp [prog] at h
  eqScoped := by simp [prog, check]
  sigsScoped := by
    simp [prog, check, LTy.ScopedIn, LExpr.SymsIn, sec, LExpr.bot, LExpr.ofLabel]
  inPlace := by
    refine ⟨?_, ?_, ?_⟩ <;>
      simp [prog, check, reveal, peek, sec, intl, LTy.InPlace, LExpr.bot, LExpr.ofLabel]

/-- The program's host labels are those `t6_release_host` asks. -/
theorem prog_host : prog.HostWF := Check.hostWfB_sound rfl

/-- The program is within B1's restrictions. -/
theorem prog_b1 : prog.B1WellFormed where
  src po h := by
    simp only [prog, List.mem_cons, List.not_mem_nil, or_false] at h
    rcases h with rfl | rfl <;> trivial
  dst po h := by
    simp only [prog, List.mem_cons, List.not_mem_nil, or_false] at h
    rcases h with rfl | rfl <;>
      exact ⟨⟨rfl, rfl⟩, trivial⟩
  via po h := by
    simp only [prog, List.mem_cons, List.not_mem_nil, or_false] at h
    rcases h with rfl | rfl <;>
      exact ⟨check, rfl, rfl, rfl, rfl, ⟨trivial, trivial⟩⟩

/-- `s: secret Int`. -/
def Γ : LCtx := [("s", .int sec)]

/-- `let b: Bool = reveal(s); @log.write(b)`: the release of whether the PIN is 1234, written
to a public sink. -/
def body : Expr :=
  .letR "b" (.bool .bot) (.declassify "reveal" (.var "s"))
    (.letR "u" (.unit .bot) (.op (.var "b")) (.atom (.var "u")))

/-- What `body` does: it releases, and writes. -/
def bodyEff : Eff :=
  (Eff.ofFnR false HostFx.none false ["reveal"]).union
    ((⟨true, HostFx.none, Option.none, []⟩ : Eff).union .none)

theorem bot_le (e : LExpr) : LExpr.le prog.top LExpr.bot e :=
  .inr ⟨⟨Level.public_le _, fun _ h => by simp [LExpr.bot, LExpr.ofLabel, Label.bot] at h⟩,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h⟩

theorem body_typed : LExprTy prog [] Γ .bot body (.unit .bot) bodyEff := by
  refine .letR (.declassify (po := reveal) (fn := check) rfl rfl (.var rfl) (Flow.refl _) (bot_le _) ⟨rfl, rfl⟩)
    (Flow.refl _) ?_ (by simp [LTy.InPlace, LExpr.bot, LExpr.ofLabel])
  refine .letR (.op (.var rfl) trivial ?_ ⟨rfl, rfl⟩) (Flow.refl _) (.atom (.var rfl))
    (by simp [LTy.InPlace, LExpr.bot, LExpr.ofLabel])
  refine .inr ⟨⟨?_, ?_⟩, ?_, ?_⟩ <;>
    simp [Program.deep, LTy.deep, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot,
      Eff.failLabel, Eff.ofFnR, Eff.ofFn, prog, Level.join] <;> decide

/-- The runs' world: the host without state, since the programs perform no host operation. -/
def w0 : World Host.unit := ⟨(), ()⟩

/-- The environment with the secret `s`. -/
def env (n : Int) : Env := [("s", .int n)]

theorem sec_hidden : ¬ Vis prog Label.bot (LExpr.bot.join sec) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Program.top, sec, LExpr.ofLabel, LExpr.join, LExpr.bot, Label.bot, Label.join,
      Level.join] at h <;> revert h <;> decide

theorem intl_hidden : ¬ Vis prog Label.bot intl := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Program.top, intl, LExpr.ofLabel, Label.bot] at h <;> revert h <;> decide

theorem envs_typed (n : Int) : LEnvTy prog (env n) (Γ.lsub prog.recordDeep []) := .cons .int .nil

theorem envs_rel (a b : Int) : EnvRel prog Label.bot (Γ.lsub prog.recordDeep []) (env a) (env b) :=
  .cons (.hidden sec_hidden) .nil

theorem Γ_inPlace : Γ.InPlace := fun q hq => by
  simp only [Γ, List.mem_singleton] at hq
  subst hq
  rfl

/-- **Different releases: the runs diverge.** PIN 1234 releases `true`, PIN 1 releases `false`;
after the same (empty) prefix, the observer sees two different releases, the second case of
`t6_release_host`. -/
theorem pin_diverges :
    eval Host.unit 6 prog w0 (env 1234) body = ([.release "reveal" (.bool true), .out "true"], .ok .unit, w0) ∧
    eval Host.unit 6 prog w0 (env 1) body = ([.release "reveal" (.bool false), .out "false"], .ok .unit, w0) ∧
    Diverged prog Label.bot (eval Host.unit 6 prog w0 (env 1234) body).1 (eval Host.unit 6 prog w0 (env 1) body).1 := by
  have r₁ : eval Host.unit 6 prog w0 (env 1234) body =
      ([.release "reveal" (.bool true), .out "true"], .ok .unit, w0) := rfl
  have r₂ : eval Host.unit 6 prog w0 (env 1) body =
      ([.release "reveal" (.bool false), .out "false"], .ok .unit, w0) := rfl
  refine ⟨r₁, r₂, ?_⟩
  rw [r₁, r₂]
  have hrv : ∀ v, EvVis prog Label.bot (Event.release "reveal" v) :=
    fun _ => ⟨reveal, rfl, LExpr.bot_le _⟩
  have hout : ∀ l, EvVis prog Label.bot (Event.out l) := fun _ => LExpr.bot_le _
  refine ⟨[], "reveal", .bool true, .bool false, [.out "true"], [.out "false"], by simp, ?_, ?_⟩
  · rw [show [Event.release "reveal" (.bool true), Event.out "true"] =
      [Event.release "reveal" (.bool true)] ++ [Event.out "true"] from rfl, proj_append,
      proj_single_vis (hrv _), proj_single_vis (hout _)]
    rfl
  · rw [show [Event.release "reveal" (.bool false), Event.out "false"] =
      [Event.release "reveal" (.bool false)] ++ [Event.out "false"] from rfl, proj_append,
      proj_single_vis (hrv _), proj_single_vis (hout _)]
    rfl

/-- **A `via` that is not injective.** PINs 1 and 2 are different secrets, and both release
`false`: the releases agree, so the corollary gives `t6`'s full conclusion. The theorem tracks the
values released, not the secrets. -/
theorem guesses_agree :
    proj prog Label.bot (eval Host.unit 6 prog w0 (env 1) body).1 = proj prog Label.bot (eval Host.unit 6 prog w0 (env 2) body).1 ∧
      OutRel (V prog Label.bot ((LTy.unit LExpr.bot).lsubst prog.recordDeep []))
        (Vis prog Label.bot (bodyEff.inst (prog.sd [])).failLabel)
        (eval Host.unit 6 prog w0 (env 1) body).2.1 (eval Host.unit 6 prog w0 (env 2) body).2.1 := by
  have r₁ : eval Host.unit 6 prog w0 (env 1) body =
      ([.release "reveal" (.bool false), .out "false"], .ok .unit, w0) := rfl
  have r₂ : eval Host.unit 6 prog w0 (env 2) body =
      ([.release "reveal" (.bool false), .out "false"], .ok .unit, w0) := rfl
  have h := t6_release_host_agree prog Label.bot Host.unit prog_wf prog_b1 prog_host
    Host.unit_lawful (σ := []) (fun _ h => nomatch h) (fun _ h => by simp at h) Γ_inPlace rfl
    (WorldRel.refl w0) (envs_typed 1) (envs_typed 2) (envs_rel 1 2) body_typed 6
    (by rw [r₁]; trivial) (by rw [r₂]; trivial) (by rw [r₁, r₂])
  exact ⟨h.1, h.2.1⟩

/-- `let b: internal Bool = peek(s); b`: a release to `internal`. -/
def peekBody : Expr := .letR "b" (.bool intl) (.declassify "peek" (.var "s")) (.atom (.var "b"))

theorem peekBody_typed :
    LExprTy prog [] Γ .bot peekBody (.bool intl) ((Eff.ofFnR false HostFx.none false ["peek"]).union .none) :=
  .letR (.declassify (po := peek) (fn := check) rfl rfl (.var rfl) (Flow.refl _) (bot_le _) ⟨rfl, rfl⟩)
    (Flow.refl _) (.atom (.var rfl)) (by simp [LTy.InPlace, intl, LExpr.ofLabel])

/-- **A hidden release.** PIN 1234 and PIN 1 release different values to `internal`, which an
observer at `public` does not see: it sees no release, so the runs do not diverge, and they are
related as `t6_host` says, their values related at the hidden type. -/
theorem hidden_release_related :
    proj prog Label.bot (eval Host.unit 6 prog w0 (env 1234) peekBody).1 =
        proj prog Label.bot (eval Host.unit 6 prog w0 (env 1) peekBody).1 ∧
      OutRel (V prog Label.bot ((LTy.bool intl).lsubst prog.recordDeep []))
        (Vis prog Label.bot (((Eff.ofFnR false HostFx.none false ["peek"]).union .none).inst (prog.sd [])).failLabel)
        (eval Host.unit 6 prog w0 (env 1234) peekBody).2.1
        (eval Host.unit 6 prog w0 (env 1) peekBody).2.1 := by
  have r₁ : eval Host.unit 6 prog w0 (env 1234) peekBody = ([.release "peek" (.bool true)], .ok (.bool true), w0) := rfl
  have r₂ : eval Host.unit 6 prog w0 (env 1) peekBody = ([.release "peek" (.bool false)], .ok (.bool false), w0) := rfl
  have hpk : ∀ v, ¬ EvVis prog Label.bot (Event.release "peek" v) := fun _ ⟨po, hpo, hv⟩ => by
    cases hpo
    exact intl_hidden hv
  rcases t6_release_host prog Label.bot Host.unit prog_wf prog_b1 prog_host Host.unit_lawful
      (σ := []) (fun _ h => nomatch h) (fun _ h => by simp at h) Γ_inPlace rfl (WorldRel.refl w0)
      (envs_typed 1234) (envs_typed 1) (envs_rel 1234 1) peekBody_typed 6 (by rw [r₁]; trivial)
      (by rw [r₂]; trivial) with h | h
  · exact ⟨h.1, h.2.1⟩
  · exfalso
    obtain ⟨pre, _, _, _, _, _, _, h₁, _⟩ := h
    rw [r₁, proj_single_hidden (hpk _)] at h₁
    cases pre <;> simp at h₁

/-- `rev(s: secret Int) -> Bool = reveal(s)`: a function that declassifies. -/
def rev : Fun where
  name := "rev"
  tparams := []
  eqParams := []
  params := [("s", .int sec)]
  result := .bool .bot
  op := false
  host := HostFx.none
  fails := false
  body := .letR "b" (.bool .bot) (.declassify "reveal" (.var "s")) (.atom (.var "b"))
  rels := ["reveal"]

def prog2 : Program := { prog with funs := [check, rev] }

/-- `c: secret Bool, s: secret Int`. -/
def Γc : LCtx := [("c", .bool sec), ("s", .int sec)]

/-- `if c then rev(s) else rev(s)`: the release happens, or not, under a secret condition. -/
def underSecret : Expr :=
  .ite (.var "c")
    (.letR "x" (.bool .bot) (.call "rev" [] [.var "s"]) (.atom (.var "x")))
    (.letR "x" (.bool .bot) (.call "rev" [] [.var "s"]) (.atom (.var "x")))

theorem sec_not_le_bot {pc : LExpr} : ¬ prog2.le (pc.join sec) LExpr.bot := by
  intro h
  have h' := LExpr.le_of_join_right h
  rcases h' with ⟨hl, _⟩ | ⟨⟨hl, _⟩, _, _⟩ <;>
    simp [prog2, prog, Program.top, sec, LExpr.ofLabel, LExpr.bot, Label.bot] at hl <;>
    revert hl <;> decide

/-- **A call of a function that declassifies, under a secret condition, has no typing**: the
release would be visible in one run only. -/
theorem release_under_secret_untyped {E : List String} {pc : LExpr} {t : LTy} {ε : Eff} :
    ¬ LExprTy prog2 E Γc pc underSecret t ε := by
  intro h
  cases h with
  | ite hc ht _ _ _ =>
    cases hc with
    | var hx =>
      simp only [Γc, lctxLookup, ite_true] at hx
      cases hx
      cases ht with
      | letR hr _ _ _ =>
        cases hr with
        | call hf _ _ _ _ _ _ _ _ hrb =>
          simp only [prog2, Program.find, List.find?] at hf
          cases hf
          obtain ⟨po, hpo, hle, _⟩ := hrb "reveal" (by simp [rev])
          simp only [prog2, prog, Program.policy, List.find?] at hpo
          cases hpo
          exact sec_not_le_bot hle

end Una.F1.ReleaseExample
