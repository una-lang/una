# 0082. One code for an operation a capability does not declare

- Status: Proposed
- Date: 2026-10-08
- Deciders: Leonardo (owner); proposed by Claude (Una-1) at the coordinator's request
- Resolves: issue #42 (v0.1 review R-028)
- Amends, if accepted: ADR 0028 rule 5 and `spec/05 §9` rule 5

## Context

**Two codes, one message.** A name after `@x.` that the capability `C` of `@x` does not declare gives the message "capability `C` has no operation `op`" in three places:

| Where | Code | Repair | Rule |
| --- | --- | --- | --- |
| A call in a body, `@log.erase()` | U1001, unresolved name | the closest operation, as an edit (`write `flush``) | `spec/10 §11` |
| An example's `expect performed @log.erase` or `expect not performed @log.erase` | **U3002**, operation not permitted by capability type | none | ADR 0028 rule 5, `spec/05 §9` rule 5 |
| A handler's `op erase(...)` for a `Log` without `erase` | U3010, handler does not match its capability | the closest operation, as an edit | `spec/05 §5` |

The same mistake, a misspelled or invented operation, is U1001 in a body and U3002 in an example. The v0.1 review (R-028) found the two rules disagree and proposed U1001 everywhere. D-075, which chose the example rule, was about host capabilities and a missing `given`. It did not weigh the misspelled operation, and its answer kept the whole option.

Measured with the v0.3.3 release, in edition `0.1` and `0.2` alike:

```una
pub note() -> Unit
  uses @log: Log[write]
=
  @log.write("x")

example "an operation the capability lacks"
  given @log = quiet()
  do note()
  expect performed @log.erase
```

This gives U3002 "capability `Log` has no operation `erase`", while `@log.erase()` in a body gives U1001 with the same text.

**Why it matters to an agent.** A code is how an agent picks a fix.
- **U3002 means a permission** (`spec/05 §3`): `@x` holds `C[ops]`, and the operation is not among `ops`. Its rank-1 repair widens the `uses` clause, which the Gate 1 notes show agents apply mechanically (una-bench `findings/notes/notes-B08.md` to `B12.md`). For an operation that does not exist, widening a `uses` clause is not a fix: an agent that follows the code goes the wrong way, and the diagnostic offers no edit.
- **U1001 means a name that resolves to nothing,** whose fix is spelling. It already carries the closest operation as an edit.
- The dogfood passes met this case once:
  - The voided run B of N-64 wrote `@keys.zzz()` in a body, got U1001, and noted "U1001 on a capability does not list its operations" (`docs/reviews/v0.3.4/dogfood-fresh/voided-B/stock/LOG.md`).
  - No run met an undeclared operation after `expect performed`. Both runs wrote few examples that name an operation.

**Editions and Gate 1.**
- Edition `0.1` is frozen: a package of edition `0.1` keeps the language of v0.1 in every later compiler (ADR 0056). A diagnostic code is not the language: the program is rejected in every option below, and ADR 0076 makes `code` an open set of the Agent ABI. #362 already changed edition `0.1` messages without an edition gate.
- Gate 1 (una-bench) pins v0.1.5, so no option changes a Gate 1 run already made. Its notes rely on U3002 for an attenuated binding (an operation `C` declares and `@x` does not hold), which every option keeps. No task of the suite names an undeclared operation after `expect performed`: three `.una` files use `expect performed`, each with an operation its capability declares.

## Options

### A. One code, U1001, at both sites (recommended)

`expect performed @x.op` and `expect not performed @x.op` give U1001 for an `op` that the capability of `@x` does not declare. The message and the closest-operation edit are the body's. An example names a `given` handler, which no `uses` clause attenuates, so U3002 is no longer given at an example. It keeps exactly its meaning in bodies and calls: an operation the capability declares and the binding does not permit.

- **Agents:** one mistake, one code, one fix. U3002's widening repair is never offered where it cannot apply. The example gains the edit it lacks today.
- **Edition 0.1:** the code changes from U3002 to U1001 at this one site, for a program that is rejected either way. That is the kind of change #362 made to messages, with no edition gate. A gate is possible, but it would keep the misleading code in the frozen edition for no benefit, since no `0.1` program can depend on which rejection it gets.
- **Gate 1:** no effect. The pin is v0.1.5, and the suite has no such example. A later re-pin would see U1001 at a site no task reaches.
- **Cost:** `performed` in `check.rs` uses U1001 and the shared message, with `closest` for the edit. `spec/05 §9` rule 5 and ADR 0028 rule 5 are amended by a dated amendment. U3002's explain text drops its sentence on `expect performed`. A conformance case covers both sites.

**U3010 stays as it is.** A handler that implements an operation its capability lacks is a mismatch between two declarations, the handler and the capability. That is not a name that resolves to nothing, and U3010 already carries the closest-name edit.

### B. Keep both codes, with cross-references in the messages

U3002 stays at the example and U1001 in the body, but each message says what the operation is and how to fix it:
- the example's U3002 adds the closest operation as an edit;
- each message ends "(an operation the capability does not declare)".

- **Agents:** better than today, since the edit fixes the common case. But the code still points an agent that keys on codes to U3002's permission fix, and a consumer that groups by code sees one mistake under two codes.
- **Edition 0.1:** messages and repairs change, codes do not.
- **Gate 1:** no effect.
- **Cost:** small. No ADR 0028 amendment is needed, only message text and a repair.

### C. Status quo

- **Agents:** the example's U3002 has no edit and suggests the wrong fix. It is rare: the dogfood passes never met it.
- **Edition 0.1, Gate 1:** no change.
- **Cost:** none. #42 closes as decided.

## Recommendation

A, with no edition gate.
- It is R-028's own proposal, and it makes the code mean one thing at every site.
- It costs one call site, a dated amendment and a test.
- It changes nothing a frozen-edition program or Gate 1 can observe beyond which code rejects a program that is rejected anyway.

B is the fallback if the owner prefers not to touch a code that edition `0.1` has given since v0.1.

## Consequences (if A is accepted)

- `spec/05 §9` rule 5 and ADR 0028 rule 5 get a dated amendment. The accepted text is untouched (`docs/adr/` immutability).
- `expect performed` and `expect not performed` with an undeclared operation give U1001 and the closest-operation edit in both editions. U3002 is no longer given at an example; in bodies and calls it is unchanged.
- A conformance case shows the same U1001, with its edit, in a body and after `expect performed`.
- Separately, and in any option: the U1001 of an undeclared operation could list the capability's operations when no close name exists, as the dogfood note asks. That is message text only, and it can follow on its own.
