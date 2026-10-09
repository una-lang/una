/-
The big-step semantics of F1 (`spec/11`), with fuel: evaluation takes a number of steps it may
use, and running out of them is its own result, so that a program that does not terminate has
a semantics too (T1 and T6 are termination-insensitive, `spec/10 §10`). The trace holds the
events of the run, in order (`spec/11 §5`). The host is a parameter, the `World`, so that two
runs can differ in it.
-/
import UnaFormal.F1.Types

namespace Una.F1

/-- The values of F1. A closure keeps the environment where its lambda was made. -/
inductive Val where
  | int (n : Int)
  | bool (b : Bool)
  | text (s : String)
  | unit
  | list (items : List Val)
  | none
  | some (value : Val)
  | ok (value : Val)
  | err (code : Nat)
  | record (name : String) (fields : List Val)
  | closure (params : List (String × LTy)) (body : Expr) (env : List (String × Val))

/-- An environment: the values of the variables in scope, innermost first. -/
abbrev Env := List (String × Val)

/-- The events of a trace: a line the operation wrote, and a request of the host. -/
inductive Event where
  | out (line : String)
  | get (url : String)

abbrev Trace := List Event

/-- The host: what `get` returns for each URL, or nothing when it fails. -/
structure World where
  get : String → Option String

/-- The code of a failure of the host or of `parse`. -/
def hostFailure : Nat := 0

/-- How an evaluation ends: with a value, with a failure, stuck (no rule applies, which T1
says never happens to a well-typed program), or out of fuel. -/
inductive Res where
  | ok (v : Val)
  | failed (code : Nat)
  | stuck
  | outOfFuel

/-- The value of variable `x` in `env`, the innermost binding. -/
def lookup (env : Env) (x : String) : Option Val :=
  match env with
  | [] => none
  | (y, v) :: rest => if x = y then some v else lookup rest x

def evalAtom (env : Env) : Atom → Option Val
  | .var x => lookup env x
  | .int n => some (.int n)
  | .bool b => some (.bool b)
  | .text s => some (.text s)
  | .unit => some .unit

def evalAtoms (env : Env) : List Atom → Option (List Val)
  | [] => some []
  | a :: rest => do
    let v ← evalAtom env a
    let vs ← evalAtoms env rest
    pure (v :: vs)

/-- A primitive on the values of its arguments. -/
def evalPrim : Prim → List Val → Option Val
  | .add, [.int a, .int b] => some (.int (a + b))
  | .sub, [.int a, .int b] => some (.int (a - b))
  | .lt, [.int a, .int b] => some (.bool (decide (a < b)))
  | .eqInt, [.int a, .int b] => some (.bool (decide (a = b)))
  | .not, [.bool b] => some (.bool (!b))
  | .len, [.list items] => some (.int items.length)
  | .len, [.text s] => some (.int s.length)
  | _, _ => none

/-- The parameters bound to the arguments, in front of `env`. -/
def bindParams {α : Type} (params : List (String × α)) (args : List Val) (env : Env) : Env :=
  (params.map Prod.fst).zip args ++ env

/-- The result of a `let`: the body runs when the bound value is a value; a failure, a stuck
evaluation or the end of fuel ends the whole. -/
def andThen (first : Trace × Res) (rest : Val → Trace × Res) : Trace × Res :=
  match first with
  | (t, .ok v) =>
    match rest v with
    | (t', r) => (t ++ t', r)
  | (t, r) => (t, r)

/-- A value ends the run. -/
def done (v : Val) : Trace × Res := ([], .ok v)

def stuck : Trace × Res := ([], .stuck)

/-- A call of the function value `fv` on `vs`, with `run` the evaluation of a body. -/
def applyWith (run : Env → Expr → Trace × Res) (fv : Val) (vs : List Val) : Trace × Res :=
  match fv with
  | .closure params body cenv =>
    if vs.length = params.length then run (bindParams params vs cenv) body else stuck
  | _ => stuck

/-- The semantic functions of the prelude's higher-order functions (`spec/14 §1`), with `app`
the call of the function value: `map` calls it once per element, in order. -/
def walkMap (app : Val → Trace × Res) : List Val → Trace × Res
  | [] => done (.list [])
  | v :: vs =>
    andThen (app v) fun r =>
      andThen (walkMap app vs) fun rest =>
        match rest with
        | .list rs => done (.list (r :: rs))
        | _ => stuck

def walkFilter (app : Val → Trace × Res) : List Val → Trace × Res
  | [] => done (.list [])
  | v :: vs =>
    andThen (app v) fun b =>
      match b with
      | .bool keep =>
        andThen (walkFilter app vs) fun rest =>
          match rest with
          | .list rs => done (.list (if keep then v :: rs else rs))
          | _ => stuck
      | _ => stuck

/-- `first_where` stops at the first element its predicate accepts. -/
def walkFirst (app : Val → Trace × Res) : List Val → Trace × Res
  | [] => done .none
  | v :: vs =>
    andThen (app v) fun b =>
      match b with
      | .bool true => done (.some v)
      | .bool false => walkFirst app vs
      | _ => stuck

def walkFold (app : Val → Val → Trace × Res) (acc : Val) : List Val → Trace × Res
  | [] => done acc
  | v :: vs => andThen (app acc v) fun acc' => walkFold app acc' vs

mutual
  /-- Structural equality of values, the semantic function of `==`. Function values have no
  equality (`Eq` excludes them, `spec/20`), and compare unequal. -/
  def Val.beq : Val → Val → Bool
    | .int a, .int b => a == b
    | .bool a, .bool b => a == b
    | .text a, .text b => a == b
    | .unit, .unit => true
    | .list xs, .list ys => Val.beqs xs ys
    | .none, .none => true
    | .some a, .some b => Val.beq a b
    | .ok a, .ok b => Val.beq a b
    | .err a, .err b => a == b
    | .record r xs, .record s ys => r == s && Val.beqs xs ys
    | _, _ => false

  def Val.beqs : List Val → List Val → Bool
    | [], [] => true
    | x :: xs, y :: ys => Val.beq x y && Val.beqs xs ys
    | _, _ => false
end

/-- The partial primitive's semantic function: the integer a text spells, or a failure. -/
def parseInt (s : String) : Trace × Res :=
  match s.toInt? with
  | some n => done (.int n)
  | none => ([], .failed hostFailure)

mutual
  def evalRhs : Nat → Program → World → Env → Rhs → Trace × Res
    | 0, _, _, _, _ => ([], .outOfFuel)
    | n + 1, p, w, env, r =>
      match r with
      | .atom a => ([], (evalAtom env a).elim .stuck .ok)
      | .prim pr args =>
        ([], ((evalAtoms env args).bind (evalPrim pr)).elim .stuck .ok)
      | .call f _ args =>
        match p.find f, evalAtoms env args with
        | some fn, some vs =>
          if vs.length = fn.params.length then eval n p w (bindParams fn.params vs []) fn.body
          else stuck
        | _, _ => stuck
      | .apply f args =>
        match evalAtom env f, evalAtoms env args with
        | some fv, some vs => applyWith (eval n p w) fv vs
        | _, _ => stuck
      | .op line =>
        match evalAtom env line with
        | some (.text s) => ([.out s], .ok .unit)
        | _ => stuck
      | .get url =>
        match evalAtom env url with
        | some (.text s) =>
          match w.get s with
          | some body => ([.get s], .ok (.text body))
          | none => ([.get s], .failed hostFailure)
        | _ => stuck
      | .construct record fields =>
        ([], ((evalAtoms env fields).map (Val.record record)).elim .stuck .ok)
      | .field value index =>
        match evalAtom env value with
        | some (.record _ fields) => ([], (fields[index]?).elim .stuck .ok)
        | _ => stuck
      | .list items => ([], ((evalAtoms env items).map Val.list).elim .stuck .ok)
      | .none => done .none
      | .some a => ([], ((evalAtom env a).map Val.some).elim .stuck .ok)
      | .lambda params body => done (.closure params body env)
      | .funRef f _ =>
        match p.find f with
        | some fn => done (.closure fn.params fn.body [])
        | none => stuck
      | .declassify pol a =>
        match p.policy pol, evalAtom env a with
        | some po, some v =>
          match p.find po.via with
          | some fn =>
            if 1 = fn.params.length then eval n p w (bindParams fn.params [v] []) fn.body
            else stuck
          | none => stuck
        | _, _ => stuck
      | .map xs f =>
        match evalAtom env xs, evalAtom env f with
        | some (.list items), some fv => walkMap (fun v => applyWith (eval n p w) fv [v]) items
        | _, _ => stuck
      | .filter xs keep =>
        match evalAtom env xs, evalAtom env keep with
        | some (.list items), some fv =>
          walkFilter (fun v => applyWith (eval n p w) fv [v]) items
        | _, _ => stuck
      | .firstWhere xs keep =>
        match evalAtom env xs, evalAtom env keep with
        | some (.list items), some fv =>
          walkFirst (fun v => applyWith (eval n p w) fv [v]) items
        | _, _ => stuck
      | .fold xs init f =>
        match evalAtom env xs, evalAtom env init, evalAtom env f with
        | some (.list items), some acc, some fv =>
          walkFold (fun a v => applyWith (eval n p w) fv [a, v]) acc items
        | _, _, _ => stuck
      | .parse s =>
        match evalAtom env s with
        | some (.text t) => parseInt t
        | _ => stuck
      | .eq a b =>
        match evalAtom env a, evalAtom env b with
        | some va, some vb => done (.bool (Val.beq va vb))
        | _, _ => stuck

  def eval : Nat → Program → World → Env → Expr → Trace × Res
    | 0, _, _, _, _ => ([], .outOfFuel)
    | n + 1, p, w, env, e =>
      match e with
      | .atom a => ([], (evalAtom env a).elim .stuck .ok)
      | .letR x _ value body =>
        andThen (evalRhs n p w env value) (fun v => eval n p w ((x, v) :: env) body)
      | .letE x _ value body =>
        andThen (eval n p w env value) (fun v => eval n p w ((x, v) :: env) body)
      | .ite cond thenE elseE =>
        match evalAtom env cond with
        | some (.bool true) => eval n p w env thenE
        | some (.bool false) => eval n p w env elseE
        | _ => stuck
      | .caseOption scrutinee noneE x someE =>
        match evalAtom env scrutinee with
        | some .none => eval n p w env noneE
        | some (.some v) => eval n p w ((x, v) :: env) someE
        | _ => stuck
      | .caseResult scrutinee x okE y errE =>
        match evalAtom env scrutinee with
        | some (.ok v) => eval n p w ((x, v) :: env) okE
        | some (.err code) => eval n p w ((y, .int code) :: env) errE
        | _ => stuck
      | .fail code => ([], .failed code)
      | .attempt body =>
        match eval n p w env body with
        | (t, .ok v) => (t, .ok (.ok v))
        | (t, .failed code) => (t, .ok (.err code))
        | (t, r) => (t, r)
      | .handle body x handler =>
        match eval n p w env body with
        | (t, .failed code) =>
          match eval n p w ((x, .int code) :: env) handler with
          | (t', r) => (t ++ t', r)
        | (t, r) => (t, r)
end

end Una.F1
