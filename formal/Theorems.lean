/-
The theorems the `formal` gate checks for `sorry` and unexpected axioms (`scripts/formal.sh`):
`#print axioms` of each, whose output the script reads. A theorem of the track is listed here
when it is stated.
-/
import UnaFormal

#print axioms Una.Level.le_refl
#print axioms Una.Level.le_trans
#print axioms Una.Level.le_antisymm
#print axioms Una.Level.public_le
#print axioms Una.Level.le_join_left
#print axioms Una.Level.le_join_right
#print axioms Una.F1.sound
#print axioms Una.F1.t1
#print axioms Una.F1.Ty.subst_subst
#print axioms Una.F1.LTy.erase_lsubst
#print axioms Una.F1.RhsTy.subst
#print axioms Una.F1.ExprTy.subst
#print axioms Una.F1.Flow.erase
#print axioms Una.F1.LRhsTy.toStruct
#print axioms Una.F1.LExprTy.toStruct
#print axioms Una.F1.Program.LWellFormed.wellFormed
#print axioms Una.F1.t1_labeled
#print axioms Una.F1.LExpr.le_sound
#print axioms Una.F1.Rel.eq_of_deep
#print axioms Una.F1.derived_agrees
#print axioms Una.F1.top_label_not_enough
#print axioms Una.F1.Examples.pick_ok
#print axioms Una.F1.Examples.prog_wf
#print axioms Una.F1.Examples.callPick_typed
#print axioms Una.F1.Examples.callPick_safe
#print axioms Una.F1.Examples.choose_decision
#print axioms Una.F1.Examples.choose_ok
#print axioms Una.F1.Examples.eq_needs_Eq
#print axioms Una.F1.LExpr.interp_inst
#print axioms Una.F1.LTy.top_lsubst
#print axioms Una.F1.LTy.var_lsubst
#print axioms Una.F1.extend_deep
#print axioms Una.F1.LValTy.flow
#print axioms Una.F1.Flow.trans
#print axioms Una.F1.unary_sound
#print axioms Una.F1.V.eq_of_deep
#print axioms Una.F1.andThen_brel
#print axioms Una.F1.t6_sound
#print axioms Una.F1.t6
#print axioms Una.F1.eval_mono
#print axioms Una.F1.t6_ended
#print axioms Una.F1.T6Example.body_typed
#print axioms Una.F1.T6Example.body_noninterferent
