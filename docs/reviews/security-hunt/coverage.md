# Security hunt: what checks and covers each rule of spec/07

- Issue: #513, part (d) of `docs/plan/security-hunt.md`
- Date: 2026-10-10, on main 286a7676 (the corpus counts are from main 56ce83ed's export)
- Updated: 2026-10-11, on main b674910d, after #508, #509, #510, #512, #526, #532 and #514; the corpus counts in the rows are from #537's export of `main` cee87e3e (1,352 targets, 4,331 declarations, 1,800 of them in documents the export marks complete). Section 3 lists what changed.

For every rule of `spec/07` §1 to §7, this table says where `una check` enforces it, how many conformance cases exercise it, which part of the formal model (F1, `formal/UnaFormal/F1/`) states it, and whether the re-check's importer reads it. Its status is:

- **covered**: the model states the rule, and the importer reads the construct, so the re-check can disagree with the checker on it;
- **partial**: the model states part of it, or only for some shapes;
- **blind**: nothing in the model states it, so no theorem speaks for it. The checker alone stands behind it, with its tests.

The hunt's two-run oracle (`docs/plan/security-hunt-oracle.md`) is the only differential check of the blind rows, and they are its first targets.

The checker sites are under `crates/una-check/src/types/` unless another path is given. The model sites are in `formal/UnaFormal/F1/`. The test counts come from the case names and `-- test:` titles under `tests/conformance/`, since most cases say only `spec/07 §7`, so they are approximate and overlap.

## 1. The rules

### §1. The guarantee and the observable channels

| Channel | Model | Status |
|---|---|---|
| Return values | `Eval.lean` `Res`, `Val`; `T6.lean` `t6_sound` | covered |
| Failures: which variant | `Res.failed`, `Val.err`: a failure is a code | covered |
| Failures: payloads | `Rhs.variant`, `Expr.raise`, `attemptK`, `handleK`, `caseSum`; the runtime shows only the fields whose deep label is public (#512) | partial: a payload of a kind that is not a sum, a handler over failures of several kinds, a handler of a failure code that reads the failure, `==` on a sum, a function value that fails with a payload, and a policy whose `via` function does are not re-checked |
| Arguments of capability operations | `Event` `out`, `read`, `write`, `req`, `release`; `Unary.lean` `Allowed` | partial: one abstract operation of one capability, `Files`, `Http`, `Env.get`; not `Keys.hmac`, `Clock.now`, a second operation, or a user capability |
| The occurrence of effects | the same events | partial, as above |
| Out of the model (timing, termination, aborts, races, hard links, other writers, task order) | the model is termination-insensitive; M5 and M6 state the host premises (`formal-track-m5.md`, `formal-track-m6.md` §8) | consistent with `spec/07 §1` |

### §2 to §4

| Rule | Checker | Tests | Model | Importer | Status |
|---|---|---|---|---|---|
| §2 lattice, five levels, categories | `una-core/src/label.rs` | 2 (U4007 only); the lattice is used everywhere | `Labels.lean` `Label`, `LExpr` | yes | covered |
| §3.1 explicit flows, fields | `expr.rs::binary`, `call.rs::field`, `ty.rs::deep_label` | ~60 | `LRhsTy.prim` with `primLabel`, `field` | yes for `add`, `sub`, `mul`, `div`, `mod`, `neg`, `lt`, `gt`, `le`, `ge`, `eq`, `ne`, `not`, `len`, `concat`, `to_text` (#526) on the operand types F1 models; an ordering of `Text` or of variants is an F1 restriction, not re-checked (#532); **not** `fixed` (38 declarations in the corpus) or `convert` (6) | partial |
| §3.2 the pc, raised by branches | `flow.rs::raise_pc`; `stmt.rs` if, when, if-none | ~16 | `ite`, `caseOption`, `caseResult`, loops | yes for `Option`, `Result` and user sum types without type parameters (#512), whose `case` the importer checks against the declared variants | covered for kinds without type parameters |
| §3.3 failures only at a public pc (U4002) | `flow.rs::fail_under_pc`, `stmt.rs::handled` | ~12 | `LExprTy.fail`, `handle`, `Program.Bound` | yes, payloads included (#512) | covered, as §1 |
| §3.4 effects under the pc, `bound(f)`, T-App | `flow.rs::sink`, `call_under_pc`, `bound_of`; `call.rs::call_value` | ~30 | `call`, `apply`, `op`, `Program.Bound` | yes | covered (within one operation) |
| §3.5 returns (U4001) | `expr.rs::returned_value`, `implicit_return`, `tail_expr` | ~40 | `Program.FunOk` | yes | covered |
| §4 sinks | `flow.rs::sink` from `call.rs::cap_op` | ~64 | `op`, `read`, `write`, `req` | the operations above | partial (as §1) |
| §4 interpolation | `expr.rs` (interpolation) | ~10 | `concat`, `toText` | yes | covered |
| §4 `expect` lines exempt | the `Ctx.labels` flag | 1 (`ifc_tour`, whose `expect` line reads no labeled value) | none: examples are not in the model | no | exemption: no case where an `expect` line reads a labeled value (#516) |

### §5. Policies

| Rule | Checker | Tests | Model | Importer | Status |
|---|---|---|---|---|---|
| 1. callable with the `via` function's `uses` and `fails` | `call.rs::call_policy` | in 13 | `declassify`, `Eff.ofPolicy` | yes | covered, within B1 |
| 2. argument into `from`; `pc ⊑ ℓ(To)` | `call.rs`, `flow.rs::declassify_under_pc` | ~3 | `declassify`: `Flow src`, `pc ⊑ po.dst.top`; `Program.RelBound` | yes | covered, within B1 |
| 3. `via` private; no labels checked in it; refinement obligations stay | `check.rs::policy`, `flow.rs::via_site`; labels off in `via` bodies | 19 | `Program.IsVia`; `via` bodies typed structurally; obligations are T5's | the body is "a `via` function, which no label rule checks" | covered as a trust boundary |
| 4. types match but for labels; no `requires` | `check.rs::policy` | 6 | `LWellFormed.policies` (structure only) | partial | partial (refinements are the verifier's) |
| 5. the report's Information section | `una-cli/src/report.rs` | none in conformance (Rust tests in `una-cli/tests`) | none | no | outside the information-flow model |
| 6. a policy call's failure is `pc ⊔ ℓ(To)` (S-18) | `call.rs` | 6 | `Eff.ofPolicy` (#453 Phase B) | yes | covered |
| B1, the model's restriction | | | `B1.lean`: a scalar `from` and `to`, a pure or failing `via` | a policy outside B1 is not re-checked (3 of 120 in the corpus have a non-scalar `to`, 4 an impure `via`) | **blind** outside B1 |
| `via` bodies that branch on a comparison | | | the retry of `viaTyped` | the top labels of the body's `let` annotations are lowered too (#510): no `via` function in the corpus is left without a structural typing | covered |

### §6. Trusted modules

| Rule | Checker | Tests | Model | Status |
|---|---|---|---|---|
| U7005, header and `una.toml` agree | `names/mod.rs` | 9 | none: the model has no modules or trust | **blind** |
| U4004, policies only in trusted modules | `check.rs::policy` | in the 9 | none | **blind** |

The model assumes every policy is lawful. That a policy can exist only in a trusted, listed module is the checker's alone.

### §7. Checking

| Rule | Checker | Tests | Model | Importer | Status |
|---|---|---|---|---|---|
| 1. structural labels | `unify.rs`, `ty.rs` | implicit | `LTy` | yes | covered |
| 2. flows: covariance, contravariance (S-Fun) | `flow.rs::below` | ~55 files; most hold accepted declarations beside the rejected ones | `Flow`, `Flows` | yes | covered |
| 2. flows: invariant kind parameters (ADR 0046), recursive kinds | `ty.rs::is_invariant`, `call.rs::expected_invariant` | 4 | none: user kinds have no type parameters in F1 | "the type `q` with type arguments" (45 declarations) | **blind**; the hunt's family G and the composed PG and GS check it with `una check` alone |
| 2. a lambda's parameter that rises after its body | `flow.rs`, `call.rs` | few | none: T-Fun is per function | no | **blind** |
| 2. `Unit` never fails a flow; one U4001 per construct in a return | `flow.rs` | not obvious | `unit` flows; sites not modeled | n/a | covered for `Unit` |
| 3. labels of values: deep labels of kinds | `ty.rs::deep_label` | ~56 files; most hold accepted declarations beside the rejected ones | `Record.deep`, `Sum`'s variants, `primLabel` | user records (#509) and sums (#512) without type parameters, when they do not reach themselves | partial: kinds with type parameters and recursive kinds are not re-checked |
| 3. `.len`, `is`, `in` | `expr.rs::compare_with_is` | few | `len` only | `is` and `in` not as primitives | partial |
| 4. operations: `ℓ⁺`, declared parameters as ⊤, inline lambdas | `flow.rs::sink`, `op_argument_flow` | ~14 | `op` with scalar arguments only (`deep` in place of `ℓ⁺`) | one operation, scalar to `Unit` | partial |
| 4. the effect floor of handler bodies | `check.rs::handler` | 14 | none: handler bodies are outside the reach set | "a `handler` declaration" (84 in the corpus) | **blind** |
| 4. `Keys.hmac` (ADR 0039) | `env.rs::capability_signature`, `call.rs::cap_op` | 6 | none (`formal-track-m5.md`: not part of M5) | "the operation `Keys.hmac`, which does not take one argument" | **blind** |
| 4. failures of host operations by their deciders (N-39) | `call.rs::structural_inputs` | 8 | `read`, `write`, `req` | yes | covered |
| 4. `Http` (S-15) | `call.rs` | 3 (+ `http_failures`) | `req`, `Program.reqFail` | yes | covered |
| 4. the file-state label (S-14, ADR 0085) | `mod.rs::file_state`, `flow.rs::entry`, `call.rs::cap_op` | 28 | `Program.fsFail`, `write`'s premise | yes, re-checked from the export | covered; the "inside a lambda is `internal`" and "an example is `public`" cases have no test |
| 5. policy placement | as §5, §6 | | | | as §6 |
| 6. exemptions: `expect`, `via` bodies, clauses; `do` lines at pc ⊥ | the `Ctx.labels` flag | only `via_exemption_body_only` | `via` bodies only | n/a | no information-flow case found for `expect`, `do` at pc ⊥, or `requires`/`ensures` |
| 7. categories (U4007) | `env.rs::lower_label` | 2 | `CatsIn`, `CatsDeclared` (declared labels only) | yes | partial (annotations in bodies are the checker's) |
| 8. type arguments carry labels | `generic.rs`, `unify.rs` | 173 (10 ok) | `lsubst`, ground instances | yes, one level, by instances | covered |
| 8. symbolic labels `ℓ_T`, `ℓ*_T` | `flow.rs::below`, `check.rs::symbolic` | in the 173 | `LExpr` `plain`, `star` | yes | covered |
| 8. `ℓ_T` only in place (#440) | by construction | none in conformance (Rust: `core_symbols.rs`) | `LTy.InPlace` | yes | covered in the model; no conformance case |
| 8. comparisons on constrained parameters, `sort` (ADR 0066) | `call.rs` | `constraint_labels` (tagged `spec/20 §4`) and `g67_l53` | none | "the constraint `ord`" | **blind** |
| 8. result summaries (ADR 0067) | `summary.rs` (SCC fixpoint) | 6 + chains | read from the export as facts that may only raise a signature; the body, or each instance, is checked against the raised result, so a summary that is too small is a rejection (`formal-track-m2.md`, confirmed by the adversarial review of #454) | positions `element`, `arg 1`, `result` only | covered at those positions |
| 8. edition 0.1's defense in depth | `call.rs` (T-App), `decided_callbacks` | 17 | the model follows edition 0.2 (`addStarAll` in the importer) | partial | partial |
| 8. no effects or failures under a symbolic pc | `flow.rs` | in the 173 | `SymFree` premises | yes | covered |
| 8. prelude results (ADR 0051) | `call.rs` | several | `map`, `filter`, `first_where`, `first` (#532), `fold`, `parse`, `append`, `set`, `sum` | every other prelude function is "a call of `prelude.x`" (51 declarations; `or_else`, `min`, `max`, `is_ok`, `abs` and `slice` are 31 of them) | partial |
| 9. `of` clauses: the callee, the call, the prelude's higher-order functions | `effects.rs` | ~225 | `LTy.fn` flags, `lambda`, `map`, `filter`, `fold`, `firstWhere` | yes; a non-empty `of_raises` is not | partial |
| 10. sites | `flow.rs` | Rust tests only | not modeled | n/a | diagnostics, not security |

### Constructs outside every rule's model

The model has none of these, so every rule is blind on them:

- `together` and `together_for` (28 declarations);
- `with`, handlers and capability parameters (18 and 86);
- `Map` (17), `Json` (6), fixed-width integers, `fixed` (38);
- kinds with type parameters other than `Option` and `Result` (45), and recursive kinds;
- two operations of one capability in a document ("several operations", 7).

Refinement predicates (113 `pred` declarations) are outside the flow model by design (`spec/07 §7` rule 6), and the re-check reports them in their own category, "outside the flow model" (#532), apart from these.

## 2. What it means for the hunt

1. **Blind rows are the oracle's first targets**, since only a run can check them. In this order (the coordinator, 2026-10-10):
   - **trusted modules and policy shapes first.** The model assumes every policy is lawful, so whether a policy can exist, where it is declared, how its `via` function is reached, and policies outside B1 rest on the checker alone. Shapes include:
     - a policy in a module that is trusted by its header but not listed in `una.toml`, or the other way around;
     - a `via` function reached other than through its policy;
     - a policy whose `to` is not scalar;
     - a `via` function with effects;
   - handler bodies and the effect floor;
   - `Keys.hmac`;
   - kinds with type parameters and invariance;
   - failure payloads;
   - `together`;
   - user sum types;
   - `Ord`-constrained comparisons and `sort`.
   S-14 to S-18 were each in a row the model had not reached yet, or had just reached.
2. **The checker–model comparison (part (b)) is strongest where the rows are covered:**
   - pc and failures;
   - returns;
   - `Files`, `Http` and the file-state label;
   - policies within B1;
   - generics with symbolic labels;
   - the `of` rules for the prelude's four higher-order functions.
   Its mutators should stay inside the importer's vocabulary so that their programs are re-checked, not set aside.
3. **Cheap model gains.** Done for the arithmetic primitives (#526), `first` (#532), user records (#509) and sum kinds (#512). The next ones by corpus count: `handler` declarations (86), `fixed` (38), the forms `with`, `together` and `together_for` (46), kinds with type parameters (45), and the prelude functions `or_else`, `min`, `max`, `is_ok`, `abs` and `slice` (31).
4. **Test gaps in the checker's own suite** (#516, closed by #518):
   - the exemptions of rule 6: an `expect` line that reads a labeled value, `do` lines at pc ⊥, and `requires`, `ensures` and `where` clauses;
   - the file-state label for a write inside a lambda, and for one reached only from an example;
   - that `sort` raises the result list itself, not only its elements. `constraint_labels` covers the elements;
   - accepted declarations in the 33 of the 111 `sweep_variance` and `sweep_label` files that hold only rejected ones. The other 78 already hold both, since a case's expected errors are a set, so a declaration without one is an accepted case.
5. **The summaries of ADR 0067 are checked, not trusted.** The re-check reads them from the export, but only to raise a result type, and then checks each body or instance against it. So a summary that is too small is rejected, never accepted. The gap there is coverage: a summary at a position other than `element`, `arg 1` or `result` is not re-checked.

## 3. Changes since 2026-10-10

| Row | Was | Now | By |
|---|---|---|---|
| §1 failures with a payload | blind | partial | #512 |
| §3.1 explicit flows | 9 primitives | 16 primitives; other operand types an F1 restriction | #526, #532 |
| §3.2 the pc, `case` on user sums | partial | covered for kinds without type parameters | #512 |
| §3.3 failures at a public pc | partial (payloads) | covered | #512 |
| §5 `via` bodies that branch on a comparison | partial | covered | #510 |
| §7.3 deep labels of kinds | partial (records not read) | partial (records and sums read; kinds with type parameters not) | #509, #512 |
| §7.8 prelude results | 8 functions | 9 functions, with `first` | #532 |
| `Result`'s error type | dropped | kept, `Err(a)` labeled at `E` | #508 |
| `pred` declarations | not re-checked | outside the flow model, counted apart | #532 |
| the reverse comparison | none | `una core --with-rejected` and `una-core-check --rejected` | #514 |

The rows that stay **blind**, the hunt's generators' targets, have each been run with `una check` alone against v0.3.7, with no finding:
- trusted modules and policies outside B1: family T;
- handler bodies and the effect floor: H;
- `Keys.hmac`: K;
- kinds with type parameters and invariance: G, and the composed PG and GS;
- `together`: C, and the composed CS, TC and CP;
- `Ord`-constrained comparisons and `sort`: O.

The breadth runs, on seeds 6 and 11, together cover about 12,000 programs. The two-run oracle (`docs/plan/security-hunt-oracle.md`) remains the only differential check of these rows.

## 4. Sources

- Checker sites: `crates/una-check/src/types/` (`flow.rs`, `call.rs`, `expr.rs`, `stmt.rs`, `check.rs`, `effects.rs`, `summary.rs`, `env.rs`, `mod.rs`), `crates/una-core/src/label.rs`, `crates/una-check/src/names/mod.rs`, `crates/una-cli/src/report.rs`.
- Model sites: `LTyping.lean`, `Syntax.lean`, `Eval.lean`, `Unary.lean`, `B1.lean`, `Import.lean` (its reasons for "not re-checked"), and the out-of-model sections of `docs/plan/formal-track*.md`.
- Corpus counts: the re-check of main 56ce83ed's export, 4,238 declarations in 1,344 targets.
