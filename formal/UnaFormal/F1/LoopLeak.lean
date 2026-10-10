/-
M6, a finding of the proof (`docs/plan/formal-track-m6.md`): **an iteration's failure decides
whether the later iterations run**, so T-Loop bounds it by what decides whether the iteration runs.
M6's draft rule (`Program.oldLoop`) typed the body under the list's top label only. Under it, the
probe below has a typing, and two runs from environments a `public` observer cannot tell apart
print different numbers of lines at `Console`, which accepts `public`: the first iteration's
`internal`-decided failure stops the second iteration's print. Under the rule as fixed, the probe
has no typing. The surface language rejects the shape already: a failure a `for` body passes on
must be public (U4002, `una check` 0.3.5).
-/
import UnaFormal.F1.BinaryLemmas
import UnaFormal.F1.CheckSoundCore

namespace Una.F1.LoopLeak

/-- `internal`. -/
def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- A program whose `Console` accepts `public`, under M6's draft rule for loops. -/
def draftProg : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := Label.bot
  acceptsWrite := ⟨.internal, []⟩
  acceptsReq := ⟨.internal, []⟩
  readResult := ⟨.internal, []⟩
  reqResult := ⟨.internal, []⟩
  reqFloor := ⟨.internal, []⟩
  fileState := Label.bot
  oldLoop := true
  records := []
  policies := []
  funs := []

/-- The same program under the rule as fixed. -/
def fixedProg : Program := { draftProg with oldLoop := false }

/-- A `public` list of `internal` texts. -/
def Γ : LCtx := [("xs", .list (.text intl) .bot)]

/-- One iteration: print `hello`, then parse the element, which fails when it spells no integer. -/
def iter : Expr :=
  .letR "o" (.unit .bot) (.op (.text "hello"))
    (.letR "n" (.int intl) (.parse (.var "x"))
      (.letR "t" (.tuple [] .bot) (.tuple []) (.atom (.var "t"))))

/-- **The probe**: `for x in xs`, with no state. -/
def probe : Expr := .loop (.list (.var "xs")) "x" (.text intl) [] [] iter

unseal Check.flowB Check.flowsB in
/-- Under the draft rule, the probe has a typing: the checker accepts it. -/
theorem draft_typed : ∃ t ε, LExprTy draftProg [] Γ .bot probe t ε := by
  obtain ⟨t, ht, _⟩ := Check.checkExpr_sound (p := draftProg) (E := []) (Γ := Γ) (pc := .bot)
    (s := .tuple [] .bot) (e := probe) (ε := _) rfl
  exact ⟨t, _, ht⟩

/-- The two lists: the first element of the first spells no integer. -/
def env (s : String) : Env := [("xs", .list [.text s, .text "1"])]

theorem intl_hidden (P : Program) (hP : P.categories = []) : ¬ Vis P Label.bot intl := by
  rintro (h | ⟨h, _⟩) <;>
    simp [Program.top, hP, intl, LExpr.ofLabel, Label.le, Label.bot] at h <;> revert h <;> decide

/-- The environments are related for a `public` observer: they differ only in `internal` texts. -/
theorem envs_related : EnvRel draftProg Label.bot Γ (env "x") (env "1") :=
  .cons (.list (.cons (.hidden (intl_hidden _ rfl)) (.cons (.hidden (intl_hidden _ rfl)) .nil))) .nil

/-- **The draft rule leaks**: the two runs end, and the `public` observer sees one `hello` in the
first and two in the second. -/
theorem draft_leaks :
    (eval Host.unit 20 draftProg ⟨(), ()⟩ (env "x") probe).2.1 = .failed hostFailure ∧
    (eval Host.unit 20 draftProg ⟨(), ()⟩ (env "1") probe).2.1 = .ok (.tuple []) ∧
    proj draftProg Label.bot (eval Host.unit 20 draftProg ⟨(), ()⟩ (env "x") probe).1 = [.out "hello"] ∧
    proj draftProg Label.bot (eval Host.unit 20 draftProg ⟨(), ()⟩ (env "1") probe).1 =
      [.out "hello", .out "hello"] := by
  refine ⟨rfl, rfl, ?_, ?_⟩ <;>
    simp [eval, evalRhs, sourceVals, evalAtom, lookup, walkLoop, andThen, bindParams, parseInt,
      done, proj, EvVis, Vis, Program.le, draftProg, LExpr.le_refl]

/-- **Under the rule as fixed, the probe has no typing.** -/
theorem fixed_untyped {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy fixedProg E Γ .bot probe t ε := by
  intro h
  cases h with
  | loopList hxs _ _ _ _ _ hb _ hfe =>
    cases hxs with
    | var hl =>
      simp [Γ, lctxLookup] at hl
      obtain ⟨rfl, rfl⟩ := hl
      have hfe := hfe rfl
      cases hb with
      | letR _ _ hb₂ _ =>
        cases hb₂ with
        | letR hp _ _ _ =>
          cases hp with
          | parse hx _ =>
            cases hx with
            | var hl =>
              simp [lctxLookup] at hl
              subst hl
              apply intl_hidden fixedProg rfl
              refine LExpr.le_trans ?_ (LExpr.le_trans hfe ?_)
              · refine LExpr.le_trans ?_ (failLabel_union_right _ _)
                refine LExpr.le_trans ?_ (failLabel_union_left _ _)
                simp only [Eff.failLabel, Option.getD_some]
                exact LExpr.le_trans (LExpr.le_join_left _ _) (LExpr.le_join_right _ _)
              · exact LExpr.join_le (LExpr.le_refl _) (LExpr.le_refl _)

end Una.F1.LoopLeak
