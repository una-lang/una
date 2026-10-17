# 0115. The hunt's oracle runner: unredacted runs and recorded releases, for development only

- Status: Accepted
- Date: 2026-10-17
- Deciders: Una project (accepted on 2026-10-18)
- Resolves: issue #794
- Target: the security hunt's release gate (`scripts/security-hunt`, `docs/release-gates.md`). The language, the spec, `una`'s command line and the release archives do not change.

## Context

The `hunt` release gate runs generated programs twice, with a literal varied, and compares what an observer below the literal's level can see (`docs/plan/security-hunt-oracle.md`). It reads `una run --json` (`una.run/1`), which renders every value whose declared label is above `public` as `<redacted:level>`: trace arguments (`crates/una-interp/src/machine.rs`, `op`), trace outcomes, and the run's result (`harness.rs`, `run_main`, `Form::Redacted`). That is right for the product: `una run` must not print a labeled value.

It leaves the oracle blind in two ways:

- **Liveness.** A program is live when its variants differ at the literal's own level, which shows that the varied literal reaches what the run does. When the literal reaches only redacted values, every variant prints the same `<redacted:secret>`, and the program counts as dead though the literal flows. Families K4, P1, T8 and the `together` forms of C6 other than its fan-out are dead for this reason today (#782, #807).
- **Declassification.** T6 holds "unless a declassification policy released information to `l`" (`spec/07 §1`). The interpreter runs a policy call as a call to its `via` function (`machine.rs`, `Rhs::Declassify`) and records nothing, so the oracle cannot tell a leak from a release. It sets aside every program with a policy to `public` (`oracle.py`, `releases_to_public`). The formal model has the event the trace lacks: `.release` (`formal/UnaFormal/F1/Binary.lean`, `EvVis`).

## Decision

1. **An observed run, behind a feature.** `una-interp` gains a Cargo feature `oracle`, in no crate's default features. With it, the harness offers an observed run of `main` that:
   - renders every trace argument, trace outcome and the run's result in the shown form, with labels kept but no value redacted;
   - records each declassification, in run order, as a release: its sequence number among the trace's events, the policy's qualified name, its `from` and `to` labels, and the released value in the shown form.

   Without the feature, the crate behaves as before this record: the observed run, its records and the policies' table are compiled out. With it, a normal run is the same as without it: the observed run is a separate entry point, and `run_main` keeps its redaction.
2. **The same run path.** `una-cli` gains a feature `oracle`, also in no default feature set, that exposes one hidden function, `run_observed`, which runs a package exactly as `una run --json` does: the same loading, check, tier-1 verification, grants, keys, limits and world. Only the interpreter's entry point differs. It returns a document of schema `una.oracle-run/1`: the fields of `una.run/1`, unredacted, and `releases`. The path is shared rather than copied, so that the oracle tests what ships.
3. **A crate that never ships.** `crates/una-oracle` has `publish = false`. It is not among the workspace's `default-members`, and it builds a binary, `una-oracle`, the only crate that enables the `oracle` features. `scripts/release.sh` builds `una-cli` and `una-rt` by name, without the feature.
4. **Guards.**
   - A test reads the feature resolution of the release build (`cargo tree -e features` for `-p una-cli -p una-rt`) and fails if `oracle` is active anywhere.
   - `scripts/check-release.sh` fails an archive, on every platform, that holds a `una-oracle` binary, the symbol `run_observed`, or the string `una.oracle-run/1`.
   - A `una` built as `scripts/release.sh` builds it lacks the observed run: a canary of the hunt gate checks it.
   - `una-oracle` refuses to run unless `UNA_HUNT_RUN=1` is set, which the hunt's sandbox sets for every run. Its documentation says that it prints labeled values and is for the oracle only. The marker guards against misuse, not against an adversary: whoever can set an environment variable can run it.
   - It is only ever fed the hunt's generated programs, whose literals are synthetic. Its documents and releases go only to the hunt's own output, which the gate's existing scans of host paths and personal data cover.
   - `una run --json` documents are byte-identical whether or not `una-oracle` is in the build, which a test checks.
5. **The oracle.** Each observed run must be the `una run --json` run of the same variant but for its values: the same status, output, events (their places, tasks, instances, capabilities and operations) and diagnostics, compared without their messages, since a failure's message renders its value; any other difference is an error of the program. The comparisons that decide a leak, and the two-run determinism check, stay on `una run --json` documents, messages included where they are compared today. The hunt's liveness uses the unredacted documents, with the releases visible at the literal's own level, as the model's `EvVis` makes them visible. Two variants are compared at an observer only when the releases that observer sees, those whose `to` label is at or below it, are equal. Variants whose releases differ are counted as explained, not as a leak. The product's comparison, which the gate's findings come from, stays on `una run --json`. The liveness floor is measured again and pinned at about 90% of the measured run, with the families above among the live ones. Three canaries cover the change: a program whose literal reaches only redacted values, which must be live; a planted release that misstates what it released, which the comparison must report; and the release build's `una`, which must lack the observed run.

## Consequences

- The oracle sees what it varies: programs whose secrets reach only redacted values become live, and programs with a policy to `public` are compared up to what their policy released, rather than set aside.
- The repository holds code that prints labeled values. It is reachable only through a feature that no release build enables, in a binary that no release archive holds, which the release scripts and a test check. A developer can still build and run it by hand; that is what it is for, on generated programs.
- The interpreter has two entry points for `main`. The observed one is tested against the normal one, so they cannot drift without a failing test.
- The gate's time grows by the observed runs; its budget is measured again.

## Alternatives considered

- **A copy of the run path in `una-oracle`.** Rejected: a copy of the loading, check and grant logic could drift from what ships, and the oracle would then test a different program than `una run`.
- **A command-line flag of `una run`.** Rejected: it would put the unredacted mode in every release binary, and `una`'s command line is out of scope (#794).
- **Liveness from the native executable's trace.** Rejected: the native runtime redacts labeled values too (`crates/una-rt/src/render.rs`, `host.rs`), and the oracle's comparison is on the interpreter.
- **No change: keep the families dead and programs with a policy to `public` set aside.** Rejected: the gate would keep not testing the paths where a secret flows into redacted values or through a policy, which is where a leak through declassification would be.

## Measurements (2026-10-18)

The gate's time budgets were measured again on an Apple M1 Max (10 cores, 32 GB), on the branch of this record. The binaries and programs did not change between runs: the runs' tips differ only in the gate's timing lines. The load average came from other work, with no other heavy job running.

**What the observed runs add.** Each variant whose run completes runs a third time, observed: 2,694 observed runs in the gate, beside the two `una run` of each variant that check determinism. They are part of what the gate exists to do, so their cost is kept: they cannot replace either plain run without moving the determinism check off `una run`. Skipping them where the plain runs already show liveness would leave the check that an observed run is the run's own (Decision, item 5) unexercised there, and it would save little, since most live programs are live only through them.

The gate now prints the seconds of each phase, and the observed runs' share of the oracle phase. The share is an estimate: their summed seconds over the gate's 4 workers. It changes no verdict.

The four new canaries (`live`, `release`, `release-build`, `message`) run in the last phase, which took 8 to 11 seconds in all.

**macOS** (budget 270 seconds before, 335 now):

| run | seconds | load average, start and end |
| --- | --- | --- |
| 1 | 273 | 4.54, 5.85 |
| 2 | 310 | 5.85, 4.08 |
| 3 | 306 | 2.14, 4.82 |
| 4 | 274 | 4.34, 4.71 |

- Run 4's phases, in seconds: setup 0, must-reject 29, must-keep 89, oracle 79, native sample 0, re-check 66, canaries and checks 11.
- The observed runs account for about 12 seconds of the oracle phase.
- The four times fall in two clusters, about 274 and about 308 seconds. The same binaries gave both. The slower cluster is unexplained, and it did not recur. This record's five later runs of the same binaries took 270 to 282 seconds: run 4, and the four in the comparison below, two of which could not run the re-check. Main's four runs beside them took 259 to 272 seconds.
- Against main's 252 to 256 seconds (2026-10-17), the fast cluster is about 20 seconds slower. The comparison by phase below attributes it.
- 335 seconds is the slowest run, 310, plus 7%, rounded up to 5 seconds.

**macOS, by phase, against main.**

- **How it was measured.** Main's gate (`d4217d2c`) was run with only these timing lines added, which print and change nothing else. Its runs alternated with this record's on the same machine, base and record by turns, in two sets of four runs.
- **Which runs count.** In the first set, the record's re-check could not run (the checker for the wrong platform, below), so its re-check and totals come from the second set only.
- **Load.** The load average was 4.3 to 11.2 across the sets.

Seconds; each cell is the mean (min-max, n):

| phase | main | this record | difference |
| --- | --- | --- | --- |
| must-reject | 28.8 (28-31, 4) | 28.0 (27-29, 4) | -0.8 |
| must-keep | 93.2 (88-97, 4) | 95.2 (92-102, 4) | +2.0 |
| oracle | 64.5 (62-66, 4) | 78.5 (76-83, 4) | +14.0 |
| re-check | 69.8 (66-72, 4) | 67.5 (66-69, 2) | -2.2 |
| canaries and checks | 9.0 (9-9, 4) | 9.5 (9-10, 4) | +0.5 |
| total | 265.5 (259-272, 4) | 273.5 (272-275, 2) | +8.0 |

What the table shows:

- **Oracle phase.** The growth is in the oracle phase, about 14 seconds.
  - About 11 to 12 seconds of it are the observed runs' process time, an estimate: their summed seconds over the 4 workers.
  - The remaining 2 to 3 seconds are within the phase's spread between runs (4 to 7 seconds). They are not measured; a possible cause is the observed runs' setup outside their processes, such as a fresh directory and the package written, or the document parsed and compared.
- **Last phase.** The new canaries add under a second to it.
- **Other phases.** The phases this record does not touch move within the spread between main's own runs: must-reject -0.8, must-keep +2.0, re-check -2.2, against spreads of 3 to 9 seconds.
- **Total.** The total grows by about 8 seconds, within the spread of main's own runs (259 to 272 seconds).
- **Earlier runs.** Of the 20 seconds between the fast cluster and main's runs of 2026-10-17, about 14 are the oracle phase. The rest is the difference between days, which the alternating runs remove.
- **Linux.** The Linux runs, set against main's of 2026-10-17, are read with this table: their growth beyond the observed runs' 14 seconds is taken as load and machine state, not as a phase this record touches.

**Linux** (`hunt-linux-arm64`, Docker Desktop's VM on the same machine; budget 390 seconds before, 425 now):

| run | seconds | load average, start and end |
| --- | --- | --- |
| 1 | 396 | 4.38, 4.22 |
| 2 | 388 | 3.43, 2.50 |

- Phases, in seconds:
  - run 1: setup 0, must-reject 47, must-keep 127, oracle 95, native sample 0, re-check 118, canaries and checks 8;
  - run 2: setup 0, must-reject 45, must-keep 123, oracle 94, native sample 0, re-check 117, canaries and checks 8.
- The observed runs account for about 14 seconds in each.
- Against main's 362 and 359 seconds (2026-10-17), the runs are about 9% slower, 26 to 34 seconds:
  - about 14 seconds are the observed runs;
  - under a second is the new canaries, as on macOS;
  - the rest, 10 to 18 seconds, has no cause measured on Linux. It is not load, which was 2.5 to 4.4 here against about 4 to 5 then. It is within the variation this machine shows between runs of the same binaries: about 13% between the macOS clusters. On macOS, where main was measured by phase beside this record, no untouched phase grew beyond that spread.
- Run 1 used a fresh build volume and run 2 the same volume, warm. The gate's time starts after the builds, so the volume affects the build steps, not the measured time. That is consistent with the two runs being 2% apart.
- 425 seconds is the slower run, 396, plus 7%, rounded up to 5 seconds.

**A checker for the wrong platform fails the gate closed.**
- **What happened.** The first Linux run rebuilt `una-core-check` inside its container. The new build replaced the host checkout's macOS build with a Linux one (#863).
- **What the gate did.** Two later macOS runs could not execute the checker. Every re-check exited 126: 1,615 errors and no program compared. Both runs reported `FAIL` and recorded `verdict: FAIL`. Nothing was counted as clean, and the gate exits 1 on any verdict other than PASS.
- **What changes.** The gate now refuses to start when a tool it runs is not an executable for the host. The check reads the first bytes of four files before the gate's clock starts, so it is in no phase's time. Those two runs' re-check times are not used here.

## Acceptance (2026-10-18)

The project accepted this record on 2026-10-18, with the measurements above. The time budgets are final: 335 seconds on macOS, and 425 seconds on Linux. The Linux budget is set from the two Linux runs, with no comparison by phase on Linux, which the macOS comparison stands in for. From this date the record's text is not edited; a later change is a dated amendment.

## Amendment (2026-10-18)

**Decision, item 5: releases are compared in run order.**

- **Before.** Two variants were compared at an observer only when the releases that observer sees were equal, and any difference between variants whose visible releases differed counted as explained. That threw away the order of the releases: a value a variant printed at `public` *before* its release to `public` was explained by that later release.
- **Now.** An observer's visible events and visible releases are compared in run order, each release after the event it follows (`after`), as the formal model's `.release` events interleave the trace. A difference is explained only when the first place where the two variants differ holds a release; everything after it, the status, output, failure and diagnostics included, is then explained. A difference before the first differing release, or with no differing release, is a leak.
- **Canary.** `release-late` prints a line, releases the secret and prints it. As written, its variants are explained. When its first line is planted to show the secret, before the release, they must be a leak at `public`.

**Other changes to item 5 and the guards.**

- **The same-run check.** An observed run must also match its run's tier, each event's outcome kind and variant, its arguments' names, and the failure's variant. Each of the failure's public fields must be among the observed run's fields, which also shows the labeled ones, with the same rendered value. A `<redacted:level>` nested in it matches any text at its own place only. Comparing rendered text cannot check the observed value's label there; a typed comparison is #892.
- **Unknown policy.** A declassification through a policy the index does not know is an internal fault of the observed run, not a release without levels.
- **The ceiling on explained comparisons.** The explained comparisons have a ceiling (`MAX_EXPLAINED`) as well as a floor.
- **Local CI.** It builds a `una` with the `oracle` feature explicitly, and `scripts/check-release.sh` requires it for the byte-identical comparison of `una run --json`. It also runs clippy and the tests of `una-cli`, `una-rt` and `una-interp` as they ship, without the feature.
- **Manifests.** A test reads every manifest of the workspace, for every target, and allows only `una-oracle` to turn the feature on.

## Amendment (2026-10-18, divergence)

**The previous amendment of 2026-10-18 is narrowed to the formal model's divergence.**

- **The flaw.** That amendment explained a difference when "the first place where the two variants differ holds a release", on either side. That is wider than `BRelR` (`formal/UnaFormal/F1/BinaryR.lean`). There, two runs diverge only when, after a common visible prefix, *both* release, through the *same* policy.
- **Now.** A difference is explained only when, at the first place where the two merged traces differ, both hold a release through the same policy. Any other first difference is a leak:
  - an event of one variant where the other releases, as when a secret decides whether a line is printed before the release;
  - two different policies;
  - one trace ending where the other goes on.
- **Canary.** `release-branch` plants exactly the first of these: in one variant, a visible line before the release, where the other variants release. It must be a leak at `public`.
- **Release positions.** A release of the observed run without a position (`after`) is an error of the program.
- **Matching.** The comparison of a failure's public fields, with a `<redacted:level>` standing for any text at its own place, is a linear scan: the first and last pieces anchor the ends, and each piece between is found in order. It has no backtracking.
