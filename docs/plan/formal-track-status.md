# Formal track: status

- Issue: #476, under #46
- Date: 2026-10-11, on #538's branch (`main` 408e59d6 with #538)
- Plan: `docs/plan/formal-track.md`; details in the milestone documents `formal-track-m2.md` to `formal-track-m5.md`

## In plain language

Una claims that a program `una check` accepts does not leak: for an observer at a label, two runs whose secrets differ look the same, except for what a declassification policy releases (T6, `spec/07 §1`). Until this track, that claim rested on tests, and 12 leaks (S-01 to S-12) were found in released versions.

Now there is a proof in Lean, and a checker that applies it to real programs:
- **The proof.** For a model of Una's core language, F1, T6 is proven with no `sorry` and only Lean's standard axioms. F1 covers functions and closures, generics, failures with codes and payloads, sum kinds, declassification, the handlers a document declares for its own capabilities (#524) and the `with` scopes that install them (#538), and the host's files and HTTP as state that runs share. Type safety (T1) is proven too.
- **The re-check.** A Lean program, `una-core-check`, reads what `una core --json` exports of a package and checks it again, by rules proven sound. It answers per declaration: accepted, rejected, not re-checked, or outside the flow model (a refinement predicate, #532).
- **Accepted means proven.** Each accepted declaration is part of a whole program that the Lean checker certified, and a theorem gives that program T6. A form the model lacks gives "not re-checked", never "accepted".
- **Replay.** The re-check rejects the leaking programs of 8 of the 12 findings (S-01 and S-06 to S-12), run on the Core that their pre-fix releases printed. The other 4 are not replayed: S-02 (JSON) and S-04 (`edit`) are outside F1, S-05 (`Files.write`) is replayed from v0.1.3's dumps, with the file-state label taken from v0.3.6: 4 of its 5 shapes are rejected and the fifth is not re-checked (#498). S-03 (`Http`) is not replayable from any release, since `Http` first shipped after its fix.
- **Corpus.** Every conformance case and example: 1,194 declarations accepted, 0 rejected. That is about 27% of the corpus's 4,330 declarations (71% of the 1,687 in complete documents, `pred` declarations aside). Most of the rest is outside F1: tuples, primitives such as `div` and `mul`, handlers installed with `with` or from outside the document, `together`, and documents the export marks incomplete. So "0 rejected" means that no checked declaration disagrees with the proof, not that the whole corpus is proven.

The track found three real leaks in released versions, S-14 to S-16, all fixed and released. Each total it reported was corrected whenever a review found a gap, and every correction lowered or held the count of accepts, except where a new theorem backed them.

## The corpus today

On 1,352 documents (every conformance case and example, exported by #524's tip), source declarations only:

| Outcome | Declarations | In documents `una check` accepts | In documents with errors |
| --- | --- | --- | --- |
| Accepted | 1,194 | 656 | 538 |
| Rejected | 0 | 0 | 0 |
| Not re-checked, in complete documents | 493 | | |
| Not re-checked: the document is not complete | 2,530 | | |
| Not re-checked, in all | 3,023 | 323 | 2,700 |
| Outside the flow model (`pred`) | 113 | 95 | 18 |

A document "with errors" is one `una check` rejects for any reason, often a refinement it refutes (U5001), while its other declarations still check; an accepted declaration there is accepted on its own information flow. The table counts each declaration once (a conformance case that declares a name twice prints two lines).

"Not re-checked" is mostly forms outside F1: tuples, fixed-width integers, primitives F1 lacks (`div`, `mul`), `given` and `together`. A declaration that reaches one of these is not re-checked either.

**Synthesized handler lines** (#524), counted apart, outside the table: the re-check reads each handler operation a document declares for its own capability as a function, `module.handler.op`, and reports it on its own line. They are not source declarations, so the headline stays comparable with every earlier figure. On this export: 19 lines, 17 accepted and 2 not re-checked (a parameter of a type F1 does not read). A declaration that invokes a handler the re-check rejects, or does not read, is not re-checked.

**The decision label** (#524, ruling D): `attempt` and a handler label what they give by what decides the failure, without the program counter, so an operation's failure handled inside a secret branch keeps a public result, as `una check` types it. T6's statement is unchanged; its proof relates failures where that label is seen (`docs/plan/formal-track-handlers.md` §4a).

A `pred` declaration is counted apart, as "outside the flow model" (#532): a refinement predicate is pure and its labels are erased (`spec/07 §7` rule 6), so it is outside the flow model by design. The count covers only the documents the re-check reads declaration by declaration. When a whole document falls back to one outcome (it is not complete, it declares a capability named as one of the host's, it declares `Header` or `Response` otherwise than `spec/24 §1`, or it has a policy F1 cannot read), every declaration with code is "not re-checked", its predicates included.

History of "accepted":
- M2 accepted 601, on v0.3.4's export of 1,276 documents. After #443's keys, `main` accepted 647, and #452 removed the 37 without a theorem: 610.
- On the re-export of 1,327 documents, the count was 641 before A2. Then: A2 556, plus 82 conditional; A3 638; M5b 642 (1,329 documents); M5c 643.
- Later items widened F1, each with its own corpus delta: 1,173 on the 1,352-document export before #524, 1,176 after it (3 declarations that invoke handlers); 1,199 before #538 (after #532's predicates, #542's prelude functions and #545's `at`), 1,194 after it (1 gained, and 6 that reach an operation of a `with` scope outside the scopes no longer accepted).

## Corrections to earlier totals

Each one came from an adversarial review or from the proof itself.

1. **#452: 37 accepts without a theorem.** `main` accepted 647, and 37 of those had no theorem behind them:
   - 2 were in documents with a policy that B1 does not cover;
   - 35 reached a function that was not re-checked (29) or was rejected (6).

   The rule since then is "no accept without a theorem": a declaration is accepted only if everything it reaches is. 647 → 610.
2. **A2: 82 accepts were conditional.** A function checked at its generic instances had no T-Fun proof until A3. The 82 declarations that were, or reached, such a function were reported in a bucket of their own, "accepted, conditional on A3". A3 proved them (`t6_release_erasure_obs`), and they moved to accepted with no other change.
3. **The mutants section of the replay.** The replays reported on #455 and #456 ran a stale `replay-mutants` binary. `run.sh` now builds both executables first, and the replay was re-run.
4. **#472: a gap on `main`, N = 0.** A host-free declaration kept the any-host note even when a caller in its document handed it a closure that writes a file. No corpus declaration was affected, so the earlier statement that such accepts were overstated was itself corrected to 0. The note now follows the declaration's own reach (`hostFreeAt`).
5. **M2's 7 rejections were not leaks.** Each one needed a fact the checker has and the export lacked. #443 added those facts to `una.core/1`, and the rejections went to 0.
6. **#507: a `Result`'s error type was dropped.** The importer read `Result[T, E]` as F1's result of `T`, whose `Err` is a code. But `to_text` and `==` read an `Err` whole, payload included. On a Core that typed `to_text` of a `Result[Int, internal Int]` as `public Text`, the re-check said accepted, while the runtime would print the `internal` payload. `una check` rejects such a source (U4001), so no corpus declaration was a leak. Now the error type is dropped only when it is a kind without payload (the document's, or `FileError`, `KeyError` and `HttpError` of the specification) whose labels the result's top label carries; otherwise the declaration is not re-checked. 756 → 754: `relay.caught` (an error type the export leaves open, `?`) and `text.failure` (`ParseError`, whose `NotADigit` carries a position). `s15_ok_http_attempt.send` stays accepted: `internal HttpError` under an `internal` result.
7. **#508: a `Result` keeps its error type.** `Result[T, E]` is F1's result of `T` and `E`, `Ok(a)` and `Err(a)` build one, and a `case` binds the `Err` arm's value at `E` raised by the scrutinee's label. `to_text` and `==` take every label of both positions, so F-R1 is closed by tracking labels instead of refusing the program. A kind without payload is still a failure's code, and a kind with a payload is not re-checked (#512). On a fresh export of `main` (1,344 documents), 759 → 765 accepted, 0 rejected: six declarations that read an `Err` arm's value (4) or use a `Result` whose error type is an ordinary type (`sw.g`, and `sw.leak`, which reaches it). An `attempt` whose error type the export leaves open (`relay.caught`, `?`) is not re-checked, never rejected.
8. **#509: user record kinds were never read.** The importer looked for a record kind's type parameters under `type_params`, which the export never writes (`params`), so only `Http`'s `Header` and `Response` were records. A record is now read when the export flags it a record (`record`, which keeps a one-variant sum kind, a failure's code, out), it has no type parameters and it does not reach itself. Accepted, split by what `una check` says of the declaration's document: in documents it accepts, **506 → 529**; in documents it rejects, 259 → 499, every one for a proof or run-time diagnostic and none for information flow (U5001, an obligation refuted, 228 → 468; U5001 with U5002 20, U5002 4, U5003 2, U6003 1 and U6005 4, unchanged). In all, 765 → 1,028, 0 rejected, none lost. The re-check proves those declarations' flows, which is sound, but a document `una check` rejects does not count toward the headline.

## Findings the track produced

| Finding | What | Fixed in |
| --- | --- | --- |
| S-14 | Whether a file exists, or a read fails, could reveal an earlier write to an `internal` path. Found by checking M3's host assumption. | v0.3.4, v0.1.6 (ADR 0085) |
| S-15 | A server could decide a public failure (`TooLarge`, `Protocol`, `TimedOut`) from the internal body and headers it received. | v0.3.4, v0.1.6 (ADR 0085) |
| S-16 | A read of a file, or an environment variable, that is not valid UTF-8 failed, so a public branch could learn something about internal content. Found by the review of M5's host premise H1. | v0.3.5, v0.1.7 (ADR 0086) |
| S-17 | A `checked` predicate that read a labeled value besides its operand failed publicly, so a caller could learn a comparison with that value. Found while widening F1 with `checked` (#453). | v0.3.6, v0.1.8 (ADR 0089) |

M3 also found that `spec/10`'s formal T-Let and T-Handle lack the rule-4 premise that `spec/07` states. It is not a finding, because the surface language cannot write a program where the two differ. `spec/10 §13` corrects it, append-only (#479, ADR 0087).

## What "accepted" rests on

**The premises on the host** (`Host.Lawful`, `Host.AnonInvariant`). Each one is a hypothesis of the theorem, and it is named in the verdict's note when it is used:
- **H1:** whether a read fails depends on the path and on which files exist, not on their content. The real host meets it since v0.3.5 and v0.1.7 (S-16).
- **H2:** whether a write fails, and which files it leaves, does not depend on what it writes.
- **H3:** a response does not depend on the program's type annotations (the host is given text).
- **H4:** every successful HTTP response is a `Response` of `spec/24`. The runtime builds it so by construction.
- **H5:** an answer depends only on the host state at its label (ADR 0094 rule 5). What an operation answers through a binding where it is labeled `l′`, and whether it fails, is a function only of the arguments and of the part of the host state at or below `l′`. It is a premise on how the deployment wires its bindings. F1 does not model operation labels until #563, so a declaration that labels an operation, or that calls an operation through a binding where it is labeled above `public`, is accepted on the checker's rules alone, not re-checked by the formal model; the re-check's note names H5 for such declarations once #563 lands. Since #1518 the export lists each `uses` entry's labels under `gives` (`spec/13 §8`), and the re-check reports a declaration whose `uses` hold one, a handler's synthesized operation among them, as not re-checked, "F1 restriction (a labeled operation, ADR 0094 rule 8)", and what reaches it as not re-checked too (`GivesImportTests`). Before it, the export wrote no label, and such a declaration could be reported accepted under H1 to H3 alone.
- **`Env`:** the environment variables that exist are the same in both runs; their values may differ, and T6 covers them (#487). Whether a variable exists is `public` (`spec/14 §3`): it is public configuration, so an operator must not encode a secret in which variables are set.

A declaration whose program uses no host operation, and whose parameters cannot carry one in, needs none of these: its note is plain, and its theorem holds on any host.

**Outside the model** (as `spec/07 §1` states):
- timing, termination and resource limits: T6 is termination-insensitive, and an abort is termination;
- other writers: the file system changes only by the package's own writes, during and between runs, and the servers do not write it;
- the network's nondeterminism: the model's request is a function of the URL, the arguments and the server's state;
- handler bodies the re-check does not read: a handler from outside the document, or one installed with `given` or of a capability not read by scopes (`docs/plan/formal-track-with.md` §4c), is outside the reach set until M7 (F2, T4). A `with` of a handler the document declares is read since #538, and its handler's events are charged where it is installed (ruling E). A handler a document declares for its own capability is read and checked (#524); the theorems hold for every handler `una check` accepted (premise Hu), not for forged or unchecked Core.

**The trusted base:**
- elaboration from the surface language to Core;
- the reading of `una.core/1` into the model (`Import.lean`), whose facts can only raise a signature;
- the Lean kernel.

## What is left

- **M6:** `edit` and loops (ADR 0071, ADR 0075), the memory model's label rules. This is the fragment where S-04 lives.
- **M7:** F2 and T4: capabilities as parameters, `given` and handlers from outside the document, which removes the rest of the handler assumption.
- **Small items:**
  - #475, an accurate note when only a function parameter blocks the any-host claim (cosmetic);
  - #453, the widening of `checked`;
  - the append-only correction of `spec/10`;
  - the trace's own events, `stop` of a host stop (ADR 0116 rule 14), `timeout` and `cancelled` (`spec/27 §5`), and Core's `quiet` (S-39, #1507, `spec/26 §6` rule 4): the model has none of them, so a declaration that runs a `for ever`, or holds `quiet`, is not re-checked. The theorem to state is that these events are sinks that accept `public`, so code under a raised program counter appends none.
- **Later, by priority:** T5 relative to the solver, T6 with `together`, and edition 0.1's own rules.

## Theorem names

`scripts/formal.sh` states 243 theorems, in `formal/Theorems.lean`.

| Claim | Theorems |
| --- | --- |
| T1, type safety | `t1`, `t1_labeled` |
| The unary half: events allowed, failures labeled (T2 and T3 with the program counter) | `unary_sound`, `unary_soundR` |
| T6, with generics, for a program without policies | `t6_sound`, `t6_ended` |
| T6 with declassification (B1) | `t6_soundR`, `t6_release_host`, `t6_release_host_agree` |
| T6 with a stateful host, and across runs | `t6_host`, `t6_host_ground`, `across_runs`, `across_runs_events` |
| A program checked at its instances | `eval_anon`, `eval_erasure`, `t6_release_erasure`, `t6_release_erasure_obs` |
| Any host, for host-free code | `eval_hostFreeIn`, `t6_release_erasure_obs_hostFreeIn`, `t6_release_erasure_obs_reach` |
| The checker is sound | `Check.checkExpr_sound`, `Check.progOk_sound`, `Check.progOk_sound_nil`, `Check.hostWfB_sound`, `reach_closed` |
| Every accept has a certificate | `Import.finalize_sound`, `Import.withHost_ex`, `Import.guardFiles_sub` |
| S-14 and S-15: the old rules leak, and the new rules reject the probes | `OldLeaks.s14_old_leaks`, `OldLeaks.s15_old_leaks`, `Untyped.s14_untyped`, `Untyped.s15_untyped`, `NewRules.s14_rejected`, `NewRules.s15_rejected` |
| Non-vacuity, one example per step | `T6Example`, `ReleaseExample`, `MonomorphExample`, `HostExamples`, `HostFreeExample`, `FilesCertExample`, `HttpExample`, `EnvExample` |
