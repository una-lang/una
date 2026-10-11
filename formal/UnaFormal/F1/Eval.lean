/-
The big-step semantics of F1 (`spec/11`), with fuel: evaluation takes a number of steps it may
use, and running out of them is its own result, so that a program that does not terminate has
a semantics too (T1 and T6 are termination-insensitive, `spec/10 §10`). The trace holds the
events of the run, in order (`spec/11 §5`).

The host is state (M5). A `Host` gives the file system and the servers as functions, so any
host is an instance, and the properties the theorem needs of it are its hypotheses
(`Host.Lawful`, H1 and H2 of `docs/plan/formal-track-m5.md` §2). A run threads the `World`, the
host's state, and ends with the world it leaves, which the next run starts from.
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
  /-- `Err(v)`: a failure's code, as `attempt` gives it, is `.err (.int c)` (#508). -/
  | err (value : Val)
  | record (name : String) (fields : List Val)
  /-- A value of a sum kind (#512): its variant, by index, and its fields. -/
  | variant (kind : String) (ctor : Nat) (fields : List Val)
  | closure (params : List (String × LTy)) (body : Expr) (env : List (String × Val))
  | tuple (items : List Val)

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

mutual
  /-- A value without a closure in it. -/
  def Val.FirstOrder : Val → Prop
    | .closure _ _ _ => False
    | .list items => Val.FirstOrderAll items
    | .some v | .ok v | .err v => v.FirstOrder
    | .record _ fs | .variant _ _ fs | .tuple fs => Val.FirstOrderAll fs
    | .int _ | .bool _ | .text _ | .unit | .none => True

  def Val.FirstOrderAll : List Val → Prop
    | [] => True
    | v :: vs => v.FirstOrder ∧ Val.FirstOrderAll vs
end

/-- A response of `Http` (`spec/24 §1`): `Response(status, headers, body)`, with each header a
`Header(name, value)` of texts. -/
def Val.IsResponse : Val → Prop
  | .record "prelude.Response" [.int _, .list hs, .text _] =>
    ∀ h ∈ hs, ∃ n x, h = .record "prelude.Header" [.text n, .text x]
  | _ => False

/-- The host: a file system and servers, abstract. `read` and `write` return `none` when they fail;
a server answers a request with a response, or fails with the code of its `HttpError` (its index in
the declaration order of `spec/24 §1`), and may change its state. A request is deterministic in its
method, its arguments and the server's state: the network's timing, and its other
nondeterminism, are outside the model, as timing is elsewhere (`spec/07 §1`). -/
structure Host where
  FS : Type
  Srv : Type
  /-- What decides failures in a file system: which files exist, and the structure around
  them. -/
  Shape : Type
  shape : FS → Shape
  read : String → FS → Option String
  write : String → String → FS → Option FS
  request : String → List Val → Srv → Except Nat Val × Srv

/-- The host's state. -/
structure World (H : Host) where
  fs : H.FS
  srv : H.Srv
  /-- The process environment `Env.get` reads (#487): part of the world, so two runs may read
  different values; no operation of the program changes it. A variable that is not valid UTF-8
  aborts the run (S-16, ADR 0086), which is termination, so the model gives texts. -/
  env : String → Option String := fun _ => none

/-- **H4**: every response of the host is a `Response` (M5c). -/
def Host.RespTyped (H : Host) : Prop :=
  ∀ (m : String) (vs : List Val) (s s' : H.Srv) (v : Val), H.request m vs s = (.ok v, s') → v.IsResponse

/-- What the theorems need of a host. `Files` (H1 and H2): whether a read fails depends on the path
and the shape only, and whether a write fails, and the shape it leaves, on the path and the shape
only, not on the content. `Http` (H4, M5c): every response is a `Response`, as the real host
builds it (`una-rt`'s `http`: digits for the status, UTF-8 for header names and the body, or
`Protocol`, and header values decoded lossily). -/
structure Host.Lawful (H : Host) : Prop where
  read_fails : ∀ {a b : H.FS} (path : String), H.shape a = H.shape b →
    ((H.read path a).isSome ↔ (H.read path b).isSome)
  write_shape : ∀ {a b : H.FS} (path c c' : String), H.shape a = H.shape b →
    (H.write path c a).map H.shape = (H.write path c' b).map H.shape
  resp_typed : H.RespTyped

/-- The host without state, on which every host operation fails: the host of the examples whose
programs perform none. -/
def Host.unit : Host where
  FS := Unit
  Srv := Unit
  Shape := Unit
  shape := fun _ => ()
  read := fun _ _ => none
  write := fun _ _ _ => none
  request := fun _ _ s => (.error 0, s)

theorem Host.unit_respTyped : Host.unit.RespTyped := fun _ _ _ _ _ h => nomatch h

theorem Host.unit_lawful : Host.unit.Lawful :=
  ⟨fun _ _ => Iff.rfl, fun _ _ _ _ => rfl, Host.unit_respTyped⟩

/-- The code of a failure of the host or of `parse`. -/
def hostFailure : Nat := 0

/-- How an evaluation ends: with a value, with a failure, stuck (no rule applies, which T1
says never happens to a well-typed program), or out of fuel. -/
inductive Res where
  | ok (v : Val)
  | failed (code : Nat)
  | stuck
  | outOfFuel
  /-- A defined runtime abort of Una (#526): a zero divisor (`Prim.aborts`). Like `outOfFuel`, the run
  does not terminate normally, so T1 counts it good and T6's `Term` excludes it (`spec/07 §1`:
  aborts and termination are outside the model); unlike it, it is not an artifact of the fuel. -/
  | aborted
  /-- A failure of a payload kind (#512): its kind, its variant's index and its payload. A failure
  of a kind without fields is `failed`, its code. -/
  | raised (kind : String) (code : Nat) (payload : List Val)

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

/-- The join of a list of texts; `none` when an item is not one. -/
def concatTexts : List Val → Option String
  | [] => some ""
  | .text s :: rest => (concatTexts rest).map (s ++ ·)
  | _ => none

mutual
  /-- The text of a value, as interpolation writes it: total, and the same for a function value
  whatever it captured, so that it reads no annotation. -/
  def Val.render : Val → String
    | .int n => toString n
    | .bool b => toString b
    | .text s => s
    | .unit => "unit"
    | .list items => "[" ++ Val.renders items ++ "]"
    | .none => "None"
    | .some v => "Some(" ++ v.render ++ ")"
    | .ok v => "Ok(" ++ v.render ++ ")"
    | .err v => "Err(" ++ v.render ++ ")"
    | .record r fs => r ++ "(" ++ Val.renders fs ++ ")"
    | .variant k i fs => k ++ "#" ++ toString i ++ "(" ++ Val.renders fs ++ ")"
    | .closure _ _ _ => "<fn>"
    | .tuple items => "(" ++ Val.renders items ++ ")"

  def Val.renders : List Val → String
    | [] => ""
    | v :: vs => v.render ++ ", " ++ Val.renders vs
end

/-- The sum of a list of integers; `none` when an item is not one. -/
def sumInts : List Val → Option Int
  | [] => some 0
  | .int n :: rest => (sumInts rest).map (n + ·)
  | _ => none

/-- Whether a primitive aborts on its arguments: `div` or `mod` by zero (#526), which Una's
runtime aborts on, and which the divisor obligation (U5001) rules out where it is proven. -/
def Prim.aborts : Prim → List Val → Bool
  | .div, [_, .int b] | .mod, [_, .int b] => b == 0
  | _, _ => false

/-- A primitive on the values of its arguments. -/
def evalPrim : Prim → List Val → Option Val
  | .add, [.int a, .int b] => some (.int (a + b))
  | .sub, [.int a, .int b] => some (.int (a - b))
  | .mul, [.int a, .int b] => some (.int (a * b))
  -- A zero divisor never reaches here: `Prim.aborts` stops the run first. The value `0` only keeps
  -- `evalPrim` total on typed arguments.
  | .div, [.int a, .int b] => some (.int (if b = 0 then 0 else Int.ediv a b))
  | .mod, [.int a, .int b] => some (.int (if b = 0 then 0 else Int.emod a b))
  | .lt, [.int a, .int b] => some (.bool (decide (a < b)))
  | .eqInt, [.int a, .int b] => some (.bool (decide (a = b)))
  | .not, [.bool b] => some (.bool (!b))
  | .len, [.list items] => some (.int items.length)
  | .len, [.text s] => some (.int s.length)
  | .sum, [.list items] => (sumInts items).map .int
  | .concat, vs => (concatTexts vs).map .text
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

/-- The handler an invocation of `o` runs (#538): the one its marker holds, when that is a
handler of `o`, and the operation's representative otherwise. A marker holds another value only in
a forged export, which `una check` never produces and the importer refuses; the model still runs a
handler of `o` there, so a typed program never gets stuck at an invocation. -/
def Program.handlerFor (p : Program) (o : String) (m : Option Val) : String :=
  match m with
  | some (.text f) => if p.handlerOf.lookup f = some o then f else (p.opRep.lookup o).getD ""
  | _ => (p.opRep.lookup o).getD ""


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

/-- A loop (`spec/17 §4`), with `body` the run of its body on an element and the current state:
each iteration's value must be a tuple of the state's length, the next state; the loop's value
is the final state. A failure ends the loop, and the state is dropped (`spec/17 §7` rule 1). -/
def walkLoop (body : World H → Val → List Val → Run H) (w : World H) (st : List Val) :
    List Val → Run H
  | [] => done w (.tuple st)
  | v :: vs =>
    andThen (body w v st) fun w r =>
      match r with
      | .tuple st' => if st'.length = st.length then walkLoop body w st' vs else stuck w
      | _ => stuck w

/-- The integers a range visits, in increasing order. -/
def rangeVals (low high : Int) (inclusive : Bool) : List Val :=
  (List.range (high - low + (if inclusive then 1 else 0)).toNat).map (fun i : Nat => .int (low + (i : Int)))

/-- The elements a loop visits: `none` when the source is not a list or a pair of integers. -/
def sourceVals (env : Env) : Source → Option (List Val)
  | .list xs =>
    match evalAtom env xs with
    | some (.list items) => some items
    | _ => none
  | .range low high inclusive =>
    match evalAtom env low, evalAtom env high with
    | some (.int a), some (.int b) => some (rangeVals a b inclusive)
    | _, _ => none

/-- An abort of the run (`spec/07 §1`): a run that does not end, which the termination-insensitive
T6 says nothing of. The UAM aborts at `set` out of its range (N-79). -/
def abort (w : World H) : Run H := ([], .outOfFuel, w)

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
    | .err a, .err b => Val.beq a b
    | .record r xs, .record s ys => r == s && Val.beqs xs ys
    | .variant k i xs, .variant k' i' ys => k == k' && i == i' && Val.beqs xs ys
    | _, _ => false

  def Val.beqs : List Val → List Val → Bool
    | [], [] => true
    | x :: xs, y :: ys => Val.beq x y && Val.beqs xs ys
    | _, _ => false
end

/-- A run of `checked` on `v` (#453): `v` when the predicate holds of its text, its failure
`code` otherwise, and stuck on a value that is not a scalar. -/
def checkRun (p : Program) (w : World H) (v : Val) (pred : String) (code : Nat) : Run H :=
  match v.line with
  | some s => if p.predSem pred s then done w v else ([], .failed code, w)
  | none => stuck w

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
        -- A zero divisor aborts the run, the outcome `aborted` (#526): `spec/07 §1` puts aborts outside the model, with termination.
        if ((evalAtoms env args).map (Prim.aborts pr)).getD false then ([], .aborted, w)
        else ([], ((evalAtoms env args).bind (evalPrim pr)).elim .stuck .ok, w)
      | .call f _ args =>
        match p.find f, evalAtoms env args with
        | some fn, some vs =>
          if vs.length = fn.params.length then eval H n p w (bindParams fn.params vs []) fn.body
          else stuck w
        | _, _ => stuck w
      | .invoke f args =>
        match p.find f, evalAtoms env args with
        | some fn, some vs =>
          if vs.length = fn.params.length then eval H n p w (bindParams fn.params vs []) fn.body
          else stuck w
        | _, _ => stuck w
      | .invokeOp o args =>
        match p.find (p.handlerFor o (env.lookup (markerName o))), evalAtoms env args with
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
          | (.ok v, srv') => ([.req m vs], .ok v, { w with srv := srv' })
          | (.error c, srv') => ([.req m vs], .failed c, { w with srv := srv' })
        | none => stuck w
      | .construct record fields =>
        ([], ((evalAtoms env fields).map (Val.record record)).elim .stuck .ok, w)
      | .variant kind ctor fields =>
        ([], ((evalAtoms env fields).map (Val.variant kind ctor)).elim .stuck .ok, w)
      | .field value index =>
        match evalAtom env value with
        | some (.record _ fields) => ([], (fields[index]?).elim .stuck .ok, w)
        | _ => stuck w
      | .list items => ([], ((evalAtoms env items).map Val.list).elim .stuck .ok, w)
      | .none => done w .none
      | .some a => ([], ((evalAtom env a).map Val.some).elim .stuck .ok, w)
      | .first a =>
        match evalAtom env a with
        | some (.list (v :: _)) => done w (.some v)
        | some (.list []) => done w .none
        | _ => stuck w
      | .mkOk a => ([], ((evalAtom env a).map Val.ok).elim .stuck .ok, w)
      | .mkErr a => ([], ((evalAtom env a).map Val.err).elim .stuck .ok, w)
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
              -- A failure of the `via` function is released as its value is (#453 Phase B,
              -- ADR 0090): the release carries `Err(c)`, and the call fails with `c`.
              | (t, .failed c, w') => (t ++ [.release pol (.err (.int c))], .failed c, w')
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
      | .checked a pred code =>
        match evalAtom env a with
        | some v => checkRun p w v pred code
        | none => stuck w
      | .eq a b =>
        match evalAtom env a, evalAtom env b with
        | some va, some vb => done w (.bool (Val.beq va vb))
        | _, _ => stuck w
      | .tuple items => ([], ((evalAtoms env items).map Val.tuple).elim .stuck .ok, w)
      | .append xs item =>
        match evalAtom env xs, evalAtom env item with
        | some (.list items), some v => done w (.list (items ++ [v]))
        | _, _ => stuck w
      | .toText a =>
        match evalAtom env a with
        | some v => done w (.text v.render)
        | none => stuck w
      | .envGet name =>
        match evalAtom env name with
        | some (.text s) => done w ((w.env s).elim .none (fun x => .some (.text x)))
        | _ => stuck w
      | .set xs index item =>
        match evalAtom env xs, evalAtom env index, evalAtom env item with
        | some (.list items), some (.int i), some v =>
          if 0 ≤ i ∧ i < items.length then done w (.tuple [.list (items.set i.toNat v), .unit])
          else abort w
        | _, _, _ => stuck w

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
        | some (.err v) => eval H n p w ((y, v) :: env) errE
        | _ => stuck w
      | .fail code => ([], .failed code, w)
      | .raise kind code args =>
        match evalAtoms env args with
        | some vs => ([], .raised kind code vs, w)
        | none => stuck w
      | .caseSum scrutinee ctor binders thenE elseE =>
        match evalAtom env scrutinee with
        | some (.variant _ i fs) =>
          if i = ctor then
            if fs.length = binders.length then eval H n p w (binders.zip fs ++ env) thenE
            else stuck w
          else eval H n p w env elseE
        | _ => stuck w
      | .attempt body =>
        match eval H n p w env body with
        | (t, .ok v, w') => (t, .ok (.ok v), w')
        | (t, .failed code, w') => (t, .ok (.err (.int code)), w')
        | res => res
      | .unreachable => ([], .aborted, w)
      | .withH o f body => eval H n p w ((markerName o, .text f) :: env) body
      | .attemptK kind body =>
        match eval H n p w env body with
        | (t, .ok v, w') => (t, .ok (.ok v), w')
        | (t, .raised k code vs, w') =>
          if k = kind then (t, .ok (.err (.variant k code vs)), w') else (t, .raised k code vs, w')
        | res => res
      | .handle body x handler =>
        match eval H n p w env body with
        | (t, .failed code, w') =>
          match eval H n p w' ((x, .int code) :: env) handler with
          | (t', r, w'') => (t ++ t', r, w'')
        | res => res
      | .handleK kind body x handler =>
        match eval H n p w env body with
        | (t, .raised k code vs, w') =>
          if k = kind then
            match eval H n p w' ((x, .variant k code vs) :: env) handler with
            | (t', r, w'') => (t ++ t', r, w'')
          else (t, .raised k code vs, w')
        | res => res
      | .letTuple binders value body =>
        match evalAtom env value with
        | some (.tuple vs) =>
          if vs.length = binders.length then eval H n p w (bindParams binders vs env) body
          else stuck w
        | _ => stuck w
      | .loop source x _ state init body =>
        match sourceVals env source, evalAtoms env init with
        | some items, some st =>
          if st.length = state.length then
            walkLoop (fun w v st => eval H n p w ((x, v) :: bindParams state st env) body) w st items
          else stuck w
        | _, _ => stuck w
end

end Una.F1
