# Draft: ADR 0116, Amendment (2026-11-07): the twins of the two `Λ` cases of section H

- Status: Draft, for the project to accept. Until it is accepted, the accepted text of ADR 0116 is unchanged, and this note records which twins the conformance suite holds.
- Issue: #1087, slice 4b (PR #1713), from its review.

**The conflict.** Section H of the amendment of 2026-10-25 names these accepted twins:
- `tconc_function_value_public`: "the same lambda and `apply`, beside `crunch_open(n)`";
- `tconc_uses_of_public`: "beside `crunch_open(n)`".

Section F of the same amendment refuses them:
- item 1: a construct that calls any function value makes every lambda and every function named as a value run beside it;
- item 7: a part that calls a function value has `τ⁰ = ⊤`, so its construct gives `⊤` to every site beside it.

So the `within` that the function value reaches has `τ_conc = ⊤` whatever the label of the sibling's data. No twin that differs only in that label can be accepted.

**What the suite holds.** The twins are `tconc_function_value_after` and `tconc_uses_of_after`. They make the same call through `apply` or `run_with` after the `together`, outside every part, as section H's own twin of `tconc_map_in_part` moves its `map` out of every part. Each is accepted; each refused case is refused.

**The amendment proposed.** Append to ADR 0116, under the amendment of 2026-10-25, H:

> The accepted twins of `tconc_function_value` and `tconc_uses_of` are `tconc_function_value_after` and `tconc_uses_of_after`: the same call, after the `together`. A twin beside `crunch_open(n)` is refused by F items 1 and 7, since a part that calls a function value has `κ = ⊤`. A later refinement of `κ` for a function value whose code is known MAY make the twins differ in the label alone.
