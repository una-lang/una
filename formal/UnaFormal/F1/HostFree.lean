/-
#459, the re-checker on any host: **a program without host operations runs the same on every
host**. `Files` and `Http` are the only state a run reads or changes (`World`), and only through
`read`, `write` and a request. A program none of whose functions performs one, run on a body that
performs none, from values whose closures perform none, gives on any host the run it gives on the
host without state (`Host.unit`), and leaves the world as it found it (`eval_hostFree`).

So T6 needs no premise on the host for such a program: `t6_release_hostFree` is
`t6_release_host` on any host, lawful or not (H1 and H2 are about reads and writes, which such a
run never performs), and `t6_release_erasure_obs_hostFree` carries it to the document's runs.
Every program the re-checker certifies is one (`Import.finalize` checks `Program.hostFreeB`), so
its "accepted" asks nothing of the host, not even H1, which the real host meets only from v0.3.5
(S-16).
-/
import UnaFormal.F1.Monomorph

namespace Una.F1

mutual
  /-- A right-hand side that performs no host operation, in its lambdas included. -/
  def Rhs.hostFree : Rhs → Bool
    | .read _ | .write _ _ | .req _ _ => false
    | .lambda _ b => b.hostFree
    | .atom _ | .prim _ _ | .call _ _ _ | .apply _ _ | .op _ | .construct _ _ | .field _ _
    | .list _ | .none | .some _ | .funRef _ _ | .declassify _ _ | .map _ _ | .filter _ _
    | .firstWhere _ _ | .fold _ _ _ | .parse _ | .eq _ _ => true

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
end

mutual
  /-- A value whose closures perform no host operation. -/
  def Val.HostFree : Val → Prop
    | .list items => Val.HostFreeAll items
    | .some v => v.HostFree
    | .ok v => v.HostFree
    | .record _ fs => Val.HostFreeAll fs
    | .closure _ b env => b.hostFree = true ∧ Val.HostFreeEnv env
    | .int _ | .bool _ | .text _ | .unit | .none | .err _ => True

  def Val.HostFreeAll : List Val → Prop
    | [] => True
    | v :: vs => v.HostFree ∧ Val.HostFreeAll vs

  def Val.HostFreeEnv : List (String × Val) → Prop
    | [] => True
    | (_, v) :: env => v.HostFree ∧ Val.HostFreeEnv env
end

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

theorem Val.hostFreeAll_iff : ∀ (vs : List Val), Val.HostFreeAll vs ↔ ∀ v ∈ vs, v.HostFree
  | [] => by simp [Val.HostFreeAll]
  | v :: vs => by simp [Val.HostFreeAll, Val.hostFreeAll_iff vs]

theorem lookup_hostFree : ∀ {env : Env} {x : String} {v : Val}, Val.HostFreeEnv env →
    lookup env x = some v → v.HostFree
  | [], _, _, _, h => by simp [lookup] at h
  | (y, u) :: env, x, v, hf, h => by
    simp only [Val.HostFreeEnv] at hf
    simp only [lookup] at h
    split at h
    · cases h; exact hf.1
    · exact lookup_hostFree hf.2 h

theorem evalAtom_hostFree {env : Env} (hf : Val.HostFreeEnv env) :
    ∀ {a : Atom} {v : Val}, evalAtom env a = some v → v.HostFree
  | .var _, _, h => lookup_hostFree hf h
  | .int _, _, h | .bool _, _, h | .text _, _, h | .unit, _, h => by
    simp only [evalAtom, Option.some.injEq] at h
    subst h
    simp [Val.HostFree]

theorem evalAtoms_hostFree {env : Env} (hf : Val.HostFreeEnv env) :
    ∀ {as : List Atom} {vs : List Val}, evalAtoms env as = some vs → Val.HostFreeAll vs
  | [], vs, h => by
    simp only [evalAtoms, Option.some.injEq] at h
    subst h
    simp [Val.HostFreeAll]
  | a :: as, vs, h => by
    simp only [evalAtoms] at h
    cases ha : evalAtom env a <;> cases has : evalAtoms env as <;> simp [ha, has] at h
    subst h
    simp only [Val.HostFreeAll]
    exact ⟨evalAtom_hostFree hf ha, evalAtoms_hostFree hf has⟩

theorem evalPrim_hostFree {pr : Prim} {vs : List Val} {v : Val} (h : evalPrim pr vs = some v) :
    v.HostFree := by
  rcases vs with _ | ⟨a, _ | ⟨b, _ | ⟨c, rest⟩⟩⟩ <;> cases pr <;> (try cases a) <;> (try cases b) <;>
    simp [evalPrim] at h <;> (try obtain ⟨_, _, rfl⟩ := h) <;> (try subst h) <;> simp [Val.HostFree]

theorem bindParams_hostFree {α : Type} : ∀ (ps : List (String × α)) {vs : List Val} {env : Env},
    Val.HostFreeAll vs → Val.HostFreeEnv env → Val.HostFreeEnv (bindParams ps vs env)
  | [], _, _, _, he => by simpa [bindParams] using he
  | _ :: _, [], _, _, he => by simpa [bindParams] using he
  | _ :: ps, v :: vs, env, hv, he => by
    simp only [Val.HostFreeAll] at hv
    have := bindParams_hostFree ps hv.2 he
    simp only [bindParams, List.map_cons, List.zip_cons_cons, List.cons_append] at this ⊢
    simp only [Val.HostFreeEnv]
    exact ⟨hv.1, this⟩

end Values

section Runs

variable {H : Host}

/-- A run on `H` that is `r'`, the run on the host without state, in the world `w`, with a value
whose closures perform no host operation. -/
def SameOn (w : World H) (r : Run H) (r' : Run Host.unit) : Prop :=
  r = (r'.1, r'.2.1, w) ∧ ∀ v, r'.2.1 = .ok v → v.HostFree

theorem SameOn.stuck (w : World H) (u : World Host.unit) : SameOn w (stuck w) (stuck u) :=
  ⟨rfl, fun _ h => nomatch h⟩

theorem SameOn.done (w : World H) (u : World Host.unit) {v : Val} (hv : v.HostFree) :
    SameOn w (done w v) (done u v) :=
  ⟨rfl, fun _ h => by cases h; exact hv⟩

theorem SameOn.ret (w : World H) (u : World Host.unit) {r : Res}
    (hr : ∀ v, r = .ok v → v.HostFree) : SameOn w ([], r, w) ([], r, u) :=
  ⟨rfl, hr⟩

theorem andThen_same {w : World H} {r : Run H} {r' : Run Host.unit} {k : World H → Val → Run H}
    {k' : World Host.unit → Val → Run Host.unit} (h : SameOn w r r')
    (hk : ∀ u v, r'.2.1 = .ok v → v.HostFree → SameOn w (k w v) (k' u v)) :
    SameOn w (andThen r k) (andThen r' k') := by
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
    (hrun : ∀ u env e, Val.HostFreeEnv env → e.hostFree = true →
      SameOn w (run w env e) (run' u env e))
    (u : World Host.unit) {fv : Val} {vs : List Val} (hfv : fv.HostFree)
    (hvs : Val.HostFreeAll vs) : SameOn w (applyWith run w fv vs) (applyWith run' u fv vs) := by
  cases fv with
  | closure ps b cenv =>
    simp only [Val.HostFree] at hfv
    simp only [applyWith]
    split
    · exact hrun u _ _ (bindParams_hostFree ps hvs hfv.2) hfv.1
    · exact SameOn.stuck w u
  | _ => exact SameOn.stuck w u

theorem walkMap_same {w : World H} {app : World H → Val → Run H}
    {app' : World Host.unit → Val → Run Host.unit}
    (happ : ∀ u v, v.HostFree → SameOn w (app w v) (app' u v)) :
    ∀ (u : World Host.unit) (vs : List Val), Val.HostFreeAll vs →
      SameOn w (walkMap app w vs) (walkMap app' u vs)
  | u, [], _ => SameOn.done w u (by simp [Val.HostFree, Val.HostFreeAll])
  | u, v :: vs, hvs => by
    simp only [Val.HostFreeAll] at hvs
    simp only [walkMap]
    refine andThen_same (happ u v hvs.1) fun u' r _ hr =>
      andThen_same (walkMap_same happ u' vs hvs.2) fun u'' rest _ hrest => ?_
    cases rest with
    | list rs =>
      refine SameOn.done w u'' ?_
      simp only [Val.HostFree, Val.HostFreeAll] at hrest ⊢
      exact ⟨hr, hrest⟩
    | _ => exact SameOn.stuck w u''

theorem walkFilter_same {w : World H} {app : World H → Val → Run H}
    {app' : World Host.unit → Val → Run Host.unit}
    (happ : ∀ u v, v.HostFree → SameOn w (app w v) (app' u v)) :
    ∀ (u : World Host.unit) (vs : List Val), Val.HostFreeAll vs →
      SameOn w (walkFilter app w vs) (walkFilter app' u vs)
  | u, [], _ => SameOn.done w u (by simp [Val.HostFree, Val.HostFreeAll])
  | u, v :: vs, hvs => by
    simp only [Val.HostFreeAll] at hvs
    simp only [walkFilter]
    refine andThen_same (happ u v hvs.1) fun u' b _ _ => ?_
    cases b with
    | bool keep =>
      refine andThen_same (walkFilter_same happ u' vs hvs.2) fun u'' rest _ hrest => ?_
      cases rest with
      | list rs =>
        refine SameOn.done w u'' ?_
        simp only [Val.HostFree] at hrest ⊢
        cases keep
        · exact hrest
        · simp only [ite_true, Val.HostFreeAll]
          exact ⟨hvs.1, hrest⟩
      | _ => exact SameOn.stuck w u''
    | _ => exact SameOn.stuck w u'

theorem walkFirst_same {w : World H} {app : World H → Val → Run H}
    {app' : World Host.unit → Val → Run Host.unit}
    (happ : ∀ u v, v.HostFree → SameOn w (app w v) (app' u v)) :
    ∀ (u : World Host.unit) (vs : List Val), Val.HostFreeAll vs →
      SameOn w (walkFirst app w vs) (walkFirst app' u vs)
  | u, [], _ => SameOn.done w u (by simp [Val.HostFree])
  | u, v :: vs, hvs => by
    simp only [Val.HostFreeAll] at hvs
    simp only [walkFirst]
    refine andThen_same (happ u v hvs.1) fun u' b _ _ => ?_
    cases b with
    | bool keep =>
      cases keep
      · exact walkFirst_same happ u' vs hvs.2
      · exact SameOn.done w u' (by simp only [Val.HostFree]; exact hvs.1)
    | _ => exact SameOn.stuck w u'

theorem walkFold_same {w : World H} {app : World H → Val → Val → Run H}
    {app' : World Host.unit → Val → Val → Run Host.unit}
    (happ : ∀ u a v, a.HostFree → v.HostFree → SameOn w (app w a v) (app' u a v)) :
    ∀ (u : World Host.unit) (acc : Val) (vs : List Val), acc.HostFree → Val.HostFreeAll vs →
      SameOn w (walkFold app w acc vs) (walkFold app' u acc vs)
  | u, acc, [], hacc, _ => SameOn.done w u hacc
  | u, acc, v :: vs, hacc, hvs => by
    simp only [Val.HostFreeAll] at hvs
    simp only [walkFold]
    exact andThen_same (happ u acc v hacc hvs.1) fun u' acc' _ hacc' =>
      walkFold_same happ u' acc' vs hacc' hvs.2

theorem parseInt_same (w : World H) (u : World Host.unit) (t : String) :
    SameOn w (parseInt w t) (parseInt u t) := by
  unfold parseInt
  cases t.toInt? with
  | some n => exact SameOn.done w u (by simp [Val.HostFree])
  | none => exact ⟨rfl, fun _ h => nomatch h⟩

variable {p : Program}

/-- **A program without host operations runs the same on every host.** A run of a body that
performs no host operation, from values whose closures perform none, in a program none of whose
functions performs one, is on any host `H` the run on the host without state, and leaves the
world as it found it; its value's closures perform no host operation either. -/
theorem eval_hostFree (hp : p.HostFree) : ∀ n : Nat,
    (∀ (w : World H) (u : World Host.unit) env r, Val.HostFreeEnv env → r.hostFree = true →
      SameOn w (evalRhs H n p w env r) (evalRhs Host.unit n p u env r)) ∧
    (∀ (w : World H) (u : World Host.unit) env e, Val.HostFreeEnv env → e.hostFree = true →
      SameOn w (eval H n p w env e) (eval Host.unit n p u env e))
  | 0 => ⟨fun _ _ _ _ _ _ => ⟨rfl, fun _ h => nomatch h⟩,
      fun _ _ _ _ _ _ => ⟨rfl, fun _ h => nomatch h⟩⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := eval_hostFree hp n
    constructor
    · intro w u env r hf hr
      have happ : ∀ (w' : World H), w' = w → ∀ u fv vs, fv.HostFree → Val.HostFreeAll vs →
          SameOn w (applyWith (eval H n p) w' fv vs) (applyWith (eval Host.unit n p) u fv vs) :=
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
            · exact ihE w u _ _ (bindParams_hostFree _ (evalAtoms_hostFree hf ha) trivial)
                (hp f fn hfd)
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
          | some s => exact ⟨rfl, fun _ h => by cases h; simp [Val.HostFree]⟩
      | read _ | write _ _ | req _ _ => simp [Rhs.hostFree] at hr
      | construct record fields =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtoms env fields <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFree]
        exact evalAtoms_hostFree hf ha
      | field value index =>
        simp only [evalRhs]
        cases hv : evalAtom env value with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | record r fs =>
            have hfs := evalAtom_hostFree hf hv
            simp only [Val.HostFree] at hfs
            refine SameOn.ret w u fun x hx => ?_
            cases hi : fs[index]? <;> simp [hi, Option.elim] at hx
            subst hx
            exact (Val.hostFreeAll_iff fs).mp hfs _ (List.mem_of_getElem? hi)
          | _ => exact SameOn.stuck w u
      | list items =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtoms env items <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFree]
        exact evalAtoms_hostFree hf ha
      | none => exact SameOn.done w u (by simp [Val.HostFree])
      | some a =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        simp only [Val.HostFree]
        exact evalAtom_hostFree hf ha
      | lambda params body =>
        refine SameOn.done w u ?_
        simp only [Val.HostFree]
        exact ⟨by simpa [Rhs.hostFree] using hr, hf⟩
      | funRef f targs =>
        simp only [evalRhs]
        cases hfd : p.find f with
        | none => exact SameOn.stuck w u
        | some fn =>
          refine SameOn.done w u ?_
          simp only [Val.HostFree]
          exact ⟨hp f fn hfd, trivial⟩
      | declassify pol a =>
        simp only [evalRhs]
        cases p.policy pol with
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
                    simp only [Val.HostFreeAll]; exact ⟨evalAtom_hostFree hf hav, trivial⟩) trivial)
                  (hp po.via fn hfd)
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
              simp only [Val.HostFree] at hi
              exact walkMap_same (fun u v hv => happ w rfl u fv [v] (evalAtom_hostFree hf hfv)
                (by simp only [Val.HostFreeAll]; exact ⟨hv, trivial⟩)) u items hi
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
              simp only [Val.HostFree] at hi
              exact walkFilter_same (fun u v hv => happ w rfl u fv [v] (evalAtom_hostFree hf hfv)
                (by simp only [Val.HostFreeAll]; exact ⟨hv, trivial⟩)) u items hi
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
              simp only [Val.HostFree] at hi
              exact walkFirst_same (fun u v hv => happ w rfl u fv [v] (evalAtom_hostFree hf hfv)
                (by simp only [Val.HostFreeAll]; exact ⟨hv, trivial⟩)) u items hi
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
                simp only [Val.HostFree] at hi
                exact walkFold_same (fun u a v ha hv => happ w rfl u fv [a, v]
                  (evalAtom_hostFree hf hfv)
                  (by simp only [Val.HostFreeAll]; exact ⟨ha, hv, trivial⟩)) u acc items
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
      | eq a b =>
        simp only [evalRhs]
        cases evalAtom env a with
        | none => exact SameOn.stuck w u
        | some va =>
          cases evalAtom env b with
          | none => exact SameOn.stuck w u
          | some vb => exact SameOn.done w u (by simp [Val.HostFree])
    · intro w u env e hf he
      cases e with
      | atom a =>
        refine SameOn.ret w u fun v hv => ?_
        cases ha : evalAtom env a <;> simp [ha, Option.elim] at hv
        subst hv
        exact evalAtom_hostFree hf ha
      | letR x ann v b =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        exact andThen_same (ihR w u env v hf he.1) fun u' x' _ hx =>
          ihE w u' _ b (by simp only [Val.HostFreeEnv]; exact ⟨hx, hf⟩) he.2
      | letE x ann v b =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        exact andThen_same (ihE w u env v hf he.1) fun u' x' _ hx =>
          ihE w u' _ b (by simp only [Val.HostFreeEnv]; exact ⟨hx, hf⟩) he.2
      | ite c t e =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        cases evalAtom env c with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | bool b =>
            cases b
            · exact ihE w u env e hf he.2
            · exact ihE w u env t hf he.1
          | _ => exact SameOn.stuck w u
      | caseOption sc ne x se =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        cases hs : evalAtom env sc with
        | none => exact SameOn.stuck w u
        | some v =>
          cases v with
          | none => exact ihE w u env ne hf he.1
          | some y =>
            have hy := evalAtom_hostFree hf hs
            simp only [Val.HostFree] at hy
            exact ihE w u _ se (by simp only [Val.HostFreeEnv]; exact ⟨hy, hf⟩) he.2
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
            simp only [Val.HostFree] at hz
            exact ihE w u _ oe (by simp only [Val.HostFreeEnv]; exact ⟨hz, hf⟩) he.1
          | err c =>
            exact ihE w u _ ee (by simp only [Val.HostFreeEnv, Val.HostFree]; exact ⟨trivial, hf⟩)
              he.2
          | _ => exact SameOn.stuck w u
      | fail c => exact ⟨rfl, fun _ h => nomatch h⟩
      | attempt b =>
        simp only [Expr.hostFree] at he
        simp only [eval]
        obtain ⟨h₁, h₂⟩ := ihE w u env b hf he
        rw [h₁]
        generalize eval Host.unit n p u env b = res at h₂
        obtain ⟨t, r, u'⟩ := res
        cases r with
        | ok x => exact ⟨rfl, fun _ h => by cases h; simp only [Val.HostFree]; exact h₂ x rfl⟩
        | failed c => exact ⟨rfl, fun _ h => by cases h; simp [Val.HostFree]⟩
        | stuck | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩
      | handle b x h =>
        simp only [Expr.hostFree, Bool.and_eq_true] at he
        simp only [eval]
        obtain ⟨h₁, _⟩ := ihE w u env b hf he.1
        rw [h₁]
        generalize hres : eval Host.unit n p u env b = res
        obtain ⟨t, r, u'⟩ := res
        cases r with
        | failed c =>
          obtain ⟨g₁, g₂⟩ := ihE w u' ((x, .int c) :: env) h
            (by simp only [Val.HostFreeEnv, Val.HostFree]; exact ⟨trivial, hf⟩) he.2
          simp only []
          rw [g₁]
          generalize eval Host.unit n p u' ((x, .int c) :: env) h = res' at g₂
          obtain ⟨t', r', u''⟩ := res'
          exact ⟨rfl, g₂⟩
        | ok v =>
          have := (ihE w u env b hf he.1).2 v (by rw [hres])
          exact ⟨rfl, fun _ h => by cases h; exact this⟩
        | stuck | outOfFuel => exact ⟨rfl, fun _ h => nomatch h⟩

end Runs

section T6

variable {p : Program} {L : Label}

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
      (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e) := by
  have hU := t6_release_host p L Host.unit hwf hb1 hh Host.unit_lawful hσ hE hΓ hpc
    (WorldRel.refl (⟨(), ()⟩ : World Host.unit)) h₁ h₂ hrel he n
  rw [((eval_hostFree hp n).2 w₁ ⟨(), ()⟩ e₁ e hf₁ hef).1,
    ((eval_hostFree hp n).2 w₂ ⟨(), ()⟩ e₂ e hf₂ hef).1]
  intro t₁ t₂
  rcases hU t₁ t₂ with ⟨hpj, ho, _⟩ | hd
  · exact .inl ⟨hpj, ho, hw⟩
  · exact .inr hd

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
    (n : Nat) (hT₁ : Term (eval H n P₂ w₁ e₁' e').2.1) (hT₂ : Term (eval H n P₂ w₂ e₂' e').2.1)
    (hfo₁ : ∀ ev ∈ (eval H n P₂ w₁ e₁' e').1, ev.FirstOrder)
    (hfo₂ : ∀ ev ∈ (eval H n P₂ w₂ e₂' e').1, ev.FirstOrder) :
    proj P₂ L (eval H n P₂ w₁ e₁' e').1 = proj P₂ L (eval H n P₂ w₂ e₂' e').1 ∨
      Diverged P₂ L (eval H n P₂ w₁ e₁' e').1 (eval H n P₂ w₂ e₂' e').1 := by
  rw [((eval_hostFree hp₂ n).2 w₁ ⟨(), ()⟩ e₁' e' hf₁ hef).1] at hT₁ hfo₁ ⊢
  rw [((eval_hostFree hp₂ n).2 w₂ ⟨(), ()⟩ e₂' e' hf₂ hef).1] at hT₂ hfo₂ ⊢
  exact t6_release_erasure_obs L hsim hproj hwf hb1 hh Host.unit_lawful Host.unit_anonInvariant hσ
    hE hΓ hpc (WorldRel.refl _) h₁ h₂ hrel he hee he₁ he₂ n hT₁ hT₂ hfo₁ hfo₂

end T6

end Una.F1
