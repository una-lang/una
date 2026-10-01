# 0049. An unlabeled first argument binds the first parameter

- Status: Accepted
- Date: 2026-09-30
- Deciders: Leonardo (answer A to issue #68, relayed by the coordinator session, 2026-09-30)
- Resolves: issue #68 (UI-009, found by the Gate 1 task builders)
- Amends: `spec/04 §3` rules 2 and 3; ADR 0024 rule 12; switches D-064 to its option B

## Context

`spec/04 §3` rule 2 lets the first argument of a call omit its label, and rule 3 counts a bare identifier equal to a parameter's name as labeled with it. The spec did not say which rule wins when the first argument is a bare identifier equal to the name of another parameter. D-064 chose rule 3, so with `minus(a: Int, b: Int)`, the call `minus(b, a)` meant `minus(a: a, b: b)`: a correct-looking call computed something else, with no diagnostic (issue #68). TypeScript and Rust have no punning, so an agent reads `minus(b, a)` as positional.

## Decision

1. In a call of a function, operation, handler or policy, an unlabeled first argument always binds the first parameter, whatever its name. Punning applies only from the second argument on.
2. So `minus(b, a)`, with parameters `(a, b)`, binds `b` to `a`, and the punned `a` gives `a` a second time: U2006. Rule 3's example is unchanged: `record(key, decision)`, with parameters `(key, decision)`, binds `key` first and puns `decision`.
3. Constructors keep `spec/04 §3` rule 4: with two or more fields, every argument is labeled or punned, including the first.
4. `spec/04 §3` rules 2 and 3 state it.

## Consequences

- A call that relied on punning its first argument into another parameter is rejected with U2006 instead of computing something else. No example or guide snippet does this.

## Alternatives considered

- **Reject such a first argument as ambiguous.** Not chosen: the positional reading is what agents expect, and the duplicate it creates is already U2006.
- **Keep D-064's option A.** Not chosen: it computes a different call from the one written, silently.
