# ADR 0032 amendment draft: an operation's outcome rendered at its label (S-45, v0.3 line)

Status: Proposed (draft for the project's decision; it is appended to docs/adr/0032-runtime-details-left-open.md only once accepted, with the date of its acceptance). Security fix of #1552 (S-45), backported to the v0.3 line from the fix on `main` (#1562); the behavior is already implemented in the pull request that adds this draft. Until then `spec/11 §5`, `spec/09 §4` and `spec/13 §8` cite it as proposed.

## Amendment (proposed 2026-11-04): an operation's outcome rendered at its label (S-45)

Rule 3 says that "an operation's outcome is redacted by the label of the binder of its result", and the Consequences say that "traces never show labeled data". Both engines implemented rule 3 as a redaction of the `ok` value only: the outcome's kind (`ok` or `fail`), the failure's variant and its payload were written in clear. Since ADR 0085 (S-14, v0.3.4) whether a `Files` operation fails is labeled data, and ADR 0085 rule 7 labels the failure decision of every `Http` request with all its arguments. The checker held those labels at every operation site, but the trace writer never consulted them.

The consequence, tested on v0.3.17 in editions `0.1` to `0.3`, in both engines: a `Files.write` of an `internal` path at a `public` program counter, whose path the trace shows as `<redacted:internal>`, is recovered exactly, bit by bit for any width, from the outcomes of later public `Files.read`s; the write's own outcome (`ok` or `NotFound`) tells which path it took; and an `Http` failure variant (`Denied` or `Unreachable`) tells whether an `internal` URL lies inside the grant. The program itself cannot observe any of them (U4003 on a branch that prints); only the trace did.

Rule 3 now reads, for a run's trace:

> **Outcomes.** An operation's outcome is rendered at its **outcome label**: the label of the decision whether the operation fails and of its payload (`spec/07 §7` rule 4: the deciding arguments capped at `accepts`, every argument of `Http`, and the file-state label for `Files`, ADR 0085), joined with the labels of its failure type and of its result's binder. When the outcome label is above `public`, the event's outcome is `{"kind":"redacted","level":"<level>"}`, with its level only: whether the operation returned or failed, the failure's variant, its payload and the value are not written, and no text is built from the value. When it is `public`, the outcome is rendered as before: `{"kind":"ok","value":...}` with the value redacted by the binder's label, or `{"kind":"fail","variant":...,"value":...}`. `aborted` and `cancelled` are not decisions of the operation and keep their kinds.

The outcome label is static: the checker computes it once per operation site, elaboration carries it in Core IR as the `outcome` label of the operation (`spec/09`, `spec/13 §8`), and the native backend embeds it in its description of the site, which fails to load without it. An operation with no recorded label counts as `⊤`, and so does one in a body that no label rule checks (a policy's `via` function). Whether an outcome is redacted therefore depends on no value.

**What it does not cover.** The rule covers what an event shows, not whether it occurs. On this line each operation invocation still appends one event, so an operation performed under a branch on an `internal` value shows by occurrence: which events appear, how many, and the `seq` of later events can depend on that value. The Consequences' "traces never show labeled data" therefore holds, after this amendment, for every argument and outcome of an event, each shown at most at its label, and not for the trace's shape.

**Examples.** An example's trace keeps its outcomes in clear: an example's data come from its source (ADR 0078), and `expect performed` and the reasons of a failed example read them. `una test --json` is therefore unchanged.

**Editions.** The rule applies to a run in every edition the v0.3 line accepts (`0.1` to `0.3`): the routes above need no construct of a later edition.

## Choice

This is the rule of the fix on `main` (#1562), without its parts for constructs this line does not have (`gives` labels of ADR 0094 and the limiter of `std.limit`), so the two lines do not diverge in meaning. A renderer-only patch that redacts every failure, or every `Files` and `Http` outcome, would close the channel without the checker's label but would also hide outcomes decided at `public`, which this line shows in clear today. The result binder's label is part of the outcome label, as on `main`: it is the conservative choice.

## Consequences

- Nothing stops compiling, no check is weakened, and no edition changes its language: the change is in what a run's trace shows.
- **Rebuild.** There is no version handshake between the compiler and the runtime. A native executable built before the fix embeds the old runtime and keeps writing outcomes in clear until it is rebuilt; a patched `una` linked with an older `libuna_rt.a` (through `UNA_RT_LIB` or a stale runtime in a source tree) still leaks, since the old runtime ignores the new `outcome` key; an older `una` with the patched runtime exits 3, since its description lacks the key. Native executables must be rebuilt with the patched toolchain, `una` and its runtime together.
- **The cost is trace observability.** In a run's trace the outcome of every `Http` request (its result is `internal Response`), every `Files.read` (`internal Text`), every `Env.get`, every `Keys.hmac`, and every `Files` operation of a package whose file-state label is above `public`, is `redacted`. An operator no longer reads from the trace whether a request failed, or how. Outcomes of `Console.print`, `Clock.now`, handler operations whose result and failure decisions are `public`, and `Files.write` in a package whose writes are all `public`, stay in clear.
- Consumers of trace outcomes must accept the kind `redacted`: the equivalence tests of both engines do, `una test`'s `expect performed` counts a redacted outcome as performed (examples are not redacted anyway), and external readers (the hunt's oracle, `docs/security/tcb.md` item 5, and the una-bench readers) need adjusting.
- Complexity: O(1) per operation site at check time (one label recorded), O(1) per event at run time (one test of a static level), O(S) space for S operation sites.
- `una --json core` gains the key `outcome` on each `op` node, additive within `una.core/1`; a consumer that renders no trace may ignore it.

## Notes on related decisions (drafts, appended on acceptance)

- **ADR 0085, note.** The file-state label also labels the outcome of every `Files` operation in a run's trace: in a package where a write can run under an `internal` program counter or with an `internal` path, every `Files` outcome is `redacted`, public writes included. No rule of ADR 0085 changes.
