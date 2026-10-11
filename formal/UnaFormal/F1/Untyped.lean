/-
M5, part 2 (`docs/plan/formal-track-m5.md` §6): **ADR 0085's rules reject S-14's and S-15's
probes.** Under the new rules (`oldHost = false`), in any program whose `Console` accepts
`public`, with any file-state label and every annotation of the probe, the probe has no typing,
when its secret is not `public`:

- **S-14** (`s14_untyped`): the write to the secret path makes the file-state label at least the
  path's label, every read's failure carries it, so the branch on the read's result runs above
  `public`, where `Console` cannot be written.
- **S-15** (`s15_untyped`): the request's failure carries its body's label, so the branch on its
  result runs above `public`.
-/
import UnaFormal.F1.LTyping
import UnaFormal.F1.Order
import UnaFormal.F1.Subst

namespace Una.F1.Untyped

variable {P : Program}

/-- S-14's probe with every annotation given: write to the secret path, read `a.txt`, and print
whether it exists. -/
def s14 (aw au ar ac a₁ a₂ : LTy) : Expr :=
  .letE "w" aw (.attempt (.letR "u" au (.write (.var "x") (.text "x")) (.atom (.var "u"))))
    (.letE "r" ar (.attempt (.letR "c" ac (.read (.text "a.txt")) (.atom (.var "c"))))
      (.caseResult (.var "r") "v"
        (.letR "o" a₁ (.op (.text "yes")) (.atom (.var "o"))) "e"
        (.letR "o" a₂ (.op (.text "no")) (.atom (.var "o")))))

/-- S-15's probe with every annotation given: send the secret as a request's body, with no
header and no key name (M5c: `post(url, headers, body, key_name)`), and print whether it failed. -/
def s15 (ar ah ak ac a₁ a₂ : LTy) : Expr :=
  .letE "r" ar (.attempt (.letR "h" ah (.list []) (.letR "k" ak .none
      (.letR "c" ac (.req "post" [.text "https://example.com", .var "h", .var "x", .var "k"])
        (.atom (.var "c"))))))
    (.caseResult (.var "r") "v"
      (.letR "o" a₁ (.op (.text "yes")) (.atom (.var "o"))) "e"
      (.letR "o" a₂ (.op (.text "no")) (.atom (.var "o"))))

/-- A branch on `r` whose arms print: what decides it is below what `Console` accepts. -/
theorem branch_bound {Γ : LCtx} {pc : LExpr} {a₁ a₂ s : LTy} {ε : Eff} {E : List String}
    (h : LExprTy P E Γ pc (.caseResult (.var "r") "v"
      (.letR "o" a₁ (.op (.text "yes")) (.atom (.var "o"))) "e"
      (.letR "o" a₂ (.op (.text "no")) (.atom (.var "o")))) s ε) :
    ∃ te ee ls, lctxLookup Γ "r" = some (.result te ee ls) ∧ P.le ls (.ofLabel P.acceptsOp) := by
  cases h with
  | caseResult hs hok _ _ _ _ =>
    cases hs with
    | var hl =>
      refine ⟨_, _, _, hl, ?_⟩
      cases hok with
      | letR hv _ _ _ =>
        cases hv with
        | op _ _ hle _ =>
          exact LExpr.le_trans (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _)) hle
  | caseResultT hs hok _ _ _ _ _ =>
    cases hs with
    | var hl =>
      refine ⟨_, _, _, hl, ?_⟩
      cases hok with
      | letR hv _ _ _ =>
        cases hv with
        | op _ _ hle _ =>
          exact LExpr.le_trans (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _)) hle

/-- `attempt` over a `let` whose body is the bound atom: its result is labeled by what decides
the value's failure. -/
theorem attempt_inv {Γ : LCtx} {pc : LExpr} {E : List String} {x : String} {ann : LTy} {v : Rhs}
    {t : LTy} {ε : Eff} (h : LExprTy P E Γ pc (.attempt (.letR x ann v (.atom (.var x)))) t ε) :
    ∃ tc t' ε', LRhsTy P E Γ pc v t' ε' ∧ t = .result tc (.int .bot) (ε'.union .none).dl := by
  cases h with
  | attempt hb _ =>
    cases hb with
    | letR hr _ hbody _ =>
      cases hbody with
      | atom _ => exact ⟨_, _, _, hr, rfl⟩

theorem dl_some {o : Bool} {g : HostFx} {l : LExpr} {rs : List String} {cd : Bool}
    {ra : List String} {d : LExpr} : ((⟨o, g, some l, rs, cd, ra, d⟩ : Eff).union .none).dl = d.join .bot := by
  simp [Eff.union, Eff.none]

/-- **S-14 under ADR 0085's rules has no typing**, for every file-state label and every
annotation. -/
theorem s14_untyped (hold : P.oldHost = false) {lx : LExpr} (hx : ¬ P.le lx (.ofLabel P.acceptsOp))
    (aw au ar ac a₁ a₂ : LTy) {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy P E [("x", .text lx)] .bot (s14 aw au ar ac a₁ a₂) t ε := by
  intro h
  cases h with
  | letE hw _ hbody _ =>
    -- The write: the secret path is below the file-state label.
    have hfs : P.le lx (.ofLabel P.fileState) := by
      cases hw with
      | attempt hb =>
        cases hb with
        | letR hr _ _ _ =>
          cases hr with
          | write hp _ _ _ _ hstate _ =>
            cases hp with
            | var hl =>
              simp [lctxLookup] at hl
              subst hl
              exact LExpr.le_trans (LExpr.le_join_right _ _) (hstate hold)
    cases hbody with
    | letE hr hflow hb _ =>
      -- The read: its failure carries the file-state label, and so does `r`.
      obtain ⟨tc, t', ε', hrd, rfl⟩ := attempt_inv hr
      obtain ⟨te, ee, ls, hl, hop⟩ := branch_bound hb
      simp [lctxLookup] at hl
      subst hl
      cases hrd with
      | read hp _ _ _ =>
        cases hp
        rw [dl_some] at hflow
        cases hflow with
        | result _ _ hls =>
          apply hx
          refine LExpr.le_trans hfs (LExpr.le_trans ?_ (LExpr.le_trans hls hop))
          refine LExpr.le_trans ?_ (LExpr.le_join_left _ _)
          simp only [Program.fsFail, hold, Bool.false_eq_true, ite_false]
          exact LExpr.le_join_right _ _

/-- **S-15 under ADR 0085's rules has no typing**, for every annotation. -/
theorem s15_untyped (hold : P.oldHost = false) {lx : LExpr} (hx : ¬ P.le lx (.ofLabel P.acceptsOp))
    (ar ah ak ac a₁ a₂ : LTy) {E : List String} {t : LTy} {ε : Eff} :
    ¬ LExprTy P E [("x", .text lx)] .bot (s15 ar ah ak ac a₁ a₂) t ε := by
  intro h
  cases h with
  | letE hr hflow hb _ =>
    obtain ⟨te, ee, ls, hl, hop⟩ := branch_bound hb
    simp [lctxLookup] at hl
    subst hl
    -- The request, under the empty headers and no key name: its failure carries the body's label.
    cases hr with
    | attempt hb₁ =>
      cases hb₁ with
      | letR hrh _ hb₂ _ =>
        cases hb₂ with
        | letR hrk _ hb₃ _ =>
          cases hb₃ with
          | letR hrq _ hb₄ _ =>
            cases hb₄ with
            | atom _ =>
              cases hrh
              cases hrk
              cases hrq with
              | @req _ _ _ _ ts hts _ _ _ _ =>
                cases hts with
                | cons _ hts =>
                  cases hts with
                  | cons _ hts =>
                    cases hts with
                    | cons h₃ hts =>
                      cases h₃ with
                      | var hlx =>
                        simp [lctxLookup] at hlx
                        subst hlx
                        simp only [Eff.union, Eff.none, Eff.failLabel] at hflow
                        cases hflow with
                        | result _ _ hls =>
                          apply hx
                          refine LExpr.le_trans ?_ (LExpr.le_trans hls hop)
                          refine LExpr.le_trans ?_ (LExpr.le_join_right _ _)
                          refine LExpr.le_trans ?_ (LExpr.le_join_right _ _)
                          refine LExpr.le_trans ?_ (LExpr.le_join_left _ _)
                          simp only [Program.reqFail, hold, Bool.false_eq_true, ite_false]
                          refine LExpr.le_trans ?_ (LExpr.le_join_left _ _)
                          refine LExpr.le_trans ?_ (LExpr.le_join_right _ _)
                          exact LExpr.le_joinAll (List.mem_map.mpr ⟨LTy.text lx, by simp, rfl⟩)

/-! ### More of ADR 0085's conformance shapes -/

/-- `s14_read_before_write`: the read comes first, the write after. The file-state label is
static, so the read's failure carries it all the same. -/
def s14rbw (ar ac aw au a₁ a₂ : LTy) : Expr :=
  .letE "r" ar (.attempt (.letR "c" ac (.read (.text "a.txt")) (.atom (.var "c"))))
    (.letE "w" aw (.attempt (.letR "u" au (.write (.var "x") (.text "x")) (.atom (.var "u"))))
      (.caseResult (.var "r") "v"
        (.letR "o" a₁ (.op (.text "yes")) (.atom (.var "o"))) "e"
        (.letR "o" a₂ (.op (.text "no")) (.atom (.var "o")))))

/-- A write's path, under the new rules, is below the file-state label. -/
theorem write_state (hold : P.oldHost = false) {Γ : LCtx} {pc : LExpr} {E : List String}
    {ann : LTy} {path content : Atom} {t : LTy} {ε : Eff}
    (h : LExprTy P E Γ pc (.attempt (.letR "u" ann (.write path content) (.atom (.var "u")))) t ε) :
    ∃ tp, LAtomTy Γ path tp ∧ P.le (pc.join (P.deep tp)) (.ofLabel P.fileState) := by
  obtain ⟨_, _, _, hw, _⟩ := attempt_inv h
  cases hw with
  | write hp _ _ _ _ hstate _ => exact ⟨_, hp, hstate hold⟩

/-- **The read before the write has no typing either.** -/
theorem s14rbw_untyped (hold : P.oldHost = false) {lx : LExpr}
    (hx : ¬ P.le lx (.ofLabel P.acceptsOp)) (ar ac aw au a₁ a₂ : LTy) {E : List String}
    {t : LTy} {ε : Eff} :
    ¬ LExprTy P E [("x", .text lx)] .bot (s14rbw ar ac aw au a₁ a₂) t ε := by
  intro h
  cases h with
  | letE hr hflow hbody _ =>
    obtain ⟨tc, t', ε', hrd, rfl⟩ := attempt_inv hr
    cases hbody with
    | letE hw _ hb _ =>
      obtain ⟨tp, hp, hfs⟩ := write_state hold hw
      cases hp with
      | var hl =>
        simp [lctxLookup] at hl
        subst hl
        obtain ⟨te, ee, ls, hl, hop⟩ := branch_bound hb
        simp [lctxLookup] at hl
        subst hl
        cases hrd with
        | read hpath _ _ _ =>
          cases hpath
          rw [dl_some] at hflow
          cases hflow with
          | result _ _ hls =>
            apply hx
            refine LExpr.le_trans (LExpr.le_trans (LExpr.le_join_right _ _) hfs)
              (LExpr.le_trans ?_ (LExpr.le_trans hls hop))
            refine LExpr.le_trans ?_ (LExpr.le_join_left _ _)
            simp only [Program.fsFail, hold, Bool.false_eq_true, ite_false]
            exact LExpr.le_join_right _ _

/-- `s14_write_under_condition`: the write's path is public, but it runs only when the secret
`c` holds. The program counter at the write is below the file-state label (ADR 0085 decision 1). -/
def s14cond (aw au au' ar ac a₁ a₂ : LTy) : Expr :=
  .letE "w" aw
    (.ite (.var "c")
      (.attempt (.letR "u" au (.write (.text "b.txt") (.text "x")) (.atom (.var "u"))))
      (.attempt (.letR "u" au' (.write (.text "c.txt") (.text "x")) (.atom (.var "u")))))
    (.letE "r" ar (.attempt (.letR "c" ac (.read (.text "a.txt")) (.atom (.var "c"))))
      (.caseResult (.var "r") "v"
        (.letR "o" a₁ (.op (.text "yes")) (.atom (.var "o"))) "e"
        (.letR "o" a₂ (.op (.text "no")) (.atom (.var "o")))))

/-- **The write under a secret condition has no typing.** -/
theorem s14cond_untyped (hold : P.oldHost = false) {lc : LExpr}
    (hc : ¬ P.le lc (.ofLabel P.acceptsOp)) (aw au au' ar ac a₁ a₂ : LTy) {E : List String}
    {t : LTy} {ε : Eff} :
    ¬ LExprTy P E [("c", .bool lc)] .bot (s14cond aw au au' ar ac a₁ a₂) t ε := by
  intro h
  cases h with
  | letE hw _ hbody _ =>
    have hfs : P.le lc (.ofLabel P.fileState) := by
      cases hw with
      | ite hcond hthen _ _ _ _ =>
        cases hcond with
        | var hl =>
          simp [lctxLookup] at hl
          subst hl
          obtain ⟨_, _, hfs⟩ := write_state hold hthen
          exact LExpr.le_trans (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _)) hfs
      | iteT hcond hthen _ _ _ _ _ =>
        cases hcond with
        | var hl =>
          simp [lctxLookup] at hl
          subst hl
          obtain ⟨_, _, hfs⟩ := write_state hold hthen
          exact LExpr.le_trans (LExpr.le_trans (LExpr.le_join_right _ _) (LExpr.le_join_left _ _)) hfs
    cases hbody with
    | letE hr hflow hb _ =>
      obtain ⟨tc, t', ε', hrd, rfl⟩ := attempt_inv hr
      obtain ⟨te, ee, ls, hl, hop⟩ := branch_bound hb
      simp [lctxLookup] at hl
      subst hl
      cases hrd with
      | read hp _ _ _ =>
        cases hp
        rw [dl_some] at hflow
        cases hflow with
        | result _ _ hls =>
          apply hc
          refine LExpr.le_trans hfs (LExpr.le_trans ?_ (LExpr.le_trans hls hop))
          refine LExpr.le_trans ?_ (LExpr.le_join_left _ _)
          simp only [Program.fsFail, hold, Bool.false_eq_true, ite_false]
          exact LExpr.le_join_right _ _

end Una.F1.Untyped
