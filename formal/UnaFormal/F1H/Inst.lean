/-
M3's first item (the owner's condition 3, 2026-10-09): instantiation commutes with the semantic
reading of labels. Reading an instantiated label under `ρ` is reading the label itself under
`ρ` extended by the instantiation, each covered `T` read as `ℓ*` of its type argument. So the
checker's instantiation at a call (`LExpr.inst`, `LTy.lsubst`) is the reading's, and a proof
about a generic body under the reading holds at every call.

Labels hold categories as lists, so two readings are compared up to the order (`Label.Equiv`).
-/
import UnaFormal.F1H.Spike
import UnaFormal.F1.Inst

namespace Una.F1H

open Una.F1 (Label LExpr)



/-- At a position that is not a type parameter, labeled instantiation instantiates the label. -/
theorem LTy.top_lsubst (rd : String → Label) (σ : List (String × LTy)) :
    ∀ t : LTy, (∀ a l, t ≠ .var a l) →
      (t.lsubst rd σ).top = t.top.inst (σ.map (fun q => (q.1, q.2.deep rd)))
  | .int _, _ | .bool _, _ | .text _, _ | .unit _, _ | .list _ _, _ | .option _ _, _
  | .result _ _, _ | .record _ _, _ | .fn _ _ _ _ _ _, _ => by simp [LTy.lsubst, LTy.top]
  | .var a l, h => absurd rfl (h a l)

/-- At a position of type `T` that `σ` covers, labeled instantiation puts `T`'s type argument,
with its own labels in place, raised by the position's other labels instantiated. -/
theorem LTy.var_lsubst (rd : String → Label) (σ : List (String × LTy)) {a : String} {u : LTy}
    (l : LExpr) (h : List.lookup a σ = some u) :
    (LTy.var a l).lsubst rd σ = u.raise ((l.dropPlain a).inst (σ.map (fun q => (q.1, q.2.deep rd)))) := by
  simp [LTy.lsubst, h]

/-- So the reading of an instantiated label takes each covered `T` as `ℓ*` of its type
argument: the extension of `ρ` by the instantiation of a call reads `T` as the reading of
`deep (σ T)`. -/
theorem extend_deep (rd : String → Label) (ρ : String → Label) (σ : List (String × LTy))
    {a : String} {u : LTy} (h : List.lookup a σ = some u) :
    LExpr.extend ρ (σ.map (fun q => (q.1, q.2.deep rd))) a = (u.deep rd).interp ρ := by
  simp [LExpr.extend, lookup_map, h]

end Una.F1H
