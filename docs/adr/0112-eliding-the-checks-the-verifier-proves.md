# 0112. Eliding the run-time checks the verifier proves

- Status: Accepted
- Date: 2026-10-15
- Deciders: Una project (accepted on 2026-10-15)
- Resolves: issue #737
- Target: the release backend of ADR 0106 only. `una build` without `--release` (Cranelift, ADR 0055) keeps every check. No edition changes: elision is not observable for a correct proof (`spec/16 §4`).
- Amends: `spec/16 §4` rule 2, the build report's `native` section (`spec/13 §6`), `docs/release-gates.md` §2.

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
- `una build --release --no-elision` builds a release executable with every check, for audit and for the gates below. It runs no elision query, so it depends on the solver no more than tier 1 does, and its report has `elided` empty. With and without elision, a program gives the same output, failures, trace and exit code (the `elision-equivalence` gate); the executables and the reports differ, at least in `elided`.

### 2. Which checks (EL-2)

A check may be elided only when, should the proof that elides it be wrong, the operation that remains is defined at every input and its effect is a wrong value: never undefined behavior, never a read or write out of bounds, never a lost failure of a resource or a capability. This is ADR 0106's rule RB-3a: a false proof of a bounds, handle or stack check would be a memory-safety bug, not a wrong answer, so those checks are never elided on the solver's word alone (§9). Two kinds meet it:

- **The `Int` promotion path.** For `+`, `-`, `*` and unary `-` on `Int` (and on a measured `Int`, ADR 0104), when the exact result is proven in the tagged range, -2^62 to 2^62 - 1, **and** each operand is proven in that range. The operand condition matters: an operand that came in large from an entry point (ADR 0109 rule 3) is held promoted, and a result proven small, as `x - x`, says nothing about it. The operation then becomes the machine operation on the untagged 64-bit values, in the unsigned form of RB-7, and the result is retagged as `((uint64_t)r << 1) | 1`, an unsigned shift. The elided path never retains or releases an operand, so a wrong operand proof can at most leak a promoted value, never free one twice.
- **The canonical representation (EL-2a).** A proof about an operand's value says something about its representation only if every `Int` whose value lies in the tagged range is held tagged. So the runtime keeps that invariant: every arbitrary-precision result is normalized to the tagged form when it fits, and so is every value that enters at an entry point (`to_int`, `parse_int`, JSON's `Number`, a host operation; ADR 0109 rule 3). A debug build asserts it at each of them, and a test feeds a small value through each entry point and checks that it arrives tagged. Elision is not enabled before this invariant is enforced.
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
- **An elision query, for `Int` outside `systems`.** No obligation covers an `Int` operation of an `app` package. A release build generates, for each such operation, one query with the same encoding as a `systems` `overflow` obligation at that site, and one for each operand's range. These queries are **advisory**: a proven answer elides the check; a refuted or unknown answer keeps it. They never give a diagnostic and never change what the package accepts:
  - a malformed elision query, an encoder error on one, or a solver crash keeps the check, is counted in the report (`native.elision_errors`), and never fails a build that would otherwise succeed;
  - they appear nowhere in the report's obligations; an elision query shows only as the cache key of an `elided` entry.

  A debug build, `una check` and `--no-elision` do not generate them.
- **A refinement that bounds a value** (`spec/16 §4` rule 2) is how an operand is proven in range: it is part of the query's hypotheses, as for any obligation.

The check stays wherever the proof is missing: an unknown answer, a query outside the encoding, or an operand not proven in range. Nothing defaults to elision.

### 4. The proof cache (EL-4)

A cached answer counts as it does for every obligation (`spec/12`, ADR 0064): only with its HMAC tag verified under the user's key and its key computed from the current lock and the exact query text. An unreadable entry is solved again.

**Deterministic limits.** An elision query runs under the solver's deterministic resource limit, the `step_budget` of `verification.lock` (`(set-option :rlimit …)`, `spec/12 §6`), so the same inputs always give the same answer and the same elided set. The wall-clock guard stays only as an outer safety net: an elision query that hits it fails the release build (U8001, "the solver hit its wall-clock guard on an elision query; build again, or with `--no-elision`"), never changing what the build emits. So elision depends only on the sources, `una.toml`, `verification.lock` and the solver, and two release builds of one commit give the same `elided` and the same bytes, with no exception in the `reproducible` gate.

### 5. The record (EL-5)

The build report's `native` section gains `elided`: one entry per elided check, with its site (`file:line:col`), its kind (`int-promotion` or `fixed-overflow`), and the proof that elided it: the obligation's id (`spec/13 §2`) or, for an elision query, its cache key, with the status `proven`.

Before the C compiler runs, `una build --release` compares three sets, each computed independently of the others:

1. **The required checks:** every check site that Core IR calls for, enumerated by a pass over Core IR that is separate from the emitter;
2. **The present checks:** the checks the emitted C contains, read back from the C itself, where the emitter writes each check through a marker that names its site (a named check macro or call per site);
3. **The elided checks:** `native.elided`.

It requires the first to equal the union of the other two, and the other two to be disjoint. Anything else, a check the emitter dropped by a bug as much as an elision without an entry, is U8001, and no executable is written. No flag, environment variable or build mode skips this comparison, `--no-elision` included, where the third set is empty. So the report lists exactly the checks that are absent, never more and never fewer.

### 6. With ADR 0106 (EL-6)

- **Before the C.** A check is elided before C is emitted (RB-3). The C compiler is never told a fact it could use to delete another check.
- **Defined forms only.** The remaining operation uses RB-7's forms: unsigned arithmetic of the width, converted back. `__builtin_assume`, `__builtin_unreachable` and `restrict` stay excluded. A wrong proof gives a wrong value, which the equivalence gates below catch where they reach it.
- **No weakened neighbor.** An elision never changes another check's operands (RB-3): a bounds check, a division test or a conversion check on the result of an elided operation reads the same value it would read with the check present.
- **`check-survival`.** Its triggers are of two sorts:
  - **Una programs** for the `Int` promotion path: an `app` package whose elision query at the trigger's site is unproven, so the check is kept and fires.
  - **Harness builds** for the fixed-width overflow and division kinds: an accepted program can never reach them, since their obligations are proven or the program is not built, so, like the stale-handle kind (ADR 0106, G2), the trigger is a test build that keeps the check present and feeds the failing values directly, showing that it fires in the shipped configuration.

  The gate reads each trigger's build report and requires `elided` to hold no entry at the trigger's site. Its canary (a check removed by hand) still makes it fail, with or without elision.

### 7. Trust (EL-7)

An elided check's guarantee rests on the verifier: the encoding of `spec/12 §2`, the generator of the query, the pinned solver of `verification.lock`, and the proof cache's integrity. For the elided checks, these join the trusted base. The build report says so: each entry of `native.elided` names the proof it rests on, and a non-empty `elided` means the executable trusts the verifier at those sites. `--no-elision` takes them out of the trusted base again.

The cost of a wrong proof is bounded by EL-2: a wrong value, the value a wrapping machine operation gives, where the interpreter would have promoted or aborted. Never undefined behavior and never a memory access out of bounds, since no memory-safety check is ever elided.

### 8. Gates and tests

- **`elision-equivalence`** (a release gate): every runnable program of the equivalence suite (ADR 0055 NB-4), built for release with and without elision, gives identical output, failures and trace, and the same as the interpreter.
- **`elision-record`** (a release gate): over the conformance suite and the examples, EL-5's three-set comparison holds, and each entry's proof is `proven` in the report's obligations or in the cache. Its canary: a test build of the emitter that drops one check without recording it must give U8001.
- **Per kind:** for `int-promotion` and `fixed-overflow`, a program whose proof holds (the check is elided, and listed) and its twin whose proof fails (the check stays, and is not listed); and a program with an operand from an entry point, whose check stays though its result is proven in range.
- **A wrong proof stays defined:** a test build that marks one refuted `overflow` obligation proven builds an executable that computes the wrapped value, and the `sanitize` gate's flags report nothing on it.
- **Advisory failures:** a forced malformed elision query gives a successful build with that check kept and counted in `elision_errors`.
- **The canonical representation:** a small value through each entry point arrives tagged (EL-2a).
- **Determinism:** two release builds of one commit, with a cold cache and with a warm one, give the same `elided` and the same bytes.
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

## Amendment (2026-10-31): check elision without emitted C (#1309)

ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend. The release path of `una build --release` becomes Una's own optimizing code generation over Core IR, inside `una`, under that amendment's requirements RP-1 to RP-10, and its design is a separate record (#1305). This record was written against the emitted C: where elision happens, how the present checks are read back, and some of its reasons, are stated in terms of C. This amendment restates each of those parts for the release path. It changes no rule on which checks may be elided, and it chooses no representation, pass or algorithm of the code generator; those are the design record's (ADR 0106's amendment, OQ-1 to OQ-4).

### What stands unchanged

- **EL-2, EL-2a and RB-3a.** Only the `Int` promotion path (result and operands proven in the tagged range, on the canonical representation of EL-2a) and the fixed-width overflow test (its `overflow` obligation proven) may be elided. Division and remainder tests, memory-safety checks (handles, inline bounds, runtime bounds), resource and cancellation checks, the abort after the last arm of a `when`, and `checked` always stay. A later kind still needs its own amendment of this record.
- **EL-3, EL-4 and EL-7:** the proofs that count, the advisory elision queries, the proof cache and its deterministic limits, and the trust each elision rests on.
- **`una build --release --no-elision`:** a release executable with every check, built by the same release path, running no elision query, with `elided` empty. The `elision-equivalence` gate keeps its meaning: with and without elision, a program gives the same output, failures, trace and exit code, and the same as the interpreter.
- **§9** (what would unlock bounds elision) and **§10** (complexity).

### Terms

In this amendment, the **release representation** is any representation of a package that the release path builds between Core IR and the machine code it writes, whatever the design record makes it (one or several, or none beyond Core IR). The **machine code** is the object the release path writes, before linking. Every operation of each is defined at every input (RP-5).

### Restatements, by section

1. **Target (the record's header) and Context.** "The release backend of ADR 0106" reads as the release path of ADR 0106's amendment, §3. "Before C is emitted" reads as "before machine code is generated", as ADR 0106's amendment, §2, states RB-3: no optimization removes or weakens a check that the Cranelift build makes, except where this record elides it on a proof, before code generation, recorded in the build report.

2. **EL-1, where elision happens.** Elision happens in release builds only, before machine code is generated, at a point of the release path where every check still carries the Core IR site it came from: on Core IR itself or on the release representation. Which of these is the design record's choice. Wherever it is applied, each elision is decided by EL-2 and EL-3 for one Core IR check site, and is the one entry of `native.elided` for that site. `una build` without `--release` and the interpreter keep every check, as before.

3. **EL-2, the remaining operation (RB-7's forms).** Where the record says "in the unsigned form of RB-7", it means an operation that ADR 0106's amendment keeps under RB-7 and RP-5: defined at every input, with a wrong proof giving a wrong value.
   - **The `Int` promotion path.** The remaining operation is the 64-bit operation modulo 2^64 on the untagged two's-complement values (wrapping addition, subtraction, multiplication and negation), and the result is retagged by a logical left shift by one of its 64-bit pattern, with bit 0 set. Both are defined for every input, in every representation of the release path; neither has a case that a later pass may treat as impossible. The text `((uint64_t)r << 1) | 1` states this retagging in C, and is read so.
   - **The fixed-width overflow test.** The remaining operation is the wrapping operation of the type's width (modulo 2^w, read back in the type's signedness), and a narrowing conversion is the truncation to the width's bit pattern, read back in the target type.
   - The elided path still never retains or releases an operand.

4. **EL-2, the reason division and remainder keep their tests.** The rule is unchanged: the zero-divisor and `MIN / -1` tests stay, and Alternative F stays rejected. The reason "in C, both cases are undefined behavior" is restated: under RP-5, no representation of the release path has an operation that is undefined at some input, so a division without its tests would have to define both cases itself; and the machine instructions do not give a wrong value for them on every target: on x86-64, `idiv` raises a divide-error exception for a zero divisor and for `MIN / -1`, which ends the process with neither a wrong value nor the interpreter's failure (on AArch64, `sdiv` returns 0 and `MIN` without a trap). A wrong proof there would therefore not stay a wrong value, which EL-2 requires, and the test costs one predictable branch beside a division. The sign correction of the Euclidean remainder is not a check: RB-7's defined form (the unsigned remainder, where the dividend is proven non-negative, as ADR 0106's measurement shows) removes it there.

5. **EL-2, the abort after the last arm of a `when`.** The rule is unchanged: the abort stays. The reason "its only removal form, `__builtin_unreachable`, is excluded by RB-7" reads: its only removal form would be an "unreachable" construct, which RP-5 excludes from every representation of the release path.

6. **EL-5, the record and the three sets.** "Before the C compiler runs" reads as "before the release path writes the executable". The three sets stay as written, each computed independently of the others:
   1. **The required checks:** every check site that Core IR calls for, enumerated by a pass over Core IR that is separate from the passes of the release path that place, move or remove checks.
   2. **The present checks:** the checks that the code the release path generates contains, read back from that code: from the release representation at the last point where each check is still identifiable by its site, or from the machine code. They are read by a pass separate from the code generator's passes, never from the code generator's own account of what it emitted. Each check carries what names its site down to that point, as the emitter's marker did in the C. How a check carries its site, and at which point the set is read, is the design record's choice; any pass after that point MUST be one that cannot remove or merge a check (RP-4), and `check-survival` and its canaries cover it.
   3. **The elided checks:** `native.elided`.

   The comparison is unchanged: the first set equals the union of the other two, and the other two are disjoint; anything else is U8001 and no executable is written; no flag, environment variable or build mode skips it, `--no-elision` included. If the design record lets a pass remove or hoist a check under a rule of its own (ADR 0106's amendment, RP-4 and OQ-4), each such removal is listed in the build report with its rule and its site, and counts in the comparison as a fourth set, disjoint from the others and never part of `native.elided`; until the design states such a rule, there is none, and the comparison is the three sets above. So the report still lists exactly the checks that are absent.

7. **EL-6, with the release path.**
   - **"Before the C"** reads: an elision removes a check and adds no fact. No pass of the release path receives the proof behind an elision as a premise (an assumed range, a failure path marked impossible) that it could use to remove, move or weaken another check (RP-4, RP-5). A proven fact that the release path uses for speed is used only under RB-7, in a defined form, recorded with its obligation (RP-10).
   - **"Defined forms only"** reads: the remaining operation uses the forms of item 3. `__builtin_assume`, `__builtin_unreachable` and `restrict` read as any construct of the release path whose false premise has no defined meaning: an assumption, an "unreachable" mark, a poison-like value, or a promise that two accesses do not overlap. RP-5 excludes them all.
   - **"No weakened neighbor"** stands as written.
   - **`check-survival`.** Its Una-program triggers stand as written. Its harness builds, for the fixed-width overflow and division kinds, are test builds of the release path itself that keep the check present and feed the failing values directly, showing that the check fires with the shipped release path and its shipped settings; no C harness is involved. The stale-handle trigger is ADR 0106's amendment's (#1307).

8. **§8, gates and tests.**
   - **`elision-record`:** its canary is a test build of the code generator (in the record, "the emitter") that drops one check without recording it, and it must give U8001.
   - **A wrong proof stays defined:** the test build that marks one refuted `overflow` obligation proven builds an executable that computes the wrapped value of item 3, equal to that value as the interpreter's model of the width computes it. The record's "the `sanitize` gate's flags report nothing on it" referred to the sanitizer run over the emitted C, which ADR 0106's amendment withdraws; it reads as the evidence that replaces that run (ADR 0106's amendment, OQ-8, #1307), which MUST report nothing on this executable once it exists.
   - The other gates and tests stand as written.

9. **Alternative C.** "Tell the C compiler the proven facts with `__builtin_assume`" reads as: give the release path's optimizer a proven fact as an assumption whose false premise has no defined meaning. It stays rejected, by RB-7 and RP-5.

10. **Consequences.** "The kernels of ADR 0106 and ADR 0107" are measured in Una, through the release path (#1306); their C versions remain measurement records that `una` never builds.

### Effect

With this amendment, no rule of this record depends on emitted C. It is implementable with the release path, once the design record (#1305) is accepted, and it binds that record: the design states where elision is applied (item 2), how the present checks are read (item 6), and how each check carries its site.

## Amendment (2026-11-25): the range facts of the Cranelift build (EL-1, #1986)

- Status: Proposed (2026-11-25), for the Owner's decision
- Lane: S (it states a trust boundary between the verifier and the code generator)
- Changes no accepted sentence of this record. It states how one existing part of the Cranelift build relates to EL-1, from the code on `main` at commit `540ea4c85`, and proposes a reading of EL-1 and one follow-up for the Owner to accept or reject.

### Context

EL-1 says that `una build` without `--release` (Cranelift, ADR 0055) "keeps every check", and that a Cranelift executable is "the fully checked reference of every release executable". The Context of this record lists, among the executable's checks, "the tag test and promotion path of each `Int` operation (`spec/16 §4` rule 2)".

The Cranelift build already leaves that tag test and promotion path out at some sites, and has since #116 (2026-10-02), before this record. It does so under ADR 0055 NB-3 ("a value whose range a refinement proves MAY use a plain machine integer with no check"), `spec/16 §4` rule 2 and `spec/18 §7`. The code is `crates/una-codegen/src/range.rs` (the range facts) and its two uses in `crates/una-codegen/src/lower.rs` (the lowering of a primitive operation, `Rhs::Prim`, and `Lowerer::prim`). Read literally, EL-1 does not describe this build. This amendment says what the build actually does, and what each fact rests on.

### What range.rs computes

`range.rs` exports three functions:

- `proven_range(ty)` gives the closed interval that values of `ty` lie in, when it is bounded on both sides. It starts from the base type's range (a fixed-width type's least and greatest values; for `Int`, unbounded) and intersects it with the bounds of each refinement of `ty`. `bounds` reads only syntactic forms over `it`: `it in a..b` (with `..<`), `it op c` and `c op it` for `>=`, `>`, `<=`, `<` and `==` with an integer literal `c`, and `and` of those. Any other term (`or`, `!=`, a variable, a call, a literal that does not parse) gives no bound. An empty interval gives `None`.
- `machine_width(ty)` holds when `proven_range(ty)` lies within the tagged range, -2^62 to 2^62 - 1.
- `unrefined(ty)` removes the refinements of an `Int`. Any other type, a fixed-width type with its own refinements included, is returned unchanged.

The code generator applies `unrefined` to the type of every binder whose `Int` refinements it does not count: an unwritten `let`, the components of `let (...)`, the fields bound by a `when` arm, loop state, a lambda's parameters and captures, the parameters of a handler's operations and the handler's own parameters, and the parameters of a predicate. It keeps the full type for two kinds of binder: a function's parameters, and a `let` whose type the source writes.

### The facts, where each comes from, and what it may change

Four kinds of fact feed `machine_width`, directly or beside it:

| Fact | Where it comes from | Kind of trust |
| --- | --- | --- |
| **F1. A small literal.** An integer literal whose value is in the tagged range (`lower.rs`, the `SMALL` test beside `machine_width`) | The literal's digits | Sound by construction: the code generator emits it as the tagged word `(n << 1) \| 1`. Not a fact of `range.rs`. |
| **F2. A fixed-width type's width.** `I8` to `I32` and `U8` to `U32`, at every binder | The `overflow` obligation of each `+`, `-`, `*`, `/`, `mod`, unary `-` and narrowing conversion on a fixed-width type (`spec/18 §3`), the literal check U2022, and the entry points that make fixed-width values | Proven by the verifier at tier 1, which `una build` always runs (`spec/16 §1` rule 5); a literal is checked by construction. Inside the body of a `reflect` predicate, which has no `overflow` obligation (`spec/18 §2`), the code generator sets `exact` and uses no fact at all. |
| **F3. An `Int` refinement that bounds both sides.** At a function's parameter and at a written `let` | A function's parameter: the `precondition`/`refinement` obligation at each call, a call of a function value included (ADR 0037, `spec/12 §3`). A written `let`: its `refinement` obligation | Proven by the verifier at tier 1. |
| **F4. A refinement of a fixed-width type** (`I64` or `U64` bounded inside the tagged range, or any fixed-width type narrowed further), at every binder, since `unrefined` keeps it | The `refinement` obligations at every point where a value flows into the refined position: construction of the record, variant or tuple that holds it, the call of a lambda (ADR 0037), a call's result (the callee's own obligation) | Proven by the verifier at tier 1, under the hypotheses that every obligation carries: the contracts of the standard library (ADR 0033) and of host capabilities (`spec/05 §2`). |

Two parts are trusted local analysis, not proofs:

- **The reading of a refinement** (`bounds`, `intersect`, `proven_range`). The interval it gives contains every value that satisfies the refinement: each recognized form gives its exact bounds, every other form gives none, and a conjunction intersects. This is sound by construction, and covered by the unit test of `range.rs`. A bug there would take a range wider or narrower than the proven one.
- **The choice of which binders keep their refinements** (`unrefined` and its callers in `lib.rs` and `lower.rs`). It is sound only as long as every refinement it keeps is one the verifier proves of every value that reaches the binder. For `Int` it keeps two kinds (F3). For fixed-width types it keeps all (F4), relying on ADR 0037 and on the construction obligations. Note that the code comment at the lambda in `lib.rs` ("a lambda's refinements are not proven of its arguments") is stricter than `spec/12 §3`, which does prove them at each call of a function value. The code is conservative for `Int` and not for fixed-width types. Both are sound under the specification. The difference is stated here so that a later change to either side is reviewed against this amendment.

A fifth premise is a runtime invariant. Every `Int` whose value lies in the tagged range is held tagged: `int_word` in `crates/una-rt/src/values.rs` is the one place that makes an arbitrary-precision object from a computed value, and it makes one only outside the tagged range. Copying between heaps (`import_node`) copies only such objects. This is EL-2a's canonical representation, held by construction in `una-rt`, which is in the trusted base (`spec/16 §4` rule 8). It has no debug assertion or entry-point test yet of the kind EL-2a asks of the release path.

The facts change generated code in these ways only:

| Use | Condition | What changes | Kind of change |
| --- | --- | --- | --- |
| `+`, `-`, `*` (`Lowerer::prim`, `Lowerer::unchecked`) | Every operand is F1, or a variable whose type is `machine_width`, and the result's type is `machine_width`, outside `exact` code | The tag test of both operands, the overflow test of the machine operation, and the call of the runtime's promoting operation (`una_rt_int_add`, `_sub`, `_mul`) are left out. The operation is a wrapping 64-bit `iadd`, `isub` or `imul` on the tagged words. | **A removed check:** the `Int` promotion path of EL-2, and, for a fixed-width type, the only overflow handling the operation has. No engine tests fixed-width arithmetic at run time: the interpreter computes it exactly, and the obligation proves that it fits. |
| `==`, `!=`, `<`, `<=`, `>`, `>=` | Every operand is F1 or `machine_width` | A word comparison (`icmp`) replaces `compare_ints` | **A choice of instruction.** No check is removed: on two tagged words, both give the same answer. |
| Representation | None | None. A value stays a tagged word or a handle, as `spec/18 §7` states. `una-cli`'s `arbitrary_precision` only lists in the build report the `Int` parameters and results that are not `machine_width`. | **None.** |

Unary `-`, `div`, `mod`, conversions, and every memory-safety, resource, cancellation and `when` check are not affected by `range.rs`.

### What a wrong fact would cost

The removed checks rest on the verifier (encoding, generator, pinned solver, proof cache), on the trusted reading of refinements above, and on the canonical representation. This is the trust that EL-7 states for release elision, here for the Cranelift build, with no `--no-elision` to take it out.

- **A wrong result fact** (operands right, result outside the proven range): the operands are tagged, and the wrapping operation on two tagged words gives an odd word again. The cost is a wrong value, as EL-2 requires.
- **A wrong operand fact** (an operand is in fact a promoted `Int`, a handle): `Lowerer::unchecked` does not retag its result. A handle in the operation can give an even word, which the rest of the program reads as a handle, and may later release. The runtime refuses any word that is not a live handle, and has no `unsafe` code outside `ffi.rs`, so this stays memory safe (`docs/security/tcb.md` §3.3). But the word may name a live object: a later read returns another value's contents, possibly from a higher label, and a release may free that object early. This is more than EL-2 allows a wrong proof to cost in a release build, where the result is retagged with bit 0 set and "never frees one twice". It needs a bug in the verifier, in the trusted reading above or in `una-rt`. It does not arise from a correct proof.

### Finding

No fact that `range.rs` uses to remove a check is unproven or unsound by construction, within code that tier 1 verifies. The removed checks rest on proven obligations (F2, F3, F4), on literals (F1) and on two trusted local premises (the reading of refinements, the canonical representation). The bodies of the standard library are not verified (ADR 0033), so a fact used inside them rests on the trusted prelude, as everything inside them does.

The gap found is the bound on a wrong operand proof stated above: the Cranelift form is weaker than EL-2's remaining operation.

### Proposed reading of EL-1

1. EL-1's "keeps every check" reads: keeps every check except ADR 0055 NB-3's machine-width arithmetic, where `range.rs` proves the operands and the result in the tagged range from the facts F1 to F4. That part is not an elision of this record. It runs no elision query, adds no entry to `native.elided`, and `--no-elision`, which concerns release builds, does not change it.
2. "The fully checked reference" reads: checked at every site except those. The `elision-equivalence` gate's comparison with the Cranelift build therefore relies on the same proofs at those sites, so it cannot detect a wrong one there. The interpreter, which computes exactly, stays the reference.
3. The verifier, the reading of refinements in `range.rs`, the choice of binders in `unrefined`'s callers, and the canonical representation of `una-rt` are in the trusted base of the Cranelift build, for the sites where these facts remove the promotion path (`docs/security/tcb.md`, proposed with this amendment).
4. A change that widens the facts, such as a new binder that keeps its `Int` refinements, `systems`' `overflow` obligations, or a new kind of check removed, needs an amendment of this record, as a new kind of elision does (EL-2).

### Proposed follow-up, for the Owner

- **Retag the result** (recommended): make `Lowerer::unchecked` set bit 0 of its result, the form EL-2 gives the release path. This costs one `bor_imm` per operation, and makes a wrong operand proof cost a wrong value only, as EL-2 requires. It changes no output for a correct proof. It is a code change in lane S, with its own card and review. This amendment does not make it.
- **Assert the canonical representation** as EL-2a asks of the release path: a debug assertion at each entry point and a test, so that the premise above is tested as well as held by construction.

### Related, outside `range.rs`

These are observed on `main` and also differ from a literal reading of EL-1. This amendment records them and decides nothing about them:

- A narrowing conversion (`Prim::Convert`) keeps its value with no test in the Cranelift build, on its proven `overflow` obligation (`lower.rs`). The interpreter faults on a value outside the target (`una-interp`, `machine.rs`).
- `div` and `mod` on `Int` call the runtime, which gives 0 for a zero divisor (`una-rt`, `values.rs`), on the proven `division` obligation. The interpreter faults.

Both cost a wrong value under a wrong proof, never an access out of bounds.
