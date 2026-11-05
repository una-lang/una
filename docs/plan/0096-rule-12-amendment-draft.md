# ADR 0096 amendment draft: the complexity of rule 12

Status: Proposed (draft for the project's decision; it is appended to docs/adr/0096-exact-rewrites-and-declaration-recovery.md only once accepted, with the date of its acceptance). From #1573.

## Amendment (proposed 2026-11-05): rule 12 is O(n log n)

Rule 12 says that a check of a file with broken declarations "stays linear in the source". The implementation is O(n log n): finding the declaration that holds a byte offset, and the line that holds one, is a binary search over starts that increase strictly, done once per declaration of the tree and once per syntax error; the renames of names that start with `_` look names up in a set of the file's words. Until #1573 these were scans, quadratic in a file with many errors, which the rule's own scaling test caught under load.

Proposed text for the last two items of rule 12: "So a check stays within O(n log n) in the source and linear in the dependency graph, where `n` counts the declarations, lines and syntax errors of a file. A scaling test checks files of 500, 1,000, 2,000 and 4,000 declarations, every fourth one broken, and one with an error in every declaration, and fails when the time of a doubling grows by more than 2.5 times."

Choice. A linear pass is possible: the tree's declarations are visited in the order of the text, so a cursor could replace the binary search. It would rely on that order for correctness, and the logarithmic factor is below the noise of the 2.5x bound at every size the tests measure. The project may prefer that strictly linear form instead; the tests would not change.
