# 0135. Query slicing for the verifier

- Status: Proposed
- Date: 2026-11-03
- Deciders: Una project (proposal written on the project's decision of 2026-11-03 to create this record; not accepted)
- Resolves: issue #1484 (finding CV-17 of the performance scan of 2026-11-02, `una-verify`)
- Related:
  - `spec/12` §3 (obligations, the path context Π), §4 (the SMT encoding, and "queries are emitted as text": the query text is normative), §5 (the script and the proof condition), §6 (statuses), §9 (`verification.lock` and the proof cache), §10 (determinism);
  - ADR 0114 rule 8.3 and `spec/29 §8` rules 4 and 5: the cone of `F64` hypotheses, today's only slicing, and its rule that a refutation which is not final is decided by the full query;
  - ADR 0041 (the proof cache and its `cached` count), ADR 0063 L5 (the editor runs tier 1 on save through the cache), ADR 0064 and its amendment of 2026-10-25 (authenticated entries; rule 6: statuses do not depend on the cache), ADR 0124 rule 10 (the key binds the solver build), ADR 0122 rule 48 (the key of edition `0.4`);
  - ADRs 0053, 0054 and 0071 (the model checks and re-solves of a refutation), ADR 0114 rule 8.6 (the pinned conversion);
  - ADR 0124 rule 8 and Alternative C (one child per query; no incremental solver);
  - issue #1479 (eviction of old proof-cache entries, CV-16), and the project's decision of 2026-11-03 that the proof cache stays.

## Context

### What a query holds today

Every obligation is `Π ⊢ goal`, sent as the satisfiability check of `Π ∧ ¬goal` (`spec/12 §3`). Π is the whole path context at the site: every binder's refinement in scope, every `requires`, every branch condition, every callee `ensures` instantiated, every selfified `let` in the refinement language, plus the ground facts of `spec/12 §3` for every term the query mentions, and the `define-fun` of every reflect predicate it applies (`generate.rs`, `Walker::push`). Only `F64` hypotheses are sliced, by the cone of ADR 0114 rule 8.3 (`slice_floats`, `generate.rs`): that rule exists because irrelevant float facts were bit-blasted in preprocessing, outside the step budget, and turned trivial integer goals into minute-long queries.

An integer obligation late in a long function therefore carries every fact the function established before it, whether or not the goal can depend on it. The cost of that is twofold:

1. **Solve time.** The solver receives facts it does not need. Z3 often ignores them cheaply, but not always: unrelated nonlinear products (`i!mul!Int`), case splits over unrelated disjunctions and large path disjunctions all consume the step budget, and an obligation can end `unknown` at the budget (U5002) only because of facts that have nothing to do with its goal.
2. **Cache stability.** The proof cache keys an answer by the exact script (`cache.rs`, `key`). Because every query of a function contains the whole prefix of its path, an edit anywhere in that prefix changes the text, and the key, of every later obligation in the function, including those whose goal it cannot affect. ADR 0063 L5 runs tier 1 on each save through the cache, so in an editor session most of these re-solves are wasted work.

The project decided on 2026-11-03 that the proof cache stays (the scan's section "The proof cache": nothing within the accepted ADRs removes the work it avoids). Slicing does not replace it; it makes each miss cheaper and makes keys change less across edits.

### What slicing is

**Slicing** drops from a query the hypotheses and definitions that are not relevant to its goal. Relevance here is syntactic, by **cone of influence**: a hypothesis is relevant when it shares a free symbol with the goal, or with a hypothesis that is relevant, transitively. The free symbols are the uninterpreted constants (the binders `u!v!…` and `i!v!…`, text literal constants `i!lit!…`) and the uninterpreted or defined function symbols (fields, `len`, `tag`, `i!mul!Int`, reflect predicates). Built-in SMT-LIB symbols (`+`, `<=`, `and`, numerals, the FloatingPoint operations) never connect anything: their meaning is fixed by the theory, not by the query.

A small example. At the index `xs[j]` of

```text
Π = { 0 <= i, i < xs.len, total = acc + w, w > 0, 0 <= j, j < xs.len }
goal: 0 <= j and j < xs.len
```

the goal's symbols are `j`, `xs` and `i!len!…`; the conjuncts `0 <= j`, `j < xs.len` and, through `xs.len`, `0 <= i` and `i < xs.len` are in its cone; `total = acc + w` and `w > 0` are not, and the sliced query drops them. The facts about `total` are what an edit to the accumulation would change, so after slicing that edit no longer changes this obligation's key.

### What makes this a decision and not a refactoring

- **The query text is normative.** `spec/12 §4` fixes the declarations' and assertions' order so identical programs give byte-identical queries, and §9 keys the cache by that text. Slicing changes the text of every query that drops something.
- **An outcome can change.** A query that ends `unknown` at the step budget with its full context may be proven once the irrelevant facts are gone. That is the point of the change, but it is a visible change of status for the same source and the same `verification.lock`, which `spec/12 §10` is about.
- **Soundness has to be argued, not assumed,** in particular for refutations: a model of the sliced query need not satisfy the hypotheses it dropped, so it need not be an execution.

## Decision

### 1. Two queries per obligation: the slice, then today's query

Let **Q** be the query of an obligation exactly as `una-smt-1` builds it today (with the `F64` cone of ADR 0114 rule 8.3 already applied). Under the new encoding `una-smt-2` (rule 6), the verifier also builds **S(Q)**, the **slice** of Q (rule 2), and decides the obligation so:

1. If S(Q) drops nothing from Q, only Q is sent, exactly as today: its script is byte-identical to the `una-smt-1` script.
2. Otherwise S(Q) is sent first, alone, as one query with the script of rule 4.
   - If its answer meets the proof condition of `spec/12 §5` rule 6 (`unsat`, then a clean exit), the obligation is **proven**. No other query is sent.
   - In every other case, `sat`, `unknown` for any reason, an error, a crash or a timeout, the slice's answer is discarded, and **today's whole pipeline runs on Q** (the solve, ADR 0053's model check, ADR 0054's interpreted re-solve, ADR 0071's first-iteration re-solve, ADR 0114 rules 8.3 and 8.6): its final status, reason, counterexample and diagnostic are the obligation's.

The slice is only ever used to **prove**. It never refutes, never yields a counterexample, and never turns `unknown` into anything but the verdict of Q. `unknown` stays an error (charter P8): an `unknown` slice is not a pass, and it is not even final; it hands the obligation to Q, whose `unknown` is U5002 as today.

### 2. The slice

S(Q) is computed from Q's parts, before Q's ground facts and definitions are added:

1. **Conjuncts.** Each hypothesis of Π (each assertion of Q but the negated goal) is split into its top-level conjuncts, as ADR 0114 rule 8.3 already does. Splitting stops at any connective other than a top-level `and`: a disjunction, an implication, a negation or an `ite` is one conjunct.
2. **Roots.** The negated goal is always kept. Its free symbols are the roots. A postcondition's path assertions (the selectors `i!v!path!n = path_n` and their disjunction, `spec/12 §3`) are ordinary conjuncts and are kept when connected, which they always are through `result` when the goal mentions `result`.
3. **Connection.** Two conjuncts are connected when they share a free symbol in the sense of "What slicing is" above. A reflect predicate's name counts as a symbol, and its definition's free symbols are connected to it, so a conjunct that applies a predicate is connected to everything its body mentions.
4. **Kept.** A conjunct is kept when it is connected, transitively, to a root. The others are dropped.
5. **Facts and definitions.** The ground facts of `spec/12 §3`, the distinctness of text literals of §4, and the definitions of reflect predicates are then added for the terms of the kept conjuncts only, by the same functions as for Q.
6. **Invariant.** Every assertion of S(Q) is an assertion of Q, a top-level conjunct of one, a ground fact of a term of Q, or a distinctness of a subset of Q's literals; every definition of S(Q) is a definition of Q, with the same body. So **Q ⊨ S(Q)**: every model of Q is a model of S(Q). The implementation checks the structural form of this invariant in debug builds, and the test plan checks it with the solver (rule 9).

### 3. Order and determinism

The kept conjuncts appear in S(Q) in Q's assertion order (a subsequence), split conjuncts in their left-to-right order inside their hypothesis; ground facts come first, in the order Q's facts would have, then the definitions and declarations, sorted by name, as `spec/12 §4` writes every query. S(Q) is a function of Q's text alone, so identical programs give byte-identical slices, and the slice of an obligation does not change when an edit touches only conjuncts outside its cone.

### 4. The script of a slice

A slice is sent with the preamble of `spec/12 §5` rule 2 (its logic and step budget chosen by its own text, by ADR 0114 rule 8.2: a slice without an `F64` term is in `QF_UFLIA` under the lock's budget, even when Q is not), its declarations and assertions, then `(check-sat)`, `(get-info :reason-unknown)` and `(exit)`. It has no `(get-model)` and no `(get-value ...)`, because its model is never read. A proof of a slice is therefore `unsat`, the reply to `(get-info :reason-unknown)`, and exit status 0, which the proof condition of `spec/12 §5` rule 6 already accepts, with no "model is not available" line. Leaving out the requested terms also keeps the declarations of the shown terms out of the slice, which would otherwise put every parameter of the function into every slice's text and undo rule 3. The weighted bound of ADR 0114 rule 8.3 applies to a slice as to every query sent: a slice over it is not sent, and the obligation goes to Q.

### 5. Soundness and monotonicity

Write V(Q) for the verdict today's pipeline gives Q, and V for the obligation's verdict under rule 1. Assume, as every proof of Una does, that the pinned solver's `unsat` and `sat` answers are correct (`00-charter.md §3`: the solver is in the Trusted zone).

**Lemma 1 (soundness of a sliced proof).** If S(Q) is unsatisfiable, so is Q. *Proof.* By the invariant, every model of Q is a model of S(Q); S(Q) has none, so Q has none. Q is `Π ∧ ¬goal` (with ADR 0114's sound weakenings of Π), so `Π ⊢ goal`. ∎ The same holds for any subset of Q's conjuncts: the soundness of a proof does not depend on how the cone is chosen, only on the invariant. A bug in the cone can cost proofs; only a bug that breaks the invariant can cost soundness, which is why rule 9 tests the invariant itself.

**Lemma 2 (monotonicity).** V never moves away from `proven`, and never moves between `refuted` and `unknown`:
- V(Q) = `proven` ⇒ V = `proven`: either the slice proves it, or the slice's answer is discarded and V = V(Q).
- V(Q) = `refuted` ⇒ V = `refuted`, with the same counterexample: the pipeline's `refuted` rests on a satisfiable query at least as strong as Q (Q itself, or Q with the facts of ADR 0071's first iteration, the interpreted definitions of ADR 0054, the pinned arguments of ADR 0114 rule 8.6, or the full float query of rule 8.3, each of which implies Q's hypotheses), so Q is satisfiable, so S(Q) is too (it is implied by Q) and cannot be proven; its answer is discarded and V = V(Q).
- V(Q) = `unknown` ⇒ V ∈ {`unknown`, `proven`}: `proven` exactly when the solver proves the slice.

So the only change of status slicing can cause is **`unknown` to `proven`**, never the reverse, never `proven` to `refuted` and never `refuted` to `proven`. A slice proven while Q is refuted would need the solver to answer `unsat` on a query that is implied by a satisfiable one, a solver unsoundness that the differential test of rule 9 looks for.

**Lemma 3 (when the slice proves).** Let D be the dropped conjuncts with the ground facts of their own terms. If Q is unsatisfiable and D is satisfiable, then S(Q) is unsatisfiable. *Proof sketch.* S(Q) and D share no free symbol (rule 2.3, with every uninterpreted symbol, constants and functions, counted). Their theories, linear integer arithmetic, Booleans and FloatingPoint, each have one fixed model of their interpreted sorts, and the uninterpreted sorts are interpreted by sets with no symbol in common. A model of S(Q) and a model of D therefore combine into one model of both: take the integer, Boolean and float domains as they are, take for each uninterpreted sort the disjoint union of the two models' domains, and interpret each symbol as its own model does, extended arbitrarily outside its own part; a quantifier-free formula's truth depends only on the values of its own symbols' terms, so neither part changes truth. That model satisfies every assertion of Q: each is a kept conjunct, a dropped one, or a ground fact of a term of one part (a term shared by both parts would share a symbol), and the distinctness of literals holds because literals of different parts are different elements of the disjoint union. ∎ So the slice loses a proof only when Π contains, outside the goal's cone, a contradiction of its own: a dead path, proven today vacuously. That case is not lost either: the slice is `sat`, and Q proves it, as today.

**Refutations need the full query, and get it.** A model of S(Q) may violate D, so it may be no execution, and a counterexample built from it could be a path the program never takes. ADR 0114 rule 8.3 accepts a cone refutation as final in one narrow case (only unused definitions dropped). This record does not extend that shortcut to general slicing: every refutation is decided by Q's pipeline, so counterexamples, the checks of ADRs 0053, 0054 and 0071, and every diagnostic stay exactly as they are under `una-smt-1`. The price is one extra query, the slice, for each obligation that is not proven, and those are the obligations that fail the build anyway. Rule 11 lists the shortcut as an open question.

### 6. The encoding version and migration

1. `una-smt-2` is `una-smt-1` with rules 1 to 5. `verification.lock`'s `encoding_version` (`spec/12 §9`) and `ita.lock`'s solver `encoding` (`spec/30 §7`) accept `"una-smt-1"` and `"una-smt-2"`; any other value stays U7004 (U7009 in `ita.lock`), with a message that lists both.
2. A lock that pins `una-smt-1` keeps today's queries, byte for byte, and today's verdicts: nothing changes for a package until its lock changes, and `spec/12 §10` keeps holding per lock.
3. The locks that `una` writes (a new package's `verification.lock`, a new `ita.lock`) pin `una-smt-2` from the slice that ships it (rule 10, slice 4). The repository's examples and conformance packages move in the same slice.
4. **Cache keys.** The key of editions `0.1` to `0.3` covers the whole lock text, and the key of edition `0.4` covers the lock's `solver` section, which holds `encoding`. Changing the line therefore changes every key of the package, sliced or not: the old keys simply miss, the first run after the change solves every obligation again, and nothing is invalidated by hand or read with a wrong meaning. The key's composition (ADR 0124 rule 10, ADR 0122 rule 48), the HMAC of ADR 0064 and the entry checks of its amendment are unchanged. A slice and Q have different scripts, so their entries never collide.
5. **What the cache stores.** A slice's answer is cached like any other (`proven`, `refuted`, and `unknown` from the step budget; never `wallclock`, `cancelled`, `cpu`, `memout` or `crash`). A cached non-proof of a slice is read as "not proven" and the obligation goes to Q, which the cache may answer too.
6. **Old entries and #1479.** After a package moves to `una-smt-2`, its `una-smt-1` entries are never looked up again. They are valid files that waste disk until the eviction of #1479 removes them; that design (`docs/plan/proof-cache-eviction.md`, not written yet) MUST treat a never-looked-up entry like any other old one, and needs no knowledge of encodings. It also needs no pairing between a slice's entry and its Q's entry: evicting either only costs a solve. Slicing reduces the churn the eviction has to absorb, since an edit renews the keys of fewer obligations, but a failing obligation can now hold two entries (its slice and Q), and the eviction's measurements should count both.

### 7. The normative text that changes

On acceptance, with the slice that implements rule 1 (slice 3):

- `spec/12 §3`: after "Each obligation is `Π ⊢ goal`…", the slice and rule 1, stated for `una-smt-2`.
- `spec/12 §4`: the last paragraph, the order of a slice (rule 3), so the text stays byte-identical per program.
- `spec/12 §5` rule 6: the script of a slice (rule 4).
- `spec/12 §6`: one paragraph: a slice only proves; every other status comes from the obligation's query.
- `spec/12 §9`: the two values of `encoding_version`, and that a slice's answer is cached like any query's.
- `spec/29 §8` rules 4 and 5: the slice is taken of the cone query, and the pipeline of rule 5 runs on that query when the slice does not prove it.
- `spec/30 §7`: the two values of the solver `encoding`.
- `spec/13 §2` and `§6`: `cached` and `decided_by` (rule 8).
- Dated notes, not edits, on ADR 0041 (the meaning of `cached`), ADR 0064 (rule 6 still holds: the cache only skips a solve of an identical script), ADR 0114 rule 8.3 (the slice comes before its pipeline and does not change it), ADR 0122 rule 48 and ADR 0124 rule 10 (the key's parts are unchanged; `encoding` takes a second value). ADR 0063 L5 needs no change: the editor runs the same pipeline and shares the same cache.
- The `explain` texts of U7004, U7009 and U5002 that quote a lock.

### 8. `una report`, the obligation listing and trust accounting

- **Trust.** A proof of a slice is a proof of the obligation (Lemma 1) by the same pinned solver, under the same proof condition: no new component, no entry in the report's `tcb`, no new field in `una.report/2`'s trust sections, and `proven` keeps its meaning, so the report does not need to say that a query was sliced. The slicer is new code inside the verifier, which is already row 2 of `docs/security/tcb.md` (a bug in the encoder can already forge a proof), so it is trusted code of the verifier, as the encoder is; what bounds its risk is that its whole soundness obligation is one property of its output, Q ⊨ S(Q), which rule 9 checks structurally and with the solver. `docs/security/tcb.md` row 2 names the slicer and that invariant with slice 3, and slice 3, a soundness-relevant change of the verifier, gets an adversarial review besides the ordinary one.
- **`cached`.** `obligations.cached` (ADR 0041 rule 2) counts an obligation whose every query that the run sent for it, the slice and, when needed, Q's pipeline, was answered from the cache. `una obligations` marks it `(cached)` on the same rule.
- **`decided_by`.** `una obligations --json` gains, per obligation, the additive key `decided_by`: `"slice"` when the slice proved it, `"query"` otherwise (and always `"query"` under `una-smt-1`). The schema stays `una.obligations/1` (additive keys, ADR 0076 rule 1). It serves the differential test and anyone asking why a status changed after the lock moved; the human output is unchanged.
- **`hypotheses`** in the listing stays the path context Π as today, not the slice's subset, so the listing of an obligation does not depend on which query decided it.

### 9. Test plan

1. **Invariant (unit and property).** For every obligation of the conformance suite, `examples/` and the std corpus, S(Q)'s assertions are a subsequence of Q's conjuncts plus facts of Q's terms (rule 2.6), and its definitions are Q's. And, with the solver, the implication itself: `Q ∧ ¬(⋀ S(Q))` is `unsat` for every one, in a test that runs the pinned solver; `unknown` there fails the test.
2. **Differential (both encodings).** The whole conformance suite and every example package run under `una-smt-1` and under `una-smt-2`, with a cold cache each. Required: identical diagnostics, counterexamples, exit codes and reports, except obligations that move from `unknown` to `proven`, which are listed with their ids and reviewed; any other difference fails. And, per obligation, both S(Q) and Q are solved: a slice `unsat` with Q `sat` is a failure (a solver inconsistency or an unsound slice, Lemma 2); a slice `sat` with Q `unsat` is counted and listed (Lemma 3: a contradiction outside the cone, or a lost connection).
3. **Counter-based evidence.** Wall time is not the measure (it is not deterministic, `spec/12 §10`). Per obligation, recorded by a measurement harness outside the normative script: the conjunct count and term size of Q and S(Q), and Z3's step count (`rlimit-count`) for each; per package, the total steps under both encodings. For the cache: on a scripted editor session (a sequence of one-line edits to each example function), the number of obligation keys that change per edit under each encoding. The measurement report is the input of the go decision of slice 2 (rule 10), and its numbers, not estimates, go into the record before acceptance of slice 3.
4. **Fuzz and mutation.** (a) Generated programs: random functions with long straight-line prefixes, unrelated branches, nonlinear products, reflect predicates, sums with fields, text literals and floats, each obligation checked by item 1's implication test and item 2's per-obligation comparison; the generator reuses the hunt's generators (#513) where they exist. (b) Mutants of the slicer (ADR 0129's mutation engine, or `cargo mutants` on `una-verify` until it covers Rust): a mutant that splits under `or`, `not` or `=>`, rewrites a conjunct, drops the negated goal's negation, reorders text, or adds an assertion must be killed by items 1 or 2; a mutant that only drops more (a weaker cone) is expected to survive the soundness tests and be caught by the counted losses of item 2. (c) Refuted corpus mutations: for each refuted obligation of the corpus, S(Q) must not be `unsat`.
5. **Conformance cases** (positive, negative, boundary): an obligation that is U5002 at the budget under `una-smt-1` and proven under `una-smt-2` (built with irrelevant nonlinear facts before it); a dead path outside the cone, proven under both (slice `sat`, Q `unsat`); a refutation whose counterexample is byte-identical under both; an obligation with an empty Π (the trivial-`ensures` queries of `spec/13 §6`'s coverage are such), whose script is unchanged; one whose slice drops nothing, whose script is byte-identical to `una-smt-1`'s; a float function whose integer goal's slice is in `QF_UFLIA`; a reflect predicate applied in a dropped conjunct and in a kept one; a lock with `encoding_version = "una-smt-3"` (U7004).
6. **Determinism.** The slice of every corpus obligation is byte-identical across two runs and across thread counts; the slicer iterates only ordered maps or index order, never a hash map.
7. **Scaling.** The slicer on a synthetic Π of 1k, 4k, 16k and 64k conjuncts in chains and in stars, with a ratio check that fails on quadratic growth (`CLAUDE.md`: a hot path of the verifier's encoding).

### 10. Slices of work

1. **The slicer, unwired.** A pure function in `una-verify` from Q's parts to S(Q), with rules 2 and 3, the invariant check in debug builds, items 1, 6 and 7 of rule 9. No query changes.
2. **Measurement.** The harness of rule 9 items 2 and 3, run over the corpus; a report with the step counts, the slice sizes, the losses of Lemma 3 and the key churn of the editor scenario. **Go decision:** slice 3 starts only if the report shows a reduction worth the second query on failures; otherwise this record is withdrawn or narrowed, and the report is kept.
3. **`una-smt-2`.** The lock values, the pipeline of rule 1, the script of rule 4, the cache rules of rule 6.5, `cached` and `decided_by`, the spec text of rule 7, the `explain` texts, the conformance cases of rule 9 item 5, the differential test in CI.
4. **Default and migration.** `una` writes `una-smt-2` in new locks; the repository's examples, conformance packages and the guide move; the release notes state that the first check after moving a lock re-solves every obligation.
5. **Release evidence.** The fuzz and mutation runs of rule 9 item 4 join the heavy gates that run on a release candidate (`docs/release-gates.md`), not on every pull request.

### 11. Open questions

1. **Retiring `una-smt-1`.** Keep both encodings until v1.0 and then drop `una-smt-1`, or keep it as long as a stable edition's lock can name it? This record keeps both and decides nothing about the end.
2. **The connection relation.** Rule 2.3 connects through every uninterpreted symbol, which gives Lemma 3. Connecting through constants only, not through shared function symbols such as `i!len!…` or a field, gives smaller slices that may lose proofs (Q then decides, at the price of the extra query). Which one is cheaper overall is for the measurement of slice 2 to say.
3. **The refutation shortcut.** Accepting a slice's refutation as final when every dropped conjunct is a definition whose binder nothing kept uses (ADR 0114 rule 8.3's rule, generalized) would save Q's solve on failing obligations, but it changes counterexamples (they would omit the dropped bindings) and needs ADRs 0053, 0054 and 0071 to be re-argued on a slice. Deferred.
4. **The wall-clock guard.** Each query sent has its own guard, so an obligation whose slice and Q both hang takes two guards, 120 s by default, before U5002. Should the slice get a shorter guard, or a share of one? A shorter guard is not deterministic either, and only the slice's answer would be lost.
5. **A minimum gain.** Should the slice be sent only when it drops at least some fraction of Q (a constant of the encoding, so still deterministic), to avoid a second query that is almost Q? This record sends it whenever it drops one conjunct.
6. **`decided_by` and `hypotheses`.** Is the new key wanted, and should the listing also show the slice's hypotheses, which would explain a proof better but make the listing depend on the encoding?
7. **The `F64` cone.** Could ADR 0114 rule 8.3's cone later be expressed as this slice plus its float abstraction, removing one of the two mechanisms? Not in this record.

## Consequences

- **Easier.** Obligations late in long functions are solved over the facts they can depend on. Some U5002 at the step budget become proofs. An edit in an editor session renews the cache keys only of the obligations whose cone it touches, so ADR 0063 L5's tier 1 on save hits the cache more often. The proof cache stays and becomes more effective.
- **Unchanged.** Every refutation, counterexample, `unknown` reason and diagnostic: they come from today's query and pipeline. Soundness rests on one checkable invariant, Q ⊨ S(Q). A package that keeps `una-smt-1` sees no change at all.
- **Harder.** Two queries for an obligation that is not proven (once per failure, and cached). Two encodings to keep while both are accepted. Every key of a package changes once when its lock moves, so its first check after the move is a cold one. The differential test doubles the verification part of CI for the corpus.
- **Deferred.** The open questions of rule 11; the eviction of #1479, which this record only constrains.

## Alternatives considered

Complexities are in S, the total size of Q's terms (the number of term nodes across its assertions and definitions), V the number of distinct free symbols, and |Π| the number of conjuncts.

1. **No slicing (today).** No extra time or space; one query per obligation. Not chosen: it leaves the U5002 at the budget that come only from irrelevant facts, and the cache churn of ADR 0063 L5's on-save runs.
2. **Syntactic cone of influence, by union-find or breadth-first search (chosen).** One pass over Q's terms collects each conjunct's free symbols; a union-find over the V symbols joins the symbols of each conjunct, and a conjunct is kept when its root is a goal symbol's root: O(S α(V)) time, O(S + V) space. A breadth-first search from the goal's symbols over an index from symbol to conjuncts gives the same set in O(S) time and space; either is linear in practice, and the implementation may take the union-find already in `slice_floats`. With symbol texts as map keys (no interning yet, CV-01), each lookup also costs O(log V) comparisons of the symbol's length; with interned symbols, O(1). The ground facts and definitions of the slice cost what they cost for Q today, on a smaller set. Chosen: deterministic, a function of the text, linear, and sound by construction for proofs (Lemma 1), with the full query keeping every other verdict.
3. **Semantic slicing by unsat cores.** Solve Q once with `:produce-unsat-cores` and named assertions, and reuse the core as the next run's slice. Time: one full solve of Q first, plus the core's extraction (solver-dependent, often a few times the solve), so no gain on the first run and none on any changed query; space O(|core|) per obligation, stored across runs. Not chosen: a core is not unique and not minimal, it depends on the solver's search, so it is not a function of the text, against `spec/12 §4` and §10; storing cores across runs is a new persistent store whose content steers what is sent, so it would need authentication like ADR 0064's and would join the Trusted zone's concerns; and the script changes for every query, `(get-model)` and the proof condition included.
4. **Incremental solving.** One long-lived solver per function or per run, with the common prefix of Π asserted once and each goal under `push`/`pop`. Time: the prefix is parsed and preprocessed once instead of once per obligation, O(S_prefix + Σ S_goal); space: the solver's state for the whole function. Not chosen: ADR 0124 rule 8 and Alternative C reject a long-lived solver (state that `(reset)` does not clear could make one answer depend on another, against `spec/12 §10`), and Alternative B rejects Z3 in the `una` process. It also shares work within one run only, so it does not help the cache.
5. **Iterative widening.** Send the 1-neighborhood of the goal first, then the 2-neighborhood, and so on up to Q, stopping at the first proof. Time: up to k queries per obligation for k rounds, each O(S) to build; space O(S). Not chosen now: each round is another solve for failing obligations, and the rounds would be constants of the encoding to stay deterministic; the measurement of slice 2 can say whether a second, intermediate round would pay.
6. **Replacing the full query on refutations by a satisfiability check of the dropped conjuncts.** Solve D alone and combine its model with the slice's (Lemma 3's construction). Time: one query of D instead of Q; space O(S). Not chosen: the combined model has to be assembled outside the solver and checked against ADRs 0053, 0054 and 0071, which is new trusted logic in the path that produces counterexamples; sending Q costs one query and reuses everything that exists.
7. **Slicing always on, with no new encoding version.** Not chosen: the lock exists to pin how obligations are encoded, `spec/12 §10` is stated per lock, and a status that changes with the same lock would be a change no user asked for. With the version, the change is one line in the lock, and its first effect, a cold cache, is expected.
8. **Gating slicing on the edition instead of the lock.** Not chosen: the encoding is configuration of the verifier, not a language feature; ADR 0114 tied its float queries to edition `0.3` because the floats themselves were new syntax. Every edition's packages can move to `una-smt-2`.
