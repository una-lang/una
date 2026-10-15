# 0112. Eliding the run-time checks the verifier proves

- Status: Proposed
- Date: 2026-10-15
- Deciders:
- Resolves: issue #737
- Target: the release backend of ADR 0106 only. `una build` without `--release` (Cranelift, ADR 0055) keeps every check. No edition changes: elision is not observable for a correct proof (`spec/16 §4`).
- Amends, once accepted: `spec/16 §4` rule 2, the build report's `native` section (`spec/13 §6`), `docs/release-gates.md` §2.

## Context

The native backend keeps every run-time check of the interpreter's semantics. ADR 0106 rule RB-3 says a check leaves a release build only where the verifier proved it unneeded, before C is emitted and recorded in the build report, and names this decision as the place that says how. ADR 0107's Gate 2 table expects the largest remaining arithmetic cost to be the tagged `Int`: an overflow branch and a promotion path on every operation, up to about 1.3x on loops of unrefined `Int` arithmetic. ADR 0109 gives `systems` packages a proven `overflow` obligation on every `Int` operation, so that this decision can drop the promotion path there.

What the verifier proves today, and what the executable checks, are two different lists:

- **Every obligation of a built program is proven.** `una build` checks the package at tier 1 (`spec/16 §1`), and an obligation that is refuted or unknown is an error (`spec/12 §6`: U5001, U5002), so no executable exists while one is. A program's `division`, `overflow`, `precondition` and other obligations are therefore all proven when it is built.
- **Most run-time checks have no obligation behind them.** The executable's checks are:
  - **arithmetic:** the tag test and promotion path of each `Int` operation (`spec/16 §4` rule 2), which no obligation covers outside `systems` (ADR 0109); the overflow test of each fixed-width operation and narrowing conversion, whose `overflow` obligation is proven (`spec/18 §3`); and the zero-divisor and `MIN / -1` tests of division and remainder, whose `division` obligation is proven;
  - **memory safety:** the handle checks of ADR 0107 rule 2 (bit, index, layout and generation), the bounds of an inline element access (ADR 0107 rule 5) and the bounds that runtime functions check;
  - **resources and concurrency:** the call depth, the stack, the memory budget and cancellation (ADR 0102), which depend on run-time limits and other tasks, not on the program's values;
  - **control:** the abort after the last arm of a `when` (the `exhaustive` obligation, `spec/09`);
  - **escapes:** `checked`, the program's own run-time check (`spec/03`), counted by the escape budget.

The constraints are those of ADR 0106 and ADR 0107: soundness is never traded for speed; a verifier bug must stay a wrong answer and never become undefined behavior or a memory-safety bug; and native equals the interpreter.

## Decision

### 1. Scope (EL-1)

Elision happens in release builds only, in the C that ADR 0106 emits, before it is emitted (RB-3).

- `una build` without `--release` keeps every check, and so does the interpreter. A Cranelift executable is the fully checked reference of every release executable.
- `una build --release --no-elision` builds a release executable with every check, for audit and for the gates below. It writes the same report, with `elided` empty.

### 2. Which checks (EL-2)

A check may be elided only when, should the proof that elides it be wrong, the operation that remains is defined at every input and its effect is a wrong value: never undefined behavior, never a read or write out of bounds, never a lost failure of a resource or a capability. This is ADR 0106's rule RB-3a: a false proof of a bounds, handle or stack check would be a memory-safety bug, not a wrong answer, so those checks are never elided on the solver's word alone (§9). Two kinds meet it:

- **The `Int` promotion path.** For `+`, `-`, `*` and unary `-` on `Int` (and on a measured `Int`, ADR 0104), when the exact result is proven in the tagged range, -2^62 to 2^62 - 1, **and** each operand is proven in that range. The operand condition matters: an operand that came in large from an entry point (ADR 0109 rule 3) is held promoted, and a result proven small, as `x - x`, says nothing about it. The operation then becomes the machine operation on 64-bit words, in the unsigned form of RB-7, with the result retagged.
- **The fixed-width overflow test.** For an operation or a narrowing conversion on a fixed-width type whose `overflow` obligation is proven (`spec/18 §3`). The operation becomes the wrapping operation of its width, in the unsigned form of RB-7.

Every other check stays, for a stated reason:

- **Division and remainder** keep their zero-divisor and `MIN / -1` tests. In C, both cases are undefined behavior, so a wrong proof there would not stay a wrong value (RB-2); and the test is one predictable branch beside a division. The sign correction of the Euclidean remainder is not a check, and RB-7's unsigned form already removes it where the dividend is proven non-negative.
- **Memory-safety checks** (handles, inline bounds, runtime bounds) stay, always. They protect against bugs of the code generator and the runtime as much as against wrong values, which no proof about the source covers (ADR 0107 rule 2, Alternative A). So a verifier bug can never reach memory.
- **Resource and cancellation checks** stay. Their limits are run-time values (`--max-depth`, `--max-memory`, the stack budget) and other tasks' state, not facts about the program. Hoisting them to an earlier point would move where a run aborts, which is observable.
- **The abort after the last arm of a `when`** stays: it is cold, and its only removal form, `__builtin_unreachable`, is excluded by RB-7.
- **`checked`** stays: it is the program's own check, which the program asked for and the escape budget counts.

A later kind needs its own amendment of this record, which shows its remaining operation defined at every input and its effect, under a wrong proof, a wrong value only.

### 3. The proof behind each elision (EL-3)

Each elided check is tied to one proof, and only these count:

- **A proven tier-1 obligation at the site.** A fixed-width operation's `overflow` obligation; in a `systems` package, an `Int` operation's `overflow` obligation (ADR 0109 rule 3).
- **An elision query, for `Int` outside `systems`.** No obligation covers an `Int` operation of an `app` package. A release build generates, for each such operation, one query with the same encoding as a `systems` `overflow` obligation at that site, and one for each operand's range. These queries are **advisory**: a proven answer elides the check; a refuted, unknown or timed-out answer keeps it. They never give a diagnostic and never change what the package accepts. A debug build and `una check` do not generate them.
- **A refinement that bounds a value** (`spec/16 §4` rule 2) is how an operand is proven in range: it is part of the query's hypotheses, as for any obligation.

The check stays wherever the proof is missing: an unknown answer, a wall-clock timeout (`spec/12 §6`), a query outside the encoding, or an operand not proven in range. Nothing defaults to elision.

### 4. The proof cache (EL-4)

A cached answer counts as it does for every obligation (`spec/12`, ADR 0064): only with its HMAC tag verified under the user's key and its key computed from the current lock and the exact query text. An unreadable entry is solved again. `unknown` with reason `wallclock` is never cached, so a timeout never elides a check in a later build.

Elision depends only on the sources, `una.toml`, `verification.lock` and the solver's answers. A wall-clock timeout can make two builds of one commit differ: the one that timed out keeps the check. That difference is on the side of more checks, and the build report records it (`unknown: wallclock`), as tier 1 already does. The `reproducible` gate compares builds whose reports record no wall-clock answer.

### 5. The record (EL-5)

The build report's `native` section gains `elided`: one entry per elided check, with its site (`file:line:col`), its kind (`int-promotion` or `fixed-overflow`), and the proof that elided it: the obligation's id (`spec/13 §2`) or, for an elision query, its cache key, with the status `proven`.

The code generator writes the same set as it emits the C. Before the C compiler runs, `una build --release` compares the two: a check missing from the emitted C without an entry, or an entry for a check the C keeps, is an internal error, U8001, and no executable is written. So the report lists exactly the checks that are absent, never more and never fewer.

### 6. With ADR 0106 (EL-6)

- **Before the C.** A check is elided before C is emitted (RB-3). The C compiler is never told a fact it could use to delete another check.
- **Defined forms only.** The remaining operation uses RB-7's forms: unsigned arithmetic of the width, converted back. `__builtin_assume`, `__builtin_unreachable` and `restrict` stay excluded. A wrong proof gives a wrong value, which the equivalence gates below catch where they reach it.
- **No weakened neighbor.** An elision never changes another check's operands (RB-3): a bounds check, a division test or a conversion check on the result of an elided operation reads the same value it would read with the check present.
- **`check-survival`.** Its trigger programs fail by construction: each overflow, division or bounds trigger has a refuted obligation or an unproven elision query at its site, so nothing there is elided. The gate also reads each trigger's build report and requires `elided` to hold no entry at the trigger's site. Its canary (a check removed by hand) still makes it fail, with or without elision.

### 7. Trust (EL-7)

An elided check's guarantee rests on the verifier: the encoding of `spec/12 §2`, the generator of the query, the pinned solver of `verification.lock`, and the proof cache's integrity. For the elided checks, these join the trusted base. The build report says so: each entry of `native.elided` names the proof it rests on, and a non-empty `elided` means the executable trusts the verifier at those sites. `--no-elision` takes them out of the trusted base again.

The cost of a wrong proof is bounded by EL-2: a wrong value, the value a wrapping machine operation gives, where the interpreter would have promoted or aborted. Never undefined behavior and never a memory access out of bounds, since no memory-safety check is ever elided.

### 8. Gates and tests

- **`elision-equivalence`** (a release gate): every runnable program of the equivalence suite (ADR 0055 NB-4), built for release with and without elision, gives identical output, failures and trace, and the same as the interpreter.
- **`elision-record`** (a release gate): over the conformance suite and the examples, the checks absent from the emitted C equal the report's `elided`, and each entry's proof is `proven` in the report's obligations or in the cache. EL-5 already enforces it per build; the gate keeps the comparison running on every program.
- **Per kind:** for `int-promotion` and `fixed-overflow`, a program whose proof holds (the check is elided, and listed) and its twin whose proof fails (the check stays, and is not listed); and a program with an operand from an entry point, whose check stays though its result is proven in range.
- **A wrong proof stays defined:** a test build that marks one refuted `overflow` obligation proven builds an executable that computes the wrapped value, and the `sanitize` gate's flags report nothing on it.
- **Determinism:** two release builds of one commit with a warm cache give the same `elided` and the same bytes.
- **Scaling:** a module with 10, 100 and 1,000 `Int` operations: the number of elision queries is linear, and a rebuild with a warm cache solves none.

### 9. What would unlock bounds elision later (#66)

Bounds checks are the next candidates: an index proven below a length. RB-3a keeps them until a proof carries an unsat certificate that a small kernel checks (#66, "proof certificates checked by a small kernel"). That decision is not taken here. For bounds elision, it would need at least:

- **A certificate per elided bounds check.** The solver produces a proof object for the unsat answer of the exact query, in a format a small checker can read (Alethe or LFSC, as cvc5 emits them, or Z3's proof terms after a translation). The release build stores it next to the record entry of EL-5.
- **A small, separate kernel.** A checker of the certificate format, small enough to review in full and with no dependency on the solver, in the trusted base in place of the solver. Without a certificate, or with one it rejects, the check stays.
- **A trusted link from the site to the query.** The certificate proves the query unsat; something must also show that the query states the site's check, `0 <= i and i < length(xs)` under the path's facts. That link is the encoding of `spec/12 §2`. Either it stays trusted, and is named so, or the kernel re-derives the bounds goal from the Core IR site in a form small enough to review.
- **Handles stay.** A handle check (ADR 0107 rule 2) guards against bugs of the code generator and the runtime, which a proof about the source cannot cover. A certificate does not unlock it.
- **Stack and resource checks stay.** Their limits are run-time values, not facts of the program.

Each of these would be its own amendment of this record, after #66's decision.

### 10. Complexity

One elision query per `Int` operation of an `app` package and one per operand, in release builds only: linear in the size of the program, each a quantifier-free query of the size of a fixed-width `overflow` query. The proof cache makes a rebuild cost one lookup per query. The record's comparison is linear in the number of checks. No pass becomes quadratic.

## Consequences

- Arithmetic whose range the verifier proves runs as machine arithmetic in release builds: the `Int` promotion path and the fixed-width overflow branch leave exactly the sites where a proof holds. This is the remaining arithmetic cost of ADR 0107's Gate 2 table. Its effect on the bar is an estimate until the implementing pull request measures it with the kernels of ADR 0106 and ADR 0107.
- The verifier joins the trusted base of release builds for the elided checks, recorded in the report and removable with `--no-elision`.
- Release builds of `app` packages run more queries; the cache bounds the cost of rebuilds.
- Memory-safety, resource, cancellation and division checks stay in every build. Removing any of them needs a new decision that meets EL-2.

## Alternatives considered

**A. Elide every check that a proven obligation covers, memory checks included.** More speed on bounds-heavy code. Not chosen: a bounds or handle check guards memory against bugs of the code generator and the runtime too, which no proof about the source covers, and with it gone a verifier bug becomes a memory-safety bug (ADR 0107 rule 2).

**B. Elide in debug builds as well.** Not chosen: the Cranelift build is the fully checked reference, and keeping it whole makes every release build comparable to it.

**C. Tell the C compiler the proven facts with `__builtin_assume` instead of removing checks.** Not chosen: RB-7 of ADR 0106 excludes forms whose false premise is undefined behavior.

**D. Make an `Int` overflow an obligation in every package.** Not chosen here: an unprovable obligation would reject programs that run today. That is the `systems` profile's choice (ADR 0109), which a package makes; this decision keeps `app` acceptance unchanged, with advisory queries.

**E. Hoist resource checks out of loops.** Not chosen: it moves the point where a run aborts on a limit, which is observable, and the limits are run-time values.

**F. Elide the division tests where the divisor is proven nonzero.** Not chosen: in C, a zero divisor or `MIN / -1` is undefined behavior, so a wrong proof would not stay a wrong value; the test is one predictable branch.

**G. Elide bounds checks on a proven index now.** Not chosen: under RB-3a, a false unsat for a bounds goal is a read or write out of bounds, and nothing checks the solver's answer today. §9 states what #66 would need.
