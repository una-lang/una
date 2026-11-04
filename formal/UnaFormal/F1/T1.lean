/-
T1, type safety, for F1 (`spec/10 §10`, M1 of `docs/plan/formal-track.md`): a well-typed
expression of a well-formed program, in an environment of the types its context gives, never
gets stuck, whatever the host does. It ends with a value of its type, with a failure, or runs
out of fuel, which is the big-step form of "it is a value or a `fail`, or it steps" for
terminating runs.
-/
import UnaFormal.F1.Lemmas

namespace Una.F1

variable {p : Program} {H : Host}

/-- What T1 allows a run of type `t` to end with: a value of type `t`, a failure, or the end
of fuel; never a stuck evaluation. In a loop's body whose state has the types `ts` (`X = some
ts`), it may also end with an exit of the loop's iteration, with a state of those types (ADR 0116
rule 20); elsewhere never. -/
def GoodX (p : Program) (X : Option (List Ty)) (t : Ty) : Res → Prop
  | .ok v => ValTy p v t
  | .failed _ => True
  -- A failure with a payload (#512): the payload has its variant's field types, which a
  -- payload kind's `attempt` and handler give as a value.
  | .raised k c vs => ∃ sm ts, p.sum k = some sm ∧ sm.fieldTys c = some ts ∧ ValsTy p vs ts
  | .exit _ vs => ∃ ts, X = some ts ∧ ValsTy p vs ts
  | .stuck => False
  | .outOfFuel | .aborted => True

/-- What T1 allows a run outside a loop's body to end with (`GoodX` at `none`): no exit. -/
abbrev Good (p : Program) (t : Ty) : Res → Prop := GoodX p none t

/-- A run that ends well outside a loop's body ends well inside one. -/
theorem GoodX.of_none {X : Option (List Ty)} {t : Ty} {r : Res} (h : Good p t r) : GoodX p X t r := by
  cases r <;> simp_all [GoodX]

theorem andThen_goodX {X : Option (List Ty)} {first : Run H} {rest : World H → Val → Run H}
    {t s : Ty} (h1 : GoodX p X t first.2.1) (h2 : ∀ w v, ValTy p v t → GoodX p X s (rest w v).2.1) :
    GoodX p X s (andThen first rest).2.1 := by
  obtain ⟨tr, r, w⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    have := h2 w v h1
    revert this
    cases rest w v
    simp
  | failed code => simp [andThen, GoodX]
  | raised k c vs => simpa [andThen, GoodX] using h1
  | exit b vs => simpa [andThen, GoodX] using h1
  | stuck => simp [GoodX] at h1
  | outOfFuel | aborted => simp [andThen, GoodX]

theorem andThen_good {first : Run H} {rest : World H → Val → Run H} {t s : Ty}
    (h1 : Good p t first.2.1) (h2 : ∀ w v, ValTy p v t → Good p s (rest w v).2.1) :
    Good p s (andThen first rest).2.1 :=
  andThen_goodX h1 h2

/-- **A response has `Response`'s type** (H4), in a program that holds `Http`'s kinds. -/
theorem ValTy.response (hr : p.HttpRecords) {v : Val} (h : v.IsResponse) :
    ValTy p v (.record "prelude.Response") := by
  obtain ⟨⟨hrec, hh, hhf⟩, ⟨rrec, hrr, hrf⟩⟩ := hr
  unfold Val.IsResponse at h
  split at h
  · rename_i hs _ _
    refine .record hrr ?_
    rw [hrf]
    refine .cons .int (.cons (.list fun x hx => ?_) (.cons .text .nil))
    obtain ⟨n, y, rfl⟩ := h x hx
    exact .record hh (by rw [hhf]; exact .cons .text (.cons .text .nil))
  · exact h.elim

/-- A call of a typed function value on typed arguments, with a sound evaluation of bodies. -/
theorem applyWith_good {run : World H → Env → Expr → Run H} {fv : Val} {vs : List Val}
    {ts : List Ty} {r : Ty}
    (hrun : ∀ {w : World H} {env : Env} {ctx : Ctx} {e : Expr} {t : Ty},
      EnvTy p env ctx → ExprTy p ctx e t → Good p t (run w env e).2.1)
    (hf : ValTy p fv (.fn ts r)) (hvs : ValsTy p vs ts) (w : World H) :
    Good p r (applyWith run w fv vs).2.1 := by
  cases hf with
  | closure hcenv hlen hbody =>
    rw [applyWith, ite_eq_left ((valsTy_length hvs).trans hlen)]
    exact hrun (bindParams_typed hvs hlen hcenv) hbody

theorem walkMap_good {app : World H → Val → Run H} {a b : Ty}
    (happ : ∀ w v, ValTy p v a → Good p b (app w v).2.1) :
    ∀ {items : List Val} (w : World H), (∀ v ∈ items, ValTy p v a) →
      Good p (.list b) (walkMap app w items).2.1
  | [], _, _ => by simp only [walkMap, done, GoodX]; exact .list (by simp)
  | v :: vs, w, h => by
    simp only [walkMap]
    refine andThen_good (happ w v (h v List.mem_cons_self)) (fun w r hr => ?_)
    refine andThen_good (walkMap_good happ w (fun u hu => h u (List.mem_cons_of_mem _ hu)))
      (fun w rest hrest => ?_)
    cases hrest with
    | list hrs =>
      simp only [done, GoodX]
      exact .list (fun u hu => by
        rcases List.mem_cons.mp hu with rfl | hu
        · exact hr
        · exact hrs u hu)

theorem walkFilter_good {app : World H → Val → Run H} {a : Ty}
    (happ : ∀ w v, ValTy p v a → Good p .bool (app w v).2.1) :
    ∀ {items : List Val} (w : World H), (∀ v ∈ items, ValTy p v a) →
      Good p (.list a) (walkFilter app w items).2.1
  | [], _, _ => by simp only [walkFilter, done, GoodX]; exact .list (by simp)
  | v :: vs, w, h => by
    simp only [walkFilter]
    refine andThen_good (happ w v (h v List.mem_cons_self)) (fun w b hb => ?_)
    cases hb with
    | @bool keep =>
      refine andThen_good (walkFilter_good happ w (fun u hu => h u (List.mem_cons_of_mem _ hu)))
        (fun w rest hrest => ?_)
      cases hrest with
      | list hrs =>
        simp only [done, GoodX]
        cases keep
        · exact .list hrs
        · exact .list (fun u hu => by
            rcases List.mem_cons.mp (by simpa using hu) with rfl | hu
            · exact h u List.mem_cons_self
            · exact hrs u hu)

theorem walkFirst_good {app : World H → Val → Run H} {a : Ty}
    (happ : ∀ w v, ValTy p v a → Good p .bool (app w v).2.1) :
    ∀ {items : List Val} (w : World H), (∀ v ∈ items, ValTy p v a) →
      Good p (.option a) (walkFirst app w items).2.1
  | [], _, _ => by simp only [walkFirst, done, GoodX]; exact .none
  | v :: vs, w, h => by
    simp only [walkFirst]
    refine andThen_good (happ w v (h v List.mem_cons_self)) (fun w b hb => ?_)
    cases hb with
    | @bool keep =>
      cases keep
      · exact walkFirst_good happ w (fun u hu => h u (List.mem_cons_of_mem _ hu))
      · simp only [done, GoodX]
        exact .some (h v List.mem_cons_self)

theorem walkFold_good {app : World H → Val → Val → Run H} {a b : Ty}
    (happ : ∀ w acc v, ValTy p acc b → ValTy p v a → Good p b (app w acc v).2.1) :
    ∀ {items : List Val} (w : World H) {acc : Val}, ValTy p acc b → (∀ v ∈ items, ValTy p v a) →
      Good p b (walkFold app w acc items).2.1
  | [], _, _, hacc, _ => by simpa [walkFold, done, GoodX] using hacc
  | v :: vs, w, _, hacc, h => by
    simp only [walkFold]
    exact andThen_good (happ w _ v hacc (h v List.mem_cons_self))
      (fun w acc' hacc' => walkFold_good happ w hacc' (fun u hu => h u (List.mem_cons_of_mem _ hu)))

/-- How a loop's iterations may end before `Run.endLoop` (ADR 0116 rule 20): with a state of the
loop's types, a failure, a `stop` with such a state, or the end of fuel; never with a `skip`, which
`Run.iter` catches, nor stuck. -/
def LoopGood (p : Program) (ts : List Ty) : Res → Prop
  | .ok v => ValTy p v (.tuple ts)
  | .failed _ => True
  | .raised k c vs => ∃ sm ts, p.sum k = some sm ∧ sm.fieldTys c = some ts ∧ ValsTy p vs ts
  | .exit stop vs => stop = true ∧ ValsTy p vs ts
  | .stuck => False
  | .outOfFuel | .aborted => True

/-- An iteration of a typed body ends well for its loop once its `skip` is caught. -/
theorem iter_loopGood {ts : List Ty} {r : Run H} (h : GoodX p (some ts) (.tuple ts) r.2.1) :
    LoopGood p ts (Run.iter r).2.1 := by
  obtain ⟨tr, res, w⟩ := r
  cases res with
  | exit b vs =>
    obtain ⟨ts', hts, hvs⟩ := h
    cases hts
    cases b
    · exact .tuple hvs
    · exact ⟨rfl, hvs⟩
  | _ => simpa [Run.iter, LoopGood, GoodX] using h

theorem andThen_loopGood {ts : List Ty} {first : Run H} {rest : World H → Val → Run H}
    (h1 : LoopGood p ts first.2.1) (h2 : ∀ w v, ValTy p v (.tuple ts) → LoopGood p ts (rest w v).2.1) :
    LoopGood p ts (andThen first rest).2.1 := by
  obtain ⟨tr, r, w⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    have := h2 w v h1
    revert this
    cases rest w v
    simp
  | stuck => simp [LoopGood] at h1
  | _ => simpa [andThen, LoopGood] using h1

/-- The end of a loop's iterations: a typed state, its value, whatever code it is in. -/
theorem endLoop_good {X : Option (List Ty)} {ts : List Ty} {r : Run H} (h : LoopGood p ts r.2.1) :
    GoodX p X (.tuple ts) (Run.endLoop ts.length r).2.1 := by
  obtain ⟨tr, res, w⟩ := r
  cases res with
  | exit b vs =>
    obtain ⟨rfl, hvs⟩ := h
    simp only [Run.endLoop]
    rw [if_pos (valsTy_length hvs)]
    exact .tuple hvs
  | _ => simpa [Run.endLoop, LoopGood, GoodX] using h

/-- A loop whose body maps a typed state to a typed state ends with a typed state. -/
theorem walkLoop_good {body : World H → Val → List Val → Run H} {a : Ty} {ts : List Ty}
    (hbody : ∀ w v st, ValTy p v a → ValsTy p st ts → GoodX p (some ts) (.tuple ts) (body w v st).2.1) :
    ∀ {items : List Val} (w : World H) {st : List Val}, ValsTy p st ts →
      (∀ v ∈ items, ValTy p v a) → LoopGood p ts (walkLoop body w st items).2.1
  | [], _, _, hst, _ => by simp only [walkLoop, done, LoopGood]; exact .tuple hst
  | v :: vs, w, st, hst, h => by
    simp only [walkLoop]
    refine andThen_loopGood (iter_loopGood (hbody w v st (h v List.mem_cons_self) hst)) (fun w r hr => ?_)
    cases hr with
    | tuple hvals =>
      simp only
      rw [ite_eq_left ((valsTy_length hvals).trans (valsTy_length hst).symm)]
      exact walkLoop_good hbody w hvals (fun u hu => h u (List.mem_cons_of_mem _ hu))

/-- A run outside a loop's body, then one inside: an exit comes only from the second. -/
theorem andThen_loopGood_of {ts : List Ty} {t : Ty} {first : Run H} {rest : World H → Val → Run H}
    (h1 : Good p t first.2.1) (h2 : ∀ w v, ValTy p v t → LoopGood p ts (rest w v).2.1) :
    LoopGood p ts (andThen first rest).2.1 := by
  obtain ⟨tr, r, w⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    have := h2 w v h1
    revert this
    cases rest w v
    simp
  | stuck => simp [GoodX] at h1
  | exit => simp [GoodX] at h1
  | _ => simpa [andThen, LoopGood, GoodX] using h1

/-- `for until`: a typed condition and body keep the state typed. -/
theorem walkUntil_good {cond body : World H → List Val → Run H} {ts : List Ty}
    (hcond : ∀ w st, ValsTy p st ts → Good p .bool (cond w st).2.1)
    (hbody : ∀ w st, ValsTy p st ts → GoodX p (some ts) (.tuple ts) (body w st).2.1) :
    ∀ (k : Nat) (w : World H) {st : List Val}, ValsTy p st ts →
      LoopGood p ts (walkUntil cond body k w st).2.1
  | 0, _, _, _ => by simp [walkUntil, LoopGood]
  | k + 1, w, st, hst => by
    simp only [walkUntil]
    refine andThen_loopGood_of (hcond w st hst) (fun w c hc => ?_)
    cases hc with
    | @bool b =>
      cases b
      · simp only
        refine andThen_loopGood (iter_loopGood (hbody w st hst)) (fun w r hr => ?_)
        cases hr with
        | tuple hvals =>
          simp only
          rw [ite_eq_left ((valsTy_length hvals).trans (valsTy_length hst).symm)]
          exact walkUntil_good hcond hbody k w hvals
      · simp only [done, LoopGood]
        exact .tuple hst

/-- `for ever`: a typed body keeps the state typed, whatever the host's schedule. -/
theorem walkEver_good {stops : Nat → Bool} {body : World H → List Val → Run H} {ts : List Ty}
    (hbody : ∀ w st, ValsTy p st ts → GoodX p (some ts) (.tuple ts) (body w st).2.1) :
    ∀ (k i : Nat) (w : World H) {st : List Val}, ValsTy p st ts →
      LoopGood p ts (walkEver stops body k i w st).2.1
  | 0, _, _, _, _ => by simp [walkEver, LoopGood]
  | k + 1, i, w, st, hst => by
    simp only [walkEver]
    split
    · simp only [done, LoopGood]
      exact .tuple hst
    · refine andThen_loopGood (iter_loopGood (hbody w st hst)) (fun w r hr => ?_)
      cases hr with
      | tuple hvals =>
        simp only
        rw [ite_eq_left ((valsTy_length hvals).trans (valsTy_length hst).symm)]
        exact walkEver_good hbody k (i + 1) w hvals

/-- `on stop` after a `for ever`: a typed block on a typed state gives the loop's value. -/
theorem finishEver_good {X : Option (List Ty)} {k : World H → List Val → Run H} {ts : List Ty}
    {st₀ : List Val} {r : Run H}
    (hk : ∀ w st, ValsTy p st ts → Good p (.tuple ts) (k w st).2.1) (hst₀ : ValsTy p st₀ ts)
    (h : GoodX p X (.tuple ts) r.2.1) : GoodX p X (.tuple ts) (finishEver k st₀ r).2.1 := by
  obtain ⟨tr, res, w⟩ := r
  cases res with
  | ok v =>
    cases h with
    | tuple hvs =>
      have := hk w _ hvs
      simp only [finishEver]
      revert this
      obtain ⟨_, r', _⟩ := k w _
      exact GoodX.of_none
  | failed c =>
    have := hk w st₀ hst₀
    simp only [finishEver]
    revert this
    obtain ⟨_, r', _⟩ := k w st₀
    cases r' <;> simp [GoodX]
  | raised kd c ps =>
    have := hk w st₀ hst₀
    simp only [finishEver]
    revert this
    obtain ⟨_, r', _⟩ := k w st₀
    cases r' <;> simp_all [GoodX]
  | _ => simpa [finishEver] using h

theorem rangeVals_int (a b : Int) (inc : Bool) : ∀ v ∈ rangeVals a b inc, ValTy p v .int := by
  intro v hv
  simp only [rangeVals, List.mem_map] at hv
  obtain ⟨_, _, rfl⟩ := hv
  exact .int

/-- A typed source gives the loop typed elements. -/
theorem source_typed {env : Env} {ctx : Ctx} {s : Source} {t : Ty}
    (henv : EnvTy p env ctx) (h : SourceTy ctx s t) :
    ∃ items, sourceVals env s = some items ∧ ∀ v ∈ items, ValTy p v t := by
  cases h with
  | list hx =>
    obtain ⟨v, hv, hvt⟩ := atom_typed henv hx
    cases hvt with
    | list hitems => exact ⟨_, by simp [sourceVals, hv], hitems⟩
  | @range low high inc hl hh =>
    obtain ⟨u, hu, hut⟩ := atom_typed henv hl
    obtain ⟨v, hv, hvt⟩ := atom_typed henv hh
    cases hut with
    | @int a =>
      cases hvt with
      | @int b => exact ⟨rangeVals a b inc, by simp only [sourceVals, hu, hv], rangeVals_int _ _ _⟩

theorem parseInt_good (w : World H) (s : String) : Good p .int (parseInt w s).2.1 := by
  unfold parseInt
  split
  · exact .int
  · trivial

/-- The bindings of a `case` on a sum (#512): its fields, typed, in front of the environment. -/
theorem zip_typed {env : Env} {ctx : Ctx} (henv : EnvTy p env ctx) :
    ∀ {xs : List String} {vs : List Val} {ts : List Ty}, ValsTy p vs ts → ts.length = xs.length →
      EnvTy p (xs.zip vs ++ env) (xs.zip ts ++ ctx)
  | [], _, _, _, _ => by simpa using henv
  | _ :: _, _, _, .nil, h => by simp at h
  | _ :: _, _, _, .cons hv hvs, h => by
    simp only [List.zip_cons_cons, List.cons_append]
    exact .cons hv (zip_typed henv hvs (by simpa using h))

/-- A value typed by a sum's name is a variant (#512): no record has that name. -/
theorem not_record_of_sum (hwf : p.WellFormed) {k : String} {sm : Sum} {rec : Record}
    (hs : p.sum k = some sm) (hr : p.record k = some rec) : False := by
  rcases hwf.disjoint k with h | h <;> simp_all

theorem sound (hwf : p.WellFormed) (h4 : H.RespTyped) : ∀ n : Nat,
    (∀ {w : World H} {env : Env} {ctx : Ctx} {r : Rhs} {t : Ty},
      EnvTy p env ctx → RhsTy p ctx r t → Good p t (evalRhs H n p w env r).2.1) ∧
    (∀ {w : World H} {env : Env} {ctx : Ctx} {e : Expr} {t : Ty} {X : Option (List Ty)},
      EnvTy p env ctx → ExprTyX p X ctx e t → GoodX p X t (eval H n p w env e).2.1)
  | 0 => ⟨fun _ _ => by simp [evalRhs, GoodX], fun _ _ => by simp [eval, GoodX]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := sound hwf h4 n
    constructor
    · intro w env ctx r t henv hr
      cases hr with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, GoodX, hvt]
      | prim has hp =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        obtain ⟨v, hv, hvt⟩ := prim_typed hp hvst
        simp only [evalRhs, hvs, hv, Option.map_some, Option.getD_some, Option.bind_some,
          Option.elim]
        split <;> simp [GoodX, hvt]
      | @call _ f _ _ fn σ hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        have hlen := paramTys_length fn σ
        simp only [evalRhs, hf, hvs]
        rw [ite_eq_left ((valsTy_length hvst).trans hlen)]
        have henv' := bindParams_typed hvst hlen (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        exact ihE henv' (body_typed hwf hf σ)
      | @invoke _ f _ fn σ hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        have hlen := paramTys_length fn σ
        simp only [evalRhs, hf, hvs]
        rw [ite_eq_left ((valsTy_length hvst).trans hlen)]
        have henv' := bindParams_typed hvst hlen (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        exact ihE henv' (body_typed hwf hf σ)
      | @invokeOp _ o _ f fn σ _ hr hf has =>
        obtain ⟨gn, hgn, hps, hres⟩ := handlerFor_typed hwf hr (lookup env (markerName o)) hf
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        rw [← hps σ] at hvst
        have hlen := paramTys_length gn σ
        simp only [evalRhs, hgn, hvs]
        rw [ite_eq_left ((valsTy_length hvst).trans hlen)]
        have henv' := bindParams_typed hvst hlen (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        rw [← hres σ]
        exact ihE henv' (body_typed hwf hgn σ)
      | apply hf has =>
        obtain ⟨fv, hv, hvt⟩ := atom_typed henv hf
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp only [evalRhs, hv, hvs]
        exact applyWith_good ihE hvt hvst w
      | op hl hs =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hl
        cases hvt <;> simp_all [evalRhs, GoodX, ValTy.unit, Val.line, Ty.Scalar]
      | hmac hk hd hs =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hk
        obtain ⟨u, hu, hut⟩ := atom_typed henv hd
        cases hvt with
        | @text k =>
          cases hut <;> simp only [Ty.Scalar] at hs <;>
            simp only [evalRhs, hv, hu, Val.line] <;> cases H.key k <;> first | trivial | exact .text
      | read hu =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hu
        cases hvt with
        | @text s =>
          simp only [evalRhs, hv]
          cases H.read s w.fs
          · trivial
          · exact .text
      | write hu hc =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hu
        obtain ⟨c, hc', hct⟩ := atom_typed henv hc
        cases hvt with
        | @text s =>
          cases hct with
          | @text c =>
            simp only [evalRhs, hv, hc']
            cases H.write s c w.fs
            · trivial
            · exact .unit
      | @req _ m _ _ has _ hr =>
        obtain ⟨vs, hvs, _⟩ := atoms_typed henv has
        simp only [evalRhs, hvs]
        generalize hq : H.request m vs w.srv = q
        obtain ⟨o, srv'⟩ := q
        cases o with
        | error c => simp [GoodX]
        | ok v => simpa [GoodX] using ValTy.response hr (h4 m vs w.srv srv' v hq)
      | construct hrec has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, GoodX]
        exact .record hrec hvst
      | variant hs hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, GoodX]
        exact .variant hs hf hvst
      | field hv hrec hi =>
        obtain ⟨v, hv', hvt⟩ := atom_typed henv hv
        cases hvt with
        | record hrec' hvals =>
          rw [hrec] at hrec'
          cases hrec'
          obtain ⟨u, hu, hut⟩ := field_typed hvals hi
          simp [evalRhs, hv', hu, GoodX, hut]
        | variant hs _ _ => exact (not_record_of_sum hwf hs hrec).elim
      | list has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, GoodX]
        exact .list (valsTy_const hvst)
      | none => simp only [evalRhs, done, GoodX]; exact .none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, GoodX]
        exact .some hvt
      | first ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt with
        | @list items _ hall =>
          cases items with
          | nil => simp [evalRhs, hv, GoodX]; exact .none
          | cons x xs => simp [evalRhs, hv, GoodX]; exact .some (hall x (by simp))
      | elemAt hxs hi =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hxs
        obtain ⟨k, hk, hkt⟩ := atom_typed henv hi
        cases hvt with
        | @list items _ hall =>
          cases hkt with
          | @int n =>
            simp only [evalRhs, hv, hk, done, GoodX]
            rcases atIndex_cases items n with h | ⟨x, hx, h⟩
            · rw [h]; exact .none
            · rw [h]; exact .some (hall x hx)
      | orElse ho hd =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ho
        obtain ⟨x, hx, hxt⟩ := atom_typed henv hd
        cases hvt with
        | none => simp [evalRhs, hv, hx, GoodX]; exact hxt
        | some hs => simp [evalRhs, hv, hx, GoodX]; exact hs
      | isOk ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt with
        | ok _ => simp [evalRhs, hv, GoodX]; exact .bool
        | err _ => simp [evalRhs, hv, GoodX]; exact .bool
      | mkOk ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, GoodX]
        exact .ok hvt
      | mkErr ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, GoodX]
        exact .err hvt
      | lambda hlen hb => simp only [evalRhs, done, GoodX]; exact .closure henv hlen hb
      | @funRef _ f _ fn σ hf =>
        simp only [evalRhs, hf, done, GoodX]
        exact .closure .nil (paramTys_length fn σ) (by simpa using body_typed hwf hf σ)
      | declassify hpo ha =>
        obtain ⟨_, _, fn, hfn, hps, hres⟩ := hwf.policies _ (policy_mem hpo)
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        have hlen := paramTys_length fn []
        rw [hps] at hlen
        simp only [evalRhs, hpo, hv, hfn]
        rw [ite_eq_left (by simpa using hlen)]
        have hvs : ValsTy p [v] (fn.paramTys []) := by rw [hps]; exact .cons hvt .nil
        have henv' := bindParams_typed hvs (paramTys_length fn []) (EnvTy.nil (p := p))
        rw [List.append_nil] at henv'
        have hbody := body_typed hwf hfn []
        rw [Ty.subst_nil, hres] at hbody
        have h := ihE (w := w) henv' hbody
        revert h
        generalize eval H _ p w (bindParams fn.params [v] []) fn.body = r
        obtain ⟨tr, res, w'⟩ := r
        cases res <;> simp [GoodX]
      | map hxs hf =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hf
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          exact walkMap_good (fun w v hv => applyWith_good ihE hft (.cons hv .nil) w) w hitems
      | filter hxs hk =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hk
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          exact walkFilter_good (fun w v hv => applyWith_good ihE hft (.cons hv .nil) w) w hitems
      | firstWhere hxs hk =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hk
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hfv]
          exact walkFirst_good (fun w v hv => applyWith_good ihE hft (.cons hv .nil) w) w hitems
      | fold hxs hi hf =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨iv, hiv, hit⟩ := atom_typed henv hi
        obtain ⟨fv, hfv, hft⟩ := atom_typed henv hf
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, hfv]
          exact walkFold_good
            (fun w acc v hacc hv => applyWith_good ihE hft (.cons hacc (.cons hv .nil)) w) w hit hitems
      | parse hs =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hs
        cases hvt with
        | @text s =>
          simp only [evalRhs, hv]
          exact parseInt_good w s
      | checked ha hs =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt <;> simp only [Ty.Scalar] at hs <;> simp only [evalRhs, hv, checkRun, Val.line] <;> split
        all_goals first | (simp only [done, GoodX]; constructor) | trivial
      | eq ha hb =>
        obtain ⟨va, hva, _⟩ := atom_typed henv ha
        obtain ⟨vb, hvb, _⟩ := atom_typed henv hb
        simp only [evalRhs, hva, hvb, done, GoodX]
        exact .bool
      | toText ha =>
        obtain ⟨v, hv, _⟩ := atom_typed henv ha
        simp only [evalRhs, hv, done, GoodX]
        exact .text
      | envGet ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt with
        | @text s =>
          simp only [evalRhs, hv, done, GoodX]
          cases w.env s
          · exact .none
          · exact .some .text
      | tuple has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, GoodX]
        exact .tuple hvst
      | append hxs hi =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨iv, hiv, hit⟩ := atom_typed henv hi
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, done, GoodX]
          exact .list (fun u hu => by
            rcases List.mem_append.mp hu with hu | hu
            · exact hitems u hu
            · simp at hu; subst hu; exact hit)
      | set hxs hi hv =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨iv, hiv, hit⟩ := atom_typed henv hi
        obtain ⟨vv, hvv, hvt⟩ := atom_typed henv hv
        cases hxt with
        | list hitems =>
          cases hit
          simp only [evalRhs, hxv, hiv, hvv]
          split
          · simp only [done, GoodX]
            refine .tuple (.cons (.list fun u hu => ?_) (.cons .unit .nil))
            rcases List.mem_or_eq_of_mem_set hu with hu | rfl
            · exact hitems u hu
            · exact hvt
          · simp [abort, GoodX]
    · intro w env ctx e t X henv he
      cases he with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [eval, hv, GoodX, hvt]
      | letR hr hb =>
        simp only [eval]
        exact andThen_goodX (GoodX.of_none (ihR henv hr)) (fun w v hv => ihE (.cons hv henv) hb)
      | letE hv hb =>
        simp only [eval]
        exact andThen_goodX (ihE henv hv) (fun w v hv => ihE (.cons hv henv) hb)
      | ite hc ht he =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hc
        cases hvt with
        | @bool b =>
          cases b
          · simp only [eval, hv]; exact ihE henv he
          · simp only [eval, hv]; exact ihE henv ht
      | caseOption hs hn hsome =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hs
        cases hvt with
        | none => simp only [eval, hv]; exact ihE henv hn
        | some hw => simp only [eval, hv]; exact ihE (.cons hw henv) hsome
      | caseResult hs hok herr =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hs
        cases hvt with
        | ok hw => simp only [eval, hv]; exact ihE (.cons hw henv) hok
        | err hw => simp only [eval, hv]; exact ihE (.cons hw henv) herr
      | fail => simp [eval, GoodX]
      | raise hs hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp only [eval, hvs, GoodX]
        exact ⟨_, _, hs, hf, hvst⟩
      | unreachable => simp [eval, GoodX]
      | withH hb =>
        simp only [eval]
        exact ihE (.cons .text henv) hb
      | @caseSum _ _ _ _ ctor _ _ _ _ _ _ hsc hs hf hlen hth hel =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hsc
        cases hvt with
        | record hrec _ => exact (not_record_of_sum hwf hs hrec).elim
        | @variant _ i fs _ _ hs' hf' hvals =>
          rw [hs] at hs'
          cases hs'
          simp only [eval, hv]
          by_cases hi : i = ctor
          · subst hi
            rw [hf] at hf'
            cases hf'
            rw [if_pos rfl, if_pos ((valsTy_length hvals).trans hlen)]
            exact ihE (zip_typed henv hvals hlen) hth
          · rw [if_neg hi]
            exact ihE henv hel
      | attemptK hb =>
        have h := ihE (w := w) henv hb
        simp only [eval]
        revert h
        cases eval H n p w env _ with
        | mk tr rw =>
          obtain ⟨r, w'⟩ := rw
          cases r with
          | ok v => intro h; exact .ok h
          | raised k c vs =>
            intro h
            dsimp only
            split
            · next hk =>
              subst hk
              obtain ⟨sm, ts, hs, hf, hvs⟩ := h
              exact .err (.variant hs hf hvs)
            · exact h
          | failed _ => intro _; trivial
          | exit _ _ => simp [GoodX]
          | stuck => intro h; exact h
          | outOfFuel | aborted => intro _; trivial
      | handleK hb hh =>
        have h := ihE (w := w) henv hb
        simp only [eval]
        revert h
        cases eval H n p w env _ with
        | mk tr rw =>
          obtain ⟨r, w'⟩ := rw
          cases r with
          | raised k c vs =>
            intro h
            dsimp only
            split
            · next hk =>
              subst hk
              obtain ⟨sm, ts, hs, hf, hvs⟩ := h
              have hgood := ihE (w := w') (.cons (ValTy.variant hs hf hvs) henv) hh
              simpa using GoodX.of_none (X := X) hgood
            · simpa [GoodX] using h
          | failed _ => simp [GoodX]
          | ok v => simp [GoodX]
          | exit _ _ => simp [GoodX]
          | stuck => simp [GoodX]
          | outOfFuel | aborted => simp [GoodX]
      | attempt hb =>
        have h := ihE (w := w) henv hb
        simp only [eval]
        revert h
        cases eval H n p w env _ with
        | mk tr rw =>
          obtain ⟨r, w'⟩ := rw
          cases r <;> simp [GoodX]
          · exact .ok
          · exact .err .int
      | handle hb hh =>
        have h := ihE (w := w) henv hb
        simp only [eval]
        revert h
        cases eval H n p w env _ with
        | mk tr rw =>
          obtain ⟨r, w'⟩ := rw
          cases r with
          | failed code =>
            intro _
            have hgood := ihE (w := w') (.cons (ValTy.int (n := (code : Int))) henv) hh
            simpa using GoodX.of_none (X := X) hgood
          | ok v => simp [GoodX]
          | stuck => simp [GoodX]
          | raised _ _ _ => simp [GoodX]
          | exit _ _ => simp [GoodX]
          | outOfFuel | aborted => simp [GoodX]
      | letTuple hv hlen hb =>
        obtain ⟨v, hv', hvt⟩ := atom_typed henv hv
        cases hvt with
        | tuple hvals =>
          simp only [eval, hv']
          rw [ite_eq_left ((valsTy_length hvals).trans hlen)]
          exact ihE (bindParams_typed hvals hlen henv) hb
      | loop hs hi hlen hb =>
        obtain ⟨items, hitems, hit⟩ := source_typed henv hs
        obtain ⟨st, hst, hstt⟩ := atoms_typed henv hi
        simp only [eval, hitems, hst]
        rw [ite_eq_left ((valsTy_length hstt).trans hlen)]
        rw [← hlen]
        exact endLoop_good (walkLoop_good
          (fun w v st' hv hst' => ihE (.cons hv (bindParams_typed hst' hlen henv)) hb) w hstt hit)
      | exit has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp only [eval, hvs]
        exact ⟨_, rfl, hvst⟩
      | loopUntil hi hlen hc hb =>
        obtain ⟨st, hst, hstt⟩ := atoms_typed henv hi
        simp only [eval, hst]
        rw [ite_eq_left ((valsTy_length hstt).trans hlen)]
        rw [← hlen]
        exact endLoop_good (walkUntil_good (fun w st' hst' => ihE (bindParams_typed hst' hlen henv) hc)
          (fun w st' hst' => ihE (bindParams_typed hst' hlen henv) hb) n w hstt)
      | loopEver hi hlen hb hlo ho =>
        obtain ⟨st, hst, hstt⟩ := atoms_typed henv hi
        simp only [eval, hst]
        rw [ite_eq_left ((valsTy_length hstt).trans hlen)]
        rw [← hlen]
        refine finishEver_good (fun w st' hst' => ?_) hstt
          (endLoop_good (walkEver_good (fun w st' hst' => ihE (bindParams_typed hst' hlen henv) hb) n 0 w hstt))
        rw [if_pos ((valsTy_length hst').trans hlo)]
        exact ihE (bindParams_typed hst' hlo henv) ho

/-- **T1 for F1.** A well-typed expression of a well-formed program, run in an environment of
its context's types with any fuel and any host, never gets stuck, and a value it ends with has
its type. -/
theorem t1 (hwf : p.WellFormed) (h4 : H.RespTyped) (w : World H) {env : Env} {ctx : Ctx} {e : Expr} {t : Ty}
    (henv : EnvTy p env ctx) (he : ExprTy p ctx e t) (n : Nat) :
    (eval H n p w env e).2.1 ≠ .stuck ∧ ∀ v, (eval H n p w env e).2.1 = .ok v → ValTy p v t := by
  have h := (sound hwf h4 n).2 (w := w) henv he
  constructor
  · intro hs
    rw [hs] at h
    exact h
  · intro v hv
    rw [hv] at h
    exact h

end Una.F1
