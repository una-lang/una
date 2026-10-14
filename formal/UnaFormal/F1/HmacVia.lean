/-
ADR 0098's shape of a `via` function (#587): a body whose only failure points are `Keys.hmac`
calls with a literal key name, at its top level. Its run records only `keyUse` ghosts and leaves
the world as it was (S1), and whether it fails, and with which code, depends on nothing the
environment holds (S2): the key name is a literal, and the host's grants are fixed.
-/
import UnaFormal.F1.B1

namespace Una.F1

/-- `@k.hmac("literal", d)` (ADR 0098 rule 2): a key name that is a text literal. -/
def Rhs.HmacLit : Rhs → Prop
  | .hmac (.text _) _ => True
  | _ => False

/-- **ADR 0098's shape.** A body whose failure points are top-level hmac calls with a literal key
name, their failure passed on; every other part is pure. A branch, a handler or an `attempt`
holds pure code only, so no hmac call is under a branch. -/
def Expr.HmacOnly : Expr → Prop
  | .atom _ => True
  | .letR _ _ v b => (v.Pure ∨ v.HmacLit) ∧ b.HmacOnly
  | .letE _ _ v b => v.Pure ∧ b.HmacOnly
  | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .caseSum _ _ _ t e => t.Pure ∧ e.Pure
  | .attempt b => b.Pure
  | _ => False

variable {H : Host}

/-- A run whose only events are `keyUse` ghosts, and that leaves the world as it was. -/
def Ghostly (w : World H) (r : Run H) : Prop :=
  (∀ ev ∈ r.1, ∃ k, ev = Event.keyUse k) ∧ r.2.2 = w

theorem Quiet.ghostly {w : World H} {r : Run H} (h : Quiet w r) : Ghostly w r :=
  ⟨(fun ev hev => by rw [h.1] at hev; nomatch hev), h.2.2⟩

theorem ghostly_andThen {w : World H} {first : Run H} {rest : World H → Val → Run H}
    (h₁ : Ghostly w first) (h₂ : ∀ v, Ghostly w (rest w v)) : Ghostly w (andThen first rest) := by
  obtain ⟨t, r, w'⟩ := first
  obtain ⟨ht, hw⟩ := h₁
  simp only at ht hw
  subst hw
  cases r with
  | ok v =>
    simp only [andThen]
    have := h₂ v
    rcases hq : rest w' v with ⟨t', r', w''⟩
    rw [hq] at this
    refine ⟨fun ev hev => ?_, this.2⟩
    rcases List.mem_append.mp hev with h | h
    · exact ht ev h
    · exact this.1 ev h
  | failed c => exact ⟨ht, rfl⟩
  | stuck => exact ⟨ht, rfl⟩
  | raised _ _ _ => exact ⟨ht, rfl⟩
  | outOfFuel | aborted => exact ⟨ht, rfl⟩

/-- An hmac call records one ghost and leaves the world as it was. -/
theorem hmac_ghostly (p : Program) (w : World H) (n : Nat) (env : Env) (k d : Atom) :
    Ghostly w (evalRhs H (n + 1) p w env (.hmac k d)) := by
  simp only [evalRhs]
  split
  · split
    · split
      · exact ⟨(fun ev hev => ⟨_, by simpa using hev⟩), rfl⟩
      · exact ⟨(fun ev hev => ⟨_, by simpa using hev⟩), rfl⟩
    · exact ⟨(fun _ h => nomatch h), rfl⟩
  · exact ⟨(fun _ h => nomatch h), rfl⟩

/-- **S1.** A body of ADR 0098's shape records only `keyUse` ghosts and leaves the world as it
was, at any fuel. -/
theorem hmacOnly_ghostly (p : Program) (w : World H) : ∀ (n : Nat) (env : Env) (e : Expr),
    e.HmacOnly → Ghostly w (eval H n p w env e)
  | 0, _, _, _ => ⟨(fun _ h => by simp [eval] at h), by simp [eval]⟩
  | n + 1, env, e, he => by
    cases e with
    | atom a => exact ((pure_run p w (n + 1)).2 env (.atom a) trivial).ghostly
    | letR x t v b =>
      simp only [eval]
      refine ghostly_andThen ?_ (fun u => hmacOnly_ghostly p w n _ b he.2)
      rcases he.1 with hv | hv
      · exact ((pure_run p w n).1 env v hv).ghostly
      · cases n with
        | zero => exact ⟨(fun _ h => nomatch h), rfl⟩
        | succ m =>
          cases v with
          | hmac k d => exact hmac_ghostly p w m env k d
          | _ => exact (show False from hv).elim
    | letE x t v b =>
      simp only [eval]
      exact ghostly_andThen ((pure_run p w n).2 env v he.1).ghostly
        (fun u => hmacOnly_ghostly p w n _ b he.2)
    | ite c t e =>
      simp only [eval]
      split
      · exact ((pure_run p w n).2 env t he.1).ghostly
      · exact ((pure_run p w n).2 env e he.2).ghostly
      · exact ⟨(fun _ h => nomatch h), rfl⟩
    | caseOption s nE x sE =>
      simp only [eval]
      split
      · exact ((pure_run p w n).2 env nE he.1).ghostly
      · exact ((pure_run p w n).2 _ sE he.2).ghostly
      · exact ⟨(fun _ h => nomatch h), rfl⟩
    | caseResult s x oE y eE =>
      simp only [eval]
      split
      · exact ((pure_run p w n).2 _ oE he.1).ghostly
      · exact ((pure_run p w n).2 _ eE he.2).ghostly
      · exact ⟨(fun _ h => nomatch h), rfl⟩
    | caseSum s i xs t e =>
      exact ((pure_run p w (n + 1)).2 env (.caseSum s i xs t e) he).ghostly
    | attempt b => exact ((pure_run p w (n + 1)).2 env (.attempt b) he).ghostly
    | _ => exact (show False from he).elim

/-! ### S2: the failure depends on nothing the environment holds -/

/-- The code of a failure, `none` for any other outcome. -/
def Res.failCode : Res → Option Nat
  | .failed c => some c
  | _ => none

/-- A run that ended: with a value or a failure. -/
def Res.Ends : Res → Prop
  | .ok _ | .failed _ | .raised _ _ _ => True
  | _ => False

theorem Rhs.Pure.pureOrFails {r : Rhs} (h : r.Pure) : r.PureOrFails := by
  cases r <;> simp_all [Rhs.PureOrFails, Rhs.Pure]

theorem Expr.Pure.pureOrFails : ∀ (e : Expr), e.Pure → e.PureOrFails
  | .atom _, _ => trivial
  | .letR _ _ _ b, h => ⟨Rhs.Pure.pureOrFails h.1, Expr.Pure.pureOrFails b h.2⟩
  | .letE _ _ v b, h => ⟨Expr.Pure.pureOrFails v h.1, Expr.Pure.pureOrFails b h.2⟩
  | .ite _ t e, h | .caseOption _ t _ e, h | .caseResult _ _ t _ e, h | .handle t _ e, h
  | .caseSum _ _ _ t e, h | .handleK _ t _ e, h =>
    ⟨Expr.Pure.pureOrFails t h.1, Expr.Pure.pureOrFails e h.2⟩
  | .attempt b, h | .attemptK _ b, h => Expr.Pure.pureOrFails b h
  | .fail _, h | .raise _ _ _, h | .unreachable, h | .letTuple _ _ _, h | .loop _ _ _ _ _ _, h
  | .withH _ _ _, h => (show False from h).elim

/-- A pure run that ends gives a value. -/
theorem pure_ends_ok (p : Program) (w : World H) (n : Nat) (env : Env) {e : Expr} (he : e.Pure)
    (h : (eval H n p w env e).2.1.Ends) : ∃ v, (eval H n p w env e).2.1 = .ok v := by
  have hq := (pure_run p w n).2 env e he
  have hr := (noRaise_run p w n).2 env e (Expr.Pure.pureOrFails _ he)
  cases hres : (eval H n p w env e).2.1 with
  | ok v => exact ⟨v, rfl⟩
  | failed c => exact (hq.2.1 c hres).elim
  | raised k c ps => exact (hr k c ps hres).elim
  | _ => rw [hres] at h; exact (show False from h).elim

theorem pure_rhs_ends_ok (p : Program) (w : World H) (n : Nat) (env : Env) {r : Rhs} (hr' : r.Pure)
    (h : (evalRhs H n p w env r).2.1.Ends) : ∃ v, (evalRhs H n p w env r).2.1 = .ok v := by
  have hq := (pure_run p w n).1 env r hr'
  have hr := (noRaise_run p w n).1 env r (Rhs.Pure.pureOrFails hr')
  cases hres : (evalRhs H n p w env r).2.1 with
  | ok v => exact ⟨v, rfl⟩
  | failed c => exact (hq.2.1 c hres).elim
  | raised k c ps => exact (hr k c ps hres).elim
  | _ => rw [hres] at h; exact (show False from h).elim

/-- When the first part of a sequence does not end, neither does the sequence. -/
theorem andThen_ends {first : Run H} {rest : World H → Val → Run H}
    (h : (andThen first rest).2.1.Ends) :
    first.2.1.Ends := by
  obtain ⟨t, r, w⟩ := first
  cases r <;> simp_all [andThen, Res.Ends]

/-- The outcome of a sequence whose first part gives a value is its rest's. -/
theorem andThen_ok_res {t : Trace} {v : Val} {w : World H} {rest : World H → Val → Run H} :
    (andThen (t, .ok v, w) rest).2.1 = (rest w v).2.1 := by
  simp [andThen]

/-- A pure body's outcome that ends is a value, with no failure code. -/
theorem pure_failCode (p : Program) (w : World H) (n : Nat) (env : Env) {e : Expr} (he : e.Pure)
    (h : (eval H n p w env e).2.1.Ends) : (eval H n p w env e).2.1.failCode = none := by
  obtain ⟨v, hv⟩ := pure_ends_ok p w n env he h
  rw [hv]; rfl

/-- **S2.** For a body of ADR 0098's shape, two runs that both end agree on whether they fail,
and on the code, whatever their environments and worlds hold: a pure part cannot fail, and an hmac
call with a literal key name fails exactly when the host has not granted that key. A run that does
not end, stuck or out of fuel before an hmac call, is the termination channel T6 does not claim
(ADR 0057). -/
theorem hmacOnly_failCode (p : Program) : ∀ (n : Nat) (w₁ w₂ : World H) (env₁ env₂ : Env)
    (e : Expr), e.HmacOnly → (eval H n p w₁ env₁ e).2.1.Ends → (eval H n p w₂ env₂ e).2.1.Ends →
      (eval H n p w₁ env₁ e).2.1.failCode = (eval H n p w₂ env₂ e).2.1.failCode
  | 0, _, _, _, _, _, _, h₁, _ => by simp [eval, Res.Ends] at h₁
  | n + 1, w₁, w₂, env₁, env₂, e, he, h₁, h₂ => by
    cases e with
    | atom a =>
      rw [pure_failCode p w₁ (n + 1) env₁ (e := .atom a) trivial h₁,
        pure_failCode p w₂ (n + 1) env₂ (e := .atom a) trivial h₂]
    | ite c t e =>
      rw [pure_failCode p w₁ (n + 1) env₁ (e := .ite c t e) he h₁,
        pure_failCode p w₂ (n + 1) env₂ (e := .ite c t e) he h₂]
    | caseOption s nE x sE =>
      rw [pure_failCode p w₁ (n + 1) env₁ (e := .caseOption s nE x sE) he h₁,
        pure_failCode p w₂ (n + 1) env₂ (e := .caseOption s nE x sE) he h₂]
    | caseResult s x oE y eE =>
      rw [pure_failCode p w₁ (n + 1) env₁ (e := .caseResult s x oE y eE) he h₁,
        pure_failCode p w₂ (n + 1) env₂ (e := .caseResult s x oE y eE) he h₂]
    | caseSum s i xs t e =>
      rw [pure_failCode p w₁ (n + 1) env₁ (e := .caseSum s i xs t e) he h₁,
        pure_failCode p w₂ (n + 1) env₂ (e := .caseSum s i xs t e) he h₂]
    | attempt b =>
      rw [pure_failCode p w₁ (n + 1) env₁ (e := .attempt b) he h₁,
        pure_failCode p w₂ (n + 1) env₂ (e := .attempt b) he h₂]
    | letE x t v b =>
      simp only [eval] at h₁ h₂ ⊢
      obtain ⟨u₁, hu₁⟩ := pure_ends_ok p w₁ n env₁ he.1 (andThen_ends h₁)
      obtain ⟨u₂, hu₂⟩ := pure_ends_ok p w₂ n env₂ he.1 (andThen_ends h₂)
      have hw₁ := ((pure_run p w₁ n).2 env₁ v he.1).2.2
      have hw₂ := ((pure_run p w₂ n).2 env₂ v he.1).2.2
      rcases hr₁ : eval H n p w₁ env₁ v with ⟨t₁, r₁, w₁'⟩
      rcases hr₂ : eval H n p w₂ env₂ v with ⟨t₂, r₂, w₂'⟩
      rw [hr₁] at hu₁ hw₁ h₁
      rw [hr₂] at hu₂ hw₂ h₂
      simp only at hu₁ hu₂ hw₁ hw₂
      subst hu₁ hu₂; subst w₁'; subst w₂'
      rw [andThen_ok_res] at h₁ h₂ ⊢
      rw [andThen_ok_res]
      exact hmacOnly_failCode p n w₁ w₂ _ _ b he.2 h₁ h₂
    | letR x t v b =>
      simp only [eval] at h₁ h₂ ⊢
      rcases he.1 with hv | hv
      · obtain ⟨u₁, hu₁⟩ := pure_rhs_ends_ok p w₁ n env₁ hv (andThen_ends h₁)
        obtain ⟨u₂, hu₂⟩ := pure_rhs_ends_ok p w₂ n env₂ hv (andThen_ends h₂)
        have hw₁ := ((pure_run p w₁ n).1 env₁ v hv).2.2
        have hw₂ := ((pure_run p w₂ n).1 env₂ v hv).2.2
        rcases hr₁ : evalRhs H n p w₁ env₁ v with ⟨t₁, r₁, w₁'⟩
        rcases hr₂ : evalRhs H n p w₂ env₂ v with ⟨t₂, r₂, w₂'⟩
        rw [hr₁] at hu₁ hw₁ h₁
        rw [hr₂] at hu₂ hw₂ h₂
        simp only at hu₁ hu₂ hw₁ hw₂
        subst hu₁ hu₂; subst w₁'; subst w₂'
        rw [andThen_ok_res] at h₁ h₂ ⊢
        rw [andThen_ok_res]
        exact hmacOnly_failCode p n w₁ w₂ _ _ b he.2 h₁ h₂
      · cases v with
        | hmac k d =>
          cases k with
          | text kn =>
            cases n with
            | zero => simp [evalRhs, andThen, Res.Ends] at h₁
            | succ m =>
              have e₁ := andThen_ends h₁
              have e₂ := andThen_ends h₂
              simp only [evalRhs] at e₁ e₂ h₁ h₂ ⊢
              cases hd₁ : evalAtom env₁ d with
              | none => rw [hd₁] at e₁; simp [Res.Ends, stuck] at e₁
              | some d₁ =>
                cases hd₂ : evalAtom env₂ d with
                | none => rw [hd₂] at e₂; simp [Res.Ends, stuck] at e₂
                | some d₂ =>
                  simp only [hd₁, hd₂] at e₁ e₂ h₁ h₂ ⊢
                  cases hl₁ : d₁.line with
                  | none => simp [hl₁, Res.Ends, stuck, evalAtom] at e₁
                  | some s₁ =>
                    cases hl₂ : d₂.line with
                    | none => simp [hl₂, Res.Ends, stuck, evalAtom] at e₂
                    | some s₂ =>
                      simp only [evalAtom, hl₁, hl₂] at h₁ h₂ ⊢
                      cases hk : H.key kn with
                      | none => simp [andThen, Res.failCode]
                      | some km =>
                        simp only [hk] at h₁ h₂ ⊢
                        rw [andThen_ok_res] at h₁ h₂ ⊢
                        rw [andThen_ok_res]
                        exact hmacOnly_failCode p (m + 1) w₁ w₂ _ _ b he.2 h₁ h₂
          | _ => exact (show False from hv).elim
        | _ => exact (show False from hv).elim
    | _ => exact (show False from he).elim

/-! ### The test of the shape is sound -/

mutual
  theorem Rhs.pureB_sound : ∀ {r : Rhs}, r.pureB = true → r.Pure := by
    intro r h
    cases r <;> simp_all [Rhs.pureB, Rhs.Pure]

  theorem Expr.pureB_sound : ∀ (e : Expr), e.pureB = true → e.Pure
    | .atom _, _ => trivial
    | .letR _ _ _ b, h => by
      simp only [Expr.pureB, Bool.and_eq_true] at h
      exact ⟨Rhs.pureB_sound h.1, Expr.pureB_sound b h.2⟩
    | .letE _ _ v b, h => by
      simp only [Expr.pureB, Bool.and_eq_true] at h
      exact ⟨Expr.pureB_sound v h.1, Expr.pureB_sound b h.2⟩
    | .ite _ t e, h | .caseOption _ t _ e, h | .caseResult _ _ t _ e, h | .handle t _ e, h
    | .caseSum _ _ _ t e, h | .handleK _ t _ e, h => by
      simp only [Expr.pureB, Bool.and_eq_true] at h
      exact ⟨Expr.pureB_sound t h.1, Expr.pureB_sound e h.2⟩
    | .attempt b, h | .attemptK _ b, h => by
      simp only [Expr.pureB] at h
      exact Expr.pureB_sound b h
    | .fail _, h | .raise _ _ _, h | .unreachable, h | .letTuple _ _ _, h | .loop _ _ _ _ _ _, h
    | .withH _ _ _, h => by simp [Expr.pureB] at h
end

theorem Rhs.hmacLitB_sound {r : Rhs} (h : r.hmacLitB = true) : r.HmacLit := by
  cases r <;> simp [Rhs.hmacLitB] at h
  rename_i k _
  cases k <;> simp_all [Rhs.HmacLit, Rhs.hmacLitB]

theorem Expr.hmacOnlyB_sound : ∀ (e : Expr), e.hmacOnlyB = true → e.HmacOnly
  | .atom _, _ => trivial
  | .letR _ _ v b, h => by
    simp only [Expr.hmacOnlyB, Bool.and_eq_true, Bool.or_eq_true] at h
    exact ⟨h.1.elim (fun h => .inl (Rhs.pureB_sound h)) (fun h => .inr (Rhs.hmacLitB_sound h)),
      Expr.hmacOnlyB_sound b h.2⟩
  | .letE _ _ v b, h => by
    simp only [Expr.hmacOnlyB, Bool.and_eq_true] at h
    exact ⟨Expr.pureB_sound v h.1, Expr.hmacOnlyB_sound b h.2⟩
  | .ite _ t e, h | .caseOption _ t _ e, h | .caseResult _ _ t _ e, h | .caseSum _ _ _ t e, h => by
    simp only [Expr.hmacOnlyB, Bool.and_eq_true] at h
    exact ⟨Expr.pureB_sound t h.1, Expr.pureB_sound e h.2⟩
  | .attempt b, h => by
    simp only [Expr.hmacOnlyB] at h
    exact Expr.pureB_sound b h
  | .fail _, h | .raise _ _ _, h | .unreachable, h | .letTuple _ _ _, h | .loop _ _ _ _ _ _, h
  | .withH _ _ _, h | .handle _ _ _, h | .handleK _ _ _ _, h | .attemptK _ _, h => by
    simp [Expr.hmacOnlyB] at h

/-- What `Fun.hmacVia` gives: no output, `Keys` only, no payload failure, and the shape. -/
theorem Fun.hmacVia_sound {fn : Fun} (h : fn.hmacVia = true) :
    fn.op = false ∧ fn.host = HostFx.keys ∧ fn.fails = true ∧ fn.raises = [] ∧
      fn.body.HmacOnly := by
  simp only [Fun.hmacVia, Bool.and_eq_true, Bool.not_eq_true', List.isEmpty_iff] at h
  obtain ⟨⟨⟨⟨⟨⟨⟨hop, hrd⟩, hwr⟩, hrq⟩, hky⟩, hf⟩, hr⟩, hb⟩ := h
  refine ⟨hop, ?_, hf, hr, Expr.hmacOnlyB_sound _ hb⟩
  rcases hh : fn.host with ⟨rd, wr, rq, ky⟩
  rw [hh] at hrd hwr hrq hky
  simp_all [HostFx.keys]

/-! ### A body of the shape never fails with a payload -/

/-- An hmac call never fails with a payload: it fails with a code, or gives a value. -/
theorem hmac_noRaise (p : Program) (w : World H) (n : Nat) (env : Env) (k d : Atom) :
    ∀ kd c ps, (evalRhs H n p w env (.hmac k d)).2.1 ≠ .raised kd c ps := by
  intro kd c ps h
  cases n with
  | zero => simp [evalRhs] at h
  | succ m =>
    simp only [evalRhs] at h
    split at h
    · split at h
      · split at h <;> simp at h
      · simp [stuck] at h
    · simp [stuck] at h

/-- **A body of ADR 0098's shape never fails with a payload**, as a `via` function's must not
(#512): its pure parts raise nothing, and an hmac call fails with a code. -/
theorem hmacOnly_noRaise (p : Program) (w : World H) : ∀ (n : Nat) (env : Env) (e : Expr),
    e.HmacOnly → ∀ k c ps, (eval H n p w env e).2.1 ≠ .raised k c ps
  | 0, _, _, _, _, _, _, h => by simp [eval] at h
  | n + 1, env, e, he, k, c, ps, h => by
    have pure_nr : ∀ (m : Nat) (env' : Env) (e' : Expr), e'.Pure →
        (eval H m p w env' e').2.1 ≠ .raised k c ps :=
      fun m env' e' hp => (noRaise_run p w m).2 env' e' (Expr.Pure.pureOrFails _ hp) k c ps
    cases e with
    | atom a => exact pure_nr (n + 1) env (.atom a) trivial h
    | ite c' t e => exact pure_nr (n + 1) env (.ite c' t e) he h
    | caseOption s nE x sE => exact pure_nr (n + 1) env (.caseOption s nE x sE) he h
    | caseResult s x oE y eE => exact pure_nr (n + 1) env (.caseResult s x oE y eE) he h
    | caseSum s i xs t e => exact pure_nr (n + 1) env (.caseSum s i xs t e) he h
    | attempt b => exact pure_nr (n + 1) env (.attempt b) he h
    | letE x t v b =>
      simp only [eval] at h
      rcases hr : eval H n p w env v with ⟨t₁, r₁, w₁⟩
      rw [hr] at h
      cases r₁ with
      | ok u =>
        simp only [andThen] at h
        rcases hq : eval H n p w₁ ((x, u) :: env) b with ⟨t₂, r₂, w₂⟩
        rw [hq] at h
        have hw : w₁ = w := by
          have := ((pure_run p w n).2 env v he.1).2.2; rw [hr] at this; exact this
        subst hw
        exact hmacOnly_noRaise p w₁ n _ b he.2 k c ps (by rw [hq]; exact h)
      | raised k' c' ps' =>
        simp only [andThen] at h
        exact pure_nr n env v he.1 (by rw [hr]; exact h)
      | _ => simp [andThen] at h
    | letR x t v b =>
      simp only [eval] at h
      rcases hr : evalRhs H n p w env v with ⟨t₁, r₁, w₁⟩
      rw [hr] at h
      have hwv : w₁ = w := by
        rcases he.1 with hv | hv
        · have := ((pure_run p w n).1 env v hv).2.2; rw [hr] at this; exact this
        · cases v with
          | hmac k' d =>
            cases n with
            | zero => simp [evalRhs] at hr; exact hr.2.2.symm
            | succ m =>
              have := (hmac_ghostly p w m env k' d).2; rw [hr] at this; exact this
          | _ => exact (show False from hv).elim
      subst hwv
      cases r₁ with
      | ok u =>
        simp only [andThen] at h
        rcases hq : eval H n p w₁ ((x, u) :: env) b with ⟨t₂, r₂, w₂⟩
        rw [hq] at h
        exact hmacOnly_noRaise p w₁ n _ b he.2 k c ps (by rw [hq]; exact h)
      | raised k' c' ps' =>
        simp only [andThen] at h
        rcases he.1 with hv | hv
        · exact (noRaise_run p w₁ n).1 env v (Rhs.Pure.pureOrFails hv) k c ps (by rw [hr]; exact h)
        · cases v with
          | hmac k'' d => exact hmac_noRaise p w₁ n env k'' d k c ps (by rw [hr]; exact h)
          | _ => exact (show False from hv).elim
      | _ => simp [andThen] at h
    | _ => exact (show False from he).elim

end Una.F1
