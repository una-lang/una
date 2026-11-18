# 0091. A Core export of the declarations rejected for information flow

- Status: Accepted (coordinator, by the owner's delegation, 2026-10-11, for his review)
- Date: 2026-10-11
- Deciders: Owner (by delegation to the coordinator, 2026-10-11), on a proposal by Owner (Una-2)
- Resolves: #514
- Amends: `spec/13` (the `una core` command, §3, and the Core export, §8)

## Context

`una --json core [path]` prints a package's Core IR as one `una.core/1` document (ADR 0083, `spec/13 §8`). When the package has diagnostics, `complete` is `false` and Core holds only the declarations without them: elaboration skips every declaration with a diagnostic (`una-check`, `elab.rs`, `elaborate_module`).

The formal track's re-check (`una-core-check`, `formal/`) reads this document and checks each declaration again, by rules proven sound (T6). So it sees what the checker accepts and never what it rejects. The differential security hunt (#513, `docs/plan/security-hunt.md`) needs the reverse comparison too. A declaration that the checker rejects for information flow (a `security` diagnostic: U4001 to U4005 and U4007 in the registry today) and that the model accepts is one of two things:
- an over-strict checker, which costs precision;
- a channel that the checker guards and the model does not have, which is a blind spot of T6 and a question for the formal track.

Either one is worth knowing, and neither is visible today.

A declaration rejected only for information flow has passed name resolution and type checking: the security pass runs on a typed program. So its resolutions exist, and elaboration can build its Core as it does for an accepted one. A declaration with a name or type error may lack them, and elaboration would lose parts of it (`Fx::lost`). Verification diagnostics (U5xxx) do not reject a declaration from Core: such declarations are exported today.

ADR 0076 lets a key be added to a `una.*/1` document without a version bump, since consumers MUST ignore unknown keys. A new command-line flag is not a key: `spec/13` §3 lists every command and flag, so the flag needs this ADR.

## Decision

1. **The flag.** `una --json core --with-rejected [path]` prints the `una.core/1` document of `una --json core [path]` with one more key, `rejected`, after `host`. The flag requires `--json`: `una core --with-rejected` without it prints a usage message on stderr and exits with the usage status. Without the flag, the document is byte-identical to the one `una --json core` prints without this ADR.

2. **The key.** `rejected` is a list, in the order of Core's modules and of each module's declarations, of
   ```json
   { "module": "app", "decl": { ... }, "diagnostics": [ { "code": "U4003", "span": { ... } } ] }
   ```
   - `decl` is the declaration's Core, in the encoding of `modules[].decls` (`spec/13 §8`).
   - `diagnostics` are the declaration's diagnostics, each with its code and its primary span, in the order `una check` reports them.

3. **What it holds.** A declaration is in `rejected` exactly when:
   - it has at least one diagnostic, and every diagnostic it has is in the `security` domain, a U4xxx code;
   - its elaboration is complete: nothing of it is lost (`Fx::lost` is empty).

   Every other declaration with a diagnostic is left out, as it is today. A wider set can be added later, as an additive change.

4. **Never accepted Core.** A declaration in `rejected` is not in `modules`, and `complete` keeps its meaning (`false` when the package has a diagnostic). A consumer that ignores unknown keys, as ADR 0076 requires, reads exactly the document it reads today. The schema stays `una.core/1` (ADR 0076, rule 1).

5. **No other consumer.** The elaboration of a rejected declaration is done only for this key. `una check`, `build`, `run`, `test`, `verify`, the interpreter, the native backend and `una core` without the flag see the same Core, diagnostics and outputs as before this ADR. A test pins it on a package that mixes accepted and rejected declarations.

6. **Diagnostics and exit.** The diagnostics still go to stderr, and the exit code follows them, as for `una core`.

7. **The formal re-check.** `una-core-check` ignores `rejected` by default, so its verdicts are unchanged. A separate mode, `una-core-check --rejected`, checks each declaration in `rejected` against the program of `modules` and `rejected` together, and reports it apart, with one of three outcomes:
   - `model-accepts-candidate`: the model's checker accepts it, so it is a candidate, either an over-strict checker or a channel the model lacks. Each candidate goes to the coordinator before any conclusion;
   - `model-rejects`: the model agrees;
   - `not-re-checked`: a form the model lacks.

   The mode never calls a rejected declaration "accepted". It never changes the verdicts of the default mode, which are computed without `rejected`.

## Consequences

- The security hunt's reverse comparison can run, declaration by declaration, on the conformance cases and examples whose information-flow rejections are its targets.
- Every existing consumer is unaffected. Plain `una core` is byte-identical, and the re-check's default verdicts are unchanged. The gate of the change checks both: a corpus run of the default mode identical to `main`'s, and the test of decision 5.
- The elaboration of a rejected declaration must not record anything that another pass reads. The change keeps it in its own call, after the existing elaboration, on the declarations decision 3 selects.
- Declarations rejected for a name or type error stay invisible to the reverse comparison. That is deferred: their Core may be incomplete, and a partial Core re-checked by the model would mean nothing.

## Alternatives considered

- **Mark rejected declarations inside `modules`** (a `rejected: true` key on each). Rejected: a consumer written against `una.core/1`, which ignores unknown keys, would read a rejected declaration as accepted Core. That is the one outcome the export must never allow.
- **A separate document**, for example `una.core-rejected/1`. Rejected: the reverse mode needs the accepted declarations as context, so it would read two documents of one package, which can drift apart. One document with a separate key keeps them together and changes nothing for current consumers.
- **Every declaration with a diagnostic**, of any domain. Deferred: a declaration with a name or type error may elaborate partially, and the model would check something that is not the program.
- **No flag: always print `rejected`.** Rejected: it would add work, and output, to every `una core` call, and consumers that do not ignore unknown keys, against ADR 0076, would break. The flag keeps the default output byte-identical.
