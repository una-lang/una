/-
Tests of the instance checking of the re-check (`Import.lean`), checked when the library builds:
labeled instantiation (`LTy.lsubst`) on the symbolic labels `ℓ_T` and `ℓ*_T`, alone and through
two levels of instances, and two mutants of it, which `ReplayMutants.lean` runs on the replay.
-/
import UnaFormal.F1.Check

namespace Una.F1.CheckTests

open Check

def lab (l : Level) (cats : List String := []) : LExpr := .ofLabel ⟨l, cats⟩
def sym (plain star : List String) : LExpr := ⟨Label.bot, plain, star⟩
def rd0 : String → Label := fun _ => Label.bot

/-- `List[internal Int]`, public at the top. -/
def listInternal : LTy := .list (.int (lab .internal)) LExpr.bot

-- `ℓ_T` in place reads as the type argument's own labels: `label(T) T` at `T := internal Int`.
#guard fmtTy ((LTy.var "T" (sym ["T"] [])).lsubst rd0 [("T", .int (lab .internal))]) ==
  "internal Int"

-- `ℓ*_T` reads as every label of the type argument: `label(T*) T` at `T := List[internal Int]`
-- is the list raised by `internal`, its element's label.
#guard fmtTy ((LTy.var "T" (sym ["T"] ["T"])).lsubst rd0 [("T", listInternal)]) ==
  "internal List[internal Int]"

-- A decision `ℓ*_T` off a position of type `T`: `label(T*) Bool`.
#guard fmtTy ((LTy.bool (sym [] ["T"])).lsubst rd0 [("T", listInternal)]) == "internal Bool"

-- Two levels: `g[T]` calls `h[List[internal T]]`, and `h`'s parameter `label(U) + label(U*) U`
-- is instantiated at `g`'s instance `T := Int`. The type argument of `h` at `g[Int]` is
-- `List[internal Int]` (the composition the worklist computes), so the parameter is that list
-- raised by `internal`.
def hTarg : LTy := .list (.var "T" (lab .internal)) LExpr.bot
#guard fmtTy (hTarg.lsubst rd0 [("T", .int LExpr.bot)]) == "public List[internal Int]"
#guard fmtTy ((LTy.var "U" (sym ["U"] ["U"])).lsubst rd0
    [("U", hTarg.lsubst rd0 [("T", .int LExpr.bot)])]) == "internal List[internal Int]"

mutual
  /-- `LTy.lsubst` with `raise` and `img` as parameters: the type argument at a position of type
  `T` is raised by the position's other labels when `raise` holds, and a symbolic label reads as
  `img` of the type argument. `LTy.lsubst rd` is `lsubstWith true (·.deep rd) rd`. -/
  def lsubstWith (raise : Bool) (img : LTy → LExpr) (σ : List (String × LTy)) : LTy → LTy
    | .int l => .int (l.inst (σ.map fun q => (q.1, img q.2)))
    | .bool l => .bool (l.inst (σ.map fun q => (q.1, img q.2)))
    | .text l => .text (l.inst (σ.map fun q => (q.1, img q.2)))
    | .unit l => .unit (l.inst (σ.map fun q => (q.1, img q.2)))
    | .list t l => .list (lsubstWith raise img σ t) (l.inst (σ.map fun q => (q.1, img q.2)))
    | .option t l => .option (lsubstWith raise img σ t) (l.inst (σ.map fun q => (q.1, img q.2)))
    | .result t l => .result (lsubstWith raise img σ t) (l.inst (σ.map fun q => (q.1, img q.2)))
    | .record r l => .record r (l.inst (σ.map fun q => (q.1, img q.2)))
    | .fn ps r o g f l =>
      .fn (lsubstsWith raise img σ ps) (lsubstWith raise img σ r) o g f
        (l.inst (σ.map fun q => (q.1, img q.2)))
    | .var a l =>
      match List.lookup a σ with
      | some t => if raise then t.raise ((l.dropPlain a).inst (σ.map fun q => (q.1, img q.2))) else t
      | none => .var a (l.inst (σ.map fun q => (q.1, img q.2)))

  def lsubstsWith (raise : Bool) (img : LTy → LExpr) (σ : List (String × LTy)) :
      List LTy → List LTy
    | [] => []
    | t :: ts => lsubstWith raise img σ t :: lsubstsWith raise img σ ts
end

/-- **Mutant 1, the label component**: the type argument at a position of type `T` is not raised
by the position's labels, so a decision `ℓ*_T` on a value of `T` is dropped. -/
def mutantLabel (rd : String → Label) (σ : List (String × LTy)) (t : LTy) : LTy :=
  lsubstWith false (·.deep rd) σ t

/-- **Mutant 2, the deep component**: `ℓ*_T` reads as the type argument's top label, not every
label of it, so the label of a list's elements is lost. -/
def mutantDeep (rd : String → Label) (σ : List (String × LTy)) (t : LTy) : LTy :=
  lsubstWith true LTy.top σ t

-- The parameterized form with the right parameters agrees with `LTy.lsubst` on the cases above.
#guard fmtTy (lsubstWith true (·.deep rd0) [("T", listInternal)] (.var "T" (sym ["T"] ["T"]))) ==
  fmtTy ((LTy.var "T" (sym ["T"] ["T"])).lsubst rd0 [("T", listInternal)])
#guard fmtTy (lsubstWith true (·.deep rd0) [("T", listInternal)] (.bool (sym [] ["T"]))) ==
  fmtTy ((LTy.bool (sym [] ["T"])).lsubst rd0 [("T", listInternal)])

-- Each mutant differs from `LTy.lsubst` on one of them.
#guard fmtTy (mutantLabel rd0 [("T", listInternal)] (.var "T" (sym ["T"] ["T"]))) ==
  "public List[internal Int]"
#guard fmtTy (mutantDeep rd0 [("T", listInternal)] (.var "T" (sym ["T"] ["T"]))) ==
  "public List[internal Int]"

end Una.F1.CheckTests
