# Spec coverage

This file supports finding R-037. The script `scripts/diag-coverage.py` in this directory regenerates the table in section 1: run `python3 docs/reviews/v0.1-review/scripts/diag-coverage.py` from the repository root.

## 1. Diagnostic codes

Script: `docs/reviews/v0.1-review/scripts/diag-coverage.py [REPO_ROOT]`. It parses the §4 registry of `spec/13`, discovers conformance cases the way `docs/compiler/testing.md` §2 defines them (single-file cases and multi-module directories, directive block only), and counts per code:

- **Fail cases**: cases with `-- expect: error <code>`.
- **Positive (same §)**: cases with no `error` expectation (`ok`, `tier0-ok`, `run-output`, `test-pass`) whose `-- spec:` cites a section cited by one of the code's failing cases, or whose `-- test:` names the code.
- **Positive (same spec file only)**: positive cases that cite only another section of the same spec file (weaker evidence).
- **Crate test files / mentions**: files under `crates/*/tests` (including snapshots) that mention the code.
- **Explain**: `crates/una-diag/explain/<code>.md` exists and has a failing and a fixed example. `crates/una-cli/tests/explain_examples.rs` checks every failing example yields exactly its code and every fixed example yields nothing, so each fixed example is a positive twin for every code but U8001.

Output (run at c2f3386):

| Code | Domain | Fail cases | Positive cases (same §) | Positive (same spec file only) | Crate test files / mentions | Explain | Failing-case spec sections |
| --- | --- | --- | --- | --- | --- | --- | --- |
| U0001 | syntax | 7 | 2 | 6 | 5 / 41 | yes+examples | spec/01 §1 |
| U0002 | syntax | 2 | 3 | 5 | 2 / 5 | yes+examples | spec/01 §2 |
| U0003 | syntax | 3 | 3 | 5 | 1 / 2 | yes+examples | spec/01 §2 |
| U0004 | syntax | 3 | 2 | 6 | 1 / 4 | yes+examples | spec/01 §6 |
| U0005 | syntax | 2 | 2 | 6 | 1 / 7 | yes+examples | spec/01 §6 |
| U0006 | syntax | 1 | 2 | 6 | 2 / 3 | yes+examples | spec/01 §6 |
| U0007 | syntax | 4 | 2 | 6 | 1 / 6 | yes+examples | spec/01 §6 |
| U0101 | syntax | 7 | 7 | 5 | 8 / 65 | yes+examples | spec/01 §2, spec/01 §5, spec/01 §7, spec/02 §1, spec/02 §3, spec/02 §6 |
| U0102 | syntax | 7 | 1 | 4 | 2 / 16 | yes+examples | spec/02 §3, spec/02 §4, spec/06 §3 |
| U0103 | syntax | 2 | 1 | 7 | 2 / 7 | yes+examples | spec/01 §5 |
| U1001 | names | 6 | 3 | 1 | 4 / 24 | yes+examples | spec/02 §1, spec/02 §2, spec/02 §6, spec/02 §7 |
| U1002 | names | 7 | 7 | 3 | 3 / 23 | yes+examples | spec/02 §1, spec/02 §3, spec/02 §6, spec/03 §2, spec/05 §1, spec/08 §1, spec/08 §2 |
| U1003 | names | 1 | 2 | 2 | 2 / 7 | yes+examples | spec/02 §1 |
| U1004 | names | 1 | 1 | 0 | 1 / 1 | yes+examples | spec/04 §5 |
| U1005 | names | 3 | 0 | 8 | 2 / 8 | yes+examples | spec/01 §4 |
| U1006 | names | 1 | 0 | 4 | 1 / 1 | yes+examples | spec/08 §1 |
| U1007 | names | 1 | 0 | 4 | 1 / 1 | yes+examples | spec/08 §1 |
| U1008 | names | 1 | 0 | 0 | 1 / 1 | yes+examples | spec/13 §7 |
| U2001 | types | 2 | 1 | 1 | 5 / 44 | yes+examples | spec/03 §1, spec/10 §3 |
| U2002 | types | 2 | 0 | 0 | 1 / 14 | yes+examples | spec/03 §2 |
| U2003 | types | 1 | 0 | 0 | 1 / 5 | yes+examples | spec/03 §2 |
| U2004 | types | 1 | 0 | 1 | 1 / 5 | yes+examples | spec/04 §3 |
| U2005 | types | 1 | 0 | 1 | 1 / 4 | yes+examples | spec/04 §3 |
| U2006 | types | 1 | 0 | 1 | 1 / 4 | yes+examples | spec/04 §3 |
| U2007 | types | 1 | 2 | 0 | 1 / 2 | yes+examples | spec/10 §4 |
| U2008 | types | 2 | 2 | 4 | 1 / 5 | yes+examples | spec/02 §8, spec/10 §4 |
| U2009 | types | 2 | 0 | 1 | 1 / 9 | yes+examples | spec/04 §3 |
| U2010 | types | 1 | 0 | 0 | 1 / 2 | yes+examples | spec/03 §7 |
| U2011 | types | 1 | 0 | 4 | 1 / 2 | yes+examples | spec/02 §6 |
| U2012 | types | 1 | 0 | 4 | 1 / 4 | yes+examples | spec/02 §8 |
| U2013 | types | 2 | 0 | 4 | 1 / 3 | yes+examples | spec/02 §8 |
| U2014 | types | 2 | 0 | 0 | 1 / 12 | yes+examples | spec/03 §3 |
| U2015 | types | 2 | 0 | 0 | 1 / 8 | yes+examples | spec/03 §4 |
| U3001 | authority | 4 | 6 | 1 | 1 / 19 | yes+examples | spec/04 §5, spec/05 §3, spec/05 §4, spec/08 §2 |
| U3002 | authority | 2 | 1 | 1 | 1 / 11 | yes+examples | spec/05 §3, spec/05 §4 |
| U3003 | authority | 1 | 1 | 0 | 2 / 8 | yes+examples | spec/04 §2, spec/04 §5 |
| U3004 | authority | 1 | 0 | 1 | 1 / 3 | yes+examples | spec/04 §2 |
| U3005 | authority | 1 | 0 | 2 | 4 / 33 | yes+examples | spec/05 §6 |
| U3006 | authority | 2 | 0 | 4 | 1 / 7 | yes+examples | spec/05 §6, spec/10 §9 |
| U3007 | authority | 1 | 0 | 1 | 1 / 4 | yes+examples | spec/06 §3 |
| U3008 | authority | 1 | 0 | 1 | 1 / 5 | yes+examples | spec/06 §3 |
| U3009 | authority | 2 | 0 | 2 | 1 / 10 | yes+examples | spec/04 §2, spec/06 §2 |
| U3010 | authority | 2 | 1 | 1 | 1 / 10 | yes+examples | spec/05 §4, spec/05 §5 |
| U3011 | authority | 1 | 1 | 1 | 1 / 2 | yes+examples | spec/05 §5 |
| U3012 | authority | 2 | 1 | 2 | 2 / 7 | yes+examples | spec/04 §8, spec/05 §7 |
| U3013 | authority | 1 | 0 | 1 | 1 / 5 | yes+examples | spec/04 §4 |
| U3014 | authority | 1 | 4 | 0 | 1 / 2 | yes+examples | spec/08 §2 |
| U3015 | authority | 1 | 0 | 2 | 1 / 2 | yes+examples | spec/14 §4 |
| U4001 | security | 3 | 1 | 1 | 1 / 27 | yes+examples | spec/07 §3 |
| U4002 | security | 4 | 1 | 1 | 1 / 9 | yes+examples | spec/07 §3 |
| U4003 | security | 3 | 1 | 1 | 1 / 4 | yes+examples | spec/07 §3, spec/07 §4 |
| U4004 | security | 1 | 0 | 2 | 1 / 2 | yes+examples | spec/07 §6 |
| U4005 | security | 3 | 1 | 1 | 1 / 4 | yes+examples | spec/07 §5 |
| U4007 | security | 1 | 0 | 2 | 1 / 5 | yes+examples | spec/07 §2 |
| U5001 | verification | 10 | 11 | 1 | 8 / 14 | yes+examples | spec/08 §2, spec/12 §3, spec/12 §6 |
| U5002 | verification | 1 | 0 | 8 | 1 / 1 | yes+examples | spec/12 §6 |
| U5003 | verification | 1 | 0 | 8 | 1 / 2 | yes+examples | spec/12 §5 |
| U6001 | runtime | 1 | 0 | 1 | 1 / 3 | yes+examples | spec/04 §8 |
| U6003 | runtime | 1 | 0 | 6 | 1 / 1 | yes+examples | spec/11 §7 |
| U7001 | project | 13 | 6 | 7 | 2 / 4 | yes+examples | spec/01 §1, spec/01 §2, spec/01 §3, spec/01 §7, spec/02 §2, spec/02 §6, spec/04 §1, spec/04 §3 |
| U7002 | project | 3 | 0 | 0 | 4 / 18 | yes+examples | spec/13 §7 |
| U7003 | project | 1 | 1 | 0 | 2 / 5 | yes+examples | spec/06 §6 |
| U7004 | project | 2 | 0 | 8 | 2 / 7 | yes+examples | spec/12 §9 |
| U7005 | project | 2 | 0 | 2 | 2 / 5 | yes+examples | spec/07 §6 |
| U8001 | internal | 0 | 0 | 0 | 1 / 3 | yes (no examples) | - |

Registered codes: 65; conformance cases: 194 (approx, positive + failing)
Codes with no failing conformance case: U8001
Codes with no positive counterpart (same section): U1005, U1006, U1007, U1008, U2002, U2003, U2004, U2005, U2006, U2009, U2010, U2011, U2012, U2013, U2014, U2015, U3004, U3005, U3006, U3007, U3008, U3009, U3013, U3015, U4004, U4007, U5002, U5003, U6001, U6003, U7002, U7004, U7005, U8001
  of which also none in the same spec file: U1008, U2002, U2003, U2010, U2014, U2015, U7002, U8001
  (every explain file's fixed example is also a positive twin, checked by crates/una-cli/tests/explain_examples.rs)
Codes not mentioned in any crate test: none
Codes without an explain file: none
Unregistered codes in conformance expectations: none
Unregistered codes mentioned in crate tests: U4006, U9999
  U4006: crates/una-cli/tests/cli.rs, crates/una-cli/tests/conformance/directives.rs
  U9999: crates/una-cli/tests/cli.rs, crates/una-cli/tests/conformance/directives.rs

### Gaps

**No failing conformance case**: U8001 only. It is an internal error with no source-level trigger by design (ADR 0034, D-126). Its crate coverage is partial: `una-verify/src/solver.rs` unit-tests parsing of a solver `(error ...)` answer, and `explain_examples.rs` checks its text, but no test drives either U8001 path end to end (solver `(error ...)` to U8001, `spec/12 §5` rule 6; runtime fault to U8001 with exit code 3, `spec/11 §4`).

**No positive conformance twin in the same section** (34 codes): U1005, U1006, U1007, U1008, U2002, U2003, U2004, U2005, U2006, U2009, U2010, U2011, U2012, U2013, U2014, U2015, U3004, U3005, U3006, U3007, U3008, U3009, U3013, U3015, U4004, U4007, U5002, U5003, U6001, U6003, U7002, U7004, U7005, U8001.

- Of these, no positive case cites even the same spec file for: U1008, U2002, U2003, U2010, U2014, U2015, U7002, U8001.
- Mitigation: every one of them except U8001 has an explain fixed example that is checked to produce no diagnostic, and the crate tests in `una-check/tests/{names,types,authority,flow}.rs` mix accepted and rejected declarations in one source and assert the exact diagnostic list, so accepted forms are implicitly covered. The seven example packages (checked at tier 1 with every obligation proven, `examples.rs`) add broad positive evidence. The gap is in the conformance suite as a portable, spec-cited suite, not in the compiler's tests overall.
- U5003, U6001, U6003, U7002, U7004 are environment, runtime or project-file codes; their positive side is every passing case (a found solver, a `main` that returns, a valid manifest and lock). Not a real gap.

**Codes without explain file**: none. **Codes absent from crate tests**: none.

**Unregistered codes in tests**: U4006 and U9999 appear in `crates/una-cli/tests/cli.rs` and `crates/una-cli/tests/conformance/directives.rs`. Both are negative tests (the explain command and the directive parser reject them). Correct per `spec/13 §4` (gaps are unassigned numbers). Note only.

## 2. MUST rules and normative rejections

Method: `a reviewer script` extracted every sentence with MUST / MUST NOT / "is an error" / "rejects" (45 sentences; the spec states most rules as "... (Uxxxx)" parentheticals rather than MUST, so rules carrying a code were reviewed too). Priority files 05, 07, 10, 11, 12 were read in full; every rule was matched against conformance cases (by `-- spec:` section), crate tests (by assertion text) and, where no test exists, a probe. Rules found tested are not listed. "Untested" below means no conformance case and no crate test encodes the rule.

### spec/01, spec/02, spec/15
- No untested MUST. Nesting limit (`spec/02 §9`) is covered by `una-syntax/tests/robustness.rs` and `cli.rs` (`deep_nesting_is_a_diagnostic_even_on_a_small_stack`), not by a conformance case.

### spec/03
- spec/03 §3 / §6 with `spec/10 §2` S-Refine: "refinements ... at every subsumption point" for a refined type used as a **type argument** (`List[Seats]`, `Option[Small]`, `Box[Small]`). Untested, and not implemented: see R-012.

### spec/04
- spec/04 §2 rule 3: "Contracts express partial correctness" (a non-terminating function may prove any `ensures`). No test documents it. Minor.

### spec/05
- spec/05 §4 rule 3: "A `with` binding scopes over the evaluation of that statement's expression only." Untested; violated through generic laundering (R-002).
- spec/05 §5 rule 2: "They MUST satisfy the operation's `ensures` (proof obligation)." No failing test (only the greenhouse example proves one). A reviewer probe (handler `op next() = 0 - 5` against `ensures result > 0`) gives U5001: implemented.
- spec/05 §6: "A lambda that uses capabilities or can fail follows the same restrictions" (not bound with `let`, returned, stored in a kind). Tested for direct cases (`authority.rs`), untested and violated when the value passes through a generic function (R-002).
- spec/05 §9 rule 6: "A field of a kind whose type is such a function type is U3006": tested for a written field type, untested and not enforced for a generic field instantiated with such a type (R-002).
- spec/05 §9 rule 7 (function argument with extra `uses` is U3003): tested only through the authority tour's positive side; A reviewer probe confirms U3003. Minor.

### spec/06
- spec/06 §7: "refinement violations ... never occur at runtime." Untested; false for refined type arguments (R-012).

### spec/07
- spec/07 §1: "reported as such in the build report: timing, termination and divergence, resource consumption and allocation, interpreter stack exhaustion." The report (and `spec/13 §6`, ADR 0034) lists `timing, termination, resource consumption, external systems beyond declared signatures`; "interpreter stack exhaustion" is missing. Spec inconsistency (Finding 4).
- spec/07 §3 rule 4 / §7 rule 3 for calls of function-typed parameters and locals under a raised pc: no conformance case; crate test `lambdas_and_function_values_carry_labels` covers labels, and A reviewer probe confirms U4002 for an effectful and a failing function value under a `secret` condition. Minor.
- spec/07 §7 rule 4 "the result of an operation keeps its declared label" from a handler returning a captured secret: untested; A reviewer probe gives U4001. Minor.

### spec/08
- No untested MUST.

### spec/10
- spec/10 §2 S-Fun, refinement premises `τ̄2 <: τ̄1` and `τ_r <: τ_r'`: untested and not implemented (R-013; already the subject of ADR 0037, Proposed, on dev/v0.1 only).
- spec/10 §9 rule 2: "A lambda whose type has non-empty `uses` or `fails` occurs only directly as a call argument (U3006)." Violated through generics (R-002).
- spec/10 §10 T4 (capability confinement) and T5 (refinement soundness): stated targets, no test states them; probes show T4 fails (R-002) and T5 fails for type arguments (R-012).

### spec/11
- spec/11 §4: "A fault that a checked and verified program cannot reach aborts the run with U8001 and exit code 3." Untested (not reachable from source by design). Minor.
- spec/11 §5: redaction "inside any rendered value": list and option elements with labeled type arguments are not fields. Untested; A reviewer probe shows an argument `List[secret Int]` to an op is printed `<redacted:secret>`, and A reviewer probe shows a `main` failure payload with such fields prints `"fields":{}`. Behavior is safe; the spec wording does not say whether a field of type `List[secret Int]` counts as "labeled above public". Note.

### spec/12
- spec/12 §3: obligations for **opaque predicate bodies** and for **`do` lines of examples**: no failing test. A reviewer probe gives U5001 for `10 / x` in an opaque predicate and for `do half(-4)` against `requires n >= 0`: implemented.
- spec/12 §3 handler-operation obligations: see spec/05 §5 above.
- spec/12 §5 rule 4: wall-clock guard yields `unknown` with reason `wallclock`, marked non-deterministic. Untested.
- spec/12 §5 rule 6: "An `(error ...)` before the status means ... U8001, never a status." Only the answer parser is unit-tested (`solver.rs`); the mapping to U8001 is untested.
- spec/12 §9: "an unreadable entry is solved again, and `unknown` with reason `wallclock` is never cached." Untested (only "second run is cached" is tested).
- spec/12 §2: division by a negative literal (`x / -2`) is encoded as the uninterpreted `div!Int`, because `-2` is unary minus on a literal, so `x == -7 implies x / -2 == 4` is refuted . Sound, incomplete; spec says "nonzero literal divisors" without saying whether `-2` is one. Note.

### spec/13, spec/14
- No untested MUST found. `fmt --check`, Files prefix confinement and Keys loading are covered by `cli.rs` and `runtime.rs`.

