# Pre-tag review of v0.2.0: summary

- **Candidate:** `ef34f2f604657572da0c8ec97612f2e923549ef8`, `main` after #255. `main` is frozen: only fixes of this review's findings merge, then #230 (version and CHANGELOG).
- **Date:** 2026-10-06. Time-box: one day. Rechecks cover the delta only.
- **Lead:** Una-1 (Section 1, area 3d, Section 4, this summary). Areas: 3a (Coordenador's reviewer agent), 3b (Una-2), 3c and 3e (Una-3).
- **Severity:**
  - **Critical:** unsound, a native divergence the spec does not allow, or a failing release gate.
  - **Major:** a diagnostic or repair that changes meaning, lost trace or exit parity, or a failing docs example.
  - **Minor:** everything else.
  - The verdict is Not ready while any Critical or Major is open. Linux is a hard gate.
- **Decisions:** Coordenador, under Leonardo's delegation, N-03 to N-10, logged for Leonardo.

## Verdict

**Not ready on the candidate.** One Critical is open: 3a-1, #267. Una-2 is fixing it, and its delta check is pending.

The tag needs all of these, in order:
1. #267 merged, with a Clean delta check from the 3a reviewer.
2. #257 (3b Majors) and #266 (#258, docs) merged.
3. #230 merged, with the CHANGELOG items listed below.
4. Every gate PASS on the tag SHA, recorded as `docs/releases/0.2.0-validation.md`.

No information-flow leak was found. Edition 0.1 isolation holds.

| Area | Owner | Result | Open Critical/Major |
| --- | --- | --- | --- |
| 1. Release gates | Una-1 | See Section 1 | see Section 1 |
| 3a. Labels and information flow | Coordenador's reviewer | **Not clean**: no leak; 1 Critical in native failure handling | 3a-1 (#267) |
| 3b. Diagnostics and repairs | Una-2 | No meaning change; 2 Major in batch application | F3, F5 (#257) |
| 3c. Runtime and CLI streams | Una-3 | Engines agree; EBADF treated as success | none (3c-1 Minor, after the tag) |
| 3d. #200 and #201 fixes | Una-1 | **Clean** for soundness and parity; 2 Minor performance findings | none |
| 3e. Edition 0.1 isolation | Una-3 | Gate 1 references identical to v0.1.3; one message regression | none (3e-1 fixed, #264) |
| 4. Docs and release consistency | Una-1 | 3 Major, 8 Minor | S4-01 (#266); S4-02 and S4-03 (#230) |

## 1. Release gates

`scripts/release-gates.sh ef34f2f604657572da0c8ec97612f2e923549ef8 OUT`, with the pinned Z3.

| Gate | Result | Seconds |
| --- | --- | --- |
| clone | PASS | 3 |
| build | PASS | 301 |
| mac | PASS | 145 |
| linux | running | |
| release | pending | |
| determinism | pending | |
| native | pending | |

The candidate is superseded by the review fixes, so this run is a baseline. The run that counts is the final one on the tag SHA, committed as `docs/releases/0.2.0-validation.md`.

The `native` gate's equivalence test did not catch 3a-1, because no program in the repository calls a `fails of` parameter under `if error pass`. #267 should add the repro to a corpus that the test runs.

## 3a. Labels and information flow (Coordenador's reviewer agent)

**Not clean, on one Critical.** No information-flow leak was found.

What it covered:
- **Prior projects:** 1,742 rerun. Every real leak is rejected.
  - The only accepted ones are the known non-leaks L20b, N07e, c42 and a25.
  - Also accepted is a52, which ADR 0067 now correctly accepts as a public length.
- **New cases:**
  - 109 new cross-feature leak/control pairs, all correct;
  - 32 more leaks, all rejected;
  - 107 native parity programs, byte-identical.
- **Edition 0.1:** 318 new twins are identical to v0.1.3.
  - The only prior differences are #253's T-App rejections.
  - Among them, G14 is a real leak that is now rejected.

The finding:
- **3a-1 (Critical, #267, N-10).** Natively, a call of a `fails of` parameter under `if error pass` swallows the failure.
  - The repro prints `100` natively and `7` in `una run`.
  - With `attempt`, native gives `Ok(0)` and the interpreter gives `Err(Bad)`.
  - It is a native divergence the spec does not allow.
  - Una-2 is fixing it, and the 3a reviewer rechecks the delta.

## 3b. Diagnostics and repairs (Una-2)

Full write-up: `3b-diagnostics-and-repairs.md`, with 61 case packages, before and after, in `cases/3b/`.

No applied repair changed meaning: every repaired program that checks ran with the intended output. Filed as #257, P0, and fixed by Una-2 before the tag (N-04):

| Id | Severity | Finding |
| --- | --- | --- |
| F3 | Major | #206: a `let result` inside a `together` task is renamed only to the end of the task. Task lets are visible after `together`, so a later `{result}` becomes U2012. |
| F5 | Major | Several U2026 on one type parameter each edit the same `T` span. Applied in a batch, they give `[T: Ord: Eq]`, which is U0101. |
| F1 | Minor | `requires p != Over` gets U2014 and U2009 with an identical edit, which a batch applies twice. |
| F4 | Minor | #211: two clauses on one line move onto one new line, and the second does not converge. |
| F2 | Minor | #206: two `let result` in one scope both become `outcome` (U1002). |
| N1, N2 | Note | The D8 note names the first labeled field, and the outer field rather than the nested one. |

These hold in both editions:
- #205 across precedence, `not`, either side, fields, calls, lambdas, paren-`if` and `requires`;
- #206 renames, with `outcome2` on collision;
- #211 for `let`, nested code, `if none`, an operation statement and `together`;
- U2026 for a single need;
- D8, which is advice only.

## 3c and 3e. Runtime streams and edition 0.1 isolation (Una-3)

Full write-up: `una-3-3e-3c-dogfood.md`, with the scaling probe `scale.py`.

- **3e, isolation:**
  - The Gate 1 references (150 packages) are identical to v0.1.3.
  - In the edition 0.1 conformance diff, the expected differences are #205, #206, #211 and #253.
  - **3e-1 (Minor, N-03):** #36 changed U3004's message in edition 0.1. Fixed by #256 (PR #264, merged).
    - The delta check passed: 960 of 964 targets are byte-identical to the candidate, and the 4 that differ are the expected U3004 cases.
    - Against v0.1.3, 3 of those cases are back byte for byte, and the fourth adds only the edition 0.2 note.
- **3c, streams:**
  - Exit codes, stdout and traces agree in every stream combination.
  - **3c-1 (Minor, N-06, #259 after the tag):** a closed stdout or stderr (EBADF) is treated as success, though spec/13 §5 says a failed write exits 2. It is a known issue in the CHANGELOG.
  - **3c-2 (N-06):** the native executable renders U6001 compactly, with the same first line, code, position and failure. This is allowed and documented in spec/16 §3 by #266.
- **Native dogfood:** 51 of 51 identical.
- **Scaling:** linear, with two exceptions:
  - **S-1 (N-07):** a loop forgets the length of a list it updates, so `for i in 0..<xs.len` with `set` is refuted. The guide documents it (#266), and #260 and ADR 0071 fix it after the tag.
  - **S-2 (N-08):** text concatenation in a loop is quadratic. #261, under ADR 0070 X10, fixes it after the tag.

## 3d. The fixes of #200 and #201 (Una-1)

Case packages and write-ups: `cases/3d/` (`README-200.md`, `README-201.md`, `tools-200/invariants.py`).

**#200, PR #213: Clean.** The rule is spec/19 §6 rule 4 and §7 rule 3: an operation in progress when its task is cancelled finishes and keeps its own outcome.
- **Original repros:** `equiv/togcancel` and `tasks/recheck_nested_cancel` ran 20 times each natively.
- **New probes:** 8 probes ran 20 times each natively. In each, task 1 fails while task 2 is inside a handler operation. The operations cover:
  - several calls and a recursion;
  - a nested handler;
  - a print;
  - an operation that fails;
  - a nested `together`, including one whose own task is cancelled;
  - a `together for` of 64 tasks;
  - 300 tasks.
- **Results:**
  - No `aborted` event appeared.
  - Each `cancelled` event is its task's last.
  - Status and output match the interpreter in every run.
  - In ordered runs, without cancellation, the traces are byte-identical to the interpreter's.
- **Not covered:** a host operation that really blocks.

**#201, PR #221: Clean for soundness.**
- **The in-place update conditions:** an update happens in place only when `Rc::make_mut` holds the only reference. A binding is dropped only when `dead_after` holds, and `vars` over-approximates what an expression reads.
- **Aliasing:** no alias observed a change. The checks covered lambdas, records, `Option`, nested lists, `edit` callers, a failing `edit` callee, loop state, `together` and `together for`.
- **Parity and memory:** output and traces match between engines, and the memory peak was 215 MiB.
- **Minor findings, filed after the tag with Una-2 (N-09):**
  - **3d-1 (#263, P1):** in the interpreter, a loop that appends to a list it binds is quadratic when it is nested in another loop or in a `together` task. Native code is linear. For 30,000 appends, the interpreter takes 10.9 s and native takes 0.4 s. The cause is that the enclosing `later` entry covers the inner loop's state.
  - **3d-2 (#262, P0):** `grow(edit acc)` or `acc = push(acc, ...)` in a loop is quadratic in both engines, at about 30 s for 30,000 appends. The caller still holds the list during the call. This predates #201.

| Pattern | n | Interpreter | Native |
| --- | --- | --- | --- |
| append in a loop | 10^5 / 10^6 | 0.30 s / 3.5 s | 0.48 s / 1.8 s |
| `set` in a loop | 10^6 | 10.0 s | 6.7 s |
| inner loop appends to the outer state | 10^4 / 3 × 10^4 | 1.5 s / 10.9 s | 0.7 s / 0.4 s |
| loop in a `together for` task | 10^4 / 3 × 10^4 | 4.4 s / 21.2 s | 1.8 s / 0.4 s |
| `grow(edit acc)` | 10^4 / 3 × 10^4 | 1.5 s / 29 s | 3.7 s / 32 s |

## 4. Docs and release consistency (Una-1)

Full write-up: `section4.md`.

| Id | Severity | Finding | Resolution |
| --- | --- | --- | --- |
| S4-01 | Major | spec/21 (Map) and spec/22 (Text) and their cross-references read as normative for 0.2, but v0.2.0 does not implement them | #258, PR #266 |
| S4-02 | Major | #230 bumps `Cargo.toml` but not `install.sh`'s `DEFAULT_VERSION`, and `tests/install.rs` would fail on the tag SHA | #230 (Una-2) |
| S4-03 | Major | #230's CHANGELOG calls #253 pending | #230 |
| S4-04 to S4-06 | Minor | The CHANGELOG has no list of 0.1 changes, no installer or gates entries, and its ADR list stops at 0068 | #230 |
| S4-07 | Minor | The guide says `una run` takes `--trace` | PR #266 |
| S4-08 | Minor | `install.sh` calls `una --version` | #230 |
| S4-09 | Minor | The README release table and CI paragraph are stale | PR #266 (the status paragraph in #230) |
| S4-10, S4-11 | Minor | The backlog's v0.2 view; D-133 to D-159 unanswered | Left for Leonardo |

What is consistent:
- ADRs 0055 to 0070 are all Accepted.
- The 84 codes in spec/13 match the compiler.
- `una explain` covers every assigned code.
- Every CLI flag the docs name exists.
- The guide and README examples run in the `build` gate, which passed.

## Items for #230's CHANGELOG

Sent to Una-2:
- **Edition 0.1 changes:** #205, #206, #211, #253 (a new rejection) and #256 (the U3004 message).
- **Known limits of v0.2.0:**
  - S-1 (#260);
  - S-2 (#261);
  - 3d-2 (#262);
  - 3d-1 (#263);
  - 3c-1 (#259).
- **ADRs 0069 and 0070:** accepted, not implemented.
- **Also:** the installer and release gates PRs, the version in `install.sh`, and #253 marked fixed.

## Remaining before the tag

- [ ] #267 merged, with the 3a delta check Clean.
- [ ] #257 merged, with the 3b delta check.
- [ ] #266 merged.
- [ ] #230 rebased onto the final `main`, with the items above, and merged.
- [ ] `scripts/release-gates.sh <tag SHA>` all PASS, and the record committed as `docs/releases/0.2.0-validation.md`.
- [ ] Leonardo's word for the tag.
