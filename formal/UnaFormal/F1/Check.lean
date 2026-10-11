/-
The executable checker of F1 (M2 of `docs/plan/formal-track.md`): the labeled typing of
`LTyping.lean` as total functions, run on the Core IR that `una core --json` exports.

It checks rather than infers. Every binder of Core carries its declared or inferred type, every
function its declared result, so each expression is checked against the type it must flow into,
and the checker never computes a join of types. `checkExpr p E Γ pc s e` succeeds only when the
expression has a labeled typing whose type flows into `s`; the branches of `if`, `case` and a
handler are checked at `s` itself, which is the least type the rules allow there. The proof that
an accepted function passes T-Fun, which makes T6 hold for it, is M4's (option B of the plan); the
rules here follow `LTyping.lean` rule by rule, in the same order, so the proof is by induction on
the same structure.
-/
import UnaFormal.F1.LTyping

namespace Una.F1.Check

/-- A rejection: what failed, in a sentence. -/
abbrev Result := Except String

def need (b : Bool) (msg : String) : Result Unit := if b then pure () else throw msg

def some! {α : Type} (o : Option α) (msg : String) : Result α :=
  match o with
  | some a => pure a
  | none => throw msg

/-- `a ⊑ b` for concrete labels. -/
def labelLe (a b : Label) : Bool :=
  decide (a.level.rank ≤ b.level.rank) && a.cats.all (fun c => b.cats.contains c)

/-- The static order of `spec/07 §7` rule 8 (`LExpr.le`). -/
def leB (top : Label) (a b : LExpr) : Bool :=
  labelLe top b.base ||
    (labelLe a.base b.base && a.plain.all (fun x => b.plain.contains x || b.star.contains x) &&
      a.star.all (fun x => b.star.contains x))

def symFree (e : LExpr) : Bool := e.plain.isEmpty && e.star.isEmpty

mutual
  /-- Equality of types, as a Boolean. -/
  def tyEq : Ty → Ty → Bool
    | .int, .int | .bool, .bool | .text, .text | .unit, .unit => true
    | .list a, .list b | .option a, .option b => tyEq a b
    | .result a e, .result b f => tyEq a b && tyEq e f
    | .record a, .record b | .var a, .var b => a == b
    | .fn ps r, .fn qs s => tysEq ps qs && tyEq r s
    | .tuple ps, .tuple qs => tysEq ps qs
    | _, _ => false

  def tysEq : List Ty → List Ty → Bool
    | [], [] => true
    | a :: as, b :: bs => tyEq a b && tysEq as bs
    | _, _ => false
end

/-- `Program.HttpRecords`, as a Boolean. -/
def httpRecordsB (p : Program) : Bool :=
  match p.record "prelude.Header", p.record "prelude.Response" with
  | some h, some r =>
    tysEq h.fieldTys [.text, .text] && tysEq r.fieldTys [.int, .list (.record "prelude.Header"), .text]
  | _, _ => false

/-- `HostFx.Le`. -/
def hostLeB (a b : HostFx) : Bool := (!a.rd || b.rd) && (!a.wr || b.wr) && (!a.rq || b.rq)

mutual
  /-- `Flow`. -/
  def flowB (top : Label) (a b : LTy) : Bool :=
    match a, b with
    | .int l₁, .int l₂ | .bool l₁, .bool l₂ | .text l₁, .text l₂ => leB top l₁ l₂
    | .unit _, .unit _ => true
    | .list t₁ l₁, .list t₂ l₂ | .option t₁ l₁, .option t₂ l₂ =>
      flowB top t₁ t₂ && leB top l₁ l₂
    | .result t₁ e₁ l₁, .result t₂ e₂ l₂ => flowB top t₁ t₂ && flowB top e₁ e₂ && leB top l₁ l₂
    | .record r₁ l₁, .record r₂ l₂ => r₁ == r₂ && leB top l₁ l₂
    | .fn ps₁ r₁ o₁ g₁ f₁ l₁, .fn ps₂ r₂ o₂ g₂ f₂ l₂ =>
      flowsB top ps₂ ps₁ && flowB top r₁ r₂ && (!o₁ || o₂) && hostLeB g₁ g₂ && (!f₁ || f₂) &&
        leB top l₁ l₂
    | .var x l₁, .var y l₂ => x == y && leB top l₁ l₂
    | .tuple ts₁ l₁, .tuple ts₂ l₂ => flowsB top ts₁ ts₂ && leB top l₁ l₂
    | _, _ => false
  termination_by sizeOf a + sizeOf b

  def flowsB (top : Label) (as bs : List LTy) : Bool :=
    match as, bs with
    | [], [] => true
    | t₁ :: ts₁, t₂ :: ts₂ => flowB top t₁ t₂ && flowsB top ts₁ ts₂
    | _, _ => false
  termination_by sizeOf as + sizeOf bs
end

mutual
  /-- `LTy.InPlace`. -/
  def inPlace : LTy → Bool
    | .var a l => l.plain.all (· == a)
    | .int l | .bool l | .text l | .unit l | .record _ l => l.plain.isEmpty
    | .list t l | .option t l => inPlace t && l.plain.isEmpty
    | .result t e l => inPlace t && inPlace e && l.plain.isEmpty
    | .fn ps r _ _ _ l => inPlaceAll ps && inPlace r && l.plain.isEmpty
    | .tuple ts l => inPlaceAll ts && l.plain.isEmpty

  def inPlaceAll : List LTy → Bool
    | [] => true
    | t :: ts => inPlace t && inPlaceAll ts
end

mutual
  /-- `LTy.Ground`. -/
  def ground : LTy → Bool
    | .var _ _ => false
    | .int l | .bool l | .text l | .unit l | .record _ l => symFree l
    | .list t l | .option t l => ground t && symFree l
    | .result t e l => ground t && ground e && symFree l
    | .fn ps r _ _ _ l => groundAll ps && ground r && symFree l
    | .tuple ts l => groundAll ts && symFree l

  def groundAll : List LTy → Bool
    | [] => true
    | t :: ts => ground t && groundAll ts
end

def symsIn (xs : List String) (e : LExpr) : Bool := (e.plain ++ e.star).all (xs.contains ·)

mutual
  /-- `LTy.ScopedIn`. -/
  def scopedIn (xs : List String) : LTy → Bool
    | .var a l => xs.contains a && symsIn xs l
    | .int l | .bool l | .text l | .unit l | .record _ l => symsIn xs l
    | .list t l | .option t l => scopedIn xs t && symsIn xs l
    | .result t e l => scopedIn xs t && scopedIn xs e && symsIn xs l
    | .fn ps r _ _ _ l => scopedInAll xs ps && scopedIn xs r && symsIn xs l
    | .tuple ts l => scopedInAll xs ts && symsIn xs l

  def scopedInAll (xs : List String) : List LTy → Bool
    | [] => true
    | t :: ts => scopedIn xs t && scopedInAll xs ts
end

/-- `Program.Bound`. -/
def boundB (p : Program) (pc : LExpr) (o : Bool) (g : HostFx) (f : Bool) : Bool :=
  (!o || (leB p.top pc (.ofLabel p.acceptsOp) && symFree pc)) &&
  (!g.rd || (leB p.top pc (.ofLabel p.acceptsRead) && symFree pc)) &&
  (!g.wr || (leB p.top pc (.ofLabel p.acceptsWrite) &&
    (p.oldHost || leB p.top pc (.ofLabel p.fileState)) && symFree pc)) &&
  (!g.rq || (leB p.top pc (.ofLabel p.acceptsReq) && symFree pc)) &&
  (!f || (leB p.top pc LExpr.bot && symFree pc))

/-- `Program.RelBound`. -/
def relBoundB (p : Program) (pc : LExpr) (rs : List String) : Bool :=
  rs.all fun pol => match p.policy pol with
    | some po => leB p.top pc po.dst.top && symFree pc
    | none => false

def isVia (p : Program) (f : String) : Bool := p.policies.any (·.via == f)

/-- The structural type of a primitive's result (`PrimTy`). -/
def primTy : Prim → List Ty → Option Ty
  | .add, [.int, .int] | .sub, [.int, .int] | .mul, [.int, .int] | .div, [.int, .int]
  | .mod, [.int, .int] => some .int
  | .lt, [.int, .int] | .eqInt, [.int, .int] => some .bool
  | .not, [.bool] => some .bool
  | .len, [.list _] | .len, [.text] => some .int
  | .sum, [.list .int] => some .int
  | .concat, ts => if ts.all (tyEq · .text) then some .text else none
  | _, _ => none

/-- A primitive's name, for the reason of a restriction. -/
def primName : Prim → String
  | .add => "add" | .sub => "sub" | .mul => "mul" | .div => "div" | .mod => "mod" | .lt => "lt"
  | .eqInt => "eq_int" | .not => "not" | .len => "len" | .sum => "sum" | .concat => "concat"

/-- A type's head, for the reason of a restriction. -/
def tyHead : Ty → String
  | .int => "Int" | .bool => "Bool" | .text => "Text" | .unit => "Unit" | .list _ => "List"
  | .option _ => "Option" | .result _ _ => "Result" | .record r => r | .fn _ _ => "a function"
  | .var a => a | .tuple _ => "a tuple"

/-- The reason a primitive's operands are outside F1 (#532). Una checked the operands' types, so
a primitive F1 has no typing for is an overload F1 does not model, such as `<` on `Text`: a model
restriction, not re-checked, never rejected, and never read as another overload. -/
def primRestriction (pr : Prim) (ts : List Ty) : String :=
  s!"F1 restriction (`{primName pr}` on {", ".intercalate (ts.map tyHead)})"

def atomTy (Γ : LCtx) : Atom → Result LTy
  | .var x => some! (lctxLookup Γ x) s!"`{x}` is not in scope"
  | .int _ => pure (.int .bot)
  | .bool _ => pure (.bool .bot)
  | .text _ => pure (.text .bot)
  | .unit => pure (.unit .bot)

def atomsTy (Γ : LCtx) (as : List Atom) : Result (List LTy) := as.mapM (atomTy Γ)

def isText : LTy → Bool
  | .text _ => true
  | _ => false

/-- `Ty.Scalar` of the type's structure. -/
def isScalar : LTy → Bool
  | .int _ | .bool _ | .text _ => true
  | _ => false

/-- One type both operands of `==` flow into: the same structure, each label the join. `None`
when the structures differ or hold a function type, which has no equality. -/
def joinTy : LTy → LTy → Option LTy
  | .int l₁, .int l₂ => some (.int (l₁.join l₂))
  | .bool l₁, .bool l₂ => some (.bool (l₁.join l₂))
  | .text l₁, .text l₂ => some (.text (l₁.join l₂))
  | .unit l₁, .unit l₂ => some (.unit (l₁.join l₂))
  | .list t₁ l₁, .list t₂ l₂ => (joinTy t₁ t₂).map (.list · (l₁.join l₂))
  | .option t₁ l₁, .option t₂ l₂ => (joinTy t₁ t₂).map (.option · (l₁.join l₂))
  | .result t₁ e₁ l₁, .result t₂ e₂ l₂ =>
    match joinTy t₁ t₂, joinTy e₁ e₂ with
    | some t, some e => some (.result t e (l₁.join l₂))
    | _, _ => none
  | .record r₁ l₁, .record r₂ l₂ => if r₁ == r₂ then some (.record r₁ (l₁.join l₂)) else none
  | .var a l₁, .var b l₂ => if a == b then some (.var a (l₁.join l₂)) else none
  | _, _ => none

def fmtLevel : Level → String
  | .«public» => "public"
  | .internal => "internal"
  | .confidential => "confidential"
  | .secret => "secret"
  | .restricted => "restricted"

/-- A label as `spec/07` writes it, with `T` and `T*` for the symbolic labels. -/
def fmtLabel (e : LExpr) : String :=
  let cats := if e.base.cats.isEmpty then "" else s!"[{", ".intercalate e.base.cats}]"
  let syms := e.plain ++ e.star.map (· ++ "*")
  " + ".intercalate (s!"{fmtLevel e.base.level}{cats}" :: syms.map (s!"label({·})"))

/-- A labeled type, for the messages of rejections. -/
def fmtTy : LTy → String
  | .int l => s!"{fmtLabel l} Int"
  | .bool l => s!"{fmtLabel l} Bool"
  | .text l => s!"{fmtLabel l} Text"
  | .unit l => s!"{fmtLabel l} Unit"
  | .list t l => s!"{fmtLabel l} List[{fmtTy t}]"
  | .option t l => s!"{fmtLabel l} Option[{fmtTy t}]"
  | .result t e l => s!"{fmtLabel l} Result[{fmtTy t}, {fmtTy e}]"
  | .record r l => s!"{fmtLabel l} {r}"
  | .fn ps r o g f l =>
    let fx := (if o then " uses op" else "") ++ (if g.rd then " uses read" else "") ++
      (if g.wr then " uses write" else "") ++ (if g.rq then " uses request" else "") ++
      (if f then " fails" else "")
    s!"{fmtLabel l} fn({", ".intercalate (ps.map fmtTy)}) -> {fmtTy r}{fx}"
  | .var a l => s!"{fmtLabel l} {a}"
  | .tuple ts l => s!"{fmtLabel l} ({", ".intercalate (ts.map fmtTy)})"

/-- `Fun.EqArgs`. -/
def eqArgs (p : Program) (E : List String) (fn : Fun) (targs : List LTy) : Bool :=
  (fn.tparams.zip targs).all (fun q => !fn.eqParams.contains q.1 || p.eqIn E q.2)

/-- Whether a type holds a value of a sum kind at any position (#512). -/
def sumIn (p : Program) : LTy → Bool
  | .record k _ => (p.sum k).isSome
  | .list t _ | .option t _ => sumIn p t
  | .result t e _ => sumIn p t || sumIn p e
  | .fn ps r _ _ _ _ => ps.attach.any (fun ⟨q, _⟩ => sumIn p q) || sumIn p r
  | .tuple ts _ => ts.attach.any (fun ⟨q, _⟩ => sumIn p q)
  | _ => false

/-- The premises T-Call and a function reference share. -/
def callable (p : Program) (E : List String) (f : String) (targs : List LTy) : Result Fun := do
  let fn ← some! (p.find f) s!"no function `{f}`"
  need (!isVia p f) s!"`{f}` is the `via` function of a policy (U4005)"
  need (targs.length == fn.tparams.length) s!"`{f}` takes {fn.tparams.length} type arguments"
  need (LTy.firstClasses targs) s!"a type argument of `{f}` is second-class"
  need (eqArgs p E fn targs) s!"a type argument of `{f}` has no equality"
  need (groundAll targs) s!"a type argument of `{f}` is not ground"
  pure fn

def flowTo (p : Program) (t ann : LTy) (what : String) : Result Unit :=
  need (flowB p.top t ann) s!"{what}, `{fmtTy t}`, does not flow into `{fmtTy ann}`"

def le (p : Program) (a b : LExpr) (what : String) : Result Unit :=
  need (leB p.top a b) what

mutual
  /-- `LRhsTy`, checked against the type `ann` of the `let` that binds it. -/
  def checkRhs (p : Program) (E : List String) (Γ : LCtx) (pc : LExpr) (ann : LTy) :
      Rhs → Result Eff
    | .atom a => do
      flowTo p (← atomTy Γ a) ann "the value"
      pure .none
    | .prim pr args => do
      let ts ← atomsTy Γ args
      let rt ← some! (primTy pr (ts.map LTy.erase)) (primRestriction pr (ts.map LTy.erase))
      flowTo p (ofScalar rt (p.primLabel pr ts)) ann "the primitive's result"
      pure .none
    | .call f targs args => do
      let fn ← callable p E f targs
      let ts ← atomsTy Γ args
      let σ := fn.tparams.zip targs
      need (flowsB p.top ts (fn.params.map (fun q => p.lsub σ q.2)))
        s!"an argument of `{f}` does not flow into its parameter"
      need (boundB p pc fn.op fn.host fn.fails)
        s!"`{f}` has effects or failures under a program counter above its bound"
      need (relBoundB p pc fn.rels)
        s!"B1 restriction (call of a declassifying function under a raised pc): `{f}`"
      flowTo p (p.lsub σ fn.result) ann s!"the result of `{f}`"
      pure (.ofCall fn)
    | .apply f args => do
      match ← atomTy Γ f with
      | .fn ps r o g fl lf =>
        let ts ← atomsTy Γ args
        let d := decision (decisionsOf ts)
        need (flowsB p.top ts (ps.map (LTy.raise d)))
          "an argument of a function value does not flow into its parameter"
        need (boundB p (pc.join (lf.join d)) o g fl)
          "a function value has effects or failures under a program counter above its bound"
        flowTo p (r.raise (lf.join d)) ann "the result of a function value"
        pure (.ofFn o g fl)
      | _ => throw "a call of a value that is not a function"
    | .op line => do
      let t ← atomTy Γ line
      need (isScalar t) "the operation's argument is not a scalar"
      le p (pc.join (p.deep t)) (.ofLabel p.acceptsOp)
        "the operation's argument or program counter is above what it accepts"
      need (symFree pc) "an operation under a symbolic program counter"
      flowTo p (.unit .bot) ann "the operation's result"
      pure ⟨true, HostFx.none, Option.none, [], false, []⟩
    | .read path => do
      let t ← atomTy Γ path
      need (isText t) "`Files.read`'s path is not a text"
      le p (pc.join (p.deep t)) (.ofLabel p.acceptsRead)
        "`Files.read`'s path or program counter is above what it accepts"
      need (symFree pc) "a host operation under a symbolic program counter"
      flowTo p (.text (.ofLabel p.readResult)) ann "`Files.read`'s result"
      pure ⟨false, { rd := true }, some (p.fsFail (pc.join (p.deep t))), [], true, []⟩
    | .write path content => do
      let t ← atomTy Γ path
      need (isText t) "`Files.write`'s path is not a text"
      let c ← atomTy Γ content
      need (isText c) "`Files.write`'s content is not a text"
      le p ((pc.join (p.deep t)).join (p.deep c)) (.ofLabel p.acceptsWrite)
        "`Files.write`'s path, content or program counter is above what it accepts"
      need (p.oldHost || leB p.top (pc.join (p.deep t)) (.ofLabel p.fileState))
        "a write whose path or program counter is above the file-state label (ADR 0085)"
      need (symFree pc) "a host operation under a symbolic program counter"
      flowTo p (.unit .bot) ann "`Files.write`'s result"
      pure ⟨false, { wr := true }, some (p.fsFail (pc.join (p.deep t))), [], true, []⟩
    | .req m args => do
      let ts ← atomsTy Γ args
      need (match httpParams m with
          | some ps => tysEq (ts.map LTy.erase) ps
          | none => false)
        s!"a request `{m}` whose arguments are not of `Http`'s parameter types (`spec/24 §1`)"
      need (httpRecordsB p) "the program's `Header` and `Response` are not `spec/24`'s"
      le p (pc.join (LExpr.joinAll (ts.map p.deep))) (.ofLabel p.acceptsReq)
        "a request's arguments or program counter are above what it accepts"
      need (symFree pc) "a host operation under a symbolic program counter"
      flowTo p (.record "prelude.Response" (.ofLabel p.reqResult)) ann "the response"
      pure ⟨false, { rq := true }, some (p.reqFail pc ts), [], true, []⟩
    | .construct record fields => do
      let rec ← some! (p.record record) s!"F1 restriction (no record `{record}`, a lookup miss)"
      need (flowsB p.top (← atomsTy Γ fields) rec.fields)
        s!"a field of `{record}` does not flow into its declared type"
      flowTo p (.record record .bot) ann s!"the `{record}` value"
      pure .none
    | .variant kind ctor fields => do
      let sm ← some! (p.sum kind) s!"F1 restriction (no sum `{kind}`, a lookup miss)"
      let fs ← some! sm.variants[ctor]? s!"`{kind}` has no variant {ctor}"
      need (flowsB p.top (← atomsTy Γ fields) fs)
        s!"a field of `{kind}` does not flow into its declared type"
      flowTo p (.record kind .bot) ann s!"the `{kind}` value"
      pure .none
    | .field value index => do
      match ← atomTy Γ value with
      | .record record l =>
        let rec ← some! (p.record record) s!"F1 restriction (no record `{record}`, a lookup miss)"
        let t ← some! rec.fields[index]? s!"`{record}` has no field {index}"
        flowTo p (t.raise l) ann "the field"
        pure .none
      | _ => throw "a field of a value that is not a record"
    | .list items => do
      match ann with
      | .list t _ =>
        let ts ← atomsTy Γ items
        need (ts.all (flowB p.top · t)) "an element does not flow into the list's element type"
        need (inPlace t) "`ℓ_T` off a position of type `T`"
        flowTo p (.list t .bot) ann "the list"
        pure .none
      | _ => throw "a list bound at a type that is not a list"
    | .none => do
      match ann with
      | .option t _ =>
        need (inPlace t) "`ℓ_T` off a position of type `T`"
        pure .none
      | _ => throw "`None` bound at a type that is not an option"
    | .some a => do
      flowTo p (.option (← atomTy Γ a) .bot) ann "the option"
      pure .none
    | .first a => do
      match ← atomTy Γ a with
      | .list t l =>
        flowTo p (.option t l) ann "the first element"
        pure .none
      | _ => throw "`first` of a value that is not a list"
    | .mkOk a => do
      match ann with
      | .result _ e _ =>
        need (inPlace e) "`ℓ_T` off a position of type `T`"
        flowTo p (.result (← atomTy Γ a) e .bot) ann "the result"
        pure .none
      | _ => throw "`Ok` bound at a type that is not a result"
    | .mkErr a => do
      match ann with
      | .result t _ _ =>
        need (inPlace t) "`ℓ_T` off a position of type `T`"
        flowTo p (.result t (← atomTy Γ a) .bot) ann "the result"
        pure .none
      | _ => throw "`Err` bound at a type that is not a result"
    | .lambda params body => do
      match ann with
      | .fn ps r o g fl _ =>
        need (params.all (fun q => inPlace q.2)) "`ℓ_T` off a position of type `T`"
        let ε ← checkExpr p E (params ++ Γ) .bot r body
        le p ε.failLabel .bot "a lambda's failure is decided above `public`"
        need ε.rels.isEmpty "B1 restriction (declassification in a lambda)"
        need ε.raises.isEmpty "F1 restriction (a lambda that fails with a payload kind)"
        need (flowsB p.top ps (params.map Prod.snd))
          "the declared parameters do not flow into the lambda's"
        need ((!ε.op || o) && hostLeB ε.host g && (!ε.fails.isSome || fl))
          "the lambda has effects or failures its declared type does not allow"
        pure .none
      | _ => throw "a lambda bound at a type that is not a function type"
    | .funRef f targs => do
      let fn ← callable p E f targs
      need fn.rels.isEmpty s!"B1 restriction (a declassifying function as a value): `{f}`"
      need fn.raises.isEmpty s!"F1 restriction (a function that fails with a payload kind, as a value): `{f}`"
      let σ := fn.tparams.zip targs
      flowTo p (.fn (fn.params.map (fun q => p.lsub σ q.2)) (p.lsub σ fn.result)
        fn.op fn.host fn.fails .bot) ann s!"`{f}` as a value"
      pure .none
    | .declassify pol a => do
      let po ← some! (p.policy pol) s!"no policy `{pol}`"
      let fn ← some! (p.find po.via) s!"no function `{po.via}`"
      flowTo p (← atomTy Γ a) po.src s!"the argument of `{pol}`"
      le p pc po.dst.top s!"`{pol}` is called under a program counter above its `to`"
      need (symFree pc) s!"`{pol}` is called under a symbolic program counter"
      flowTo p po.dst ann s!"the result of `{pol}`"
      pure (.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol)
    | .map xs f => do
      match ← atomTy Γ xs, ← atomTy Γ f with
      | .list te lx, .fn [tp] tr o g fl lf =>
        flowTo p te tp "an element given to `map`'s function"
        need (boundB p ((pc.join lx).join lf) o g fl)
          "`map`'s function has effects or failures above its bound"
        flowTo p (.list (tr.raise lf) (lx.join lf)) ann "the result of `map`"
        pure (.ofFn o g fl)
      | _, _ => throw "`map` on values of the wrong types"
    | .filter xs keep => do
      match ← atomTy Γ xs, ← atomTy Γ keep with
      | .list te lx, .fn [tp] (.bool lb) o g fl lf =>
        flowTo p te tp "an element given to `filter`'s predicate"
        need (boundB p ((pc.join lx).join lf) o g fl)
          "`filter`'s predicate has effects or failures above its bound"
        flowTo p (.list te ((lx.join lf).join lb)) ann "the result of `filter`"
        pure (.ofFn o g fl)
      | _, _ => throw "`filter` on values of the wrong types"
    | .firstWhere xs keep => do
      match ← atomTy Γ xs, ← atomTy Γ keep with
      | .list te lx, .fn [tp] (.bool lb) o g fl lf =>
        flowTo p te tp "an element given to `first_where`'s predicate"
        need (boundB p ((((pc.join lx).join lf).join (p.deep te)).join lb) o g fl)
          "`first_where`'s predicate has effects or failures above its bound"
        flowTo p (.option te ((lx.join lf).join lb)) ann "the result of `first_where`"
        pure (.ofFn o g fl)
      | _, _ => throw "`first_where` on values of the wrong types"
    | .fold xs init f => do
      match ← atomTy Γ xs, ← atomTy Γ init, ← atomTy Γ f with
      | .list te lx, ti, .fn [ta, tp] tr o g fl lf =>
        flowTo p te tp "an element given to `fold`'s function"
        flowTo p ti ta "`fold`'s initial value"
        flowTo p tr ta "the result of `fold`'s function"
        need (boundB p ((pc.join lx).join lf) o g fl)
          "`fold`'s function has effects or failures above its bound"
        flowTo p (ta.raise (lx.join lf)) ann "the result of `fold`"
        pure (.ofFn o g fl)
      | _, _, _ => throw "`fold` on values of the wrong types"
    | .parse s => do
      let t ← atomTy Γ s
      need (isText t) "`parse_int` of a value that is not a text"
      flowTo p (.int (p.deep t)) ann "the result of `parse_int`"
      pure ⟨false, HostFx.none, some (pc.join (p.deep t)), [], true, []⟩
    | .checked a _ _ => do
      let t ← atomTy Γ a
      need (isScalar t) "`checked` of a value that is not a scalar"
      le p (pc.join (p.deep t)) LExpr.bot
        "`checked` of a labeled value, or under a raised program counter (U4002)"
      need (symFree pc) "`checked` under a symbolic program counter"
      flowTo p t ann "the value `checked` gives"
      pure ⟨false, HostFx.none, some (pc.join (p.deep t)), [], true, []⟩
    | .eq a b => do
      let t₁ ← atomTy Γ a
      let t₂ ← atomTy Γ b
      -- Una checked that both sides have one type with equality (#532): a pair F1 cannot join, or
      -- a type F1 gives no equality, is a reading F1 does not model, not re-checked.
      let t ← some! (joinTy t₁ t₂) "F1 restriction (`==` on values F1 reads at different types)"
      -- F1 has no `==` on sums (#512): a model restriction, not re-checked, never rejected.
      need (p.eqIn E t || sumIn p t) "F1 restriction (`==` on a type without equality in F1)"
      need (p.eqIn E t) "F1 restriction (`==` on a sum)"
      flowTo p (.bool ((p.deep t₁).join (p.deep t₂))) ann "the result of `==`"
      pure .none
    | .tuple items => do
      flowTo p (.tuple (← atomsTy Γ items) .bot) ann "the tuple"
      pure .none
    | .toText a => do
      let t ← atomTy Γ a
      need (p.eqIn E t || sumIn p t) "F1 restriction (`to_text` of a type without equality in F1)"
      need (p.eqIn E t) "F1 restriction (`to_text` of a sum)"
      flowTo p (.text (p.deep t)) ann "the text of a value"
      pure .none
    | .envGet name => do
      let t ← atomTy Γ name
      need (isText t) "`Env.get`'s name is not a text"
      le p (p.deep t) .bot "`Env.get`'s name is above what it accepts, `public`"
      flowTo p (.option (.text (.ofLabel ⟨.internal, []⟩)) .bot) ann "`Env.get`'s result"
      pure .none
    | .append xs item => do
      match ← atomTy Γ xs with
      | .list te lx =>
        flowTo p (← atomTy Γ item) te "the item given to `append`"
        flowTo p (.list te lx) ann "the result of `append`"
        pure .none
      | _ => throw "`append` to a value that is not a list"
    | .set xs index item => do
      match ← atomTy Γ xs, ← atomTy Γ index with
      | .list te lx, .int li =>
        le p li .bot "`set`'s index is above `public`"
        flowTo p (← atomTy Γ item) te "the item given to `set`"
        flowTo p (.tuple [.list te lx, .unit .bot] .bot) ann "the result of `set`"
        pure .none
      | _, _ => throw "`set` on values of the wrong types"

  /-- `LExprTy`, checked against the type `s` its value must flow into. -/
  def checkExpr (p : Program) (E : List String) (Γ : LCtx) (pc : LExpr) (s : LTy) :
      Expr → Result Eff
    | .atom a => do
      flowTo p (← atomTy Γ a) s "the value"
      pure .none
    | .letR x ann value body => do
      need (inPlace ann) s!"`ℓ_T` off a position of type `T` in the type of `{x}`"
      let ε₁ ← checkRhs p E Γ pc ann value
      let ε₂ ← checkExpr p E ((x, ann) :: Γ) (pc.join ε₁.failLabel) s body
      pure (ε₁.union ε₂)
    | .letE x ann value body => do
      need (inPlace ann) s!"`ℓ_T` off a position of type `T` in the type of `{x}`"
      let ε₁ ← checkExpr p E Γ pc ann value
      let ε₂ ← checkExpr p E ((x, ann) :: Γ) (pc.join ε₁.failLabel) s body
      pure (ε₁.union ε₂)
    | .ite cond thenE elseE => do
      match ← atomTy Γ cond with
      | .bool lc =>
        match s with
        | .tuple ts _ =>
          -- Core's join of updated bindings (M6b): the condition's label goes into each component.
          need (inPlaceAll ts) "`ℓ_T` off a position of type `T`"
          let ε₁ ← checkExpr p E Γ (pc.join lc) (.tuple ts .bot) thenE
          let ε₂ ← checkExpr p E Γ (pc.join lc) (.tuple ts .bot) elseE
          need (flowsB p.top (ts.map (LTy.raise lc)) ts)
            "a component of the joined state is below the condition's label (U4001)"
          pure (ε₁.union ε₂)
        | _ =>
        need (inPlace s) "`ℓ_T` off a position of type `T`"
        let ε₁ ← checkExpr p E Γ (pc.join lc) s thenE
        let ε₂ ← checkExpr p E Γ (pc.join lc) s elseE
        flowTo p (s.raise lc) s "the value of an `if`, raised by its condition,"
        pure (ε₁.union ε₂)
      | _ => throw "a condition that is not a `Bool`"
    | .caseOption scrutinee noneE x someE => do
      match ← atomTy Γ scrutinee with
      | .option te ls =>
        match s with
        | .tuple ts _ =>
          need (inPlaceAll ts) "`ℓ_T` off a position of type `T`"
          let ε₁ ← checkExpr p E Γ (pc.join ls) (.tuple ts .bot) noneE
          let ε₂ ← checkExpr p E ((x, te.raise ls) :: Γ) (pc.join ls) (.tuple ts .bot) someE
          need (flowsB p.top (ts.map (LTy.raise ls)) ts)
            "a component of the joined state is below the scrutinee's label (U4001)"
          pure (ε₁.union ε₂)
        | _ =>
        need (inPlace s) "`ℓ_T` off a position of type `T`"
        let ε₁ ← checkExpr p E Γ (pc.join ls) s noneE
        let ε₂ ← checkExpr p E ((x, te.raise ls) :: Γ) (pc.join ls) s someE
        flowTo p (s.raise ls) s "the value of a `case`, raised by its scrutinee,"
        pure (ε₁.union ε₂)
      | _ => throw "a `case` on options of a value that is not an option"
    | .caseResult scrutinee x okE y errE => do
      match ← atomTy Γ scrutinee with
      | .result te ee ls =>
        match s with
        | .tuple ts _ =>
          need (inPlaceAll ts) "`ℓ_T` off a position of type `T`"
          let ε₁ ← checkExpr p E ((x, te.raise ls) :: Γ) (pc.join ls) (.tuple ts .bot) okE
          let ε₂ ← checkExpr p E ((y, ee.raise ls) :: Γ) (pc.join ls) (.tuple ts .bot) errE
          need (flowsB p.top (ts.map (LTy.raise ls)) ts)
            "a component of the joined state is below the scrutinee's label (U4001)"
          pure (ε₁.union ε₂)
        | _ =>
        need (inPlace s) "`ℓ_T` off a position of type `T`"
        let ε₁ ← checkExpr p E ((x, te.raise ls) :: Γ) (pc.join ls) s okE
        let ε₂ ← checkExpr p E ((y, ee.raise ls) :: Γ) (pc.join ls) s errE
        flowTo p (s.raise ls) s "the value of a `case`, raised by its scrutinee,"
        pure (ε₁.union ε₂)
      | _ => throw "a `case` on results of a value that is not a result"
    | .fail _ => do
      le p pc .bot "a failure under a program counter above `public` (U4002)"
      need (symFree pc) "a failure under a symbolic program counter"
      need (inPlace s) "`ℓ_T` off a position of type `T`"
      pure ⟨false, HostFx.none, some .bot, [], true, []⟩
    | .raise kind code args => do
      let sm ← some! (p.sum kind) s!"F1 restriction (no sum `{kind}`, a lookup miss)"
      let fs ← some! sm.variants[code]? s!"`{kind}` has no variant {code}"
      need (flowsB p.top (← atomsTy Γ args) fs)
        s!"a field of the failure `{kind}` does not flow into its declared type"
      le p pc .bot "a failure under a program counter above `public` (U4002)"
      need (symFree pc) "a failure under a symbolic program counter"
      need (inPlace s) "`ℓ_T` off a position of type `T`"
      pure ⟨false, HostFx.none, some .bot, [], false, [kind]⟩
    | .unreachable => do
      need (inPlace s) "`ℓ_T` off a position of type `T`"
      pure .none
    | .caseSum scrutinee ctor binders thenE elseE => do
      match ← atomTy Γ scrutinee with
      | .record kind ls =>
        let sm ← some! (p.sum kind) s!"F1 restriction (a `case` on variants of `{kind}`, which is not a sum: a lookup miss)"
        let fs ← some! sm.variants[ctor]? s!"`{kind}` has no variant {ctor}"
        need (fs.length == binders.length) s!"a pattern of `{kind}` with the wrong number of fields"
        let Γ' := binders.zip (fs.map (LTy.raise ls)) ++ Γ
        match s with
        | .tuple ts _ =>
          need (inPlaceAll ts) "`ℓ_T` off a position of type `T`"
          let ε₁ ← checkExpr p E Γ' (pc.join ls) (.tuple ts .bot) thenE
          let ε₂ ← checkExpr p E Γ (pc.join ls) (.tuple ts .bot) elseE
          need (flowsB p.top (ts.map (LTy.raise ls)) ts)
            "a component of the joined state is below the scrutinee's label (U4001)"
          pure (ε₁.union ε₂)
        | _ =>
        need (inPlace s) "`ℓ_T` off a position of type `T`"
        let ε₁ ← checkExpr p E Γ' (pc.join ls) s thenE
        let ε₂ ← checkExpr p E Γ (pc.join ls) s elseE
        flowTo p (s.raise ls) s "the value of a `case`, raised by its scrutinee,"
        pure (ε₁.union ε₂)
      | _ => throw "a `case` on variants of a value that is not of a sum"
    | .attempt body => do
      match s with
      | .result t e l =>
        let ε ← checkExpr p E Γ pc t body
        need ε.raises.isEmpty "an `attempt` of a code over a failure with a payload"
        le p ε.failLabel l "the result of `attempt` is below what decides the failure"
        need (flowB p.top (.int .bot) e) "the error of `attempt` is not a failure's code"
        pure ⟨ε.op, ε.host, Option.none, ε.rels, false, []⟩
      | _ => throw "`attempt` at a type that is not a result"
    | .attemptK kind body => do
      match s with
      | .result t e l =>
        let ε ← checkExpr p E Γ pc t body
        need (!ε.codes) s!"an `attempt` of `{kind}` over a failure with a code"
        need (ε.raises.all (· == kind)) s!"an `attempt` of `{kind}` over a failure of another kind"
        le p ε.failLabel l "the result of `attempt` is below what decides the failure"
        need (flowB p.top (.record kind .bot) e) s!"the error of `attempt` is not `{kind}`"
        pure ⟨ε.op, ε.host, Option.none, ε.rels, false, []⟩
      | _ => throw "`attempt` at a type that is not a result"
    | .handle body x handler => do
      match s with
      | .tuple ts _ =>
        need (inPlaceAll ts) "`ℓ_T` off a position of type `T`"
        let ε₁ ← checkExpr p E Γ pc (.tuple ts .bot) body
        need ε₁.raises.isEmpty "a handler of a code over a failure with a payload"
        let ε₂ ← checkExpr p E ((x, .int ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) (.tuple ts .bot) handler
        need (flowsB p.top (ts.map (LTy.raise ε₁.failLabel)) ts)
          "a component of the joined state is below what decides the failure (U4001)"
        pure ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
      | _ =>
      need (inPlace s) "`ℓ_T` off a position of type `T`"
      let ε₁ ← checkExpr p E Γ pc s body
      need ε₁.raises.isEmpty "a handler of a code over a failure with a payload"
      let ε₂ ← checkExpr p E ((x, .int ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) s handler
      flowTo p (s.raise ε₁.failLabel) s "the value of a handled expression, raised by its failure,"
      pure ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
    | .handleK kind body x handler => do
      match s with
      | .tuple ts _ =>
        need (inPlaceAll ts) "`ℓ_T` off a position of type `T`"
        let ε₁ ← checkExpr p E Γ pc (.tuple ts .bot) body
        need (!ε₁.codes) s!"a handler of `{kind}` over a failure with a code"
        need (ε₁.raises.all (· == kind)) s!"a handler of `{kind}` over a failure of another kind"
        let ε₂ ← checkExpr p E ((x, .record kind ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel)
          (.tuple ts .bot) handler
        need (flowsB p.top (ts.map (LTy.raise ε₁.failLabel)) ts)
          "a component of the joined state is below what decides the failure (U4001)"
        pure ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
      | _ =>
      need (inPlace s) "`ℓ_T` off a position of type `T`"
      let ε₁ ← checkExpr p E Γ pc s body
      need (!ε₁.codes) s!"a handler of `{kind}` over a failure with a code"
      need (ε₁.raises.all (· == kind)) s!"a handler of `{kind}` over a failure of another kind"
      let ε₂ ← checkExpr p E ((x, .record kind ε₁.failLabel) :: Γ) (pc.join ε₁.failLabel) s handler
      flowTo p (s.raise ε₁.failLabel) s "the value of a handled expression, raised by its failure,"
      pure ⟨ε₁.op || ε₂.op, ε₁.host.or ε₂.host, ε₂.fails, ε₁.rels ++ ε₂.rels, ε₂.codes, ε₂.raises⟩
    | .letTuple binders value body => do
      match ← atomTy Γ value with
      | .tuple ts l =>
        need (flowsB p.top (ts.map (LTy.raise l)) (binders.map Prod.snd))
          "a component of a tuple does not flow into its binder's type"
        need (binders.all (fun q => inPlace q.2)) "`ℓ_T` off a position of type `T`"
        checkExpr p E (binders ++ Γ) pc s body
      | _ => throw "the components of a value that is not a tuple"
    -- T-Loop: the declared state types are an invariant (ADR 0075), checked once.
    | .loop (.list xs) x xty state init body => do
      match ← atomTy Γ xs with
      | .list te ls =>
        flowTo p (te.raise ls) xty s!"an element of the list, as the loop variable `{x}`,"
        need (inPlace xty) "`ℓ_T` off a position of type `T`"
        need (flowsB p.top (← atomsTy Γ init) (state.map Prod.snd))
          "a loop's initial value does not flow into its state's type"
        need (state.all (fun q => inPlace q.2)) "`ℓ_T` off a position of type `T`"
        let ε ← checkExpr p E ((x, xty) :: (state ++ Γ)) (pc.join ls)
          (.tuple (state.map Prod.snd) .bot) body
        need (p.oldLoop || leB p.top ε.failLabel (pc.join ls))
          "an iteration's failure is decided above what decides whether it runs (U4002)"
        need (flowsB p.top ((state.map Prod.snd).map (LTy.raise ls)) (state.map Prod.snd))
          "a component of a loop's state is below what decides its iterations (U4001)"
        flowTo p (.tuple (state.map Prod.snd) .bot) s "the value of a loop"
        pure ε
      | _ => throw "a loop over a value that is not a list"
    | .loop (.range low high _) x xty state init body => do
      match ← atomTy Γ low, ← atomTy Γ high with
      | .int l₁, .int l₂ =>
        flowTo p (.int (l₁.join l₂)) xty s!"the range's bounds, as the loop variable `{x}`,"
        need (inPlace xty) "`ℓ_T` off a position of type `T`"
        need (flowsB p.top (← atomsTy Γ init) (state.map Prod.snd))
          "a loop's initial value does not flow into its state's type"
        need (state.all (fun q => inPlace q.2)) "`ℓ_T` off a position of type `T`"
        let ε ← checkExpr p E ((x, xty) :: (state ++ Γ)) (pc.join (l₁.join l₂))
          (.tuple (state.map Prod.snd) .bot) body
        need (p.oldLoop || leB p.top ε.failLabel (pc.join (l₁.join l₂)))
          "an iteration's failure is decided above what decides whether it runs (U4002)"
        need (flowsB p.top ((state.map Prod.snd).map (LTy.raise (l₁.join l₂))) (state.map Prod.snd))
          "a component of a loop's state is below what decides its iterations (U4001)"
        flowTo p (.tuple (state.map Prod.snd) .bot) s "the value of a loop"
        pure ε
      | _, _ => throw "a range whose bounds are not integers"
end

/-- T-Fun (`Program.FunOk`). -/
def checkFun (p : Program) (fn : Fun) : Result Unit := do
  let ε ← checkExpr p fn.eqParams fn.params .bot fn.result fn.body
  need (!ε.op || fn.op) s!"`{fn.name}` performs an operation it does not declare"
  need (hostLeB ε.host fn.host) s!"`{fn.name}` performs a host operation it does not declare"
  need (!ε.fails.isSome || fn.fails) s!"`{fn.name}` may fail, and does not declare it"
  le p ε.failLabel .bot s!"a failure of `{fn.name}` is decided above `public` (U4002)"
  need (ε.rels.all (fn.rels.contains ·)) s!"`{fn.name}` declassifies through a policy it does not declare"
  need (!ε.codes || fn.raises.isEmpty) s!"`{fn.name}` fails with a code, and declares a payload kind"
  need (ε.raises.all (fn.raises.contains ·)) s!"`{fn.name}` fails with a payload kind it does not declare"

/-- The parts of `Program.LWellFormed` about one function's signature. -/
def checkSignature (fn : Fun) : Result Unit := do
  need (fn.eqParams.all (fn.tparams.contains ·)) s!"`{fn.name}` constrains a type it does not have"
  need (fn.params.all (fun q => scopedIn fn.tparams q.2) && scopedIn fn.tparams fn.result)
    s!"the signature of `{fn.name}` mentions a type parameter it does not have"
  need (fn.params.all (fun q => inPlace q.2) && inPlace fn.result)
    s!"`ℓ_T` off a position of type `T` in the signature of `{fn.name}`"

mutual
  /-- `Rhs.Pure`. -/
  def rhsPure : Rhs → Bool
    | .atom _ | .prim _ _ | .construct _ _ | .field _ _ | .list _ | .none | .some _ | .first _ | .mkOk _
    | .mkErr _ | .eq _ _ | .variant _ _ _
    | .lambda _ _ | .funRef _ _ => true
    | _ => false

  /-- `Expr.Pure`. -/
  def exprPure : Expr → Bool
    | .atom _ => true
    | .letR _ _ v b => rhsPure v && exprPure b
    | .letE _ _ v b => exprPure v && exprPure b
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e
    | .caseSum _ _ _ t e | .handleK _ t _ e =>
      exprPure t && exprPure e
    | .attempt b | .attemptK _ b => exprPure b
    | .fail _ | .raise _ _ _ | .unreachable | .letTuple _ _ _ | .loop _ _ _ _ _ _ => false
end

/-- `Rhs.PureOrFails`. -/
def rhsPureOrFails : Rhs → Bool
  | .checked _ _ _ => true
  | r => rhsPure r

/-- `Expr.PureOrFails`. -/
def exprPureOrFails : Expr → Bool
  | .atom _ => true
  | .letR _ _ v b => rhsPureOrFails v && exprPureOrFails b
  | .letE _ _ v b => exprPureOrFails v && exprPureOrFails b
  | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e
  | .caseSum _ _ _ t e | .handleK _ t _ e =>
    exprPureOrFails t && exprPureOrFails e
  | .attempt b | .attemptK _ b => exprPureOrFails b
  | .fail _ => true
  | .raise _ _ _ | .unreachable | .letTuple _ _ _ | .loop _ _ _ _ _ _ => false

/-- The restrictions of option B1 on the policies, as one Boolean. -/
def b1Ok (p : Program) : Bool :=
  p.policies.all fun po => isScalar po.src && ground po.dst && isScalar po.dst &&
    match p.find po.via with
    | some fn => !fn.op && fn.host.isNone && exprPureOrFails fn.body && (fn.fails || exprPure fn.body) &&
        fn.raises.isEmpty
    | none => false

/-- `Program.B1WellFormed`: the restrictions of option B1 on the policies. A program outside them
is not re-checked, never rejected: `spec/07 §5` requires none of them. -/
def checkB1 (p : Program) : Result Unit := do
  for po in p.policies do
    need (isScalar po.src) s!"B1 restriction (non-scalar from): `{po.name}`"
    need (ground po.dst && isScalar po.dst) s!"B1 restriction (non-scalar to): `{po.name}`"
    match p.find po.via with
    | some fn =>
      need (!fn.op && fn.host.isNone && exprPureOrFails fn.body && (fn.fails || exprPure fn.body))
        s!"B1 restriction (impure via): `{po.via}`"
      need fn.raises.isEmpty s!"B1 restriction (via fails with a payload kind): `{po.via}`"
    | none => throw s!"no `via` function `{po.via}`"
  -- The same test as one Boolean, which the proof of soundness reads (`checkB1_sound`); the loop
  -- above gives each restriction its own message.
  need (b1Ok p) "B1 restriction"

/-- The parts of `Program.LWellFormed` about records and policies. -/
def checkDecls (p : Program) : Result Unit := do
  for rec in p.records do
    need (rec.fields.all ground) s!"a field of `{rec.name}` is not ground"
    need (rec.fields.all (fun t => leB p.top (p.deep t) (.ofLabel rec.deep)))
      s!"the deep label of `{rec.name}` is below a field's"
  for sm in p.sums do
    need (p.record sm.name).isNone s!"`{sm.name}` is both a record and a sum"
    need (sm.variants.all (·.all ground)) s!"a field of `{sm.name}` is not ground"
    need (sm.variants.all (·.all inPlace)) s!"`ℓ_T` in a field of `{sm.name}`"
    need (sm.variants.all (·.all (fun t => leB p.top (p.deep t) (.ofLabel sm.deep))))
      s!"the deep label of `{sm.name}` is below a field's"
  for po in p.policies do
    need (inPlace po.src && inPlace po.dst) s!"`ℓ_T` in the policy `{po.name}`"
    need (p.find po.via).isSome s!"no `via` function `{po.via}`"

end Una.F1.Check

namespace Una.F1.Check

mutual
  /-- `f` applied to every type a right-hand side writes: lambda parameters and the type
  arguments of calls. -/
  def Rhs.mapTypes (f : LTy → LTy) : Rhs → Rhs
    | .call g targs args => .call g (targs.map f) args
    | .funRef g targs => .funRef g (targs.map f)
    | .lambda params body => .lambda (params.map fun q => (q.1, f q.2)) (Expr.mapTypes f body)
    | r => r

  /-- `f` applied to every type a body writes: the types of its `let`s and what its right-hand
  sides write. -/
  def Expr.mapTypes (f : LTy → LTy) : Expr → Expr
    | .atom a => .atom a
    | .letR x t v b => .letR x (f t) (Rhs.mapTypes f v) (Expr.mapTypes f b)
    | .letE x t v b => .letE x (f t) (Expr.mapTypes f v) (Expr.mapTypes f b)
    | .ite c t e => .ite c (Expr.mapTypes f t) (Expr.mapTypes f e)
    | .caseOption s n x so => .caseOption s (Expr.mapTypes f n) x (Expr.mapTypes f so)
    | .caseResult s x o y e => .caseResult s x (Expr.mapTypes f o) y (Expr.mapTypes f e)
    | .fail c => .fail c
    | .raise k c args => .raise k c args
    | .caseSum s i xs t e => .caseSum s i xs (Expr.mapTypes f t) (Expr.mapTypes f e)
    | .unreachable => .unreachable
    | .attempt b => .attempt (Expr.mapTypes f b)
    | .attemptK k b => .attemptK k (Expr.mapTypes f b)
    | .handle b x h => .handle (Expr.mapTypes f b) x (Expr.mapTypes f h)
    | .handleK k b x h => .handleK k (Expr.mapTypes f b) x (Expr.mapTypes f h)
    | .letTuple bs v b => .letTuple (bs.map fun q => (q.1, f q.2)) v (Expr.mapTypes f b)
    | .loop src x xt st init b =>
      .loop src x (f xt) (st.map fun q => (q.1, f q.2)) init (Expr.mapTypes f b)
end

mutual
  /-- The calls and function references of a right-hand side with type arguments. -/
  def Rhs.genericCalls : Rhs → List (String × List LTy)
    | .call g targs _ | .funRef g targs => if targs.isEmpty then [] else [(g, targs)]
    | .lambda _ body => Expr.genericCalls body
    | _ => []

  /-- The calls and function references of a body with type arguments. -/
  def Expr.genericCalls : Expr → List (String × List LTy)
    | .letR _ _ v b => Rhs.genericCalls v ++ Expr.genericCalls b
    | .letE _ _ v b => Expr.genericCalls v ++ Expr.genericCalls b
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e
    | .caseSum _ _ _ t e | .handleK _ t _ e =>
      Expr.genericCalls t ++ Expr.genericCalls e
    | .attempt b | .attemptK _ b | .letTuple _ _ b | .loop _ _ _ _ _ b => Expr.genericCalls b
    | .atom _ | .fail _ | .raise _ _ _ | .unreachable => []
end

/-- The instance of a generic function at ground type arguments: its signature and body with
the type parameters instantiated (`LTy.lsubst`), and no type parameters. The instance runs as the
generic function does, since evaluation reads no type, so a program whose reachable instances
pass the checker is the generic-free program of its instances, which T6 covers. -/
def instantiate (subst : List (String × LTy) → LTy → LTy) (fn : Fun) (targs : List LTy)
    (name : String) : Fun :=
  let σ := fn.tparams.zip targs
  { name, tparams := [], eqParams := [],
    params := fn.params.map fun q => (q.1, subst σ q.2), result := subst σ fn.result,
    op := fn.op, host := fn.host, fails := fn.fails, body := Expr.mapTypes (subst σ) fn.body }

end Una.F1.Check
