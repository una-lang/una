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
