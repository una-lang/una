# Escalations

## E1. Operation arguments and the labels of the operation's parameter types (F-001)

**Question:** How should an operation argument relate to the label structure of the operation's declared parameter type?

**Context:**
- `spec/07 §7` rule 4 says the labels of an operation's parameter types do not bound its arguments, because `accepts` bounds them. Otherwise `Files.write`, which accepts `internal` content declared `Text`, could never receive internal data.
- That reasoning holds for data positions, but a handler body assumes the declared type everywhere.
- At a contravariant position, as in `f: fn(secret Int) -> Int`, the handler may pass secret data to the argument. At an invariant kind argument, as in `Sink[secret Int]`, it may do the same.
- A public function from the caller then returns the secret as public, and F-001 prints a PIN.
- Handler instance arguments already get the full flow check, so only operation arguments are affected.

**Options:**
- **A (recommended).** An operation argument flows (rule 2) into the declared parameter type, with every covariant data position of that type taken as `⊤` for the comparison, so `accepts` alone keeps bounding data. Contravariant positions and invariant kind arguments are compared as at any other call argument. Every program that is sound today stays accepted, `Files.write` is unchanged, and the check reuses the flow code (`call.rs:688`).
- **B.** In v0.1, reject operation parameter types that contain a function type or an invariant kind argument (U2002 at the parameter). This is the smallest change, but it forbids callbacks through capabilities, and the spec allows those today.
- **C.** Use the full flow into the declared type plus `accepts`. This breaks rule 4's own `Files.write` case, so it is not recommended.

**Recommendation:** A, with an ADR that amends `spec/07 §7` rule 4 and ADR 0046 rule 3, and an `op argument` row in the flow and variance matrices of the closure plan (§3).

**Blocks the tag:** yes. F-001 is Critical under the owner's rule.
