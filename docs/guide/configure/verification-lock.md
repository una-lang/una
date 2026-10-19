---
construct: "verification.lock"
editions: ["0.1", "0.2"]
spec: ["spec/13 §7", "spec/12 §9", "spec/12 §5", "spec/12 §10", "spec/13 §5"]
codes: [U5003, U7004]
---

# `verification.lock`

## What it does

Una proves a program's obligations, the facts each line needs to be safe, such as a value staying in its range, with a solver: a separate program, Z3, that decides whether such a fact always holds. `verification.lock` pins which Z3 and which settings, so that a proof means the same on every machine. It sits at the package root, beside `una.toml`, and is committed. Proofs already found are kept in the proof cache, a folder of saved answers keyed by the lock's contents, so changing the lock verifies every obligation again.

Learn more: [The manifest and the lock](/learn/packages/#s11-3) and [What the verifier proves](/learn/contracts/#s5-4).

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`verification.lock`

```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

`src/shop.una`

```una
module shop

pub kind Seats = Int in 0..5000

pub cap(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw
```

## Forms

```text
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "<a non-empty string: the exact Z3 version>"
logic            = "QF_UFLIA"
step_budget      = <an integer of at least 1>
random_seed      = <an integer of at least 0>
```

Exactly these six keys.

## Examples

A missing Z3, or Z3 at a version other than the one the lock names, is rejected once there is something to prove. Verification never passes silently. Here the lock names a version the machine does not have:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`verification.lock`

```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "4.12.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
```

`src/shop.una`

```una
-- fails with U5003
module shop

pub kind Seats = Int in 0..5000

pub cap(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw
```

## Edge cases

**Exactly the six keys**:

`una.toml`

```toml
[package]
name = "shop"
version = "0.1.0"
edition = "0.2"
```

`verification.lock`

```toml
encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
```

`src/shop.una`

```una
-- fails with U7004
module shop

pub kind Seats = Int in 0..5000

pub cap(raw: Int) -> Seats = if raw < 0 then 0 else if raw > 5000 then 5000 else raw
```

Further rules:

- **A single file** uses the default lock, the lock built into `una`, whose solver version `una --version` prints.
- **Determinism.** For identical sources, `una.toml` and `verification.lock`, checking produces identical diagnostics, statuses and reports, except obligations that ran out of wall-clock time.

## Diagnostics

- [U5003](../../../crates/una-explain/explain/U5003.md): a missing Z3, or Z3 at a version other than the one the lock names.
- [U7004](../../../crates/una-explain/explain/U7004.md): a missing lock, a syntax error, a missing or unknown key, or a wrong value.

## Edition

The lock is the same in editions `0.1` and `0.2`.

## Normative source

- `spec/12 §9`: the lock file and cache; `spec/12 §5`: the solver protocol; `spec/12 §10`: determinism.
- `spec/13 §7`: project files; `spec/13 §5`: `una --version` and the default lock.
