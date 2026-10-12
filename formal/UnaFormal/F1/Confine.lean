/-
#562, ADR 0093's authority confinement (`docs/plan/formal-track-boundary.md`): **every effect
performed during a call into a boundary module is granted by the module's ceiling**.

F1 has no capability bindings by name, so a ceiling is said of what runs: the events of the trace
(`out`, `read`, `write`, `req`, and `release`, which no ceiling grants), and the user capability
operations an invocation dispatches to a handler (`invokeOp`, #538). `Expr.within a` checks a body
against a ceiling `a`, as `Expr.hostFree` checks it against none, and `eval_confined` is
`eval_hostFreeIn`'s method (#472) with "no host operation" generalized to "only what `a` grants".
Whatever runs during the call is confined: the callee, the functions it reaches, the handlers its
invocations may dispatch to, and the closures it is given, the boundary's own code and the
caller's alike. T6 and `Program` do not change: a ceiling is a separate parameter.
-/
import UnaFormal.F1.HostFree

namespace Una.F1

/-- A ceiling (ADR 0093 rule 1): the operation `op` (`out`), `Files.read`, `Files.write`, a
request of `Http`, `Env.get`, and the user capability operations, by name. -/
structure Auth where
  out : Bool := false
  rd : Bool := false
  wr : Bool := false
  rq : Bool := false
  env : Bool := false
  ops : List String := []

/-- The events a ceiling allows. No ceiling grants a release (N-155): a policy belongs to the
trusted modules, and a boundary module never reaches one. -/
def Auth.allows (a : Auth) : Event → Prop
  | .out _ => a.out = true
  | .read _ => a.rd = true
  | .write _ _ => a.wr = true
  | .req _ _ => a.rq = true
  | .release _ _ => False
  -- The ghost of an invocation of a `with` scope's operation (#561): granted when the ceiling
  -- names the operation, as `Rhs.within` asks of `invokeOp`.
  | .perf o => o ∈ a.ops
  -- The ghost of a #524 invocation, by the handler's function: confined by what the handler does,
  -- not by the operation's name (R11 of `docs/plan/formal-track-boundary.md`); #572 ties it to
  -- the handler's operation.
  | .perfFn _ => True

mutual
  /-- A right-hand side that performs only what `a` grants, in its lambdas included. -/
  def Rhs.within (a : Auth) : Rhs → Bool
    | .op _ => a.out
    | .read _ => a.rd
    | .write _ _ => a.wr
    | .req _ _ => a.rq
    | .envGet _ => a.env
    | .invokeOp o _ => a.ops.contains o
    | .declassify _ _ => false
    | .lambda _ b => b.within a
    | .atom _ | .prim _ _ | .call _ _ _ | .invoke _ _ | .apply _ _ | .construct _ _ | .variant _ _ _ | .field _ _
    | .list _ | .none | .some _ | .first _ | .elemAt _ _ | .orElse _ _ | .isOk _ | .mkOk _ | .mkErr _ | .funRef _ _
    | .map _ _ | .filter _ _
    | .firstWhere _ _ | .fold _ _ _ | .parse _ | .checked _ _ _ | .eq _ _ | .tuple _ | .append _ _ | .set _ _ _
    | .toText _ => true

  /-- A body that performs only what `a` grants, in its lambdas included. -/
  def Expr.within (a : Auth) : Expr → Bool
    | .atom _ | .fail _ | .raise _ _ _ | .unreachable => true
    | .withH _ _ b => b.within a
    | .letR _ _ v b => v.within a && b.within a
    | .letE _ _ v b => v.within a && b.within a
    | .ite _ t e => t.within a && e.within a
    | .caseOption _ n _ s => n.within a && s.within a
    | .caseResult _ _ o _ e => o.within a && e.within a
    | .caseSum _ _ _ t e => t.within a && e.within a
    | .attempt b | .attemptK _ b => b.within a
    | .handle b _ h | .handleK _ b _ h => b.within a && h.within a
    | .letTuple _ _ b | .loop _ _ _ _ _ b => b.within a
end

mutual
  /-- A value whose closures perform only what `a` grants and call only functions in `S`. -/
  def Val.WithinIn (S : String → Prop) (a : Auth) : Val → Prop
    | .list items => Val.WithinAllIn S a items
    | .some v => Val.WithinIn S a v
    | .ok v => Val.WithinIn S a v
    | .err v => Val.WithinIn S a v
    | .record _ fs | .variant _ _ fs | .tuple fs => Val.WithinAllIn S a fs
    | .closure _ b env => b.within a = true ∧ (∀ g ∈ b.callees, S g) ∧ Val.WithinEnvIn S a env
    | .int _ | .bool _ | .text _ | .unit | .none => True

  def Val.WithinAllIn (S : String → Prop) (a : Auth) : List Val → Prop
    | [] => True
    | v :: vs => Val.WithinIn S a v ∧ Val.WithinAllIn S a vs

  def Val.WithinEnvIn (S : String → Prop) (a : Auth) : List (String × Val) → Prop
    | [] => True
    | (_, v) :: env => Val.WithinIn S a v ∧ Val.WithinEnvIn S a env
end

/-- The program's part of a confinement: every function of `S` has a body within `a` and callees
in `S`, and every handler an invocation of an operation of `a` may dispatch to is in `S`. -/
structure Program.ConfinedAt (p : Program) (S : String → Prop) (a : Auth) : Prop where
  funs : ∀ f fn, p.find f = some fn → S f → fn.body.within a = true ∧ ∀ g ∈ fn.body.callees, S g
  handlers : ∀ o, a.ops.contains o = true → ∀ m, S (p.handlerFor o m)

section Values

variable {S : String → Prop} {a : Auth}

theorem Val.withinAll_iff : ∀ (vs : List Val), Val.WithinAllIn S a vs ↔ ∀ v ∈ vs, Val.WithinIn S a v
  | [] => by simp [Val.WithinAllIn]
  | v :: vs => by simp [Val.WithinAllIn, Val.withinAll_iff vs]

theorem lookup_within : ∀ {env : Env} {x : String} {v : Val}, Val.WithinEnvIn S a env →
    lookup env x = some v → Val.WithinIn S a v
  | [], _, _, _, h => by simp [lookup] at h
  | (y, u) :: env, x, v, hf, h => by
    simp only [Val.WithinEnvIn] at hf
    simp only [lookup] at h
    split at h
    · cases h; exact hf.1
    · exact lookup_within hf.2 h

theorem evalAtom_within {env : Env} (hf : Val.WithinEnvIn S a env) :
    ∀ {x : Atom} {v : Val}, evalAtom env x = some v → Val.WithinIn S a v
  | .var _, _, h => lookup_within hf h
  | .int _, _, h | .bool _, _, h | .text _, _, h | .unit, _, h => by
    simp only [evalAtom, Option.some.injEq] at h
    subst h
    simp [Val.WithinIn]

theorem evalAtoms_within {env : Env} (hf : Val.WithinEnvIn S a env) :
    ∀ {as : List Atom} {vs : List Val}, evalAtoms env as = some vs → Val.WithinAllIn S a vs
  | [], vs, h => by
    simp only [evalAtoms, Option.some.injEq] at h
    subst h
    simp [Val.WithinAllIn]
  | x :: as, vs, h => by
    simp only [evalAtoms] at h
    cases ha : evalAtom env x <;> cases has : evalAtoms env as <;> simp [ha, has] at h
    subst h
    simp only [Val.WithinAllIn]
    exact ⟨evalAtom_within hf ha, evalAtoms_within hf has⟩

-- The case split over every primitive needs `HostFree`'s budget (#548).
set_option maxHeartbeats 1000000 in
theorem evalPrim_within {pr : Prim} {vs : List Val} {v : Val} (h : evalPrim pr vs = some v) :
    Val.WithinIn S a v := by
  cases pr
  case slice =>
    rcases vs with _ | ⟨x, _ | ⟨y, _ | ⟨z, _ | ⟨d, rest⟩⟩⟩⟩ <;> (try cases x) <;> (try cases y) <;>
      (try cases z) <;> simp [evalPrim] at h <;> (try subst h) <;> simp [Val.WithinIn]
  all_goals rcases vs with _ | ⟨x, _ | ⟨y, _ | ⟨z, rest⟩⟩⟩ <;> (try cases x) <;> (try cases y) <;>
    simp [evalPrim] at h <;> (try obtain ⟨_, _, rfl⟩ := h) <;> (try subst h) <;> simp [Val.WithinIn]

theorem zip_within : ∀ (xs : List String) {vs : List Val} {env : Env},
    Val.WithinAllIn S a vs → Val.WithinEnvIn S a env → Val.WithinEnvIn S a (xs.zip vs ++ env)
  | [], _, _, _, he => by simpa using he
  | _ :: _, [], _, _, he => by simpa using he
  | _ :: xs, _ :: _, _, hv, he => by
    simp only [Val.WithinAllIn] at hv
    simp only [List.zip_cons_cons, List.cons_append, Val.WithinEnvIn]
    exact ⟨hv.1, zip_within xs hv.2 he⟩

theorem bindParams_within {α : Type} : ∀ (ps : List (String × α)) {vs : List Val} {env : Env},
    Val.WithinAllIn S a vs → Val.WithinEnvIn S a env → Val.WithinEnvIn S a (bindParams ps vs env)
  | [], _, _, _, he => by simpa [bindParams] using he
  | _ :: _, [], _, _, he => by simpa [bindParams] using he
  | _ :: ps, v :: vs, env, hv, he => by
    simp only [Val.WithinAllIn] at hv
    have := bindParams_within ps hv.2 he
    simp only [bindParams, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
    simp only [Val.WithinEnvIn]
    exact ⟨hv.1, this⟩

/-- A response of `Http` (H4) has no closure, so it is within any ceiling. -/
theorem isResponse_within {v : Val} (h : v.IsResponse) : Val.WithinIn S a v := by
  unfold Val.IsResponse at h
  split at h
  · rename_i st hs body
    have hm : Val.WithinAllIn S a hs := by
      rw [Val.withinAll_iff]
      intro x hx
      obtain ⟨n, y, rfl⟩ := h x hx
      simp [Val.WithinIn, Val.WithinAllIn]
    simp [Val.WithinIn, Val.WithinAllIn, hm]
  · exact h.elim

end Values

section Runs

variable {H : Host} {S : String → Prop} {a : Auth}

/-- A run confined by `a`: each of its events is allowed, and its value and its failure's payload
are values within `a`. -/
def Conf (S : String → Prop) (a : Auth) (r : Run H) : Prop :=
  (∀ ev ∈ r.1, a.allows ev) ∧ (∀ v, r.2.1 = .ok v → Val.WithinIn S a v) ∧
    ∀ k c ps, r.2.1 = .raised k c ps → Val.WithinAllIn S a ps

/-- A ghost the ceiling allows, in front of a confined run (#561). -/
theorem Conf.ghost {r : Run H} {ev : Event} (hev : a.allows ev) (h : Conf S a r) :
    Conf S a (Run.ghost ev r) := by
  obtain ⟨tr, res, w⟩ := r
  refine ⟨fun e he => ?_, h.2⟩
  simp only [Run.ghost, List.mem_cons] at he
  rcases he with rfl | he
  · exact hev
  · exact h.1 e he

theorem Conf.stuck (w : World H) : Conf S a (stuck w) :=
  ⟨by simp [Una.F1.stuck], (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩

theorem Conf.abort (w : World H) : Conf S a (Una.F1.abort w) :=
  ⟨by simp [Una.F1.abort], (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩

theorem Conf.done (w : World H) {v : Val} (hv : Val.WithinIn S a v) : Conf S a (done w v) :=
  ⟨by simp [Una.F1.done], (fun _ h => by cases h; exact hv), (fun _ _ _ h => nomatch h)⟩

/-- A run without events, whose result's values are within `a`. -/
theorem Conf.ret (w : World H) {r : Res}
    (hr : ∀ v, r = .ok v → Val.WithinIn S a v)
    (hr' : ∀ k c ps, r = .raised k c ps → Val.WithinAllIn S a ps := by
      intro _ _ _ h
      first
      | (simp at h; done)
      | (simp only [Option.elim] at h; split at h <;> simp_all)
      | (split at h <;> simp_all)) :
    Conf S a (([], r, w) : Run H) :=
  ⟨by simp, hr, hr'⟩

/-- A run that ends with a failure or out of fuel, without events. -/
theorem Conf.halt (w : World H) {r : Res} (h₁ : ∀ v, r ≠ .ok v) (h₂ : ∀ k c ps, r ≠ .raised k c ps) :
    Conf S a (([], r, w) : Run H) :=
  ⟨by simp, (fun v h => absurd h (h₁ v)), (fun k c ps h => absurd h (h₂ k c ps))⟩

theorem andThen_conf {r : Run H} {k : World H → Val → Run H} (h : Conf S a r)
    (hk : ∀ w v, r.2.1 = .ok v → Val.WithinIn S a v → Conf S a (k w v)) :
    Conf S a (andThen r k) := by
  obtain ⟨t, res, w⟩ := r
  obtain ⟨ht, hv, hr⟩ := h
  cases res with
  | ok v =>
    obtain ⟨ht', hv', hr'⟩ := hk w v rfl (hv v rfl)
    simp only [andThen]
    generalize k w v = kr at ht' hv' hr' ⊢
    obtain ⟨t', r', w'⟩ := kr
    refine ⟨?_, hv', hr'⟩
    intro ev hev
    rcases List.mem_append.mp hev with h | h
    · exact ht ev h
    · exact ht' ev h
  | raised k c ps => exact ⟨ht, (fun _ h => nomatch h), hr⟩
  | failed _ | stuck | outOfFuel | aborted => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩

theorem applyWith_conf {run : World H → Env → Expr → Run H} {w : World H}
    (hrun : ∀ env e, Val.WithinEnvIn S a env → e.within a = true → (∀ g ∈ e.callees, S g) →
      Conf S a (run w env e))
    {fv : Val} {vs : List Val} (hfv : Val.WithinIn S a fv)
    (hvs : Val.WithinAllIn S a vs) : Conf S a (applyWith run w fv vs) := by
  cases fv with
  | closure ps b cenv =>
    simp only [Val.WithinIn] at hfv
    simp only [applyWith]
    split
    · exact hrun _ _ (bindParams_within ps hvs hfv.2.2) hfv.1 hfv.2.1
    · exact Conf.stuck w
  | _ => exact Conf.stuck w

theorem walkMap_conf {app : World H → Val → Run H}
    (happ : ∀ w v, Val.WithinIn S a v → Conf S a (app w v)) :
    ∀ (w : World H) (vs : List Val), Val.WithinAllIn S a vs → Conf S a (walkMap app w vs)
  | w, [], _ => Conf.done w (by simp [Val.WithinIn, Val.WithinAllIn])
  | w, v :: vs, hvs => by
    simp only [Val.WithinAllIn] at hvs
    simp only [walkMap]
    refine andThen_conf (happ w v hvs.1) fun w' r _ hr =>
      andThen_conf (walkMap_conf happ w' vs hvs.2) fun w'' rest _ hrest => ?_
    cases rest with
    | list rs =>
      refine Conf.done w'' ?_
      simp only [Val.WithinIn, Val.WithinAllIn] at hrest ⊢
      exact ⟨hr, hrest⟩
    | _ => exact Conf.stuck w''

theorem walkFilter_conf {app : World H → Val → Run H}
    (happ : ∀ w v, Val.WithinIn S a v → Conf S a (app w v)) :
    ∀ (w : World H) (vs : List Val), Val.WithinAllIn S a vs → Conf S a (walkFilter app w vs)
  | w, [], _ => Conf.done w (by simp [Val.WithinIn, Val.WithinAllIn])
  | w, v :: vs, hvs => by
    simp only [Val.WithinAllIn] at hvs
    simp only [walkFilter]
    refine andThen_conf (happ w v hvs.1) fun w' b _ _ => ?_
    cases b with
    | bool keep =>
      refine andThen_conf (walkFilter_conf happ w' vs hvs.2) fun w'' rest _ hrest => ?_
      cases rest with
      | list rs =>
        refine Conf.done w'' ?_
        simp only [Val.WithinIn] at hrest ⊢
        cases keep
        · exact hrest
        · simp only [ite_true, Val.WithinAllIn]
          exact ⟨hvs.1, hrest⟩
      | _ => exact Conf.stuck w''
    | _ => exact Conf.stuck w'

theorem walkFirst_conf {app : World H → Val → Run H}
    (happ : ∀ w v, Val.WithinIn S a v → Conf S a (app w v)) :
    ∀ (w : World H) (vs : List Val), Val.WithinAllIn S a vs → Conf S a (walkFirst app w vs)
  | w, [], _ => Conf.done w (by simp [Val.WithinIn])
  | w, v :: vs, hvs => by
    simp only [Val.WithinAllIn] at hvs
    simp only [walkFirst]
    refine andThen_conf (happ w v hvs.1) fun w' b _ _ => ?_
    cases b with
    | bool keep =>
      cases keep
      · exact walkFirst_conf happ w' vs hvs.2
      · exact Conf.done w' (by simp only [Val.WithinIn]; exact hvs.1)
    | _ => exact Conf.stuck w'

theorem walkFold_conf {app : World H → Val → Val → Run H}
    (happ : ∀ w x v, Val.WithinIn S a x → Val.WithinIn S a v → Conf S a (app w x v)) :
    ∀ (w : World H) (acc : Val) (vs : List Val), Val.WithinIn S a acc → Val.WithinAllIn S a vs →
      Conf S a (walkFold app w acc vs)
  | w, acc, [], hacc, _ => Conf.done w hacc
  | w, acc, v :: vs, hacc, hvs => by
    simp only [Val.WithinAllIn] at hvs
    simp only [walkFold]
    exact andThen_conf (happ w acc v hacc hvs.1) fun w' acc' _ hacc' =>
      walkFold_conf happ w' acc' vs hacc' hvs.2

theorem walkLoop_conf {body : World H → Val → List Val → Run H}
    (hb : ∀ w v st, Val.WithinIn S a v → Val.WithinAllIn S a st → Conf S a (body w v st)) :
    ∀ (w : World H) (st vs : List Val), Val.WithinAllIn S a st → Val.WithinAllIn S a vs →
      Conf S a (walkLoop body w st vs)
  | w, st, [], hst, _ => Conf.done w (by simp only [Val.WithinIn]; exact hst)
  | w, st, v :: vs, hst, hvs => by
    simp only [Val.WithinAllIn] at hvs
    simp only [walkLoop]
    refine andThen_conf (hb w v st hvs.1 hst) fun w' r _ hr => ?_
    cases r with
    | tuple st' =>
      simp only [Val.WithinIn] at hr
      simp only
      split
      · exact walkLoop_conf hb w' st' vs hr hvs.2
      · exact Conf.stuck w'
    | _ => exact Conf.stuck w'

theorem parseInt_conf (w : World H) (t : String) : Conf S a (parseInt w t) := by
  unfold parseInt
  cases t.toInt? with
  | some n => exact Conf.done w (by simp [Val.WithinIn])
  | none => exact Conf.halt w (fun _ h => nomatch h) (fun _ _ _ h => nomatch h)

theorem checkRun_conf {p : Program} (w : World H) {v : Val} {pred : String} {code : Nat}
    (hv : Val.WithinIn S a v) : Conf S a (checkRun p w v pred code) := by
  unfold checkRun
  split
  · split
    · exact Conf.done w hv
    · exact Conf.halt w (fun _ h => nomatch h) (fun _ _ _ h => nomatch h)
  · exact Conf.stuck w

variable {p : Program}

/-- **Confinement.** On a host whose responses are `Response`s (H4), a run of a body within `a`,
from values within `a`, in a program whose functions in `S`, and handlers of `a`'s operations,
are within `a`, performs only events `a` allows; its value and its failure's payload are within
`a`. -/
theorem eval_confined (hH : H.RespTyped) (hp : p.ConfinedAt S a) : ∀ n : Nat,
    (∀ (w : World H) env r, Val.WithinEnvIn S a env → r.within a = true →
      (∀ g ∈ r.callees, S g) → Conf S a (evalRhs H n p w env r)) ∧
    (∀ (w : World H) env e, Val.WithinEnvIn S a env → e.within a = true →
      (∀ g ∈ e.callees, S g) → Conf S a (eval H n p w env e))
  | 0 => ⟨fun w _ _ _ _ _ => by
        simp only [evalRhs]; exact Conf.halt w (fun _ h => nomatch h) (fun _ _ _ h => nomatch h),
      fun w _ _ _ _ _ => by
        simp only [eval]; exact Conf.halt w (fun _ h => nomatch h) (fun _ _ _ h => nomatch h)⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := eval_confined hH hp n
    have hfun : ∀ (w : World H) (f : String) (fn : Fun) (vs : List Val), p.find f = some fn → S f →
        Val.WithinAllIn S a vs →
        Conf S a (eval H n p w (bindParams fn.params vs []) fn.body) := fun w f fn vs hfd hS hvs =>
      have hfn := hp.funs f fn hfd hS
      ihE w _ _ (bindParams_within (env := []) _ hvs trivial) hfn.1 hfn.2
    constructor
    · intro w env r hf hr hc
      have happ : ∀ (w' : World H) fv vs, Val.WithinIn S a fv → Val.WithinAllIn S a vs →
          Conf S a (applyWith (eval H n p) w' fv vs) :=
        fun w' _ _ hfv hvs => applyWith_conf (ihE w') hfv hvs
      cases r with
      | atom x =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtom env x <;> simp [ha, Option.elim] at hv
        subst hv
        exact evalAtom_within hf ha
      | prim pr args =>
        simp only [evalRhs]
        cases ha : evalAtoms env args with
        | none => exact Conf.ret w fun v hv => by simp [Option.elim] at hv
        | some vs =>
          simp only [Option.map_some, Option.getD_some, Option.bind_some]
          split
          · exact Conf.ret w fun v hv => by simp at hv
          · refine Conf.ret w fun v hv => ?_
            cases hpr : evalPrim pr vs with
            | none => simp [hpr, Option.elim] at hv
            | some v' =>
              simp [hpr, Option.elim] at hv
              subst hv
              exact evalPrim_within hpr
      | call f targs args =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact Conf.stuck w
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact Conf.stuck w
          | some vs =>
            simp only []
            split
            · exact hfun w f fn vs hfd (hc f (by simp [Rhs.callees])) (evalAtoms_within hf ha)
            · exact Conf.stuck w
      | invoke f args =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact Conf.stuck w
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact Conf.stuck w
          | some vs =>
            simp only []
            split
            · exact Conf.ghost trivial (hfun w f fn vs hfd (hc f (by simp [Rhs.callees])) (evalAtoms_within hf ha))
            · exact Conf.stuck w
      | invokeOp o args =>
        simp only [Rhs.within] at hr
        simp only [evalRhs]
        cases hfd : p.find (p.handlerFor o (lookup env (markerName o))) with
        | none => exact Conf.stuck w
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact Conf.stuck w
          | some vs =>
            simp only []
            split
            · exact Conf.ghost (by simpa [Auth.allows] using hr) (hfun w _ fn vs hfd (hp.handlers o hr _) (evalAtoms_within hf ha))
            · exact Conf.stuck w
      | apply f args =>
        simp only [evalRhs]
        cases hfa : evalAtom env f with
        | none => exact Conf.stuck w
        | some fv =>
          cases ha : evalAtoms env args with
          | none => exact Conf.stuck w
          | some vs => exact happ w fv vs (evalAtom_within hf hfa) (evalAtoms_within hf ha)
      | op line =>
        simp only [Rhs.within] at hr
        simp only [evalRhs]
        cases evalAtom env line with
        | none => exact Conf.stuck w
        | some v =>
          dsimp only
          cases v.line with
          | none => exact Conf.stuck w
          | some s =>
            refine ⟨?_, (fun _ h => by cases h; simp [Val.WithinIn]), (fun _ _ _ h => nomatch h)⟩
            intro ev hev
            simp only [List.mem_singleton] at hev
            subst hev
            exact hr
      | read path =>
        simp only [Rhs.within] at hr
        simp only [evalRhs]
        cases evalAtom env path with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | text s =>
            dsimp only
            have hev : ∀ ev ∈ [Event.read s], a.allows ev := by
              intro ev hev
              simp only [List.mem_singleton] at hev
              subst hev
              exact hr
            cases H.read s w.fs with
            | some c => exact ⟨hev, (fun _ h => by cases h; simp [Val.WithinIn]), (fun _ _ _ h => nomatch h)⟩
            | none => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
          | _ => exact Conf.stuck w
      | write path content =>
        simp only [Rhs.within] at hr
        simp only [evalRhs]
        cases evalAtom env path with
        | none => cases evalAtom env content <;> exact Conf.stuck w
        | some v =>
          cases evalAtom env content with
          | none => cases v <;> exact Conf.stuck w
          | some c =>
            cases v with
            | text s =>
              cases c with
              | text c' =>
                dsimp only
                have hev : ∀ ev ∈ [Event.write s c'], a.allows ev := by
                  intro ev hev
                  simp only [List.mem_singleton] at hev
                  subst hev
                  exact hr
                cases H.write s c' w.fs with
                | some fs' => exact ⟨hev, (fun _ h => by cases h; simp [Val.WithinIn]), (fun _ _ _ h => nomatch h)⟩
                | none => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
              | _ => exact Conf.stuck w
            | _ => cases c <;> exact Conf.stuck w
      | req m args =>
        simp only [Rhs.within] at hr
        simp only [evalRhs]
        cases evalAtoms env args with
        | none => exact Conf.stuck w
        | some vs =>
          dsimp only
          have hev : ∀ ev ∈ [Event.req m vs], a.allows ev := by
            intro ev hev
            simp only [List.mem_singleton] at hev
            subst hev
            exact hr
          -- A response comes from the host: H4 makes it a `Response`, which has no closure.
          cases hq : H.request m vs w.srv with
          | mk res srv' =>
            cases res with
            | ok v => exact ⟨hev, (fun _ h => by cases h; exact isResponse_within (hH m vs _ _ v hq)),
                (fun _ _ _ h => nomatch h)⟩
            | error c => exact ⟨hev, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
      | construct record fields =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtoms env fields <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtoms_within hf ha
      | variant kind ctor fields =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtoms env fields <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtoms_within hf ha
      | field value index =>
        simp only [evalRhs]
        cases hv : evalAtom env value with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | record r fs =>
            have hfs := evalAtom_within hf hv
            simp only [Val.WithinIn] at hfs
            refine Conf.ret w fun x hx => ?_
            cases hi : fs[index]? <;> simp [hi, Option.elim] at hx
            subst hx
            exact (Val.withinAll_iff fs).mp hfs _ (List.mem_of_getElem? hi)
          | _ => exact Conf.stuck w
      | list items =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtoms_within hf ha
      | none => exact Conf.done w (by simp [Val.WithinIn])
      | some x =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtom env x <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtom_within hf ha
      | first x =>
        simp only [evalRhs]
        cases hv : evalAtom env x with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | list items =>
            have his := evalAtom_within hf hv
            simp only [Val.WithinIn] at his
            cases items with
            | nil => exact Conf.done w (by simp [Val.WithinIn])
            | cons y ys => exact Conf.done w ((Val.withinAll_iff _).mp his y (by simp))
          | _ => exact Conf.stuck w
      | elemAt xs i =>
        simp only [evalRhs]
        cases hv : evalAtom env xs with
        | none => cases evalAtom env i <;> exact Conf.stuck w
        | some v =>
          cases evalAtom env i with
          | none => cases v <;> exact Conf.stuck w
          | some k =>
            cases v with
            | list items =>
              cases k with
              | int n =>
                have his := evalAtom_within hf hv
                simp only [Val.WithinIn] at his
                refine Conf.done w ?_
                rcases atIndex_cases items n with h | ⟨x, hx, h⟩
                · rw [h]; simp [Val.WithinIn]
                · rw [h]; exact (Val.withinAll_iff _).mp his x hx
              | _ => exact Conf.stuck w
            | _ => cases k <;> exact Conf.stuck w
      | orElse o d =>
        simp only [evalRhs]
        cases ho : evalAtom env o with
        | none => cases evalAtom env d <;> exact Conf.stuck w
        | some v =>
          cases hd : evalAtom env d with
          | none => cases v <;> exact Conf.stuck w
          | some x =>
            cases v with
            | some y =>
              have hy := evalAtom_within hf ho
              simp only [Val.WithinIn] at hy
              exact Conf.done w hy
            | none => exact Conf.done w (evalAtom_within hf hd)
            | _ => exact Conf.stuck w
      | isOk r =>
        simp only [evalRhs]
        cases evalAtom env r with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | ok _ => exact Conf.done w (by simp [Val.WithinIn])
          | err _ => exact Conf.done w (by simp [Val.WithinIn])
          | _ => exact Conf.stuck w
      | mkOk x =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtom env x <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtom_within hf ha
      | mkErr x =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtom env x <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtom_within hf ha
      | lambda params body =>
        refine Conf.done w ?_
        simp only [Val.WithinIn]
        exact ⟨by simpa [Rhs.within] using hr, by simpa [Rhs.callees] using hc, hf⟩
      | funRef f targs =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact Conf.stuck w
        | some fn =>
          refine Conf.done w ?_
          simp only [Val.WithinIn]
          have hfn := hp.funs f fn hfd (hc f (by simp [Rhs.callees]))
          exact ⟨hfn.1, hfn.2, trivial⟩
      | declassify _ _ => simp [Rhs.within] at hr
      | map xs f =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact Conf.stuck w
        | some xv =>
          cases hfv : evalAtom env f with
          | none => cases xv <;> exact Conf.stuck w
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_within hf hx
              simp only [Val.WithinIn] at hi
              exact walkMap_conf (fun w' v hv => happ w' fv [v] (evalAtom_within hf hfv)
                (by simp only [Val.WithinAllIn]; exact ⟨hv, trivial⟩)) w items hi
            | _ => exact Conf.stuck w
      | filter xs keep =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact Conf.stuck w
        | some xv =>
          cases hfv : evalAtom env keep with
          | none => cases xv <;> exact Conf.stuck w
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_within hf hx
              simp only [Val.WithinIn] at hi
              exact walkFilter_conf (fun w' v hv => happ w' fv [v] (evalAtom_within hf hfv)
                (by simp only [Val.WithinAllIn]; exact ⟨hv, trivial⟩)) w items hi
            | _ => exact Conf.stuck w
      | firstWhere xs keep =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact Conf.stuck w
        | some xv =>
          cases hfv : evalAtom env keep with
          | none => cases xv <;> exact Conf.stuck w
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_within hf hx
              simp only [Val.WithinIn] at hi
              exact walkFirst_conf (fun w' v hv => happ w' fv [v] (evalAtom_within hf hfv)
                (by simp only [Val.WithinAllIn]; exact ⟨hv, trivial⟩)) w items hi
            | _ => exact Conf.stuck w
      | fold xs init f =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact Conf.stuck w
        | some xv =>
          cases hacc : evalAtom env init with
          | none => cases xv <;> exact Conf.stuck w
          | some acc =>
            cases hfv : evalAtom env f with
            | none => cases xv <;> exact Conf.stuck w
            | some fv =>
              cases xv with
              | list items =>
                have hi := evalAtom_within hf hx
                simp only [Val.WithinIn] at hi
                exact walkFold_conf (fun w' x v hx' hv => happ w' fv [x, v]
                  (evalAtom_within hf hfv)
                  (by simp only [Val.WithinAllIn]; exact ⟨hx', hv, trivial⟩)) w acc items
                  (evalAtom_within hf hacc) hi
              | _ => exact Conf.stuck w
      | parse s =>
        simp only [evalRhs]
        cases evalAtom env s with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | text t => exact parseInt_conf w t
          | _ => exact Conf.stuck w
      | checked x pred code =>
        simp only [evalRhs]
        cases ha : evalAtom env x with
        | none => exact Conf.stuck w
        | some v => exact checkRun_conf w (evalAtom_within hf ha)
      | eq x y =>
        simp only [evalRhs]
        cases evalAtom env x with
        | none => exact Conf.stuck w
        | some va =>
          cases evalAtom env y with
          | none => exact Conf.stuck w
          | some vb => exact Conf.done w (by simp [Val.WithinIn])
      | toText x =>
        simp only [evalRhs]
        cases evalAtom env x with
        | none => exact Conf.stuck w
        | some v => exact Conf.done w (by simp [Val.WithinIn])
      | envGet name =>
        simp only [evalRhs]
        cases evalAtom env name with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | text s =>
            refine Conf.done w ?_
            cases w.env s <;> simp [Val.WithinIn]
          | _ => exact Conf.stuck w
      | tuple items =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.WithinIn]
        exact evalAtoms_within hf ha
      | append xs item =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact Conf.stuck w
        | some xv =>
          cases hi : evalAtom env item with
          | none => cases xv <;> exact Conf.stuck w
          | some v =>
            cases xv with
            | list items =>
              have hl := evalAtom_within hf hx
              have hv := evalAtom_within hf hi
              simp only [Val.WithinIn, Val.withinAll_iff] at hl ⊢
              refine Conf.done w ?_
              simp only [Val.WithinIn, Val.withinAll_iff, List.mem_append, List.mem_singleton]
              rintro y (hy | rfl)
              · exact hl y hy
              · exact hv
            | _ => exact Conf.stuck w
      | set xs index item =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact Conf.stuck w
        | some xv =>
          cases evalAtom env index with
          | none => cases xv <;> exact Conf.stuck w
          | some iv =>
            cases hi : evalAtom env item with
            | none => cases xv <;> cases iv <;> exact Conf.stuck w
            | some v =>
              cases xv with
              | list items =>
                cases iv with
                | int i =>
                  simp only
                  split
                  · have hl := evalAtom_within hf hx
                    have hv := evalAtom_within hf hi
                    simp only [Val.WithinIn, Val.withinAll_iff] at hl
                    refine Conf.done w ?_
                    simp only [Val.WithinIn, Val.WithinAllIn, Val.withinAll_iff, and_true]
                    intro y hy
                    rcases List.mem_or_eq_of_mem_set hy with hy | rfl
                    · exact hl y hy
                    · exact hv
                  · exact Conf.abort w
                | _ => exact Conf.stuck w
              | _ => cases iv <;> exact Conf.stuck w
    · intro w env e hf he hc
      cases e with
      | atom x =>
        refine Conf.ret w fun v hv => ?_
        cases ha : evalAtom env x <;> simp [ha, Option.elim] at hv
        subst hv
        exact evalAtom_within hf ha
      | letR x ann v b =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        exact andThen_conf (ihR w env v hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))) fun w' x' _ hx =>
          ihE w' _ b (by simp only [Val.WithinEnvIn]; exact ⟨hx, hf⟩) he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
      | letE x ann v b =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        exact andThen_conf (ihE w env v hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))) fun w' x' _ hx =>
          ihE w' _ b (by simp only [Val.WithinEnvIn]; exact ⟨hx, hf⟩) he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
      | ite c t e =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        cases evalAtom env c with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | bool b =>
            cases b
            · exact ihE w env e hf he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
            · exact ihE w env t hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact Conf.stuck w
      | caseOption sc ne x se =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        cases hs : evalAtom env sc with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | none => exact ihE w env ne hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | some y =>
            have hy := evalAtom_within hf hs
            simp only [Val.WithinIn] at hy
            exact ihE w _ se (by simp only [Val.WithinEnvIn]; exact ⟨hy, hf⟩) he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact Conf.stuck w
      | caseResult sc x oe y ee =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        cases hs : evalAtom env sc with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | ok z =>
            have hz := evalAtom_within hf hs
            simp only [Val.WithinIn] at hz
            exact ihE w _ oe (by simp only [Val.WithinEnvIn]; exact ⟨hz, hf⟩) he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | err z =>
            have hz := evalAtom_within hf hs
            simp only [Val.WithinIn] at hz
            exact ihE w _ ee (by simp only [Val.WithinEnvIn]; exact ⟨hz, hf⟩)
              he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact Conf.stuck w
      | fail c => exact Conf.halt w (fun _ h => nomatch h) (fun _ _ _ h => nomatch h)
      | raise kind code args =>
        simp only [eval]
        cases ha : evalAtoms env args with
        | none => exact Conf.stuck w
        | some vs =>
          exact ⟨by simp, (fun _ h => nomatch h), (fun _ _ _ h => by cases h; exact evalAtoms_within hf ha)⟩
      | unreachable => exact Conf.halt w (fun _ h => nomatch h) (fun _ _ _ h => nomatch h)
      | withH o f b =>
        simp only [Expr.within] at he
        simp only [eval]
        exact ihE w _ b (by simp only [Val.WithinEnvIn]; exact ⟨by simp [Val.WithinIn], hf⟩) he
          (fun g hg => hc g (by simpa [Expr.callees] using hg))
      | caseSum sc i xs t e =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        cases hs : evalAtom env sc with
        | none => exact Conf.stuck w
        | some v =>
          cases v with
          | variant k j fs =>
            have hfs := evalAtom_within hf hs
            simp only [Val.WithinIn] at hfs
            dsimp only
            split
            · split
              · exact ihE w _ t (zip_within xs hfs hf) he.1
                  (fun g hg => hc g (by simp [Expr.callees, hg]))
              · exact Conf.stuck w
            · exact ihE w env e hf he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact Conf.stuck w
      | attemptK kind b =>
        simp only [Expr.within] at he
        simp only [eval]
        have h₂ := ihE w env b hf he hc
        generalize eval H n p w env b = res at h₂ ⊢
        obtain ⟨t, r, w'⟩ := res
        obtain ⟨ht, hv, hr⟩ := h₂
        cases r with
        | ok x => exact ⟨ht, (fun _ h => by cases h; simp only [Val.WithinIn]; exact hv x rfl),
            (fun _ _ _ h => nomatch h)⟩
        | raised k c ps =>
          dsimp only
          split
          · exact ⟨ht, (fun _ h => by cases h; simp only [Val.WithinIn]; exact hr k c ps rfl),
              (fun _ _ _ h => nomatch h)⟩
          · exact ⟨ht, (fun _ h => nomatch h), hr⟩
        | failed c => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
        | stuck | outOfFuel | aborted => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
      | handleK kind b x h =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        have h₂ := ihE w env b hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
        generalize eval H n p w env b = res at h₂ ⊢
        obtain ⟨t, r, w'⟩ := res
        obtain ⟨ht, hv, hr⟩ := h₂
        cases r with
        | raised k c ps =>
          dsimp only
          split
          · have g₂ := ihE w' ((x, .variant k c ps) :: env) h
              (by simp only [Val.WithinEnvIn, Val.WithinIn]; exact ⟨hr k c ps rfl, hf⟩) he.2
              (fun g hg => hc g (by simp [Expr.callees, hg]))
            generalize eval H n p w' ((x, .variant k c ps) :: env) h = res' at g₂ ⊢
            obtain ⟨t', r', w''⟩ := res'
            obtain ⟨ht', hv', hr'⟩ := g₂
            refine ⟨?_, hv', hr'⟩
            intro ev hev
            rcases List.mem_append.mp hev with h | h
            · exact ht ev h
            · exact ht' ev h
          · exact ⟨ht, (fun _ h => nomatch h), hr⟩
        | ok v => exact ⟨ht, (fun _ h' => by cases h'; exact hv v rfl), (fun _ _ _ h' => nomatch h')⟩
        | failed c => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
        | stuck | outOfFuel | aborted => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
      | attempt b =>
        simp only [Expr.within] at he
        simp only [eval]
        have h₂ := ihE w env b hf he hc
        generalize eval H n p w env b = res at h₂ ⊢
        obtain ⟨t, r, w'⟩ := res
        obtain ⟨ht, hv, hr⟩ := h₂
        cases r with
        | ok x => exact ⟨ht, (fun _ h => by cases h; simp only [Val.WithinIn]; exact hv x rfl),
            (fun _ _ _ h => nomatch h)⟩
        | failed c => exact ⟨ht, (fun _ h => by cases h; simp [Val.WithinIn]), (fun _ _ _ h => nomatch h)⟩
        | raised k c ps => exact ⟨ht, (fun _ h => nomatch h), hr⟩
        | stuck | outOfFuel | aborted => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
      | handle b x h =>
        simp only [Expr.within, Bool.and_eq_true] at he
        simp only [eval]
        have h₂ := ihE w env b hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
        generalize eval H n p w env b = res at h₂ ⊢
        obtain ⟨t, r, w'⟩ := res
        obtain ⟨ht, hv, hr⟩ := h₂
        cases r with
        | failed c =>
          have g₂ := ihE w' ((x, .int c) :: env) h
            (by simp only [Val.WithinEnvIn, Val.WithinIn]; exact ⟨trivial, hf⟩) he.2
            (fun g hg => hc g (by simp [Expr.callees, hg]))
          simp only []
          generalize eval H n p w' ((x, .int c) :: env) h = res' at g₂ ⊢
          obtain ⟨t', r', w''⟩ := res'
          obtain ⟨ht', hv', hr'⟩ := g₂
          refine ⟨?_, hv', hr'⟩
          intro ev hev
          rcases List.mem_append.mp hev with h | h
          · exact ht ev h
          · exact ht' ev h
        | ok v => exact ⟨ht, (fun _ h => by cases h; exact hv v rfl), (fun _ _ _ h => nomatch h)⟩
        | raised k c ps => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => by cases h; exact hr k c ps rfl)⟩
        | stuck | outOfFuel | aborted => exact ⟨ht, (fun _ h => nomatch h), (fun _ _ _ h => nomatch h)⟩
      | letTuple bs v b =>
        simp only [Expr.within] at he
        simp only [eval]
        cases hv : evalAtom env v with
        | none => exact Conf.stuck w
        | some x =>
          cases x with
          | tuple vs =>
            simp only
            split
            · have hvs := evalAtom_within hf hv
              simp only [Val.WithinIn] at hvs
              exact ihE w _ b (bindParams_within bs hvs hf) he
                (fun g hg => hc g (by simpa [Expr.callees] using hg))
            · exact Conf.stuck w
          | _ => exact Conf.stuck w
      | loop src x xt st init b =>
        simp only [Expr.within] at he
        simp only [eval]
        cases hs : sourceVals env src with
        | none => exact Conf.stuck w
        | some items =>
          cases hi : evalAtoms env init with
          | none => exact Conf.stuck w
          | some stv =>
            simp only
            split
            · have hitems : Val.WithinAllIn S a items := by
                cases src with
                | list xs =>
                  simp only [sourceVals] at hs
                  split at hs
                  · rename_i its hx
                    cases hs
                    have := evalAtom_within hf hx
                    simpa [Val.WithinIn] using this
                  · cases hs
                | range lo hi' inc =>
                  simp only [sourceVals] at hs
                  split at hs
                  · cases hs
                    rw [Val.withinAll_iff]
                    intro v hv
                    simp only [rangeVals, List.mem_map] at hv
                    obtain ⟨_, _, rfl⟩ := hv
                    simp [Val.WithinIn]
                  · cases hs
              exact walkLoop_conf (fun w' v st' hv hst => ihE w' _ b
                  (by simp only [Val.WithinEnvIn]; exact ⟨hv, bindParams_within st hst hf⟩) he
                  (fun g hg => hc g (by simpa [Expr.callees] using hg)))
                w stv items (evalAtoms_within hf hi) hitems
            · exact Conf.stuck w

/-- **The four forms of ADR 0093's rule 3.** Each is a run of a function of `S`, or of a value
within `a`, so each is confined:
- a call into the boundary module, by `call` or `invoke`, or an invocation that dispatches to a
  handler declared there (B5): the run of the function's body (`body_confined`);
- the module's function used as a value (B4): `funRef` gives a closure within `a`
  (`funRef_within`), and every `apply` of it, wherever it happens, is confined (`apply_confined`);
- a functional argument (A1): a value of the environment, which must be within `a`. -/
theorem body_confined (hH : H.RespTyped) (hp : p.ConfinedAt S a) {f : String} {fn : Fun}
    (hfd : p.find f = some fn) (hS : S f) {vs : List Val} (hvs : Val.WithinAllIn S a vs)
    (n : Nat) (w : World H) :
    ∀ ev ∈ (eval H n p w (bindParams fn.params vs []) fn.body).1, a.allows ev :=
  have hfn := hp.funs f fn hfd hS
  ((eval_confined hH hp n).2 w _ _ (bindParams_within (env := []) _ hvs trivial) hfn.1 hfn.2).1

theorem funRef_within (hp : p.ConfinedAt S a) {f : String} {fn : Fun} (hfd : p.find f = some fn)
    (hS : S f) (cenv : Env) (hc : Val.WithinEnvIn S a cenv) :
    Val.WithinIn S a (.closure fn.params fn.body cenv) := by
  have hfn := hp.funs f fn hfd hS
  simp only [Val.WithinIn]
  exact ⟨hfn.1, hfn.2, hc⟩

theorem apply_confined (hH : H.RespTyped) (hp : p.ConfinedAt S a) {fv : Val} {vs : List Val}
    (hfv : Val.WithinIn S a fv) (hvs : Val.WithinAllIn S a vs) (n : Nat) (w : World H) :
    ∀ ev ∈ (applyWith (eval H n p) w fv vs).1, a.allows ev :=
  (applyWith_conf (fun env e he hw hc => (eval_confined hH hp n).2 w env e he hw hc) hfv hvs).1

end Runs

/-! ### The check -/

/-- The functions an invocation of `o` may dispatch to (`Program.handlerFor`): the operation's
representative, and every handler of `o`. -/
def Program.handlersOf (p : Program) (o : String) : List String :=
  (p.opRep.lookup o).getD "" :: (p.handlerOf.filter (·.2 == o)).map (·.1)

theorem lookup_pair_mem {α β : Type} [BEq α] [LawfulBEq α] :
    ∀ {l : List (α × β)} {x : α} {y : β}, l.lookup x = some y → (x, y) ∈ l
  | [], _, _, h => by simp [List.lookup] at h
  | (k, v) :: l, x, y, h => by
    simp only [List.lookup] at h
    split at h
    · rename_i hk
      cases h
      have : x = k := by simpa using hk
      subst this
      exact List.mem_cons_self
    · exact List.mem_cons_of_mem _ (lookup_pair_mem h)

theorem handlerFor_mem (p : Program) (o : String) (m : Option Val) :
    p.handlerFor o m ∈ p.handlersOf o := by
  unfold Program.handlerFor Program.handlersOf
  split
  · rename_i f
    split
    · rename_i hf
      refine List.mem_cons_of_mem _ (List.mem_map.mpr ⟨(f, o), ?_, rfl⟩)
      exact List.mem_filter.mpr ⟨lookup_pair_mem hf, by simp⟩
    · exact List.mem_cons_self
  · exact List.mem_cons_self

/-- The check of a call site's confinement, over the candidate set `S` the importer computes:
every function of `S` has a body within `a` and callees in `S`, and every handler an operation of
`a` may dispatch to is in `S`. Linear in `S`'s functions and their call edges. -/
def confinedB (p : Program) (a : Auth) (S : List String) : Bool :=
  S.all (fun f => match p.find f with
    | some fn => fn.body.within a && fn.body.callees.all (S.contains ·)
    | none => true) &&
  a.ops.all (fun o => (p.handlersOf o).all (S.contains ·))

theorem confinedB_sound {p : Program} {a : Auth} {S : List String} (h : confinedB p a S = true) :
    p.ConfinedAt (· ∈ S) a := by
  simp only [confinedB, Bool.and_eq_true, List.all_eq_true] at h
  obtain ⟨hf, ho⟩ := h
  refine ⟨fun f fn hfd hS => ?_, fun o hoa m => ?_⟩
  · have := hf f hS
    simp only [hfd, Bool.and_eq_true, List.all_eq_true, List.contains_iff_mem] at this
    exact ⟨this.1, fun g hg => this.2 g hg⟩
  · have := ho o (by simpa using hoa)
    simp only [List.contains_iff_mem] at this
    exact this _ (handlerFor_mem p o m)

/-- **Rejection by a run.** If some run of the callee from values within `a` performs an event the
ceiling does not allow, no candidate set holding the callee passes the check: the check is not
vacuous. -/
theorem confinedB_refutes {H : Host} (hH : H.RespTyped) {p : Program} {a : Auth} {f : String}
    {fn : Fun} (hfd : p.find f = some fn) {vs : List Val} {n : Nat} {w : World H} {ev : Event}
    (hev : ev ∈ (eval H n p w (bindParams fn.params vs []) fn.body).1) (hno : ¬ a.allows ev)
    (S : List String) (hS : f ∈ S) (hvs : Val.WithinAllIn (· ∈ S) a vs) : confinedB p a S = false := by
  cases h : confinedB p a S
  · rfl
  · exact absurd (body_confined hH (confinedB_sound h) hfd hS hvs n w ev hev) hno

end Una.F1
