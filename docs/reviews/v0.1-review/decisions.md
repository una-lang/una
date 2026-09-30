# Decisions review

The scope is the 132 entries of `docs/reviews/open-decisions.md` (D-001 to D-132) and the ADRs that were Provisional. On 2026-09-29 the owner kept every chosen option and accepted ADRs 0017 to 0035, so no ADR is Provisional at `c2f3386`. The review below is still independent: each decision was matched against its `DECISION D-NNN` tags and tests, often with a CLI probe, and judged on the criteria of the review brief. Those criteria, in order: spec text and intent, charter principles and proof boundary, strictness, consistency with accepted ADRs, simplicity, and agent ergonomics.

## Statistics

| Recommendation | Count |
| --- | --- |
| Keep | 131 |
| Switch | 0 |
| Escalate | 1 (D-093) |

- Implementation mismatches: 4 decisions are right, but their implementation is incomplete.
  - D-048: R-020.
  - D-079: R-002.
  - D-085: R-001.
  - D-103: R-011.
- These are fixes, not reversals. Four of the five Critical findings they connect to trace back to them: R-001, R-002, R-003 and R-011. Most Critical findings (R-004 to R-010) are gaps that no decision covered.
- ADRs: the accepted ADRs 0017 to 0035 were read with their decisions, and none should be reverted.
- Two ADRs need amendments, both through the escalations:
  - ADR 0031 rules 4 and 5, for the handler pc (R-003).
  - ADR 0028 rule 9, for effectful type arguments (R-002).
- ADRs 0036 and 0037 are Proposed on `dev/v0.1`, are outside this tree, and were not evaluated as decisions. R-013 and R-040 cover their subjects.

## Table

"A" is `docs/reviews/v0.1-review/adversarial/`. Paths without a crate prefix are under `crates/una-check/src/` unless the cell says otherwise.

| ID | Recommendation | Rationale | Evidence | Refactoring cost |
| --- | --- | --- | --- | --- |
| D-001 | Keep | Acronym follows the name. Superseded by ADR 0025, which applied the same rule (TAM became UAM). | `docs/spec/11-dynamic-semantics.md:5-7`, ADR 0025:36. Matches (as UAM). | None |
| D-002 | Keep | Cosmetic. Note: the historical title now reads "Una Language Specification v0.3", a name that document never had (renamed twice). | `docs/00-charter.md:3`. Matches. | One line |
| D-003 | Keep | Ordinary English words are not the codename. | the first naming audit command hits only history files and `OVERNIGHT.md`. Matches. | None |
| D-004 | Keep | Superseded by D-058 (probes now `u0001`, `U00001`, `concat!("T","0001")`). Obsolete. | `crates/una-diag/src/codes.rs:761-767`, `crates/una-cli/tests/cli.rs:57`. Matches D-058. | None |
| D-005 | Keep | Obsolete: D-083 replaced it, CI now runs on every push; no `overnight/**` entry remains. | `.github/workflows/ci.yml:1-10`. Superseded, not a mismatch. | None |
| D-006 | Keep | Keeps test writes under `target/`. | `crates/una-cli/tests/conformance/discovery.rs:260-262`; no `temp_dir()` in crates. Matches. | None |
| D-007 | Keep | Keeps the owner's text intact. | `docs/reviews/triage.md:98`. Matches. | None |
| D-008 | Keep | Only reading consistent with the list. | `OVERNIGHT.md:262`; probe `007` gives U0007 with edit to `7`. Matches. | None |
| D-009 | Keep | Now spec text (`spec/13 §7`); consistent with ADR 0013 D6. | `crates/una-project/src/package.rs:161`, `crates/una-cli/tests/cli.rs:290`; probe: garbage in `src/.#x.una` and `src/.hid/y.una` ignored, exit 0. Matches. | None |
| D-010 | Keep | Now spec text (`spec/13 §5`: "Usage errors stay on stderr, with exit code 2"). | `crates/una-cli/src/lib.rs:421`; probe `--json check <missing>`: stdout empty, stderr message, exit 2. Matches. | None |
| D-011 | Keep | Old expectation contradicted ADR 0015 D3. | `crates/una-cli/tests/cli.rs:91,158`. Matches. | None |
| D-012 | Keep | Only stderr output is colored. | `crates/una-cli/src/main.rs:11`; probe `--color always` on report, obligations, test, explain, run: no ESC byte on stdout. Matches. | None |
| D-013 | Keep | Rule 1 applies to every line; stricter; one canonical form. | `crates/una-syntax/src/lexer.rs:288`, `tests/lexer.rs:521`; probe tab before comment line and inside brackets: U0002 with edit. Matches. | None |
| D-014 | Keep | Precise message at the right token. | `lexer.rs:617`, `tests/lexer.rs:623`; probe `@if`: U0101 "keyword `if` cannot name a capability". Matches. | None |
| D-015 | Keep | Prevents a generic function being swallowed by recovery. | `lexer.rs:58`, `tests/lexer.rs:594`; probe: unclosed `(` then `g[T](...)`: both errors reported. Matches. | None |
| D-016 | Keep | Consistent with ADR 0017's enumeration; both options reject. Ergonomics note in R-045. | `lexer.rs:561`, `tests/lexer.rs:278`; probe `12abc`: U0101 at `abc`. Matches. | None |
| D-017 | Keep | Affects only recovery after U0002; edit uses the same width. | `lexer.rs:273`, `tests/lexer.rs:510`. Matches. | None |
| D-018 | Keep | Keeps LL(2); rejects rather than guesses; now spec text (`spec/02:50`). | `parser.rs:795,864`, `tests/parser.rs:174`; probe `secret [Int]`: U0101 with `secret List[T]` hint. Matches. | None |
| D-019 | Keep | Avoids cascades; scoped to one declaration. | `parser.rs:300`, `tests/parser.rs:694`; probe: `a & b` gives only U0001; a U0101 in the next declaration is still reported. Matches. | None |
| D-020 | Keep | Purely syntactic rule, stricter reading of "each clause". | `parser.rs:97`, `tests/parser.rs:411`, `tests/conformance/syntax/handling_clause_out_of_order.una`. Matches. | None |
| D-021 | Keep | Never loses documentation. Spec gap noted in R-026. | `parser.rs:630`, `tests/parser.rs:717`; probe: two doc groups split by a blank line and a plain comment both appear in `una contract`. Matches. | None |
| D-022 | Keep | Deterministic; spec text now (`spec/02:206`). The limit also counts operator chains, so a 257-term sum is rejected; acceptable and raisable later. | `parser.rs:107,126`, `tests/robustness.rs`, `cli/src/lib.rs:245`; probes: 20 000 parens and 200 000-term sum give U0101 (no crash); 250-deep nesting passes check, fmt, run, test, obligations, report, contract. Matches. | None |
| D-023 | Keep | Converging bytes (ADR 0011, P4); spec/15 now states the rules. | `crates/una-fmt/src/layout.rs:241`, `rewrite.rs:265`, `tests/format.rs:265,296`; probe: applying every U7001 edit equals `una fmt` output. Matches. | None |
| D-024 | Keep | One shape, one rule; stricter; spec/15 now states it. | `rewrite.rs:496`, `tests/format.rs:166`, `tests/conformance/fmt/inline_arm_body.una`; probe: one-line arm joined. Matches. | None |
| D-025 | Keep | Order-independent output across agents. | `layout.rs:274`, `rewrite.rs:131`; probe `import a.c (z, K)` becomes `(K, z)`. Matches. | None |
| D-026 | Keep | Scope decision is sound and imported kinds were added. See R-023 on constructor reordering and evaluation order (a spec issue, not this decision's scope choice). | `rewrite.rs:40,144`, `tests/format.rs:198`, `tests/conformance/fmt/imported_constructor_order/`; probe: imported `K(y:, x:)` reordered. Matches. | None |
| D-027 | Keep | Formatting a partial tree could delete code. | `cli/src/lib.rs:229`, `una-fmt/src/lib.rs:5`; probe: file with syntax errors gets no U7001 and `una fmt` leaves it untouched, exit 1. Matches. | None |
| D-028 | Keep | Small, independent, mechanically applicable edits. | `una-fmt/src/diff.rs:11`; probe: 7 U7001 regions, applying all edits reproduces `una fmt` byte for byte. Matches. | None |
| D-029 | Keep | Safe partial formatting, atomic writes. Minor defect in R-024 (symlinks). | `cli/src/lib.rs:496`, `tests/cli.rs:398`; probe: permissions kept (600), broken files untouched. Matches, with R-024. | None (R-024 is small) |
| D-030 | Keep | Checks every file while allowing only the documented rewrites; normalizer sorts labeled args only for upper-case callees. | `crates/una-fmt/tests/properties.rs:120-304,360`. Matches. | None |
| D-031 | Keep | Canonical form must not depend on checkout. | `.gitattributes:1-5`; `git ls-files --eol`: only the two byte-exact cases are non-LF. Matches. | None |
| D-032 | Keep | Settings live in `una.toml`; now spec text (`spec/13:277`). | `una-project/src/package.rs:100`, `tests/project.rs:210`, `cli/tests/cli.rs:230`; probe: no `una.toml` or no `src/` gives exit 2. Matches. | None |
| D-033 | Keep | Strict, loosenable later. | `una-project/src/manifest.rs:32`, `tests/project.rs:65`; probe: edition, unknown key, negative budget, bad entry, bad grant all U7002 (plus U1005, U1002 for categories). Matches. Exit code note in R-040. | None |
| D-034 | Keep | Only one encoding, solver, logic exist. | `una-project/src/lock.rs:50`, `tests/project.rs:176`; probe: wrong `encoding_version` and each missing key reported (U7004). Matches. | None |
| D-035 | Keep | More problems per run; defaults are strictest. | `cli/src/package.rs:55`, `tests/cli.rs:490`; probe: broken TOML plus a module error reports both. Matches. Exit code note in R-040. | None |
| D-036 | Keep | One diagnostic shape; testable position. | `cli/src/package.rs:78`; probe `--json`: U7004 at `una.toml` 1:1..1:1. Matches. | None |
| D-037 | Keep | Makes both reference uses valid; ambiguity reported, not guessed. Untagged in code (only ADR 0023 cites it). | `crates/una-check/src/names/` (no `DECISION D-037` tag); probe: `Red` from two imported kinds in an untyped context gives U1001 "ambiguous"; module `NotFound` shadows prelude. Matches. | None |
| D-038 | Keep | One code per meaning. | `names/resolve.rs:87,332`, `types/env.rs:1155`, `tests/names.rs:93`; probe: unknown type U1001, variant as type U2002. Matches. | None |
| D-039 | Keep | Strict where it matters (locals), tolerant of prelude growth. Note in R-045. | `names/resolve.rs:107`, `tests/names.rs:142,186`; probe `let x` over parameter `x`: U1002. Matches. | None |
| D-040 | Keep | One cycle, one diagnostic; `pub` edit is mechanical. | `names/mod.rs:613,671`, `tests/names.rs:289`; probe: one U1003 at first import of `a.b`; U1004 with edit `pub ` at the declaration. Matches. | None |
| D-041 | Keep | Avoids cascades from lost declarations. | `names/mod.rs:576`, `tests/names.rs:356`; probe: import of a lost name and its use unreported; U1002 still reported; other modules still resolved. Matches. | None |
| D-042 | Keep | Stricter, same classes. Type parameters in lower case are already U0101 in the parser. | `names/conventions.rs:1`, `tests/names.rs:208`; probe: `TT_x` U1005, `bad__m` path segment U1005. Matches. | None |
| D-043 | Keep | Every code keeps checked failing and fixed examples. | `crates/una-cli/tests/explain_examples.rs:4`, `crates/una-diag/explain/U1003.md`. Matches. | None |
| D-044 | Keep | Protects finished codes with no list to maintain; moot now that all milestones are done, still a good guard. | `crates/una-cli/tests/conformance/main.rs:186`. Matches. | None |
| D-045 | Keep | `spec/07 §6` asks for two declarations, and a single file cannot supply the second one, so U7005 is both the literal reading and the stricter one. | `names/mod.rs:728`, `tests/names.rs:322`. Matches. | none |
| D-046 | Keep | Now normative in `spec/10 §11` rule 3. M3 skips contextual variants and M4 resolves them. | `names/resolve.rs:305,442,459,522`, `tests/names.rs:125`. Matches (probe: an unknown pattern variant gives U1001). | none |
| D-047 | Keep | A staging decision. M5 now reports an unbound `@c` as U3001. | `types/call.rs:551`, `types/check.rs:148`, `tests/types.rs:922`. Matches. | none |
| D-048 | Keep (fix the implementation) | Option A is `spec/10 §12` rule 1. Patterns, `is` and `fail` match it, but `expect fails V` also accepts any variant in scope. | `types/check.rs:1146-1172` (`expected_failure`), `types/expr.rs:648,1034`, `tests/types.rs:856`. **Partial mismatch, R-020.** | low |
| D-049 | Keep | `spec/10 §12` rule 2. A bottom type is what the free τ of T-Fail means. | `types/expr.rs:705-735`. Matches (the doc comment is duplicated at 706-709, which is cosmetic). | none |
| D-050 | Keep | Now in `spec/03 §3`, and `spec/12 §4` encodes text literals. | `types/expr.rs:194`. Matches: `requires t != ""` is refuted at tier 1 when it should be. | none |
| D-051 | Keep | `spec/10 §12` rule 3. Stricter, and each flagged arm is dead code. | `types/stmt.rs:389`, `tests/types.rs:575`. Matches. | none |
| D-052 | Keep | Follows the order of `spec/04 §3`. An unresolved name is U1001 with an edit. | `types/call.rs:3,22,124`, `tests/types.rs:464`. Matches (probe: `p.z` gives U1001 "write `x`"). | none |
| D-053 | Keep | Function types have no parameter names, and Core IR has no constructor values. This can be relaxed later. | `types/call.rs:217,472`, `types/expr.rs:329`, `tests/types.rs:440`. Matches. | none |
| D-054 | Keep | Defaulting would hide mistakes and break the explicit types of Core IR. | `types/unify.rs:9`, `types/check.rs:528`, `tests/types.rs:380`. Matches. | none |
| D-055 | Keep | Signatures written in Una go through the same lowering as user code. The guard test compares only names . | `types/mod.rs:36-40`, `src/std/prelude.una`, `tests/types.rs:158-230`. Matches, and a manual diff of the 20 prelude signatures against `spec/14 §1` found no drift. | none |
| D-056 | Keep | M4 staging of U2015, which M5 completes. | `types/check.rs:401`, `types/call.rs:391`, `tests/types.rs:752`. Matches (probes: declared-failing, inferred-failing and effectful callees all give U2015). | none |
| D-057 | Keep | A staging decision, superseded by the label model of D-088. | `types/env.rs:951`. Matches. | none |
| D-058 | Keep | Test data only. It keeps the old-prefix probes and an exact rename audit. | `una-diag/src/codes.rs:762`, `una-cli/tests/cli.rs:57`. Matches. | none |
| D-059 | Keep | A one-time snapshot regeneration that the owner approved, with no semantic effect. | `una-syntax/tests/snapshots/examples__events__*.snap`, `una-diag/tests/snapshots/`. Consistent. | none |
| D-060 | Keep | The payload makes the example agree with `spec/06 §1, §3`. | `examples/events/src/events/checkin.una:8,31,36`. Matches. | none |
| D-061 | Keep | Each excluded form has no encoding in `spec/12 §4`, and the edits keep the agent cost low. This can be relaxed later. | `types/expr.rs:507,576,618`. Matches (probes give U2014 for `t < "z"`, `l == Low` and `n is 1`). | none |
| D-062 | Keep | The literal reading, now normative in `spec/10 §12` rule 8. It over-rejects some finite kinds , but relaxing it later breaks no program. | `types/facts.rs:89,113`, `tests/types.rs:275`. Matches, and aliases are seen through. | none |
| D-063 | Keep | Sound on all 8 routes tried: direct, lambda in a generic body, `Box[T]`, generic as a function value, `let h = same`, `map`, `contains` on `Option[fn]`, and `ensures`. | `types/facts.rs:276-340`, `types/expr.rs:496`, `types/check.rs:558`, `tests/types.rs:825`. Matches. | none |
| D-064 | Keep | The literal reading of `spec/04 §3` rule 3. Note that `sub(b, a)` silently means `sub(a: a, b: b)` . | `types/call.rs:665,933`, `tests/types.rs:528`. Matches. | none |
| D-065 | Keep | Stricter, and required for reflect soundness. The "totality" rationale is incomplete, because an opaque predicate can still call a recursive function outside its cycle. | `types/facts.rs:212`, `types/check.rs:711`. Matches. | none |
| D-066 | Keep | Prevents an ill-typed declassified value. The mismatch moved to U4005 per D-094. | `types/check.rs:748`, `types/call.rs:472`, `tests/types.rs:518`, `tests/conformance/security/via_of_another_type`. Matches. | none |
| D-067 | Keep | A wrong count is a type mismatch, and U2004 is about labels. | `types/call.rs:679`. Matches (probes give "takes 2 arguments, but 3 are given" and "missing argument `b`"). | none |
| D-068 | Keep | The tour inputs violated `spec/04 §3` rule 2, so fixing the inputs was right. | `tests/conformance/syntax/grammar_tour.una:40`, `tests/conformance/names/resolution_tour.una:16`. Both check clean. | none |
| D-069 | Keep | Option B would let wrong field accesses pass. Checking lambdas last covers the common case. | `types/call.rs:1102`, `types/check.rs:413`. Matches. | none |
| D-070 | Keep | An internal record, sorted deterministically, and superseded by M7 and M8. | `types/mod.rs:187`, `tests/types.rs:125-148`. Matches. | none |
| D-071 | Keep | One site per code, as `spec/05 §9` rule 1 now states. | `types/env.rs:824`, `types/check.rs:1001`, `types/authority.rs:255,350,591`, `tests/conformance/authority/` cases. Matches. | none |
| D-072 | Keep | Callee-declared requirements decide first, which follows "flow downward by name" (`spec/05 §4`). Option B is ambiguous. | `types/env.rs:758`, `types/authority.rs:90,891,1139`, `types/call.rs:570`, `tests/conformance/authority/inferred_headers`, `inferred_capability_conflict`. Matches. | none |
| D-073 | Keep | Each handler problem is reported where the agent edits. | `types/check.rs:1034`, `tests/conformance/authority/handler_*`. Matches. | none |
| D-074 | Keep | `spec/05 §4` rule 2 requires checking the handler's own `uses`, and the `given` order is sound. | `types/check.rs:455,464`. Matches (probes give U3001 for missing `@other`, and U3010 for the wrong capability). | none |
| D-075 | Keep | U3014 gets a case of its own. A nonexistent operation is U3002 after `expect performed` but U1001 in a body (see R-028). | `types/check.rs:1116`, `tests/conformance/authority/host_capability_in_example`, `given_missing`. Matches ADR 0028 rule 5. | low |
| D-076 | Keep | The manifest's grants are static, and checking them early is stricter. `una check` can reject what `una run --grant` accepts . | `types/authority.rs:1185,1211`, `tests/conformance/authority/*_host_capability`. Matches. | none |
| D-077 | Keep | Imports serve the handlers, and the use is what matters. | `types/authority.rs:578`, `tests/conformance/authority/testing_outside_handlers`. Matches. | none |
| D-078 | Keep | One diagnostic per module, fixed by one change. | `types/authority.rs:1296`, `tests/conformance/authority/escape_budget_exceeded`. Matches. | none |
| D-079 | Keep (fix the implementation) | Option A is the right rule, but only values whose type is directly an effectful `fn` are checked. Effectful function values escape through generics and `Option`, and outlive their `with` scope. | `types/expr.rs:231,255,901`, `types/authority.rs:1337` (`stored_effects`). **Mismatch, R-002.** | medium |
| D-080 | Keep | The literal reading of `spec/04 §4`. Probes that hid effects in parentheses, methods, function values, `if` branches, interpolation and list literals were all caught. | `types/authority.rs:817-858`, `tests/conformance/authority/nested_effect.una`. Matches, except inside predicate bodies (see R-029). | none |
| D-081 | Keep | Adds no CLI syntax and never shows possibly incomplete inferred clauses. | `una-cli/src/contract.rs`, `una-cli/src/lib.rs:555-600`, `una-cli/tests/cli.rs:565`. Matches, except the order of checks (see R-027). | low |
| D-082 | Keep | Method calls and function values need types before they can resolve, so option B cannot work. | `types/mod.rs:129-160`, `types/check.rs:152`. Matches. | none |
| D-083 | Keep | Follows the owner's instruction with one matrix. The entry cites an "`on.push` filter" that does not exist (see R-044). | `.github/workflows/ci.yml:3-26`. Matches in behavior. | none |
| D-084 | Keep | Each unit-test file states the rules of its own milestone, and conformance still compares full output. | `tests/types.rs:60-78` (`later` filter). Matches. | none |
| D-085 | Keep (fix the implementation) | The codes are right, but S-Fun is enforced only at direct call arguments, and `Fn` unification ignores `uses` and `fails`. Effects and failures enter pure function types through generics. | `types/call.rs:315-356` (called only at `call.rs:286,790`), `types/unify.rs:207-213`. **Mismatch, R-001.** | medium |
| D-086 | Keep | Probe: after the rank-1 edits for U3002, U3003, U3009 and U3008 were applied, `una fmt --check` passed and the diagnostics went away. | `types/authority.rs:516,542`. Matches. | none |
| D-087 | Keep | Consistent with U1002 meaning a name defined twice. | `types/authority.rs:860`, `types/env.rs:1215`, `tests/conformance/authority/cap_bound_twice.una`. Matches. | none |
| D-088 | Keep | The model of Core IR (`spec/09 §2`). No label was lost through generics, `Option`, `List`, `map` and `first`, generic records, closures or lambdas. | `types/ty.rs:39`, `types/unify.rs:60,141`, `types/flow.rs:51`. Matches. | none |
| D-089 | Keep | Flow points are where a declared type meets a value (spec/07 §7 rule 2). My probes of implicit flows, lambda results and generic calls were all rejected. | `una-check/src/types/flow.rs:16`, `tests/flow.rs:420`; probes p1, p2. Matches | none |
| D-090 | Keep | `.len` and `is` read only structure. Probes: `xs == []` and `filter` on secret elements are rejected, and `xs.first()` on a secret-topped list is U4001. | `check.rs:160`, `stmt.rs:465`, `expr.rs:78,600`. Matches | none |
| D-091 | Keep | Stricter than T-Checked alone and closes a real channel. | `expr.rs:980`; probe p2 gives U4002. Matches | none |
| D-092 | Keep, with the fix of R-003 | Treating `accepts` as the sink is right, and handler parameters are raised. But the handler body starts at pc ⊥ (see D-093). | `flow.rs:281`, `check.rs:913`, `env.rs:685`. Matches | none for D-092 itself |
| D-093 | Escalate (amend) | Checking the handler's `uses` at the installation pc is the wrong place: the handler's effects run where the operation is performed, under a possibly higher pc. Noninterference (T6) breaks (see R-003). | `check.rs:458`, `flow.rs:195`; `A/flow-handler-pc.una`. Matches the entry, but the entry is unsound | low: one check per handler op, plus ADR and spec text |
| D-094 | Keep | Strictest reading of "only entry point" (spec/07 §5 rule 3). | `check.rs:753,823`, `flow.rs:258`; conformance `security/via_called_directly`, `via_of_another_type`. Matches | none |
| D-095 | Keep | Clauses produce no run-time value. `checked` is still covered by D-091. | `check.rs:66,685,986`, `flow.rs:46`. Matches | none |
| D-096 | Keep | One diagnostic per edit site. | `env.rs:1039`; probe `cat.una` gives 3x U4007 at each category. Matches | none |
| D-097 | Keep | Spans point at the edit site, with the related condition. | `flow.rs:132,282`; probes show the related "labeled condition". Matches | none |
| D-098 | Keep | Keeps one resolver. | `una-check/src/types/records.rs:2`, `una-core/src/ir.rs:452`. Matches | none |
| D-099 | Keep | A debug aid. | `una-cli/src/lib.rs:428`, `tests/cli.rs:637`; probe dump printed with U4007 on stderr. Matches | none |
| D-100 | Keep | Follows the spec/11 §5 example. | `una-interp/src/machine.rs:223,797`; probe trace shows `"<redacted:secret>"`. Matches | none |
| D-101 | Keep | stdout belongs to the program. The `rejected` status exits with 1. | `una-cli/src/runtime.rs:4`; probes `leak.una`, `fl.una`. Matches | none |
| D-102 | Keep (see R-025) | One document on stdout. | `runtime.rs:4`, `tests/runtime.rs:599`. Matches, but `result` is not redacted | low |
| D-103 | Keep A, fix the implementation (see R-011) | Adding grants up and resolving links is right. The link check misses a dangling link, which `fs::write` then follows. | `una-interp/src/host.rs:149-181`, `una-cli/src/runtime.rs:94`; `A/runtime-files-dangling-link` escaped the prefix. Mismatch with spec/13 §5 and spec/14 §3 | low |
| D-104 | Keep | Consistent with scalar-value columns. `is_digits("")` holding is vacuous truth. Note: validators should also require `len > 0`. | `value.rs:94`, `machine.rs:920`; probe `pre.una`: `to_int("-")`, `""`, `"+5"`, Arabic digits all give None. Matches | none |
| D-105 | Keep | The stack is sized from the depth, so no overflow. | `una-interp/src/lib.rs:25`, `machine.rs:441`; `runtime/call_depth`. Matches | none |
| D-106 | Keep | Stricter: an unexpected failure fails the example. | `harness.rs:152`; `runtime/examples_fail`. Matches | none |
| D-107 | Keep (see R-025) | Redacting nested labeled fields is right. | `machine.rs:224`; probe `red.una` shows `pin: <redacted:secret>`. Matches for traces | none |
| D-108 | Keep | Unreachable after tier 1, and no new code is invented. | `machine.rs:1117`. Matches | none |
| D-109 | Keep | Locates U3012 at `uses` before the run. | `runtime.rs:171`, `tests/runtime.rs:501`; probe gives `rejected` with U3012. Matches | none |
| D-110 | Keep | One expectation form. | `tests/conformance/main.rs:48`, `docs/compiler/testing.md:49`. Matches | none |
| D-111 | Keep | Verifies Core, which is what runs. | `una-verify/src/generate.rs:740`, `una-interp/src/machine.rs:465`. Matches | none |
| D-112 | Keep | Avoids refutations for the wrong reason. | `una-cli/src/verification.rs:4`, `lib.rs:92`. Matches | none |
| D-113 | Keep | Ids stay per clause, and the disjunction of paths is sound. | `generate.rs:34`, `tests/verification.rs:299`; `postcondition_paths_proven`. Matches | none |
| D-114 | Keep | Each fact is discharged elsewhere. Probe: a refined field of a `fail` payload is itself an obligation, so no unverified construction path was found. | `generate.rs:1865`; probe `gf.una`. Matches | none |
| D-115 | Keep | Uninterpreted functions add no axioms, so this is sound. U1005 rejects `_` in kind names at tier 0, which keeps sort names injective. | `una-verify/src/smt.rs:2,207`; probe `col.una`. Matches | none |
| D-116 | Keep | Independent queries, and a hang is isolated. | `una-verify/src/run.rs:73`, `solver.rs:2`. Matches | none |
| D-117 | Keep | Atomic writes, and time-dependent results are not cached. | `una-verify/src/cache.rs:2`. Matches | none |
| D-118 | Keep | Surface syntax suits agents. Shared ids for identical goals are the owner's explicit answer. | `generate.rs:29`; `una obligations` on the fifth example package lists `ob-a099c4ca4354` twice. Matches | none |
| D-119 | Keep (see R-021) | Source names only. An empty model prints `counterexample: ` with nothing after it. | `generate.rs:40,2018`; probe `red2.una`. Matches | none |
| D-120 | Keep (see R-021) | The table order is right, but repairs are proposed even for constant goals (`guard with if 0 != 0`). | `una-verify/src/report.rs:91`; probe `red2.una`. Matches | trivial |
| D-121 | Keep | An unproven obligation is an error in the program. | `una-cli/src/verification.rs:4`. Matches | none |
| D-122 | Keep (see R-045) | A whole-package view. Handlers with `uses` do not appear in `authority`, which follows spec/13 §6 ("every function"). | `una-cli/src/report.rs:3`, `tests/abi.rs:168`; `A/flow-handler-pc.una` report. Matches | none |
| D-123 | Keep | The semantic test catches `x >= x`. | `report.rs:3`, `tests/abi.rs:117`. Matches | none |
| D-124 | Keep | Consistent with `una contract`. | `una-cli/src/trace.rs:4`, `tests/abi.rs:272`; probe: unknown id exits with 2. Matches | none |
| D-125 | Keep | Every command is headless. | `tests/cli.rs:71`, `tests/abi.rs:305`; probe `una explain U4001 --json` prints `una.explain/1`. Matches | none |
| D-126 | Keep | An ICE example would be a known bug left in the compiler. | `tests/explain_examples.rs:147`. Matches | none |
| D-127 | Keep | A test keeps the coverage claim true. | `tests/examples.rs:1,330`. Matches | none |
| D-128 | Keep | Snippets are copied verbatim, so each is tested whole. | `una-cli/tests/guide.rs:1-2` (tag wraps a line). Matches | none |
| D-129 | Keep | Fits the CI quota. | `scripts/release.sh:4`, `.github/workflows/release.yml:3`, `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`. Matches | none |
| D-130 | Keep | The release belongs to the owner. No tag exists in the repo. | `release.yml:4`; `git tag -l` is empty. Matches | none |
| D-131 | Keep | `Ok` is exactly a normal return, and the rule stays modular (the callee's `ensures`, not its body). | `generate.rs:923-955`; `attempt_ensures_proven` and `attempt_ensures_refuted`. Matches | none |
| D-132 | Keep | The report needs what `una-cli` holds, and moving it later is cheap. | `una-cli/src/report.rs:4`. Matches | none |

## Escalation

- D-093: the handler's `uses` are checked at the installation pc, but its operations run at the pc of the site that performs them. `A/flow-handler-pc.una` leaks a secret through a handler's Console effect (R-003). The fix amends ADR 0031 and spec/07 §7, so the owner decides the rule. See `escalations.md` E2.

## Answers

```
D-001: Keep
D-002: Keep
D-003: Keep
D-004: Keep
D-005: Keep
D-006: Keep
D-007: Keep
D-008: Keep
D-009: Keep
D-010: Keep
D-011: Keep
D-012: Keep
D-013: Keep
D-014: Keep
D-015: Keep
D-016: Keep
D-017: Keep
D-018: Keep
D-019: Keep
D-020: Keep
D-021: Keep
D-022: Keep
D-023: Keep
D-024: Keep
D-025: Keep
D-026: Keep
D-027: Keep
D-028: Keep
D-029: Keep
D-030: Keep
D-031: Keep
D-032: Keep
D-033: Keep
D-034: Keep
D-035: Keep
D-036: Keep
D-037: Keep
D-038: Keep
D-039: Keep
D-040: Keep
D-041: Keep
D-042: Keep
D-043: Keep
D-044: Keep
D-045: Keep
D-046: Keep
D-047: Keep
D-048: Keep
D-049: Keep
D-050: Keep
D-051: Keep
D-052: Keep
D-053: Keep
D-054: Keep
D-055: Keep
D-056: Keep
D-057: Keep
D-058: Keep
D-059: Keep
D-060: Keep
D-061: Keep
D-062: Keep
D-063: Keep
D-064: Keep
D-065: Keep
D-066: Keep
D-067: Keep
D-068: Keep
D-069: Keep
D-070: Keep
D-071: Keep
D-072: Keep
D-073: Keep
D-074: Keep
D-075: Keep
D-076: Keep
D-077: Keep
D-078: Keep
D-079: Keep
D-080: Keep
D-081: Keep
D-082: Keep
D-083: Keep
D-084: Keep
D-085: Keep
D-086: Keep
D-087: Keep
D-088: Keep
D-089: Keep
D-090: Keep
D-091: Keep
D-092: Keep
D-093: Escalate
D-094: Keep
D-095: Keep
D-096: Keep
D-097: Keep
D-098: Keep
D-099: Keep
D-100: Keep
D-101: Keep
D-102: Keep
D-103: Keep
D-104: Keep
D-105: Keep
D-106: Keep
D-107: Keep
D-108: Keep
D-109: Keep
D-110: Keep
D-111: Keep
D-112: Keep
D-113: Keep
D-114: Keep
D-115: Keep
D-116: Keep
D-117: Keep
D-118: Keep
D-119: Keep
D-120: Keep
D-121: Keep
D-122: Keep
D-123: Keep
D-124: Keep
D-125: Keep
D-126: Keep
D-127: Keep
D-128: Keep
D-129: Keep
D-130: Keep
D-131: Keep
D-132: Keep
```
