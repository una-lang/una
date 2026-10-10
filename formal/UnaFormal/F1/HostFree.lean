/-
#459, the re-checker on any host: **a program without host operations runs the same on every
host**. `Files` and `Http` are the only state a run reads or changes (`World`), and only through
`read`, `write` and a request. A program none of whose functions performs one, run on a body that
performs none, from values whose closures perform none, gives on any host the run it gives on the
host without state (`Host.unit`), and leaves the world as it found it (`eval_hostFree`).

So T6 needs no premise on the host for such a program: `t6_release_hostFree` is
`t6_release_host` on any host, lawful or not (H1 and H2 are about reads and writes, which such a
run never performs), and `t6_release_erasure_obs_hostFree` carries it to the document's runs.
A program the re-checker certifies is one unless it uses `Files` (`Program.hostFreeB`, M5b): its
"accepted" then asks nothing of the host, not even H1, which the real host meets only from v0.3.5
(S-16). One that uses `Files` rests on a lawful host, and its note says so (`Import.withHost`).
-/
import UnaFormal.F1.Monomorph

namespace Una.F1

mutual
  /-- A right-hand side that performs no host operation, in its lambdas included. -/
  def Rhs.hostFree : Rhs → Bool
    | .read _ | .write _ _ | .req _ _ | .envGet _ => false
    | .lambda _ b => b.hostFree
    | .atom _ | .prim _ _ | .call _ _ _ | .apply _ _ | .op _ | .construct _ _ | .field _ _
    | .list _ | .none | .some _ | .funRef _ _ | .declassify _ _ | .map _ _ | .filter _ _
    | .firstWhere _ _ | .fold _ _ _ | .parse _ | .checked _ _ _ | .eq _ _ | .tuple _ | .append _ _ | .set _ _ _
    | .toText _ => true

  /-- A body that performs no host operation, in its lambdas included. -/
  def Expr.hostFree : Expr → Bool
    | .atom _ | .fail _ => true
    | .letR _ _ v b => v.hostFree && b.hostFree
    | .letE _ _ v b => v.hostFree && b.hostFree
    | .ite _ t e => t.hostFree && e.hostFree
    | .caseOption _ n _ s => n.hostFree && s.hostFree
    | .caseResult _ _ o _ e => o.hostFree && e.hostFree
    | .attempt b => b.hostFree
    | .handle b _ h => b.hostFree && h.hostFree
    | .letTuple _ _ b | .loop _ _ _ _ _ b => b.hostFree
end

mutual
  /-- A value whose closures perform no host operation and call only functions in `S`. -/
  def Val.HostFreeIn (S : String → Prop) : Val → Prop
    | .list items => Val.HostFreeAllIn S items
    | .some v => Val.HostFreeIn S v
    | .ok v => Val.HostFreeIn S v
    | .record _ fs | .tuple fs => Val.HostFreeAllIn S fs
    | .closure _ b env => b.hostFree = true ∧ (∀ g ∈ b.callees, S g) ∧ Val.HostFreeEnvIn S env
    | .int _ | .bool _ | .text _ | .unit | .none | .err _ => True

  def Val.HostFreeAllIn (S : String → Prop) : List Val → Prop
    | [] => True
    | v :: vs => Val.HostFreeIn S v ∧ Val.HostFreeAllIn S vs

  def Val.HostFreeEnvIn (S : String → Prop) : List (String × Val) → Prop
    | [] => True
    | (_, v) :: env => Val.HostFreeIn S v ∧ Val.HostFreeEnvIn S env
end

/-- A value whose closures perform no host operation (`S`: every function). -/
abbrev Val.HostFree : Val → Prop := Val.HostFreeIn fun _ => True
abbrev Val.HostFreeAll : List Val → Prop := Val.HostFreeAllIn fun _ => True
abbrev Val.HostFreeEnv : List (String × Val) → Prop := Val.HostFreeEnvIn fun _ => True

/-- A program none of whose functions performs a host operation. -/
def Program.HostFree (p : Program) : Prop :=
  ∀ f fn, p.find f = some fn → fn.body.hostFree = true

/-- `Program.HostFree`, as a Boolean. -/
def Program.hostFreeB (p : Program) : Bool := p.funs.all (·.body.hostFree)

theorem Program.hostFreeB_sound {p : Program} (h : p.hostFreeB = true) : p.HostFree := by
  intro f fn hf
  unfold Program.find at hf
  exact List.all_eq_true.mp h fn (List.mem_of_find?_eq_some hf)

section Values

variable {S : String → Prop}

theorem Val.hostFreeAll_iff : ∀ (vs : List Val), Val.HostFreeAllIn S vs ↔ ∀ v ∈ vs, Val.HostFreeIn S v
  | [] => by simp [Val.HostFreeAllIn]
  | v :: vs => by simp [Val.HostFreeAllIn, Val.hostFreeAll_iff vs]

theorem lookup_hostFree : ∀ {env : Env} {x : String} {v : Val}, Val.HostFreeEnvIn S env →
    lookup env x = some v → Val.HostFreeIn S v
  | [], _, _, _, h => by simp [lookup] at h
  | (y, u) :: env, x, v, hf, h => by
    simp only [Val.HostFreeEnvIn] at hf
    simp only [lookup] at h
    split at h
    · cases h; exact hf.1
    · exact lookup_hostFree hf.2 h

theorem evalAtom_hostFree {env : Env} (hf : Val.HostFreeEnvIn S env) :
    ∀ {a : Atom} {v : Val}, evalAtom env a = some v → Val.HostFreeIn S v
  | .var _, _, h => lookup_hostFree hf h
  | .int _, _, h | .bool _, _, h | .text _, _, h | .unit, _, h => by
    simp only [evalAtom, Option.some.injEq] at h
    subst h
    simp [Val.HostFreeIn]

theorem evalAtoms_hostFree {env : Env} (hf : Val.HostFreeEnvIn S env) :
    ∀ {as : List Atom} {vs : List Val}, evalAtoms env as = some vs → Val.HostFreeAllIn S vs
  | [], vs, h => by
    simp only [evalAtoms, Option.some.injEq] at h
    subst h
    simp [Val.HostFreeAllIn]
  | a :: as, vs, h => by
    simp only [evalAtoms] at h
    cases ha : evalAtom env a <;> cases has : evalAtoms env as <;> simp [ha, has] at h
    subst h
    simp only [Val.HostFreeAllIn]
    exact ⟨evalAtom_hostFree hf ha, evalAtoms_hostFree hf has⟩

theorem evalPrim_hostFree {pr : Prim} {vs : List Val} {v : Val} (h : evalPrim pr vs = some v) :
    Val.HostFreeIn S v := by
  rcases vs with _ | ⟨a, _ | ⟨b, _ | ⟨c, rest⟩⟩⟩ <;> cases pr <;> (try cases a) <;> (try cases b) <;>
    simp [evalPrim] at h <;> (try obtain ⟨_, _, rfl⟩ := h) <;> (try subst h) <;> simp [Val.HostFreeIn]

theorem bindParams_hostFree {α : Type} : ∀ (ps : List (String × α)) {vs : List Val} {env : Env},
    Val.HostFreeAllIn S vs → Val.HostFreeEnvIn S env → Val.HostFreeEnvIn S (bindParams ps vs env)
  | [], _, _, _, he => by simpa [bindParams] using he
  | _ :: _, [], _, _, he => by simpa [bindParams] using he
  | _ :: ps, v :: vs, env, hv, he => by
    simp only [Val.HostFreeAllIn] at hv
    have := bindParams_hostFree ps hv.2 he
    simp only [bindParams, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
    simp only [Val.HostFreeEnvIn]
    exact ⟨hv.1, this⟩

end Values

section Runs

variable {H : Host} {S : String → Prop}

/-- A run on `H` that is `r'`, the run on the host without state, in the world `w`, with a value
whose closures perform no host operation. -/
def SameOn (S : String → Prop) (w : World H) (r : Run H) (r' : Run Host.unit) : Prop :=
  r = (r'.1, r'.2.1, w) ∧ ∀ v, r'.2.1 = .ok v → Val.HostFreeIn S v

theorem SameOn.stuck (w : World H) (u : World Host.unit) : SameOn S w (stuck w) (stuck u) :=
  ⟨rfl, fun _ h => nomatch h⟩

theorem SameOn.done (w : World H) (u : World Host.unit) {v : Val} (hv : Val.HostFreeIn S v) :
    SameOn S w (done w v) (done u v) :=
  ⟨rfl, fun _ h => by cases h; exact hv⟩

theorem SameOn.ret (w : World H) (u : World Host.unit) {r : Res}
    (hr : ∀ v, r = .ok v → Val.HostFreeIn S v) : SameOn S w ([], r, w) ([], r, u) :=
  ⟨rfl, hr⟩

theorem andThen_same {w : World H} {r : Run H} {r' : Run Host.unit} {k : World H → Val → Run H}
    {k' : World Host.unit → Val → Run Host.unit} (h : SameOn S w r r')
    (hk : ∀ u v, r'.2.1 = .ok v → Val.HostFreeIn S v → SameOn S w (k w v) (k' u v)) :
    SameOn S w (andThen r k) (andThen r' k') := by
  obtain ⟨rfl, hv⟩ := h
  obtain ⟨t, res, u⟩ := r'
  cases res with
  | ok v =>
    obtain ⟨h₁, h₂⟩ := hk u v rfl (hv v rfl)
    simp only [andThen]
    rw [h₁]
    generalize k' u v = kr at h₂
    obtain ⟨t', r'', u'⟩ := kr
    exact ⟨rfl, h₂⟩
  | failed _ | stuck | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩

theorem applyWith_same {run : World H → Env → Expr → Run H}
    {run' : World Host.unit → Env → Expr → Run Host.unit} {w : World H}
    (hrun : ∀ u env e, Val.HostFreeEnvIn S env → e.hostFree = true → (∀ g ∈ e.callees, S g) →
      SameOn S w (run w env e) (run' u env e))
    (u : World Host.unit) {fv : Val} {vs : List Val} (hfv : Val.HostFreeIn S fv)
    (hvs : Val.HostFreeAllIn S vs) : SameOn S w (applyWith run w fv vs) (applyWith run' u fv vs) := by
  cases fv with
  | closure ps b cenv =>
    simp only [Val.HostFreeIn] at hfv
    simp only [applyWith]
    split
    · exact hrun u _ _ (bindParams_hostFree ps hvs hfv.2.2) hfv.1 hfv.2.1
    · exact SameOn.stuck w u
  | _ => exact SameOn.stuck w u

theorem walkMap_same {w : World H} {app : World H → Val → Run H}
    {app' : World Host.unit → Val → Run Host.unit}
    (happ : ∀ u v, Val.HostFreeIn S v → SameOn S w (app w v) (app' u v)) :
    ∀ (u : World Host.unit) (vs : List Val), Val.HostFreeAllIn S vs →
      SameOn S w (walkMap app w vs) (walkMap app' u vs)
  | u, [], _ => SameOn.done w u (by simp [Val.HostFreeIn, Val.HostFreeAllIn])
  | u, v :: vs, hvs => by
    simp only [Val.HostFreeAllIn] at hvs
    simp only [walkMap]
    refine andThen_same (happ u v hvs.1) fun u' r _ hr =>
      andThen_same (walkMap_same happ u' vs hvs.2) fun u'' rest _ hrest => ?_
    cases rest with
    | list rs =>
      refine SameOn.done w u'' ?_
      simp only [Val.HostFreeIn, Val.HostFreeAllIn] at hrest ⊢
      exact ⟨hr, hrest⟩
    | _ => exact SameOn.stuck w u''

theorem walkFilter_same {w : World H} {app : World H → Val → Run H}
    {app' : World Host.unit → Val → Run Host.unit}
    (happ : ∀ u v, Val.HostFreeIn S v → SameOn S w (app w v) (app' u v)) :
    ∀ (u : World Host.unit) (vs : List Val), Val.HostFreeAllIn S vs →
      SameOn S w (walkFilter app w vs) (walkFilter app' u vs)
  | u, [], _ => SameOn.done w u (by simp [Val.HostFreeIn, Val.HostFreeAllIn])
  | u, v :: vs, hvs => by
    simp only [Val.HostFreeAllIn] at hvs
    simp only [walkFilter]
    refine andThen_same (happ u v hvs.1) fun u' b _ _ => ?_
    cases b with
    | bool keep =>
      refine andThen_same (walkFilter_same happ u' vs hvs.2) fun u'' rest _ hrest => ?_
      cases rest with
      | list rs =>
        refine SameOn.done w u'' ?_
        simp only [Val.HostFreeIn] at hrest ⊢
        cases keep
        · exact hrest
        · simp only [ite_true, Val.HostFreeAllIn]
          exact ⟨hvs.1, hrest⟩
      | _ => exact SameOn.stuck w u''
    | _ => exact SameOn.stuck w u'

theorem walkFirst_same {w : World H} {app : World H → Val → Run H}
    {app' : World Host.unit → Val → Run Host.unit}
    (happ : ∀ u v, Val.HostFreeIn S v → SameOn S w (app w v) (app' u v)) :
    ∀ (u : World Host.unit) (vs : List Val), Val.HostFreeAllIn S vs →
      SameOn S w (walkFirst app w vs) (walkFirst app' u vs)
  | u, [], _ => SameOn.done w u (by simp [Val.HostFreeIn])
  | u, v :: vs, hvs => by
    simp only [Val.HostFreeAllIn] at hvs
    simp only [walkFirst]
    refine andThen_same (happ u v hvs.1) fun u' b _ _ => ?_
    cases b with
    | bool keep =>
      cases keep
      · exact walkFirst_same happ u' vs hvs.2
      · exact SameOn.done w u' (by simp only [Val.HostFreeIn]; exact hvs.1)
    | _ => exact SameOn.stuck w u'

theorem walkFold_same {w : World H} {app : World H → Val → Val → Run H}
    {app' : World Host.unit → Val → Val → Run Host.unit}
    (happ : ∀ u a v, Val.HostFreeIn S a → Val.HostFreeIn S v → SameOn S w (app w a v) (app' u a v)) :
    ∀ (u : World Host.unit) (acc : Val) (vs : List Val), Val.HostFreeIn S acc → Val.HostFreeAllIn S vs →
      SameOn S w (walkFold app w acc vs) (walkFold app' u acc vs)
  | u, acc, [], hacc, _ => SameOn.done w u hacc
  | u, acc, v :: vs, hacc, hvs => by
    simp only [Val.HostFreeAllIn] at hvs
    simp only [walkFold]
    exact andThen_same (happ u acc v hacc hvs.1) fun u' acc' _ hacc' =>
      walkFold_same happ u' acc' vs hacc' hvs.2

theorem SameOn.abort (w : World H) (u : World Host.unit) : SameOn S w (abort w) (abort u) :=
  ⟨rfl, fun _ h => nomatch h⟩

theorem walkLoop_same {w : World H} {body : World H → Val → List Val → Run H}
    {body' : World Host.unit → Val → List Val → Run Host.unit}
    (hb : ∀ u v st, Val.HostFreeIn S v → Val.HostFreeAllIn S st → SameOn S w (body w v st) (body' u v st)) :
    ∀ (u : World Host.unit) (st vs : List Val), Val.HostFreeAllIn S st → Val.HostFreeAllIn S vs →
      SameOn S w (walkLoop body w st vs) (walkLoop body' u st vs)
  | u, st, [], hst, _ => SameOn.done w u (by simp only [Val.HostFreeIn]; exact hst)
  | u, st, v :: vs, hst, hvs => by
    simp only [Val.HostFreeAllIn] at hvs
    simp only [walkLoop]
    refine andThen_same (hb u v st hvs.1 hst) fun u' r _ hr => ?_
    cases r with
    | tuple st' =>
      simp only [Val.HostFreeIn] at hr
      simp only
      split
      · exact walkLoop_same hb u' st' vs hr hvs.2
      · exact SameOn.stuck w u'
    | _ => exact SameOn.stuck w u'

theorem parseInt_same (w : World H) (u : World Host.unit) (t : String) :
    SameOn S w (parseInt w t) (parseInt u t) := by
  unfold parseInt
  cases t.toInt? with
  | some n => exact SameOn.done w u (by simp [Val.HostFreeIn])
  | none => exact ⟨rfl, fun _ h => nomatch h⟩

/-- `checked` (#453) performs no host operation: its run is the same on any host. -/
theorem checkRun_same {p : Program} (w : World H) (u : World Host.unit) {v : Val} {pred : String}
    {code : Nat} (hv : Val.HostFreeIn S v) :
    SameOn S w (checkRun p w v pred code) (checkRun p u v pred code) := by
  unfold checkRun
  split
  · split
    · exact SameOn.done w u hv
    · exact ⟨rfl, fun _ h => nomatch h⟩
  · exact SameOn.stuck w u

variable {p : Program}

/-- **A program without host operations runs the same on every host.** A run of a body that
performs no host operation, from values whose closures perform none, in a program none of whose
functions performs one, is on any host `H` the run on the host without state, and leaves the
world as it found it; its value's closures perform no host operation either. -/
theorem eval_hostFreeIn
    (hp : ∀ f fn, p.find f = some fn → S f → fn.body.hostFree = true ∧ ∀ g ∈ fn.body.callees, S g)
    (hv : ∀ pol po, p.policy pol = some po → S po.via) : ∀ n : Nat,
    (∀ (w : World H) (u : World Host.unit) env r, Val.HostFreeEnvIn S env → r.hostFree = true →
      (∀ g ∈ r.callees, S g) →
      SameOn S w (evalRhs H n p w env r) (evalRhs Host.unit n p u env r)) ∧
    (∀ (w : World H) (u : World Host.unit) env e, Val.HostFreeEnvIn S env → e.hostFree = true →
      (∀ g ∈ e.callees, S g) →
      SameOn S w (eval H n p w env e) (eval Host.unit n p u env e))
  | 0 => ⟨fun _ _ _ _ _ _ _ => ⟨rfl, fun _ h => nomatch h⟩,
      fun _ _ _ _ _ _ _ => ⟨rfl, fun _ h => nomatch h⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := eval_hostFreeIn hp hv n
    constructor
    · intro w u env r hf hr hc
      have happ : ∀ (w' : World H), w' = w → ∀ u fv vs, Val.HostFreeIn S fv → Val.HostFreeAllIn S vs →
          SameOn S w (applyWith (eval H n p) w' fv vs) (applyWith (eval Host.unit n p) u fv vs) :=
        fun w' hw' u _ _ hfv hvs => hw' ▸ applyWith_same (ihE w) u hfv hvs
      cases r with
      | atom a =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        exact evalAtom_hostFree hf ha
      | prim pr args =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtoms env args with
        | none => simp [ha, Option.elim] at hv
        | some vs =>
          cases hpr : evalPrim pr vs with
          | none => simp [ha, hpr, Option.elim] at hv
          | some v' =>
            simp [ha, hpr, Option.elim] at hv
            subst hv
            exact evalPrim_hostFree hpr
      | call f targs args =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact SameOn.stuck w u
        | some fn =>
          cases ha : evalAtoms env args with
          | none => exact SameOn.stuck w u
          | some vs =>
            simp only []
            split
            · have hfn := hp f fn hfd (hc f (by simp [Rhs.callees]))
              exact ihE w u _ _ (bindParams_hostFree _ (evalAtoms_hostFree hf ha) trivial) hfn.1 hfn.2
            · exact SameOn.stuck w u
      | apply f args =>
        simp only [evalRhs]
        cases hfa : evalAtom env f with
        | none => exact SameOn.stuck w u
        | some fv =>
          cases ha : evalAtoms env args with
          | none => exact SameOn.stuck w u
          | some vs =>
            exact happ w rfl u fv vs (evalAtom_hostFree hf hfa) (evalAtoms_hostFree hf ha)
      | op line =>
        simp only [evalRhs]
        cases evalAtom env line with
        | none => exact SameOn.stuck w u
        | some v =>
          dsimp only
          cases v.line with
          | none => exact SameOn.stuck w u
          | some s => exact ⟨rfl, fun _ h => by cases h; simp [Val.HostFreeIn]⟩
      | read _ | write _ _ | req _ _ => simp [Rhs.hostFree] at hr
      | construct record fields =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtoms env fields <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFreeIn]
        exact evalAtoms_hostFree hf ha
      | field value index =>
        simp only [evalRhs]
        cases hv : evalAtom env value with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | record r fs =>
            have hfs := evalAtom_hostFree hf hv
            simp only [Val.HostFreeIn] at hfs
            refine SameOn.ret w u fun x hx => ?_
            cases hi : fs[index]? <;> simp [hi, Option.elim] at hx
            subst hx
            exact (Val.hostFreeAll_iff fs).mp hfs _ (List.mem_of_getElem? hi)
          | _ => exact SameOn.stuck w u
      | list items =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFreeIn]
        exact evalAtoms_hostFree hf ha
      | none => exact SameOn.done w u (by simp [Val.HostFreeIn])
      | some a =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFreeIn]
        exact evalAtom_hostFree hf ha
      | lambda params body =>
        refine SameOn.done w u ?_
        simp only [Val.HostFreeIn]
        exact ⟨by simpa [Rhs.hostFree] using hr, by simpa [Rhs.callees] using hc, hf⟩
      | funRef f targs =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact SameOn.stuck w u
        | some fn =>
          refine SameOn.done w u ?_
          simp only [Val.HostFreeIn]
          have hfn := hp f fn hfd (hc f (by simp [Rhs.callees]))
          exact ⟨hfn.1, hfn.2, trivial⟩
      | declassify pol a =>
        simp only [evalRhs]
        cases hpo : p.policy pol with
        | none => exact SameOn.stuck w u
        | some po =>
          cases hav : evalAtom env a with
          | none => exact SameOn.stuck w u
          | some v =>
            dsimp only
            cases hfd : p.find po.via with
            | none => exact SameOn.stuck w u
            | some fn =>
              simp only []
              split
              · obtain ⟨h₁, h₂⟩ := ihE w u (bindParams fn.params [v] []) fn.body
                  (bindParams_hostFree _ (by
                    simp only [Val.HostFreeAllIn]; exact ⟨evalAtom_hostFree hf hav, trivial⟩) trivial)
                  (hp po.via fn hfd (hv pol po hpo)).1 (hp po.via fn hfd (hv pol po hpo)).2
                rw [h₁]
                generalize eval Host.unit n p u (bindParams fn.params [v] []) fn.body = res at h₂
                obtain ⟨t, r, u'⟩ := res
                cases r with
                | ok x => exact ⟨rfl, fun _ h => by cases h; exact h₂ x rfl⟩
                | failed _ | stuck | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩
              · exact SameOn.stuck w u
      | map xs f =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact SameOn.stuck w u
        | some xv =>
          cases hfv : evalAtom env f with
          | none => cases xv <;> exact SameOn.stuck w u
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_hostFree hf hx
              simp only [Val.HostFreeIn] at hi
              exact walkMap_same (fun u v hv => happ w rfl u fv [v] (evalAtom_hostFree hf hfv)
                (by simp only [Val.HostFreeAllIn]; exact ⟨hv, trivial⟩)) u items hi
            | _ => exact SameOn.stuck w u
      | filter xs keep =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact SameOn.stuck w u
        | some xv =>
          cases hfv : evalAtom env keep with
          | none => cases xv <;> exact SameOn.stuck w u
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_hostFree hf hx
              simp only [Val.HostFreeIn] at hi
              exact walkFilter_same (fun u v hv => happ w rfl u fv [v] (evalAtom_hostFree hf hfv)
                (by simp only [Val.HostFreeAllIn]; exact ⟨hv, trivial⟩)) u items hi
            | _ => exact SameOn.stuck w u
      | firstWhere xs keep =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact SameOn.stuck w u
        | some xv =>
          cases hfv : evalAtom env keep with
          | none => cases xv <;> exact SameOn.stuck w u
          | some fv =>
            cases xv with
            | list items =>
              have hi := evalAtom_hostFree hf hx
              simp only [Val.HostFreeIn] at hi
              exact walkFirst_same (fun u v hv => happ w rfl u fv [v] (evalAtom_hostFree hf hfv)
                (by simp only [Val.HostFreeAllIn]; exact ⟨hv, trivial⟩)) u items hi
            | _ => exact SameOn.stuck w u
      | fold xs init f =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact SameOn.stuck w u
        | some xv =>
          cases hacc : evalAtom env init with
          | none => cases xv <;> exact SameOn.stuck w u
          | some acc =>
            cases hfv : evalAtom env f with
            | none => cases xv <;> exact SameOn.stuck w u
            | some fv =>
              cases xv with
              | list items =>
                have hi := evalAtom_hostFree hf hx
                simp only [Val.HostFreeIn] at hi
                exact walkFold_same (fun u a v ha hv => happ w rfl u fv [a, v]
                  (evalAtom_hostFree hf hfv)
                  (by simp only [Val.HostFreeAllIn]; exact ⟨ha, hv, trivial⟩)) u acc items
                  (evalAtom_hostFree hf hacc) hi
              | _ => exact SameOn.stuck w u
      | parse s =>
        simp only [evalRhs]
        cases evalAtom env s with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | text t => exact parseInt_same w u t
          | _ => exact SameOn.stuck w u
      | checked a pred code =>
        simp only [evalRhs]
        cases ha : evalAtom env a with
        | none => exact SameOn.stuck w u
        | some v => exact checkRun_same w u (evalAtom_hostFree hf ha)
      | eq a b =>
        simp only [evalRhs]
        cases evalAtom env a with
        | none => exact SameOn.stuck w u
        | some va =>
          cases evalAtom env b with
          | none => exact SameOn.stuck w u
          | some vb => exact SameOn.done w u (by simp [Val.HostFreeIn])
      | toText a =>
        simp only [evalRhs]
        cases evalAtom env a with
        | none => exact SameOn.stuck w u
        | some v => exact SameOn.done w u (by simp [Val.HostFreeIn])
      | envGet _ => simp [Rhs.hostFree] at hr
      | tuple items =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFreeIn]
        exact evalAtoms_hostFree hf ha
      | append xs item =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact SameOn.stuck w u
        | some xv =>
          cases hi : evalAtom env item with
          | none => cases xv <;> exact SameOn.stuck w u
          | some v =>
            cases xv with
            | list items =>
              have hl := evalAtom_hostFree hf hx
              have hv := evalAtom_hostFree hf hi
              simp only [Val.HostFreeIn, Val.hostFreeAll_iff] at hl ⊢
              refine SameOn.done w u ?_
              simp only [Val.HostFreeIn, Val.hostFreeAll_iff, List.mem_append, List.mem_singleton]
              rintro y (hy | rfl)
              · exact hl y hy
              · exact hv
            | _ => exact SameOn.stuck w u
      | set xs index item =>
        simp only [evalRhs]
        cases hx : evalAtom env xs with
        | none => exact SameOn.stuck w u
        | some xv =>
          cases evalAtom env index with
          | none => cases xv <;> exact SameOn.stuck w u
          | some iv =>
            cases hi : evalAtom env item with
            | none => cases xv <;> cases iv <;> exact SameOn.stuck w u
            | some v =>
              cases xv with
              | list items =>
                cases iv with
                | int i =>
                  simp only
                  split
                  · have hl := evalAtom_hostFree hf hx
                    have hv := evalAtom_hostFree hf hi
                    simp only [Val.HostFreeIn, Val.hostFreeAll_iff] at hl
                    refine SameOn.done w u ?_
                    simp only [Val.HostFreeIn, Val.HostFreeAllIn, Val.hostFreeAll_iff, and_true]
                    intro y hy
                    rcases List.mem_or_eq_of_mem_set hy with hy | rfl
                    · exact hl y hy
                    · exact hv
                  · exact SameOn.abort w u
                | _ => exact SameOn.stuck w u
              | _ => cases iv <;> exact SameOn.stuck w u
    · intro w u env e hf he hc
      cases e with
      | atom a =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        exact evalAtom_hostFree hf ha
      | letR x ann v b =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        exact andThen_same (ihR w u env v hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))) fun u' x' _ hx =>
          ihE w u' _ b (by simp only [Val.HostFreeEnvIn]; exact ⟨hx, hf⟩) he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
      | letE x ann v b =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        exact andThen_same (ihE w u env v hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))) fun u' x' _ hx =>
          ihE w u' _ b (by simp only [Val.HostFreeEnvIn]; exact ⟨hx, hf⟩) he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
      | ite c t e =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        cases evalAtom env c with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | bool b =>
            cases b
            · exact ihE w u env e hf he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
            · exact ihE w u env t hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact SameOn.stuck w u
      | caseOption sc ne x se =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        cases hs : evalAtom env sc with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | none => exact ihE w u env ne hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | some y =>
            have hy := evalAtom_hostFree hf hs
            simp only [Val.HostFreeIn] at hy
            exact ihE w u _ se (by simp only [Val.HostFreeEnvIn]; exact ⟨hy, hf⟩) he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact SameOn.stuck w u
      | caseResult sc x oe y ee =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        cases hs : evalAtom env sc with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | ok z =>
            have hz := evalAtom_hostFree hf hs
            simp only [Val.HostFreeIn] at hz
            exact ihE w u _ oe (by simp only [Val.HostFreeEnvIn]; exact ⟨hz, hf⟩) he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | err c =>
            exact ihE w u _ ee (by simp only [Val.HostFreeEnvIn, Val.HostFreeIn]; exact ⟨trivial, hf⟩)
              he.2 (fun g hg => hc g (by simp [Expr.callees, hg]))
          | _ => exact SameOn.stuck w u
      | fail c => exact ⟨rfl, fun _ h => nomatch h⟩
      | attempt b =>
        simp only [Expr.hostFree] at he
        simp only [eval]
        obtain ⟨h₁, h₂⟩ := ihE w u env b hf he hc
        rw [h₁]
        generalize eval Host.unit n p u env b = res at h₂
        obtain ⟨t, r, u'⟩ := res
        cases r with
        | ok x => exact ⟨rfl, fun _ h => by cases h; simp only [Val.HostFreeIn]; exact h₂ x rfl⟩
        | failed c => exact ⟨rfl, fun _ h => by cases h; simp [Val.HostFreeIn]⟩
        | stuck | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩
      | handle b x h =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        obtain ⟨h₁, _⟩ := ihE w u env b hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))
        rw [h₁]
        generalize hres : eval Host.unit n p u env b = res
        obtain ⟨t, r, u'⟩ := res
        cases r with
        | failed c =>
          obtain ⟨g₁, g₂⟩ := ihE w u' ((x, .int c) :: env) h
            (by simp only [Val.HostFreeEnvIn, Val.HostFreeIn]; exact ⟨trivial, hf⟩) he.2
            (fun g hg => hc g (by simp [Expr.callees, hg]))
          simp only []
          rw [g₁]
          generalize eval Host.unit n p u' ((x, .int c) :: env) h = res' at g₂
          obtain ⟨t', r', u''⟩ := res'
          exact ⟨rfl, g₂⟩
        | ok v =>
          have := (ihE w u env b hf he.1 (fun g hg => hc g (by simp [Expr.callees, hg]))).2 v (by rw [hres])
          exact ⟨rfl, fun _ h => by cases h; exact this⟩
        | stuck | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩
      | letTuple bs v b =>
        simp only [Expr.hostFree] at he
        simp only [eval]
        cases hv : evalAtom env v with
        | none => exact SameOn.stuck w u
        | some x =>
          cases x with
          | tuple vs =>
            simp only
            split
            · have hvs := evalAtom_hostFree hf hv
              simp only [Val.HostFreeIn] at hvs
              exact ihE w u _ b (bindParams_hostFree bs hvs hf) he
                (fun g hg => hc g (by simpa [Expr.callees] using hg))
            · exact SameOn.stuck w u
          | _ => exact SameOn.stuck w u
      | loop src x xt st init b =>
        simp only [Expr.hostFree] at he
        simp only [eval]
        cases hs : sourceVals env src with
        | none => exact SameOn.stuck w u
        | some items =>
          cases hi : evalAtoms env init with
          | none => exact SameOn.stuck w u
          | some stv =>
            simp only
            split
            · have hitems : Val.HostFreeAllIn S items := by
                cases src with
                | list xs =>
                  simp only [sourceVals] at hs
                  split at hs
                  · rename_i its hx
                    cases hs
                    have := evalAtom_hostFree hf hx
                    simpa [Val.HostFreeIn] using this
                  · cases hs
                | range lo hi' inc =>
                  simp only [sourceVals] at hs
                  split at hs
                  · cases hs
                    rw [Val.hostFreeAll_iff]
                    intro v hv
                    simp only [rangeVals, List.mem_map] at hv
                    obtain ⟨_, _, rfl⟩ := hv
                    simp [Val.HostFreeIn]
                  · cases hs
              exact walkLoop_same (fun u' v st' hv hst => ihE w u' _ b
                  (by simp only [Val.HostFreeEnvIn]; exact ⟨hv, bindParams_hostFree st hst hf⟩) he
                  (fun g hg => hc g (by simpa [Expr.callees] using hg)))
                u stv items (evalAtoms_hostFree hf hi) hitems
            · exact SameOn.stuck w u

/-- **A program without host operations runs the same on every host**: `eval_hostFreeIn` with
every function in the set. -/
theorem eval_hostFree (hp : p.HostFree) : ∀ n : Nat,
    (∀ (w : World H) (u : World Host.unit) env r, Val.HostFreeEnv env → r.hostFree = true →
      SameOn (fun _ => True) w (evalRhs H n p w env r) (evalRhs Host.unit n p u env r)) ∧
    (∀ (w : World H) (u : World Host.unit) env e, Val.HostFreeEnv env → e.hostFree = true →
      SameOn (fun _ => True) w (eval H n p w env e) (eval Host.unit n p u env e)) := fun n =>
  let h := eval_hostFreeIn (S := fun _ => True) (H := H) (fun f fn hf _ => ⟨hp f fn hf, fun _ _ => trivial⟩)
    (fun _ _ _ => trivial) n
  ⟨fun w u env r hf hr => h.1 w u env r hf hr (fun _ _ => trivial),
    fun w u env e hf he => h.2 w u env e hf he (fun _ _ => trivial)⟩

end Runs

section T6

variable {p : Program} {L : Label}

/-- **T6 with declassification on any host, for code that reaches no host operation** (#472).
The conclusion of `t6_release_host`, without its premise that the host is lawful, for a body whose
reach, a set `S` of functions closed under the calls and references of their bodies, `via`
functions included, holds no host operation, run from inputs whose closures reach only `S`: such a
run never reads or writes a file, so H1 and H2 are never used, and it is the run on the lawful host
without state (`eval_hostFreeIn`). The rest of the program may use `Files`. -/
theorem t6_release_hostFreeIn {S : String → Prop} (hwf : p.LWellFormed) (hb1 : p.B1WellFormed)
    (hh : p.HostWF)
    (hp : ∀ f fn, p.find f = some fn → S f → fn.body.hostFree = true ∧ ∀ g ∈ fn.body.callees, S g)
    (hv : ∀ pol po, p.policy pol = some po → S po.via)
    {H : Host} {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env}
    {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}
    (hσ : GroundSubst σ) (hE : p.EqArgsOK σ E) (hΓ : Γ.InPlace) (hpc : pc.PlainFree)
    (hw : WorldRel p L w₁ w₂) (h₁ : LEnvTy p e₁ (Γ.lsub p.recordDeep σ))
    (h₂ : LEnvTy p e₂ (Γ.lsub p.recordDeep σ)) (hrel : EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂)
    (he : LExprTy p E Γ pc e t ε) (hf₁ : Val.HostFreeEnvIn S e₁) (hf₂ : Val.HostFreeEnvIn S e₂)
    (hef : e.hostFree = true) (hec : ∀ g ∈ e.callees, S g) (n : Nat) :
    BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
      (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e) := by
  have hU := t6_release_host p L Host.unit hwf hb1 hh Host.unit_lawful hσ hE hΓ hpc
    (WorldRel.refl (⟨(), (), fun _ => none⟩ : World Host.unit)) h₁ h₂ hrel he n
  rw [((eval_hostFreeIn hp hv n).2 w₁ ⟨(), (), fun _ => none⟩ e₁ e hf₁ hef hec).1,
    ((eval_hostFreeIn hp hv n).2 w₂ ⟨(), (), fun _ => none⟩ e₂ e hf₂ hef hec).1]
  intro t₁ t₂
  rcases hU t₁ t₂ with ⟨hpj, ho, _⟩ | hd
  · exact .inl ⟨hpj, ho, hw⟩
  · exact .inr hd

/-- **T6 with declassification on any host, for a program without host operations.** The
conclusion of `t6_release_host`, without its premise that the host is lawful: a run of such a
program never reads or writes a file, so H1 and H2 are never used, and the run is the one on the
lawful host without state (`eval_hostFree`). The real host before v0.3.5, which broke H1 (S-16),
is one such host. -/
theorem t6_release_hostFree (hwf : p.LWellFormed) (hb1 : p.B1WellFormed) (hh : p.HostWF)
    (hp : p.HostFree) {H : Host} {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env}
    {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}
    (hσ : GroundSubst σ) (hE : p.EqArgsOK σ E) (hΓ : Γ.InPlace) (hpc : pc.PlainFree)
    (hw : WorldRel p L w₁ w₂) (h₁ : LEnvTy p e₁ (Γ.lsub p.recordDeep σ))
    (h₂ : LEnvTy p e₂ (Γ.lsub p.recordDeep σ)) (hrel : EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂)
    (he : LExprTy p E Γ pc e t ε) (hf₁ : Val.HostFreeEnv e₁) (hf₂ : Val.HostFreeEnv e₂)
    (hef : e.hostFree = true) (n : Nat) :
    BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
      (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e) :=
  t6_release_hostFreeIn (S := fun _ => True) hwf hb1 hh
    (fun f fn hf _ => ⟨hp f fn hf, fun _ _ => trivial⟩) (fun _ _ _ => trivial) hσ hE hΓ hpc hw h₁ h₂
    hrel he hf₁ hf₂ hef (fun _ _ => trivial) n

/-- **What the observer sees of the document's runs, on any host, for code that reaches no host
operation** (#472): `t6_release_erasure_obs` without its premises on the host, for a body whose
reach `S` in the document's program holds no host operation, from inputs whose closures reach only
`S`. The rest of the document may use `Files`. -/
theorem t6_release_erasure_obs_hostFreeIn {S : String → Prop} {ρ : String → String}
    {P₁ P₂ : Program} (L : Label)
    (hsim : Sim ρ P₁ P₂) (hproj : ∀ t, proj P₁ L t = proj P₂ L t)
    (hwf : P₁.LWellFormed) (hb1 : P₁.B1WellFormed) (hh : P₁.HostWF)
    (hp₂ : ∀ f fn, P₂.find f = some fn → S f → fn.body.hostFree = true ∧ ∀ g ∈ fn.body.callees, S g)
    (hv₂ : ∀ pol po, P₂.policy pol = some po → S po.via)
    {H : Host} {w₁ w₂ : World H}
    {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
    {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : P₁.EqArgsOK σ E) (hΓ : Γ.InPlace)
    (hpc : pc.PlainFree) (h₁ : LEnvTy P₁ e₁ (Γ.lsub P₁.recordDeep σ))
    (h₂ : LEnvTy P₁ e₂ (Γ.lsub P₁.recordDeep σ)) (hrel : EnvRel P₁ L (Γ.lsub P₁.recordDeep σ) e₁ e₂)
    (he : LExprTy P₁ E Γ pc e t ε) {e' : Expr} {e₁' e₂' : Env} (hee : e.anon ρ = e'.anon id)
    (he₁ : Val.anonEnv ρ e₁ = Val.anonEnv id e₁') (he₂ : Val.anonEnv ρ e₂ = Val.anonEnv id e₂')
    (hef : e'.hostFree = true) (hec : ∀ g ∈ e'.callees, S g)
    (hf₁ : Val.HostFreeEnvIn S e₁') (hf₂ : Val.HostFreeEnvIn S e₂')
    (n : Nat) (hT₁ : Term (eval H n P₂ w₁ e₁' e').2.1) (hT₂ : Term (eval H n P₂ w₂ e₂' e').2.1) :
    proj P₂ L (eval H n P₂ w₁ e₁' e').1 = proj P₂ L (eval H n P₂ w₂ e₂' e').1 ∨
      Diverged P₂ L (eval H n P₂ w₁ e₁' e').1 (eval H n P₂ w₂ e₂' e').1 := by
  have s₁ := ((eval_hostFreeIn (H := H) hp₂ hv₂ n).2 w₁ ⟨(), (), fun _ => none⟩ e₁' e' hf₁ hef hec).1
  have s₂ := ((eval_hostFreeIn (H := H) hp₂ hv₂ n).2 w₂ ⟨(), (), fun _ => none⟩ e₂' e' hf₂ hef hec).1
  rw [s₁] at hT₁ ⊢
  rw [s₂] at hT₂ ⊢
  exact t6_release_erasure_obs L hsim hproj hwf hb1 hh Host.unit_lawful Host.unit_anonInvariant hσ
    hE hΓ hpc (WorldRel.refl _) h₁ h₂ hrel he hee he₁ he₂ n hT₁ hT₂

/-- **What the observer sees of the document's runs, on any host.** `t6_release_erasure_obs`
without its premises on the host, for a document whose program, body and inputs perform no host
operation: the runs on any host are those on the host without state, which is lawful and reads no
annotation. -/
theorem t6_release_erasure_obs_hostFree {ρ : String → String} {P₁ P₂ : Program} (L : Label)
    (hsim : Sim ρ P₁ P₂) (hproj : ∀ t, proj P₁ L t = proj P₂ L t)
    (hwf : P₁.LWellFormed) (hb1 : P₁.B1WellFormed) (hh : P₁.HostWF) (hp₂ : P₂.HostFree)
    {H : Host} {w₁ w₂ : World H}
    {σ : List (String × LTy)} {e₁ e₂ : Env} {Γ : LCtx} {E : List String} {pc : LExpr} {e : Expr}
    {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : P₁.EqArgsOK σ E) (hΓ : Γ.InPlace)
    (hpc : pc.PlainFree) (h₁ : LEnvTy P₁ e₁ (Γ.lsub P₁.recordDeep σ))
    (h₂ : LEnvTy P₁ e₂ (Γ.lsub P₁.recordDeep σ)) (hrel : EnvRel P₁ L (Γ.lsub P₁.recordDeep σ) e₁ e₂)
    (he : LExprTy P₁ E Γ pc e t ε) {e' : Expr} {e₁' e₂' : Env} (hee : e.anon ρ = e'.anon id)
    (he₁ : Val.anonEnv ρ e₁ = Val.anonEnv id e₁') (he₂ : Val.anonEnv ρ e₂ = Val.anonEnv id e₂')
    (hef : e'.hostFree = true) (hf₁ : Val.HostFreeEnv e₁') (hf₂ : Val.HostFreeEnv e₂')
    (n : Nat) (hT₁ : Term (eval H n P₂ w₁ e₁' e').2.1) (hT₂ : Term (eval H n P₂ w₂ e₂' e').2.1) :
    proj P₂ L (eval H n P₂ w₁ e₁' e').1 = proj P₂ L (eval H n P₂ w₂ e₂' e').1 ∨
      Diverged P₂ L (eval H n P₂ w₁ e₁' e').1 (eval H n P₂ w₂ e₂' e').1 :=
  t6_release_erasure_obs_hostFreeIn (S := fun _ => True) L hsim hproj hwf hb1 hh
    (fun f fn hf _ => ⟨hp₂ f fn hf, fun _ _ => trivial⟩) (fun _ _ _ => trivial) hσ hE hΓ hpc h₁ h₂
    hrel he hee he₁ he₂ hef (fun _ _ => trivial) hf₁ hf₂ n hT₁ hT₂

end T6

end Una.F1
