/-
The big-step semantics of F1 (`spec/11`), with fuel: evaluation takes a number of steps it may
use, and running out of them is its own result, so that a program that does not terminate has
a semantics too (T1 and T6 are termination-insensitive, `spec/10 §10`). The trace holds the
events of the run, in order (`spec/11 §5`).

F1H (M5): the host is state. A `Host` gives the file system and the servers as functions, so any
host is an instance, and the properties the theorem needs of it are its hypotheses
(`Host.Lawful`, H1 and H2 of `docs/plan/formal-track-m5.md` §2). A run threads the `World`, the
host's state, and ends with the world it leaves, which the next run starts from.
-/
import UnaFormal.F1H.Types

namespace Una.F1H

open Una.F1 (Label LExpr)

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

/-- The line the operation writes for a scalar. -/
def Val.line : Val → Option String
  | .int n => Option.some (toString n)
  | .bool b => Option.some (toString b)
  | .text s => Option.some s
  | _ => Option.none

/-- An environment: the values of the variables in scope, innermost first. -/
abbrev Env := List (String × Val)

/-- The events of a trace: a line the operation wrote, the path a read sent, the path and content
a write sent, a request with its arguments, and a release, the value a call of the policy `pol`
declassified (option B1 of `docs/plan/formal-track-m4.md` §3). Each is a sink at its operation's
`accepts`. -/
inductive Event where
  | out (line : String)
  | read (path : String)
  | write (path content : String)
  | req (method : String) (args : List Val)
  | release (pol : String) (value : Val)

abbrev Trace := List Event

/-- The host: a file system and servers, abstract. `read` and `write` return `none` when they fail;
a server answers a request, or fails, and may change its state. -/
structure Host where
  FS : Type
  Srv : Type
  /-- What decides failures in a file system: which files exist, and the structure around
  them. -/
  Shape : Type
  shape : FS → Shape
  read : String → FS → Option String
  write : String → String → FS → Option FS
  request : String → List Val → Srv → Option String × Srv

/-- The host's state. -/
structure World (H : Host) where
  fs : H.FS
  srv : H.Srv

/-- What `Files` needs of a host (H1 and H2): whether a read fails depends on the path and the
shape only, and whether a write fails, and the shape it leaves, on the path and the shape only,
not on the content. -/
structure Host.Lawful (H : Host) : Prop where
  read_fails : ∀ {a b : H.FS} (path : String), H.shape a = H.shape b →
    ((H.read path a).isSome ↔ (H.read path b).isSome)
  write_shape : ∀ {a b : H.FS} (path c c' : String), H.shape a = H.shape b →
    (H.write path c a).map H.shape = (H.write path c' b).map H.shape

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

/-- The sum of a list of integers; `none` when an item is not one. -/
def sumInts : List Val → Option Int
  | [] => some 0
  | .int n :: rest => (sumInts rest).map (n + ·)
  | _ => none

/-- A primitive on the values of its arguments. -/
def evalPrim : Prim → List Val → Option Val
  | .add, [.int a, .int b] => some (.int (a + b))
  | .sub, [.int a, .int b] => some (.int (a - b))
  | .lt, [.int a, .int b] => some (.bool (decide (a < b)))
  | .eqInt, [.int a, .int b] => some (.bool (decide (a = b)))
  | .not, [.bool b] => some (.bool (!b))
  | .len, [.list items] => some (.int items.length)
  | .len, [.text s] => some (.int s.length)
  | .sum, [.list items] => (sumInts items).map .int
  | _, _ => none

/-- The parameters bound to the arguments, in front of `env`. -/
def bindParams {α : Type} (params : List (String × α)) (args : List Val) (env : Env) : Env :=
  (params.map Prod.fst).zip args ++ env

variable {H : Host}

/-- A run: its trace, how it ended, and the world it leaves. -/
abbrev Run (H : Host) := Trace × Res × World H

/-- The result of a `let`: the body runs when the bound value is a value, in the world the bound
code left; a failure, a stuck evaluation or the end of fuel ends the whole. -/
def andThen (first : Run H) (rest : World H → Val → Run H) : Run H :=
  match first with
  | (t, .ok v, w) =>
    match rest w v with
    | (t', r, w') => (t ++ t', r, w')
  | (t, r, w) => (t, r, w)

/-- A value ends the run. -/
def done (w : World H) (v : Val) : Run H := ([], .ok v, w)

def stuck (w : World H) : Run H := ([], .stuck, w)

/-- A call of the function value `fv` on `vs`, with `run` the evaluation of a body. -/
def applyWith (run : World H → Env → Expr → Run H) (w : World H) (fv : Val) (vs : List Val) :
    Run H :=
  match fv with
  | .closure params body cenv =>
    if vs.length = params.length then run w (bindParams params vs cenv) body else stuck w
  | _ => stuck w

/-- The semantic functions of the prelude's higher-order functions (`spec/14 §1`), with `app`
the call of the function value: `map` calls it once per element, in order. -/
def walkMap (app : World H → Val → Run H) (w : World H) : List Val → Run H
  | [] => done w (.list [])
  | v :: vs =>
    andThen (app w v) fun w r =>
      andThen (walkMap app w vs) fun w rest =>
        match rest with
        | .list rs => done w (.list (r :: rs))
        | _ => stuck w

def walkFilter (app : World H → Val → Run H) (w : World H) : List Val → Run H
  | [] => done w (.list [])
  | v :: vs =>
    andThen (app w v) fun w b =>
      match b with
      | .bool keep =>
        andThen (walkFilter app w vs) fun w rest =>
          match rest with
          | .list rs => done w (.list (if keep then v :: rs else rs))
          | _ => stuck w
      | _ => stuck w

/-- `first_where` stops at the first element its predicate accepts. -/
def walkFirst (app : World H → Val → Run H) (w : World H) : List Val → Run H
  | [] => done w .none
  | v :: vs =>
    andThen (app w v) fun w b =>
      match b with
      | .bool true => done w (.some v)
      | .bool false => walkFirst app w vs
      | _ => stuck w

def walkFold (app : World H → Val → Val → Run H) (w : World H) (acc : Val) : List Val → Run H
  | [] => done w acc
  | v :: vs => andThen (app w acc v) fun w acc' => walkFold app w acc' vs

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
def parseInt (w : World H) (s : String) : Run H :=
  match s.toInt? with
  | some n => done w (.int n)
  | none => ([], .failed hostFailure, w)

mutual
  def evalRhs (H : Host) : Nat → Program → World H → Env → Rhs → Run H
    | 0, _, w, _, _ => ([], .outOfFuel, w)
    | n + 1, p, w, env, r =>
      match r with
      | .atom a => ([], (evalAtom env a).elim .stuck .ok, w)
      | .prim pr args =>
        ([], ((evalAtoms env args).bind (evalPrim pr)).elim .stuck .ok, w)
      | .call f _ args =>
        match p.find f, evalAtoms env args with
        | some fn, some vs =>
          if vs.length = fn.params.length then eval H n p w (bindParams fn.params vs []) fn.body
          else stuck w
        | _, _ => stuck w
      | .apply f args =>
        match evalAtom env f, evalAtoms env args with
        | some fv, some vs => applyWith (eval H n p) w fv vs
        | _, _ => stuck w
      | .op line =>
        match evalAtom env line with
        | some v =>
          match v.line with
          | some s => ([.out s], .ok .unit, w)
          | none => stuck w
        | none => stuck w
      | .read path =>
        match evalAtom env path with
        | some (.text s) =>
          match H.read s w.fs with
          | some c => ([.read s], .ok (.text c), w)
          | none => ([.read s], .failed hostFailure, w)
        | _ => stuck w
      | .write path content =>
        match evalAtom env path, evalAtom env content with
        | some (.text s), some (.text c) =>
          match H.write s c w.fs with
          | some fs' => ([.write s c], .ok .unit, { w with fs := fs' })
          | none => ([.write s c], .failed hostFailure, w)
        | _, _ => stuck w
      | .req m args =>
        match evalAtoms env args with
        | some vs =>
          match H.request m vs w.srv with
          | (some body, srv') => ([.req m vs], .ok (.text body), { w with srv := srv' })
          | (none, srv') => ([.req m vs], .failed hostFailure, { w with srv := srv' })
        | none => stuck w
      | .construct record fields =>
        ([], ((evalAtoms env fields).map (Val.record record)).elim .stuck .ok, w)
      | .field value index =>
        match evalAtom env value with
        | some (.record _ fields) => ([], (fields[index]?).elim .stuck .ok, w)
        | _ => stuck w
      | .list items => ([], ((evalAtoms env items).map Val.list).elim .stuck .ok, w)
      | .none => done w .none
      | .some a => ([], ((evalAtom env a).map Val.some).elim .stuck .ok, w)
      | .lambda params body => done w (.closure params body env)
      | .funRef f _ =>
        match p.find f with
        | some fn => done w (.closure fn.params fn.body [])
        | none => stuck w
      | .declassify pol a =>
        match p.policy pol, evalAtom env a with
        | some po, some v =>
          match p.find po.via with
          | some fn =>
            if 1 = fn.params.length then
              match eval H n p w (bindParams fn.params [v] []) fn.body with
              | (t, .ok r, w') => (t ++ [.release pol r], .ok r, w')
              | res => res
            else stuck w
          | none => stuck w
        | _, _ => stuck w
      | .map xs f =>
        match evalAtom env xs, evalAtom env f with
        | some (.list items), some fv =>
          walkMap (fun w v => applyWith (eval H n p) w fv [v]) w items
        | _, _ => stuck w
      | .filter xs keep =>
        match evalAtom env xs, evalAtom env keep with
        | some (.list items), some fv =>
          walkFilter (fun w v => applyWith (eval H n p) w fv [v]) w items
        | _, _ => stuck w
      | .firstWhere xs keep =>
        match evalAtom env xs, evalAtom env keep with
        | some (.list items), some fv =>
          walkFirst (fun w v => applyWith (eval H n p) w fv [v]) w items
        | _, _ => stuck w
      | .fold xs init f =>
        match evalAtom env xs, evalAtom env init, evalAtom env f with
        | some (.list items), some acc, some fv =>
          walkFold (fun w a v => applyWith (eval H n p) w fv [a, v]) w acc items
        | _, _, _ => stuck w
      | .parse s =>
        match evalAtom env s with
        | some (.text t) => parseInt w t
        | _ => stuck w
      | .eq a b =>
        match evalAtom env a, evalAtom env b with
        | some va, some vb => done w (.bool (Val.beq va vb))
        | _, _ => stuck w

  def eval (H : Host) : Nat → Program → World H → Env → Expr → Run H
    | 0, _, w, _, _ => ([], .outOfFuel, w)
    | n + 1, p, w, env, e =>
      match e with
      | .atom a => ([], (evalAtom env a).elim .stuck .ok, w)
      | .letR x _ value body =>
        andThen (evalRhs H n p w env value) (fun w v => eval H n p w ((x, v) :: env) body)
      | .letE x _ value body =>
        andThen (eval H n p w env value) (fun w v => eval H n p w ((x, v) :: env) body)
      | .ite cond thenE elseE =>
        match evalAtom env cond with
        | some (.bool true) => eval H n p w env thenE
        | some (.bool false) => eval H n p w env elseE
        | _ => stuck w
      | .caseOption scrutinee noneE x someE =>
        match evalAtom env scrutinee with
        | some .none => eval H n p w env noneE
        | some (.some v) => eval H n p w ((x, v) :: env) someE
        | _ => stuck w
      | .caseResult scrutinee x okE y errE =>
        match evalAtom env scrutinee with
        | some (.ok v) => eval H n p w ((x, v) :: env) okE
        | some (.err code) => eval H n p w ((y, .int code) :: env) errE
        | _ => stuck w
      | .fail code => ([], .failed code, w)
      | .attempt body =>
        match eval H n p w env body with
        | (t, .ok v, w') => (t, .ok (.ok v), w')
        | (t, .failed code, w') => (t, .ok (.err code), w')
        | res => res
      | .handle body x handler =>
        match eval H n p w env body with
        | (t, .failed code, w') =>
          match eval H n p w' ((x, .int code) :: env) handler with
          | (t', r, w'') => (t ++ t', r, w'')
        | res => res
end

end Una.F1H
