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
-/
import Lean.Data.Json
import UnaFormal.F1.Check

namespace Una.F1.Import

abbrev J := Lean.Json

/-- What reading one declaration gathers: the operations it mentions, as capability and
operation names, and the functions and policies it refers to. -/
structure Refs where
  ops : List (String × String) := []
  funs : List String := []
  policies : List String := []

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
      | _, _ =>
        if c.funs.contains f then
          refFun f
          -- A call of a `via` function brings its policy, which makes the call U4005.
          if let some pol := List.lookup f c.vias then refPolicy pol
          let targs ← (← arr (← field j "type_args")).mapM (ty c)
          if !Check.groundAll targs then
            throw "a call with type parameters as type arguments (F1 has one level of generics)"
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
  let ps ← (← arr (← field v "params")).mapM fun q => do
    match ← arr q with
    | [x, t] => pure (← str x, ← ty c t)
    | _ => throw "a parameter"
  let uses ← arr (← field v "uses")
  for u in uses do
    let cap ← str (← field u "capability")
    for o in ← arr (← field u "ops") do
      let o ← str o
      modify fun s => { s with ops := if s.ops.contains (cap, o) then s.ops else s.ops ++ [(cap, o)] }
  pure {
    name, tparams := tps, eqParams := eqs, params := ps, result := ← ty c (← field v "result"),
    op := !uses.isEmpty, get := false, fails := !isNull (← field v "fails"), body }

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

/-- The `kind` of a node the checker does not know: an outcome per declaration. -/
inductive Outcome where
  | accepted
  | rejected (why : String)
  | notRechecked (why : String)

/-- Checks one function declaration of a document. -/
def checkFun (cats : List String) (caps : List (String × J)) (decls : List Decl)
    (recs : List Record) (d : Decl) : Outcome :=
  let funNames := decls.filterMap fun e =>
    if e.kind == "fun" then
      match plainRun (do pure s!"{e.module}.{← str (← field e.value "name")}") with
      | .ok n => some n
      | .error _ => none
    else none
  let vias := decls.filterMap fun e =>
    if e.kind == "policy" then
      match plainRun (do pure (← path (← field e.value "via"), s!"{e.module}.{← str (← field e.value "name")}")) with
      | .ok q => some q
      | .error _ => none
    else none
  let c : Ctx := { records := recs.map (·.name), funs := funNames, vias }
  let findDecl := fun (q : String) => decls.find? fun e =>
    match plainRun (do pure s!"{e.module}.{← str (← field e.value "name")}") with
    | .ok n => n == q && (e.kind == "fun" || e.kind == "policy")
    | .error _ => false
  let read : M (Fun × List Fun × List Policy) := do
    let fn ← signature c d.module d.value (← expr c (← field d.value "body"))
    let pols ← (← get).policies.mapM fun pol => do
      let some e := findDecl pol | throw s!"the policy `{pol}`"
      let via ← path (← field e.value "via")
      refFun via
      let src ← ty c (← field e.value "from")
      let dst ← ty c (← field e.value "to")
      pure ({ name := pol, src, dst, via } : Policy)
    let callees ← (← get).funs.mapM fun f => do
      let some e := findDecl f | throw s!"the function `{f}`"
      signature c e.module e.value (.atom .unit)
    pure (fn, callees, pols)
  match run read with
  | .error why => .notRechecked why
  | .ok ((fn, callees, pols), refs) =>
    if (List.lookup fn.name vias).isSome then .notRechecked "a `via` function, which no label rule checks"
    else
      let acc : Except String Label := match refs.ops with
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
      match acc with
      | .error why => .notRechecked why
      | .ok acceptsOp =>
        let p : Program := {
          categories := cats, acceptsOp, acceptsGet := Label.bot, getResult := Label.bot,
          records := recs, policies := pols, funs := fn :: callees }
        match (do
            Check.checkDecls p
            for f in p.funs do Check.checkSignature f
            Check.checkFun p fn : Check.Result Unit) with
        | .ok () => .accepted
        | .error why => .rejected why

/-- Edition 0.1 allows `==` on a type parameter without `Eq` (U2026 is edition 0.2's); F1 is
edition 0.2, so such a function is outside it. -/
def edition01 (edition : String) : Outcome → Outcome
  | .rejected why =>
    if edition == "0.1" && why.startsWith "`==` on a type without equality" then
      .notRechecked "edition 0.1's `==` on a type parameter without `Eq`"
    else .rejected why
  | o => o

/-- The outcome of every declaration with code, by qualified name; an error when the document is
not one the checker reads. -/
def checkDocument (doc : J) : Except String (List (String × Outcome)) := do
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
  let caps := (decls.filter (·.kind == "cap")).map (fun d => d.value) ++ host
  let caps := caps.filterMap fun cj =>
    match plainRun (do str (← field cj "name")) with
    | .ok n => some (n, cj)
    | .error _ => none
  let cats := categories doc []
  let funNames := (decls.filter (·.kind == "fun")).map named
  let recs := records decls funNames
  pure (code.map fun d =>
    (named d, if d.kind == "fun" then edition01 edition (checkFun cats caps decls recs d)
      else .notRechecked s!"a `{d.kind}` declaration"))

end Una.F1.Import
