# 0109. Profiles `app` and `systems`

- Status: Proposed
- Date: 2026-10-15
- Deciders:
- Resolves: issue #22 (the design of the profiles; issue #31 implements it)
- Target: edition `0.3`, as a preview construct (`spec/13 §7`). Editions `0.1` and `0.2` do not change, and the Gate 1 pin does not move.
- Amends, once accepted: `spec/13 §7` (the `[package] profile` key and the manifest's `[run] max_memory` key), `spec/18 §3` (the `overflow` obligation on `Int` in a `systems` package), `spec/16 §4` rule 2; a new section of `spec/16` holds the rules.

## Context

The roadmap lists two profiles for v0.2, `app` and `systems` (`docs/plan/roadmap.md`), and the lexical grammar reserves `profile` and `simd` "for the systems profile" (ADR 0014). Nothing defines them yet. Issue #22 asks what a profile changes: allowed features, defaults, runtime checks, the gates for foreign code (FFI, #33) and SIMD, and the interplay with editions. The performance track (ADRs 0106 to 0108, #737) raises the same question from the other side: may a profile buy speed by turning a check off?

The constraints:

- **No weakening, ever.** Una's safety properties (memory safety, the information-flow theorem T6, capability authority, contracts and refinements, bounds, the memory and depth budgets) hold for every package. A profile that switched one off would make "verified Una" mean two things, and every claim of the charter would need "unless the profile says otherwise". The project's rule is to pick the most secure option.
- **A check is removed only by a proof.** The native backend keeps every run-time check that the verifier has not proven unnecessary (#737). A profile is a declaration, not a proof, so it removes none.
- **Editions stay frozen.** Editions `0.1` and `0.2` are fixed; edition `0.3` is a preview that may only get stricter for the programs `0.2` can express (`spec/13 §7`).
- **One language.** A profile is not a dialect: the same source means the same thing in both, and the interpreter is the reference for both.

## Decision

### 1. Declaration

A package declares its profile in `una.toml`, in edition `0.3`:

```toml
[package]
name = "ring"
version = "0.1.0"
edition = "0.3"
profile = "systems"     # "app" by default
```

- `profile` is `"app"` or `"systems"`. Any other value is U7002.
- Without the key, the profile is `app`.
- In editions `0.1` and `0.2`, the key is unknown and U7002, as today. Their packages are `app`.
- A single file is `app`.
- `profile` stays a reserved word of the language (ADR 0014). This decision adds no syntax: the profile is a property of the package, not of a module.

### 2. The order: `systems` is stricter than or equal to `app`

The two profiles are ordered: `app ≤ systems`. The relation is the decision's invariant, and every later rule that names a profile MUST keep it:

1. **Acceptance.** A program without gated constructs (rule 5) that `systems` accepts, `app` accepts too. A `systems` package only adds obligations and errors to the same program.
2. **Behavior.** A program accepted by both has the same behavior in both: the same output, failures, aborts, traces, and the same result under the interpreter and the native backend. A profile changes no evaluation rule.
3. **Checks.** No profile removes, weakens or defers a run-time check, a static check or an obligation. Run-time checks are removed only where the verifier proves them unnecessary (#737), and that rule is the same in both profiles: a proof is a proof whatever the profile.
4. **Theorems.** Every theorem of the formal track (F1) and every safety property of the specification holds for both. The profile appears in no premise of any of them.

`app` is today's language, unchanged. Declaring `profile = "app"` changes nothing.

### 3. What `systems` adds: `Int` stays in a machine word

In a `systems` package, an `Int` never promotes to an arbitrary-precision integer, and the verifier proves it:

- Each `+`, `-`, `*`, `/`, `mod` and unary `-` on `Int` outside clauses carries an obligation of kind `overflow` (`spec/18 §3`): its exact result lies in the tagged range, -2^62 to 2^62 - 1. So does an `Int` literal outside that range, which is refuted at once. Clauses and the bodies of `reflect` predicates compute exactly and carry none, as for fixed-width types.
- So does each prelude function whose `Int` result can leave the range:
  - `sum` over a list of `Int` or of a fixed-width type;
  - `to_int()` on `I64` and `U64`, whose values can lie outside 63 bits;
  - `abs`.

  Each carries an `overflow` obligation on its result. Functions whose result is bounded by a size, as `len`, carry none.
- The repairs are those of `spec/18 §3`: a guard, a refinement, a fixed-width type, or an explicit wrapping or saturating method.

The obligations are proofs, and the run-time promotion path stays in the executable. Where #737 elides proven checks, the proven obligations of this rule are what let it drop the promotion path of each operation. So `systems` code gets machine-integer arithmetic through proofs, never through a switch. A program that `systems` rejects for an `overflow` obligation runs unchanged under `app`, where the value promotes.

Why this is the one thing `systems` adds: predictable machine arithmetic is what systems code needs from the profile, and it makes the performance of `Int` code not depend on how large values become at run time. It is a narrowing: `app ≤ systems` holds by construction.

### 4. Defaults that `systems` makes explicit

A default becomes a required declaration, never a different value:

- **The memory budget.** A `systems` package declares `[run] max_memory`, in MiB, at least 1. A package with an `[run] entry` and without it is U7002. The value is the default of `una run` and the native executable, which `--max-memory` may lower, never raise. An `app` package may declare it too, with the same meaning; without it, the default stays 4096 MiB (`spec/11 §7`).
- Nothing else changes: the call depth, the escape budget (`[escape_budget]`, default 0) and the grants of `[run]` keep their rules in both profiles.

### 5. Gates for constructs that only `systems` may use

Some later constructs belong to `systems` code only. A profile **gates** a construct; it never defines one. Each gated construct has its own ADR, which MUST:

- keep every safety property of rule 2.4 or name, precisely, what enters the trusted base;
- report each use in the build report, as `checked` and policies are reported today;
- reject the construct in an `app` package. The diagnostic code is registered by the construct's ADR, since none is used yet.

Two are reserved now:

- **Foreign code (FFI, #33).** A foreign function declaration is gated to `systems`, and to modules listed in `[trusted]`. Its ADR states what a foreign call may assume and what it must guarantee, how its effects enter `uses`, how its labels are declared, and how the TCB report counts it (#65). Until that ADR is accepted, foreign code does not exist in either profile.
- **SIMD (`simd`).** Vector operations over fixed-width elements are gated to `systems`. Their ADR defines each operation by the scalar loop it equals, with no undefined lane, so `systems` code keeps the interpreter's semantics. Until then, `simd` stays reserved.

A gated construct widens what `systems` accepts, so it is the one exception to rule 2.1. It is admitted only through an accepted ADR that keeps rule 2.4, and the exception is bounded: removing the gated constructs from a `systems` program gives a program for which rule 2.1 holds.

### 6. Editions

- The profile exists from edition `0.3`. It is preview material under `spec/13 §7`. For a program that edition `0.2` can express, `systems` only adds obligations and errors, which is the narrowing the preview rule admits.
- An edition and a profile are independent axes. The edition fixes the language's syntax and rules; the profile fixes which of them a package opts into. A later edition may move a `systems` rule into every package, never the reverse.
- The Core export (`spec/13 §8`) records the profile in its header, `"profile": "systems"`, so the re-check of the formal track sees it. Its theorems do not depend on it (rule 2.4).

### 7. The build report

The report's `package` section gains `profile`, and the obligations listing gains nothing new: the `overflow` obligations of rule 3 are listed with their kind, as for fixed-width types. A future gated construct adds its section by its own ADR.

### 8. Complexity

Rule 3 adds one obligation per `Int` operation of a `systems` package: linear in the size of the program, each a quantifier-free query of the same size as a fixed-width `overflow` query. The proof cache and the fallback of ADR 0053 and ADR 0054 apply unchanged. Rule 4 is a manifest check, O(1). No pass becomes quadratic.

## Consequences

- One language with one meaning. A reader never needs to know a package's profile to know what its accepted code does; it only tells what more was proven or declared.
- `systems` packages pay obligations for `Int` arithmetic, which is the price of machine arithmetic without a switch. The repairs are the familiar ones of fixed-width code.
- The performance track keeps one rule: checks fall only to proofs (#737). A `systems` package is where those proofs exist for every `Int` operation, so it is where the promotion path disappears first.
- FFI and SIMD have a place to land, and a contract their ADRs must meet before they exist.
- Per-module profiles, a profile for libraries, and profiles across package dependencies are deferred: the language has no package dependencies yet.

## Tests

- Manifest: `profile` accepted in edition `0.3`; U7002 for another value, and for the key in editions `0.1` and `0.2`; a `systems` package with an entry and without `[run] max_memory`, U7002; `--max-memory` above the declared value is a usage error.
- Rule 3: an `Int` addition that can leave 63 bits is refuted in `systems` with a counterexample and accepted in `app`; a guarded one is proven; a literal of 2^62 is refuted; `sum`, `to_int()` on `U64` and `abs` carry their obligations; clauses carry none.
- Rule 2: the conformance cases and examples, checked as `systems`, keep their outcomes or gain only `overflow` errors and U7002 for a missing budget; the same programs, run under both profiles, give the same traces.
- Scaling: a `systems` module with 10, 100 and 1,000 `Int` operations; the per-operation cost stays within a fixed ratio.

## Alternatives considered

**A. A `systems` profile that turns checks off** (unchecked arithmetic, no bounds checks, no budgets), as other languages' release modes do. Not chosen: it makes safety depend on a manifest key, splits the meaning of every program, and breaks the rule that only proofs remove checks.

**B. Profiles as editions** (`edition = "0.3-systems"`). Not chosen: editions version the language over time; a profile is a choice within one version. Mixing them doubles every edition.

**C. Per-module profiles.** More precise, since one module of an application may be systems code. Deferred: the TCB and the report are per package today, and a module-level gate for FFI would be harder to audit. It can be added later without changing rule 2.

**D. `systems` that traps on promotion at run time** instead of proving its absence. Not chosen: a program would abort under `systems` where it computes a value under `app`, which breaks rule 2.2, and the trap is a run-time check where a proof can do better.

**E. No profiles; gate FFI and SIMD by `[trusted]` alone.** Simpler. Not chosen: `[trusted]` already means "skips label checks inside `via`"; overloading it with foreign code mixes two trusted bases, and the machine-integer guarantee of rule 3 still needs a package-level opt-in.
