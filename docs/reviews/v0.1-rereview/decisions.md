# Decisions of the v0.1 closure

In scope:
- D-133 to D-159, in `docs/history/v0.1/open-decisions.md`, "Decisions of the v0.1 closure run";
- ADRs 0039 to 0045;
- the "Needs review" items R-1, R-3 and R-5 of fix request 0001;
- the four findings of the first review that the closure deferred: R-023, R-025, R-028 and R-031.

The criteria are those of the first review. In order:
1. the spec's text, then its intent;
2. the charter principles and the proof boundary;
3. strictness;
4. consistency with accepted ADRs and the owner's answers;
5. simplicity;
6. agent ergonomics.

Each decision was matched against its `DECISION` tags and tests, usually with a CLI probe. The owner answered D-136 and D-148 in commit `3a5f4e9` during the review; those rows evaluate the owner's answer.

## Statistics

| Item | Keep | Switch | Escalate |
| --- | --- | --- | --- |
| D-133 to D-159 (27) | 26 | 1 (D-144) | 0 |
| ADRs 0039 to 0043 (Accepted) | 5 | 0 | 0 |
| ADRs 0044 and 0045 (Proposed) | 0 | 0 | 2 (accept 0044; amend and accept 0045) |
| R-1, R-3, R-5 | 3 (R-3 and R-5 each add one spec sentence) | 0 | 0 |
| Deferred R-023, R-025, R-028, R-031 | 4 (stay deferred) | 0 | 0 |

Two decisions are sound rules with broken implementations:
- **D-137:** see N-003 (Critical).
- **D-157:** see N-016 (Minor).

D-144 should switch because a release build silently drops refined function kinds (N-006, reproduced with a release build of `85787d7`). ADR 0045 is the matching Proposed rule.

## Table

"A reviewer probe" means a scratch program whose shape is in the rationale. The programs worth keeping are in `adversarial/`.

| ID | Recommendation | Rationale | Evidence | Refactoring cost |
| --- | --- | --- | --- | --- |
| D-133 | Keep | Acting on the grant directory itself as a file is never meaningful. Denying it keeps ADR 0038 D9 rule 3 total, and it is the stricter option. | `host.rs:181`; CLI test `files_grants_never_follow_a_final_link_and_keep_directories_inside_r011` (`crates/una-cli/tests/runtime.rs:453`) covers `.` and `sub/..` | none |
| D-134 | Keep | Stricter than D9 rule 2, and it stops probing for directories outside the grant. Rule 1 already removes `..` lexically, so the only new case is a link that leaves the prefix and a link that comes back. | `host.rs:185-199`. No test covers the out-and-back case (N-022). | none |
| D-135 | Keep | U1009 rejects `module prelude`, and tier 1 runs only after tier 0 is clean, so no program that reaches the verifier can spell that path. `prelude_kind` also requires that the package does not declare the kind. Option B costs more and guarantees nothing more. | `generate.rs:257,397,541`. A reviewer probe: U1009 at 1:1. | none |
| D-136 | Keep (as amended by the owner in `3a5f4e9`) | Accepting labeled arguments is sound because the result joins their labels. Keeping S-Fun on function-typed parameters is required, since a lambda can capture labels that the arguments do not show. At `4ea31cf`, the exception for `is_some`, `is_ok` and `append` was strict but contradicted ADR 0038 D3.4. The owner has since removed it. | `call.rs:1245-1272`. A reviewer probe: `fold`, `or_else`, `starts_with`, `at` with a secret index and `.len` all raise the result (U4001 on each leak). a reviewer probe shows the old exception. | done (`3a5f4e9`) |
| D-137 | Keep the rule; fix the implementation (blocks the tag, N-003) | Raising every data position by the join of all type-argument labels is sound and simple, and option B can refine it later without rejecting more programs. The lazy path for function values is broken. | `expr.rs:325-333`, `unify.rs:70`. Direct calls are raised (reviewer probe); a function value bound with `let` is not (reviewer probe). | Rule: none. Fix: medium (`expr.rs`, `unify.rs`) |
| D-138 | Keep | One diagnostic per instantiation is enough to repair it. Skipping sites already reported avoids two codes at one span. | `generic.rs:61`. A reviewer probe: one U3016 at 18:16. | none |
| D-139 | Keep | A stored function whose parameter or result is second-class lets the effectful value escape. The stricter option. | `ty.rs:287`. a reviewer probe: U3016 at 25:66. | none |
| D-140 | Keep | `[note]` stores `note` exactly as `List[...]` does. In practice U3006 fires first at each element and D-138 suppresses the U3016, so this is defense in depth that no test covers (N-022). | `expr.rs:488`; a reviewer probe line 17, a reviewer probe | none |
| D-141 | Keep (the entry text is stale) | After ADR 0039, `hmac` accepts `public` with `data` carried, and `accepts` defaults to `public`. The `⊤` floor now applies only to an unknown operation, which is already an error. Harmless, but the premise "accepts every label" is false (N-023). | `check.rs:957-962`, `env.rs:690-708`; `security/hmac_handler_worst_caller.una` | text only |
| D-142 | Keep | One edit repairs it. | `env.rs:1069`. a reviewer probe: one U3016 at 7:12. | none |
| D-143 | Keep | Each code names the construct to change. | `authority.rs:1388`. a reviewer probe: `fs: List[fn ...]` gives U3016 only, and field `g` gives U3006 only. | none |
| D-144 | Switch to B (every build) | `spec/10 §2` states the check with no build restriction: "a component that was lost is U8001, never a silent pass". The spec outranks the ADR's "debug-build" wording. Release binaries are what ship, and today they silently drop the refinement of a refined function kind (N-006). The check runs inline in `Fx::ty` and `refined_kept`, not as a second walk, so it costs little. Because this edits the wording of the owner's ADR 0038 D1, it is also listed under escalations. | `elab.rs:249` and `elab.rs:334` are gated by `cfg!(debug_assertions)`. | Low: remove two guards; reword ADR 0038 D1 |
| D-145 | Keep | The annotation is what the agent edits. | `expr.rs:937`. a reviewer probe: U4001 at the annotation (23:52). | none |
| D-146 | Keep | The literal rule (`bound(f) = ⊥` for a failing `f`) would reject fake stores that fail, which ADR 0038 D4 keeps valid, and the owner said failures follow data flow. `spec/07 §7` rule 4 now states the rule. No leak found through a failure, a callee or a result. | `check.rs:983`, `flow.rs:244-270,355-365`. a reviewer probe: each leak is U4001 or U4002; the fixed-value handler is OK. | none |
| D-147 | Keep | Follows the by-kind paragraph of `spec/13 §3` (ADR 0033). It is conservative: it hides some valid repairs (a `let` local gets no `requires`) but never proposes one that cannot apply. | `place()` in `generate.rs:2486`, `repairs()` in `report.rs:95`; CLI tests `repairs-*` in `crates/una-cli/tests/verification.rs:513-640`; a reviewer probe | none |
| D-148 | Keep | No release or tag exists, and the change only adds fields. At `4ea31cf` the spec preamble literally required a version bump, which was a text mismatch. The owner's `3a5f4e9` answers Keep and adds "Schema versions freeze at the `v0.1.0` tag". The tag must be cut from a commit that contains `3a5f4e9`. | `spec/13` preamble and §6; `git show 3a5f4e9 -- docs/spec/13-diagnostics-and-abi.md` | none |
| D-149 | Keep | Option B contradicts `spec/03 §4`, which makes opaque kinds and operation `ensures` the carriers of opaque facts. Naming a fact does not establish it. | `call.rs:411`, `check.rs:654,884,895`. a reviewer probe: a handler that returns `"x"` for `ensures valid_slug(result)` gets U5001. | none |
| D-150 | Keep | The variance is correct: parameters take the union of the refinements, the result takes their intersection, and the rule flips for nested parameters. Sound in both branch orders. | `ty.rs:86` (`combine`/`mix`), `unify.rs:292-318`; `verification/fn_type_join_not_refined.una`; a reviewer probe give U5001 | none |
| D-151 | Keep | The result is the value's own declaration. Naming the expected type instead would suggest weakening the callee's contract. | `generate.rs:1820`; a reviewer probe | none |
| D-152 | Keep | Checking each path gives the precise failing path and a counterexample, and the guard repair applies. | `generate.rs:1957`; a reviewer probe | none |
| D-153 | Keep | Follows the wording of ADR 0037 rule 2. The syntactic "carried" test can only add obligations, never skip one. | `generate.rs:1743`; `fn_type_same_refinement_proven.una`, `fn_type_parameter_refuted.una` | none |
| D-154 | Keep | Nested positions get a name the agent can locate. It changes goal text only. | `generate.rs:1745`; `fn_value_precondition_goal_text.una` | none |
| D-155 | Keep | A mechanical edit would have to regroup sibling arms, and `spec/13 §3` allows a repair that is help without an edit. | `parser.rs:1655`; `syntax/nested_pattern.una` | none |
| D-156 | Keep | Gives one code per problem; U2015 already covers the rest. | `authority.rs:866`; `authority/effect_in_predicate_line.una` | none |
| D-157 | Keep (finish the U2009 half) | The advice gives the agent a useful next step, but only U2005 lists the open names; U2009 gives no help in that case (N-016). | `call.rs:943-948` against `call.rs:1117-1127`; a reviewer probe | Minor: one branch |
| D-158 | Keep | Names the construct once, as `CLAUDE.md` asks. | `names/mod.rs:393`; a reviewer probe | none |
| D-159 | Keep | The omitted line is a mismatch in which obligations the checker and the verifier record at `-k` divisors, not a soundness issue: the original program checks, proves and runs correctly. It waits for R-031's line in `spec/12 §2`, and the backlog row records it. | `runtime/euclidean_division_signs.una`; `docs/plan/backlog.md:43` | none |
| ADR 0039 | Keep (matches E4 A) | The result is `internal ⊔ ℓ*(data)` and `key_name` is `public`. Rule 3 applies ADR 0038 D4 to `Keys` handlers. | `flow-hmac-without-policy` is now U4001; `security/hmac_secret_key_name.una`, `hmac_under_secret_pc.una`, `hmac_handler_worst_caller.una`; `spec/14 §3`, `spec/07 §7` rule 4 updated | none |
| ADR 0040 | Keep (matches E5 A) | U2002 fits "ill-formed type". An inferred argument takes the base type, so no fact is assumed without proof. | a reviewer probe: `Option[Pos]`, `List[Int in 0..5]`, nested, user `Box[Pos]`, aliases and refined list kinds each give U2002 once. a reviewer probe: `id(p)` loses `Pos` (U5001). | none |
| ADR 0041 | Keep (matches E6 A) | The cache is in charter §3, `spec/12 §9` and `spec/13 §6`, and the report counts cached results. The Consequences sentence that keeps the schema name goes beyond E6 A and is decided by D-148 (N-023). | a reviewer probe: first report "0 cached", second "11 cached"; JSON `obligations.cached`, `tcb.proof_cache`; backlog row for cache authentication | none |
| ADR 0042 | Keep (matches E8 A, plus D-149) | U2016 applies in code, function clauses and examples. The kind `where` and operation-clause exception is required by `spec/03 §4` and is decided by D-149 (N-023). | a reviewer probe: U2016 at function `requires`, code, `ensures` and example `do`; kind `where` accepted; registry row and `una explain U2016` present | none |
| ADR 0043 | Keep (matches E9 A) | Rules 1 to 3 implement E9 A. I found no bypass through a helper function or a delegating handler. ADR 0028 is not marked as amended (N-023). | `escape-testing-via-handler`: U3015 at 15:14 and 26:26; `authority/testing_*.una` | none |
| ADR 0044 | Accept (does not block the tag) | `spec/10 §12` rule 1 already makes the lookup static. A `do` with no error kind finds nothing, which is what U1001 means, and U1001 is already the code for a variant outside the `do`'s kind. Stricter, cheap and reversible. | a reviewer probe: `do safe(1)` then `expect fails Bad` passes `una check` (exit 0) and fails only in `una test`. a reviewer probe: the R-020 fix gives U1001. | Low: the lookup in `una-check`, one case, one spec line |
| ADR 0045 | Amend, then Accept, and implement before the tag (blocks, N-006) | ADR 0037 rule 10 (Accepted) says such a refinement is never dropped. Today release drops it silently, and debug raises an internal error on a user program. U2002 is strict and consistent with ADR 0040. Amend in three ways: (a) remove "or range sugar", because `fn(Int) -> Int in 0..5` refines the result and is supported and checked; (b) report an alias `kind G = F` once, at `F`'s clause; (c) use a message that names the kind. | a reviewer probe: U8001 at 7:1, exit 3 (reproduced). a reviewer probe: range sugar on the result gives U5001 as intended (reproduced). a reviewer probe: an alias gives U8001 twice. | Low: kind well-formedness, one case, `spec/03 §3` |
| R-1 | Keep as stated (does not block) | `spec/13 §5` and ADR 0036 rule 3 already say it: the status lives in the command's document, which is the ABI agents read. Adding diagnostics would duplicate that data and change the ABI. | `spec/13 §5`; ADR 0036 rule 3 | none |
| R-3 | State the order in the spec: count first (U2001), then labels (U2004 to U2006). Keep the code (does not block). | This matches the compiler and the guide, and gives one code per problem: fixing the count leads to the label error next. | a reviewer probe: `f(1, b: 2, b: 3)` gives U2001 only. a reviewer probe (three parameters) gives U2006 only. | One sentence in `spec/10 §12` rule 13 |
| R-5 | List `--solver-wallclock` in the `spec/13 §5` rows of `check`, `run`, `test`, `obligations`, `trace` and `report` (does not block) | The flag is already normative in `spec/12 §5`, so the command table should be complete for agents. | `--help` of those six commands shows it; `contract` and `fmt` correctly do not | One spec edit |
| R-023 | Keep deferred (v0.1.x; does not block) | It is reachable only in example `do` lines. The unformatted program is already U7001, so no accepted program changes meaning; the defect is that the U7001 repair (and `una fmt`) does not preserve behavior. | a reviewer probe: `expect fails First` passes as written and fails with `Second` after `una fmt` (reproduced) | Low |
| R-025 | Keep deferred (v0.1.x; does not block) | Example data come only from source literals (U3014 keeps host capabilities out of examples), so nothing is revealed that is not already in the source. The output is still inconsistent. | a reviewer probe: `result.value = "\"PIN-1234\""` while the trace shows `<redacted:secret>` | Low |
| R-028 | Keep deferred (v0.2; does not block) | Spec and code agree: `spec/05 §9` rule 5 and ADR 0028 choose U3002 on purpose. This is a consistency wish, not a defect. | `spec/05 §9` rule 5 | Medium (ADR amendment) |
| R-031 | Keep deferred (v0.1.x; does not block) | Sound: the extra counterexamples are an accepted incompleteness of the uninterpreted operators, and the fix needs a line in `spec/12 §2`. | a reviewer probe: `x * -2` is refuted with `x = 0, result = -1` | Low |

## Amendments recommended for ADR 0045

1. Remove "or range sugar" from rule 1: in `kind F = fn(Int) -> Int in 0..5` the range binds to the result, which is already checked.
2. Report an alias `kind G = F` once, at the clause of `F`.
3. Use a message that names the kind.

## Answers

```
D-133: Keep
D-134: Keep
D-135: Keep
D-136: Keep
D-137: Keep
D-138: Keep
D-139: Keep
D-140: Keep
D-141: Keep
D-142: Keep
D-143: Keep
D-144: Switch to B
D-145: Keep
D-146: Keep
D-147: Keep
D-148: Keep
D-149: Keep
D-150: Keep
D-151: Keep
D-152: Keep
D-153: Keep
D-154: Keep
D-155: Keep
D-156: Keep
D-157: Keep
D-158: Keep
D-159: Keep
ADR 0039: Keep
ADR 0040: Keep
ADR 0041: Keep
ADR 0042: Keep
ADR 0043: Keep
ADR 0044: Escalate
ADR 0045: Escalate
R-1: Keep
R-3: Keep
R-5: Keep
R-023: Keep
R-025: Keep
R-028: Keep
R-031: Keep
```

Notes on the answers:
- D-137 and D-157: keep the rules, and fix the implementations (N-003, N-016).
- ADR 0044: the recommendation is Accept (E5).
- ADR 0045: the recommendation is Amend, then Accept, before the tag (E3).
- R-1: keep as stated.
- R-3: keep the compiler's order, and add one sentence to spec/10 §12 rule 13: count (U2001) before labels.
- R-5: list `--solver-wallclock` in spec/13 §5 for `check`, `run`, `test`, `obligations`, `trace` and `report`.
- R-023, R-025, R-028 and R-031: keep them deferred. None blocks the tag.
