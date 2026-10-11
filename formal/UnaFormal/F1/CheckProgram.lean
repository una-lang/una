/-
M4 completion, part A2: **the whole-program check**. The re-check reads a document into one F1
program, `P`, with the real bodies of every declaration it accepts, every `via` function, all of
the document's policies, records and categories, one operation and the host's labels. `progOk`
tests every field of `Program.LWellFormed` and `Program.B1WellFormed` on `P` itself, as Booleans;
`CheckProgramSound.lean` proves that it implies them.

A function is tested in `P`, not in the per-declaration program with signature stubs that
`Import.checkWith` builds, so no transfer between the two programs is needed: the certificate is
taken on the program the theorem speaks of.
-/
import UnaFormal.F1.Check

namespace Una.F1

mutual
  /-- The functions a right-hand side names: its calls and its function values, lambdas
  included. The typing finds each in the program (`reach_closed`). -/
  def Rhs.callees : Rhs → List String
    | .call f _ _ | .funRef f _ | .invoke f _ => [f]
    | .lambda _ b => b.callees
    | _ => []

  def Expr.callees : Expr → List String
    | .atom _ | .fail _ | .raise _ _ _ | .unreachable => []
    | .withH _ _ b => b.callees
    | .letR _ _ v b => v.callees ++ b.callees
    | .letE _ _ v b => v.callees ++ b.callees
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e
    | .caseSum _ _ _ t e | .handleK _ t _ e =>
      t.callees ++ e.callees
    | .attempt b | .attemptK _ b | .letTuple _ _ b | .loop _ _ _ _ _ b => b.callees
end

end Una.F1

namespace Una.F1.Check

mutual
  /-- `Ty.Closed`: no type parameter. -/
  def tyClosed : Ty → Bool
    | .var _ => false
    | .int | .bool | .text | .unit | .record _ => true
    | .list t | .option t => tyClosed t
    | .result t e => tyClosed t && tyClosed e
    | .fn ps r => tysClosed ps && tyClosed r
    | .tuple ts => tysClosed ts

  def tysClosed : List Ty → Bool
    | [] => true
    | t :: ts => tyClosed t && tysClosed ts
end

/-- The categories of a label are among `cats`. -/
def labelCats (cats : List String) (l : Label) : Bool := l.cats.all (cats.contains ·)

mutual
  /-- `LTy.CatsIn`. -/
  def tyCats (cats : List String) : LTy → Bool
    | .int l | .bool l | .text l | .unit l | .record _ l | .var _ l => labelCats cats l.base
    | .list t l | .option t l => tyCats cats t && labelCats cats l.base
    | .result t e l => tyCats cats t && tyCats cats e && labelCats cats l.base
    | .fn ps r _ _ _ l => tysCats cats ps && tyCats cats r && labelCats cats l.base
    | .tuple ts l => tysCats cats ts && labelCats cats l.base

  def tysCats (cats : List String) : List LTy → Bool
    | [] => true
    | t :: ts => tyCats cats t && tysCats cats ts
end

/-- `Program.CatsDeclared`. -/
def catsDeclared (p : Program) : Bool :=
  labelCats p.categories p.acceptsOp && labelCats p.categories p.acceptsRead &&
    labelCats p.categories p.acceptsWrite && labelCats p.categories p.acceptsReq &&
    labelCats p.categories p.readResult && labelCats p.categories p.reqResult &&
    labelCats p.categories p.reqFloor && labelCats p.categories p.fileState &&
    p.funs.all (fun fn => fn.params.all (fun q => tyCats p.categories q.2) &&
      tyCats p.categories fn.result) &&
    p.records.all (fun rec => rec.fields.all (tyCats p.categories)) &&
    p.policies.all (fun po => tyCats p.categories po.src && tyCats p.categories po.dst)

/-- The parts of `Program.LWellFormed` about a record: ground, closed fields with `ℓ_T` in
place, below its declared deep label. -/
def recordOk (p : Program) (rec : Record) : Bool :=
  rec.fields.all fun t =>
    ground t && inPlace t && tyClosed t.erase && leB p.top (p.deep t) (.ofLabel rec.deep)

/-- The parts of `Program.LWellFormed` about a sum kind (#512): every variant's fields as a
record's, with the declared categories, and no record of the same name. -/
def sumOk (p : Program) (sm : Sum) : Bool :=
  (p.record sm.name).isNone &&
  sm.variants.all fun fs => fs.all fun t =>
    ground t && inPlace t && tyClosed t.erase && tyCats p.categories t &&
      leB p.top (p.deep t) (.ofLabel sm.deep)

/-- The parts of `Program.LWellFormed` about an operation with a handler (#524, Hu): a function of
the program, not generic, that releases nothing and performs no host operation, takes scalars, and
whose operations are at or above the operation's `accepts` (the effect floor). -/
def opOk (p : Program) (q : String × Label) : Bool :=
  match p.find q.1 with
  | some fn => fn.tparams.isEmpty && fn.rels.isEmpty && fn.host.isNone &&
      (!fn.op || leB p.top (.ofLabel q.2) (.ofLabel p.acceptsOp)) &&
      fn.params.all (fun r => isScalar r.2) && !isVia p q.1
  | none => false

/-- `Program.LWellFormed.handlers` (#538): each operation's representative is a handler of it,
and every handler of it takes what the representative takes, gives what it gives, fails as it
does, and accepts at least what it accepts. -/
def handlersOk (p : Program) : Bool :=
  p.opRep.all fun q =>
    List.lookup q.2 p.handlerOf == some q.1 &&
    match p.find q.2, List.lookup q.2 p.ops with
    | some fn, some acc => p.handlerOf.all fun g =>
        g.2 != q.1 ||
        match p.find g.1, List.lookup g.1 p.ops with
        | some gn, some acc' =>
          leB p.top (.ofLabel acc) (.ofLabel acc') &&
          flowsB p.top (fn.params.map Prod.snd) (gn.params.map Prod.snd) &&
          flowB p.top gn.result fn.result && gn.fails == fn.fails && gn.raises == fn.raises &&
          gn.tparams == fn.tparams && gn.op == fn.op
        | _, _ => false
    | _, _ => false

/-- The parts of `Program.LWellFormed` about a policy: closed types with `ℓ_T` in place, and a
`via` function whose structural signature is the policy's, `from` to `to`. -/
def policyOk (p : Program) (po : Policy) : Bool :=
  tyClosed po.src.erase && tyClosed po.dst.erase && inPlace po.src && inPlace po.dst &&
    match p.find po.via with
    | some fn => tysEq (fn.paramTys []) [po.src.erase] && tyEq fn.result.erase po.dst.erase
    | none => false

/-- `Program.HostWF`: ADR 0085's rules, the host's sinks at most `internal`, its results and the
floor of a request's failure at least `internal` (`internalLabel`, `⟨.internal, []⟩`). -/
def hostWfB (p : Program) : Bool :=
  !p.oldHost &&
  leB p.top (.ofLabel p.acceptsRead) (.ofLabel ⟨.internal, []⟩) &&
  leB p.top (.ofLabel p.acceptsWrite) (.ofLabel ⟨.internal, []⟩) &&
  leB p.top (.ofLabel p.acceptsReq) (.ofLabel ⟨.internal, []⟩) &&
  leB p.top (.ofLabel ⟨.internal, []⟩) (.ofLabel p.readResult) &&
  leB p.top (.ofLabel ⟨.internal, []⟩) (.ofLabel p.reqResult) &&
  leB p.top (.ofLabel ⟨.internal, []⟩) (.ofLabel p.reqFloor) &&
  -- The file-state label is capped at `internal` (ADR 0085 decision 1), which an exported value
  -- must respect (M5b).
  leB p.top (.ofLabel p.fileState) (.ofLabel ⟨.internal, []⟩) &&
  !p.oldLoop

/-- The parts of `Program.LWellFormed` and the premises of `t6_release_host` that are not about
one function: the host's labels (`Program.HostWF`), records, policies and categories. -/
def progDocOk (p : Program) : Bool :=
  hostWfB p &&
  p.records.all (recordOk p) &&
  p.sums.all (sumOk p) &&
  p.ops.all (opOk p) &&
  handlersOk p &&
  p.policies.all (policyOk p) &&
  catsDeclared p &&
  b1Ok p

/-- The type with its top label lowered to `⊥`: the same structure. -/
def _root_.Una.F1.LTy.lowerTop : LTy → LTy
  | .int _ => .int .bot
  | .bool _ => .bool .bot
  | .text _ => .text .bot
  | .unit _ => .unit .bot
  | .list t _ => .list t .bot
  | .option t _ => .option t .bot
  | .result t e _ => .result t e .bot
  | .record r _ => .record r .bot
  | .fn ps r o g f _ => .fn ps r o g f .bot
  | .var a _ => .var a .bot
  | .tuple ts _ => .tuple ts .bot

/-- The expression with the top label of every `let` annotation lowered to `⊥` (#510), the
same structure: a `via` body's temporaries, such as `let %t0: secret Bool = x > 3` before an `if`,
keep the labels of the parameters they are computed from, which the retry below lowers. Only
`let` annotations change; a right-hand side, a lambda included, is kept as it is. -/
def _root_.Una.F1.Expr.lowerLets : Expr → Expr
  | .letR x t v b => .letR x t.lowerTop v b.lowerLets
  | .letE x t v b => .letE x t.lowerTop v.lowerLets b.lowerLets
  | .ite c a b => .ite c a.lowerLets b.lowerLets
  | .caseOption sc n x sm => .caseOption sc n.lowerLets x sm.lowerLets
  | .caseResult sc x ok y er => .caseResult sc x ok.lowerLets y er.lowerLets
  | .attempt b => .attempt b.lowerLets
  | .handle b x h => .handle b.lowerLets x h.lowerLets
  | .letTuple bs v b => .letTuple bs v b.lowerLets
  | .loop src x xt st init b => .loop src x xt st init b.lowerLets
  | .caseSum sc i xs a b => .caseSum sc i xs a.lowerLets b.lowerLets
  | .attemptK k b => .attemptK k b.lowerLets
  | .handleK k b x h => .handleK k b.lowerLets x h.lowerLets
  | .atom a => .atom a
  | .fail c => .fail c
  | .raise k c args => .raise k c args
  | .unreachable => .unreachable
  | .withH o f b => .withH o f b.lowerLets

/-- A body the labeled checker types under these parameters, against the result raised to `⊤`. -/
def viaTypedAt (p : Program) (fn : Fun) (ps : List (String × LTy)) (body : Expr := fn.body) :
    Bool :=
  match checkExpr p fn.eqParams ps .bot (fn.result.raise (.ofLabel p.top)) body with
  | .ok _ => true
  | .error _ => false

/-- A `via` function's structural typing, by the labeled checker: its body checked against its
result raised to `⊤`, then erased (`LExprTy.toStruct`). No label is checked in a `via` function
(`spec/07 §5` rule 3), so when its parameters' labels stop the check, as T-Checked's premise
does on a `checked` of the `from` value (#453 Phase B), the body is checked again under the
parameters with their top labels at `⊥`, which have the same structure; and then with the top
labels of the body's `let` annotations at `⊥` too (#510), since a temporary computed from a
parameter keeps its label. -/
def viaTyped (p : Program) (fn : Fun) : Bool :=
  viaTypedAt p fn fn.params || viaTypedAt p fn (fn.params.map fun q => (q.1, q.2.lowerTop)) ||
    viaTypedAt p fn (fn.params.map fun q => (q.1, q.2.lowerTop)) fn.body.lowerLets

/-- T-Fun by the checker. -/
def funOk (p : Program) (fn : Fun) : Bool :=
  match checkFun p fn with
  | .ok _ => true
  | .error _ => false

/-- The parts of `Program.LWellFormed` about the function `fn` of `p`. A function named in
`cond` is not tested for T-Fun: it is checked at its instances, which part A3 covers, and the
soundness theorem takes its T-Fun as a named hypothesis. -/
def progFunOk (cond : List String) (p : Program) (fn : Fun) : Bool :=
  fn.eqParams.all (fn.tparams.contains ·) &&
  fn.params.all (fun q => scopedIn fn.tparams q.2) && scopedIn fn.tparams fn.result &&
  fn.params.all (fun q => inPlace q.2) && inPlace fn.result &&
  (if isVia p fn.name then viaTyped p fn else cond.contains fn.name || funOk p fn)

/-- **The whole-program check.** -/
def progOk (cond : List String) (p : Program) : Bool :=
  progDocOk p && p.funs.all (progFunOk cond p)

end Una.F1.Check
