/-
Trace safety of conditions on effects (ADR 0092, #561, `docs/plan/formal-track-trace-safety.md`).

A function's header may condition an operation of a binding, `journal where not dry_run`
(`Fun.conds`). The check here asks, at every site where the operation may be performed, that the
path knowledge `K` imply the condition: at the operation itself, and at a call, an invocation or a
reference to a named function, where the callee's own condition, instantiated at the arguments,
is also known. A lambda's body is checked under the knowledge of its definition. Implications are
decided by truth table over the condition's atoms, each a `Bool` variable or an opaque comparison
identified by its syntax (a ruling of the review of the formal track).

The theorem (`trace_safety`) says that a run of a checked function on arguments where a
condition is false never performs its operation, whatever the run's outcome.
-/
import UnaFormal.F1.Confine

namespace Una.F1

/-! ### Conditions and their values -/

/-- An atom of a condition, as the decision procedure sees it: a variable, or a comparison of
integers, identified by its syntax. -/
inductive CAtom where
  | var (x : String)
  | eq (a b : Atom)
  | lt (a b : Atom)
  deriving DecidableEq

/-- The value of a condition in an environment. Total: a variable that is not a `Bool`, and a
comparison of values that are not integers, are `false`, the same in the check and in the run. -/
def CAtom.eval (env : Env) : CAtom → Bool
  | .var x => match lookup env x with
    | some (.bool b) => b
    | _ => false
  | .eq a b => match evalAtom env a, evalAtom env b with
    | some (.int x), some (.int y) => x == y
    | _, _ => false
  | .lt a b => match evalAtom env a, evalAtom env b with
    | some (.int x), some (.int y) => decide (x < y)
    | _, _ => false

/-- A condition's value under an assignment of its atoms. -/
def Cond.under (σ : CAtom → Bool) : Cond → Bool
  | .tt => true
  | .ff => false
  | .var x => σ (.var x)
  | .not c => !c.under σ
  | .and a b => a.under σ && b.under σ
  | .or a b => a.under σ || b.under σ
  | .eqInt a b => σ (.eq a b)
  | .ltInt a b => σ (.lt a b)

/-- A condition's value in an environment: under the atoms' own values. -/
def Cond.eval (env : Env) (c : Cond) : Bool := c.under (CAtom.eval env)

/-- The atoms of a condition, without repetition. -/
def Cond.atoms : Cond → List CAtom
  | .tt | .ff => []
  | .var x => [.var x]
  | .not c => c.atoms
  | .and a b | .or a b => (a.atoms ++ b.atoms).eraseDups
  | .eqInt a b => [.eq a b]
  | .ltInt a b => [.lt a b]

/-! ### Implication, by truth table -/

/-- Every assignment of `atoms`, as lists of their values. -/
def assignments : List CAtom → List (List (CAtom × Bool))
  | [] => [[]]
  | a :: as => (assignments as).flatMap fun r => [(a, true) :: r, (a, false) :: r]

/-- The assignment a list gives, `false` elsewhere. -/
def asAssign (r : List (CAtom × Bool)) (a : CAtom) : Bool := (r.lookup a).getD false

/-- The most atoms the decision procedure tabulates (§4 of the plan, coverage only). -/
def maxAtoms : Nat := 12

/-- `hyp ⊨ goal`, by truth table over their atoms; `false` beyond `maxAtoms` atoms. -/
def implies (hyp goal : Cond) : Bool :=
  let atoms := (hyp.atoms ++ goal.atoms).eraseDups
  atoms.length ≤ maxAtoms &&
    (assignments atoms).all fun r => !(hyp.under (asAssign r)) || goal.under (asAssign r)

/-! ### Instantiation at a call -/

/-- An atom of the callee, at the call's arguments: a parameter becomes its argument, anything
else a value that is no integer, as it is in the callee's own environment. -/
def substAtom (θ : List (String × Atom)) : Atom → Atom
  | .var x => (θ.lookup x).getD .unit
  | a => a

/-- The callee's condition at the call's arguments, with the same value in the caller's
environment as the condition has in the callee's. -/
def Cond.subst (θ : List (String × Atom)) : Cond → Cond
  | .tt => .tt
  | .ff => .ff
  | .var x => match θ.lookup x with
    | some (.var y) => .var y
    | some (.bool b) => if b then .tt else .ff
    | _ => .ff
  | .not c => .not (c.subst θ)
  | .and a b => .and (a.subst θ) (b.subst θ)
  | .or a b => .or (a.subst θ) (b.subst θ)
  | .eqInt a b => .eqInt (substAtom θ a) (substAtom θ b)
  | .ltInt a b => .ltInt (substAtom θ a) (substAtom θ b)

/-! ### What a function may perform -/

/-- Two pairs of lists, joined. -/
def pairJoin (a b : List String × List String) : List String × List String :=
  (a.1 ++ b.1, a.2 ++ b.2)

mutual
  /-- The operations an expression performs directly, and the functions whose runs it starts:
  its calls, its invocations' handlers, and the bodies of the lambdas it defines. A call of a
  function value is not counted: its body is charged where it is defined. -/
  def Rhs.reach (p : Program) : Rhs → List String × List String
    | .op _ => (["Console.print"], [])
    | .read _ => (["Files.read"], [])
    | .hmac _ _ => (["Keys.hmac"], [])
    | .write _ _ => (["Files.write"], [])
    | .req m _ => ([s!"Http.{m}"], [])
    | .call g _ _ | .funRef g _ => ([], [g])
    | .invoke f _ => ([f], [f])
    | .invokeOp o _ => ([o], p.handlersOf o)
    | .lambda _ body => body.reach p
    | _ => ([], [])

  def Expr.reach (p : Program) : Expr → List String × List String
    | .atom _ | .fail _ | .raise _ _ _ | .unreachable => ([], [])
    | .letTuple _ _ b => b.reach p
    | .letR _ _ v b => pairJoin (v.reach p) (b.reach p)
    | .letE _ _ v b => pairJoin (v.reach p) (b.reach p)
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .caseSum _ _ _ t e =>
      pairJoin (t.reach p) (e.reach p)
    | .attempt b | .attemptK _ b | .loop _ _ _ _ _ b => b.reach p
    | .handle b _ h | .handleK _ b _ h => pairJoin (b.reach p) (h.reach p)
    | .withH _ f b => pairJoin ([], [f]) (b.reach p)
    | .exit _ _ => ([], [])
    | .loopUntil c _ _ b => pairJoin (c.reach p) (b.reach p)
    | .loopEver _ _ _ b _ o => pairJoin (b.reach p) (o.reach p)

end

/-- The operations a run of `f` may perform, transitively, after `fuel` rounds over the program's
functions; `p.funs.length` rounds reach every function. -/
def mayPerformN (p : Program) : Nat → String → List String
  | 0, _ => []
  | n + 1, f =>
    match p.find f with
    | none => []
    | some fn =>
      let (ops, starts) := fn.body.reach p
      ops ++ starts.flatMap (mayPerformN p n)

/-- `f` may perform `o`. -/
def mayPerform (p : Program) (f o : String) : Bool :=
  (mayPerformN p (p.funs.length + 1) f).contains o

/-- The operations some function of `p` conditions. -/
def Program.condOps (p : Program) : List String := p.funs.flatMap fun fn => fn.conds.map Prod.fst

/-- The functions `mayPerform` deems not to perform `o` are closed: none of them performs `o`
directly, and each starts only such functions. Checked, so that the proof needs no fixpoint. -/
def closedB (p : Program) (o : String) : Bool :=
  p.funs.all fun fn =>
    mayPerform p fn.name o ||
      (!(fn.body.reach p).1.contains o && (fn.body.reach p).2.all fun g => !mayPerform p g o)

/-- Code none of whose runs performs a conditioned operation of `p`, as a closure (stage 1 of
#561: a closure that may perform one is not re-checked). -/
def closureFree (p : Program) (body : Expr) : Bool :=
  p.condOps.all fun o => !(body.reach p).1.contains o && (body.reach p).2.all fun g => !mayPerform p g o

/-! ### Stage 1: closures -/

mutual
  /-- Every closure the code may build runs code free of the program's conditioned operations
  (stage 1 of #561). Code that fails it is not re-checked, never rejected. -/
  def Rhs.closuresOk (p : Program) : Rhs → Bool
    | .lambda _ body => closureFree p body && body.closuresOk p
    | .funRef g _ => p.condOps.all fun o => !mayPerform p g o
    | _ => true

  def Expr.closuresOk (p : Program) : Expr → Bool
    | .atom _ | .fail _ | .raise _ _ _ | .unreachable => true
    | .letR _ _ v b => v.closuresOk p && b.closuresOk p
    | .letE _ _ v b => v.closuresOk p && b.closuresOk p
    | .ite _ t e | .caseOption _ t _ e | .caseResult _ _ t _ e | .caseSum _ _ _ t e =>
      t.closuresOk p && e.closuresOk p
    | .attempt b | .attemptK _ b | .loop _ _ _ _ _ b | .letTuple _ _ b | .withH _ _ b => b.closuresOk p
    | .handle b _ h | .handleK _ b _ h => b.closuresOk p && h.closuresOk p
    | .exit _ _ => true
    | .loopUntil c _ _ b => c.closuresOk p && b.closuresOk p
    | .loopEver _ _ _ b _ o => b.closuresOk p && o.closuresOk p
end

/-- Stage 1 reads the program: its closures run no conditioned operation. -/
def Program.stage1B (p : Program) : Bool := p.funs.all (·.body.closuresOk p)

/-! ### The check -/

/-- The variables an atom of a condition names. -/
def CAtom.vars : CAtom → List String
  | .var x => [x]
  | .eq a b | .lt a b => [a, b].filterMap fun | .var x => some x | _ => none

/-- What a path knows (#573): literals over `Bool` variables, the branches taken, and the
comparisons that `let`s bound to some of those variables, so that a branch on such a variable
also learns its comparison. -/
structure Know where
  lits : List (String × Bool) := []
  defs : List (String × CAtom) := []

/-- `K` after a branch on `x` with value `b`. -/
def Know.lit (K : Know) (x : String) (b : Bool) : Know := { K with lits := (x, b) :: K.lits }

/-- `K` after `x` is bound to the value of the comparison `c`, which must not read `x`. -/
def Know.define (K : Know) (x : String) (c : CAtom) : Know :=
  if c.vars.contains x then K else { K with defs := (x, c) :: K.defs }

/-- The literal of an atom with value `b`, as a condition. -/
def CAtom.lit : CAtom → Bool → Cond
  | .var x, b => if b then .var x else .not (.var x)
  | .eq a c, b => if b then .eqInt a c else .not (.eqInt a c)
  | .lt a c, b => if b then .ltInt a c else .not (.ltInt a c)

/-- The conjunction of a list of conditions. -/
def Cond.conj (cs : List Cond) : Cond := cs.foldr .and .tt

/-- The knowledge of a path, as a condition: each branch literal, and, for a variable a `let`
bound to a comparison, the same literal of that comparison. -/
def knowledge (K : Know) : Cond :=
  Cond.conj (K.lits.map (fun q => CAtom.lit (.var q.1) q.2) ++
    K.defs.flatMap fun d => (K.lits.filter (·.1 == d.1)).map fun q => CAtom.lit d.2 q.2)

/-- The variables a condition names. -/
def Cond.vars (c : Cond) : List String := c.atoms.flatMap CAtom.vars

/-- A site performing `o` directly, under `K`: its condition, if any, is implied. -/
def siteOk (conds : List (String × Cond)) (K : Know) (o : String) : Bool :=
  match conds.lookup o with
  | some c => implies (knowledge K) c
  | none => true

/-- A site that starts a run of `g` on `args` (or, for a reference, on unknown arguments), under
`K`: for each conditioned operation `g` may perform, `K` and `g`'s own condition at the
arguments imply the caller's. -/
def startOk (p : Program) (conds : List (String × Cond)) (K : Know) (g : String)
    (args : Option (List Atom)) : Bool :=
  conds.all fun (o, c) =>
    !mayPerform p g o ||
      let c' := match p.find g, args with
        | some fn, some as =>
          if as.length = fn.params.length then
            ((fn.conds.lookup o).getD .tt).subst ((fn.params.map Prod.fst).zip as)
          else .ff
        | _, _ => .tt
      implies (.and (knowledge K) c') c

/-- A name bound here does not shadow a variable of a condition: the conditions read the
function's parameters, which keep their values for the whole call. -/
def freshFor (conds : List (String × Cond)) (x : String) : Bool :=
  !(conds.any fun (_, c) => c.vars.contains x)

/-- `K` after binding `x`: what it knew of an earlier `x` no longer holds, and neither does a
comparison that read it (#573). F1 changes a name only by binding it again: its environment
only grows, an `edit` call returns its new value, and a loop binds its names afresh. -/
def forget (K : Know) (x : String) : Know :=
  { lits := K.lits.filter (·.1 != x),
    defs := K.defs.filter fun d => d.1 != x && !d.2.vars.contains x }

/-- The comparison a right-hand side computes, if it is one of integers: `lt` or `eqInt` of two
atoms (#573). -/
def Rhs.compares (r : Rhs) : Option CAtom :=
  match r with
  -- `Rhs` has constructors `some` and `none`: the option's are named in full.
  | .prim Prim.lt [a, b] => Option.some (CAtom.lt a b)
  | .prim Prim.eqInt [a, b] => Option.some (CAtom.eq a b)
  | _ => Option.none

/-- `K` after `let x = v`: `x` is forgotten, then defined when `v` is a comparison. -/
def afterLet (K : Know) (x : String) (v : Rhs) : Know :=
  match v.compares with
  | some c => (forget K x).define x c
  | none => forget K x

mutual
  def Rhs.condOk (p : Program) (conds : List (String × Cond)) (K : Know) :
      Rhs → Bool
    | .op _ => siteOk conds K "Console.print"
    | .read _ => siteOk conds K "Files.read"
    | .hmac _ _ => siteOk conds K "Keys.hmac"
    | .write _ _ => siteOk conds K "Files.write"
    | .req m _ => siteOk conds K s!"Http.{m}"
    | .call g _ args => startOk p conds K g (some args)
    | .funRef g _ => p.condOps.all (fun o => !mayPerform p g o)
    | .invoke f args => siteOk conds K f && startOk p conds K f (some args)
    | .invokeOp o args =>
      siteOk conds K o && (p.handlersOf o).all
        (startOk p conds K · (some args))
    | .lambda _ body => closureFree p body
    | _ => true

  def Expr.condOk (p : Program) (conds : List (String × Cond)) (K : Know) :
      Expr → Bool
    | .atom _ | .fail _ | .raise _ _ _ | .unreachable => true
    | .letR x _ v b => freshFor conds x && v.condOk p conds K && b.condOk p conds (afterLet K x v)
    | .letE x _ v b => freshFor conds x && v.condOk p conds K && b.condOk p conds (forget K x)
    | .ite (.var x) t e => t.condOk p conds (K.lit x true) && e.condOk p conds (K.lit x false)
    | .ite _ t e => t.condOk p conds K && e.condOk p conds K
    | .caseOption _ t x e => freshFor conds x && t.condOk p conds K && e.condOk p conds (forget K x)
    | .caseResult _ x t y e =>
      freshFor conds x && freshFor conds y && t.condOk p conds (forget K x) &&
        e.condOk p conds (forget K y)
    | .caseSum _ _ bs t e =>
      bs.all (freshFor conds) && t.condOk p conds (bs.foldl forget K) && e.condOk p conds K
    | .attempt b | .attemptK _ b => b.condOk p conds K
    | .handle b x h | .handleK _ b x h =>
      freshFor conds x && b.condOk p conds K && h.condOk p conds (forget K x)
    | .withH o _ b => freshFor conds (markerName o) && b.condOk p conds (forget K (markerName o))
    | .letTuple bs _ b => bs.all (freshFor conds ·.1) && b.condOk p conds (bs.foldl (forget · ·.1) K)
    | .loop _ x _ st _ b =>
      freshFor conds x && st.all (freshFor conds ·.1) &&
        b.condOk p conds (st.foldl (forget · ·.1) (forget K x))
    | .exit _ _ => true
    -- ADR 0116's `until` and `ever` (#946): the condition and the body run with the state bound
    -- afresh, as a loop's body does, and `on stop` with its own binders of the state.
    | .loopUntil c st _ b =>
      st.all (freshFor conds ·.1) && c.condOk p conds (st.foldl (forget · ·.1) K) &&
        b.condOk p conds (st.foldl (forget · ·.1) K)
    | .loopEver _ st _ b os o =>
      st.all (freshFor conds ·.1) && os.all (freshFor conds ·.1) &&
        b.condOk p conds (st.foldl (forget · ·.1) K) && o.condOk p conds (os.foldl (forget · ·.1) K)
end

/-- `f` passes the condition check: its body, with its own conditions, from no knowledge. -/
def checkConds (p : Program) (f : Fun) : Bool := f.body.condOk p f.conds {}

/-- `p` passes trace safety's checks: every function's conditions, the closedness of the
functions that perform no conditioned operation, and the `via` functions of its policies, which
perform none. -/
def Program.traceSafeB (p : Program) : Bool :=
  p.funs.all (checkConds p) && p.condOps.all (closedB p) &&
    p.policies.all fun po => p.condOps.all fun o => !mayPerform p po.via o

/-! ### Performances -/

/-- The event `ev` is a performance of `o`: its ghost, or the operation's own event. -/
def PerfEv (o : String) : Event → Prop
  | .perf o' | .perfFn o' => o' = o
  | .out _ => o = "Console.print"
  | .read _ => o = "Files.read"
  | .write _ _ => o = "Files.write"
  | .req m _ => o = s!"Http.{m}"
  | .keyUse _ => o = "Keys.hmac"
  | .release _ _ => False

/-- `tr` performs `o`. -/
def Performs (o : String) (tr : Trace) : Prop := ∃ ev ∈ tr, PerfEv o ev

end Una.F1
