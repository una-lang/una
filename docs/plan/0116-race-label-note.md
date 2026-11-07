# Draft: ADR 0116, Amendment (2026-11-07): the timing label of a race, with IF5 item 4 as refined

- Status: Draft, for the project to accept. Until it is accepted, the accepted text of ADR 0116 is unchanged, and this note records how `spec/27 §6` rule 8 reads it.
- Issue: #1087, slice 4a (PR #1712), from its review.

**The texts that meet.**
- The amendment of 2026-10-25, C, says that each branch `b` of a race `F` has `τ_conc(b) = τ_conc(F) ⊔ κ(F)`, so that `t = κ(F) ⊔ τ_conc(F)`.
- P2 defines `κ(F)` as "`τ⁰` of the whole construct: IF5 items 1 to 5".
- F item 7 says that IF5 item 4 is unchanged, so that a part that calls `map` has `τ⁰ = ⊤` even with a lambda written at the call, and that "a separate issue can label a lambda written at the call by its captures".
- That issue, #1023 (merged 2026-11-02), refined IF5 item 4 (`spec/26 §7` rule 5 item 4): a lambda written at the call counts by its captures and its body.

**The reading.** `t` of a race is `τ⁰` of the race's own code, every branch included, by IF5 as it stands, #1023 included, joined with `τ_conc` of the race at its own site and with `L_res`. That is C's `κ(F) ⊔ τ_conc(F)`, with `κ(F)` taken by P2 as `τ⁰` of the construct under the current IF5.

The `κ` that `ctx` and `conc` carry (`spec/27 §11` rules 3 and 4) stays the coarser one of F item 7, `⊤` for a part that calls a prelude function with a function argument. The sites inside the branches take it, and so does every site that a part reaches.

**Why.**
- **Soundness.** The race's outcome depends on the time its branches take, which `τ⁰` of their code bounds (IF5, #1023's argument), and on the work that runs around the race, which `τ_conc(F)` bounds.
- **Why a coarse `κ` exists at all.** It covers the graph of P1, whose one node `Λ` for all lambdas cannot tell lambdas apart beside a part. The race's own seed holds its lambdas' code directly.
- **The literal reading is stricter.** Reading `κ(F)` as the coarse label would refuse `security/first_of_timing_lambda` (the `calm` race over `map(fn(x) -> x + n)` on `public` data), which the preview accepts. F forbids that except as a fix of soundness, and it is not one.

**The amendment proposed.** Append to ADR 0116, under the amendment of 2026-10-25, C:

> `κ(F)` in `t` is `τ⁰` of the race's code by IF5 as it stands, with item 4 as #1023 refined it; the coarser `κ` of F item 7 is the one `ctx` and `conc` carry to the sites inside and beside the parts.
