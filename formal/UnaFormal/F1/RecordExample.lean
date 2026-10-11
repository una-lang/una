/-
Non-vacuity of #509: **a user record's fields carry their labels.** The re-check now reads a
document's record kinds (`Import.records`), and `to_text` of a record takes `ℓ*` of its type, the
labels its fields declare included (`Program.recordDeep`).

- **A record that holds a secret** (`card_rejected`, `card_leaks`, `card_untyped`): a program
  prints `to_text(Card(s))` at `Console`, which accepts `public`, where `Card`'s one field is an
  `internal Text` and `s` is read from the environment. The check rejects it, and its runs from
  environments where `SECRET` is `"1"` and `"2"` print different lines, so T6 refutes every
  typing of it.
- **A public record is certified** (`public_card_checked`, `public_card_runs`): `Tag("x")`, whose
  field is a `public Text`, is accepted, typed, and prints.
- **The relation hides an `internal` field** (`card_rel_hidden`): two `Card` values whose fields
  differ are related for a `public` observer, while their texts differ.
-/
import UnaFormal.F1.Import

namespace Una.F1.RecordExample

open Check

def labels : Import.HostLabels :=
  { acceptsReq := ⟨.internal, []⟩, reqResult := ⟨.internal, []⟩, fileState := some Label.bot }

def intl : LExpr := .ofLabel ⟨.internal, []⟩

/-- `Card` holds an `internal Text`; `Tag` a `public` one. -/
def recs : List Record :=
  [⟨"main.Card", [.text intl], ⟨.internal, []⟩⟩, ⟨"main.Tag", [.text .bot], Label.bot⟩]

/-- `Console` accepts `public`. -/
def prog : Program := Import.mkProgram [] Label.bot labels recs [] []

/-- A world on the stateless host whose environment sets `SECRET` to `v`. -/
def wSet (v : String) : World Host.unit := ⟨(), (), fun x => if x = "SECRET" then some v else none⟩

theorem internal_hidden : ¬ Vis prog Label.bot (.ofLabel internalLabel) := by
  rintro (⟨h, _⟩ | ⟨⟨h, _⟩, _, _⟩) <;>
    simp [prog, Import.mkProgram, Program.top, internalLabel, LExpr.ofLabel, Label.bot] at h <;>
    revert h <;> decide

/-- Two environments that differ in `SECRET`'s value are related for a `public` observer. -/
theorem set_rel (a b : String) : WorldRel prog Label.bot (wSet a) (wSet b) :=
  ⟨fun _ => rfl, fun h => absurd h internal_hidden, fun x => by simp only [wSet]; split <;> rfl⟩

/-- `print(to_text(r))` at `Console`. -/
def printR (r : String) : Expr :=
  .letR "m" (.text .bot) (.toText (.var r))
    (.letR "u" (.unit .bot) (.op (.var "m")) (.atom (.var "u")))

/-- `when Env.get("SECRET") is None -> () is Some(s) -> k`, with `s` an `internal Text`. -/
def withSecret (k : Expr) : Expr :=
  .letR "e" (.option (.text intl) .bot) (.envGet (.text "SECRET"))
    (.caseOption (.var "e") (.atom .unit) "s" k)

unseal flowB flowsB in
theorem certified : prog.LWellFormed ∧ prog.B1WellFormed ∧ prog.HostWF :=
  progOk_sound_nil (by rfl)

theorem out_vis (s : String) : EvVis prog Label.bot (.out s) := LExpr.le_refl _

/-- T6 over `set_rel`: a typed program's two runs show a `public` observer the same lines. -/
theorem same_lines {e : Expr} {t : LTy} {ε : Eff} (ht : LExprTy prog [] [] .bot e t ε) (n : Nat)
    (h₁ : Term (eval Host.unit n prog (wSet "1") [] e).2.1)
    (h₂ : Term (eval Host.unit n prog (wSet "2") [] e).2.1) :
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] e).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "2") [] e).1 := by
  obtain ⟨hwf, _, hh⟩ := certified
  exact (t6_host_ground (Γ := []) hwf rfl hh Host.unit_lawful (fun _ h => nomatch h) rfl
    (set_rel "1" "2") .nil .nil .nil ht n h₁ h₂).1

/-! ### A record that holds a secret -/

/-- `let c: Card = Card(s)` and print it. -/
def cardLeak : Expr :=
  withSecret (.letR "c" (.record "main.Card" .bot) (.construct "main.Card" [.var "s"]) (printR "c"))

unseal flowB flowsB in
theorem card_rejected : ∃ msg, checkExpr prog [] [] .bot (.unit .bot) cardLeak = .error msg :=
  ⟨_, rfl⟩

theorem card_leaks : (eval Host.unit 10 prog (wSet "1") [] cardLeak).1 = [.out "main.Card(1, )"] ∧
    (eval Host.unit 10 prog (wSet "2") [] cardLeak).1 = [.out "main.Card(2, )"] := ⟨rfl, rfl⟩

/-- **The leak has no typing.** -/
theorem card_untyped {t : LTy} {ε : Eff} : ¬ LExprTy prog [] [] .bot cardLeak t ε := by
  intro ht
  have h := same_lines ht 10 (by exact True.intro) (by exact True.intro)
  rw [card_leaks.1, card_leaks.2, proj_single_vis (out_vis _), proj_single_vis (out_vis _)] at h
  simp at h

/-! ### A public record -/

/-- `let t: Tag = Tag("x")` and print it, after reading the secret. -/
def publicCard : Expr :=
  withSecret (.letR "c" (.record "main.Tag" .bot) (.construct "main.Tag" [.text "x"]) (printR "c"))

unseal flowB flowsB in
theorem public_card_checked :
    ∃ ε, checkExpr prog [] [] .bot (.unit .bot) publicCard = .ok ε := ⟨_, rfl⟩

/-- **A public record is certified**, and it runs and prints. -/
theorem public_card_runs : (∃ t ε, LExprTy prog [] [] .bot publicCard t ε ∧ ∀ n : Nat,
    Term (eval Host.unit n prog (wSet "1") [] publicCard).2.1 →
    Term (eval Host.unit n prog (wSet "2") [] publicCard).2.1 →
    proj prog Label.bot (eval Host.unit n prog (wSet "1") [] publicCard).1 =
      proj prog Label.bot (eval Host.unit n prog (wSet "2") [] publicCard).1) ∧
    (eval Host.unit 10 prog (wSet "1") [] publicCard).1 = [.out "main.Tag(x, )"] := by
  obtain ⟨ε, hc⟩ := public_card_checked
  obtain ⟨t, ht, _⟩ := checkExpr_sound hc
  exact ⟨⟨t, ε, ht, same_lines ht⟩, rfl⟩

/-! ### The relation -/

/-- **An `internal` field is hidden**: two `Card` values that differ are related for a `public`
observer, while their texts differ. -/
theorem card_rel_hidden :
    V prog Label.bot (.record "main.Card" .bot) (.record "main.Card" [.text "1"])
        (.record "main.Card" [.text "2"]) ∧
      (Val.record "main.Card" [.text "1"]).render ≠ (Val.record "main.Card" [.text "2"]).render :=
  ⟨.record rfl (.cons (.hidden (fun h => internal_hidden (by
      simpa [LTy.top, intl, internalLabel] using h))) .nil), by decide⟩

end Una.F1.RecordExample
