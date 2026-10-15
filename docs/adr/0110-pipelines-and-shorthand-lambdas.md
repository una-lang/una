# 0110. Pipelines and shorthand lambdas: method calls already chain

- Status: Proposed
- Date: 2026-10-15
- Deciders:
- Resolves: issue #35 ("Pipelines `|>` and `it` shorthand lambdas", `plan/backlog.md`, design D)
- Target: edition `0.3`. Editions `0.1` and `0.2` do not change, and the Gate 1 pin does not move.
- Amends, once accepted: `plan/backlog.md` (the row of #35), and the description of the `|>` shape that ADR 0096 rule 3 lists. No grammar, type rule or Core form changes.

## Context

The backlog row of #35 names two constructs from other languages: a pipeline operator `|>`, as in F#, Elixir and OCaml, and an `it` shorthand for one-parameter lambdas, as in Kotlin. Una's syntax is its own: a construct is added because Una needs it, never because another language has it, and it answers to the principles of charter §2. So this ADR asks what each option would give an agent writing Una, and what it would cost.

**What Una has today.**
- **Method-call syntax** (`spec/04 §3`). `e.f(args)` is `f(e, args)` when `e`'s type has no field `f` of function type, with `e` as the first, unlabeled argument. `e.f` is `f(e)` for a function of one parameter. A chain reads left to right: `orders.filter(keep: fn(o) -> o.paid).map(each: fn(o) -> o.total).sum`. Core IR has no method call; it is the call (`spec/09`).
- **Labeled arguments** (charter P5, `spec/04 §3`). The first argument may go unlabeled and binds the first parameter (ADR 0049), and every other one is labeled. In a method call the receiver is that first argument, so the rest of the chain is labeled as written.
- **Lambdas** (`spec/04 §6`). `fn(p) -> e`, with a body of one line. The standard library passes them under labels: `map(each:)`, `filter(keep:)`, `first_where(keep:)`, `fold(initial:, step:)` (`spec/14`).
- **One effect per line** (`spec/04 §4`, U3013). A capability operation, or a call that uses capabilities or can fail, is the whole expression of its line. So a chain can only hold pure calls, whatever its syntax.
- **One logical line** (`spec/15`). A line break ends a statement unless it sits inside brackets, and the formatter joins continued lines, since no line width applies.
- **`it` is a keyword.** It names the value that a `where` clause or a `checked` predicate constrains (`spec/03`). It is reserved everywhere else, and a binding named `it` is U0101 (#206).

**What agents write.** The measurement uses the corpus of ADR 0096: the 84 Una runs of una-bench `full-b` and `full-c`, read from their transcripts only. It counts what the agents wrote to `.una` files and the diagnostics they read.

| What | Occurrences | Runs (of 84) |
| --- | --- | --- |
| `\|>` written | 11 | 2 |
| U0101 on a line with `\|>` | 2 | 1 |
| an `it` or `_` shorthand in a lambda's place (`xs.map(it.total)`, `(_.f)`) | 0 | 0 |
| lambdas written (`fn(`) | 2,783 | 61 |
| of which passed under a label (`each: fn(`) | 2,758 | 60 |
| of which passed unlabeled after a method call (`.map(fn(`) | 1 | 1 |
| a call's result used as a receiver (`).f`), a chain | 509 | 36 |
| U2004 on a line with a lambda | 32 | 22 |
| U0101 on a line with a lambda | 104 | 29 |
| U3013 on a line with a chain | 2 | 1 |

- **No pipeline habit.** Ten of the 11 `|>` are one line of one run, rewritten across edits: `|> fn(new_refused) -> ShareSummary(…)`, which applies a lambda to the value before it.
- **No shorthand habit.** Agents name lambda parameters, and they label lambdas correctly: 2,758 of 2,783.
- **Chaining is already used,** in 36 runs.
- **The lambda errors are elsewhere.** The 32 U2004 on lambda lines are, in the cases read, an unlabeled second argument of a call *inside* the lambda's body (`fn(c) -> same_card(c, ending, card.expires)`), and one unlabeled `fold` step. A pipeline or a shorthand changes neither. The 104 U0101 are mostly a lambda whose body spans lines (ADR 0096's table: 78 occurrences in 28 runs), or `if`, `let` and `when` inside a lambda's body.

**Where the cost is.** ADR 0096's table puts the largest syntax cost on lines continued outside brackets: an `if … then … else` wrapped across lines, with 562 occurrences in 71 runs. A pipeline is idiomatically written one stage per line, each starting with `|>`, which is that same shape.

## Decision

1. **No pipeline operator.** `|>` is not added. Method-call syntax is Una's pipeline: `x.f(a: 1).g(b: 2)` is `g(f(x, a: 1), b: 2)`.
2. **No shorthand lambda.** `it` keeps its one meaning, the constrained value of a `where` clause or a `checked` predicate. No `_`, `$0` or other implicit parameter is added. A lambda names its parameters, `fn(o) -> o.total`.
3. **The `|>` shape gets Una's form as its description.** It stays U0101, with the description of ADR 0096 rule 3 and no edit: "Una chains calls with method syntax: `x |> f(a: 1)` is written `x.f(a: 1)`; to apply a lambda, name the value with `let`". There is no edit, because `.f` binds tighter than an operator, so the receiver may need parentheses, and applying a lambda has no method form.
4. **The backlog row of #35 closes as decided.** It moves to the decided rows with this ADR, so that the backlog no longer suggests that `|>` and `it` lambdas are planned.

A later proposal may reopen either construct with new evidence: agents that reach for it in a measured corpus, or a need that method calls and `let` cannot express.

## Consequences

- **Nothing to learn, nothing to fail.** No grammar, type rule, Core form or formatter rule changes. The guide keeps one way to chain calls. Charter P4 is kept: one canonical form, with no second spelling of a call.
- **Labels stay where they are.** With method calls, the receiver is the unlabeled first argument, and every other argument keeps its label (P5, ADR 0049). There is no second slot rule to learn.
- **Effects stay visible.** A chain holds only pure calls (U3013), and so would a pipeline. Each effect keeps its own line and its own handling clause (`if error`, `if none`).
- **Intermediate values keep their names.** Where a chain grows long, `let` names each step. The names appear in counterexamples, which bind source names (`spec/12 §4`, `§7`).
- **Deferred:** the multi-line lambda body, the largest lambda cost, which ADR 0096 already describes ("move the block into a named function"). A construct for it, if any, is a separate decision.

## Alternatives considered

**A. A pipeline operator, `x |> f(a: 1)` for `f(x, a: 1)`.**
- It is method-call syntax spelled with a sigil, against charter P3 (words over symbols), and it gives every call chain a second canonical candidate, against P4.
- Its usual layout is one stage per line with a leading `|>`. In Una a line break outside brackets ends the statement, so that layout is a syntax error of the most costly kind measured (562 occurrences of the wrapped `if`). Allowing it would need a continuation rule that `if`, `and` and every other operator would then ask for too.
- It inserts the value in the first, unlabeled slot, as method syntax does, so it adds no expressive power. A form that fills another slot needs a placeholder (alternative C).
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
