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
of fuel; never a stuck evaluation. -/
def Good (p : Program) (t : Ty) : Res → Prop
  | .ok v => ValTy p v t
  | .failed _ => True
  -- A failure with a payload (#512): the payload has its variant's field types, which a
  -- payload kind's `attempt` and handler give as a value.
  | .raised k c vs => ∃ sm ts, p.sum k = some sm ∧ sm.fieldTys c = some ts ∧ ValsTy p vs ts
  | .stuck => False
  | .outOfFuel | .aborted => True

theorem andThen_good {first : Run H} {rest : World H → Val → Run H} {t s : Ty}
    (h1 : Good p t first.2.1) (h2 : ∀ w v, ValTy p v t → Good p s (rest w v).2.1) :
    Good p s (andThen first rest).2.1 := by
  obtain ⟨tr, r, w⟩ := first
  cases r with
  | ok v =>
    simp only [andThen]
    have := h2 w v h1
    revert this
    cases rest w v
    simp
  | failed code => simp [andThen, Good]
  | raised k c vs => simpa [andThen, Good] using h1
  | stuck => simp [Good] at h1
  | outOfFuel | aborted => simp [andThen, Good]

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
  | [], _, _ => by simp only [walkMap, done, Good]; exact .list (by simp)
  | v :: vs, w, h => by
    simp only [walkMap]
    refine andThen_good (happ w v (h v List.mem_cons_self)) (fun w r hr => ?_)
    refine andThen_good (walkMap_good happ w (fun u hu => h u (List.mem_cons_of_mem _ hu)))
      (fun w rest hrest => ?_)
    cases hrest with
    | list hrs =>
      simp only [done, Good]
      exact .list (fun u hu => by
        rcases List.mem_cons.mp hu with rfl | hu
        · exact hr
        · exact hrs u hu)

theorem walkFilter_good {app : World H → Val → Run H} {a : Ty}
    (happ : ∀ w v, ValTy p v a → Good p .bool (app w v).2.1) :
    ∀ {items : List Val} (w : World H), (∀ v ∈ items, ValTy p v a) →
      Good p (.list a) (walkFilter app w items).2.1
  | [], _, _ => by simp only [walkFilter, done, Good]; exact .list (by simp)
  | v :: vs, w, h => by
    simp only [walkFilter]
    refine andThen_good (happ w v (h v List.mem_cons_self)) (fun w b hb => ?_)
    cases hb with
    | @bool keep =>
      refine andThen_good (walkFilter_good happ w (fun u hu => h u (List.mem_cons_of_mem _ hu)))
        (fun w rest hrest => ?_)
      cases hrest with
      | list hrs =>
        simp only [done, Good]
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
  | [], _, _ => by simp only [walkFirst, done, Good]; exact .none
  | v :: vs, w, h => by
    simp only [walkFirst]
    refine andThen_good (happ w v (h v List.mem_cons_self)) (fun w b hb => ?_)
    cases hb with
    | @bool keep =>
      cases keep
      · exact walkFirst_good happ w (fun u hu => h u (List.mem_cons_of_mem _ hu))
      · simp only [done, Good]
        exact .some (h v List.mem_cons_self)

theorem walkFold_good {app : World H → Val → Val → Run H} {a b : Ty}
    (happ : ∀ w acc v, ValTy p acc b → ValTy p v a → Good p b (app w acc v).2.1) :
    ∀ {items : List Val} (w : World H) {acc : Val}, ValTy p acc b → (∀ v ∈ items, ValTy p v a) →
      Good p b (walkFold app w acc items).2.1
  | [], _, _, hacc, _ => by simpa [walkFold, done, Good] using hacc
  | v :: vs, w, _, hacc, h => by
    simp only [walkFold]
    exact andThen_good (happ w _ v hacc (h v List.mem_cons_self))
      (fun w acc' hacc' => walkFold_good happ w hacc' (fun u hu => h u (List.mem_cons_of_mem _ hu)))

/-- A loop whose body maps a typed state to a typed state ends with a typed state. -/
theorem walkLoop_good {body : World H → Val → List Val → Run H} {a : Ty} {ts : List Ty}
    (hbody : ∀ w v st, ValTy p v a → ValsTy p st ts → Good p (.tuple ts) (body w v st).2.1) :
    ∀ {items : List Val} (w : World H) {st : List Val}, ValsTy p st ts →
      (∀ v ∈ items, ValTy p v a) → Good p (.tuple ts) (walkLoop body w st items).2.1
  | [], _, _, hst, _ => by simp only [walkLoop, done, Good]; exact .tuple hst
  | v :: vs, w, st, hst, h => by
    simp only [walkLoop]
    refine andThen_good (hbody w v st (h v List.mem_cons_self) hst) (fun w r hr => ?_)
    cases hr with
    | tuple hvals =>
      simp only
      rw [ite_eq_left ((valsTy_length hvals).trans (valsTy_length hst).symm)]
      exact walkLoop_good hbody w hvals (fun u hu => h u (List.mem_cons_of_mem _ hu))

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
    (∀ {w : World H} {env : Env} {ctx : Ctx} {e : Expr} {t : Ty},
      EnvTy p env ctx → ExprTy p ctx e t → Good p t (eval H n p w env e).2.1)
  | 0 => ⟨fun _ _ => by simp [evalRhs, Good], fun _ _ => by simp [eval, Good]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := sound hwf h4 n
    constructor
    · intro w env ctx r t henv hr
      cases hr with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good, hvt]
      | prim has hp =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        obtain ⟨v, hv, hvt⟩ := prim_typed hp hvst
        simp only [evalRhs, hvs, hv, Option.map_some, Option.getD_some, Option.bind_some,
          Option.elim]
        split <;> simp [Good, hvt]
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
        cases hvt <;> simp_all [evalRhs, Good, ValTy.unit, Val.line, Ty.Scalar]
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
        | error c => simp [Good]
        | ok v => simpa [Good] using ValTy.response hr (h4 m vs w.srv srv' v hq)
      | construct hrec has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .record hrec hvst
      | variant hs hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .variant hs hf hvst
      | field hv hrec hi =>
        obtain ⟨v, hv', hvt⟩ := atom_typed henv hv
        cases hvt with
        | record hrec' hvals =>
          rw [hrec] at hrec'
          cases hrec'
          obtain ⟨u, hu, hut⟩ := field_typed hvals hi
          simp [evalRhs, hv', hu, Good, hut]
        | variant hs _ _ => exact (not_record_of_sum hwf hs hrec).elim
      | list has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .list (valsTy_const hvst)
      | none => simp only [evalRhs, done, Good]; exact .none
      | some ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good]
        exact .some hvt
      | first ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt with
        | @list items _ hall =>
          cases items with
          | nil => simp [evalRhs, hv, Good]; exact .none
          | cons x xs => simp [evalRhs, hv, Good]; exact .some (hall x (by simp))
      | elemAt hxs hi =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv hxs
        obtain ⟨k, hk, hkt⟩ := atom_typed henv hi
        cases hvt with
        | @list items _ hall =>
          cases hkt with
          | @int n =>
            simp only [evalRhs, hv, hk, done, Good]
            rcases atIndex_cases items n with h | ⟨x, hx, h⟩
            · rw [h]; exact .none
            · rw [h]; exact .some (hall x hx)
      | orElse ho hd =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ho
        obtain ⟨x, hx, hxt⟩ := atom_typed henv hd
        cases hvt with
        | none => simp [evalRhs, hv, hx, Good]; exact hxt
        | some hs => simp [evalRhs, hv, hx, Good]; exact hs
      | isOk ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt with
        | ok _ => simp [evalRhs, hv, Good]; exact .bool
        | err _ => simp [evalRhs, hv, Good]; exact .bool
      | mkOk ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good]
        exact .ok hvt
      | mkErr ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [evalRhs, hv, Good]
        exact .err hvt
      | lambda hlen hb => simp only [evalRhs, done, Good]; exact .closure henv hlen hb
      | @funRef _ f _ fn σ hf =>
        simp only [evalRhs, hf, done, Good]
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
        cases res <;> simp [Good]
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
        all_goals first | (simp only [done, Good]; constructor) | trivial
      | eq ha hb =>
        obtain ⟨va, hva, _⟩ := atom_typed henv ha
        obtain ⟨vb, hvb, _⟩ := atom_typed henv hb
        simp only [evalRhs, hva, hvb, done, Good]
        exact .bool
      | toText ha =>
        obtain ⟨v, hv, _⟩ := atom_typed henv ha
        simp only [evalRhs, hv, done, Good]
        exact .text
      | envGet ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        cases hvt with
        | @text s =>
          simp only [evalRhs, hv, done, Good]
          cases w.env s
          · exact .none
          · exact .some .text
      | tuple has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp [evalRhs, hvs, Good]
        exact .tuple hvst
      | append hxs hi =>
        obtain ⟨xv, hxv, hxt⟩ := atom_typed henv hxs
        obtain ⟨iv, hiv, hit⟩ := atom_typed henv hi
        cases hxt with
        | list hitems =>
          simp only [evalRhs, hxv, hiv, done, Good]
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
          · simp only [done, Good]
            refine .tuple (.cons (.list fun u hu => ?_) (.cons .unit .nil))
            rcases List.mem_or_eq_of_mem_set hu with hu | rfl
            · exact hitems u hu
            · exact hvt
          · simp [abort, Good]
    · intro w env ctx e t henv he
      cases he with
      | atom ha =>
        obtain ⟨v, hv, hvt⟩ := atom_typed henv ha
        simp [eval, hv, Good, hvt]
      | letR hr hb =>
        simp only [eval]
        exact andThen_good (ihR henv hr) (fun w v hv => ihE (.cons hv henv) hb)
      | letE hv hb =>
        simp only [eval]
        exact andThen_good (ihE henv hv) (fun w v hv => ihE (.cons hv henv) hb)
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
      | fail => simp [eval, Good]
      | raise hs hf has =>
        obtain ⟨vs, hvs, hvst⟩ := atoms_typed henv has
        simp only [eval, hvs, Good]
        exact ⟨_, _, hs, hf, hvst⟩
      | unreachable => simp [eval, Good]
      | withH hb =>
        simp only [eval]
        exact ihE (.cons .text henv) hb
      | @caseSum _ _ _ ctor _ _ _ _ _ _ hsc hs hf hlen hth hel =>
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
              simpa using hgood
            · exact h
          | failed _ => simp [Good]
          | ok v => simp
          | stuck => simp [Good]
          | outOfFuel | aborted => simp [Good]
      | attempt hb =>
        have h := ihE (w := w) henv hb
        simp only [eval]
        revert h
        cases eval H n p w env _ with
        | mk tr rw =>
          obtain ⟨r, w'⟩ := rw
          cases r <;> simp [Good]
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
            simpa using hgood
          | ok v => simp
          | stuck => simp [Good]
          | raised _ _ _ => simp
          | outOfFuel | aborted => simp [Good]
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
        exact walkLoop_good (fun w v st' hv hst' => ihE (.cons hv (bindParams_typed hst' hlen henv)) hb)
          w hstt hit

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
