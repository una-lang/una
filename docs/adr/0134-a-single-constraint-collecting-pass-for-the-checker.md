# 0134. A single constraint-collecting pass for the checker

- Status: Proposed
- Date: 2026-11-02
- Deciders:
- Resolves: issue #1482 (the design; each slice of the migration is a card of its own)
- Related:
  - ADR 0007 (inference of `uses` is the least fixpoint), ADR 0067 (result summaries of generic functions, L2: the least fixpoint per strongly connected component), ADR 0075 (label variables of editable bindings, rule 5: the re-solving rounds and their bound), ADR 0077 (shared types; stage 3a: each solved variable resolved once), ADR 0084 (`una fix` rounds), ADR 0085 (the file-state label; rule 6: the probe of pass 2), ADR 0093 and ADR 0094 (authority at boundaries and the `gives` labels inferred with `uses`), ADR 0116 (IF5, IF6, P9: `τ`, `τ_conc`, `L_res`, and its section E on the cost of the analysis), ADR 0117 (IF2, IF3, and the amendment on slice S2, point 3: the probe of `within`);
  - the performance scan of the checker of 2026-11-02, items CT-18 (this record), CT-05 and CT-06 (work shared by the rounds of today's design) and CT-01 and CT-02 (worklists for the other fixpoints).
- Amends, when accepted: nothing in the language. ADR 0075 rule 5, ADR 0085 rule 6 and ADR 0117's amendment on S2, point 3, describe how the checker computes their labels; each gets a dated amendment pointing here when the slice that changes its computation lands. Their rules on what the labels are do not change.

## Context

### What the checker does today

`check_types_and` (`crates/una-check/src/types/mod.rs`) checks the bodies of a package's modules several times. Every check below is a full traversal of every body in the package unless it says otherwise. Each one exists because some label or header depends on a whole-program fact that is known only after a check:

| # | Traversal | Where | Why it exists | How many |
|---|---|---|---|---|
| 1 | **Pass 1** (`Pass::Collect`), diagnostics dropped | `mod.rs`, the first loop over `check::module` | collects the call facts that `uses` inference needs (ADR 0007, `authority::infer`), the call edges, the `within` sites, and, per declaration, solves the label variables of editable bindings (ADR 0075 rule 2) | 1 |
| 2 | **Summary rounds**, diagnostics dropped (edition 0.2 and later) | `summary_fixpoint`, `check::declarations_at` | the result summaries of generic functions reach their least fixpoint over each strongly connected component, callees first, with a worklist (ADR 0067 L2); each worklist step checks the function once, and once more when that changed an editable binding's solution (ADR 0075) | every generic function at least once, at most twice per worklist step; once more each time an edit-label round changes a solution |
| 3 | **Edit-label rounds**, each a full pass 1 with diagnostics dropped (edition 0.2 and later) | the `while env.edit_labels_seen` loop | ADR 0075 rule 5: once the summaries are final, the label variables must count every callee's final summary; a changed solution sends the summaries back to their fixpoint | at least 1 whenever the package has an un-annotated `let edit` with an open position; bounded by `variables × (atoms + 1) + 2`, past which U8001 |
| 4 | **Probe rounds**, each a full pass 2 with diagnostics dropped | the `if withins || env.files_written` loop | ADR 0117 IF2 (amendment on S2, point 3): `τ(e)` of each `within` needs the whole call graph; ADR 0085 rule 6 (S-14): the file-state label must be known before a read checked earlier than a write; each can raise the other through the program counter | 1 for a package that writes files and has no `within`; at least 2 for a package with a `within` (the first round sets the labels, the second confirms them stable); bounded by `(sites + 1) × atoms + 2`, past which U8001 |
| 5 | **Pass 2** (`Pass::Check`), reports | the loop after the probe | checks with the inferred headers, the solved label variables, the summaries, the timing labels and the file-state label | 1 |
| 6 | **Standard modules**, diagnostics dropped | the last `check::module` loop | the records their contracts are elaborated from | 1 over the standard modules (not the package; out of scope here, see CT-19) |

Counting from the code at `main` `01737b2c2` (not measured): a package of edition 0.4 with a `within` checks every body at least four times (pass 1, two probe rounds, pass 2), and at least five times when it also has an un-annotated editable binding, plus a check of each generic function per summary step. A package with neither checks every body twice. Each traversal also builds full diagnostics with locations, messages and repairs that are then dropped (CT-05), and each probe round resolves and records the type of every expression into tables that are dropped (CT-06). `una fix` multiplies all of this by its rounds, up to 17 checks (ADR 0084 rule 8).

A grep of `tests/conformance` (2,645 `.una` files, an approximation by file, not by package) finds 67 files with `within`, 375 with a capability `write` call, and 180 with an un-annotated `let edit`, 34 of them initialized with an empty collection. The packages people write for servers (ADR 0117) have `within` and file writes as a matter of course, so the multi-traversal case is the common one for that use.

### Why it is shaped this way

Every repeated traversal computes the least fixpoint of a monotone function over labels. Labels form a finite lattice: four levels above `public`, the package's categories, and the symbolic labels its solutions name (ADR 0075 rule 5 counts these as atoms). Each loop iterates "check everything under the current state, collect what the check derived, join it into the state" until nothing rises, which is Kleene iteration with a whole-package traversal as the step. It is correct and simple. Its cost is that one rise anywhere re-traverses every body, and that the confirming round, which changes nothing, is a full traversal too.

The edit-label solver (`edit_labels.rs`) already has the shape this record proposes: pass 1 records the flows into each label variable as bounds, and solving is linear over the strongly connected components of the variables. What it lacks is the cross-declaration part, which the rounds supply.

## Options

Sizes: `B` the total size of the package's bodies (nodes); `D` the number of declarations; `V` the label variables of all kinds (editable-binding variables, summary positions, `within` timing labels, the file-state label, `gives` labels of inferred headers); `C` the label constraints a traversal emits, `C = O(B)`; `h` the height of the label lattice, `h = 4 + k + s` for `k` categories and `s` symbols; `j` the cost of one join, `O(k + s)` with sorted symbol lists; `r` the number of rounds today's loops take (`r_e` edit-label rounds, `r_p` probe rounds).

### A. Keep the rounds, and share their work

Keep every loop. Make each round cheaper: a discarding report in scratch passes that keeps only `(code, span)` (CT-05); a third pass value that skips records in probe rounds (CT-06); and a dirty set, so that a round re-checks only the declarations that read a variable that rose in the previous round, with the confirming round reduced to the declarations whose inputs changed.

- **Time.** Today `O(B · (2 + r_e + r_p) · j)` plus the summary checks. With the dirty set, `O(B · j + Σ_rises size(readers))`. Worst case, every variable rises `h` times and is read by every declaration, `O(B · V · h · j)` as today; average case, a rise is read by a few declarations, close to `O(B · j)` plus a constant number of re-checked declarations.
- **Space.** `O(B)` transient per traversal, as today, plus `O(V · j)` state and `O(D)` for the dirty set.
- **For.** No change of design; each piece ships alone; CT-05 and CT-06 are low risk.
- **Against.** The dirty set needs an exact reads-of-variable relation, which is the dependency graph of option B in another form, so it carries most of B's risk with less of its gain. Each round is still a traversal, so the constant factor of at least 2 for packages with `within` stays.

### B. One constraint-collecting traversal, then solving, then reporting (chosen)

One traversal of each body emits, besides what pass 2 reports today, a constraint for every label that depends on a whole-program fact, with variables in place of the unknown labels. Then one solver computes the least solution of all of them. Then a report phase checks each deferred label check against the solution and reports it.

- **Constraints.** All have the form `x ⊒ f(y₁, ..., yₙ)` with `f` monotone: a join of constants and variables, a substitution of a summary's symbols at a call (ADR 0067), or a meet with a constant (the caps of ADR 0085 rule 1 and the effect bounds of ADR 0117 IF2). Deferred checks have the form `f(y₁, ..., yₙ) ⊑ c` (a sink, `spec/07 §7` rule 4) or `f(...) ⊑ g(...)` (a place of IF2), each with the span, code and site sequence number it reports at.
- **Solving.** A worklist over the strongly connected components of the constraint graph, in topological order, re-evaluating a constraint only when one of its inputs rose. Each variable rises at most `h` times.
- **Time.** Traversal `O(B · j)`. Solving `O((V + C) · h · j)` in the worst case. In the average case a variable rises once or twice, so `O((V + C) · j)`, linear. Report `O(S · j)` for `S` deferred checks. In all, `O(B · j)` on average, one traversal with no confirming round; `O(B · h · j)` in the worst case, below today's `O(B · r · j)` with `r` up to `V · h`.
- **Space.** `O(C + V · j) = O(B · j)` for the constraint graph, held until the report phase. Today's records already hold `O(B)`, so the peak grows by a constant factor, not an order.
- **For.** The best average-case time of the three; the fixpoint becomes explicit, checkable data instead of the side effect of a loop.
- **Against.** A constraint that the traversal fails to emit is a label too low, which is a leak (see Soundness). The checker's label code paths change in many places.

### C. Incremental per-function summaries

Check each function once into a summary of its labels as functions of its inputs: its entry program counter, the summaries of its callees, `conc(f)` (ADR 0116 IF6), the file-state label. Compose summaries over the call graph's components, re-checking a component only when the summary of something it depends on changes.

- **Time.** `O(B · j)` for the checks, plus composition. A summary that is a join of constants and input variables is a pair `(constant, set of inputs)`, so composition along an edge is `O(inputs · j)`. A meet with a constant or a substitution of symbols breaks that form, and a summary as a general monotone function can grow with the number of paths. Worst case superlinear in `B`. Average case `O(B · j)`, with re-checks after an edit limited to the edited component and what depends on it.
- **Space.** `O(F · p · j)` for `F` functions with `p` inputs each, which grows with `τ_conc` and the file-state label: every function has both as inputs.
- **For.** The best fit for an editor (`una lsp`), where one edit should not re-check the package.
- **Against.** The labels flow both ways: summaries flow from callees to callers (ADR 0067), while the entry program counter of ADR 0085 rule 1, `conc(f)` of ADR 0116 IF6 and the timing floor of ADR 0117 IF2 flow from callers to callees. A summary must therefore be parametric in what its callers give it, which is option B's constraint graph cut per function, with a representation that is larger in the worst case. Its batch time is no better than B's.

### D. Keep everything as it is

- **Time and space.** Today's: `O(B · (2 + r_e + r_p) · j)` time, `O(B)` space.
- **Against.** The constant factor of 4 or more is paid on every check of a server package, in every `una fix` round and in every editor check.

### Decision on the options

Option B, reached through option A's low-risk pieces first (slices 1 and 2 below), which ship alone and reduce the cost of today's rounds while the differential harness is built. Option B has the best average-case time, `O(B · j)` with one traversal and no confirming round, and a worst case strictly below today's. Option C's average case equals B's for a whole package, and it is better only for incremental re-checks after an edit; it is left as an open question for the editor, to be built on B's constraint graph, not instead of it.

## Decision

1. **One traversal per body.** For the modules of a package, the checker traverses each declaration's body once. That traversal does everything pass 1 and pass 2 do today. Where pass 2 would need a label that depends on a whole-program fact, the traversal uses a label variable, records the constraint that bounds it, and defers the check that reads it.

2. **The label variables.** Exactly these labels are variables during the traversal, each with the rule that defines it unchanged:
   - the label variables of un-annotated editable bindings (ADR 0075 rules 1 and 2);
   - the positions of the result summaries of generic functions (ADR 0067), in edition 0.2 and later;
   - `τ(e)` of each `within` and each `first of` (ADR 0117 IF2, IF3, with ADR 0116 IF5, IF6 and `L_res` of P9);
   - the file-state label (ADR 0085 rule 1);
   - the `gives` labels of headers whose `uses` are inferred (ADR 0094), and the program counters and failure labels that any of these raise.

   Every other label is computed during the traversal as today.

3. **Solving.** After the traversal of the package, and after `uses` inference, which keeps its three separate phases (headers, then bindings, then handlers) and its own fixpoint, one solver computes the least solution of the constraints. It processes the strongly connected components of the constraint graph in topological order with a worklist. A constraint is re-evaluated only when an input rose.

4. **The bound stays.** A variable that rises more than `h` times means a non-monotone constraint. The solver stops with U8001 at the variable's span ("did not reach a fixpoint", with this record and the defining ADR named), never silently with a label that could be too low. This replaces the round bounds of ADR 0075 rule 5 and of ADR 0117's amendment on S2, point 3, and keeps their purpose.

5. **The solution is verified.** After solving, every constraint is evaluated once more against the solution, `O(C · j)`. A constraint the solution does not satisfy is U8001. The confirmations that run after pass 2 today (`authority::within_timing` and the file-state check of ADR 0085 rule 6) stay. So a solver bug is an internal error, never a leak.

6. **Reporting.** Each deferred check is evaluated against the solution and, when it fails, reported with the span, code and message pass 2 gives it today. Its message names the solved label, as pass 2's does (ADR 0075, Consequences). Repairs are built as follows:
   - a declaration with at least one failed deferred check is checked once more, alone, with the solution fixed, in the reporting mode of today's pass 2. Its diagnostics and repairs are then those of pass 2 exactly, built by the code that builds them today;
   - a declaration with none is not traversed again.

   This replay is linear in the size of the declarations in error. A clean package pays nothing for it.

7. **Records.** The types the traversal records for elaboration carry label variables where the labels are variables. They are resolved against the solution once, when the records are materialized, as unification variables are resolved once today (ADR 0077 stage 3a). So the Core IR, the obligations and the editor index are built from the same labels pass 2 records today.

8. **Shapes of open positions.** ADR 0075 rule 1 gives each label position inside the type that an open position comes to stand for a variable of its own. That type is known only when the declaration's unification ends. A declaration whose open position gets its shape that way is traversed once more, alone, with the shape known: at most once per such declaration (ADR 0075 rule 5 records a shape once), linear in its size. This is the one exception to rule 1, and it is local to the declaration.

9. **The standard modules** are out of scope: their silent check stays as it is (CT-19 is separate).

## Soundness and information flow

**The main risk is a label leak:** a label the new checker computes lower than today's, which accepts a program whose output depends on data above what its sinks accept.

**What makes the single pass equivalent to today's fixpoint.** Let `σ` be the state the rounds carry: the editable-binding solutions and shapes, the summaries, the `within` timing labels and the file-state label. Let `Φ(σ)` be what a full check of the package under `σ` derives for them. Today's loops compute `σ* = lfp(Φ)` by iteration from the least state, then run pass 2 under `σ*`. `Φ` is monotone (labels only rise, ADR 0075 rule 5, ADR 0085 rule 6, ADR 0117 S2 point 3), and the lattice has finite height, so `σ*` exists and the iteration reaches it. The single pass rests on one invariant:

- **I1, label independence of everything else.** The constraints a body emits, and every judgment of the check that is not a label comparison, do not depend on the values of the variables of Decision rule 2. That covers types and shapes, name resolution, authority, the control structure the traversal follows, and which non-label diagnostics are reported.

Under I1, the constraints emitted with variables, read as a function of `σ`, are `Φ` itself. Then:

- their least solution is `lfp(Φ) = σ*`;
- each deferred check evaluated under `σ*` is the check pass 2 performs under `σ*`;
- so the diagnostics are the same, and every sink is checked against the labels its values really have.

**Where I1 is known not to hold.**
- **The shapes of open positions** (Decision rule 8) need a local re-traversal.
- **Reads of the report during the check.** Some decisions consult diagnostics already reported (`Report::has` at `effects.rs` and `expr.rs`, `count()` and the poisoning codes in `check.rs`, from the scan's CT-05). Any read whose answer can depend on a label diagnostic breaks I1: a deferred check has no diagnostic yet when the read runs. Slice 0 audits every such read. Each one is made label-independent, or its declaration is replayed (Decision rule 6) before the read's result is used. Either way, no read gets a different answer than pass 2 gives it today.
- **Uses inference** is not label-dependent, except for the `gives` labels, which rule 2 makes variables. Its order-dependent witnesses (the first call in `first`, the conflicting pair of `merge`, scan CT-01) are untouched by this record.

Slice 0 enumerates every branch of the checker on a label value outside the deferred checks. A grep at `01737b2c2` finds about 37 lines that branch on `Pass::Check` and 16 that name `Pass::Collect`, all of which the audit covers. The list is part of the slice's review evidence.

**Termination.** The lattice has height `h` per variable. Every constraint is monotone. The worklist re-evaluates a constraint only when an input rose, and each variable rises at most `h` times. So solving ends after at most `V · h` rises, and the bound of Decision rule 4 turns any violation of monotonicity into U8001. The traversal itself is one pass per body, and the local re-traversals of rules 6 and 8 are at most one per declaration each.

**What catches an omitted constraint.** The verification of Decision rule 5 catches a wrong solution. It cannot catch a constraint the traversal never emitted, since the solution then satisfies every constraint it has. Only the differential test below catches that. So the old path is kept as the oracle until the last slice has been differential-clean on the whole corpus. This is a soundness change: every slice needs an independent review and an adversarial review (project rule).

**What stays fail-closed.** Every branch that labels with `⊤` today stays: function values, `opaque`, `unseen`, handlers of unknown installation, and the timing sources of concurrency. Summaries only over-approximate. No label is lowered for speed, and no check is elided.

## Differential-test plan

1. **The oracle.** Until the last slice, the checker keeps today's rounds behind an internal switch, a function argument of `check_types_and` that only tests reach, with no command-line flag, no environment variable and no setting. The new path is the default in `una check` only when the slice that switches it lands.
2. **What is compared.** For each package, both paths are run and these are compared:
   - the set of diagnostics, each as `(code, file, start, end, message, related spans, repairs with their edits)`;
   - the list in output order, byte for byte, as `una check --json` prints it;
   - the solved labels of every variable of Decision rule 2: they must be **equal**, not merely ordered. A label higher in the new path is an over-rejection, and a lower one is a leak. Both fail;
   - the Core IR printed by `una core`, the obligations, and `una core --with-rejected` (ADR 0091).
3. **The corpus.**
   - every conformance case in `tests/conformance`, in each edition it runs in;
   - every package in `examples/`, and the native equivalence programs;
   - the hunt corpus: the fixed run of the security hunt, seed 767, 6,288 programs (`docs/release-gates.md`, "The hunt gate");
   - the una-bench task packages assembled as `harness.bench.validate` assembles them (ADR 0085's corpus).

   The test runs per pull request on the conformance cases and examples. On the hunt corpus and una-bench it runs on the slice's release candidate, as the heavy gates do.
4. **Generated stress.** Chains and cycles of 250 to 4,000 functions that carry a label through editable bindings, generic summaries, `within` and file writes, in both declaration orders. These come with scaling tests that assert the number of traversals per body (a counter, as `first_kinds` counts facts read). They also assert that solving work is at most `c · (V + C)` on chains, and at most `c · (V + C) · h` on the worst case.
5. **Tie-break rule.** Output order is by file, start and code (`spec/13 §1`, `una_diag::in_output_order`), and the sort is stable, so equal keys keep their reporting order. Pass 2 reports in traversal order. The new path gives each diagnostic the sequence number of the site where it arises in the traversal's pre-order. Diagnostics found during the traversal and deferred checks are merged by that number before the stable sort. So equal keys keep pass 2's order. Diagnostics reported after the traversal (U8001 of the solver, the checks after pass 2 such as `crossing::releases` and `grants`) come after every traversal diagnostic, as today. Among themselves they keep today's order of those functions. No difference in order is accepted. If a slice cannot keep it, that slice stops and this record is amended with the rule first.
6. **Passing.** A slice merges only with zero differences on the per-pull-request corpus and with the scaling tests. Before the old path is removed, there must be zero differences on the whole corpus.

## Diagnostics order and repairs

- **The set of diagnostics does not change, and neither does their order.** Messages name the same solved labels. Spans, related spans and codes are the same.
- **Repairs are built by today's code,** in the replay of Decision rule 6, with the same inputs: frames, names in scope and the solution. So `binding_permitting`, the edit-distance suggestions and the label annotations proposed for editable bindings give the same edits, in the same rank order.
- **`una fix`** (ADR 0084) keeps its rules: the first repair of each diagnostic, the candidate order per file, at most 16 rounds, and the stop on a state already seen. Each round is one check, so a round costs `O(B · j)` on average instead of `O(B · r · j)`. The rounds, their edits and their stops are identical, since every round's diagnostics are.
- **The editor (`una lsp`)** gets the same diagnostics, hovers and index. Its incremental re-checking is option C, left open.

## Migration in slices

Each slice is independently shippable: it builds, passes the full test suite and the differential test of its scope, and leaves `una check` output unchanged.

0. **Instrumentation and audit.** Counters of body traversals per declaration and of solver work, behind test-only accessors. The internal switch of the differential plan. The audit of I1 (every label-dependent branch and report read), committed as a list. No behavior change.
1. **Discarding scratch reports** (CT-05) and **probe rounds without records** (CT-06). These make today's rounds cheaper and change no output. Each is low risk, with its own card.
2. **Worklists for the other fixpoints** (CT-01, CT-02), as their own cards. These are not required by this record. Their attribution changes are governed by their own decisions, not here.
3. **The file-state label and the timing labels as variables.** Pass 2 emits their constraints, solves them, and reports their deferred checks. The probe rounds go away. Pass 1, the edit-label rounds and the summary rounds stay. This slice amends ADR 0085 rule 6 and ADR 0117 S2 point 3 on how the labels are computed.
4. **Editable-binding variables and summaries as variables across declarations.** Pass 1's per-declaration solve becomes part of the package's constraint graph. The edit-label rounds and the summary rounds go away, except for the shape re-traversal of Decision rule 8. This slice amends ADR 0075 rule 5 and ADR 0067's note on rounds.
5. **Pass 1 and pass 2 merged.** The authority checks that read inferred headers are deferred until after inference, and so are the `gives` labels. One traversal remains (Decision rule 1).
6. **The old path removed,** after slice 5 has been differential-clean on the whole corpus, the hunt included. Its counters and the scaling tests stay.

Slices 3 to 5 are each a soundness change, with an adversarial review. The full differential runs on the release candidate of the first release that contains each of them.

## What this does not change

- **The language.** No rule of any ADR or of the specification on what a label, a summary, a timing label, the file-state label or an inferred header is. Only how the checker computes them.
- **Output, in every edition.** For editions `0.1` and `0.2`, `una check`, `una check --json`, `una fix`, `una core` and the obligations stay byte-identical, as a hard requirement of every slice. Edition 0.1 has no editable bindings and no summaries, so only the file-state slice (3) and the merge of the passes (5) reach it. Editions `0.3` and `0.4` are held to the same differential with no accepted difference.
- **The release lines.** Nothing is backported to `release/v0.1` or `release/v0.3`.
- **The fixpoints that are already linear or worklist-based** (`first_kinds`, `crossing::releases`, `installations`, the summary worklist's own order): left as they are, except where slice 4 absorbs the summary rounds.
- **The three phases of `uses` inference** and their order.
- **The U8001 bounds,** in purpose. Their places change (Decision rules 4 and 5), and none is removed.
- **The silent check of the standard modules** (CT-19).
- **No cache** is added. The constraint graph lives for one check.

## Consequences

- One traversal per body for every package, so `una check`, each `una fix` round and each editor check lose the factor of two to five that `within`, file writes and editable bindings cost today.
- The label fixpoints become data: a constraint graph whose solution is verified after solving. A wrong solution is U8001, never a silent label.
- The checker's label paths change in many places. Until slice 6 there are two paths to maintain, with the old one as the oracle.
- Peak memory of a check grows by the constraint graph, `O(B · j)`, the same order as the records it already holds.

## Alternatives considered

Options A, C and D are above, each with the reason it was not chosen. Two more:

- **A build-time or on-disk cache of round results.** Not chosen: it is a cache of label results, which would have to be trusted, and the project adds no new caches (scan, "No new cache is proposed").
- **Fewer rounds by a cheaper approximation of `τ` or the file-state label** (for example, `⊤` for any package with a `within`). Not chosen: it rejects honest programs that are accepted today, and the project's security rule is the most secure option that stays usable, not the fastest.

## Open questions

1. **The editor.** Should option C, incremental per-function summaries over B's constraint graph, be planned for `una lsp`, and when: before or after v1.0?
2. **How long the oracle lives.** Should the old path be kept for one minor release after slice 5, or removed in slice 6 as soon as the whole corpus is differential-clean? Keeping it costs a second implementation to maintain. Removing it loses the oracle for the hunt's future generators.
3. **Is I1 already true?** The slice 0 audit may find label-dependent branches outside the deferred checks that cannot be made label-independent. Each one forces a replay like Decision rule 6. If there are many, slice 5 may keep two traversals (collect, then check) with the label rounds gone. That is still a gain over today, but not the single pass. Should that outcome be accepted as the end state, or should the record be revised?
4. **Shapes of open positions.** Can Decision rule 8's re-traversal be removed by recording each editable binding's bounds as found types and pairing them after unification ("deferred pairing")? Or does a read of the binding inside the same declaration need its inner labels during the traversal?
5. **Measurement.** The counts here are read from the code, not measured. Slice 0's counters will give the real number of traversals per package on the examples and the conformance cases. Should the gain be measured in a quiet window as the release gates are, or are traversal counts enough evidence for this record?
6. **The order of slices 3 and 4.** Slice 3 (timing and file state) removes the most traversals for servers, and slice 4 (editable bindings) the most for generic library code. Which comes first is a priority call.
