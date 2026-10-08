# Dogfood pass: five edition 0.2 programs from the guide alone

- **Decision:** N-62. **Writer:** Claude (Una-1), acting as a fresh agent.
- **Toolchain:** a debug build of main `1eee9fa`, plus #400, which changes only the counterexamples of zero divisors and touches none of these programs.
- **Sources allowed:** `docs/guide/una-for-agents.md`, `docs/guide/syntax-reference.md` and `una explain`. No spec, ADR or compiler source.
- **Loop for each program:**
  1. Write it from the guide.
  2. `una fmt`, then `una --json check`.
  3. Read every diagnostic, then `una explain` where the message left me unsure.
  4. Fix, and repeat until the check is clean and the examples pass.
- **The final programs:** under `dogfood/`. Each is a package of edition `0.2` that checks clean at tier 1, and whose examples pass.
- **Caveat:** I know Una from working on it, so this is a lower bound on what a new agent meets: I may have avoided mistakes a new agent would make. I record what the documents and the messages gave, not what I knew.

## Summary

| Program | Iterations to clean | Diagnostics met | Message plus `una explain` enough? |
| --- | --- | --- | --- |
| `greet`, a CLI that reads `Env` and prints | 2 | U4003 | No: the fix, a policy in a trusted module, is only in guide §9 |
| `orders`, a JSON transform | 3 | U0101 ×2, U0004 ×2, U0001 ×2, U0006 ×2, then U1002 | No for the first: the cascade misleads, and the escape is only in the syntax reference. Yes for U1002 |
| `stock`, an HTTP client with a secret header | 2, plus 1 to add an example with a handler | U4001 | Partly: the message is exact, but its advice cannot apply; guide §17 has the fix |
| `tally`, a `Map` tally with a contract | 4 | U2002 ×3, then U5002 and U5001 ×2, then U0101 ×2 | U2002 yes, with an edit. U5002 no. U5001 yes. U0101 yes |
| `sensors`, a `together` fan-out with failures | 1 | none | Yes: written and passing on the first try |

12 iterations in all. Of the eight problems met:
- 4 were fixed from the message alone (U1002 shadowing, U2002, U5001, `edit` placement);
- 2 needed the guide (U4003, U4001);
- 1 needed the syntax reference (the U0101 to U0006 cascade);
- 1 needed a pattern that neither document gives (U5002 across a loop).

## Findings, ranked by what they cost an agent

Cost weighs the iterations lost, whether the diagnostics lead to the fix or away from it, and how much an agent must read or restructure.

### D-1. High: a literal `{` in text gives a cascade of eight misleading errors

`orders`, iteration 1. A test input written as JSON, `summary("{\"lines\": [...]}")`, gave eight diagnostics on two lines. The messages point away from the cause:
- U0101 "unclosed `(` before the end of the file";
- U0004 "unterminated text literal";
- U0001 "invalid character `\`";
- U0006 "text literal inside interpolation; bind it with `let` and interpolate the name";
- U0101 "unmatched `)`".

The cause is one character: `{` opens an interpolation, so `\"` lands inside an expression. None of the eight says so, and U0006's advice, to bind the inner text with `let`, sends an agent off course.

The guide says only "a literal `}` in text is `\}`" (§3). It does not mention `\{` or `\"`. The syntax reference lists every escape (`\"`, `\\`, `\n`, `\t`, `\{`, `\}`, `\u{H}`). JSON in text is common in agent work: test inputs, request bodies, expected outputs.

Suggested:
- When an interpolation starts with `\` or `"`, report one diagnostic at the `{`: "`{` starts an interpolation; write `\{` for a literal brace". Give it the edit, and drop the cascade.
- Make §3 of the guide list the escapes, `\{` first.

### D-2. High: data from `Env` or `Files` cannot be printed, and only the guide shows the fix

`greet`, iteration 1. Printing anything derived from `@env.get`, even the length of the name, is U4003. The message is right ("declassify it through a policy first").
- `una explain U4003` fixes its own example by removing the `secret` label from a parameter. An agent cannot do that with host data: `Env.get` and `Files.read` return `internal` text.
- The fix is three changes in two files, all in guide §9:
  - a module headed `trusted module`;
  - `[trusted] modules` in `una.toml`;
  - a `policy` with `from`, `to`, `via`, `purpose` and `owner`, and its private `via` function, imported where it is used.

For the most common first program, a CLI that reads its environment, the diagnostics alone do not lead there.

Suggested:
- U4003's explain text gains a second fixed example, a policy for `internal` host data.
- The guide's §8 or §9 opens with "printing what you read from `Env` or `Files` takes a policy", with the pattern.

### D-3. Medium-high: contracts across a loop fail with a message an agent cannot act on

`tally`, iteration 2. `ensures result.len <= votes.len` on a `for` loop over `edit counts.set(...)` is U5002, with this reason: "the loop forgets every fact about `counts` but the refinements of its type, and its length when the loop only `set`s a list, as `spec/17 §5` rule 3 says, so the counterexample sets `counts` freely; guard the use with an `if`, or check the value with `checked`".
- It cites the spec, which an agent working from the guide does not read.
- It speaks of a list's length while `counts` is a map.
- Its advice, a guard or `checked`, does not prove the postcondition.

Guide §11 states the rule ("no other fact about them survives the loop"), but no pattern for what to do instead. What worked was to move the contract to the per-iteration helper: `add_vote(edit counts, vote)` with `ensures counts.has(key: vote)`, which the solver proves and a caller then uses.

Suggested:
- A guide paragraph in §11 with that pattern.
- A U5002 reason that drops the spec reference and the list wording when the binding is not a list, with a repair that names the helper pattern.

### D-4. Medium: `Map[Text, Nat]` is rejected, and the forced fix loses the refinement

`tally`, iteration 1. `Map[Text, Nat]` is U2002: "this type argument is the refined kind `Nat`, and a refined type cannot instantiate a type parameter". The message is excellent and carries an edit to `Int`. But:
- **The guide never says it.** §4 lists `Nat` among the built-in types, and §14's examples use `Map[Text, Int]` without saying why.
- **The forced `Int` drops the guarantee.** In iteration 2, `votes_for(...) -> Nat` and `ensures result >= 0` were refuted with `result = -1`, since map values are now any `Int`. That is correct, but it is a second round trip the first message only hints at ("check the refinement where the value is used").

Suggested: a sentence in §4 or §14 saying that refined kinds (`Nat`, `Int in 0..5`) cannot be type arguments, so write the base type and refine at the use.

### D-5. Medium: a `secret` header value gives advice that cannot apply

`stock`, iteration 1. `Header(name: "X-Api-Token", value: token)` with `token: secret Text` is U4001: "field `value` of `Header` is `internal Text`, and this value is `secret Text`". The message is exact.
- `una explain U4001` says "raise the declared label, or lower the value's label through a policy". An agent cannot raise a prelude kind's field.
- Guide §17 has the answer: a policy to `internal`, or `key_name` for a bearer token. The diagnostic does not point to either.

Suggested: when the declared position belongs to a prelude kind or a host operation, U4001's message or help names the policy and, for `Header`, `key_name`.

### D-6. Low: the guide and the syntax reference are stale on U1002 for built-in names

- Guide §21 says that in edition `0.2`, "`I8` to `U64` become built-in types, so a user kind with one of those names is U1002". The syntax reference (line 163) says the same of `Eq`, `Ord`, `Integer` and `I8` to `U64`.
- Since ADR 0079 (#130), every built-in type name is U1002 for a kind, variant or capability in edition `0.2`: `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option`, `Result` and `Map`.
- Not met in these programs, but an agent migrating a package with a `kind Result` would be surprised.

Suggested: update both lists.

### D-7. Low: the no-shadowing rule is not in the guide

`orders`, iteration 2. A pattern `is Array(items)` under an outer `let items = ...` is U1002: "`items` is already bound in this scope; a local cannot rebind another". The message is clear and the fix took one rename. But neither document states that a local name cannot shadow another, which agents coming from most languages assume they can do.

Suggested: one line in guide §5.

### D-8. Low: `edit` placement on a call

`tally`, iteration 3. `edit add_vote(counts, vote)` is U0101: "`edit` marks an argument, as in `f(edit x)`, or the receiver of a method call, as in `edit total.record(e)`". The message is clear and enough.
- The guide's §11 example writes `edit tally.record(amount)` (receiver form). Next to it, the function form `record(edit tally, amount)` is easy to get wrong.
- There is no mechanical edit, though one is unambiguous: move `edit` to the first argument.

Suggested: the edit.

## What worked first time, worth keeping

- **`together`** (§18): the fan-out, the first-failure-wins rule and `attempt` per task were right on the first write. The example's trace shows each task's events with its path.
- **A handler for `Http` in an example** (§17), from the operation signatures in the guide. The trace redacts the `internal` header value.
- **JSON parsing with defaults** (§15): the "parse, then branch" pattern worked as written, and building output with `empty_map().with(...)` inferred its types.
- **Mechanical edits:**
  - U2002's edit to `Int` was right;
  - the canonical form from `una fmt` was always accepted.
- **Every message was one sentence that named the construct.** The costly ones (D-1 to D-5) cost because of where they point, not because they were vague.

## The programs

| Case | What it shows |
| --- | --- |
| `dogfood/greet/` | `Env` read, a policy from `internal Text` to `public Nat` in a trusted module, and `[run] grants`. `una run` prints `hello, your name has N letters` |
| `dogfood/orders/` | JSON parse, walk and write, with text that holds JSON (`\{`, `\"`). The example passes |
| `dogfood/stock/` | `Http.get` with a custom header declassified from `secret` to `internal`, and an `Http` handler in an example. The example passes |
| `dogfood/tally/` | A `Map` tally in a loop, a proven `ensures counts.has(key: vote)` on the update helper, and a caller that discharges a `requires` from it. The example passes |
| `dogfood/sensors/` | `together for` with `if error pass`, and with `attempt`, over a capability that fails. Three examples pass |
