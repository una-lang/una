# 0110. Pipelines and shorthand lambdas: method calls already chain

- Status: Accepted
- Date: 2026-10-15
- Deciders: Una project (accepted on 2026-10-15)
- Resolves: issue #35 ("Pipelines `|>` and `it` shorthand lambdas", `plan/backlog.md`, design D)
- Target: none: no language rule changes in any edition. Only the backlog and the description of one diagnostic change, and the Gate 1 pin does not move.
- Amends: `plan/backlog.md` (the row of #35), and the description of the `|>` shape that ADR 0096 rule 3 lists. No grammar, type rule or Core form changes.

## Context

The backlog row of #35 names two constructs from other languages: a pipeline operator `|>`, as in F#, Elixir and OCaml, and an `it` shorthand for one-parameter lambdas, as in Kotlin. The pipelines disagree on where the value goes: Elixir passes it as the first argument, while F# and OCaml apply the function to it last, so `x |> f a` is `f a x`. Una's syntax is its own: a construct is added because Una needs it, never because another language has it, and it answers to the principles of charter §2. So this ADR asks what each option would give an agent writing Una, and what it would cost.

**What Una has today.**
- **Method-call syntax** (`spec/04 §3`). `e.f(args)` is `f(e, args)`, with `f` resolved in scope and `e` as the first, unlabeled argument, unless `e`'s type has a field `f` of function type, which it then calls. A qualified function, a constructor (`Some`, `Ok`) and a capability operation have no method form. `e.f` is `f(e)` for a function of one parameter. A chain reads left to right: `orders.filter(keep: fn(o) -> o.paid).map(each: fn(o) -> o.total).sum`. Core IR has no method call; it is the call (`spec/09`).
- **Labeled arguments** (charter P5, `spec/04 §3`). The first argument may go unlabeled and binds the first parameter (ADR 0049), and every other one is labeled. In a method call the receiver is that first argument, so the rest of the chain is labeled as written.
- **Lambdas** (`spec/04 §6`). `fn(p) -> e`, with a body of one line. The standard library passes them under labels: `map(each:)`, `filter(keep:)`, `first_where(keep:)`, `fold(initial:, step:)` (`spec/14`).
- **One effect per line** (`spec/04 §4`, U3013). A capability operation, or a call that uses capabilities or can fail, is the whole expression of its line. So a chain can only hold pure calls, whatever its syntax.
- **One logical line** (`spec/15`). A line break ends a statement unless it sits inside brackets, and the formatter joins continued lines, since no line width applies.
- **`it` is a keyword.** It names the value that a `where` clause or a `checked` predicate constrains (`spec/03`). It is reserved everywhere else, and a binding named `it` is U0101 (#206).

**What agents write.** The measurement uses the corpus of ADR 0096: the 84 Una runs of una-bench `full-b` and `full-c`, read from their transcripts only. It counts what the agents wrote to `.una` files, through `Write` (162 writes), `Edit` (1,858) and shell heredocs (6), and the diagnostics they read, each matched by the source line that the diagnostic prints. `|>` is counted outside comments and text literals.

| What | Occurrences | Runs (of 84) |
| --- | --- | --- |
| `\|>` written | 11 | 2 |
| U0101 on a line with `\|>` | 2 | 1 |
| an `it` or `_` shorthand in a lambda's place (`xs.map(it.total)`, `(_.f)`) | 0 | 0 |
| a lambda parameter named `it` (`fn(it) ->`) | 0 | 0 |
| lambdas written (`fn(`) | 2,783 | 61 |
| of which passed under a label (`each: fn(`) | 2,758 | 60 |
| of which passed unlabeled after a method call (`.map(fn(`) | 1 | 1 |
| a call's result used as a receiver (`).f`), a chain | 509 | 36 |
| U2004 on a line with a lambda | 32 | 22 |
| U0101 on a line with a lambda | 104 | 29 |
| U3013 on a line with a chain | 2 | 1 |
| a chain continued on a line that starts with `.` | 16 | 3 |
| U0101 on such a line | 3 | 3 |

- **No pipeline habit.** Ten of the 11 `|>` are one line of one run, rewritten across edits: `|> fn(new_refused) -> ShareSummary(…)`, which applies a lambda to the value before it.
- **No shorthand habit.** Agents name lambda parameters, and they label lambdas correctly: 2,758 of 2,783. None names a lambda parameter `it`, the form a Kotlin habit would take in Una.
- **Chaining is already used,** in 36 runs.
- **The lambda errors are elsewhere.** All 32 U2004 on lambda lines were read. 28 are an unlabeled argument of a call *inside* the lambda's body (`fn(c) -> same_card(c, ending, card.expires)`), 3 are another unlabeled argument on the same line (`fold(xs, 0, step: …)`), and 1 is the lambda itself passed unlabeled (`fold(seed: initial, fn(acc, doc) -> …)`). A pipeline or a shorthand changes none of them. The 104 U0101 are mostly a lambda whose body spans lines (ADR 0096's table: 78 occurrences in 28 runs), or `if`, `let` and `when` inside a lambda's body.

**The limit of this evidence.** These agents worked with Una's guide in their workspace, which shows method calls and labeled lambdas and never `|>` or a shorthand. So the counts show what agents write given Una's documentation, which is the setting Una is designed for, and not what they would reach for without it. A corpus of unprimed runs could change the counts of the first rows.

**Where the cost is.** ADR 0096's table puts the largest syntax cost on lines continued outside brackets: an `if … then … else` wrapped across lines, with 562 occurrences in 71 runs. A pipeline is idiomatically written one stage per line, each starting with `|>`, and so is a method chain in the languages agents come from, each stage starting with `.`. In Una both are that same shape. For chains it is measured: 16 continuation lines in 3 runs, with 3 U0101.

## Decision

1. **No pipeline operator.** `|>` is not added. Method-call syntax is Una's pipeline: `x.f(a: 1).g(b: 2)` is `g(f(x, a: 1), b: 2)`.
2. **No shorthand lambda.** `it` keeps its one meaning, the constrained value of a `where` clause or a `checked` predicate. No `_`, `$0` or other implicit parameter is added. A lambda names its parameters, `fn(o) -> o.total`.
3. **The `|>` shape gets Una's form as its description.** It stays U0101, with the description of ADR 0096 rule 3 and no edit: "Una has no `|>`: pass the value as an argument, `f(x, a: 1)`, or as the receiver of a method call, `x.f(a: 1)`, when `f` is a function in scope; to apply a lambda or a constructor, name the value with `let`". The description names no slot for `x` beyond the first, so it does not assume Elixir's reading over F#'s. There is no edit: which slot the writer meant is not in the source, `.f` binds tighter than an operator so the receiver may need parentheses, and a qualified function, a constructor or a lambda has no method form.
4. **The backlog row of #35 closes as decided.** It moves to the decided rows with this ADR, so that the backlog no longer suggests that `|>` and `it` lambdas are planned.

A later proposal may reopen either construct with new evidence: agents that reach for it in a measured corpus, or a need that method calls and `let` cannot express.

## Consequences

- **Nothing to learn, nothing to fail.** No grammar, type rule, Core form or formatter rule changes. The guide keeps one way to chain calls. Charter P4 is kept: one canonical form, with no second spelling of a call.
- **Labels stay where they are.** With method calls, the receiver is the unlabeled first argument, and every other argument keeps its label (P5, ADR 0049). There is no second slot rule to learn.
- **Effects stay visible.** A chain holds only pure calls (U3013), and so would a pipeline. Each effect keeps its own line and its own handling clause (`if error`, `if none`).
- **Intermediate values keep their names.** Where a chain grows long, `let` names each step. The names appear in counterexamples, which bind source names (`spec/12 §4`, `§7`).
- **What method calls cost.** A chain written one stage per line, with a leading `.`, is a syntax error, as any line continued outside brackets is (`spec/15`): 16 lines in 3 runs, 3 diagnostics. A long chain is written on one line, or broken into `let` steps. This cost is the chosen form's own, and it is no larger with method calls than it would be with `|>`.
- **Deferred:** the multi-line lambda body, the largest lambda cost, which ADR 0096 already describes ("move the block into a named function"), and the leading-dot continuation above, which ADR 0096's list of shapes could take in a later amendment, with a description or an exact join. A construct for either is a separate decision.

## Alternatives considered

**A. A pipeline operator, `x |> f(a: 1)` for `f(x, a: 1)`.**
- It is method-call syntax spelled with a sigil, against charter P3 (words over symbols), and it gives every call chain a second canonical candidate, against P4.
- It must choose a slot. Elixir's agents expect the first, F#'s and OCaml's the last. With the first, it is method syntax again; with the last, it reverses the reading of every call whose first parameter is the subject, as in the whole standard library (`spec/14`).
- Its layout, one stage per line, needs a continuation rule that Una does not have. Method chains share that cost (Consequences), so it does not weigh against `|>` alone; but adding `|>` with its layout would need the rule, and `if`, `and` and every other operator would then ask for it too.
- In the first slot it adds no expressive power over method syntax. A form that fills a labeled slot needs a placeholder (alternative C).
- With effects it adds nothing: a stage that performs an effect cannot sit inside a chain (U3013).
- Agents wrote it 11 times, in 2 runs of 84.

**B. An `it` shorthand for one-parameter lambdas, `xs.map(each: it.total)`.**
- `it` already means the constrained value in `where` and `checked`. A second meaning in a body would make one keyword name two things, and which one a given `it` means would depend on where it stands.
- A reserved word used as a name is already a measured mistake (ADR 0096: 75 occurrences in 43 runs, `it` among them). A shorthand would make `it` legal in some expressions and not in others, which the parser could not explain by one rule.
- Without the `fn(o) ->`, a lambda is no longer visible as one, and its parameter has no name for a counterexample or a repair to print.
- Agents wrote no shorthand in 84 runs, and labeled 2,758 of 2,783 lambdas correctly. What they get wrong in lambdas is a multi-line body and an unlabeled argument inside the body, and a shorthand fixes neither.

**C. A label-aware pipeline, `x |> f(into: _, key: k)` or `x.f(into: ., key: k)`, which places the value in a named slot.**
- It is the one form that method syntax cannot express: a value that is not the first parameter. The standard library puts the subject first (`spec/14`), and in the corpus the need did not appear.
- It needs a placeholder. `_` is the start of a name, the third most frequent syntax mistake (116 occurrences in 48 runs), and is refused today (U0001). Reusing it as a placeholder would make `_` legal in one position only.
- `let` with a labeled call says the same with a name: `let entry = …` then `insert(key: k, into: entry)`.

**D. Keep method calls, and add nothing (chosen).** Method calls chain left to right, keep labels and effects as they are, and are what agents already write.
