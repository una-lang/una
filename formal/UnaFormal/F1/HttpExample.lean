/-
Non-vacuity of M5c (#470): a request of `Http` with its real signature (`spec/24 §1`). A program
certified by the whole-program check, as the re-check certifies one (`Import.mkProgram` with
`spec/24`'s `Header` and `Response`), sends `get(url, [], None)`. The host's server is a counter:
each response's status is the number of requests it has answered, so the server's state decides
the response. The host meets H1, H2 and H4 (`counter_lawful`), and `t6_host_ground` relates two
runs from server states an observer at `public` cannot tell apart, whose responses differ.
-/
import UnaFormal.F1.Import

namespace Una.F1.HttpExample

open Check

/-- A host with no file system and a server that counts the requests it has answered. -/
def counter : Host where
  FS := Unit
  Srv := Nat
  Shape := Unit
  shape := fun _ => ()
  read := fun _ _ => none
  write := fun _ _ _ => none
  request := fun _ _ n => (.ok (.record "prelude.Response" [.int n, .list [], .text ""]), n + 1)

/-- **The counter meets H1, H2 and H4**: its responses are `Response`s. -/
theorem counter_lawful : counter.Lawful where
  read_fails := fun _ _ => Iff.rfl
  write_shape := fun _ _ _ _ => rfl
  resp_typed := by
    intro m vs n s' v hq
    simp only [counter] at hq
    injection hq with hv _
    injection hv with hv
    subst hv
    simp [Val.IsResponse]

/-- `Http` accepts `internal`, and its response is `internal` (`spec/24 §1`). -/
def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def prog : Program :=
  Import.mkProgram [] Label.bot labels [Import.headerRec, Import.responseRec] [] []

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `let h = []; let k = None; let r = attempt http.get("https://example.com", h, k) in r`. -/
def body : Expr :=
  .letR "h" (.list (.record "prelude.Header" .bot) .bot) (.list [])
    (.letR "k" (.option (.text .bot) .bot) .none
      (.letE "r" (.result (.record "prelude.Response" intl) (.int .bot) intl)
        (.attempt (.letR "c" (.record "prelude.Response" intl)
          (.req "get" [.text "https://example.com", .var "h", .var "k"]) (.atom (.var "c"))))
        (.atom (.var "r"))))

unseal flowB flowsB in
/-- **The whole-program check certifies the program**, with `Http`'s kinds as `spec/24` fixes them. -/
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

unseal flowB flowsB in
theorem body_checked :
    ∃ ε, checkExpr prog [] [] .bot (.result (.record "prelude.Response" intl) (.int .bot) intl) body = .ok ε :=
  ⟨_, rfl⟩

/-- Two worlds whose servers have answered 0 and 5 requests. -/
def w₁ : World counter := ⟨(), (0 : Nat), fun _ => none⟩
def w₂ : World counter := ⟨(), (5 : Nat), fun _ => none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

/-- An observer at `public` sees no server state. -/
theorem worlds_rel : WorldRel prog Label.bot w₁ w₂ := ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun _ => rfl⟩

/-- **T6 with a request of `Http`** (M5c): the two runs, from servers in different states, show a
public observer the same events and related outcomes, and leave servers it cannot tell apart. -/
theorem request_runs : ∃ t ε, LExprTy prog [] [] .bot body t ε ∧ ∀ n : Nat,
    Term (eval counter n prog w₁ [] body).2.1 → Term (eval counter n prog w₂ [] body).2.1 →
    proj prog Label.bot (eval counter n prog w₁ [] body).1 =
        proj prog Label.bot (eval counter n prog w₂ [] body).1 ∧
      OutRel (V prog Label.bot t) (Vis prog Label.bot ε.failLabel)
        (eval counter n prog w₁ [] body).2.1 (eval counter n prog w₂ [] body).2.1 ∧
      WorldRel prog Label.bot (eval counter n prog w₁ [] body).2.2
        (eval counter n prog w₂ [] body).2.2 := by
  obtain ⟨hwf, _, hh⟩ := certified
  obtain ⟨ε, hc⟩ := body_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  refine ⟨t, ε, ht, fun n h₁ h₂ => ?_⟩
  exact t6_host_ground (Γ := []) hwf rfl hh counter_lawful (fun _ h => nomatch h) rfl worlds_rel
    .nil .nil .nil ht n h₁ h₂

/-- The runs end, send the request, and get different responses: statuses 0 and 5. -/
example : (eval counter 6 prog w₁ [] body).2.1 =
      .ok (.ok (.record "prelude.Response" [.int 0, .list [], .text ""])) ∧
    (eval counter 6 prog w₂ [] body).2.1 =
      .ok (.ok (.record "prelude.Response" [.int 5, .list [], .text ""])) := ⟨rfl, rfl⟩

end Una.F1.HttpExample
