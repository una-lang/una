/-
M4 completion, part A3: **evaluation reads no annotation**. `anon ρ` forgets every type a body,
a value or a function carries (the type of each `let`, the type arguments of each call, the
parameter types of lambdas and closures) and renames the functions it names by `ρ`. A run of a
program, forgotten, is the run of the program's forgotten functions on the forgotten body and
environment (`eval_anon`). So two programs whose functions agree once forgotten, up to `ρ`, run
the same, up to annotations (`eval_erasure`): an instance of a generic function runs as the
generic function does. The host may be any whose responses read no annotation either
(`Host.AnonInvariant`): a request is given values, and a real host is given their text.
-/
import UnaFormal.F1.Eval
import UnaFormal.F1.Binary

namespace Una.F1

/-- The type that stands for a forgotten one. -/
def anonTy : LTy := .unit LExpr.bot

mutual
  /-- A right-hand side with its annotations forgotten and its functions renamed by `ρ`. -/
  def Rhs.anon (ρ : String → String) : Rhs → Rhs
    | .call f _ args => .call (ρ f) [] args
    | .funRef f _ => .funRef (ρ f) []
    | .lambda ps b => .lambda (ps.map fun q => (q.1, anonTy)) (b.anon ρ)
    | .atom a => .atom a
    | .prim pr args => .prim pr args
    | .apply f args => .apply f args
    | .op a => .op a
    | .read a => .read a
    | .write a c => .write a c
    | .req m args => .req m args
    | .construct r fs => .construct r fs
    | .field v i => .field v i
    | .list items => .list items
    | .none => .none
    | .some a => .some a
    | .declassify pol a => .declassify pol a
    | .map xs f => .map xs f
    | .filter xs k => .filter xs k
    | .firstWhere xs k => .firstWhere xs k
    | .fold xs i f => .fold xs i f
    | .parse s => .parse s
    | .checked a pred code => .checked a pred code
    | .eq a b => .eq a b
    | .tuple items => .tuple items
    | .append xs item => .append xs item
    | .set xs i item => .set xs i item
    | .toText a => .toText a
    | .envGet a => .envGet a

  /-- A body with its annotations forgotten and its functions renamed by `ρ`. -/
  def Expr.anon (ρ : String → String) : Expr → Expr
    | .atom a => .atom a
    | .letR x _ v b => .letR x anonTy (v.anon ρ) (b.anon ρ)
    | .letE x _ v b => .letE x anonTy (v.anon ρ) (b.anon ρ)
    | .ite c t e => .ite c (t.anon ρ) (e.anon ρ)
    | .caseOption s n x sm => .caseOption s (n.anon ρ) x (sm.anon ρ)
    | .caseResult s x o y er => .caseResult s x (o.anon ρ) y (er.anon ρ)
    | .fail c => .fail c
    | .attempt b => .attempt (b.anon ρ)
    | .handle b x h => .handle (b.anon ρ) x (h.anon ρ)
    | .letTuple bs v b => .letTuple (bs.map fun q => (q.1, anonTy)) v (b.anon ρ)
    | .loop src x _ st init b => .loop src x anonTy (st.map fun q => (q.1, anonTy)) init (b.anon ρ)
end

/-- Parameters with their types forgotten. -/
def anonParams (ps : List (String × LTy)) : List (String × LTy) := ps.map fun q => (q.1, anonTy)

mutual
  /-- A value with the annotations of its closures forgotten. -/
  def Val.anon (ρ : String → String) : Val → Val
    | .int n => .int n
    | .bool b => .bool b
    | .text s => .text s
    | .unit => .unit
    | .list items => .list (Val.anons ρ items)
    | .none => .none
    | .some v => .some (v.anon ρ)
    | .ok v => .ok (v.anon ρ)
    | .err c => .err c
    | .record r fs => .record r (Val.anons ρ fs)
    | .closure ps b env => .closure (anonParams ps) (b.anon ρ) (Val.anonEnv ρ env)
    | .tuple items => .tuple (Val.anons ρ items)

  def Val.anons (ρ : String → String) : List Val → List Val
    | [] => []
    | v :: vs => v.anon ρ :: Val.anons ρ vs

  def Val.anonEnv (ρ : String → String) : List (String × Val) → List (String × Val)
    | [] => []
    | (x, v) :: env => (x, v.anon ρ) :: Val.anonEnv ρ env
end

theorem Val.anons_eq (ρ : String → String) : ∀ vs : List Val, Val.anons ρ vs = vs.map (Val.anon ρ)
  | [] => rfl
  | v :: vs => by simp [Val.anons, Val.anons_eq ρ vs]

/-- **Erasure keeps a response** (H4): it holds no closure. -/
theorem Val.anon_response {ρ : String → String} {v : Val} (h : v.IsResponse) : v.anon ρ = v := by
  unfold Val.IsResponse at h
  split at h
  · rename_i st hs body
    have hm : ∀ l : List Val, (∀ x ∈ l, ∃ n y, x = .record "prelude.Header" [.text n, .text y]) →
        l.map (Val.anon ρ) = l := by
      intro l hl
      induction l with
      | nil => rfl
      | cons x xs ih =>
        obtain ⟨n, y, rfl⟩ := hl x (by simp)
        simp [Val.anon, Val.anons, ih fun z hz => hl z (by simp [hz])]
    have hm := hm hs h
    simp [Val.anon, Val.anons_eq, hm]
  · exact h.elim

theorem Val.anonEnv_eq (ρ : String → String) :
    ∀ env : Env, Val.anonEnv ρ env = env.map (fun q => (q.1, q.2.anon ρ))
  | [] => rfl
  | (x, v) :: env => by simp [Val.anonEnv, Val.anonEnv_eq ρ env]

/-- An event with its value forgotten. -/
def Event.anon (ρ : String → String) : Event → Event
  | .out s => .out s
  | .read s => .read s
  | .write s c => .write s c
  | .req m vs => .req m (Val.anons ρ vs)
  | .release pol v => .release pol (v.anon ρ)

/-- An outcome with its value forgotten. -/
def Res.anon (ρ : String → String) : Res → Res
  | .ok v => .ok (v.anon ρ)
  | .failed c => .failed c
  | .stuck => .stuck
  | .outOfFuel => .outOfFuel

/-- A run with its values forgotten. -/
def runAnon {H : Host} (ρ : String → String) (r : Run H) : Run H :=
  (r.1.map (Event.anon ρ), r.2.1.anon ρ, r.2.2)

/-- A host whose responses read no annotation: a request whose arguments are forgotten gets the
same response, and leaves the servers as the request does. -/
def Host.AnonInvariant (H : Host) : Prop :=
  ∀ (ρ : String → String) (m : String) (vs : List Val) (s : H.Srv),
    H.request m (Val.anons ρ vs) s = H.request m vs s

/-- The host without state reads no annotation. -/
theorem Host.unit_anonInvariant : Host.unit.AnonInvariant := fun _ _ _ _ => rfl

/-- A function with everything but its parameters' names and its body forgotten, renamed by `ρ`. -/
def Fun.anon (ρ : String → String) (fn : Fun) : Fun where
  name := ρ fn.name
  tparams := []
  eqParams := []
  params := anonParams fn.params
  result := anonTy
  op := false
  host := HostFx.none
  fails := false
  body := fn.body.anon ρ
  rels := []

/-- A program whose functions are forgotten, with their names kept. -/
def Program.anon (p : Program) : Program := { p with funs := p.funs.map (Fun.anon id) }

section Lemmas

variable {ρ : String → String} {H : Host}

theorem lookup_anon : ∀ (env : Env) (x : String),
    lookup (Val.anonEnv ρ env) x = (lookup env x).map (Val.anon ρ)
  | [], _ => rfl
  | (y, v) :: env, x => by
    simp only [Val.anonEnv, lookup]
    split <;> simp [lookup_anon env x]

theorem evalAtom_anon (env : Env) : ∀ a : Atom,
    evalAtom (Val.anonEnv ρ env) a = (evalAtom env a).map (Val.anon ρ)
  | .var x => by simp [evalAtom, lookup_anon]
  | .int _ | .bool _ | .text _ | .unit => by simp [evalAtom, Val.anon]

theorem evalAtoms_anon (env : Env) : ∀ as : List Atom,
    evalAtoms (Val.anonEnv ρ env) as = (evalAtoms env as).map (·.map (Val.anon ρ))
  | [] => rfl
  | a :: as => by
    simp only [evalAtoms, evalAtom_anon, evalAtoms_anon env as]
    cases evalAtom env a <;> cases evalAtoms env as <;> simp

theorem sumInts_anon : ∀ vs : List Val, sumInts (vs.map (Val.anon ρ)) = sumInts vs
  | [] => rfl
  | v :: vs => by
    cases v <;> simp [sumInts, Val.anon, sumInts_anon vs]

theorem concatTexts_anon : ∀ vs : List Val, concatTexts (vs.map (Val.anon ρ)) = concatTexts vs
  | [] => rfl
  | v :: vs => by cases v <;> simp [concatTexts, Val.anon, concatTexts_anon vs]

mutual
  /-- The text of a value reads no annotation. -/
  theorem Val.render_anon : ∀ v : Val, (v.anon ρ).render = v.render
    | .list items | .tuple items => by
      simp only [Val.anon, Val.render]; rw [Val.renders_anon items]
    | .record r fs => by simp only [Val.anon, Val.render]; rw [Val.renders_anon fs]
    | .some v | .ok v => by simp only [Val.anon, Val.render]; rw [Val.render_anon v]
    | .int _ | .bool _ | .text _ | .unit | .none | .err _ | .closure _ _ _ => by
      simp [Val.anon, Val.render]

  theorem Val.renders_anon : ∀ vs : List Val, Val.renders (Val.anons ρ vs) = Val.renders vs
    | [] => rfl
    | v :: vs => by simp only [Val.anons, Val.renders]; rw [Val.render_anon v, Val.renders_anon vs]
end

theorem evalPrim_anon (pr : Prim) (vs : List Val) :
    evalPrim pr (vs.map (Val.anon ρ)) = (evalPrim pr vs).map (Val.anon ρ) := by
  cases pr
  case concat =>
    simp only [evalPrim, concatTexts_anon]
    cases concatTexts vs <;> simp [Val.anon]
  all_goals (rcases vs with _ | ⟨a, _ | ⟨b, _ | ⟨c, rest⟩⟩⟩ <;>
    simp only [List.map_cons, List.map_nil] <;>
    (try cases a) <;> (try cases b) <;>
    simp [evalPrim, Val.anon, Val.anons_eq, sumInts_anon, Function.comp_def])

mutual
  theorem Val.beq_anon : ∀ a b : Val, Val.beq (a.anon ρ) (b.anon ρ) = Val.beq a b
    | .list xs, b => by
      cases b with
      | list ys => simp only [Val.anon, Val.beq]; exact Val.beqs_anon xs ys
      | _ => simp [Val.anon, Val.beq]
    | .some a, b => by
      cases b with
      | some b => simp only [Val.anon, Val.beq]; exact Val.beq_anon a b
      | _ => simp [Val.anon, Val.beq]
    | .ok a, b => by
      cases b with
      | ok b => simp only [Val.anon, Val.beq]; exact Val.beq_anon a b
      | _ => simp [Val.anon, Val.beq]
    | .record r xs, b => by
      cases b with
      | record s ys => simp only [Val.anon, Val.beq]; rw [Val.beqs_anon xs ys]
      | _ => simp [Val.anon, Val.beq]
    | .int _, b | .bool _, b | .text _, b | .unit, b | .none, b | .err _, b
    | .closure _ _ _, b | .tuple _, b => by
      cases b <;> simp [Val.anon, Val.beq]

  theorem Val.beqs_anon : ∀ xs ys : List Val, Val.beqs (Val.anons ρ xs) (Val.anons ρ ys) = Val.beqs xs ys
    | [], [] => rfl
    | x :: xs, y :: ys => by
      simp only [Val.anons, Val.beqs]; rw [Val.beq_anon x y, Val.beqs_anon xs ys]
    | [], _ :: _ | _ :: _, [] => by simp [Val.anons, Val.beqs]
end

theorem Val.line_anon : ∀ v : Val, (v.anon ρ).line = v.line
  | .int _ | .bool _ | .text _ | .unit | .list _ | .none | .some _ | .ok _ | .err _ | .record _ _
  | .closure _ _ _ | .tuple _ => by simp [Val.anon, Val.line]

theorem zip_anon (xs : List String) : ∀ vs : List Val,
    (xs.zip vs).map (fun q => (q.1, q.2.anon ρ)) = xs.zip (vs.map (Val.anon ρ)) := by
  induction xs with
  | nil => intro vs; simp
  | cons x xs ih =>
    intro vs
    cases vs with
    | nil => simp
    | cons v vs => simp [ih vs]

theorem bindParams_anon {α : Type} (ps : List (String × α)) (qs : List (String × LTy))
    (hps : ps.map Prod.fst = qs.map Prod.fst) (vs : List Val) (env : Env) :
    Val.anonEnv ρ (bindParams ps vs env) = bindParams qs (vs.map (Val.anon ρ)) (Val.anonEnv ρ env) := by
  simp only [bindParams, Val.anonEnv_eq, List.map_append, ← hps, zip_anon]

theorem anonParams_fst (ps : List (String × LTy)) : (anonParams ps).map Prod.fst = ps.map Prod.fst := by
  simp [anonParams]

theorem anonParams_length (ps : List (String × LTy)) : (anonParams ps).length = ps.length := by
  simp [anonParams]

theorem runAnon_done (w : World H) (v : Val) : runAnon ρ (done w v) = done w (v.anon ρ) := rfl

theorem runAnon_stuck (w : World H) : runAnon ρ (stuck w) = stuck w := rfl

theorem andThen_anon {r r' : Run H} {k k' : World H → Val → Run H} (hr : runAnon ρ r = r')
    (hk : ∀ w v, runAnon ρ (k w v) = k' w (v.anon ρ)) :
    runAnon ρ (andThen r k) = andThen r' k' := by
  subst hr
  obtain ⟨t, res, w⟩ := r
  cases res with
  | ok v =>
    simp only [andThen, runAnon, Res.anon]
    have := hk w v
    generalize k w v = kv at this
    obtain ⟨t', r', w'⟩ := kv
    rw [← this]
    cases r' <;> simp [runAnon, Res.anon]
  | failed _ | stuck | outOfFuel => simp [andThen, runAnon, Res.anon]

theorem applyWith_anon {run run' : World H → Env → Expr → Run H}
    (hrun : ∀ w env e, runAnon ρ (run w env e) = run' w (Val.anonEnv ρ env) (e.anon ρ))
    (w : World H) (fv : Val) (vs : List Val) :
    runAnon ρ (applyWith run w fv vs) = applyWith run' w (fv.anon ρ) (vs.map (Val.anon ρ)) := by
  cases fv with
  | closure ps b cenv =>
    simp only [applyWith, Val.anon, List.length_map, anonParams_length]
    split
    · rw [hrun, bindParams_anon ps (anonParams ps) (anonParams_fst ps).symm]
    · rfl
  | _ => simp [applyWith, Val.anon, runAnon_stuck]

theorem walkMap_anon {app app' : World H → Val → Run H}
    (h : ∀ w v, runAnon ρ (app w v) = app' w (v.anon ρ)) :
    ∀ (w : World H) (vs : List Val), runAnon ρ (walkMap app w vs) = walkMap app' w (vs.map (Val.anon ρ))
  | w, [] => by simp [walkMap, runAnon_done, Val.anon, Val.anons]
  | w, v :: vs => by
    simp only [walkMap, List.map_cons]
    refine andThen_anon (h w v) fun w r => andThen_anon (walkMap_anon h w vs) fun w rest => ?_
    cases rest <;> simp [runAnon_done, runAnon_stuck, Val.anon, Val.anons]

theorem walkFilter_anon {app app' : World H → Val → Run H}
    (h : ∀ w v, runAnon ρ (app w v) = app' w (v.anon ρ)) :
    ∀ (w : World H) (vs : List Val),
      runAnon ρ (walkFilter app w vs) = walkFilter app' w (vs.map (Val.anon ρ))
  | w, [] => by simp [walkFilter, runAnon_done, Val.anon, Val.anons]
  | w, v :: vs => by
    simp only [walkFilter, List.map_cons]
    refine andThen_anon (h w v) fun w b => ?_
    cases b with
    | bool keep =>
      simp only [Val.anon]
      refine andThen_anon (walkFilter_anon h w vs) fun w rest => ?_
      cases rest <;> cases keep <;> simp [runAnon_done, runAnon_stuck, Val.anon, Val.anons]
    | _ => simp [Val.anon, runAnon_stuck]

theorem walkFirst_anon {app app' : World H → Val → Run H}
    (h : ∀ w v, runAnon ρ (app w v) = app' w (v.anon ρ)) :
    ∀ (w : World H) (vs : List Val),
      runAnon ρ (walkFirst app w vs) = walkFirst app' w (vs.map (Val.anon ρ))
  | w, [] => by simp [walkFirst, runAnon_done, Val.anon]
  | w, v :: vs => by
    simp only [walkFirst, List.map_cons]
    refine andThen_anon (h w v) fun w b => ?_
    cases b with
    | bool keep =>
      cases keep
      · simp only [Val.anon]; exact walkFirst_anon h w vs
      · simp [Val.anon, runAnon_done]
    | _ => simp [Val.anon, runAnon_stuck]

theorem walkFold_anon {app app' : World H → Val → Val → Run H}
    (h : ∀ w a v, runAnon ρ (app w a v) = app' w (a.anon ρ) (v.anon ρ)) :
    ∀ (w : World H) (acc : Val) (vs : List Val),
      runAnon ρ (walkFold app w acc vs) = walkFold app' w (acc.anon ρ) (vs.map (Val.anon ρ))
  | w, acc, [] => by simp [walkFold, runAnon_done]
  | w, acc, v :: vs => by
    simp only [walkFold, List.map_cons]
    exact andThen_anon (h w acc v) fun w acc' => walkFold_anon h w acc' vs

theorem walkLoop_anon {body body' : World H → Val → List Val → Run H}
    (h : ∀ w v st, runAnon ρ (body w v st) = body' w (v.anon ρ) (st.map (Val.anon ρ))) :
    ∀ (w : World H) (st vs : List Val),
      runAnon ρ (walkLoop body w st vs) = walkLoop body' w (st.map (Val.anon ρ)) (vs.map (Val.anon ρ))
  | w, st, [] => by simp [walkLoop, runAnon_done, Val.anon, Val.anons_eq]
  | w, st, v :: vs => by
    simp only [walkLoop, List.map_cons]
    refine andThen_anon (h w v st) fun w r => ?_
    cases r with
    | tuple st' =>
      simp only [Val.anon, Val.anons_eq, List.length_map]
      split
      · exact walkLoop_anon h w st' vs
      · rfl
    | _ => simp [Val.anon, runAnon_stuck]

theorem rangeVals_anon (a b : Int) (i : Bool) : (rangeVals a b i).map (Val.anon ρ) = rangeVals a b i := by
  simp [rangeVals, Val.anon]

theorem sourceVals_anon (env : Env) : ∀ s : Source,
    sourceVals (Val.anonEnv ρ env) s = (sourceVals env s).map (·.map (Val.anon ρ))
  | .list xs => by
    simp only [sourceVals, evalAtom_anon]
    cases evalAtom env xs with
    | none => rfl
    | some v => cases v <;> simp [Val.anon, Val.anons_eq]
  | .range lo hi i => by
    simp only [sourceVals, evalAtom_anon]
    cases evalAtom env lo with
    | none => rfl
    | some u =>
      cases evalAtom env hi with
      | none => cases u <;> rfl
      | some v => cases u <;> cases v <;> simp [Val.anon, rangeVals_anon]

theorem parseInt_anon (w : World H) (t : String) : runAnon ρ (parseInt w t) = parseInt w t := by
  unfold parseInt
  cases t.toInt? <;> simp [runAnon, done, Res.anon, Val.anon]

end Lemmas

section Main

variable {P Q : Program} {ρ : String → String} {H : Host}

/-- **Evaluation reads no annotation.** When `Q`'s functions are `P`'s forgotten, up to `ρ`, and
the two programs have the same policies, whose `via` functions `ρ` keeps, a run of `P`, forgotten,
is the run of `Q` on the forgotten environment and body, from the same world, on a host whose
responses read no annotation. -/
theorem eval_anon (hH : H.AnonInvariant) (h4 : H.RespTyped) (hf : ∀ f, (P.find f).map (Fun.anon ρ) = Q.find (ρ f))
    (hp : ∀ pol, P.policy pol = Q.policy pol)
    (hv : ∀ pol po, P.policy pol = some po → ρ po.via = po.via)
    (hps : P.predSem = Q.predSem) : ∀ n : Nat,
    (∀ w env r, runAnon ρ (evalRhs H n P w env r) =
      evalRhs H n Q w (Val.anonEnv ρ env) (r.anon ρ)) ∧
    (∀ w env e, runAnon ρ (eval H n P w env e) = eval H n Q w (Val.anonEnv ρ env) (e.anon ρ))
  | 0 => ⟨fun w env r => by simp [evalRhs, runAnon, Res.anon],
      fun w env e => by simp [eval, runAnon, Res.anon]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := eval_anon hH h4 hf hp hv hps n
    have happ : ∀ w fv vs, runAnon ρ (applyWith (eval H n P) w fv vs) =
        applyWith (eval H n Q) w (fv.anon ρ) (vs.map (Val.anon ρ)) := applyWith_anon ihE
    constructor
    · intro w env r
      cases r with
      | atom a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a <;> simp [runAnon, Res.anon]
      | prim pr args =>
        simp only [evalRhs, Rhs.anon, evalAtoms_anon]
        cases evalAtoms env args with
        | none => simp [runAnon, Res.anon]
        | some vs =>
          simp only [Option.map_some, Option.bind_some, evalPrim_anon]
          cases evalPrim pr vs <;> simp [runAnon, Res.anon]
      | call f targs args =>
        simp only [evalRhs, Rhs.anon, evalAtoms_anon, ← hf f]
        cases P.find f with
        | none => simp [runAnon_stuck]
        | some fn =>
          cases evalAtoms env args with
          | none => simp [runAnon_stuck]
          | some vs =>
            simp only [Option.map_some, Fun.anon, List.length_map, anonParams_length]
            split
            · rw [ihE, bindParams_anon fn.params (anonParams fn.params) (anonParams_fst _).symm]
              rfl
            · rfl
      | apply f args =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon, evalAtoms_anon]
        cases evalAtom env f with
        | none => simp [runAnon_stuck]
        | some fv =>
          cases evalAtoms env args with
          | none => simp [runAnon_stuck]
          | some vs => simpa using happ w fv vs
      | op line =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env line with
        | none => simp [runAnon_stuck]
        | some v =>
          simp only [Option.map_some, Val.line_anon]
          cases v.line <;> simp [stuck, runAnon, Event.anon, Res.anon, Val.anon]
      | read path =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env path with
        | none => simp [runAnon_stuck]
        | some v =>
          cases v with
          | text s =>
            simp only [Option.map_some, Val.anon]
            cases H.read s w.fs <;> simp [runAnon, Event.anon, Res.anon, Val.anon]
          | _ => simp [Val.anon, runAnon_stuck]
      | write path content =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env path with
        | none => simp [runAnon_stuck]
        | some v =>
          cases evalAtom env content with
          | none => cases v <;> simp [Val.anon, runAnon_stuck]
          | some c =>
            cases v with
            | text s =>
              cases c with
              | text t =>
                simp only [Option.map_some, Val.anon]
                cases H.write s t w.fs <;> simp [runAnon, Event.anon, Res.anon, Val.anon]
              | _ => simp [Val.anon, runAnon_stuck]
            | _ => cases c <;> simp [Val.anon, runAnon_stuck]
      | req m args =>
        simp only [evalRhs, Rhs.anon, evalAtoms_anon]
        cases evalAtoms env args with
        | none => simp [runAnon_stuck]
        | some vs =>
          simp only [Option.map_some, ← Val.anons_eq, hH ρ m vs w.srv]
          generalize hq : H.request m vs w.srv = q
          obtain ⟨o, srv'⟩ := q
          cases o with
          | error c => simp [runAnon, Event.anon, Res.anon]
          | ok v =>
            simp [runAnon, Event.anon, Res.anon, Val.anon_response (h4 m vs w.srv srv' v hq)]
      | construct record fields =>
        simp only [evalRhs, Rhs.anon, evalAtoms_anon]
        cases evalAtoms env fields <;> simp [runAnon, Res.anon, Val.anon, Val.anons_eq]
      | field value index =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env value with
        | none => simp [runAnon_stuck]
        | some v =>
          cases v with
          | record r fs =>
            simp only [Option.map_some, Val.anon, Val.anons_eq, List.getElem?_map]
            cases fs[index]? <;> simp [runAnon, Res.anon]
          | _ => simp [Val.anon, runAnon_stuck]
      | list items =>
        simp only [evalRhs, Rhs.anon, evalAtoms_anon]
        cases evalAtoms env items <;> simp [runAnon, Res.anon, Val.anon, Val.anons_eq]
      | none => simp [evalRhs, Rhs.anon, runAnon_done, Val.anon]
      | some a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a <;> simp [runAnon, Res.anon, Val.anon]
      | lambda ps b =>
        simp only [evalRhs, Rhs.anon, runAnon_done, Val.anon]
        rfl
      | funRef f targs =>
        simp only [evalRhs, Rhs.anon, ← hf f]
        cases P.find f with
        | none => simp [runAnon_stuck]
        | some fn => simp [runAnon_done, Val.anon, Fun.anon, Val.anonEnv]
      | declassify pol a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon, ← hp pol]
        cases hpo : P.policy pol with
        | none => simp [runAnon_stuck]
        | some po =>
          cases evalAtom env a with
          | none => simp [runAnon_stuck]
          | some v =>
            have hfv := hf po.via
            rw [hv pol po hpo] at hfv
            simp only [Option.map_some, ← hfv]
            cases P.find po.via with
            | none => simp [runAnon_stuck]
            | some fn =>
              simp only [Option.map_some, Fun.anon, anonParams_length]
              split
              · have hrun := ihE w (bindParams fn.params [v] []) fn.body
                rw [bindParams_anon fn.params (anonParams fn.params) (anonParams_fst _).symm] at hrun
                simp only [List.map_cons, List.map_nil, Val.anonEnv] at hrun
                rw [← hrun]
                generalize eval H n P w (bindParams fn.params [v] []) fn.body = res
                obtain ⟨t, r, w'⟩ := res
                cases r <;> simp [runAnon, Res.anon, Event.anon, Val.anon]
              · rfl
      | map xs f =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env xs with
        | none => simp [runAnon_stuck]
        | some xv =>
          cases evalAtom env f with
          | none => cases xv <;> simp [runAnon_stuck, Val.anon]
          | some fv =>
            cases xv with
            | list items =>
              simp only [Option.map_some, Val.anon, Val.anons_eq]
              exact walkMap_anon (fun w v => by simpa using happ w fv [v]) w items
            | _ => simp [Val.anon, runAnon_stuck]
      | filter xs k =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env xs with
        | none => simp [runAnon_stuck]
        | some xv =>
          cases evalAtom env k with
          | none => cases xv <;> simp [runAnon_stuck, Val.anon]
          | some fv =>
            cases xv with
            | list items =>
              simp only [Option.map_some, Val.anon, Val.anons_eq]
              exact walkFilter_anon (fun w v => by simpa using happ w fv [v]) w items
            | _ => simp [Val.anon, runAnon_stuck]
      | firstWhere xs k =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env xs with
        | none => simp [runAnon_stuck]
        | some xv =>
          cases evalAtom env k with
          | none => cases xv <;> simp [runAnon_stuck, Val.anon]
          | some fv =>
            cases xv with
            | list items =>
              simp only [Option.map_some, Val.anon, Val.anons_eq]
              exact walkFirst_anon (fun w v => by simpa using happ w fv [v]) w items
            | _ => simp [Val.anon, runAnon_stuck]
      | fold xs i f =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env xs with
        | none => simp [runAnon_stuck]
        | some xv =>
          cases evalAtom env i with
          | none => cases xv <;> simp [runAnon_stuck, Val.anon]
          | some acc =>
            cases evalAtom env f with
            | none => cases xv <;> simp [runAnon_stuck, Val.anon]
            | some fv =>
              cases xv with
              | list items =>
                simp only [Option.map_some, Val.anon, Val.anons_eq]
                exact walkFold_anon (fun w a v => by simpa using happ w fv [a, v]) w acc items
              | _ => simp [Val.anon, runAnon_stuck]
      | parse a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some v =>
          cases v with
          | text t => simpa [Val.anon] using parseInt_anon w t
          | _ => simp [Val.anon, runAnon_stuck]
      | checked a pred code =>
        -- `checked` (#453): erasure keeps a scalar's text, and the two programs share `predSem`.
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some v =>
          simp only [Option.map_some, checkRun, Val.line_anon, hps]
          split
          · split <;> simp [runAnon, done, Res.anon]
          · simp [runAnon_stuck]
      | eq a b =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some va =>
          cases evalAtom env b with
          | none => simp [runAnon_stuck]
          | some vb => simp [runAnon_done, Val.anon, Val.beq_anon]
      | tuple items =>
        simp only [evalRhs, Rhs.anon, evalAtoms_anon]
        cases evalAtoms env items <;> simp [runAnon, Res.anon, Val.anon, Val.anons_eq]
      | append xs item =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env xs with
        | none => simp [runAnon_stuck]
        | some xv =>
          cases evalAtom env item with
          | none => cases xv <;> simp [runAnon_stuck, Val.anon]
          | some v =>
            cases xv with
            | list items => simp [runAnon_done, Val.anon, Val.anons_eq]
            | _ => simp [Val.anon, runAnon_stuck]
      | toText a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some v => simp [runAnon_done, Val.anon, Val.render_anon]
      | envGet a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some v =>
          cases v with
          | text s =>
            simp only [Option.map_some, Val.anon]
            cases w.env s <;> simp [runAnon_done, Val.anon]
          | _ => simp [Val.anon, runAnon_stuck]
      | set xs index item =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env xs with
        | none => simp [runAnon_stuck]
        | some xv =>
          cases evalAtom env index with
          | none => cases xv <;> simp [runAnon_stuck, Val.anon]
          | some iv =>
            cases evalAtom env item with
            | none => cases xv <;> cases iv <;> simp [runAnon_stuck, Val.anon]
            | some v =>
              cases xv with
              | list items =>
                cases iv with
                | int i =>
                  simp only [Option.map_some, Val.anon, Val.anons_eq, List.length_map]
                  split
                  · simp [runAnon_done, Val.anon, Val.anons_eq, List.map_set]
                  · rfl
                | _ => simp [Val.anon, runAnon_stuck]
              | _ => cases iv <;> simp [Val.anon, runAnon_stuck]
    · intro w env e
      cases e with
      | atom a =>
        simp only [eval, Expr.anon, evalAtom_anon]
        cases evalAtom env a <;> simp [runAnon, Res.anon]
      | letR x ann v b =>
        simp only [eval, Expr.anon]
        exact andThen_anon (ihR w env v) fun w u => by rw [ihE]; rfl
      | letE x ann v b =>
        simp only [eval, Expr.anon]
        exact andThen_anon (ihE w env v) fun w u => by rw [ihE]; rfl
      | ite c t e =>
        simp only [eval, Expr.anon, evalAtom_anon]
        cases evalAtom env c with
        | none => simp [stuck, runAnon, Res.anon]
        | some v =>
          cases v with
          | bool b => cases b <;> simp [Val.anon, ihE]
          | _ => simp [Val.anon, stuck, runAnon, Res.anon]
      | caseOption sc ne x se =>
        simp only [eval, Expr.anon, evalAtom_anon]
        cases evalAtom env sc with
        | none => simp [stuck, runAnon, Res.anon]
        | some v =>
          cases v with
          | none => simp [Val.anon, ihE]
          | some u => simp only [Option.map_some, Val.anon, ihE]; rfl
          | _ => simp [Val.anon, stuck, runAnon, Res.anon]
      | caseResult sc x oe y ee =>
        simp only [eval, Expr.anon, evalAtom_anon]
        cases evalAtom env sc with
        | none => simp [stuck, runAnon, Res.anon]
        | some v =>
          cases v with
          | ok u => simp only [Option.map_some, Val.anon, ihE]; rfl
          | err c => simp only [Option.map_some, Val.anon, ihE]; rfl
          | _ => simp [Val.anon, stuck, runAnon, Res.anon]
      | fail c => simp [eval, Expr.anon, runAnon, Res.anon]
      | attempt b =>
        simp only [eval, Expr.anon, ← ihE]
        generalize eval H n P w env b = res
        obtain ⟨t, r, w'⟩ := res
        cases r <;> simp [runAnon, Res.anon, Val.anon]
      | handle b x h =>
        simp only [eval, Expr.anon, ← ihE]
        generalize eval H n P w env b = res
        obtain ⟨t, r, w'⟩ := res
        cases r with
        | failed c =>
          simp only [runAnon, Res.anon]
          have hh := ihE w' ((x, .int c) :: env) h
          simp only [Val.anonEnv, Val.anon] at hh
          rw [← hh]
          generalize eval H n P w' ((x, .int c) :: env) h = res'
          obtain ⟨t', r', w''⟩ := res'
          cases r' <;> simp [runAnon, Res.anon]
        | _ => simp [runAnon, Res.anon]
      | letTuple bs v b =>
        simp only [eval, Expr.anon, evalAtom_anon]
        cases evalAtom env v with
        | none => simp [stuck, runAnon, Res.anon]
        | some u =>
          cases u with
          | tuple vs =>
            simp only [Option.map_some, Val.anon, Val.anons_eq, List.length_map]
            split
            · rw [ihE, bindParams_anon bs (anonParams bs) (anonParams_fst bs).symm]
              rfl
            · rfl
          | _ => simp [Val.anon, stuck, runAnon, Res.anon]
      | loop src x xt st init b =>
        simp only [eval, Expr.anon, sourceVals_anon, evalAtoms_anon]
        cases sourceVals env src with
        | none => simp [stuck, runAnon, Res.anon]
        | some items =>
          cases evalAtoms env init with
          | none => simp [stuck, runAnon, Res.anon]
          | some stv =>
            simp only [Option.map_some, List.length_map]
            split
            · refine walkLoop_anon (fun w v st' => ?_) w stv items
              rw [ihE, Val.anonEnv, bindParams_anon st (anonParams st) (anonParams_fst st).symm]
              rfl
            · rfl

/-- `find` in a forgotten program is `find`, forgotten. -/
theorem Program.find_anon (p : Program) (f : String) :
    p.anon.find f = (p.find f).map (Fun.anon id) := by
  simp only [Program.anon, Program.find, List.find?_map]
  rfl

/-- `P₁` runs as `P₂` up to annotations, with `P₁`'s functions renamed by `ρ`: their functions
agree once forgotten, they have the same policies, and `ρ` keeps the `via` functions. -/
structure Sim (ρ : String → String) (P₁ P₂ : Program) : Prop where
  funs : ∀ f, (P₁.find f).map (Fun.anon ρ) = (P₂.find (ρ f)).map (Fun.anon id)
  policies : P₁.policies = P₂.policies
  vias : ∀ po ∈ P₁.policies, ρ po.via = po.via
  /-- The meaning of `checked`'s predicates is the same (#453). -/
  predSem : P₁.predSem = P₂.predSem

/-- **Evaluation erasure.** Two programs that agree up to annotations run the same, up to
annotations, on bodies and environments that agree up to annotations. -/
theorem eval_erasure {ρ : String → String} {P₁ P₂ : Program} (h : Sim ρ P₁ P₂) {H : Host}
    (hH : H.AnonInvariant) (h4 : H.RespTyped) (w : World H) (n : Nat)
    {env₁ env₂ : Env} {e₁ e₂ : Expr} (henv : Val.anonEnv ρ env₁ = Val.anonEnv id env₂)
    (he : e₁.anon ρ = e₂.anon id) :
    runAnon ρ (eval H n P₁ w env₁ e₁) = runAnon id (eval H n P₂ w env₂ e₂) := by
  have hpol : ∀ pol, P₁.policy pol = P₂.anon.policy pol := fun pol => by
    simp [Program.policy, Program.anon, h.policies]
  have h₁ := (eval_anon (P := P₁) (Q := P₂.anon) (ρ := ρ) hH h4
    (fun f => by rw [Program.find_anon, h.funs f]) hpol
    (fun pol po hpo => h.vias po (by
      unfold Program.policy at hpo
      exact List.mem_of_find?_eq_some hpo)) (by simp [Program.anon, h.predSem]) n).2 w env₁ e₁
  have h₂ := (eval_anon (P := P₂) (Q := P₂.anon) (ρ := id) hH h4
    (fun f => by rw [Program.find_anon]; rfl) (fun pol => by simp [Program.policy, Program.anon])
    (fun _ _ _ => rfl) (by simp [Program.anon]) n).2 w env₂ e₂
  rw [h₁, h₂, henv, he]

/-- Erasure keeps whether a run ended. -/
theorem Term_anon {ρ : String → String} {r : Res} : Term (r.anon ρ) ↔ Term r := by
  cases r <;> simp [Res.anon, Term]

/-- Erasure keeps what the observer sees, up to the forgotten values of releases. -/
theorem proj_anon {ρ : String → String} (p : Program) (L : Label) (t : Trace) :
    proj p L (t.map (Event.anon ρ)) = (proj p L t).map (Event.anon ρ) := by
  unfold proj
  rw [List.filter_map]
  congr 1
  apply List.filter_congr
  intro ev _
  cases ev <;> simp only [Function.comp, Event.anon, EvVis] <;> exact decide_eq_decide.mpr Iff.rfl

/-! ### Erasure is injective on first-order values -/

/-- An event whose values, if any, are first-order. -/
def Event.FirstOrder : Event → Prop
  | .release _ v => v.FirstOrder
  | .req _ vs => Val.FirstOrderAll vs
  | _ => True

mutual
  /-- Erasure keeps whether a value is first-order. -/
  theorem Val.firstOrder_anon {ρ : String → String} : ∀ v : Val, (v.anon ρ).FirstOrder ↔ v.FirstOrder
    | .list items => by simp only [Val.anon, Val.FirstOrder]; exact Val.firstOrderAll_anon items
    | .record _ fs => by simp only [Val.anon, Val.FirstOrder]; exact Val.firstOrderAll_anon fs
    | .tuple fs => by simp only [Val.anon, Val.FirstOrder]; exact Val.firstOrderAll_anon fs
    | .some v | .ok v => by simp only [Val.anon, Val.FirstOrder]; exact Val.firstOrder_anon v
    | .closure _ _ _ | .int _ | .bool _ | .text _ | .unit | .none | .err _ => by
      simp [Val.anon, Val.FirstOrder]

  theorem Val.firstOrderAll_anon {ρ : String → String} :
      ∀ vs : List Val, Val.FirstOrderAll (Val.anons ρ vs) ↔ Val.FirstOrderAll vs
    | [] => by simp [Val.anons, Val.FirstOrderAll]
    | v :: vs => by
      simp only [Val.anons, Val.FirstOrderAll, Val.firstOrder_anon v, Val.firstOrderAll_anon vs]
end

mutual
  /-- **Erasure is injective on first-order values**, whatever the renamings. -/
  theorem Val.anon_inj {ρ ρ' : String → String} :
      ∀ (a b : Val), a.FirstOrder → b.FirstOrder → a.anon ρ = b.anon ρ' → a = b
    | .list xs, b, ha, hb, h => by
      cases b with
      | list ys =>
        simp only [Val.anon, Val.list.injEq] at h
        rw [Val.anons_inj xs ys ha hb h]
      | _ => simp [Val.anon] at h
    | .record r xs, b, ha, hb, h => by
      cases b with
      | record s ys =>
        simp only [Val.anon, Val.record.injEq] at h
        rw [h.1, Val.anons_inj xs ys ha hb h.2]
      | _ => simp [Val.anon] at h
    | .tuple xs, b, ha, hb, h => by
      cases b with
      | tuple ys =>
        simp only [Val.anon, Val.tuple.injEq] at h
        rw [Val.anons_inj xs ys ha hb h]
      | _ => simp [Val.anon] at h
    | .some x, b, ha, hb, h => by
      cases b with
      | some y => simp only [Val.anon, Val.some.injEq] at h; rw [Val.anon_inj x y ha hb h]
      | _ => simp [Val.anon] at h
    | .ok x, b, ha, hb, h => by
      cases b with
      | ok y => simp only [Val.anon, Val.ok.injEq] at h; rw [Val.anon_inj x y ha hb h]
      | _ => simp [Val.anon] at h
    | .closure _ _ _, _, ha, _, _ => absurd ha (by simp [Val.FirstOrder])
    | .int _, b, _, _, h | .bool _, b, _, _, h | .text _, b, _, _, h | .unit, b, _, _, h
    | .none, b, _, _, h | .err _, b, _, _, h => by
      cases b <;> simp_all [Val.anon]

  theorem Val.anons_inj {ρ ρ' : String → String} :
      ∀ (xs ys : List Val), Val.FirstOrderAll xs → Val.FirstOrderAll ys →
        Val.anons ρ xs = Val.anons ρ' ys → xs = ys
    | [], [], _, _, _ => rfl
    | x :: xs, y :: ys, hx, hy, h => by
      simp only [Val.anons, List.cons.injEq] at h
      rw [Val.anon_inj x y hx.1 hy.1 h.1, Val.anons_inj xs ys hx.2 hy.2 h.2]
    | [], _ :: _, _, _, h | _ :: _, [], _, _, h => by simp [Val.anons] at h
end

theorem Event.anon_inj {ρ ρ' : String → String} {a b : Event} (ha : a.FirstOrder)
    (hb : b.FirstOrder) (h : a.anon ρ = b.anon ρ') : a = b := by
  cases a <;> cases b <;> simp_all [Event.anon, Event.FirstOrder]
  · exact Val.anons_inj _ _ ha hb h.2
  · exact Val.anon_inj _ _ ha hb h.2

theorem Event.firstOrder_anon {ρ : String → String} (ev : Event) :
    (ev.anon ρ).FirstOrder ↔ ev.FirstOrder := by
  cases ev <;> simp [Event.anon, Event.FirstOrder, Val.firstOrder_anon, Val.firstOrderAll_anon]

theorem trace_anon_inj {ρ ρ' : String → String} :
    ∀ {t u : Trace}, (∀ ev ∈ t, ev.FirstOrder) → (∀ ev ∈ u, ev.FirstOrder) →
      t.map (Event.anon ρ) = u.map (Event.anon ρ') → t = u
  | [], [], _, _, _ => rfl
  | a :: t, b :: u, ht, hu, h => by
    simp only [List.map_cons, List.cons.injEq] at h
    rw [Event.anon_inj (ht a (by simp)) (hu b (by simp)) h.1,
      trace_anon_inj (fun ev hev => ht ev (by simp [hev])) (fun ev hev => hu ev (by simp [hev])) h.2]
  | [], _ :: _, _, _, h | _ :: _, [], _, _, h => by simp at h

end Main

end Una.F1
