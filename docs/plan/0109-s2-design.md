# ADR 0109 slice S2: the `overflow` obligation on `Int` in `systems` packages (design note)

- Issue: #31 (ADR 0109 rule 3, Implementation step 3; rule 2's parity; rule 8's complexity)
- Base: slice S1 (`dev/31-profiles`): the profile reaches the checker as `Env.profile`.
- Scope: no construct is added. A `systems` package gains one obligation per `Int` operation outside clauses; `app` packages are unchanged (rule 2.1).

## 1. What the tree has

- The checker records a pending `overflow` site for fixed-width arithmetic (`crates/una-check/src/types/fixed.rs`, `overflow_site`), gated off inside clauses and `reflect` bodies (`exact_integers()`), with no goal: only `(kind, span, origin)`.
- The verifier finds the same sites on its own from Core IR (`crates/una-verify/src/generate.rs`, `fixed_sites`), builds the exact term of the operation and the goal "exact result in the type's range", with a guard text for the repair, and reports U5001 with a counterexample or U5002 (`report.rs`). A CLI test (`verification.rs`, "the verifier generates every obligation the checker found") requires the two sides to agree on kind and position for every conformance package.
- The Core `Program` and `una_verify::obligations` know no profile. The Core export header gained `"profile"` in S1, from the manifest.

## 2. Decisions

**D1: the profile is a field of the Core program.** `una_core::Profile { App, Systems }` becomes the one type (`una-project` re-exports it, so S1's API is unchanged), and `Program` gains `profile`, set by elaboration from `Env.profile` and written to the Core export header from the program, not the manifest. The verifier reads `program.profile`; nothing else is passed. The formal track's re-check sees the profile where ADR 0109 rule 6 put it.

**D2: the checker's site.** `overflow_site` fires also when the magnitude is `Ty::Int` and `env.profile == Systems`, outside clauses and `reflect` bodies as today. That covers `+`, `-`, `*`, `/`, `mod`, unary `-` on `Int` and on `Int of U` (the magnitude is `Int`), and `convert()` on an `Int` magnitude with a ratio above 1 (rule 3). The callers of `overflow_site` for dense element-wise operations are **not** extended to `Int` elements in this slice: ADR 0120 rule 34 states the element obligation for fixed-width elements, and the `Int` case is left to a note in the PR and a follow-up issue, so the checker and the verifier stay in step here.
- **Literals.** An `Int` literal outside -2^62 to 2^62-1, outside clauses, in a `systems` package, records a site at the literal (a negated literal is one site, at the whole literal). The verifier emits the ground goal, refuted at once with the literal as its counterexample (rule 3: "refuted at once").
- **Prelude calls.** At a call of the prelude's `sum` (over `Int` or a fixed-width type) and `abs`, the checker records a site at the call (`call.rs`, beside `dense_result`); `to_int()` on `I64` and `U64` records a site at the method call (`fixed.rs`, the conversion branch). `to_int(t: Text)`, `parse_int`, JSON's `Number`, `Clock.now()` and `len` record none.

**D3: the verifier's site.** `fixed_sites` is factored over a pair of bounds; the fixed-width branch keeps its text, and a new branch for `Base::Int` under `program.profile == Systems` covers `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg` on a non-literal operand, and `Mul` by the ratio of `convert()` (which elaborates to a `Mul`), with the goal `exact in -4611686018427387904..4611686018427387903`. For the calls: `abs` gets the goal over its result binder under the callee's `ensures` (provable when the operand is bounded); `sum` gets the goal over its result binder with no further fact, so it is U5002 unless the program bounds it (the honest outcome: a sum of unbounded integers has no bound; the repair is a guard or a refinement); `to_int()` on a 64-bit value gets the goal over its argument. The report's repairs for an `Int` site are rule 3's: a guard, a refinement of the operands, a fixed-width type, or a wrapping or saturating method; the fixed-width texts stay for fixed-width sites. **The guard bounds the operands, never the result:** a test on the exact result (`if a + b in least..greatest`) is itself an `Int` operation with an obligation that cannot be proven in a `systems` package, so the proposed guard holds no arithmetic: for `+` and `-`, each operand in -2^61..2^61 - 1; for `*`, each operand in -(2^31 - 1)..2^31 - 1; for `/`, `mod`, unary `-` and `abs`, the operand `a` in -4611686018427387903..4611686018427387903 (an `Int` operand is unbounded to the verifier, so excluding the minimum alone proves nothing); for `convert()`, the magnitude in -(2^62 / k)..(2^62 - 1) / k with k the ratio; for `to_int()`, the value in least..greatest. Operands are written as they are, never with `.to_int()`.
- **Prelude functions as values.** `abs` or `sum` passed as a function value (`xs.map(abs)`) in a `systems` package, outside clauses, records an `overflow` site at the reference on both sides; the verifier reports it unknown with the reason that the function's operands have no bounds at that use (no query is made), and the repair is to write the call inside a lambda, whose operand's bounds then apply. Without this, a `systems` package could compute `abs(-2^62)` through `map` with no obligation.

**D4: parity.** The CLI parity test runs the new `systems` cases too; every site the checker records has its verifier obligation and conversely.

**D5: nothing else changes.** No run-time check, no codegen change: the promotion path stays in the executable (rule 3, "What the proofs buy"); the elision of proven checks is #737's. `ObligationKind::Overflow` keeps its name and its place in the listing (rule 7).

## 3. Conformance and tests (ADR 0109 "Tests", rule 3 and rule 2)

- `tests/conformance/verification/systems_int_overflow` (`ita.una`, edition 0.4, `profile systems`, no `run`): `a + b` on unbounded `Int` parameters refuted with a counterexample (U5001); the same guarded by `if a in 0..1000 and b in 0..1000` proven; a literal `4611686018427387904` refuted; unary `-` of a parameter refuted and of a literal in range accepted; `sum`, `to_int()` on `U64`, `abs`, `convert()` on an `Int of Meter` magnitude and `+` on `Int of Meter` each carrying their obligation (the expected statuses stated per site); a `requires` clause and a `reflect` body with the same arithmetic carrying none.
- `tests/conformance/verification/systems_int_overflow_app`: the same module as an `app` package, accepted with no obligation.
- `tests/conformance/verification/systems_int_overflow_0_3`: the refuted addition as a `una.toml` package of edition 0.3 with `profile = "systems"`.
- Rule 2 (CLI test, `crates/una-cli/tests/profiles.rs`): every conformance package of editions 0.3 and 0.4 under `tests/conformance/{verification,runtime}` re-checked with the profile forced to `systems` (and a budget added where `run` has an entry) keeps its outcome or gains only `overflow` diagnostics (U5001, U5002) and the budget error; and the runtime cases run under both profiles give the same traces. If this test is slow, it runs under the existing slow-test gate the native tests use and says so.
- Scaling (`crates/una-cli/tests/systems_scaling.rs`, the shape of `dense_scaling.rs`): a `systems` module with 10, 100 and 1,000 `Int` operations, each proven; the verification time per operation stays within a fixed ratio (growth bound 3 across the two largest sizes).
- Unit tests in `una-verify` for the `Int` goal text, the guard text and the repairs.

## 4. Complexity

One obligation per `Int` operation of a `systems` package, linear in the program's size, each a quantifier-free query of the size of a fixed-width `overflow` query (rule 8). The proof cache and the fallbacks of ADR 0053 and ADR 0054 apply unchanged. An `app` package pays nothing.

## 5. Spec and docs changed in this slice

- `spec/18 §3`: the rule 3 text for `Int` in a `systems` package (the obligation, the literal, the prelude functions, the entry points, the repairs), as ADR 0109 rule 3 states it.
- `spec/12 §3`: the `overflow` kind's sentence gains the `Int` case.
- `spec/13 §8`: the Core header's `profile` comes from the program.
- `docs/guide/configure/package/profile.md`: what `systems` adds, with one example.
- Changelog fragment `changelog.d/31-systems-overflow.md`.
