/-
The reading of `una.core/1` (ADR 0083, `spec/13 §8`) into F1, for the checker of M2.

Each function declaration of a complete document is read on its own. A declaration that uses a
form F1 lacks is reported as not re-checked, with the form, and never as accepted (ADR 0083
rule 3). The reading is part of the trusted base of the re-check (`docs/plan/formal-track.md`
§5), so it maps each form to the F1 form with the same meaning and nothing else:

- Core's `gt a b` is F1's `lt b a`, and Core's `eq` is F1's `==` on any type with equality;
- the prelude's `map`, `filter`, `first_where`, `fold` and `parse_int` are F1's axioms of them;
- `Option` and `Result` of the prelude are F1's options and results, with the result's error type
  (#508). F1's failures are codes, so a failure with a payload, or a handler that reads the failure,
  is outside F1 (#512): an error type that is a kind without payload is read as `Int`, its code,
  with the kind's own labels (#507), and any other error type is read as a type, so a kind with a
  payload is not re-checked;
- a kind with one variant and no type parameters is a record;
- F1 has one operation, from a scalar (`Int`, `Bool` or `Text`) to `Unit`, which never fails: a function is read only when its body,
  its types and the signatures of what it calls mention at most one operation, of that shape,
  whose `accepts` becomes F1's;
- `⊤` is `restricted` with every category the document names.

A declaration is accepted only when everything it reaches, except handler bodies, is accepted
too (`checkDocumentWith`). F1's operation is an event of the trace: what the program's handler
of it does is not modeled, so a handler's body is not in the reach set. That is M2's
assumption, stated in `docs/plan/formal-track-m2.md` §1.
-/
import Lean.Data.Json
import UnaFormal.F1.CheckProgram
import UnaFormal.F1.HostFreeReach

namespace Una.F1

/-- `t` with the symbolic labels `ℓ*_x`, `x ∈ xs`, added to its top label. -/
def LTy.addStarTop (xs : List String) : LTy → LTy
  | .int l => .int { l with star := l.star ++ xs }
  | .bool l => .bool { l with star := l.star ++ xs }
  | .text l => .text { l with star := l.star ++ xs }
  | .unit l => .unit { l with star := l.star ++ xs }
  | .list t l => .list t { l with star := l.star ++ xs }
  | .option t l => .option t { l with star := l.star ++ xs }
  | .result t e l => .result t e { l with star := l.star ++ xs }
  | .record r l => .record r { l with star := l.star ++ xs }
  | .fn ps r o g f l => .fn ps r o g f { l with star := l.star ++ xs }
  | .var a l => .var a { l with star := l.star ++ xs }
  | .tuple ts l => .tuple ts { l with star := l.star ++ xs }

/-- `t` with `ℓ*_x`, `x ∈ xs`, added at the position `steps` of a result summary, as
`spec/13 §5` writes it (`element`, `arg N`, `result`); `none` at a position F1 does not have. -/
def LTy.addStarAt (xs : List String) : List String → LTy → Option LTy
  | [], t => some (t.addStarTop xs)
  | "element" :: rest, .list t l => (LTy.addStarAt xs rest t).map (.list · l)
  | "arg 1" :: rest, .option t l => (LTy.addStarAt xs rest t).map (.option · l)
  | "arg 1" :: rest, .result t e l => (LTy.addStarAt xs rest t).map (.result · e l)
  | "result" :: rest, .fn ps r o g f l => (LTy.addStarAt xs rest r).map (.fn ps · o g f l)
  | _, _ => none

mutual
  /-- `t` with `ℓ*_x`, `x ∈ xs`, added at every position: edition 0.1 raises every position of
  a generic function's result by every type argument (D-137). -/
  def LTy.addStarAll (xs : List String) : LTy → LTy
    | .list t l => (LTy.list (LTy.addStarAll xs t) l).addStarTop xs
    | .option t l => (LTy.option (LTy.addStarAll xs t) l).addStarTop xs
    | .result t e l => (LTy.result (LTy.addStarAll xs t) (LTy.addStarAll xs e) l).addStarTop xs
    | .fn ps r o g f l => (LTy.fn (LTy.addStarAlls xs ps) (LTy.addStarAll xs r) o g f l).addStarTop xs
    | t => t.addStarTop xs

  def LTy.addStarAlls (xs : List String) : List LTy → List LTy
    | [] => []
    | t :: ts => LTy.addStarAll xs t :: LTy.addStarAlls xs ts
end

/-- A function type that may perform effects (`op`) or fail, at the top of `t`. -/
def LTy.withFlags (op fails : Bool) (host : HostFx := {}) : LTy → LTy
  | .fn ps r o g f l => .fn ps r (o || op) (g.or host) (f || fails) l
  | t => t

/-- F1's failures are codes (#508): every `attempt` in a tail position of `e`, which gives `e`'s
value, has a failure's code as its error type at `t`, the type `e` is checked at. A loop's body
gives its state, a tuple, so an `attempt` there has none; an `attempt`'s body gives its result's
value. An `attempt` whose error type is a kind with a payload, or a type the export leaves open,
is not a shape F1 has (#512): its declaration is not re-checked, never rejected. -/
partial def Expr.attemptsAt (t : LTy) : Expr → Bool
  | .attempt b => match t with
    | .result v (.int _) _ => b.attemptsAt v
    | _ => false
  | .letR _ _ _ b | .letE _ _ _ b | .letTuple _ _ b => b.attemptsAt t
  | .ite _ a b | .caseOption _ a _ b | .caseResult _ _ a _ b | .handle a _ b =>
    a.attemptsAt t && b.attemptsAt t
  | .loop _ _ _ st _ b => b.attemptsAt (.tuple (st.map Prod.snd) .bot)
  | .atom _ | .fail _ => true

end Una.F1

namespace Una.F1.Import

abbrev J := Lean.Json

/-- What reading one declaration gathers: the operations it mentions, as capability and
operation names, and the functions and policies it refers to. -/
structure Refs where
  ops : List (String × String) := []
  /-- The host effects it mentions: `Files.read` and `Files.write` (M5b). -/
  host : HostFx := {}
  funs : List String := []
  policies : List String := []
  /-- Whether a call's type arguments mention type parameters: two levels of generics, which
  the checker covers at the function's instances. -/
  nested : Bool := false

/-- Reading fails with the reason a declaration is outside F1. -/
abbrev M := StateT Refs (Except String)

def field (j : J) (k : String) : M J :=
  match j.getObjVal? k with
  | .ok v => pure v
  | .error _ => throw s!"a node without `{k}`"

def str (j : J) : M String :=
  match j.getStr? with
  | .ok s => pure s
  | .error _ => throw "a string expected"

def arr (j : J) : M (List J) :=
  match j.getArr? with
  | .ok a => pure a.toList
  | .error _ => throw "an array expected"

def kind (j : J) : M String := do str (← field j "kind")

def isNull (j : J) : Bool :=
  match j with
  | .null => true
  | _ => false

/-- `module.name`. -/
def path (j : J) : M String := do
  pure s!"{← str (← field j "module")}.{← str (← field j "name")}"

def level : String → M Level
  | "public" => pure .«public»
  | "internal" => pure .internal
  | "confidential" => pure .confidential
  | "secret" => pure .secret
  | "restricted" => pure .restricted
  | l => throw s!"the level `{l}`"

def label (j : J) : M LExpr := do
  let lv ← level (← str (← field j "level"))
  let cats ← (← arr (← field j "categories")).mapM str
  let syms ← (← arr (← field j "symbols")).mapM str
  let mut plain : List String := []
  let mut star : List String := []
  for s in syms do
    if s.startsWith "?" then throw "a label variable"
    if s.endsWith "*" then star := star ++ [String.ofList s.toList.dropLast]
    else plain := plain ++ [s]
  pure ⟨⟨lv, cats⟩, plain, star⟩

/-- The kinds of the document that are records, by qualified name. -/
structure Ctx where
  records : List String
  funs : List String
  /-- The host effects the function being read declares: a lambda given to an `of` parameter
  performs its caller's effects (ADR 0062), the host's included (M6b). -/
  host : HostFx := {}
  /-- The `via` functions of the document's policies, with their policy. -/
  vias : List (String × String) := []
  /-- Edition 0.1 has no `Eq` constraint: `==` applies to values of any type parameter
  (U2026 is edition 0.2's). The reading constrains every type parameter of a function whose
  body compares values by `Eq`, which only adds the check, at each call, that the type argument
  has equality. -/
  allEq : Bool := false
  /-- The policies each function may release through (`Fun.rels`), computed over the document. -/
  rels : List (String × List String) := []
  /-- The kinds whose values F1's failure codes represent faithfully, by qualified name: kinds
  without type parameters whose variants carry no payload (#507). -/
  codes : List String := []

/-- The effects a `uses` entry names: `Files`'s operations (M5b) and `Http`'s (M5c) are host
effects; every other operation is F1's one operation, gathered for G3. -/
def usesEffects (uses : List J) : M (Bool × HostFx) := do
  let mut op := false
  let mut host : HostFx := {}
  for u in uses do
    let cap ← str (← field u "capability")
    for o in ← arr (← field u "ops") do
      let o ← str o
      if cap == "Files" then
        if o == "read" then host := { host with rd := true }
        else if o == "write" then host := { host with wr := true }
        else throw s!"the operation `Files.{o}`"
        modify fun s => { s with host := s.host.or host }
      else if cap == "Http" then
        if ["get", "post", "put", "delete"].contains o then host := { host with rq := true }
        else throw s!"the operation `Http.{o}`"
        modify fun s => { s with host := s.host.or host }
      else if cap == "Env" && o == "get" then
        -- `Env.get` is the model's `envGet`, which performs no event (M6b).
        pure ()
      else
        op := true
        modify fun s => { s with ops := if s.ops.contains (cap, o) then s.ops else s.ops ++ [(cap, o)] }
  pure (op, host)

partial def ty (c : Ctx) (j : J) : M LTy := do
  let l ← label (← field j "label")
  match ← kind j with
  | "fn" =>
    let ps ← (← arr (← field j "params")).mapM (ty c)
    let r ← ty c (← field j "result")
    let (op, host) ← usesEffects (← arr (← field j "uses"))
    pure (.fn ps r op host (!isNull (← field j "fails")) l)
  | "base" =>
    let b ← field j "base"
    match ← kind b with
    | "int" => pure (.int l)
    | "bool" => pure (.bool l)
    | "text" => pure (.text l)
    | "unit" => pure (.unit l)
    | "param" =>
      -- The checker labels a value of `T` with `ℓ_T`, in place; a signature writes `T` alone.
      -- Both are the same type at every instance, since instantiation drops `ℓ_T` at a position
      -- of type `T` (`LTy.lsubst`, `LExpr.dropPlain`), so the reading drops it.
      let a ← str (← field b "value")
      -- `?` is a type the export leaves open (an inferred error type of `fails of`), not a type
      -- parameter: no rule reads it (#508).
      if a == "?" then throw "a type the export leaves open (`?`)"
      pure (.var a (l.dropPlain a))
    | "named" =>
      let q ← path b
      -- A kind is a record or a failure's code, never both (#509); a document where one name is
      -- both is not one F1 reads.
      if c.records.contains q && c.codes.contains q then
        throw s!"the kind `{q}`, both a record and a failure's code"
      let args ← arr (← field b "args")
      match q, args with
      | "prelude.List", [t] => pure (.list (← ty c t) l)
      | "prelude.Option", [t] => pure (.option (← ty c t) l)
      | "prelude.Result", [t, e] =>
        -- The error type is kept (#508), so `to_text` and `==` see its labels. A kind without
        -- payload is a failure's code, F1's `Int`, with the kind's own labels (#507); any other
        -- error type is read as a type, and a kind with a payload is not one F1 has (#512).
        let code ← match ← kind e with
          | "base" =>
            let eb ← field e "base"
            if (← kind eb) == "named" then
              if (← arr (← field eb "args")).isEmpty then
                let q ← path eb
                if c.records.contains q && c.codes.contains q then
                  throw s!"the kind `{q}`, both a record and a failure's code"
                pure (c.codes.contains q)
              else pure false
            else pure false
          | _ => pure false
        let et ← if code then pure (.int (← label (← field e "label"))) else ty c e
        pure (.result (← ty c t) et l)
      | _, [] => if c.records.contains q then pure (.record q l) else throw s!"the type `{q}`"
      | _, _ => throw s!"the type `{q}` with type arguments"
    | "tuple" => pure (.tuple (← (← arr (← field b "value")).mapM (ty c)) l)
    | k => throw s!"the type `{k}`"
  | k => throw s!"the type `{k}`"

def atom (j : J) : M Atom := do
  match ← kind j with
  | "var" => pure (.var (← str (← field j "value")))
  | "int" =>
    match (← str (← field j "value")).toInt? with
    | some n => pure (.int n)
    | none => throw "an integer literal"
  | "bool" =>
    match (← field j "value").getBool? with
    | .ok b => pure (.bool b)
    | .error _ => throw "a boolean literal"
  | "text" => pure (.text (← str (← field j "value")))
  | "unit" => pure .unit
  | k => throw s!"the atom `{k}`"

def atoms (j : J) : M (List Atom) := do (← arr j).mapM atom

/-- Whether the name `x` is read anywhere in `j`. -/
partial def reads (x : String) (j : J) : Bool :=
  match j with
  | .obj kvs =>
    (j.getObjValD "kind" == .str "var" && j.getObjValD "value" == .str x) ||
      kvs.toArray.any (fun kv => reads x kv.2)
  | .arr items => items.any (reads x)
  | _ => false

/-- Whether `j` applies `==` or `!=`. -/
partial def comparesValues (j : J) : Bool :=
  match j with
  | .obj kvs =>
    (j.getObjValD "kind" == .str "prim" &&
      ((j.getObjValD "prim").getObjValD "kind" == .str "eq" ||
        (j.getObjValD "prim").getObjValD "kind" == .str "ne")) ||
      kvs.toArray.any (fun kv => comparesValues kv.2)
  | .arr items => items.any comparesValues
  | _ => false

def refFun (f : String) : M Unit :=
  modify fun s => { s with funs := if s.funs.contains f then s.funs else s.funs ++ [f] }

def refPolicy (pol : String) : M Unit :=
  modify fun s => { s with policies := if s.policies.contains pol then s.policies else s.policies ++ [pol] }

/-- A binder `[name, type]`. -/
def binder (c : Ctx) (j : J) : M (String × LTy) := do
  match ← arr j with
  | [x, t] => pure (← str x, ← ty c t)
  | _ => throw "a binder"

/-- The name a pattern binds, or one nothing reads. -/
def bindName (j : J) : M String := if isNull j then pure "%unread" else str j

/-- The variables a refinement formula of the export reads (#453): its `var` nodes. -/
partial def formulaVars (j : J) : List String :=
  match j with
  | .obj kvs =>
    let here := match j.getObjValD "kind", j.getObjValD "value" with
      | .str "var", .str x => [x]
      | _, _ => []
    here ++ kvs.toArray.toList.flatMap (fun kv => formulaVars kv.2)
  | .arr items => items.toList.flatMap formulaVars
  | _ => []

mutual
  partial def rhs (c : Ctx) (j : J) : M Rhs := do
    match ← kind j with
    | "atom" => pure (.atom (← atom (← field j "value")))
    | "prim" =>
      let pk ← kind (← field j "prim")
      let args ← atoms (← field j "args")
      match pk, args with
      | "add", _ => pure (.prim .add args)
      | "sub", _ => pure (.prim .sub args)
      | "lt", _ => pure (.prim .lt args)
      | "gt", [a, b] => pure (.prim .lt [b, a])
      | "not", _ => pure (.prim .not args)
      | "len", _ => pure (.prim .len args)
      | "eq", [a, b] => pure (.eq a b)
      | "concat", _ => pure (.prim .concat args)
      | "to_text", [a] => pure (.toText a)
      | k, _ => throw s!"the primitive `{k}`"
    | "call" =>
      if (j.getObjVal? "predicate").toBool then throw "a call that names a predicate too"
      let f ← path (← field j "function")
      let args ← atoms (← field j "args")
      match f, args with
      | "prelude.map", [xs, g] => pure (.map xs g)
      | "prelude.filter", [xs, g] => pure (.filter xs g)
      | "prelude.first_where", [xs, g] => pure (.firstWhere xs g)
      | "prelude.fold", [xs, i, g] => pure (.fold xs i g)
      | "prelude.parse_int", [s] => pure (.parse s)
      | "prelude.sum", [xs] => pure (.prim .sum [xs])
      | "prelude.append", [xs, x] => pure (.append xs x)
      | "prelude.set", [xs, i, x] => pure (.set xs i x)
      | _, _ =>
        if c.funs.contains f then
          refFun f
          -- A call of a `via` function brings its policy, which makes the call U4005.
          if let some pol := List.lookup f c.vias then refPolicy pol
          let targs ← (← arr (← field j "type_args")).mapM (ty c)
          if !Check.groundAll targs then modify fun s => { s with nested := true }
          pure (.call f targs args)
        else throw s!"a call of `{f}`"
    | "apply" => pure (.apply (← atom (← field j "callee")) (← atoms (← field j "args")))
    | "op" =>
      let cap ← str (← field j "capability")
      let o ← str (← field j "op")
      -- The host's capabilities are named by their path, and a node whose `capability` field
      -- is not its path's name is not read (Una-1's finding on #471).
      let (std, pathName) : Bool × J := match j.getObjVal? "capability_path" with
        | .ok cp => (cp.getObjValD "module" == Lean.Json.str "std.host", cp.getObjValD "name")
        | .error _ => (false, Lean.Json.null)
      if pathName != Lean.Json.null && pathName != Lean.Json.str cap then
        throw s!"an operation whose capability `{cap}` is not its path's"
      -- The host's stateful operations: `Files.read` and `Files.write` are the model's `read`
      -- and `write` (M5b); a request of `Http` is its `req` (M5c), typed at `httpParams`.
      if std && cap == "Files" then
        match o, ← atoms (← field j "args") with
        | "read", [path] =>
          modify fun s => { s with host := { s.host with rd := true } }
          pure (.read path)
        | "write", [path, content] =>
          modify fun s => { s with host := { s.host with wr := true } }
          pure (.write path content)
        | _, _ => throw s!"the operation `Files.{o}`"
      else if std && cap == "Http" then
        if !["get", "post", "put", "delete"].contains o then throw s!"the operation `Http.{o}`"
        modify fun s => { s with host := { s.host with rq := true } }
        pure (.req o (← atoms (← field j "args")))
      else if std && cap == "Env" && o == "get" then
        -- `Env.get` reads the world's environment, whose variables are the same in both runs (#487).
        match ← atoms (← field j "args") with
        | [name] => pure (.envGet name)
        | _ => throw "the operation `Env.get`, with other than one argument"
      else
      modify fun s => { s with ops := if s.ops.contains (cap, o) then s.ops else s.ops ++ [(cap, o)] }
      match ← atoms (← field j "args") with
      | [a] => pure (.op a)
      | _ => throw s!"the operation `{cap}.{o}`, which does not take one argument"
    | "construct" =>
      let ctor ← field j "ctor"
      let q ← path (← field ctor "kind")
      let args ← atoms (← field j "args")
      match q, ← str (← field ctor "name"), args with
      | "prelude.Option", "Some", [a] => pure (.some a)
      | "prelude.Option", "None", [] => pure .none
      | "prelude.Result", "Ok", [a] => pure (.mkOk a)
      | "prelude.Result", "Err", [a] => pure (.mkErr a)
      | _, _, _ =>
        if c.records.contains q then pure (.construct q args) else throw s!"a value of `{q}`"
    | "field" =>
      match (← field j "index").getNat? with
      | .ok i => pure (.field (← atom (← field j "value")) i)
      | .error _ => throw "a field index"
    | "list" => pure (.list (← atoms (← field j "value")))
    | "tuple" => pure (.tuple (← atoms (← field j "value")))
    | "lambda" =>
      let ps ← (← arr (← field j "params")).mapM fun q => do
        match ← arr q with
        | [x, t] => pure (← str x, ← ty c t)
        | _ => throw "a parameter"
      pure (.lambda ps (← expr c (← field j "body")))
    | "declassify" =>
      let pol ← path (← field j "policy")
      refPolicy pol
      pure (.declassify pol (← atom (← field j "arg")))
    -- `checked(a, φ) else fail C` (#453): the predicate is abstract in F1, named by its
    -- formula's text, so two `checked` of one formula are one predicate; its meaning is the
    -- program's `predSem`, and the theorems hold whatever it is. A failure with a payload is
    -- outside F1, as for `fail`.
    | "checked" =>
      if !(← arr (← field j "args")).isEmpty then throw "a failure with a payload"
      -- The model's predicate reads its operand only (`Program.predSem`, on its text): a formula
      -- that reads any other value, whose label would decide the failure too, is outside F1.
      let value ← field j "value"
      let own : List String := ["it"] ++ (match value.getObjValD "kind", value.getObjValD "value" with
        | .str "var", .str x => [x]
        | _, _ => [])
      if (formulaVars (← field j "formula")).any (fun x => !own.contains x) then
        throw "a `checked` predicate that reads a value other than its operand"
      let code ← match (← field (← field j "ctor") "index").getNat? with
        | .ok i => pure i
        | .error _ => throw "a failure's index"
      pure (.checked (← atom (← field j "value")) (← field j "formula").compress code)
    | k => throw s!"the form `{k}`"

  partial def expr (c : Ctx) (j : J) : M Expr := do
    match ← kind j with
    | "atom" => pure (.atom (← atom (← field j "atom")))
    | "let" =>
      let x ← str (← field j "name")
      let t ← ty c (← field j "ty")
      -- A lambda given to an `of` parameter has a function type without its effects, which are
      -- its caller's (ADR 0062); the export says what they are (#443).
      let t ← match j.getObjVal? "of_lambda" with
        | .ok ol => do
          let effects := ol.getObjValD "effects" == .bool true
          let fails := ol.getObjValD "fails" == .bool true
          pure (t.withFlags effects fails (if effects then c.host else {}))
        | .error _ => pure t
      let v ← field j "value"
      let body ← expr c (← field j "body")
      match ← kind v with
      | "rhs" =>
        let r ← rhs c (← field v "value")
        -- A lambda's body gives the result of its function type (`Expr.attemptsAt`).
        match r, t with
        | .lambda _ lb, .fn _ res _ _ _ _ =>
          if !lb.attemptsAt res then throw "an `attempt` whose error type is not a failure's code"
        | _, _ => pure ()
        pure (.letR x t r body)
      | "expr" =>
        let e ← expr c (← field v "value")
        if !e.attemptsAt t then throw "an `attempt` whose error type is not a failure's code"
        pure (.letE x t e body)
      | k => throw s!"the binding `{k}`"
    | "if" =>
      pure (.ite (← atom (← field j "cond")) (← expr c (← field j "then"))
        (← expr c (← field j "otherwise")))
    | "case" =>
      let sc ← atom (← field j "scrutinee")
      let arms ← arr (← field j "arms")
      let other ← field j "otherwise"
      let armOf (name : String) : M (Option (List J × J)) := do
        for a in arms do
          if (← str (← field (← field a "ctor") "name")) == name then
            return some (← arr (← field a "binds"), ← field a "body")
        return none
      let orElse (o : Option (List J × J)) : M (List J × J) := do
        match o with
        | some a => pure a
        | none => if isNull other then throw "a `case` without an arm" else pure ([], other)
      let first ← match arms with
        | a :: _ => path (← field (← field a "ctor") "kind")
        | [] => throw "a `case` without arms"
      match first with
      | "prelude.Option" =>
        let (_, noneB) ← orElse (← armOf "None")
        let (someBinds, someB) ← orElse (← armOf "Some")
        let x ← match someBinds with
          | [b] => bindName b
          | _ => pure "%unread"
        pure (.caseOption sc (← expr c noneB) x (← expr c someB))
      | "prelude.Result" =>
        let (okBinds, okB) ← orElse (← armOf "Ok")
        let (errBinds, errB) ← orElse (← armOf "Err")
        let x ← match okBinds with
          | [b] => bindName b
          | _ => pure "%unread"
        let y ← match errBinds with
          | [b] => bindName b
          | _ => pure "%unread"
        pure (.caseResult sc x (← expr c okB) y (← expr c errB))
      | q => throw s!"a `case` on `{q}`"
    | "fail" =>
      if !(← arr (← field j "args")).isEmpty then throw "a failure with a payload"
      match (← field (← field j "ctor") "index").getNat? with
      | .ok i => pure (.fail i)
      | .error _ => throw "a failure's index"
    | "attempt" => pure (.attempt (← expr c (← field j "body")))
    | "handle" =>
      let x ← str (← field j "error")
      let h ← field j "handler"
      if reads x h then throw "a handler that reads the failure"
      pure (.handle (← expr c (← field j "body")) "%unread" (← expr c h))
    | "let_tuple" =>
      let bs ← (← arr (← field j "names")).mapM (binder c)
      pure (.letTuple bs (← atom (← field j "value")) (← expr c (← field j "body")))
    | "loop" =>
      let src ← field j "source"
      let source ← match ← kind src with
        | "list" => pure (Source.list (← atom (← field src "value")))
        | "range" =>
          match (← field src "inclusive").getBool? with
          | .ok inc => pure (Source.range (← atom (← field src "low")) (← atom (← field src "high")) inc)
          | .error _ => throw "a range's `inclusive`"
        | k => throw s!"a loop over `{k}`"
      let (x, xt) ← binder c (← field j "var")
      let st ← (← arr (← field j "state")).mapM (binder c)
      pure (.loop source x xt st (← atoms (← field j "init")) (← expr c (← field j "body")))
    | k => throw s!"the form `{k}`"
end

def constraintEq (tp : String) (j : J) : M (Option String) := do
  match ← kind j with
  | "none" => pure none
  | "eq" => pure (some tp)
  | k => throw s!"the constraint `{k}`"

/-- A function's signature, with `body` as its body. -/
def signature (c : Ctx) (module : String) (v : J) (body : Expr) : M Fun := do
  let name := s!"{module}.{← str (← field v "name")}"
  let tps ← (← arr (← field v "type_params")).mapM str
  let cs ← arr (← field v "constraints")
  let eqs ← (tps.zip cs).filterMapM (fun q => constraintEq q.1 q.2)
  let eqs := if c.allEq && comparesValues (v.getObjValD "body") then tps else eqs
  -- The parameters `uses of` and `fails of` name (ADR 0062, exported by #443): their function
  -- types perform the effects, and may fail, as the function itself does.
  -- Each of the four keys #443 added is required (#514, N-99): a function without one is not
  -- re-checked, never read with a permissive default (pure, unable to fail, no raise).
  let usesOf ← (← arr (← field v "uses_of")).mapM str
  let failsOf : Option String ← match ← field v "fails_of" with
    | .str x => pure (some x)
    | .null => pure none
    | _ => throw "a `fails_of` that is neither a name nor null"
  let ps ← (← arr (← field v "params")).mapM fun q => do
    match ← arr q with
    | [x, t] => do
      let x' ← str x
      pure (x', (← ty c t).withFlags (usesOf.contains x') (failsOf == some x'))
    | _ => throw "a parameter"
  -- The result summary (ADR 0067), exported by #443: the symbolic labels a call puts at each
  -- position of the result; in edition 0.1, every position by every type parameter (D-137).
  if !(← arr (← field v "of_raises")).isEmpty then
    throw "`of` raises (ADR 0067 amendment), outside F1"
  let resultLabels ← arr (← field v "result_labels")
  let declared ← ty c (← field v "result")
  let result ← if tps.isEmpty then pure declared
    else if c.allEq then pure (declared.addStarAll tps)
    else resultLabels.foldlM (fun acc rl => do
          let pos ← str (← field rl "position")
          let from_ ← (← arr (← field rl "from")).mapM str
          let steps := if pos == "top" then [] else pos.splitOn "."
          match acc.addStarAt from_ steps with
          | some t => pure t
          | none => throw s!"a result summary at `{pos}`, outside F1") declared
  -- The body gives the result (`Expr.attemptsAt`).
  if !body.attemptsAt result then throw "an `attempt` whose error type is not a failure's code"
  let (op, host) ← usesEffects (← arr (← field v "uses"))
  pure {
    name, tparams := tps, eqParams := eqs, params := ps, result,
    op := op || !usesOf.isEmpty, host,
    fails := !isNull (← field v "fails") || failsOf.isSome, body,
    rels := (List.lookup name c.rels).getD [] }

/-- Every category a label of the document names. -/
partial def categories (j : J) (acc : List String) : List String :=
  match j with
  | .obj kvs =>
    kvs.toArray.foldl (fun acc kv =>
      if kv.1 == "categories" then
        match kv.2 with
        | .arr cs => cs.foldl (fun acc c =>
            match c with
            | .str s => if acc.contains s then acc else acc ++ [s]
            | _ => acc) acc
        | _ => acc
      else categories kv.2 acc) acc
  | .arr items => items.foldl (fun acc i => categories i acc) acc
  | _ => acc

/-- A declaration of the document, with its module. -/
structure Decl where
  module : String
  kind : String
  value : J

def run {α : Type} (m : M α) : Except String (α × Refs) := m.run {}

def plainRun {α : Type} (m : M α) : Except String α := (run m).map Prod.fst

/-- The kinds without payload (#507): the document's kinds without type parameters whose
variants have no fields, and the specification's error kinds, `FileError` and `KeyError`
(`spec/14 §3`) and `HttpError` (`spec/24 §1`), when the document does not declare them, as the
export does not declare the first two. A document that declares one of them otherwise has its own
declaration read. A code is read as its variant's index and renders as it (#508), which tells two
values apart exactly when their variants differ only while no variant has a field: the test
`the_prelude_error_kinds_the_formal_model_reads_as_codes_have_no_payload` (`una-check`) fails if
one of the three gains a payload in either edition's prelude, and then this list must drop it. -/
def codeKinds (decls : List Decl) : List String :=
  let declared := decls.filterMap fun d =>
    if d.kind != "data" then none
    else match plainRun (do str (← field d.value "name")) with
      | .ok n => some s!"{d.module}.{n}"
      | .error _ => none
  let own := decls.filterMap fun d =>
    if d.kind != "data" then none
    else
      match plainRun (do
          -- A kind's type parameters are its `params` in the export (ADR 0083).
          if !(← arr (← field d.value "params")).isEmpty then throw "generic"
          for v in ← arr (← field d.value "variants") do
            if !(← arr (← field v "fields")).isEmpty then throw "a payload"
          pure s!"{d.module}.{← str (← field d.value "name")}") with
      | .ok q => some q
      | .error _ => none
  own ++ (["prelude.FileError", "prelude.KeyError", "prelude.HttpError"].filter
    fun q => !declared.contains q)

/-- The record kinds a type names, at any depth. -/
partial def LTy.recordNames : LTy → List String
  | .record r _ => [r]
  | .list t _ | .option t _ => LTy.recordNames t
  | .result t e _ => LTy.recordNames t ++ LTy.recordNames e
  | .fn ps r _ _ _ _ => ps.flatMap LTy.recordNames ++ LTy.recordNames r
  | .tuple ts _ => ts.flatMap LTy.recordNames
  | _ => []

/-- The record kinds: kinds the export flags as records (`record`, ADR 0083), without type
parameters (`params`, #509), whose fields F1 reads and which do not reach themselves, until no
more are dropped. The flag separates
a record from a sum kind with one variant, such as `kind Bad = | Oops`, a failure's code
(`codeKinds`). -/
def records (decls : List Decl) (funs : List String) : List Record := Id.run do
  let cands := decls.filterMap fun d =>
    if d.kind != "data" then none
    else
      match plainRun (do
          let tps ← arr (← field d.value "params")
          if !tps.isEmpty then throw "generic"
          match (← field d.value "record").getBool? with
          | .ok true => pure ()
          | _ => throw "not a record"
          match ← arr (← field d.value "variants") with
          | [v] => pure (s!"{d.module}.{← str (← field d.value "name")}", ← arr (← field v "fields"))
          | _ => throw "not a record") with
      | .ok r => some r
      | .error _ => none
  let mut names := cands.map Prod.fst
  let mut out : List (String × List LTy) := []
  for _ in [0:cands.length + 1] do
    let c : Ctx := { records := names, funs }
    out := cands.filterMap fun (q, fs) =>
      if !names.contains q then none
      else
        match plainRun (fs.mapM fun f => do
            match ← arr f with
            | [_, t] => ty c t
            | _ => throw "a field") with
        | .ok ts => some (q, ts)
        | .error _ => none
    -- A record that reaches itself through its fields, such as `kind Node` with
    -- `next: Option[Node]`, is not one F1 reads (#509): its values are trees of any depth, and
    -- F1's equality of records unfolds them only so far (`Program.recordEq`), so `==` over one
    -- would be rejected where Una allows it. The records that reach it are dropped in turn.
    -- Each edge once: a record with many fields of one record type would otherwise push that
    -- type many times, past the bound below, and a cycle could go unseen.
    let edges := out.map fun (q, ts) =>
      (q, ((ts.flatMap LTy.recordNames).filter (names.contains ·)).eraseDups)
    let reach := fun (q : String) => Id.run do
      let mut seen : List String := []
      let mut todo := (List.lookup q edges).getD []
      for _ in [0:edges.length * edges.length + 1] do
        match todo with
        | [] => break
        | r :: rest =>
          todo := rest
          if !seen.contains r then
            seen := r :: seen
            todo := todo ++ (List.lookup r edges).getD []
      pure seen
    out := out.filter fun (q, _) => !(reach q).contains q
    names := out.map Prod.fst
  -- The deep labels: the least solution over nested records (`spec/07 §7` rule 2).
  let mut deep : List (String × Label) := out.map fun q => (q.1, Label.bot)
  for _ in [0:out.length + 1] do
    let rd := fun r => (List.lookup r deep).getD Label.bot
    deep := out.map fun (q, ts) => (q, (LTy.deeps rd ts).base)
  pure (out.map fun (q, ts) => ⟨q, ts, (List.lookup q deep).getD Label.bot⟩)

/-- The outcome of a declaration. `acceptedIfA3` is accepted on the condition of part A3 of M4
completion (instance erasure): the declaration is, or reaches, a function checked at its
instances, whose T-Fun the whole-program theorem takes as a named hypothesis. -/
inductive Outcome where
  | accepted (note : String)
  | acceptedIfA3 (note : String)
  | rejected (why : String)
  | notRechecked (why : String)

/-- The host's labels the certificate describes (M5b; the binding rule of #467): `Files`'s and
`Http`'s from the document's host signatures, and the file-state label from its `file_state` key,
which the checker of ADR 0085 computes. The key is untrusted: the checker re-checks every write
against it (`Check.checkRhs`) and its cap at `internal` (`Check.hostWfB`), so a value too low gives
a rejection or a declaration not re-checked, never an accepted one, and a value too high only
labels failures higher. A signature the document lacks leaves the strictest labels
`Program.HostWF` allows. `Header`'s value label is not exported, so a request's failure floor stays
`internal`, as `Header` declares (M5c, #470, reads it). -/
structure HostLabels where
  acceptsRead : Label := Label.bot
  readResult : Label := ⟨.internal, []⟩
  acceptsWrite : Label := Label.bot
  acceptsReq : Label := Label.bot
  reqResult : Label := ⟨.internal, []⟩
  reqFloor : Label := ⟨.internal, []⟩
  /-- `none` when the document has no `file_state` key. -/
  fileState : Option Label := none
  /-- Why the labels could not be read, if they could not. -/
  why : Option String := none

/-- A concrete label: no symbol. -/
def concrete (j : J) : M Label := do
  let l ← label j
  if !l.plain.isEmpty || !l.star.isEmpty then throw "a host label with a symbol"
  pure l.base

/-- The host's labels of a document, from its capabilities `Files` and `Http` and its
`file_state` key. -/
def readHostLabels (capDecls : List (String × J)) (doc : J) : HostLabels :=
  let read : M HostLabels := do
    let mut h : HostLabels := {}
    if let some (_, fj) := capDecls.find? (·.1 == "Files") then
      for o in ← arr (← field fj "ops") do
        let n ← str (← field o "name")
        let acc ← concrete (← field o "accepts")
        if n == "read" then
          let res ← concrete (← field (← field o "result") "label")
          h := { h with acceptsRead := acc, readResult := res }
        else if n == "write" then h := { h with acceptsWrite := acc }
    -- `Http`'s operations share their `accepts` and result labels (`spec/24 §1`).
    if let some (_, hj) := capDecls.find? (·.1 == "Http") then
      let ops ← arr (← field hj "ops")
      if let o :: _ := ops then
        let acc ← concrete (← field o "accepts")
        let res ← concrete (← field (← field o "result") "label")
        h := { h with acceptsReq := acc, reqResult := res }
    if let .ok fs := doc.getObjVal? "file_state" then
      let l ← concrete fs
      h := { h with fileState := some l }
    pure h
  match plainRun read with
  | .ok h => h
  | .error why => { why := some why }

/-- What checking a function declaration needs: the function as F1 reads it, the signatures of
what it calls, its policies, the `accepts` of its operation, and whether it calls a generic
function at its own type parameters. -/
structure Prep where
  fn : Fun
  callees : List Fun
  pols : List Policy
  acceptsOp : Label
  /-- The one operation the function mentions, if any. -/
  op : Option (String × String)
  nested : Bool

/-- A substitution of type arguments: `LTy.lsubst` with the record labels of the document, or a
mutant of it in the tests. -/
abbrev Subst := (String → Label) → List (String × LTy) → LTy → LTy

/-- Reads one function declaration of a document; the error is why it is outside F1. -/
def prepare (c : Ctx) (hl : HostLabels) (caps : List (String × J)) (decls : List Decl) (d : Decl) :
    Except String Prep := do
  let findDecl := fun (q : String) => decls.find? fun e =>
    match plainRun (do pure s!"{e.module}.{← str (← field e.value "name")}") with
    | .ok n => n == q && (e.kind == "fun" || e.kind == "policy")
    | .error _ => false
  let read : M (Fun × List Fun × List Policy) := do
    let (_, fnHost) ← usesEffects (← arr (← field d.value "uses"))
    let fn ← signature c d.module d.value (← expr { c with host := fnHost } (← field d.value "body"))
    -- The policies the function and its callees may release through, whose `to` labels bound
    -- the calls (`Program.RelBound`).
    for pol in fn.rels do refPolicy pol
    for f in (← get).funs do
      for pol in (List.lookup f c.rels).getD [] do refPolicy pol
    -- Every policy of the document, so that B1's restrictions are checked over the whole
    -- program's policies (`Program.B1WellFormed`), not only those the function reaches.
    for (_, pol) in c.vias do refPolicy pol
    let pols ← (← get).policies.mapM fun pol => do
      let some e := findDecl pol | throw s!"the policy `{pol}`"
      let via ← path (← field e.value "via")
      refFun via
      let types : M (LTy × LTy) := do pure (← ty c (← field e.value "from"), ← ty c (← field e.value "to"))
      match plainRun types with
      | .ok (src, dst) => pure ({ name := pol, src, dst, via } : Policy)
      | .error why => throw s!"B1 restriction (a policy outside F1): `{pol}`: {why}"
    let callees ← (← get).funs.mapM fun f => do
      let some e := findDecl f | throw s!"the function `{f}`"
      -- A `via` function's body is read: option B1 asks it to be pure (`Check.checkB1`).
      if (c.vias.find? (·.1 == f)).isSome then
        match plainRun (do expr c (← field e.value "body")) with
        | .ok b => signature c e.module e.value b
        | .error why => throw s!"the `via` function `{f}` is outside F1: {why}"
      else signature c e.module e.value (.atom .unit)
    pure (fn, callees, pols)
  let ((fn, callees, pols), refs) ← run read
  if (List.lookup fn.name c.vias).isSome then throw "a `via` function, which no label rule checks"
  -- A declaration that reaches `Files` is re-checked against the document's host labels.
  if !refs.host.isNone then
    if let some why := hl.why then throw s!"the host's labels: {why}"
    if hl.fileState.isNone then throw "the export lacks the file-state label (`file_state`)"
  let acceptsOp ← match refs.ops with
    | [] => pure Label.bot
    | [(cap, o)] => do
      let some (_, capJ) := caps.find? (·.1 == cap) | throw s!"the capability `{cap}`"
      let ops ← plainRun (arr (← plainRun (field capJ "ops")))
      let some opJ := ops.find? (fun o' => match plainRun (do str (← field o' "name")) with
          | .ok n => n == o
          | .error _ => false)
        | throw s!"the operation `{cap}.{o}`"
      plainRun do
        let ps ← arr (← field opJ "params")
        let c0 : Ctx := { records := [], funs := [] }
        let shape ← match ps with
          | [q] => do
            match ← arr q with
            | [_, t] => pure (← ty c0 t)
            | _ => throw "a parameter"
          | _ => throw s!"the operation `{cap}.{o}`, which does not take one argument"
        let r ← ty c0 (← field opJ "result")
        match shape, r with
        | .int _, .unit _ | .bool _, .unit _ | .text _, .unit _ =>
          if !isNull (← field opJ "fails") then throw s!"the operation `{cap}.{o}`, which may fail"
          let a ← label (← field opJ "accepts")
          pure a.base
        | _, _ => throw s!"the operation `{cap}.{o}`, which does not take a scalar and give `Unit`"
    | _ => throw "several operations"
  pure { fn, callees, pols, acceptsOp, op := refs.ops.head?, nested := refs.nested }

/-- The program the re-checker certifies, with the document's host labels (`HostLabels`), which
`Check.progOk` checks against `Program.HostWF`. -/
def mkProgram (cats : List String) (acceptsOp : Label) (hl : HostLabels) (recs : List Record)
    (pols : List Policy) (funs : List Fun) : Program where
  categories := cats
  acceptsOp := acceptsOp
  acceptsRead := hl.acceptsRead
  acceptsWrite := hl.acceptsWrite
  acceptsReq := hl.acceptsReq
  readResult := hl.readResult
  reqResult := hl.reqResult
  reqFloor := hl.reqFloor
  fileState := hl.fileState.getD Label.bot
  records := recs
  policies := pols
  funs := funs

/-- Checks `fn` (the prepared function, or one of its instances) in the program of `prep`. -/
def checkWith (cats : List String) (hl : HostLabels) (recs : List Record) (prep : Prep) (fn : Fun) :
    Check.Result Unit := do
  let p := mkProgram cats prep.acceptsOp hl recs prep.pols (fn :: prep.callees)
  Check.checkDecls p
  Check.checkB1 p
  for f in p.funs do Check.checkSignature f
  Check.checkFun p fn

/-- What a call whose `function` F1 cannot read reaches (#514, N-99): no function of any document,
so a declaration that holds one, and every declaration that reaches it, is not re-checked
("reaches `%unread-call`, outside the document"), and its releases are never computed without the
call's. A lambda without `function` is an ordinary lambda, not this. -/
def unreadCall : String := "%unread-call"

/-- A `call` node of the refinement language (`una_core::Term::Call`, `p(a, ...)` of a predicate),
not of Core (`Rhs::Call`, which names a `function`). Refinements are pure and their labels erased
(`spec/07 §7` rule 6): such a node is neither an instantiation nor a reach edge. A `call` with
neither key is unknown, and the scans below fail closed on it. -/
def predicateCall (j : J) : Bool :=
  (j.getObjVal? "predicate").toBool && !(j.getObjVal? "function").toBool

/-- The function a Core call names. A node that names both a function and a predicate is not a
well-formed term of either kind: it reads as a call whose function F1 cannot read, so the scans
fail closed on it. -/
def callFunction (j : J) : Except String String :=
  if (j.getObjVal? "predicate").toBool then .error "a call that names a predicate too"
  else plainRun (do path (← field j "function"))

/-- The calls of `j` with type arguments, as F1 reads them: the generic function, its type
arguments (`none` when F1 does not read them), and whether they are ground. Every node is
searched, inside every kind of declaration, so that a function value, a handler or `main` is an
instantiation source too. -/
partial def rawGenericCalls (c : Ctx) (j : J) : List (Option String × Option (List LTy)) :=
  match j with
  | .obj kvs =>
    -- Every call is reported, fail-closed (#514, N-99): one whose `function` F1 cannot read
    -- names no function (`none`), and one without readable `type_args` has unknown type
    -- arguments (`none`); the search then makes the instances unknown, never skips the call.
    let here : List (Option String × Option (List LTy)) :=
      if j.getObjValD "kind" != .str "call" || predicateCall j then []
      else
        match callFunction j with
        | .error _ => [(none, none)]
        | .ok f =>
          [(some f, match plainRun (do (← arr (← field j "type_args")).mapM (ty c)) with
            | .ok ts => some ts
            | .error _ => none)]
    here ++ kvs.toArray.toList.flatMap (fun kv => rawGenericCalls c kv.2)
  | .arr items => items.toList.flatMap (rawGenericCalls c)
  | _ => []

/-- The nodes of `j` other than calls that carry type arguments, by kind, with the function they
name if any. Today's `una.core/1` writes type arguments only on calls; a later form that
instantiates a function another way is a source the search does not read, so it makes the
instances of what it names, or of every function checked at its instances, unknown. -/
partial def otherInstantiations (j : J) : List (String × Option String) :=
  match j with
  | .obj kvs =>
    let here : List (String × Option String) :=
      match j.getObjValD "kind", j.getObjVal? "type_args" with
      | .str k, .ok (.arr targs) =>
        if k == "call" || targs.isEmpty then []
        else [(k, match plainRun (do path (← field j "function")) with
          | .ok f => some f
          | .error _ => none)]
      | _, _ => []
    here ++ kvs.toArray.toList.flatMap (fun kv => otherInstantiations kv.2)
  | .arr items => items.toList.flatMap otherInstantiations
  | _ => []

/-- The policies a body declassifies through and the functions it calls, lambdas included: what
its releases may come from (option B1). A release inside a lambda is outside B1, which the
checker reports; counting it here keeps a caller from being checked against a release set that
misses it. -/
partial def rawRels (j : J) : List String × List String :=
  match j with
  | .obj kvs =>
    match j.getObjValD "kind" with
    | .str "declassify" =>
      match plainRun (do path (← field j "policy")) with
      | .ok pol => ([pol], [])
      | .error _ => ([], [])
    | k =>
      let here : List String := if k == .str "call" && !predicateCall j then
          match callFunction j with
          | .ok f => if f.startsWith "prelude." then [] else [f]
          | .error _ => [unreadCall]
        else []
      kvs.toArray.toList.foldl (fun acc kv =>
        let r := rawRels kv.2
        (acc.1 ++ r.1, acc.2 ++ r.2)) ([], here)
  | .arr items => items.toList.foldl (fun acc i =>
      let r := rawRels i
      (acc.1 ++ r.1, acc.2 ++ r.2)) ([], [])
  | _ => ([], [])

/-- The functions a declaration reaches directly: the calls in its body, lambdas included, and the
functions it uses as values (a lambda around a call, `function` in Core). Prelude and library
functions are kept: the fixpoint exempts only the prelude functions F1 models. A capability
operation (`op`) is not a call: what it reaches is its handler's body, which is not in the reach
set (M2's assumption that an operation is an event, `docs/plan/formal-track-m2.md`). -/
partial def rawCallees (j : J) : List String :=
  match j with
  | .obj kvs =>
    let here : List String :=
      match j.getObjValD "kind" with
      | .str "call" =>
        if predicateCall j then []
        else match callFunction j with
          | .ok f => [f]
          | .error _ => [unreadCall]
      | .str "lambda" =>
        match plainRun (do path (← field j "function")) with
        | .ok f => [f]
        | .error _ => []
      | _ => []
    -- `checked`'s predicate (#453) is a term of the refinement language, pure, which the model
    -- leaves abstract (`Program.predSem`): what it calls is not run by the model, so it is not
    -- reached. `una check` keeps a predicate's body pure and non-failing (U2015).
    let skip := j.getObjValD "kind" == .str "checked"
    here ++ (kvs.toArray.toList.filter (fun kv => !(skip && (kv.1 == "predicate" || kv.1 == "formula")))).flatMap
      (fun kv => rawCallees kv.2)
  | .arr items => items.toList.flatMap rawCallees
  | _ => []

/-- The prelude functions F1 gives a semantics to: the only functions outside the document a
declaration may reach and still be accepted. -/
def preludeF1 : List String :=
  ["prelude.map", "prelude.filter", "prelude.first_where", "prelude.fold", "prelude.parse_int",
    "prelude.sum", "prelude.append", "prelude.set"]

/-- A rejection by a restriction of option B1 is not a flow the model forbids: the program is
outside B1, so it is not re-checked (ADR 0008). -/
def classify (why : String) : Outcome :=
  if (why.splitOn "B1 restriction").length > 1 then .notRechecked why else .rejected why

/-- The key of an instance: its type arguments, written. -/
def instKey (ts : List LTy) : String := ", ".intercalate (ts.map Check.fmtTy)

mutual
  /-- A call of a function checked at its instances, at ground type arguments, made a call of
  the instance, `f[key]`, as the instance search names it. -/
  def Rhs.toInst (cond : List String) : Rhs → Rhs
    | .call g targs args =>
      if cond.contains g && Check.groundAll targs then .call s!"{g}[{instKey targs}]" [] args
      else .call g targs args
    | .funRef g targs =>
      if cond.contains g && Check.groundAll targs then .funRef s!"{g}[{instKey targs}]" []
      else .funRef g targs
    | .lambda ps b => .lambda ps (Expr.toInst cond b)
    | r => r

  def Expr.toInst (cond : List String) : Expr → Expr
    | .atom a => .atom a
    | .letR x t v b => .letR x t (Rhs.toInst cond v) (Expr.toInst cond b)
    | .letE x t v b => .letE x t (Expr.toInst cond v) (Expr.toInst cond b)
    | .ite c t e => .ite c (Expr.toInst cond t) (Expr.toInst cond e)
    | .caseOption sc n x sm => .caseOption sc (Expr.toInst cond n) x (Expr.toInst cond sm)
    | .caseResult sc x o y er => .caseResult sc x (Expr.toInst cond o) y (Expr.toInst cond er)
    | .fail c => .fail c
    | .attempt b => .attempt (Expr.toInst cond b)
    | .handle b x h => .handle (Expr.toInst cond b) x (Expr.toInst cond h)
    | .letTuple bs v b => .letTuple bs v (Expr.toInst cond b)
    | .loop src x xt st init b => .loop src x xt st init (Expr.toInst cond b)
end

/-- The parts of the program checked at instances, from the final members: the functions checked
at their instances among them (`cond`), recomputed so that one demoted earlier leaves with its
instances; `P₁`'s functions, with the calls renamed and the instances of `cond` in place of it;
and the renaming, an instance to its generic function and a function of `cond` to a name no
function has. -/
def monoParts (members : List String) (isNested : String → Bool) (readFn : String → Option Fun)
    (instFuns : List (String × Fun)) (viaFuns : List Fun) :
    List String × List Fun × List (String × String) :=
  let cond := members.filter isNested
  let insts := instFuns.filter fun (g, _) => cond.contains g
  let toInstFn := fun (fn : Fun) => { fn with body := Expr.toInst cond fn.body }
  let mono := (members.filter (!cond.contains ·)).filterMap (fun f => (readFn f).map toInstFn) ++
    insts.map (toInstFn ·.2) ++ viaFuns
  let ren := insts.map (fun (g, inst) => (inst.name, g)) ++ cond.map (fun g => (g, s!"{g}[]"))
  (cond, mono, ren)

/-- What an accepted declaration's note says when its program uses `Files`: its theorem is
`t6_release_erasure_obs`, on a lawful host (H1 to H3), not the any-host
`t6_release_erasure_obs_hostFree`. -/
def hostNote : String := "rests on a lawful host (H1 to H3): the program uses `Files`"

/-- `Http`'s kinds as `spec/24 §1` fixes them (M5c): `Header(name: Text, value: internal Text)`
and `Response(status: Int, headers: List[Header], body: Text)`, as the export names them. -/
def headerRec : Record :=
  ⟨"prelude.Header", [.text .bot, .text (.ofLabel ⟨.internal, []⟩)], ⟨.internal, []⟩⟩

def responseRec : Record :=
  ⟨"prelude.Response", [.int .bot, .list (.record "prelude.Header" .bot) .bot, .text .bot],
    ⟨.internal, []⟩⟩

/-- The document's records, with `Http`'s kinds the specification's: `none` when the document
declares one of them otherwise (default-deny: the export does not define them). -/
def withHttpRecords (recs : List Record) : Option (List Record) :=
  let same := fun (a b : Record) =>
    a.fields.map Check.fmtTy == b.fields.map Check.fmtTy &&
      Check.labelLe a.deep b.deep && Check.labelLe b.deep a.deep
  let ok := fun (fixed : Record) => match recs.find? (·.name == fixed.name) with
    | some r => same r fixed
    | none => true
  if ok headerRec && ok responseRec then
    some (recs.filter (fun r => r.name != headerRec.name && r.name != responseRec.name) ++
      [headerRec, responseRec])
  else none

/-- The document's declarations of `Http`'s kinds agree with `spec/24 §1` (`headerRec`,
`responseRec`): each `Header` or `Response` data declaration has one variant whose fields have the
specification's types and labels. Default-deny: one that cannot be read, or differs, is `false`. -/
def httpKindsOk (decls : List Decl) : Bool :=
  let c : Ctx := { records := [headerRec.name, responseRec.name], funs := [] }
  decls.all fun d =>
    if d.kind != "data" then true
    else match plainRun (do str (← field d.value "name")) with
      | .ok "Header" | .ok "Response" =>
        let fixed := if d.value.getObjValD "name" == Lean.Json.str "Header" then headerRec else responseRec
        match plainRun (do
            match ← arr (← field d.value "variants") with
            | [v] => (← arr (← field v "fields")).mapM fun f => do
                match ← arr f with
                | [_, t] => ty c t
                | _ => throw "a field"
            | _ => throw "not one variant") with
        | .ok ts => ts.map Check.fmtTy == fixed.fields.map Check.fmtTy
        | .error _ => false
      | _ => true

/-- The functions `f` reaches in `P`, transitively, with every `via` function of `P`'s policies:
what a run of `f` can call or reference (#472). -/
def reachIn (P : Program) (f : String) : List String := Id.run do
  let mut s := f :: P.policies.map (·.via)
  for _ in [0:P.funs.length + 1] do
    s := (s ++ s.flatMap fun g => match P.find g with
      | some fn => fn.body.callees
      | none => []).eraseDups
  pure s

/-- **`f` reaches no host operation in `P`** (#472), as `t6_release_erasure_obs_reach` asks:
its reach `S` is closed under the calls and references of the bodies in it, none of them performs
a host operation, `S` holds every `via` function, every record of `P` is closure-free, and `f`'s
own parameters have closure-free types (`LTy.cf`), so a caller cannot hand it a closure that
writes a file. -/
def hostFreeAt (P : Program) (f : String) : Bool :=
  let S := reachIn P f
  P.recordsCF && P.policies.all (fun po => S.contains po.via) &&
    S.all fun g => match P.find g with
      | some fn => fn.body.hostFree && fn.body.callees.all S.contains &&
          (g != f || fn.params.all (·.2.cf))
      | none => true

/-- The note of a declaration whose certified program performs no host operation, but which
takes a closure a caller may make one that does (#475). -/
def closureNote : String :=
  "rests on a lawful host (H1 to H3): a caller may hand it a closure that uses the host"

/-- The note of a declaration certified in a program that uses the host's stateful capabilities:
its theorem is `t6_release_erasure_obs` on a lawful host, H1 to H3 for `Files`, and H4 too when
`Http` is used (M5c). -/
def hostNoteFor (files http : Bool) (env : Bool := false) : String :=
  -- An `Env`-only program keeps the closure clause, which the file-state guard reads
  -- (`usesFilesNote`): a declaration that takes a closure may be handed one that writes (#495).
  let base := match files, http with
      | true, true => "rests on a lawful host (H1 to H4): the program uses `Files` and `Http`"
      | false, true => "rests on a lawful host (H1 to H4): the program uses `Http`"
      | true, false => hostNote
      -- The certified program performs no host operation, but `f` takes a closure, which a
      -- caller may make one that does (#475).
      | false, false => closureNote
  if env then base ++
    ", and `Env`, whose variables are the same in both runs: whether a variable exists is public configuration, so an operator must not encode a secret in which variables are set" else base

mutual
  /-- Whether a right-hand side reads the environment (`Env.get`, M6b), in its lambdas included. -/
  def Rhs.usesEnv : Rhs → Bool
    | .envGet _ => true
    | .lambda _ b => Expr.usesEnv b
    | _ => false

  def Expr.usesEnv : Expr → Bool
    | .atom _ | .fail _ => false
    | .letR _ _ v b => Rhs.usesEnv v || Expr.usesEnv b
    | .letE _ _ v b => Expr.usesEnv v || Expr.usesEnv b
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e =>
      Expr.usesEnv t || Expr.usesEnv e
    | .attempt b | .letTuple _ _ b | .loop _ _ _ _ _ b => Expr.usesEnv b
end

/-- Whether a note's verdict may rest on the file-state label: the program uses `Files`, or a
caller may hand the declaration a closure that does (#475), which the guard keeps covering. -/
def usesFilesNote (note : String) : Bool :=
  (note.splitOn "uses `Files`").length > 1 || (note.splitOn "a closure that uses the host").length > 1

/-- A verdict's note, with the host's note unless `f` reaches no host operation and cannot be
handed one (`hostFreeAt`). A host-free certified program is not enough on its own: it may be a part of the
document, and a caller outside it may hand `f` a closure that writes a file (#472). -/
def withHost (docFree : Bool) (P : Program) (f : String) (note : String) : String :=
  let fx : HostFx := P.funs.foldl (fun (acc : HostFx) fn => acc.or fn.host) {}
  let hn := hostNoteFor (fx.rd || fx.wr) fx.rq (P.funs.any (fun fn => Expr.usesEnv fn.body))
  if docFree || hostFreeAt P f then note
  else if note == "" then hn else note ++ "; " ++ hn

theorem withHost_ex (d : Bool) (P : Program) (f n : String) :
    (∃ n₀, withHost d P f n = withHost d P f n₀) ↔ True :=
  ⟨fun _ => trivial, fun _ => ⟨n, rfl⟩⟩

/-- **The verdicts, from the certificates alone** (M4 completion, A2 and A3). Every declaration
the steps before report accepted gets its verdict here, and only from a certificate:

- accepted, when the program checked at instances, `P₁` (`mono`, with `pAll`'s other fields),
  passes `progOk []` and agrees with the document's program `pAll` up to annotations (`progSim`),
  and the declaration is a function of `pAll`; or when the program of the declarations that reach
  no function checked at its instances, `pPlain`, passes `progOk []` and has it;
- accepted, conditional on A3, when only `pAll` passes `progOk cond`;
- not re-checked otherwise.

A certificate whose program performs no host operation (`Program.hostFreeB`) has a theorem on any
host (`t6_release_hostFree`, `t6_release_erasure_obs_hostFree`), the real one included. One whose
program uses `Files` (M5b) has `t6_release_erasure_obs`, on a lawful host (H1 to H3), and the
verdict's note says so (`withHost`).

`finalize_sound` states it. -/
def finalize (docFree : Bool) (pAll pPlain : Program) (cond : List String) (mono : List Fun)
    (ren : List (String × String)) (outs : List (String × Outcome)) : List (String × Outcome) :=
  let p₁ : Program := { pAll with funs := mono }
  let monoOk := Check.progOk [] p₁ && progSim ren p₁ pAll
  let plainOk := Check.progOk [] pPlain
  let condOk := Check.progOk cond pAll
  outs.map fun (f, o) =>
    let verdict := fun (note : String) =>
      if monoOk && pAll.funs.any (·.name == f) then Outcome.accepted (withHost docFree pAll f note)
      else if plainOk && pPlain.funs.any (·.name == f) then .accepted (withHost docFree pPlain f note)
      else if condOk && pAll.funs.any (·.name == f) then .acceptedIfA3 (withHost docFree pAll f note)
      else .notRechecked "whole program: no certificate"
    match o with
    | .accepted note => (f, verdict note)
    | .acceptedIfA3 note => (f, verdict note)
    | o => (f, o)

/-- **A declaration reported accepted has a certificate**: it is a function of a program that
passes `progOk []`, the program of the declarations that reach no function checked at its
instances, or the document's program, which the program checked at instances agrees with up to
annotations. One reported accepted on the condition of A3 is a function of a program that passes
`progOk cond`. Its note says whether the program performs no host operation, whose theorem holds
on any host, or uses `Files` and rests on a lawful host (`withHost`). -/
theorem finalize_sound {docFree : Bool} {pAll pPlain : Program} {cond : List String} {mono : List Fun}
    {ren : List (String × String)} {outs : List (String × Outcome)} {f note : String} :
    ((f, Outcome.accepted note) ∈ finalize docFree pAll pPlain cond mono ren outs →
      (Check.progOk [] pPlain = true ∧ pPlain.funs.any (·.name == f) = true ∧
        ∃ n₀, note = withHost docFree pPlain f n₀) ∨
      (Check.progOk [] { pAll with funs := mono } = true ∧
        progSim ren { pAll with funs := mono } pAll = true ∧
        pAll.funs.any (·.name == f) = true ∧ ∃ n₀, note = withHost docFree pAll f n₀)) ∧
    ((f, Outcome.acceptedIfA3 note) ∈ finalize docFree pAll pPlain cond mono ren outs →
      Check.progOk cond pAll = true ∧ pAll.funs.any (·.name == f) = true ∧
        ∃ n₀, note = withHost docFree pAll f n₀) := by
  constructor
  · intro h
    simp only [finalize, List.mem_map] at h
    obtain ⟨⟨g, o⟩, _, ho⟩ := h
    cases o <;> simp only [Prod.mk.injEq] at ho <;> obtain ⟨rfl, hv⟩ := ho <;>
      (try split at hv) <;> (try split at hv) <;> (try split at hv) <;>
      (try (simp only [Outcome.accepted.injEq] at hv; subst hv)) <;>
      simp_all [Bool.and_eq_true, withHost_ex]
  · intro h
    simp only [finalize, List.mem_map] at h
    obtain ⟨⟨g, o⟩, _, ho⟩ := h
    cases o <;> simp only [Prod.mk.injEq] at ho <;> obtain ⟨rfl, hv⟩ := ho <;>
      (try split at hv) <;> (try split at hv) <;> (try split at hv) <;>
      (try (simp only [Outcome.acceptedIfA3.injEq] at hv; subst hv)) <;>
      simp_all [Bool.and_eq_true, withHost_ex]

/-- The capabilities a document declares, its own and the host's: module, name and operations. -/
abbrev Caps' := List (String × String × List String)

/-- **`std.host`, as the specification fixes it** (`spec/14 §3`; `spec/24` for edition 0.2's
`Http`): its capabilities and their operations. The re-check does not take the export's word for
what `std.host` is (Una-1's finding on #471): a document's `host` entry that differs from these is a
write site (`hostSites`). Of these operations, only `Files.write` writes a file. -/
def stdHost : Caps' :=
  [("std.host", "Console", ["print"]), ("std.host", "Clock", ["now"]),
   ("std.host", "Env", ["get"]), ("std.host", "Files", ["read", "write"]),
   ("std.host", "Keys", ["hmac"]), ("std.host", "Http", ["get", "post", "put", "delete"])]

/-- A `uses` entry that may grant a file write: one that names no capability in `known`, grants an
operation that capability does not declare, or grants `std.host`'s `Files.write`. -/
def usesSite (known : Caps') (u : J) (anyHost : Bool := false) : Option String :=
  match u.getObjValD "capability", u.getObjValD "ops" with
  | .str c, .arr grants =>
    match known.find? (fun (_, n', _) => n' == c) with
    | none => some "a `uses` that names a capability the document does not declare"
    | some (m, _, ops) =>
      let gs := grants.toList.filterMap fun g => match g with
        | .str o => some o
        | _ => none
      if gs.length != grants.size || !gs.all ops.contains then
        some "a `uses` that grants an operation its capability does not declare"
      else if m == "std.host" && c == "Files" && gs.contains "write" then
        some "a `uses` that grants `Files.write`"
      else if anyHost && m == "std.host" && (c == "Files" || c == "Http" || c == "Env") then
        some s!"a `uses` that grants `{c}`"
      else none
  | _, _ => some "a `uses` entry that cannot be read"

/-- **A node that may write a file**, found anywhere in a declaration (M5b; Una-1's findings on
#471 and the coordinator's rule). Default-deny: a node is safe only when it is positively
identified, and every other node counts as a site.
- An operation is safe when its capability path resolves to a capability the document declares
  (`known`: its own, and `std.host` as `stdHost` fixes it), its `capability` field is that path's
  name, its operation is one the capability declares, and it is not `std.host`'s `Files.write`.
- An element of a `uses` array is safe when `usesSite` finds nothing in it. Only `uses` arrays hold
  grants: a handler declaration's capability and operations are not read as one.
`none` when the node and its children hold no site; otherwise what was found. -/
partial def writeSite (known : Caps') (j : J) (anyHost : Bool := false) : Option String :=
  match j with
  | .obj kvs =>
    let here : Option String :=
      if j.getObjValD "kind" == Lean.Json.str "op" then
        let path := j.getObjValD "capability_path"
        match j.getObjValD "capability", path.getObjValD "module", path.getObjValD "name",
            j.getObjValD "op" with
        | .str c, .str m, .str n, .str o =>
          match known.find? (fun (m', n', _) => m' == m && n' == n) with
          | none => some "an operation whose capability cannot be resolved"
          | some (_, _, ops) =>
            if c != n then some "an operation whose capability is not its path's"
            else if !ops.contains o then some "an operation its capability does not declare"
            else if m == "std.host" && n == "Files" && o == "write" then some "a `Files.write`"
            else if anyHost && m == "std.host" && (n == "Files" || n == "Http" || n == "Env") then
              some s!"an operation of `{n}`"
            else none
        | _, _, _, _ => some "an operation whose capability cannot be resolved"
      else match j.getObjValD "uses" with
        | .arr us => us.foldl (fun acc u => acc <|> usesSite known u anyHost) none
        | .null => none
        | _ => some "a `uses` that cannot be read"
    match here with
    | some w => some w
    | none => kvs.toArray.foldl (fun acc kv => acc <|> writeSite known kv.2 anyHost) none
  | .arr xs => xs.foldl (fun acc x => acc <|> writeSite known x anyHost) none
  | _ => none

/-- The document's `host` entries that differ from `std.host` as `stdHost` fixes it: a capability it
does not have, or other operations. -/
def hostSites (host : List J) : List String :=
  host.filterMap fun cj =>
    match cj.getObjValD "name", cj.getObjValD "ops" with
    | .str n, .arr ops =>
      let names := ops.toList.filterMap fun o => match o.getObjValD "name" with
        | .str x => some x
        | _ => none
      match stdHost.find? (fun (_, n', _) => n' == n) with
      | some (_, _, spec) =>
        if names.length == ops.size && names.all spec.contains && spec.all names.contains then none
        else some s!"a host capability `{n}` whose operations are not `std.host`'s"
      | none => some s!"a host capability `{n}` that `std.host` does not declare"
    | _, _ => some "a host capability that cannot be read"

/-- **The file-state label is verified for the whole package**, or nothing that uses `Files` is
accepted (M5b, Una-1's finding on #471). The exported `file_state` is untrusted, and the re-check
checks it only against the writes of the programs it certifies. A write anywhere else, in a
declaration rejected or not re-checked, in a handler, or in a node the reading cannot tell from
one, never meets it, and a lowered label would let a read's failure reveal it (S-14). So unless
every declaration that holds a write site (`writeSite`) is a function accepted in `out`, every
declaration accepted on a lawful host whose note says its program uses `Files` (`usesFilesNote`) is
not re-checked. A program that uses only `Http` does not read the file-state label. -/
def guardFiles (sites : List (String × String × String)) (out : List (String × Outcome)) :
    List (String × Outcome) :=
  let unchecked := sites.find? fun (name, kind, _) =>
    !(kind == "fun" && match List.lookup name out with
      | some (.accepted _) => true
      | _ => false)
  match unchecked with
  | none => out
  | some (name, _, what) =>
    let why := s!"the file-state label is not verified for the whole package ({what} in `{name}`)"
    out.map fun (f, o) => match o with
      | .accepted note => if usesFilesNote note then (f, .notRechecked why) else (f, o)
      | .acceptedIfA3 note => if usesFilesNote note then (f, .notRechecked why) else (f, o)
      | o => (f, o)

/-- **The guard only demotes**: what it reports accepted, `finalize` reported accepted, with the
same note, so `finalize_sound` covers it. -/
theorem guardFiles_sub {sites : List (String × String × String)} {out : List (String × Outcome)}
    {f note : String} (h : (f, Outcome.accepted note) ∈ guardFiles sites out) :
    (f, Outcome.accepted note) ∈ out := by
  unfold guardFiles at h
  dsimp only at h
  split at h
  · exact h
  · simp only [List.mem_map] at h
    obtain ⟨⟨g, o⟩, hm, ho⟩ := h
    cases o <;> simp only at ho <;> (try split at ho) <;> simp_all

/-- How far instances are followed: the depth of an instance chain, past which the function is
in polymorphic recursion, the number of instances of a document, and the steps of the search,
past which what is left in its queue is unknown. -/
structure Caps where
  depth : Nat := 16
  count : Nat := 256
  fuel : Nat := 256 * 18

/-- The outcome of every declaration with code, by qualified name; an error when the document is
not one the checker reads. -/
def checkDocumentWith (subst : Subst) (doc : J) (caps : Caps := {}) :
    Except String (List (String × Outcome)) := do
  let edition := match doc.getObjValD "edition" with
    | .str e => e
    | _ => "?"
  let get := fun {α : Type} (m : M α) => plainRun m
  if (← get (do str (← field doc "schema"))) != "una.core/1" then throw "not una.core/1"
  let complete := (doc.getObjValD "complete") == .bool true
  let modules ← get (do arr (← field doc "modules"))
  let decls ← modules.flatMapM fun m => get do
    let mp ← str (← field m "path")
    (← arr (← field m "decls")).mapM fun dj => do
      -- A tuple variant holds its one field under `value`; a struct variant holds its fields
      -- inline (ADR 0083 and its amendment).
      let value := match dj.getObjVal? "value" with
        | .ok v => v
        | .error _ => dj
      pure ({ module := mp, kind := ← kind dj, value } : Decl)
  let named := fun (d : Decl) => match plainRun (do str (← field d.value "name")) with
    | .ok n => s!"{d.module}.{n}"
    | .error _ => s!"{d.module}.?"
  let code := decls.filter fun d => d.kind != "data" && d.kind != "cap" && d.kind != "policy" &&
    d.kind != "alias"
  if !complete then
    return code.map fun d => (named d, .notRechecked "the document is not complete")
  let host ← get (do arr (← field doc "host"))
  let capDecls := (decls.filter (·.kind == "cap")).map (fun d => d.value) ++ host
  let capDecls := capDecls.filterMap fun cj =>
    match plainRun (do str (← field cj "name")) with
    | .ok n => some (n, cj)
    | .error _ => none
  -- `Files` and `Http` name the host's capabilities in `uses`, which carries no module: a
  -- document that declares its own capability by either name is not re-checked.
  if (decls.filter (·.kind == "cap")).any (fun d => match plainRun (do str (← field d.value "name")) with
      | .ok n => n == "Files" || n == "Http"
      | .error _ => true) then
    return code.map fun d => (named d, .notRechecked "a capability named as one of the host's")
  let hl := readHostLabels capDecls doc
  let cats := categories doc []
  let funNames := (decls.filter (·.kind == "fun")).map named
  -- `Http`'s kinds are the specification's (M5c): a document that declares them otherwise is not
  -- re-checked.
  let some recs := (if httpKindsOk decls then withHttpRecords (records decls funNames) else none)
    | return code.map fun d =>
        (named d, .notRechecked "`Header` or `Response`, declared otherwise than `spec/24 §1`")
  let vias := decls.filterMap fun e =>
    if e.kind == "policy" then
      match plainRun (do pure (← path (← field e.value "via"), s!"{e.module}.{← str (← field e.value "name")}")) with
      | .ok q => some q
      | .error _ => none
    else none
  -- A policy whose name or `via` F1 cannot read would be missing from B1's check: the whole
  -- document is then not re-checked, rather than a policy silently dropped.
  if (decls.filter (·.kind == "policy")).length != vias.length then
    return code.map fun d => (named d, .notRechecked "a policy F1 cannot read")
  let c : Ctx := { records := recs.map (·.name), funs := funNames, vias, allEq := edition == "0.1",
                   codes := codeKinds decls }
  let rd : String → Label := fun r => match recs.find? (·.name == r) with
    | some rec => rec.deep
    | none => Label.bot
  -- The policies each function may release through: its own declassifications and those of the
  -- functions it calls, outside lambdas, to a fixpoint.
  let direct := (decls.filter (·.kind == "fun")).map fun d =>
    (named d, rawRels (d.value.getObjValD "body"))
  let mut relMap : List (String × List String) := direct.map fun q => (q.1, q.2.1)
  for _ in [0:direct.length + 1] do
    relMap := direct.map fun (f, ps, cs) =>
      (f, (ps ++ cs.flatMap fun g => (List.lookup g relMap).getD []).eraseDups)
  let c := { c with rels := relMap }
  let preps : List (String × Except String Prep) :=
    (decls.filter (·.kind == "fun")).map fun d => (named d, prepare c hl capDecls decls d)
  let prepOf := fun (f : String) => List.lookup f preps
  let isNested := fun (f : String) => match prepOf f with
    | some (.ok pr) => pr.nested && !pr.fn.tparams.isEmpty
    | _ => false
  -- The instances of the functions checked at their instances: every source of a call with
  -- type arguments, in every declaration. A source F1 cannot read makes the function's set of
  -- instances unknown.
  let mut incomplete : List (String × String) := []
  let mut queue : List (String × List LTy × String × Nat) := []
  -- Every instance enqueued once, so the search's steps count distinct instances.
  let mut enqueued : List (String × String) := []
  let keyOf := fun (ts : List LTy) => ", ".intercalate (ts.map Check.fmtTy)
  let nestedNames := preps.filterMap fun (f, _) => if isNested f then some f else none
  for d in decls do
    for (k, named?) in otherInstantiations d.value do
      match named? with
      | some f => incomplete := incomplete ++ [(f, s!"instantiated by a `{k}` node, which F1 does not read")]
      | none => incomplete := incomplete ++ nestedNames.map fun f =>
          (f, s!"a `{k}` node instantiates a function F1 cannot name")
    for (f?, targs) in rawGenericCalls c d.value do
      let some f := f?
        | do
            incomplete := incomplete ++ nestedNames.map fun g =>
              (g, s!"a call in `{named d}` whose function F1 cannot read")
            continue
      if isNested f then
        let arity := match prepOf f with
          | some (.ok pr) => pr.fn.tparams.length
          | _ => 0
        match targs with
        | some ts =>
          if ts.length != arity then
            incomplete := incomplete ++
              [(f, s!"called in `{named d}` with {ts.length} type arguments, of its {arity}")]
          else if Check.groundAll ts then
            if !enqueued.contains (f, keyOf ts) then
              enqueued := enqueued ++ [(f, keyOf ts)]
              queue := queue ++ [(f, ts, named d, 0)]
          else
            match prepOf (named d) with
            | some (.ok pr) =>
              if !(pr.nested && !pr.fn.tparams.isEmpty) then
                incomplete := incomplete ++ [(f, s!"a call in `{named d}` at type parameters it does not instantiate")]
            | _ => incomplete := incomplete ++ [(f, s!"instantiated from `{named d}`, which is outside F1")]
        | none => incomplete := incomplete ++
            [(f, s!"instantiated in `{named d}` at type arguments F1 cannot read")]
  let mut done : List (String × List String) := []
  let mut results : List (String × Outcome) := []
  -- The instances checked, with their generic function, for the program of part A3.
  let mut instFuns : List (String × Fun) := []
  let mut depthHit : List String := []
  let mut countHit : Bool := false
  let mut seen : Nat := 0
  let mut fuel := caps.fuel
  while !queue.isEmpty && fuel > 0 do
    fuel := fuel - 1
    match queue with
    | [] => pure ()
    | (f, ts, origin, depth) :: rest =>
      queue := rest
      let key := ts.map Check.fmtTy
      let already := match List.lookup f done with
        | some ks => ks.contains (", ".intercalate key)
        | none => false
      if already then continue
      if depth > caps.depth then
        depthHit := depthHit ++ [f]
        continue
      if seen ≥ caps.count then
        countHit := true
        continue
      seen := seen + 1
      done := match List.lookup f done with
        | some ks => (f, ks ++ [", ".intercalate key]) :: done.filter (·.1 != f)
        | none => (f, [", ".intercalate key]) :: done
      match prepOf f with
      | some (.ok pr) =>
        let inst := Check.instantiate (subst rd) pr.fn ts s!"{f}[{", ".intercalate key}]"
        let outcome := match checkWith cats hl recs pr inst with
          | .ok () => Outcome.accepted ""
          | .error why => classify s!"at its instance `{inst.name}`, instantiated in `{origin}`: {why}"
        results := results ++ [(f, outcome)]
        instFuns := instFuns ++ [(f, inst)]
        for (g, us) in Check.Expr.genericCalls inst.body do
          if isNested g then
            if Check.groundAll us then
              if !enqueued.contains (g, keyOf us) then
                enqueued := enqueued ++ [(g, keyOf us)]
                queue := queue ++ [(g, us, inst.name, depth + 1)]
            else incomplete := incomplete ++ [(g, s!"a call in `{inst.name}` that is not ground")]
      | _ => pure ()
  -- What the search did not reach is unknown: never accepted.
  for (f, _, _, _) in queue do
    incomplete := incomplete ++ [(f, "the instance search stopped at its fuel")]
  let instanceOutcome := fun (f : String) =>
    match incomplete.find? (·.1 == f) with
    | some (_, why) => Outcome.notRechecked s!"not every instance is re-checked: {why}"
    | none =>
      if depthHit.contains f then
        .notRechecked s!"polymorphic recursion: an instance chain deeper than {caps.depth}"
      else if countHit then .notRechecked s!"more than {caps.count} instances in the document"
      else
        let mine := results.filter (·.1 == f) |>.map (·.2)
        match mine.find? (fun o => match o with | .rejected _ => true | _ => false) with
        | some o => o
        | none =>
          if mine.isEmpty then .notRechecked "never instantiated at a ground type"
          else .accepted s!"at {mine.length} instance(s)"
  let own : List (String × Outcome) := code.map fun d =>
    (named d, if d.kind != "fun" then .notRechecked s!"a `{d.kind}` declaration"
      else match prepOf (named d) with
        | some (.ok pr) =>
          if pr.nested then
            if pr.fn.tparams.isEmpty then .notRechecked "a call at type arguments that are not ground"
            else instanceOutcome (named d)
          else match checkWith cats hl recs pr pr.fn with
            | .ok () => .accepted ""
            | .error why => classify why
        | some (.error why) => .notRechecked why
        | none => .notRechecked "not read")
  -- **No accept without a theorem.** A declaration is checked against its callees' signatures,
  -- and T6 needs every function of the program to pass T-Fun, so a declaration is accepted only
  -- when every function it reaches, transitively, is accepted too. A `via` function is checked by
  -- B1's restrictions instead (`checkB1`). A prelude function is exempt only as one F1 gives a
  -- semantics to (`map`, `filter`, `first_where`, `fold`, `parse_int`, `sum`): any other function
  -- outside the document, such as `prelude.append` or a library function, is not re-checked. A
  -- declaration's own rejection is kept: it is a flow the model forbids, whatever it reaches.
  -- What it reaches is everything except capability handler bodies: an operation is an event of
  -- the model (M2), so a handler's body is not in the reach set.
  let reach : List (String × List String) := (decls.filter (·.kind == "fun")).map fun d =>
    (named d, (rawCallees (d.value.getObjValD "body")).eraseDups)
  let isVia := fun (f : String) => (vias.find? (·.1 == f)).isSome
  let close := fun (start : List (String × Outcome)) => Id.run do
    let mut final := start
    for _ in [0:reach.length + 1] do
      final := final.map fun (f, o) =>
        match o with
        | .accepted note =>
          let bad := ((List.lookup f reach).getD []).filterMap fun g =>
            if g == f || isVia g then none
            else match List.lookup g final with
              | some (.accepted _) | some (.acceptedIfA3 _) => none
              | none =>
                if preludeF1.contains g then none
                else some s!"reaches `{g}`, outside the document"
              | some (.rejected _) => some s!"reaches `{g}`, which is rejected"
              | some (.notRechecked why) => some s!"reaches `{g}`, which is not re-checked ({why})"
          match bad with
          | why :: _ => (f, .notRechecked why)
          | [] => (f, .accepted note)
        | o => (f, o)
    pure final
  let mut final := close own
  -- **The whole program (M4 completion, part A2).** The declarations accepted so far, each
  -- checked in its own program with its callees' signatures, are read into one F1 program `P`
  -- with their real bodies, every `via` function, the document's policies, records and
  -- categories, its one operation and the host's labels. A declaration stays accepted only if
  -- `P` passes `Check.progOk`, whose soundness (`Check.progOk_sound`) gives
  -- `P.LWellFormed ∧ P.B1WellFormed ∧ P.HostWF`. A function that fails in `P` is demoted
  -- with what reaches it, and `P` is checked again, at most once per declaration; the verdict
  -- rests on the last check, of `P` itself.
  let readOf := fun (f : String) => match prepOf f with
    | some (.ok pr) => some pr
    | _ => none
  let accepted := fun (outs : List (String × Outcome)) =>
    outs.filterMap fun (f, o) => match o with
      | .accepted _ => some f
      | _ => none
  let demote := fun (outs : List (String × Outcome)) (fs : List (String × String)) =>
    close (outs.map fun (f, o) => match List.lookup f fs with
      | some why => (f, .notRechecked why)
      | none => (f, o))
  -- One operation: F1's program has one (G3).
  let ops := ((accepted final).filterMap fun f => (readOf f).bind (·.op)).eraseDups
  if ops.length > 1 then
    final := demote final ((accepted final).filterMap fun f =>
      match (readOf f).bind (·.op) with
      | some _ => some (f, "the document's accepted declarations use several operations; F1 has one")
      | none => none)
  let acceptsOp := match (accepted final).findSome? fun f =>
      (readOf f).bind fun pr => pr.op.map fun _ => pr.acceptsOp with
    | some a => a
    | none => Label.bot
  let some pr0 := (accepted final).findSome? readOf | pure final
  let viaFuns := pr0.callees.filter fun fn => isVia fn.name
  let mkProg := fun (members : List String) =>
    mkProgram cats acceptsOp hl recs pr0.pols (members.filterMap (fun f => (readOf f).map (·.fn)) ++ viaFuns)
  -- The functions checked at their instances: their T-Fun is part A3's hypothesis.
  let cond := (accepted final).filter isNested
  let docWhy := fun (P : Program) =>
    if !Check.hostWfB P then "the host's labels"
    else if !Check.b1Ok P then "B1 restriction"
    else if !Check.catsDeclared P then "a declared label names a category the document does not"
    else match P.policies.find? (!Check.policyOk P ·), P.records.find? (!Check.recordOk P ·) with
      | some po, _ =>
        s!"the policy `{po.name}` and its `via` function `{po.via}` do not agree on its types, \
          or a type of it is not closed"
      | none, some rec => s!"the record `{rec.name}`: a field that is not ground, or above its deep label"
      | none, none => "the whole-program check"
  let p0 := mkProg (accepted final)
  if !Check.progDocOk p0 then
    let why := docWhy p0
    return demote final ((accepted final).map fun f => (f, s!"whole program: {why}"))
  match viaFuns.find? fun fn => !Check.progFunOk cond p0 fn with
  | some v =>
    return demote final ((accepted final).map fun f =>
      (f, s!"whole program: the `via` function `{v.name}` has no structural typing the checker derives"))
  | none => pure ()
  let rounds := (accepted final).length + 1
  let mut settled := false
  for _ in [0:rounds] do
    let members := accepted final
    let P := mkProg members
    let bad := members.filterMap fun f => match readOf f with
      | some pr =>
        if Check.progFunOk cond P pr.fn then none
        else
          let why := match Check.checkFun P pr.fn with
            | .error e => e
            | .ok () => "its signature"
          some (f, s!"whole program: {why}")
      | none => some (f, "whole program: not read")
    if bad.isEmpty then
      settled := true
      break
    final := demote final bad
  if !settled then
    return demote final ((accepted final).map fun f => (f, "whole program: the check did not settle"))
  -- The certificates. `P`, the document's program, with the functions checked at their
  -- instances as they are (part A2's hypothesis `hA3` for them); the program of the declarations
  -- that reach none of them; and `P₁`, where each of them is replaced by its instances and the
  -- calls of it are renamed to the instance they reach (part A3), which agrees with `P` up to
  -- annotations. `finalize` gives every verdict from them.
  let members := accepted final
  -- The functions checked at their instances among the final members: one demoted in the rounds
  -- above leaves with its instances, which would otherwise keep `P₁` from agreeing with `P`.
  let (cond, mono, ren) := monoParts members isNested (fun f => (readOf f).map (·.fn)) instFuns viaFuns
  let reachesCond := Id.run do
    let mut rc := cond
    for _ in [0:members.length + 1] do
      rc := members.filter fun f =>
        rc.contains f || ((List.lookup f reach).getD []).any rc.contains
    pure rc
  let plain := members.filter fun f => !reachesCond.contains f
  -- Every write site of the document, in a declaration of any kind (Una-1's finding on #471).
  let opNames := fun (cj : J) => match plainRun (do
      (← arr (← field cj "ops")).mapM fun o => do str (← field o "name")) with
    | .ok ns => ns
    | .error _ => []
  let known : Caps' :=
    (decls.filter (·.kind == "cap")).filterMap (fun d =>
      match plainRun (do str (← field d.value "name")) with
      | .ok n => some (d.module, n, opNames d.value)
      | .error _ => none) ++ stdHost
  let sites := decls.filterMap (fun d =>
      (writeSite known d.value).map fun w => (named d, d.kind, w)) ++
    (hostSites host).map fun w => ("std.host", "host", w)
  -- The document performs no host operation (#472): no declaration of any kind, lambdas and
  -- handler bodies included, holds an operation or a grant of `Files` or `Http`, or a node the scan
  -- cannot identify, and its `host` key is `std.host`'s. Then no caller in it can hand a
  -- declaration a closure that writes, and no package imports another (`spec/10 §4`), so every
  -- accepted declaration keeps the any-host claim.
  let docFree := sites.isEmpty && decls.all fun d => (writeSite known d.value true).isNone
  pure (guardFiles sites (finalize docFree (mkProg members) (mkProg plain) cond mono ren final))

/-- `checkDocumentWith` with labeled instantiation, `LTy.lsubst`. -/
def checkDocument (doc : J) : Except String (List (String × Outcome)) :=
  checkDocumentWith (fun rd σ t => t.lsubst rd σ) doc

/-- What the model says of a declaration the checker rejected for information flow (ADR 0091).
Never "accepted": a rejected declaration is not certified Core. -/
inductive ReverseOutcome where
  /-- The model's checker accepts it: a candidate, either an over-strict checker or a channel
  the model lacks (a blind spot of T6), for the coordinator's review before any conclusion. -/
  | modelAcceptsCandidate (note : String)
  /-- The model rejects it too. -/
  | modelRejects (why : String)
  /-- A form the model lacks. -/
  | notRechecked (why : String)

/-- The document `doc` with each declaration of its `rejected` key (ADR 0091) put back in its
module, read as complete: the context the reverse comparison checks a rejected declaration in.
Declarations that the export leaves out for other diagnostics stay out, so a rejected declaration
that reaches one is not re-checked. -/
def withRejected (doc : J) (rejected : List (String × J)) : Except String J := do
  let modules ← plainRun (do arr (← field doc "modules"))
  let paths := modules.filterMap fun m => match m.getObjValD "path" with
    | .str p => some p
    | _ => none
  let extra := (rejected.map Prod.fst).eraseDups.filter (!paths.contains ·)
  let modules := modules ++ extra.map fun p =>
    Lean.Json.mkObj [("path", .str p), ("decls", .arr #[]), ("examples", .arr #[])]
  let modules := modules.map fun m => match m.getObjValD "path", m.getObjValD "decls" with
    | .str p, .arr ds =>
      m.setObjVal! "decls" (.arr (ds ++ ((rejected.filter (·.1 == p)).map Prod.snd).toArray))
    | _, _ => m
  pure ((doc.setObjVal! "modules" (.arr modules.toArray)).setObjVal! "complete" (.bool true))

/-- The reverse comparison (ADR 0091 decision 7): each declaration of `doc`'s `rejected` key,
checked in the context of the document's declarations and the other rejected ones, with what the
model says of it. The outcomes of the declarations of `modules` are not computed here: the default
mode computes them without `rejected`.

**Neither outcome certifies anything.** The context is read as complete although the export leaves
out the declarations with other diagnostics, and whole-program inputs (the file-state label, the
policies) come from the export or from what is present. A missing declaration or input can make a
false `modelAcceptsCandidate`, and it can hide a real one as a false `modelRejects`. The outcomes
are leads for review, never verdicts, and never enter the corpus figures. -/
def checkRejected (doc : J) : Except String (List (String × ReverseOutcome)) := do
  let entries ← match doc.getObjVal? "rejected" with
    | .ok (.arr es) => pure es.toList
    | .ok _ => throw "`rejected` is not a list"
    | .error _ => pure []
  let rejected ← entries.mapM fun e => plainRun (do
    pure (← str (← field e "module"), ← field e "decl"))
  if rejected.isEmpty then return []
  let names := rejected.filterMap fun (m, dj) =>
    let v := match dj.getObjVal? "value" with
      | .ok v => v
      | .error _ => dj
    match v.getObjValD "name" with
    | .str n => some s!"{m}.{n}"
    | _ => none
  let outcomes ← checkDocument (← withRejected doc rejected)
  pure (outcomes.filterMap fun (n, o) =>
    if !names.contains n then none
    else some (n, match o with
      | .accepted note | .acceptedIfA3 note => .modelAcceptsCandidate note
      | .rejected why => .modelRejects why
      | .notRechecked why => .notRechecked why))

end Una.F1.Import
