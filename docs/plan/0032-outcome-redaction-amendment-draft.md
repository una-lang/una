# ADR 0032 amendment draft, release line v0.1: an operation's outcome rendered at its label (S-45)

Status: Proposed (draft for the project's decision; it is appended to docs/adr/0032-runtime-details-left-open.md only once accepted, with the date of its acceptance). Backport of the security fix of #1552 (S-45) to this line, whose only edition is `0.1`. The behavior is implemented in the pull request that carries this draft. Until it is accepted, `spec/11 §5`, `spec/07 §4` and `spec/09 §4` cite it as proposed. The same amendment is drafted on `main` for editions `0.1` to `0.4`.

## Amendment (proposed 2026-11-04): an operation's outcome rendered at its label (S-45)

Rule 3 says that "an operation's outcome is redacted by the label of the binder of its result", and the interpreter implemented it as a redaction of the `ok` value only: the outcome's kind (`ok` or `fail`), the failure's variant and its payload were written in clear. Since ADR 0085 (S-14, v0.1.6) whether a `Files` operation fails is labeled data: its decision carries the file-state label and the label of its path. The checker held that label at every operation site, but the trace writer never consulted it.

On v0.1.13 a `Files.write` of an `internal` path at a `public` program counter, whose path the trace shows as `<redacted:internal>`, is recovered exactly from the outcomes of later public `Files.read`s (`ok` or `fail NotFound`), and from its own outcome when the path names a directory that does not exist; with k writes and k reads, k bits of an `internal` value. The program cannot observe these outcomes (U4003 on a branch that prints); only the trace does.

Rule 3 now reads, for the trace of `una run`:

> **Outcomes.** An operation's outcome is rendered at its **outcome label** `L_out`: the label of the decision whether the operation fails and of its payload (`spec/07 §7` rule 4: the deciding arguments capped at `accepts`, and the file-state label for `Files`, ADR 0085), joined with the labels of its failure type and of its result's binder. When `L_out` is above `public`, the event's outcome is `{"kind":"redacted","level":"<level>"}`, with the level of `L_out` only: whether the operation returned or failed, the failure's variant, its payload and the value are not written, and no text is built from the value. When `L_out` is `public`, the outcome is rendered as before. `aborted` is not a decision of the operation and keeps its kind.

`L_out` is static: the checker computes it once per operation site in its second pass, and elaboration carries it in Core IR as the `outcome` label of the operation (`spec/09 §4`). An operation the checker left no label for, and one in a body that no label rule checks (a policy's `via` function), has `L_out = ⊤`, so the backport fails closed. Whether an outcome is redacted therefore depends on no value.

**Examples.** An example's trace keeps its outcomes in clear: an example reaches no host capability (`spec/11 §6`), its data come from its source, and `expect performed` and the reasons of a failed example read them. `una test --json` is therefore unchanged.

**Soundness argument** (not a theorem). The outcome the trace writes is the value the program receives. Its decision and payload carry `L_out`'s decision part and its value the binder's label; T6 (`spec/07 §1`) says the parts labeled `public` are equal in two runs whose inputs agree at `public`. Writing exactly those parts, and the constant `redacted` with a static level otherwise, adds nothing a `public` observer of the program could not already see. Out of scope on this line: whether an event occurs at all, which an operation under a raised program counter decides, is the trace's shape, recorded by occurrence as `spec/11 §5` specifies here.

## Consequences

- Nothing stops compiling, no check is weakened, and the edition's language is unchanged: the change is in what the trace of a run shows.
- **The cost is trace observability.** The outcome of every `Files.read` (`internal Text`), `Env.get` (`Option[internal Text]`) and `Keys.hmac` (`internal Hashed`), of every handler operation whose result or failure type is labeled, and of every `Files` operation of a package whose file-state label is above `public`, is `redacted` in a run's trace. `Console.print`, `Clock.now`, handler operations whose result and failure type are `public`, and `Files.write` in a package whose writes are all `public` keep their outcomes in clear.
- Readers of run traces must accept the kind `redacted`, among them consumers pinned to this line.
- Complexity: O(1) per operation site at check time (one label recorded), O(1) per event at run time (one test of a static label), O(S) space for S operation sites.
