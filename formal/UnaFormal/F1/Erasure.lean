/-
M4 completion, part A3: **evaluation reads no annotation**. `anon ρ` forgets every type a body,
a value or a function carries (the type of each `let`, the type arguments of each call, the
parameter types of lambdas and closures) and renames the functions it names by `ρ`. A run of a
program, forgotten, is the run of the program's forgotten functions on the forgotten body and
environment (`eval_anon`). So two programs whose functions agree once forgotten, up to `ρ`, run
the same, up to annotations (`eval_erasure`): an instance of a generic function runs as the
generic function does.
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
    | .get a => .get a
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
    | .eq a b => .eq a b

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

theorem Val.anonEnv_eq (ρ : String → String) :
    ∀ env : Env, Val.anonEnv ρ env = env.map (fun q => (q.1, q.2.anon ρ))
  | [] => rfl
  | (x, v) :: env => by simp [Val.anonEnv, Val.anonEnv_eq ρ env]

/-- An event with its value forgotten. -/
def Event.anon (ρ : String → String) : Event → Event
  | .out s => .out s
  | .get u => .get u
  | .release pol v => .release pol (v.anon ρ)

/-- An outcome with its value forgotten. -/
def Res.anon (ρ : String → String) : Res → Res
  | .ok v => .ok (v.anon ρ)
  | .failed c => .failed c
  | .stuck => .stuck
  | .outOfFuel => .outOfFuel

/-- A run with its values forgotten. -/
def runAnon (ρ : String → String) (r : Trace × Res) : Trace × Res :=
  (r.1.map (Event.anon ρ), r.2.anon ρ)

/-- A function with everything but its parameters' names and its body forgotten, renamed by `ρ`. -/
def Fun.anon (ρ : String → String) (fn : Fun) : Fun where
  name := ρ fn.name
  tparams := []
  eqParams := []
  params := anonParams fn.params
  result := anonTy
  op := false
  get := false
  fails := false
  body := fn.body.anon ρ
  rels := []

/-- A program whose functions are forgotten, with their names kept. -/
def Program.anon (p : Program) : Program := { p with funs := p.funs.map (Fun.anon id) }

section Lemmas

variable {ρ : String → String}

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

theorem evalPrim_anon (pr : Prim) (vs : List Val) :
    evalPrim pr (vs.map (Val.anon ρ)) = (evalPrim pr vs).map (Val.anon ρ) := by
  rcases vs with _ | ⟨a, _ | ⟨b, _ | ⟨c, rest⟩⟩⟩ <;> cases pr <;>
    simp only [List.map_cons, List.map_nil] <;>
    (try cases a) <;> (try cases b) <;>
    simp [evalPrim, Val.anon, Val.anons_eq, sumInts_anon, Function.comp_def]

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
    | .closure _ _ _, b => by
      cases b <;> simp [Val.anon, Val.beq]

  theorem Val.beqs_anon : ∀ xs ys : List Val, Val.beqs (Val.anons ρ xs) (Val.anons ρ ys) = Val.beqs xs ys
    | [], [] => rfl
    | x :: xs, y :: ys => by
      simp only [Val.anons, Val.beqs]; rw [Val.beq_anon x y, Val.beqs_anon xs ys]
    | [], _ :: _ | _ :: _, [] => by simp [Val.anons, Val.beqs]
end

theorem Val.line_anon : ∀ v : Val, (v.anon ρ).line = v.line
  | .int _ | .bool _ | .text _ | .unit | .list _ | .none | .some _ | .ok _ | .err _ | .record _ _
  | .closure _ _ _ => by simp [Val.anon, Val.line]

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

theorem runAnon_done (v : Val) : runAnon ρ (done v) = done (v.anon ρ) := rfl

theorem runAnon_stuck : runAnon ρ stuck = stuck := rfl

theorem andThen_anon {r r' : Trace × Res} {k k' : Val → Trace × Res} (hr : runAnon ρ r = r')
    (hk : ∀ v, runAnon ρ (k v) = k' (v.anon ρ)) :
    runAnon ρ (andThen r k) = andThen r' k' := by
  subst hr
  obtain ⟨t, res⟩ := r
  cases res with
  | ok v =>
    simp only [andThen, runAnon, Res.anon]
    have := hk v
    generalize k v = kv at this
    obtain ⟨t', r'⟩ := kv
    rw [← this]
    cases r' <;> simp [runAnon, Res.anon]
  | failed _ | stuck | outOfFuel => simp [andThen, runAnon, Res.anon]

theorem applyWith_anon {run run' : Env → Expr → Trace × Res}
    (hrun : ∀ env e, runAnon ρ (run env e) = run' (Val.anonEnv ρ env) (e.anon ρ))
    (fv : Val) (vs : List Val) :
    runAnon ρ (applyWith run fv vs) = applyWith run' (fv.anon ρ) (vs.map (Val.anon ρ)) := by
  cases fv with
  | closure ps b cenv =>
    simp only [applyWith, Val.anon, List.length_map, anonParams_length]
    split
    · rw [hrun, bindParams_anon ps (anonParams ps) (anonParams_fst ps).symm]
    · rfl
  | _ => simp [applyWith, Val.anon, runAnon_stuck]

theorem walkMap_anon {app app' : Val → Trace × Res} (h : ∀ v, runAnon ρ (app v) = app' (v.anon ρ)) :
    ∀ vs : List Val, runAnon ρ (walkMap app vs) = walkMap app' (vs.map (Val.anon ρ))
  | [] => by simp [walkMap, runAnon_done, Val.anon, Val.anons]
  | v :: vs => by
    simp only [walkMap, List.map_cons]
    refine andThen_anon (h v) fun r => andThen_anon (walkMap_anon h vs) fun rest => ?_
    cases rest <;> simp [runAnon_done, runAnon_stuck, Val.anon, Val.anons]

theorem walkFilter_anon {app app' : Val → Trace × Res}
    (h : ∀ v, runAnon ρ (app v) = app' (v.anon ρ)) :
    ∀ vs : List Val, runAnon ρ (walkFilter app vs) = walkFilter app' (vs.map (Val.anon ρ))
  | [] => by simp [walkFilter, runAnon_done, Val.anon, Val.anons]
  | v :: vs => by
    simp only [walkFilter, List.map_cons]
    refine andThen_anon (h v) fun b => ?_
    cases b with
    | bool keep =>
      simp only [Val.anon]
      refine andThen_anon (walkFilter_anon h vs) fun rest => ?_
      cases rest <;> cases keep <;> simp [runAnon_done, runAnon_stuck, Val.anon, Val.anons]
    | _ => simp [Val.anon, runAnon_stuck]

theorem walkFirst_anon {app app' : Val → Trace × Res}
    (h : ∀ v, runAnon ρ (app v) = app' (v.anon ρ)) :
    ∀ vs : List Val, runAnon ρ (walkFirst app vs) = walkFirst app' (vs.map (Val.anon ρ))
  | [] => by simp [walkFirst, runAnon_done, Val.anon]
  | v :: vs => by
    simp only [walkFirst, List.map_cons]
    refine andThen_anon (h v) fun b => ?_
    cases b with
    | bool keep =>
      cases keep
      · simp only [Val.anon]; exact walkFirst_anon h vs
      · simp [Val.anon, runAnon_done]
    | _ => simp [Val.anon, runAnon_stuck]

theorem walkFold_anon {app app' : Val → Val → Trace × Res}
    (h : ∀ a v, runAnon ρ (app a v) = app' (a.anon ρ) (v.anon ρ)) :
    ∀ (acc : Val) (vs : List Val),
      runAnon ρ (walkFold app acc vs) = walkFold app' (acc.anon ρ) (vs.map (Val.anon ρ))
  | acc, [] => by simp [walkFold, runAnon_done]
  | acc, v :: vs => by
    simp only [walkFold, List.map_cons]
    exact andThen_anon (h acc v) fun acc' => walkFold_anon h acc' vs

theorem parseInt_anon (t : String) : runAnon ρ (parseInt t) = parseInt t := by
  unfold parseInt
  cases t.toInt? <;> simp [runAnon, done, Res.anon, Val.anon]

end Lemmas

section Main

variable {P Q : Program} {ρ : String → String}

/-- **Evaluation reads no annotation.** When `Q`'s functions are `P`'s forgotten, up to `ρ`, and
the two programs have the same policies, whose `via` functions `ρ` keeps, a run of `P`, forgotten,
is the run of `Q` on the forgotten environment and body. -/
theorem eval_anon (hf : ∀ f, (P.find f).map (Fun.anon ρ) = Q.find (ρ f))
    (hp : ∀ pol, P.policy pol = Q.policy pol)
    (hv : ∀ pol po, P.policy pol = some po → ρ po.via = po.via) (w : World) : ∀ n : Nat,
    (∀ env r, runAnon ρ (evalRhs n P w env r) = evalRhs n Q w (Val.anonEnv ρ env) (r.anon ρ)) ∧
    (∀ env e, runAnon ρ (eval n P w env e) = eval n Q w (Val.anonEnv ρ env) (e.anon ρ))
  | 0 => ⟨fun env r => by simp [evalRhs, runAnon, Res.anon],
      fun env e => by simp [eval, runAnon, Res.anon]⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := eval_anon hf hp hv w n
    have happ : ∀ fv vs, runAnon ρ (applyWith (eval n P w) fv vs) =
        applyWith (eval n Q w) (fv.anon ρ) (vs.map (Val.anon ρ)) := applyWith_anon ihE
    constructor
    · intro env r
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
          | some vs => simpa using happ fv vs
      | op line =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env line with
        | none => simp [runAnon_stuck]
        | some v =>
          simp only [Option.map_some, Val.line_anon]
          cases v.line <;> simp [stuck, runAnon, Event.anon, Res.anon, Val.anon]
      | get url =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env url with
        | none => simp [runAnon_stuck]
        | some v =>
          cases v with
          | text s =>
            simp only [Option.map_some, Val.anon]
            cases w.get s <;> simp [runAnon, Event.anon, Res.anon, Val.anon]
          | _ => simp [Val.anon, runAnon_stuck]
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
              · have hrun := ihE (bindParams fn.params [v] []) fn.body
                rw [bindParams_anon fn.params (anonParams fn.params) (anonParams_fst _).symm] at hrun
                simp only [List.map_cons, List.map_nil, Val.anonEnv] at hrun
                rw [← hrun]
                generalize eval n P w (bindParams fn.params [v] []) fn.body = res
                obtain ⟨t, r⟩ := res
                cases r <;> simp [runAnon, Res.anon, Event.anon]
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
              exact walkMap_anon (fun v => by simpa using happ fv [v]) items
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
              exact walkFilter_anon (fun v => by simpa using happ fv [v]) items
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
              exact walkFirst_anon (fun v => by simpa using happ fv [v]) items
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
                exact walkFold_anon (fun a v => by simpa using happ fv [a, v]) acc items
              | _ => simp [Val.anon, runAnon_stuck]
      | parse a =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some v =>
          cases v with
          | text t => simpa [Val.anon] using parseInt_anon t
          | _ => simp [Val.anon, runAnon_stuck]
      | eq a b =>
        simp only [evalRhs, Rhs.anon, evalAtom_anon]
        cases evalAtom env a with
        | none => simp [runAnon_stuck]
        | some va =>
          cases evalAtom env b with
          | none => simp [runAnon_stuck]
          | some vb => simp [runAnon_done, Val.anon, Val.beq_anon]
    · intro env e
      cases e with
      | atom a =>
        simp only [eval, Expr.anon, evalAtom_anon]
        cases evalAtom env a <;> simp [runAnon, Res.anon]
      | letR x ann v b =>
        simp only [eval, Expr.anon]
        exact andThen_anon (ihR env v) fun u => by rw [ihE]; rfl
      | letE x ann v b =>
        simp only [eval, Expr.anon]
        exact andThen_anon (ihE env v) fun u => by rw [ihE]; rfl
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
        generalize eval n P w env b = res
        obtain ⟨t, r⟩ := res
        cases r <;> simp [runAnon, Res.anon, Val.anon]
      | handle b x h =>
        simp only [eval, Expr.anon, ← ihE]
        generalize eval n P w env b = res
        obtain ⟨t, r⟩ := res
        cases r with
        | failed c =>
          simp only [runAnon, Res.anon]
          have hh := ihE ((x, .int c) :: env) h
          simp only [Val.anonEnv, Val.anon] at hh
          rw [← hh]
          generalize eval n P w ((x, .int c) :: env) h = res'
          obtain ⟨t', r'⟩ := res'
          cases r' <;> simp [runAnon, Res.anon]
        | _ => simp [runAnon, Res.anon]

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

/-- **Evaluation erasure.** Two programs that agree up to annotations run the same, up to
annotations, on bodies and environments that agree up to annotations. -/
theorem eval_erasure {ρ : String → String} {P₁ P₂ : Program} (h : Sim ρ P₁ P₂) (w : World) (n : Nat)
    {env₁ env₂ : Env} {e₁ e₂ : Expr} (henv : Val.anonEnv ρ env₁ = Val.anonEnv id env₂)
    (he : e₁.anon ρ = e₂.anon id) :
    runAnon ρ (eval n P₁ w env₁ e₁) = runAnon id (eval n P₂ w env₂ e₂) := by
  have hpol : ∀ pol, P₁.policy pol = P₂.anon.policy pol := fun pol => by
    simp [Program.policy, Program.anon, h.policies]
  have h₁ := (eval_anon (P := P₁) (Q := P₂.anon) (ρ := ρ)
    (fun f => by rw [Program.find_anon, h.funs f]) hpol
    (fun pol po hpo => h.vias po (by
      unfold Program.policy at hpo
      exact List.mem_of_find?_eq_some hpo)) w n).2 env₁ e₁
  have h₂ := (eval_anon (P := P₂) (Q := P₂.anon) (ρ := id)
    (fun f => by rw [Program.find_anon]; rfl) (fun pol => by simp [Program.policy, Program.anon])
    (fun _ _ _ => rfl) w n).2 env₂ e₂
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

end Main

end Una.F1
