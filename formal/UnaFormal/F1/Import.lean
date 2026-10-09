/-
The reading of `una.core/1` (ADR 0083, `spec/13 §8`) into F1, for the checker of M2.

Each function declaration of a complete document is read on its own. A declaration that uses a
form F1 lacks is reported as not re-checked, with the form, and never as accepted (ADR 0083
rule 3). The reading is part of the trusted base of the re-check (`docs/plan/formal-track.md`
§5), so it maps each form to the F1 form with the same meaning and nothing else:

- Core's `gt a b` is F1's `lt b a`, and Core's `eq` is F1's `==` on any type with equality;
- the prelude's `map`, `filter`, `first_where`, `fold` and `parse_int` are F1's axioms of them;
- `Option` and `Result` of the prelude are F1's options and results, the error type of a result
  dropped: F1's failures are codes, so a failure with a payload, a `case` or a handler that reads
  the failure, is outside F1;
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

namespace Una.F1

/-- `t` with the symbolic labels `ℓ*_x`, `x ∈ xs`, added to its top label. -/
def LTy.addStarTop (xs : List String) : LTy → LTy
  | .int l => .int { l with star := l.star ++ xs }
  | .bool l => .bool { l with star := l.star ++ xs }
  | .text l => .text { l with star := l.star ++ xs }
  | .unit l => .unit { l with star := l.star ++ xs }
  | .list t l => .list t { l with star := l.star ++ xs }
  | .option t l => .option t { l with star := l.star ++ xs }
  | .result t l => .result t { l with star := l.star ++ xs }
  | .record r l => .record r { l with star := l.star ++ xs }
  | .fn ps r o g f l => .fn ps r o g f { l with star := l.star ++ xs }
  | .var a l => .var a { l with star := l.star ++ xs }

/-- `t` with `ℓ*_x`, `x ∈ xs`, added at the position `steps` of a result summary, as
`spec/13 §5` writes it (`element`, `arg N`, `result`); `none` at a position F1 does not have. -/
def LTy.addStarAt (xs : List String) : List String → LTy → Option LTy
  | [], t => some (t.addStarTop xs)
  | "element" :: rest, .list t l => (LTy.addStarAt xs rest t).map (.list · l)
  | "arg 1" :: rest, .option t l => (LTy.addStarAt xs rest t).map (.option · l)
  | "arg 1" :: rest, .result t l => (LTy.addStarAt xs rest t).map (.result · l)
  | "result" :: rest, .fn ps r o g f l => (LTy.addStarAt xs rest r).map (.fn ps · o g f l)
  | _, _ => none

mutual
  /-- `t` with `ℓ*_x`, `x ∈ xs`, added at every position: edition 0.1 raises every position of
  a generic function's result by every type argument (D-137). -/
  def LTy.addStarAll (xs : List String) : LTy → LTy
    | .list t l => (LTy.list (LTy.addStarAll xs t) l).addStarTop xs
    | .option t l => (LTy.option (LTy.addStarAll xs t) l).addStarTop xs
    | .result t l => (LTy.result (LTy.addStarAll xs t) l).addStarTop xs
    | .fn ps r o g f l => (LTy.fn (LTy.addStarAlls xs ps) (LTy.addStarAll xs r) o g f l).addStarTop xs
    | t => t.addStarTop xs

  def LTy.addStarAlls (xs : List String) : List LTy → List LTy
    | [] => []
    | t :: ts => LTy.addStarAll xs t :: LTy.addStarAlls xs ts
end

/-- A function type that may perform effects (`op`) or fail, at the top of `t`. -/
def LTy.withFlags (op fails : Bool) : LTy → LTy
  | .fn ps r o g f l => .fn ps r (o || op) g (f || fails) l
  | t => t

end Una.F1

namespace Una.F1.Import

abbrev J := Lean.Json

/-- What reading one declaration gathers: the operations it mentions, as capability and
operation names, and the functions and policies it refers to. -/
structure Refs where
  ops : List (String × String) := []
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
  /-- The `via` functions of the document's policies, with their policy. -/
  vias : List (String × String) := []
  /-- Edition 0.1 has no `Eq` constraint: `==` applies to values of any type parameter
  (U2026 is edition 0.2's). The reading constrains every type parameter of a function whose
  body compares values by `Eq`, which only adds the check, at each call, that the type argument
  has equality. -/
  allEq : Bool := false
  /-- The policies each function may release through (`Fun.rels`), computed over the document. -/
  rels : List (String × List String) := []

partial def ty (c : Ctx) (j : J) : M LTy := do
  let l ← label (← field j "label")
  match ← kind j with
  | "fn" =>
    let ps ← (← arr (← field j "params")).mapM (ty c)
    let r ← ty c (← field j "result")
    let uses ← arr (← field j "uses")
    for u in uses do
      let cap ← str (← field u "capability")
      for o in ← arr (← field u "ops") do
        let o ← str o
        modify fun s => { s with ops := if s.ops.contains (cap, o) then s.ops else s.ops ++ [(cap, o)] }
    pure (.fn ps r (!uses.isEmpty) false (!isNull (← field j "fails")) l)
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
      pure (.var a (l.dropPlain a))
    | "named" =>
      let q ← path b
      let args ← arr (← field b "args")
      match q, args with
      | "prelude.List", [t] => pure (.list (← ty c t) l)
      | "prelude.Option", [t] => pure (.option (← ty c t) l)
      | "prelude.Result", t :: _ => pure (.result (← ty c t) l)
      | _, [] => if c.records.contains q then pure (.record q l) else throw s!"the type `{q}`"
      | _, _ => throw s!"the type `{q}` with type arguments"
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

/-- The name a pattern binds, or one nothing reads. -/
def bindName (j : J) : M String := if isNull j then pure "%unread" else str j

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
      | k, _ => throw s!"the primitive `{k}`"
    | "call" =>
      let f ← path (← field j "function")
      let args ← atoms (← field j "args")
      match f, args with
      | "prelude.map", [xs, g] => pure (.map xs g)
      | "prelude.filter", [xs, g] => pure (.filter xs g)
      | "prelude.first_where", [xs, g] => pure (.firstWhere xs g)
      | "prelude.fold", [xs, i, g] => pure (.fold xs i g)
      | "prelude.parse_int", [s] => pure (.parse s)
      | "prelude.sum", [xs] => pure (.prim .sum [xs])
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
      | _, _, _ =>
        if c.records.contains q then pure (.construct q args) else throw s!"a value of `{q}`"
    | "field" =>
      match (← field j "index").getNat? with
      | .ok i => pure (.field (← atom (← field j "value")) i)
      | .error _ => throw "a field index"
    | "list" => pure (.list (← atoms (← field j "value")))
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
          pure (t.withFlags effects fails)
        | .error _ => pure t
      let v ← field j "value"
      let body ← expr c (← field j "body")
      match ← kind v with
      | "rhs" => pure (.letR x t (← rhs c (← field v "value")) body)
      | "expr" => pure (.letE x t (← expr c (← field v "value")) body)
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
        if y != "%unread" && reads y errB then throw "a `case` that reads a failure's payload"
        pure (.caseResult sc x (← expr c okB) "%unread" (← expr c errB))
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
  let usesOf ← match v.getObjVal? "uses_of" with
    | .ok j => (← arr j).mapM str
    | .error _ => pure []
  let failsOf : Option String ← match v.getObjVal? "fails_of" with
    | .ok (.str x) => pure (some x)
    | _ => pure none
  let ps ← (← arr (← field v "params")).mapM fun q => do
    match ← arr q with
    | [x, t] => do
      let x' ← str x
      pure (x', (← ty c t).withFlags (usesOf.contains x') (failsOf == some x'))
    | _ => throw "a parameter"
  -- The result summary (ADR 0067), exported by #443: the symbolic labels a call puts at each
  -- position of the result; in edition 0.1, every position by every type parameter (D-137).
  match v.getObjVal? "of_raises" with
  | .ok (.arr rs) => if !rs.isEmpty then throw "`of` raises (ADR 0067 amendment), outside F1"
  | _ => pure ()
  let declared ← ty c (← field v "result")
  let result ← if tps.isEmpty then pure declared
    else if c.allEq then pure (declared.addStarAll tps)
    else match v.getObjVal? "result_labels" with
      | .ok j => (← arr j).foldlM (fun acc rl => do
          let pos ← str (← field rl "position")
          let from_ ← (← arr (← field rl "from")).mapM str
          let steps := if pos == "top" then [] else pos.splitOn "."
          match acc.addStarAt from_ steps with
          | some t => pure t
          | none => throw s!"a result summary at `{pos}`, outside F1") declared
      | .error _ => pure declared
  let uses ← arr (← field v "uses")
  for u in uses do
    let cap ← str (← field u "capability")
    for o in ← arr (← field u "ops") do
      let o ← str o
      modify fun s => { s with ops := if s.ops.contains (cap, o) then s.ops else s.ops ++ [(cap, o)] }
  pure {
    name, tparams := tps, eqParams := eqs, params := ps, result,
    op := !uses.isEmpty || !usesOf.isEmpty, get := false,
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

/-- The record kinds: kinds with one variant and no type parameters, whose fields F1 reads,
until no more are dropped. -/
def records (decls : List Decl) (funs : List String) : List Record := Id.run do
  let cands := decls.filterMap fun d =>
    if d.kind != "data" then none
    else
      match plainRun (do
          let tps ← arr (← field d.value "type_params")
          if !tps.isEmpty then throw "generic"
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
def prepare (c : Ctx) (caps : List (String × J)) (decls : List Decl) (d : Decl) :
    Except String Prep := do
  let findDecl := fun (q : String) => decls.find? fun e =>
    match plainRun (do pure s!"{e.module}.{← str (← field e.value "name")}") with
    | .ok n => n == q && (e.kind == "fun" || e.kind == "policy")
    | .error _ => false
  let read : M (Fun × List Fun × List Policy) := do
    let fn ← signature c d.module d.value (← expr c (← field d.value "body"))
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

/-- Checks `fn` (the prepared function, or one of its instances) in the program of `prep`. -/
def checkWith (cats : List String) (recs : List Record) (prep : Prep) (fn : Fun) :
    Check.Result Unit := do
  let p : Program := {
    categories := cats, acceptsOp := prep.acceptsOp, acceptsGet := Label.bot,
    getResult := Label.bot, records := recs, policies := prep.pols, funs := fn :: prep.callees }
  Check.checkDecls p
  Check.checkB1 p
  for f in p.funs do Check.checkSignature f
  Check.checkFun p fn

/-- The calls of `j` with type arguments, as F1 reads them: the generic function, its type
arguments (`none` when F1 does not read them), and whether they are ground. Every node is
searched, inside every kind of declaration, so that a function value, a handler or `main` is an
instantiation source too. -/
partial def rawGenericCalls (c : Ctx) (j : J) : List (String × Option (List LTy)) :=
  match j with
  | .obj kvs =>
    let here : List (String × Option (List LTy)) :=
      if j.getObjValD "kind" != .str "call" then []
      else
        match plainRun (do
            let f ← path (← field j "function")
            let targs ← arr (← field j "type_args")
            pure (f, targs)) with
        | .ok (f, targs) =>
          if targs.isEmpty then []
          else [(f, match plainRun (targs.mapM (ty c)) with
            | .ok ts => some ts
            | .error _ => none)]
        | .error _ => []
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
      let here : List String := if k == .str "call" then
          match plainRun (do path (← field j "function")) with
          | .ok f => if f.startsWith "prelude." then [] else [f]
          | .error _ => []
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
        match plainRun (do path (← field j "function")) with
        | .ok f => [f]
        | .error _ => []
      | .str "lambda" =>
        match plainRun (do path (← field j "function")) with
        | .ok f => [f]
        | .error _ => []
      | _ => []
    here ++ kvs.toArray.toList.flatMap (fun kv => rawCallees kv.2)
  | .arr items => items.toList.flatMap rawCallees
  | _ => []

/-- The prelude functions F1 gives a semantics to: the only functions outside the document a
declaration may reach and still be accepted. -/
def preludeF1 : List String :=
  ["prelude.map", "prelude.filter", "prelude.first_where", "prelude.fold", "prelude.parse_int",
    "prelude.sum"]

/-- A rejection by a restriction of option B1 is not a flow the model forbids: the program is
outside B1, so it is not re-checked (ADR 0008). -/
def classify (why : String) : Outcome :=
  if (why.splitOn "B1 restriction").length > 1 then .notRechecked why else .rejected why

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
  let cats := categories doc []
  let funNames := (decls.filter (·.kind == "fun")).map named
  let recs := records decls funNames
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
  let c : Ctx := { records := recs.map (·.name), funs := funNames, vias, allEq := edition == "0.1" }
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
    (decls.filter (·.kind == "fun")).map fun d => (named d, prepare c capDecls decls d)
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
    for (f, targs) in rawGenericCalls c d.value do
      if isNested f then
        match targs with
        | some ts =>
          if Check.groundAll ts then
            if !enqueued.contains (f, keyOf ts) then
              enqueued := enqueued ++ [(f, keyOf ts)]
              queue := queue ++ [(f, ts, named d, 0)]
          else
            match prepOf (named d) with
            | some (.ok pr) =>
              if !(pr.nested && !pr.fn.tparams.isEmpty) then
                incomplete := incomplete ++ [(f, s!"a call in `{named d}` at type parameters it does not instantiate")]
            | _ => incomplete := incomplete ++ [(f, s!"instantiated from `{named d}`, which is outside F1")]
        | none => incomplete := incomplete ++ [(f, s!"instantiated in `{named d}` at types outside F1")]
  let mut done : List (String × List String) := []
  let mut results : List (String × Outcome) := []
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
        let outcome := match checkWith cats recs pr inst with
          | .ok () => Outcome.accepted ""
          | .error why => classify s!"at its instance `{inst.name}`, instantiated in `{origin}`: {why}"
        results := results ++ [(f, outcome)]
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
          else match checkWith cats recs pr pr.fn with
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
  -- `P.LWellFormed ∧ P.B1WellFormed` and `t6`'s `hget`. A function that fails in `P` is demoted
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
  -- The host's labels: those of `Http.get`, the host operation F1 models, or `⊥` when the host
  -- has none.
  let httpGet := host.findSome? fun cj =>
    match plainRun (do str (← field cj "name")) with
    | .ok "Http" =>
      match plainRun (do arr (← field cj "ops")) with
      | .ok ops => ops.find? fun o => match plainRun (do str (← field o "name")) with
        | .ok n => n == "get"
        | .error _ => false
      | .error _ => none
    | _ => none
  let hostLabels : Except String (Label × Label) := match httpGet with
    | none => .ok (Label.bot, Label.bot)
    | some o => plainRun do
      pure ((← label (← field o "accepts")).base, (← label (← field (← field o "result") "label")).base)
  let some pr0 := (accepted final).findSome? readOf | pure final
  let (acceptsGet, getResult) ← match hostLabels with
    | .ok q => pure q
    | .error why =>
      return demote final ((accepted final).map fun f => (f, s!"the host's `Http.get` labels: {why}"))
  let viaFuns := pr0.callees.filter fun fn => isVia fn.name
  let mkProg := fun (members : List String) => ({
    categories := cats, acceptsOp, acceptsGet, getResult, records := recs, policies := pr0.pols,
    funs := members.filterMap (fun f => (readOf f).map (·.fn)) ++ viaFuns } : Program)
  -- The functions checked at their instances: their T-Fun is part A3's hypothesis.
  let cond := (accepted final).filter isNested
  let docWhy := fun (P : Program) =>
    if !Check.leB P.top (.ofLabel P.acceptsGet) (.ofLabel P.getResult) then
      "the host's response is labeled below what `Http.get` accepts"
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
  -- The certificate: the last program, checked whole. Those that reach no function checked at
  -- its instances are certified again, in their own program, with no hypothesis left.
  let members := accepted final
  if !Check.progOk cond (mkProg members) then
    return demote final (members.map fun f => (f, "whole program: the final check failed"))
  let reachesCond := Id.run do
    let mut rc := cond
    for _ in [0:members.length + 1] do
      rc := members.filter fun f =>
        rc.contains f || ((List.lookup f reach).getD []).any rc.contains
    pure rc
  let plain := members.filter fun f => !reachesCond.contains f
  if !Check.progOk [] (mkProg plain) then
    return demote final (members.map fun f =>
      (f, "whole program: the check without functions checked at their instances failed"))
  pure (final.map fun (f, o) => match o with
    | .accepted note => if reachesCond.contains f then (f, .acceptedIfA3 note) else (f, o)
    | o => (f, o))

/-- `checkDocumentWith` with labeled instantiation, `LTy.lsubst`. -/
def checkDocument (doc : J) : Except String (List (String × Outcome)) :=
  checkDocumentWith (fun rd σ t => t.lsubst rd σ) doc

end Una.F1.Import
