# The formal track: a plan for mechanizing Una Core

- Status: Proposal, for the owner's decision (N-67)
- Date: 2026-10-08
- Issue: #46, the Lean 4 mechanization of Una Core (`spec/10 §10`, theorems T1 to T7)
- Author: Claude (Una-2), at the coordinator's request

## 1. Why now

The roadmap's formal track was due before v0.2 (`docs/plan/roadmap.md §2`). It has not started, and v0.3.3 is released.

In the meantime the security report has logged 13 findings, S-01 to S-13. Twelve of them, S-01 to S-12, are information-flow leaks: a program that the checker accepted printed, returned or failed differently depending on a secret.
- Each one was found by a person or an agent writing adversarial programs, mostly in matrices built after the fact: 96 cells for H-3, 194 cells for the annotation class.
- Each fix closed one class and opened a question about the next: S-09, S-10 and S-11 are three wrappers around one rule.
- Nine of them reached released versions: S-01, S-05, S-06 and S-08 to S-12 in v0.1.x, and S-07 in v0.3.0.

T6, noninterference, is the claim the language makes to its users (`spec/07 §1`). Today that claim rests on tests. This plan puts a proof under the part of the rules where the leaks were, and ties that proof to the Rust checker that users run.

## 2. Which theorems first

| Order | Theorem | Why at this place |
| --- | --- | --- |
| 1 | **T1, type safety, for the fragment** | The step that every other proof uses: preservation keeps the typing of each run, and progress makes "the run stops" mean a value or a failure. It is the cheapest part, about a week. |
| 2 | **T6, noninterference, for the fragment** | It is where the 12 leaks were. It is the language's main claim. And the part that is new to Una, the two symbolic labels `ℓ_T` and `ℓ*_T` with T-App, has never been checked by anything but tests. |
| 3 | **T2 and T3, effect and failure soundness** | They are almost lemmas of T6's proof, which already tracks events and failures in the semantics. |
| 4 | **T4, capability confinement** | It is the second claim. Its rules (second-class function types, `with`, handlers, ADR 0038 D2) are simpler than the label rules and have had fewer findings; #15's false U1001 was a missed program, not a leak. It needs a larger fragment: capabilities, `with` and handlers. |
| 5 | **T5, refinement soundness** | It depends on the solver. The statement is relative to Z3 at the pinned version, an assumption to state, not prove. Its value is lower than T6's until the encoding (`spec/12`) is modeled too. |
| 6 | **T7, decidability** | For the fragment's checker it comes nearly for free: an executable Lean checker made of total functions decides typing by construction. Obligation decidability is a fact about the logic RL (`spec/12 §2`), which is a citation. |

I agree with the coordinator's order, **T6, then T4**, with T1 for the same fragment as the first step of T6 rather than a separate goal.

## 3. The first fragment, F1

F1 is the smallest part of Core (`spec/09`) in which every one of S-01 to S-12 can be written, but S-04 (see §4). It is defined on Core IR, not on the surface language, for the reason given in §5.

**In F1:**
- **Values:** `Int`, `Bool`, `Text`, `Unit`, records with declared field labels, `Option`, `List`, and function values (named functions and lambdas).
- **Labels:** levels and categories as a finite lattice, every position of a type labeled (`spec/07 §7` rule 1), and the symbolic labels `ℓ_T` and `ℓ*_T` of one level of generic functions (rule 8).
- **Typing:**
  - explicit flows and the program counter (rules 2 and 3);
  - T-If and T-Case;
  - T-App for function values;
  - generic instantiation with `ℓ*(σ(T))`;
  - flows at annotations, fields, arguments and results, with covariance through `List` and `Option` and contravariance through function parameters.
- **Effects:** one capability with one operation, whose argument is a sink with an `accepts` label (rule 4). A failure, with `fail`, `attempt` and `if error`, and U4002's rule.
- **The prelude** as axioms: `map`, `filter`, `first_where` and `fold`, and one partial primitive, a parser whose outcome depends on its input. Each axiom is a typing signature together with a semantic function. Its soundness, that the result and the failure carry what decides them, is a proof obligation per primitive, discharged against that semantic function.
- **Host failures:** one host operation whose failure is decided by an argument, as `Http.get`'s by its URL (rule 4, N-39).
- **Semantics:** big-step with traces of events and an explicit outcome, a value or a failure. Termination-insensitive: the theorem speaks of two runs that both end.

**Left out of F1, with the reason:**
- refinements and the verifier: they are T5's;
- `with`, handlers and capabilities as parameters: they are T4's, in F2;
- the memory model (`edit`, loops): the next step, F1+, for S-04;
- `together`, maps, fixed-width integers, text, JSON and HTTP: no finding lives only in them;
- edition 0.1's defense-in-depth rules: the model follows edition 0.2, and since #354 edition 0.1 tells `ℓ*_T` from `ℓ_T` in its flows too. Edition 0.1's own rules can be a later variant, if the owner wants release/v0.1 covered.

## 4. Measured against S-01 to S-12

Two questions per finding:
- **Proof:** would proving T6 for F1 have forced the rule that the fix introduced? That is, is the leak a gap in the rules as written?
- **Re-check:** would the Lean checker of §5, run on the program's Core IR, have rejected the leaking program? That is, is the leak a gap between the rules and the Rust checker?

| Finding | Class | Proof | Re-check | Notes |
| --- | --- | --- | --- | --- |
| S-01 | A decided `T` through a function value (edition 0.1) | Yes | Yes | The case T-App with `ℓ*_T` exists for. |
| S-02 | `parse_json` gives a public outcome | Yes | Yes | The primitive's soundness obligation fails for a signature whose failure is `public`. |
| S-03 | A failure of `Http` decided by an `internal` URL | Yes | Yes | The host operation in F1. |
| S-04 | An un-annotated `let edit` labeled by the order of unification | No | Yes, in F1+ | An inference bug, not a rule gap. The rules (ADR 0075) are sound; the Rust checker solved them wrongly. Only a re-check sees it, and only with `edit` in the fragment. |
| S-05 | A failure of `Files.write` decided by an `internal` path (edition 0.1) | Yes | Yes | Same class as S-03. |
| S-06 | Callbacks of the prelude (edition 0.1) | Yes | Yes | The prelude axioms of F1. The leak was that T-App applied only to calls written in the body. |
| S-07 | S-06 on main | Yes | Yes | |
| S-08 | User generic higher-order functions (edition 0.1) | Yes | Yes | The callee's effects under the decided `ℓ*_T`. |
| S-09 | Annotated lambdas given to the prelude (edition 0.1) | Partly | Yes | A precise rule for when a lambda accepts `ℓ*_T` comes out of the proof; the reading "after unification" was the Rust checker's. |
| S-10 | Annotated lambdas inside a wrapper | No | Yes | An implementation detail (lookup by the argument's span). Core has no wrappers, so a checker on Core cannot miss it. |
| S-11 | Mixed annotated and inferred parameters | Partly | Yes | As S-09. |
| S-12 | An annotation drops the decision (edition 0.1) | Yes | Yes | The flow of `ℓ*_T` into an annotation `T`. |

**Result.**
- A proof of T6 for F1 would have forced the rule behind 8 of the 12 findings outright, and 2 more in part.
- The Lean checker on Core would have rejected **11 of the 12** leaking programs, and **all 12** with F1+.

The difference between the two columns is the reason for §5. Four findings (S-04, S-09, S-10, S-11) were the Rust checker failing to apply sound rules, and no proof about a model catches that alone.

S-13, build paths in executables, is not an information-flow finding and is outside the formal track.

## 5. Keeping the model tied to the Rust checker

A proof about a Lean model says nothing about `una` unless something connects the two. Three options:

| Option | What runs | What it catches | Cost |
| --- | --- | --- | --- |
| A. Differential corpus | The Lean checker and `una check` on the same corpus, comparing accept and reject | Disagreements on the corpus only | Low. The corpus decides what is found, as today. |
| **B. Re-check of Core** | A Lean checker for the label rules over Core IR, proven sound (T6), and run on the Core of **every** program that `una check` accepts, in the gates | Any program whose Core the rules reject: a Rust checker bug becomes a disagreement on that program, whatever the corpus is | Medium. It needs a stable export of Core. |
| C. Extraction | The Rust checker replaced by code generated from Lean | Everything, by construction | High. It rewrites the TCB and leaves diagnostics, repairs and performance to redo. |

**Recommendation: B, with A as its first deliverable.**
- **The checker** is written in Lean as total functions (`lake exe una-core-check`). Its soundness theorem is T6 for F1: a Core program it accepts is noninterferent.
- **What it re-checks.** It re-derives every label of a Core program from the declared types of its parameters, its annotations and the `accepts` of its operations. It does not take the Rust checker's inferred labels on trust: where a binder's label in the Core says less than the rules derive, it rejects.
- **Where it runs:**
  - in a release gate, `formal`, on every conformance case, example package, review program and laundering matrix;
  - in `local-ci.sh` whenever `crates/una-check`, `crates/una-core` or `formal/` changes.
- **Disagreement policy:**
  - Lean rejects what `una check` accepts: a soundness finding, handled as any S-finding.
  - Lean accepts what `una check` rejects: precision. It is logged, and expected where the Rust checker is deliberately conservative (`spec/07 §7` rule 8 rejects some sound programs).
- **The export.** `una check --dump-core` prints Core today, as debug output (`spec/09 §4`), with the symbolic labels `T` and `T*` that the checker records. B needs a versioned JSON form of it: `una.core/1`, by an ADR, since it is a schema of the Agent ABI (ADR 0076).
- **Trusted, and stated as such:**
  - elaboration from the surface to Core (`elab`);
  - the Lean kernel;
  - the export.

  The identity runs of #397 and #407 already guard elaboration's output, and the corpus does.

**A as the first deliverable.** Before T6 is proven, the Lean checker can already be compared with `una check` on the corpus. Replaying the regression programs of S-01 to S-12 through it shows the "re-check" column of §4 on real programs, the evidence for the owner before the proof effort grows.

## 6. Toolchain, pinning and CI

- **Place:** a directory `formal/` in the `una` repository, so that a change of a rule in `spec/10` and its proof move in one pull request, as a spec change moves with its implementation (CLAUDE.md).
- **Toolchain:** Lean 4 at one release, pinned in `formal/lean-toolchain`, installed by `elan`. Lake, with `lake-manifest.json` pinning every dependency by commit. Start without Mathlib: the fragment needs finite lattices, lists and maps, which Lean's core library and Batteries have. Mathlib's build time would weigh on every gate run. If a later proof needs it, it is pinned by commit like the rest.
- **The Linux image** of `scripts/local-ci/Dockerfile` gains the pinned `elan` and toolchain by digest, as Rust and Z3 are pinned (`verification.lock`), and a named volume for Lake's build cache.
- **Gate `formal`** (`scripts/release-gates.sh`):
  - `lake build` of every proof;
  - a check that no theorem depends on `sorry`: `#print axioms` for the stated theorems, allowing only `propext`, `Classical.choice` and `Quot.sound`;
  - the differential run of §5.
- **Times, to be measured in the spike:** a cold `lake build` of a Batteries-only development of this size is a few minutes; a warm one is seconds.

## 7. Milestones and effort

Estimates are in agent-days of one session with the machine available for light runs. Proof engineering in Lean by agents has no track record in this project, so the ranges are wide, and M1 is also a go/no-go spike.

| Milestone | Content | Exit criterion | Agent-days |
| --- | --- | --- | --- |
| M0 | `formal/` with pinned toolchain, Lake, the image and the gate skeleton; an ADR for `una.core/1` and its export | `lake build` passes in the gate on macOS and Linux | 2 to 3 |
| M1 | F1's syntax, labels, typing rules and big-step semantics; **T1** for F1. **Spike review:** effort so far against this table | T1 proven without `sorry` | 5 to 8 |
| M2 | The executable Lean checker for F1 and the differential run (option A). Replay of S-01 to S-12 | Each leaking program of S-01 to S-12 (11 in F1) rejected by the Lean checker; disagreements on the corpus triaged | 4 to 6 |
| M3 | **T6** for F1 without generics: pc, explicit flows, sinks, failures, the prelude axioms | T6 proven for the generic-free part | 8 to 12 |
| M4 | **T6** with generics: `ℓ_T`, `ℓ*_T`, T-App, instantiation; T2 and T3 as corollaries | T6 for F1 proven; the checker of M2 proven sound (option B) | 8 to 15 |
| M5 | F1+: `edit` and loops (ADR 0071, 0075), for S-04 | The re-check covers the memory model's label rules | 5 to 8 |
| M6 | F2 and **T4**: capabilities as parameters, `with`, handlers, second-class function types | T4 for F2 | 8 to 12 |
| Later | T5 relative to the solver; T6 with `together` (ADR 0065); edition 0.1's own rules | By the owner's priority | Not estimated |

**Total to T6 with the re-check in the gate (M0 to M4): 27 to 44 agent-days.**
**With F1+ and T4 (to M6): 40 to 64.**

The value comes early:
- M2 alone, in about two weeks, adds an independent checker on the whole corpus;
- M4 makes it a proven one.

## 8. Risks

- **The symbolic labels may not have a clean semantic reading.** `ℓ*_T` as "every label of the type argument" must be given a meaning in a two-run relation. If the proof shows the rules need a change, that is the point of the exercise, but it may cost more than M4's range. The spike at M1 states the relation before M3 starts.
- **Elaboration is trusted.** A bug in `elab` that changes a program's structure, not its labels, is not caught. The identity runs and the corpus remain the guard there, as #397 showed.
- **Rule churn.** Each change of `spec/07` or `spec/10` must update the model. Section 6 puts both in one pull request, and the gate fails if the proofs break. That is a cost on every rule change, intended.
- **Machine time.** The gate adds a build and a differential run. Both are light next to the native gate, but they join the queue that one machine serves.

## 9. Decisions for the owner

1. Start the track now, with M0 to M2 as the first commitment, about 11 to 17 agent-days, and decide on M3 and M4 at the spike review.
2. Option B, the proven re-check of Core in the gates, as the tie between Lean and Rust.
3. `formal/` in the `una` repository, with Batteries only.
4. A new export, `una.core/1`, as a schema of the Agent ABI, by an ADR at M0.
5. Edition 0.2's rules as the model; edition 0.1's own rules later, only if release/v0.1 needs them covered.
