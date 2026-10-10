/-
Examples that the definitions of F1 are not vacuous: a generic function the labeled typing
accepts, whose result carries `ℓ_T` in place, a program it accepts, and T1 applied to a typed
call of the function and to its run.
-/
import UnaFormal.F1.Erase
import UnaFormal.F1.InPlace

namespace Una.F1.Examples

/-- `ℓ_T`. -/
def lT : LExpr := ⟨Label.bot, ["T"], []⟩

/-- `pick[T](a: T, b: T, c: Bool) -> T = if c then a else b`. -/
def pick : Fun where
  name := "pick"
  tparams := ["T"]
  eqParams := []
  params := [("a", .var "T" lT), ("b", .var "T" lT), ("c", .bool .bot)]
  result := .var "T" lT
  op := false
  host := HostFx.none
  fails := false
  body := .ite (.var "c") (.atom (.var "a")) (.atom (.var "b"))

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
  policies := []
  funs := [pick]

theorem le_self {top : Label} (e : LExpr) : LExpr.le top e e :=
  .inr ⟨Label.le_refl _, fun _ hx => .inl hx, fun _ hx => hx⟩

theorem bot_le (e : LExpr) : LExpr.le prog.top LExpr.bot e :=
  .inr ⟨⟨Level.public_le _, fun _ h => by simp [LExpr.bot, LExpr.ofLabel, Label.bot] at h⟩,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h,
    fun _ h => by simp [LExpr.bot, LExpr.ofLabel] at h⟩

theorem lT_join_le : LExpr.le prog.top (lT.join LExpr.bot) lT :=
  .inr ⟨⟨Nat.le_refl _, fun _ hx => by
      simp [lT, LExpr.join, LExpr.bot, LExpr.ofLabel, Label.bot, Label.join] at hx⟩,
    fun x hx => .inl (by simpa [lT, LExpr.join, LExpr.bot, LExpr.ofLabel] using hx),
    fun _ hx => by simp [lT, LExpr.join, LExpr.bot, LExpr.ofLabel] at hx⟩

/-- The labeled typing accepts `pick`: its result is `T` with `ℓ_T`, which a call replaces by
the type argument's own labels. -/
theorem pick_ok : prog.FunOk pick := by
  refine ⟨.var "T" (lT.join .bot), ⟨false || false, HostFx.none.or HostFx.none, Option.none, [] ++ []⟩,
    ?_, .var lT_join_le, by simp, ⟨id, id, id⟩, by simp, ?_, by simp⟩
  · have hthen : LExprTy prog [] [("a", .var "T" lT), ("b", .var "T" lT), ("c", .bool .bot)]
        (LExpr.bot.join .bot) (.atom (.var "a")) (.var "T" lT) .none :=
      .atom (.var rfl)
    have helse : LExprTy prog [] [("a", .var "T" lT), ("b", .var "T" lT), ("c", .bool .bot)]
        (LExpr.bot.join .bot) (.atom (.var "b")) (.var "T" lT) .none :=
      .atom (.var rfl)
    exact .ite (.var rfl) hthen helse (.var (le_self lT)) (.var (le_self lT)) (by simp [LTy.InPlace, lT])
  · exact le_self _

/-- The labeled typing accepts the program. -/
theorem prog_wf : prog.LWellFormed where
  funs fn h := by
    simp only [prog, List.mem_singleton] at h
    subst h
    exact .inl pick_ok
  records rec h := by simp [prog] at h
  recordDeep rec h := by simp [prog] at h
  policies po h := by simp [prog] at h
  categories := by
    refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩ <;>
      simp [prog, pick, lT, Label.bot, LTy.CatsIn, LExpr.CatsIn, LExpr.bot, LExpr.ofLabel]
    rintro _ _ (⟨_, rfl⟩ | ⟨_, rfl⟩ | ⟨_, rfl⟩) <;> simp [LTy.CatsIn, LExpr.CatsIn]
  recordsGround _ h := by simp [prog] at h
  eqScoped := by simp [prog, pick]
  sigsScoped := by
    simp only [prog, List.mem_singleton]
    rintro _ rfl
    refine ⟨?_, ?_⟩
    · simp only [pick, List.mem_cons, List.not_mem_nil, or_false]
      rintro _ (rfl | rfl | rfl) <;>
        simp [LTy.ScopedIn, LExpr.SymsIn, lT, LExpr.bot, LExpr.ofLabel]
    · simp [pick, LTy.ScopedIn, LExpr.SymsIn, lT]
  inPlace := by
    refine ⟨?_, ?_, ?_⟩ <;> simp [prog, pick, lT, LTy.InPlace, LExpr.bot, LExpr.ofLabel]
    rintro _ _ (⟨_, rfl⟩ | ⟨_, rfl⟩ | ⟨_, rfl⟩) <;> simp [LTy.InPlace]

/-- The type of `pick[Int](...)`: `Int`, with `ℓ_T` replaced in place by the argument's labels. -/
def pickInt : LTy := prog.lsub [("T", .int .bot)] pick.result

/-- `let x: Int = pick[Int](1, 2, true) in x`. -/
def callPick : Expr :=
  .letR "x" pickInt (.call "pick" [.int .bot] [.int 1, .int 2, .bool true]) (.atom (.var "x"))

/-- The labeled typing accepts the call at the public program counter. -/
theorem callPick_typed :
    LExprTy prog [] [] .bot callPick pickInt ((Eff.ofFn false HostFx.none false).union .none) := by
  refine .letR (t := pickInt) ?_ (.int (le_self _)) (.atom (.var rfl)) ?ip
  case ip => simp [pickInt, Program.lsub, LTy.lsubst, pick, lT, LTy.InPlace, LTy.raise, LExpr.join, LExpr.inst, LExpr.dropPlain, LExpr.joinAll, LExpr.bot, LExpr.ofLabel]
  refine LRhsTy.call (fn := pick) (targs := [.int .bot]) rfl ?_ rfl rfl ?_
    (by simp [LTy.GroundAll, LTy.Ground, LExpr.SymFree, LExpr.bot, LExpr.ofLabel]) (.cons .int (.cons .int (.cons .bool .nil)))
    (.cons (.int (bot_le _)) (.cons (.int (bot_le _)) (.cons (.bool (bot_le _)) .nil)))
    ⟨fun h => (by cases h), fun h => (by cases h), fun h => (by cases h), fun h => (by cases h),
      fun h => (by cases h)⟩ (fun _ h => nomatch h)
  · intro ⟨_, h, _⟩
    simp [prog] at h
  · intro q _ h
    simp [pick] at h

/-- **T1, applied:** the run of the typed call never gets stuck, and its value is an integer. -/
theorem callPick_safe {H : Host} (w : World H) (n : Nat) :
    (eval H n prog w [] callPick).2.1 ≠ .stuck ∧
      ∀ v, (eval H n prog w [] callPick).2.1 = .ok v → ValTy prog v pickInt.erase :=
  t1_labeled prog_wf w .nil callPick_typed n

/-- A run of `pick[Int](1, 2, true)` gives `1`. -/
example :
    (eval Host.unit 10 prog ⟨(), ()⟩ []
      (.letR "x" (.int .bot) (.call "pick" [.int .bot] [.int 1, .int 2, .bool true])
        (.atom (.var "x")))).2.1 = .ok (.int 1) := by
  rfl

/-! ### A decision given to a function value -/

/-- `ℓ*_T`. -/
def lsT : LExpr := ⟨Label.bot, [], ["T"]⟩

def tT : LTy := .var "T" lT

/-- The label of `a == b` over `T` values: every label of both, `ℓ*_T`. -/
def cLabel : LExpr := (tT.deep (fun _ => Label.bot)).join (tT.deep (fun _ => Label.bot))

def xTy : LTy := tT.raise cLabel

def rTy : LTy := (LTy.int .bot).raise (LExpr.bot.join (decision (decisionsOf [xTy])))

/-- `choose[T: Eq](a: T, b: T, k: fn(T) -> Int) -> Int = let c = a == b; let x = if c then a
else b; k(x)`: `x` carries the decision `ℓ*_T`, which `k`'s parameter `T` does not accept, so
T-App raises the call's result by it. -/
def choose : Fun where
  name := "choose"
  tparams := ["T"]
  eqParams := ["T"]
  params := [("a", tT), ("b", tT), ("k", .fn [tT] (.int .bot) false HostFx.none false .bot)]
  result := .int lsT
  op := false
  host := HostFx.none
  fails := false
  body :=
    .letR "c" (.bool cLabel) (.eq (.var "a") (.var "b"))
      (.letE "x" xTy (.ite (.var "c") (.atom (.var "a")) (.atom (.var "b")))
        (.letR "r" rTy (.apply (.var "k") [.var "x"]) (.atom (.var "r"))))

def prog2 : Program := { prog with funs := [choose] }

/-- The decision `x` carries is `ℓ*_T`, computed from the comparison. -/
theorem choose_decision : decisionsOf [xTy] = ["T"] := by decide

theorem choose_le (a b : LExpr) (hb : a.base.level = .«public» ∧ a.base.cats = [])
    (hp : ∀ x ∈ a.plain, x ∈ b.plain ∨ x ∈ b.star) (hs : ∀ x ∈ a.star, x ∈ b.star) :
    LExpr.le prog2.top a b :=
  .inr ⟨⟨by rw [hb.1]; exact Level.public_le _, fun _ h => by rw [hb.2] at h; nomatch h⟩, hp, hs⟩

/-- What `choose`'s body may do: nothing. -/
def chooseEff : Eff :=
  Eff.none.union ((Eff.none.union Eff.none).union ((Eff.ofFn false HostFx.none false).union Eff.none))

/-- The labeled typing accepts `choose`, `Eq` constraint, comparison, decision and all. -/
theorem choose_ok : prog2.FunOk choose := by
  refine ⟨rTy, chooseEff, ?hty, .int (choose_le _ _ ?_ ?_ ?_), ?_, ?_, ?_, le_self _,
    fun _ h => by simp [chooseEff, Eff.union, Eff.none, Eff.ofFn] at h⟩
  case hty =>
    dsimp only [choose]
    refine .letR ?r1 (.bool (le_self _)) ?b1 (by simp [LTy.InPlace, cLabel, tT, lT, LTy.deep, LExpr.promote, LExpr.join])
    · exact .eq (.var rfl) (.var rfl) (.var (le_self _)) (.var (le_self _)) rfl
    refine .letE ?r2 (.var (le_self _)) ?b2 (by simp [LTy.InPlace, xTy, cLabel, tT, lT, LTy.raise, LTy.deep, LExpr.promote, LExpr.join])
    · exact .ite (.var rfl) (.atom (.var rfl)) (.atom (.var rfl)) (.var (le_self _))
        (.var (le_self _)) (by simp [LTy.InPlace, lT])
    refine .letR ?r3 (.int (le_self _)) (.atom (.var rfl)) (by simp [LTy.InPlace, rTy, LTy.raise, LExpr.join, LExpr.bot, LExpr.ofLabel, decision])
    refine LRhsTy.apply (ps := [tT]) (r := .int .bot) (o := false) (g := HostFx.none) (fl := false)
      (lf := .bot) (ts := [xTy]) ?hf ?has ?fl ?bd
    case hf => exact .var rfl
    case has => exact .cons (.var rfl) .nil
    case bd => exact ⟨fun h => (by cases h), fun h => (by cases h), fun h => (by cases h), fun h => (by cases h),
      fun h => (by cases h)⟩
    case fl =>
      refine .cons (.var (choose_le _ _ ?_ ?_ ?_)) .nil
      all_goals simp [xTy, tT, lT, cLabel, LTy.raise, LTy.deep, LExpr.join, LExpr.promote,
        Label.join, Label.bot, decision, Level.join, Level.rank]
      all_goals decide
  all_goals simp [xTy, rTy, tT, lT, lsT, cLabel, LTy.raise, LTy.deep, LExpr.join,
    LExpr.promote, LExpr.bot, LExpr.ofLabel, Label.join, Label.bot, decision,
    Level.join, Level.rank, choose, chooseEff, Eff.union, Eff.none, Eff.ofFn, HostFx.Le, HostFx.or,
    HostFx.none]
  all_goals decide

/-- **`==` needs `Eq`.** In a function that does not constrain `T` by `Eq`, such as
`same[T](a: T, b: T) -> Bool = a == b`, no typing of `a == b` exists over values of `T`
(U2026, `spec/20`). -/
theorem eq_needs_Eq {p : Program} {Γ : LCtx} {pc : LExpr} {b : Atom} {t : LTy} {ε : Eff}
    {l : LExpr} (ha : lctxLookup Γ "a" = some (.var "T" l)) :
    ¬ LRhsTy p [] Γ pc (.eq (.var "a") b) t ε := by
  intro h
  cases h with
  | eq ha' _ hf₁ _ he =>
    cases ha' with
    | var hx =>
      rw [ha] at hx
      cases hx
      cases hf₁
      simp [Program.eqIn, LTy.eqIn] at he

end Una.F1.Examples

namespace Una.F1.Examples

/-- The preservation of `ℓ_T` in place, applied: the typed call of `pick` keeps it. -/
theorem callPick_inPlace : pickInt.InPlace :=
  (LExprTy.inPlace prog_wf (fun _ h => by simp at h) rfl callPick_typed).1

end Una.F1.Examples
