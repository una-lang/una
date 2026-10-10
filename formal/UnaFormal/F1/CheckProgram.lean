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
    | .call f _ _ | .funRef f _ => [f]
    | .lambda _ b => b.callees
    | _ => []

  def Expr.callees : Expr → List String
    | .atom _ | .fail _ => []
    | .letR _ _ v b => v.callees ++ b.callees
    | .letE _ _ v b => v.callees ++ b.callees
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .handle t _ e =>
      t.callees ++ e.callees
    | .attempt b => b.callees
end

end Una.F1

namespace Una.F1.Check

mutual
  /-- Equality of types, as a Boolean. -/
  def tyEq : Ty → Ty → Bool
    | .int, .int | .bool, .bool | .text, .text | .unit, .unit => true
    | .list a, .list b | .option a, .option b | .result a, .result b => tyEq a b
    | .record a, .record b | .var a, .var b => a == b
    | .fn ps r, .fn qs s => tysEq ps qs && tyEq r s
    | _, _ => false

  def tysEq : List Ty → List Ty → Bool
    | [], [] => true
    | a :: as, b :: bs => tyEq a b && tysEq as bs
    | _, _ => false
end

mutual
  /-- `Ty.Closed`: no type parameter. -/
  def tyClosed : Ty → Bool
    | .var _ => false
    | .int | .bool | .text | .unit | .record _ => true
    | .list t | .option t | .result t => tyClosed t
    | .fn ps r => tysClosed ps && tyClosed r

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
    | .list t l | .option t l | .result t l => tyCats cats t && labelCats cats l.base
    | .fn ps r _ _ _ l => tysCats cats ps && tyCats cats r && labelCats cats l.base

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
  leB p.top (.ofLabel ⟨.internal, []⟩) (.ofLabel p.reqFloor)

/-- The parts of `Program.LWellFormed` and the premises of `t6_release_host` that are not about
one function: the host's labels (`Program.HostWF`), records, policies and categories. -/
def progDocOk (p : Program) : Bool :=
  hostWfB p &&
  p.records.all (recordOk p) &&
  p.policies.all (policyOk p) &&
  catsDeclared p &&
  b1Ok p

/-- A `via` function's structural typing, by the labeled checker: its body checked against its
result raised to `⊤`, then erased (`LExprTy.toStruct`). -/
def viaTyped (p : Program) (fn : Fun) : Bool :=
  match checkExpr p fn.eqParams fn.params .bot (fn.result.raise (.ofLabel p.top)) fn.body with
  | .ok _ => true
  | .error _ => false

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
