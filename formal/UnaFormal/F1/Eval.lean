/-
The big-step semantics of F1a (`spec/11`), with fuel: evaluation takes a number of steps it may
use, and running out of them is its own result, so that a program that does not terminate
has a semantics too (T1 and T6 are termination-insensitive, `spec/10 §10`). The trace holds
the events of the operation, in order (`spec/11 §5`).
-/
import UnaFormal.F1.Syntax

namespace Una.F1

/-- The values of F1a. A closure keeps the environment where its lambda was made. -/
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
  | closure (params : List (String × Ty)) (body : Expr) (env : List (String × Val))

/-- An environment: the values of the variables in scope, innermost first. -/
abbrev Env := List (String × Val)

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
def bindParams (params : List (String × Ty)) (args : List Val) (env : Env) : Env :=
  (params.map Prod.fst).zip args ++ env

/-- The function `f` of the program. -/
def Program.find (p : Program) (f : String) : Option Fun :=
  p.funs.find? (fun fn => fn.name = f)

/-- The result of a `let`: the body runs when the bound value is a value; a failure, a stuck
evaluation or the end of fuel ends the whole. -/
def andThen (first : List String × Res) (rest : Val → List String × Res) :
    List String × Res :=
  match first with
  | (t, .ok v) =>
    match rest v with
    | (t', r) => (t ++ t', r)
  | (t, r) => (t, r)

mutual
  def evalRhs : Nat → Program → Env → Rhs → List String × Res
    | 0, _, _, _ => ([], .outOfFuel)
    | n + 1, p, env, r =>
      match r with
      | .atom a => ([], (evalAtom env a).elim .stuck .ok)
      | .prim pr args =>
        ([], ((evalAtoms env args).bind (evalPrim pr)).elim .stuck .ok)
      | .call f args =>
        match p.find f, evalAtoms env args with
        | some fn, some vs =>
          if vs.length = fn.params.length then eval n p (bindParams fn.params vs []) fn.body
          else ([], .stuck)
        | _, _ => ([], .stuck)
      | .apply f args =>
        match evalAtom env f, evalAtoms env args with
        | some (.closure params body cenv), some vs =>
          if vs.length = params.length then eval n p (bindParams params vs cenv) body
          else ([], .stuck)
        | _, _ => ([], .stuck)
      | .op line =>
        match evalAtom env line with
        | some (.text s) => ([s], .ok .unit)
        | _ => ([], .stuck)
      | .construct record fields =>
        ([], ((evalAtoms env fields).map (Val.record record)).elim .stuck .ok)
      | .field value index =>
        match evalAtom env value with
        | some (.record _ fields) => ([], (fields[index]?).elim .stuck .ok)
        | _ => ([], .stuck)
      | .list items => ([], ((evalAtoms env items).map Val.list).elim .stuck .ok)
      | .none => ([], .ok .none)
      | .some a => ([], ((evalAtom env a).map Val.some).elim .stuck .ok)
      | .lambda params body => ([], .ok (.closure params body env))

  def eval : Nat → Program → Env → Expr → List String × Res
    | 0, _, _, _ => ([], .outOfFuel)
    | n + 1, p, env, e =>
      match e with
      | .atom a => ([], (evalAtom env a).elim .stuck .ok)
      | .letR x _ value body => andThen (evalRhs n p env value) (fun v => eval n p ((x, v) :: env) body)
      | .letE x _ value body => andThen (eval n p env value) (fun v => eval n p ((x, v) :: env) body)
      | .ite cond thenE elseE =>
        match evalAtom env cond with
        | some (.bool true) => eval n p env thenE
        | some (.bool false) => eval n p env elseE
        | _ => ([], .stuck)
      | .caseOption scrutinee noneE x someE =>
        match evalAtom env scrutinee with
        | some .none => eval n p env noneE
        | some (.some v) => eval n p ((x, v) :: env) someE
        | _ => ([], .stuck)
      | .fail code => ([], .failed code)
      | .attempt body =>
        match eval n p env body with
        | (t, .ok v) => (t, .ok (.ok v))
        | (t, .failed code) => (t, .ok (.err code))
        | (t, r) => (t, r)
      | .handle body x handler =>
        match eval n p env body with
        | (t, .failed code) =>
          match eval n p ((x, .int code) :: env) handler with
          | (t', r) => (t ++ t', r)
        | (t, r) => (t, r)
end

end Una.F1
