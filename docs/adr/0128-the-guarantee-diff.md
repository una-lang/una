# 0128. The guarantee diff

- Status: Accepted
- Date: 2026-10-31
- Deciders: Una project (accepted on 2026-11-02, with the changes of "Decisions of acceptance")
- Resolves: issue #1291 (the guarantee diff)
- Related:
  - ADR 0127 (Una Quality foundation) and `spec/32` (Draft), with which this record shares symbol identity (`spec/32 §3` rule 4), the snapshot header, the output model, the analysis statuses and the exit codes; Una Quality reads this record's verdicts as UQ9101 to UQ9109;
  - issue #1293 and ADR 0129 (specification mutation testing and trivial-contract detection), whose results this record reports as deltas;
  - issue #1289 (requirement coverage), whose traceability this record compares;
  - issue #52 (the semantic diff, `una diff`, of `plan/backlog.md`), which this record realizes for requirements, contracts, capabilities, information, escapes and the trusted base;
  - `spec/13 §6` (the build report), whose facts are the first input of the diff;
  - ADR 0092 (conditions on allowed effects and the locked contracts of `[contracts]`), ADR 0093 (authority at module boundaries), ADR 0094 (on whose behalf an operation answers);
  - `spec/12` (verification): the decidable fragment (§2), statuses (§6), obligation identity (§8), lock and cache (§9); ADR 0041 and ADR 0064 (the proof cache and its authentication);
  - `spec/08` (requirements and tests), whose rule 2 promises that a diff surfaces a requirement's activation;
  - ADR 0083 (`una.core/1`), the encoding contract terms are carried in;
  - ADR 0088 §6 (attestations), for the provenance this record defers.
- Target: the toolchain. No construct of the language changes, no diagnostic code `U....` is added, and no edition changes.
- Amends, on acceptance and with the slices that implement it (section J):
  - **Specification:** a new chapter, `spec/33-guarantee-diff.md`, with the categories, the comparison rules, the documents, the approvals file and the exit codes;
  - **Diagnostics and ABI:** `spec/13 §5` (the command `una diff`);
  - **Quality:** `spec/32 §18` (the codes UQ9105 to UQ9109, rule 25) and `§14.4` (approvals of UQ9101 to UQ9109, rule 22).

> **In plain words.**
> - **What it is.** `una diff` compares a package with a baseline and lists what changed in what a reader can trust: contracts, authority, `trusted` code, `checked` escapes, declassifications, obligations, the solver pin and requirements.
> - **How it decides.** Lists of facts are compared as lists. Contracts are compared by logical implication with the solver, where the fragment of `spec/12` allows. Where it cannot decide, the answer is "review required" or "inconclusive", never "unchanged".
> - **What blocks.** With `--gate`, every regression blocks unless an approval recorded in the repository covers exactly that change. A flag cannot approve anything.
> - **What it never does.** It never reports more proven obligations as an improvement, never runs `git`, and never uses the network.

## Questions for acceptance

Four points rest on assumptions that acceptance confirms or changes; the answers are under "Decisions of acceptance":

1. **A relaxed precondition blocks (rule 8).** A weaker `requires` breaks no caller, so classical compatibility rules accept it. But a precondition is also a restriction the program enforces: removing `requires is_admin(user)` lets every caller reach the function. This record assumes the most secure reading: a relaxed precondition of a public function is a regression that needs an approval. The alternative, reporting it without blocking, is recorded under Alternatives.
2. **Approvals live in their own file (rule 22).** ADR 0127 rule 28 makes a waiver in `quality.toml` the approval of UQ9101 to UQ9104. That file can be replaced by a pipeline by path (ADR 0127 rule 20), and the issue asks for an approval recorded in the repository. This record assumes that approvals of guarantee changes live in `guarantees.toml` at the package root, read from that fixed path only, and that, on acceptance of both records, ADR 0127 rule 28 is aligned: a `quality.toml` waiver of UQ9101 to UQ9109 is UQ0001. Both records are Proposed, so the alignment is a decision for acceptance, not an edit made here.
3. **The approver cannot be verified (rule 22).** As in ADR 0127 Question 2: the tool records the approver's text and binds the approval to one change; the review of the change that adds the entry is the approval. Signed approvals are deferred.
4. **The command is `una diff` (rule 1).** The name comes from the backlog row of issue #52. The issue calls the report the guarantee diff; the schema names say so (`una.guarantee-diff/1`).

## Decisions of acceptance (2026-11-02)

The project accepted this record on 2026-11-02 with the changes below, which the rules carry:

1. **Question 1: accepted.** A relaxed precondition of a public function is a regression that needs an approval (rule 8). The witness of the refuted implication, an input the new precondition accepts and the old one refused, is shown with the change, so the reviewer sees what is now let in.
2. **Question 2: accepted.** Approvals of guarantee changes live in the approvals file at its fixed path (rule 22), and ADR 0127 rule 28 is aligned: a `quality.toml` waiver of UQ9101 to UQ9109 is UQ0001. The file's TOML form is interim only: its final form is `guarantees.una`, in Una's own data syntax, a member of ADR 0131's family. The question and Question 3 pointed to rule 20; they name rule 22.
3. **Question 3: accepted.** The tool records the approver's text and cannot verify it; signed approvals stay deferred.
4. **Question 4: accepted.** The command is `una diff`.
5. **The quality configuration cannot lower these codes** (rule 24). UQ9101 to UQ9109 take no severity below `block` in `quality.toml`; a configuration that sets one lower, or waives one of them or UQ9201, is UQ0001 (ADR 0127 rules 21 and 28). So `una quality --gate` never passes a change that `una diff --gate` blocks.
6. **Expiry follows the preset** (rule 22): an approval new against the baseline expires within the preset's `max_days` (ADR 0127 rule 28: 30 days in `high_assurance`, 90 in the others), never more than 90 days.
7. **Reviewed equivalent mutants join the approvals file** (rule 22, ADR 0129 rule 12), with a code of their own, UQ9302: what shrinks a denominator lives at a fixed path, as an approval does.

Open items, each owned by the card named:
- How `una diff`, which takes no configuration by flag, learns the preset in effect for the bound of decision 6, from a fixed path only: slice 4, #1319.
- The grammar of `guarantees.una` (its head word `guarantees`, by ADR 0131 rule 5) and its conversion by `una migrate`: ADR 0131's slices, from #1333.

## Glossary

- **Fact:** a value the compiler states about the package in the build report (`spec/13 §6`) or the obligation listing (`spec/13 §2`), such as a function's `uses` or a trusted module. Facts are compared as values.
- **Relation:** a conclusion the diff infers: two contracts compared by implication, two symbols matched by fingerprint. Every relation states its basis.
- **Change:** one difference between the baseline and the current package, with a category, a verdict and a stable id.
- **Verdict:** `regression`, `review_required`, `inconclusive` or `change` (rule 4).
- **Approval:** a recorded, expiring acceptance of one regression, by its change id.
- **Guarantee snapshot:** the document of every fact and contract of a package that the diff compares (`una.guarantee-snapshot/1`).

## Context

`una check` decides whether one version of a package is correct against its own contracts. It cannot say whether a new version promises less than the old one: a change that deletes an `ensures`, adds an operation to `uses` or moves code into a `trusted` module still checks. An agent that simplifies a function by dropping the condition that made it hard to prove produces a smaller, better-covered function that checks, and a reviewer who reads only the result sees an improvement. The issue (#1291) and the Una Quality epic (#1292) ask for the opposite view: what changed in what a reader can trust.

The facts exist:
- the build report (`spec/13 §6`) lists, per function, its `uses`; per handler its authority; the dependencies and their ceilings; the policy calls; the `checked` counts against `[escape_budget]`; the capabilities' assumptions; the trusted modules, host capabilities, solver and proof cache; the requirements and their derived functions; and the counts of obligations by status;
- the obligation listing gives each obligation's stable id, kind, goal and status (`spec/12 §8`);
- `una.toml` and `ita.una` hold the grants, `[boundaries]` (ADR 0093), `[contracts]` locks (ADR 0092 rule 8), `[trusted]` and `[escape_budget]`; `verification.lock` and `ita.lock` pin the solver;
- `una.core/1` (ADR 0083) carries every contract clause and refinement in a stable encoding with spans.

The forces:
- **A diff of text is not a diff of guarantees.** Moving a condition from `requires` into a refined parameter kind changes the text and not the contract; writing `x < 10` as `x <= 9` changes nothing. Comparing contracts needs implication, which the solver decides in the fragment of `spec/12 §2`.
- **The solver does not always decide.** Opaque predicates, `F64` queries in the logic `ALL`, a changed predicate body and an `unknown` all leave a comparison open. Reporting an open comparison as "unchanged" would let a weakening pass silently.
- **Counts mislead.** More proven obligations may mean more code or one obligation split in two. A count without its denominator and its cause is not evidence of improvement.
- **Approvals are the weak point.** An agent can pass a flag. Only a file reviewed with the change can carry an approval, and it must be bound to that change so it cannot be reused.
- **Renames are common.** A function moved to another module must not show as one removal and one addition, which would hide whether its contract changed.

### Departures from the issue

- **"Contracts weakened ... a `requires` tightened"** is extended in both directions: a relaxed `requires` is a regression too (Question 1).
- **"Quality-report deltas"** are not part of this report. ADR 0127 rule 17 gives structure, tests and coverage to the quality diff; this record compares guarantees only, and Una Quality reads its verdicts.
- **"Requirement coverage and specification-quality changes, when those cards land"** are reported as deltas; specification-quality deltas never block here (rule 17), since they come from sampling and heuristics.

## Decision

### A. Shape

1. **The command.** `una diff` compares guarantees:
   - `una diff [path] --baseline <snapshot>`: checks the package at `path` at tier 1 (with the proof cache, `spec/12 §9`), builds its guarantee snapshot in memory, and compares it with the baseline.
   - `una diff --from <snapshot> --to <snapshot>`: compares two snapshot files and checks no package. The report says both sides came from files.
   - `una diff snapshot [path] --out <file>`: checks the package at tier 1 and writes its `una.guarantee-snapshot/1`.
   - Flags: `--json` (one `una.guarantee-diff/1` document), `--sarif` (one SARIF 2.1.0 log; with `--json`, a usage error), `--gate` (rule 21), `--date <YYYY-MM-DD>` (the date approvals expire against, as ADR 0127 rule 30), and the solver flags of `una check`.
   - Rejected: `--baseline <git ref>`. As ADR 0127 rule 25: the tool reads files only and never runs or emulates a version-control system.
2. **The baseline artifact.** `una.guarantee-snapshot/1` shares the header of `una.quality-snapshot/1` (`spec/32 §17.2`): `schema`, `tool` (version and the digest of the built-in solver), `package` (name, version, edition), `source_digest`, `date`, and `symbols`, each with its kind, qualified name and body fingerprint (`spec/32 §3` rule 4). It adds:
   - `project`: the SHA-256 of each project file (`una.toml` or `ita.una`, `verification.lock` or `ita.lock`, `quality.toml`, `guarantees.toml`, or `quality.una` and `guarantees.una` once ADR 0131's slices land), and the parsed values the diff compares: grants, `[boundaries]`, `[contracts]`, `[trusted]`, `[escape_budget]`, `[labels]`, the lock's fields, and the dependencies with their sources, digests and ceilings;
   - per function and handler operation: `public`, the parameters with their types, the result type, `fails`, `uses` with operations, conditions and labels, and the contract: every `requires` and `ensures` clause and every refinement the parameters and result carry, as `una.core/1` terms;
   - `context`: every kind, predicate and capability the contracts name, with its definition as `una.core/1` and its fingerprint, so a comparison can be made from two snapshots without sources;
   - per symbol: its obligations, each with id, kind, normalized goal, status, reason and the digest of its query;
   - the build report's facts: `authority`, `handler_authority`, `dependencies`, `information`, `escapes`, `assumptions`, `tcb`, `requirements` and `coverage`, and the `checked` sites with their spans;
   - `approvals`: the entries of `guarantees.toml`, so the next diff can tell an approval added by the change under review.
   It holds no value of a run. A snapshot is untrusted input: the report prints its digest, and the recommended pipeline recomputes the baseline from the base revision rather than reading a file from the change under review (ADR 0127 rule 25).
   - Rejected: diffing two `una report --json` documents. The build report has no contract terms, no context and no symbol fingerprints, so it could compare lists but never decide an implication or follow a rename.
3. **One output model with Una Quality.** The finding record of `spec/32 §16` is the shape of a change in SARIF and in `una quality`; the statuses of `spec/32 §2` are the statuses of each part of the analysis. The diff owns contracts, assumptions, authority, trust, escapes, information, obligations, toolchain and requirements. Una Quality owns structure, tests and coverage, and reads the diff's verdicts (ADR 0127 rule 17). Neither computes what the other owns.

### B. Verdicts, basis and scope

4. **Verdicts.** Each change has one verdict:
   - `regression`: the current package promises less, assumes more, can do more, or trusts more than the baseline;
   - `review_required`: the change is outside what the diff can decide (a comparison outside the fragment, an opaque predicate whose body changed, a signature that cannot be mapped);
   - `inconclusive`: a decision was attempted and failed (the solver answered `unknown`, reached its budget or its wall-clock guard);
   - `change`: a difference that is reported and does not block, such as a stronger `ensures` or a removed `trusted` module.
   A comparison is `unchanged` only when it was decided: the two sides are equal as values, or equivalent by two proven implications. Nothing undecided is ever `unchanged`, and `review_required` and `inconclusive` block under `--gate` like a regression (rule 21).
5. **Basis.** Each change states how it was found: `fact` (two values of the build report or the project files differ), `solver` (an implication query, with its digest and, when refuted, the solver's model as a witness), `match` (a symbol matched by fingerprint) or `heuristic` (a symbol matched as `extracted_from`). The human report and the documents keep facts and inferred relations apart, as the issue requires.
6. **Scope and limits.** Each comparison states what it covered: the clauses compared, the context it assumed (rule 9), the logic of its query, and what it left out. The report lists, once, what the diff never compares (section I) and the `out_of_model` channels of `spec/13 §6`.

### C. Symbol identity

7. **Matching.** Symbols are matched by `spec/32 §3` rule 4, unchanged: equal qualified names; then equal and unique body fingerprints (`renamed`); then `extracted_from` (`heuristic`); the rest are `new` or `removed`. Consequences for guarantees:
   - a `renamed` public function is compared as one function, so its contract change shows as such, not as one removal and one addition;
   - an `extracted_from` match is reported but never used to call a contract `unchanged`: the extracted function is compared as new;
   - a removed public function is a regression (category `symbol.public.removed`), since its callers lose it; a new public function is a `change`, and its authority and trust count in rules 11 to 13;
   - ambiguous fingerprints match nothing (`spec/32 §3` rule 4), so an ambiguity can only add regressions, never hide one.

### D. Contracts and assumptions

8. **Comparing a contract.** For a matched public function or handler operation, with parameters mapped (rule 10):
   - the **effective precondition** `P` is the conjunction of its `requires` clauses and the refinements of its parameters' types; the **effective postcondition** `Q` is the conjunction of its `ensures` clauses and the refinement of its result type. Moving a condition between a clause and a refined kind therefore changes nothing;
   - with `Γ` the context of rule 9, the diff decides `Γ ∧ P_old ⇒ P_new` and `Γ ∧ P_new ⇒ P_old`, and `Γ ∧ P_old ∧ P_new ∧ Q_new ⇒ Q_old` and its converse, each as the satisfiability of its negation, one query each, in the encoding of `spec/12 §4`;
   - the outcomes:

| Precondition | Category | Verdict |
| --- | --- | --- |
| equivalent | none | unchanged |
| stronger: an input the old one accepted is now rejected | `contract.requires.strengthened` | regression: breaks callers |
| weaker: an input the old one rejected is now accepted | `contract.requires.relaxed` | regression (Question 1) |
| neither implies the other | `contract.requires.incomparable` | regression |

| Postcondition | Category | Verdict |
| --- | --- | --- |
| equivalent | none | unchanged |
| weaker: a result the old one excluded is now allowed | `contract.ensures.weakened` | regression: promises less |
| stronger | `contract.ensures.strengthened` | change |
| neither implies the other | `contract.ensures.incomparable` | regression |

   - a refuted implication gives a witness: the solver's model over the parameters (and `result`), shown as `spec/12 §7` shows a counterexample, labeled as a model of the query, not a run;
   - a text-equal contract whose context fingerprints are equal on both sides is `unchanged` by `fact`, with no query;
   - `fails`: a variant added to the declared error kind is a regression (`contract.fails.widened`), since callers must handle more; a removed variant is a `change`;
   - the queries are cached in the proof cache like obligations (`spec/12 §9`), keyed by their text.
9. **The context of a comparison.** `Γ` holds the ground facts of `spec/12 §3` and the definitions the contracts name:
   - a kind, reflected predicate or capability whose fingerprint is equal on both sides is one symbol in the query;
   - one whose fingerprint differs is declared twice, once per side, with each side's definition, so the comparison sees the difference; a definition outside the encoding makes the comparison `review_required`;
   - an opaque predicate is one uninterpreted symbol when its fingerprint is equal on both sides; when its body changed, every comparison that names it is `review_required`, with the predicate named;
   - a query with an `F64` term is sent in the logic `ALL` (`spec/12 §9`); an `unknown` there is `inconclusive`.
10. **Mapping signatures.** Parameters are mapped by name and type; a parameter renamed at the same position with the same type is mapped by position, with basis `match`. A parameter added, removed or retyped, or a changed result type, is `signature.changed` (a regression for a public function), and its contract comparison is `review_required`, since the two formulas range over different inputs.
11. **Assumptions.** The operations of a capability declaration are assumptions about the environment (`spec/13 §6` `assumptions`): their `ensures` are hypotheses for every caller. The direction is reversed: an operation's `ensures` that became **stronger** assumes more and is a regression (`assumption.strengthened`); a weaker one is a `change`. A new operation of a capability is a `change`; its use counts in rule 12.
12. **Locks.** A `[contracts]` entry (ADR 0092 rule 8) removed, an operation added to it, or a lock condition `C_lock` that became weaker (decided by implication, as rule 8) is a regression (`contract.lock.weakened`). A new or stronger lock is a `change`.

### E. Authority, trust, escapes and information

13. **Authority** (`authority.*`, code UQ9101), each by `fact` unless stated:
    - an operation `(capability, operation)` the package's footprint did not have (`authority.footprint.widened`);
    - a public function's or handler's `uses` gaining an operation or a binding (`authority.uses.widened`); a private function's is a `change`, since its callers' headers bound it;
    - a `where` condition on an operation (ADR 0092) removed, or replaced by one the old condition does not imply (decided by implication, basis `solver`), (`authority.condition.relaxed`);
    - an operation's label (ADR 0094) raised so that it answers on behalf of more (`authority.label.widened`);
    - a grant of `[run] grants` added or widened, a `[boundaries]` ceiling (ADR 0093) widened or removed, a dependency's ceiling or effective ceiling widened, a new host capability (`authority.grant.widened`, `authority.ceiling.widened`).
14. **Trust** (UQ9102): a module added to `[trusted]` (`trusted.module.added`); a trusted module whose content digest changed (`trusted.module.changed`), since its body is outside proof and any change in it is unproven; a dependency's trusted module the root newly consents to, or a new registry source (`trusted.dependency.added`). A removed trusted module is a `change`.
15. **Escapes** (UQ9105): a new `checked` site, matched within a matched symbol by its predicate text and ordinal (`escape.checked.added`); a raised `[escape_budget]` (`escape.budget.raised`). A removed site is a `change`.
16. **Information** (UQ9103): a new policy call site (`information.declassification.added`); a policy whose `from` or `to` label, `via` function, purpose or owner changed (`information.policy.changed`); a label category removed from `[labels]` (`information.category.removed`); a new entry of `information.timing` (`information.timing.added`). A removed policy call is a `change`.

### F. Obligations, toolchain, requirements and specification quality

17. **Obligations** (UQ9106). Within a matched symbol, obligations are matched by kind and normalized goal, as multisets, since the id of `spec/12 §8` includes the module path and changes on a rename:
    - a status that fell from `proven` to `unknown` or `refuted` is a regression (`obligation.regressed`);
    - a new or removed obligation is a `change`; a removed one is listed with its goal, and the loss of the property it proved is caught, where it matters, by the contract comparison of rule 8;
    - an obligation whose goal is equal and whose query digest differs is `obligation.reproved` (a `change`): its context changed and the proof was redone;
    - the report prints the counts by status of both sides, eligible included, and the number solved and cached in this run. **More proven obligations is never reported as an improvement**: the summary states the counts, the number added and removed, and no direction.
18. **Toolchain** (UQ9107): a changed `verification.lock` field or `ita.lock` solver section, or a changed edition, is a regression (`toolchain.lock.changed`, `toolchain.edition.changed`), since every proof then rests on a different verifier or different rules. When the two snapshots were made by different versions of `una` or different built-in solvers, the change `toolchain.verifier.changed` is reported, and every obligation and comparison of the report carries the marker `verifier_changed`: proofs made by a different verifier are flagged for special review. It does not block by itself, because the lock pins the solver the project chose.
19. **Requirements** (UQ9108), from the build report's `requirements` and `spec/08 §1`:
    - a requirement removed, or its `text` changed (`requirement.removed`, `requirement.text.changed`);
    - a requirement whose `status` became `active` or whose `source` changed (`requirement.activated`, `requirement.source.changed`): `spec/08 §1` rule 2 makes activation a person's decision, and this is where the diff surfaces it;
    - a requirement that lost a derived function or its last test (`requirement.derivation.lost`);
    - `requirement.traceability` of #1289, with numerator and denominator on both sides, when that card lands; until then `not_measured`.
    A new requirement is a `change`.
20. **Specification quality** (after ADR 0129): new and resolved trivial-contract findings and the change of `spec.sensitivity` per contract, with raw counts, as `change` entries only. They come from sampled mutants and heuristic checks, so they never block here; Una Quality's evolution rules may gate them by its policy.
21. **Policy files.** A changed `quality.toml` is reported with its digests and the rules it lowered against the baseline's (`policy.quality.weakened`, a regression; otherwise a `change`), as ADR 0127 rule 21 requires. A changed `guarantees.toml` is a `change`, and its approvals are listed by rule 23.

### G. Approvals and the gate

22. **The approvals file.** Approvals live in `guarantees.toml` at the package root, read from that path only: no flag, environment variable or configuration names another. Its TOML form is interim only, as ADR 0127 rule 20: the final form is `guarantees.una`, in Una's own data syntax (ADR 0131), and `una migrate` converts the file as it converts `quality.toml`. The same file holds the reviewed equivalent mutants of ADR 0129 rule 12, as entries of their own that never approve a change. The interim form:

    ```toml
    schema = "una.guarantee-approvals/1"

    [[approval]]
    change = "gd-3f2a9c01b7de"
    category = "contract.requires.relaxed"
    symbol = "billing.refunds.issue"
    reason = "refunds of any amount are now allowed for support staff, see the policy change"
    approver = "billing maintainers"
    expires = 2026-12-15
    ```

    - `change` is the change's id: `gd-` and the first 12 hex digits of SHA-256 over the category, the symbol's baseline and current names, and the SHA-256 of the before and after values (for a contract, the Core terms of both sides and their context fingerprints). An approval therefore covers exactly one change; any further edit of either side gives another id, and the approval no longer applies;
    - every key is required; an unknown key, a wrong type or an entry whose `category` or `symbol` disagrees with its `change` is invalid;
    - `expires`, for an approval new against the baseline, is at most the preset's `max_days` after the run's date (ADR 0127 rule 28: 30 days in `high_assurance`, 90 in the other presets), and never more than 90 days;
    - the file is data: nothing in it is executed.
    - Rejected: approvals as waivers in `quality.toml` (Question 2), and a command-line flag (the issue: "not a flag an agent can pass").
23. **What every report shows.** Every approval, with its state: `active` (it covered a change in this run), `unused` (no change has its id), `lapsed` (expired) or `invalid`. An approval new against the baseline's `approvals` is marked `added_in_this_change`, so a reviewer sees that the change approves itself. `una fix`, Una Quality's recommendations and every repair never add, edit or suggest an approval.
24. **The gate.** Without `--gate`, the run reports and exits 0, or 1 for an invalid `guarantees.toml`, 2 for a usage error, 3 for an internal error. With `--gate`, the outcome is `block` when any change is a `regression`, `review_required` or `inconclusive` without an `active` approval, when an approval is `invalid`, when the current package does not pass `una check`, or when a part of the analysis is not `complete` (statuses of `spec/32 §2`; for example the solver is missing, or the baseline's schema or a metric version is not comparable). The exit codes:

    | Code | Meaning |
    | --- | --- |
    | `0` | No unapproved regression, review or inconclusive change; every part of the analysis complete |
    | `1` | A blocking change, an invalid approval, a package that `una check` rejects, or an incomplete analysis |
    | `2` | Usage error (`spec/13 §5`) |
    | `3` | Internal error (U8001) |

    The diff has no configuration of severities: under `--gate`, every guarantee regression blocks. Una Quality cannot lower them either: UQ9101 to UQ9109 take no severity below `block` in `quality.toml`, and a configuration that sets one lower is UQ0001 (ADR 0127 rule 21), so `una quality --gate` never passes a change that `una diff --gate` blocks. The recommended pipeline runs `una diff --gate`. Code `4` of ADR 0127 rule 27 is not used: an incomplete analysis always blocks the guarantee gate.
25. **Codes.** Each category maps to a code of the `UQ` registry, assurance gate, for SARIF and for `una quality`: UQ9101 authority, UQ9102 trust, UQ9103 information, UQ9104 contracts and public signatures, and, added to `spec/32 §18` on acceptance, **UQ9105** escapes, **UQ9106** obligation regressions, **UQ9107** toolchain, **UQ9108** requirements, **UQ9109** assumptions. A `review_required` or `inconclusive` change takes the code of its category, with its verdict in the record. An incomplete analysis is UQ0101; a package `una check` rejects is UQ0102.

### H. Output, determinism and security

26. **Output.** The human report prints a header (package, both sides' source digests, tool versions, the baseline's path and digest, the date), then the gate's outcome, then the changes grouped by verdict (unapproved regressions first, then review, inconclusive, approved, changes), each with its category, symbol, basis, the before and after values or clauses, the witness when there is one, and its approval; then the obligation counts of rule 17, the requirement counts, the approvals, the parts of the analysis that are not complete, and the limits of rule 6. Names and texts from the sources are escaped (ADR 0127 rule 31).
    - `--json` prints `una.guarantee-diff/1`, compact, keys in this order: `schema`, `tool`, `package`, `baseline` (`path`, `digest`, `tool_version`, `source_digest`, `kind`), `current` (the same, `kind` `package` or `snapshot`), `date`, `outcome` (`pass`, `block`), `summary` (counts by verdict and by category), `changes` (each `id`, `category`, `code`, `verdict`, `basis`, `symbol` with `baseline`, `current` and `match`, `before`, `after`, `scope`, `witness`, `approval`, `markers`), `obligations` (counts by status per side, added, removed, reproved, solved, cached), `requirements`, `approvals`, `analysis` (each part with its status and reason), `out_of_model`. Changes are ordered by verdict, then category, then file and position. Adding a key keeps the version; `category` is an open field, `verdict`, `basis` and `outcome` are closed.
    - `--sarif` prints one SARIF 2.1.0 log as `spec/32 §17.3`, with `tool.driver.name` `"una diff"`, one result per change whose verdict is not `change`, `ruleId` its code, `level` `error` when it blocks and `note` when approved, and the category, verdict, basis and change id in `properties`.
27. **Determinism.** The same baseline, sources, project files, tool version, date and solver flags give the same bytes. Comparisons run under the lock's step budget; a wall-clock result is `inconclusive` and marked non-deterministic, as `spec/12 §5` rule 4 marks an obligation.
28. **Security.** As ADR 0127 rule 31: snapshots and `guarantees.toml` are data, parsed with size and depth limits, unknown keys rejected; nothing in them is executed; the diff starts only the solver children of `una check` (ADR 0124); it reads the package's files and the paths given on the command line; it never runs `git` and never uses the network. The witness of a refuted comparison is a solver model over the contract's variables, never a value of a run.

### I. Out of scope

29. Not part of this record:
    - **a guarantee graph** (requirement, contract, function, authority, obligation, proof and artifact as one linked structure): later input, built on these snapshots if it is built;
    - behavior changes that change no contract, fact or obligation: the diff compares what the package states and proves, not what its code does (tests and Una Quality's mutation score speak to that);
    - the quality diff of structure, tests and coverage (ADR 0127);
    - signing of snapshots and approvals, and provenance in the style of SLSA (ADR 0088 §6);
    - a review of the toolchain's own source: the marker of rule 18 flags proofs made by a different verifier, and a change to the verifier's implementation in this repository gets its review through the repository's own process;
    - the internals of dependencies beyond their facts in the build report and the contracts the root package calls.

## Complexity

For V symbols, F facts and C contracts that differ between the sides:
- symbol matching: O(V log V), as `spec/32 §3`;
- fact comparison: O(F log F), sorted keys on both sides;
- obligations: O(k log k) per matched symbol with k obligations;
- contracts: at most four solver queries per contract that differs as text or context (two per side of rule 8), none for one equal by `fact`; each query is cached, and its cost is that of an obligation of `spec/12`;
- no pass is quadratic; scaling tests at growing V and F come with slice 1.

## Security

The gate cannot be passed by an undecided comparison (rule 4), by a flag (rule 22), by an approval of another change (the change id), by an approval that hides itself (rule 23), or by a missing analysis (rule 24). The diff adds no code to the trusted base of a checked program: it never changes what `una check` accepts. Its baseline is untrusted, and the recommended pipeline recomputes it (rule 2).

## Verification

No obligation kind is added. Contract comparisons are solver queries in the encoding of `spec/12 §4`, cached like obligations, whose `unknown` is `inconclusive` and whose out-of-fragment terms are `review_required`.

## Information flow

The diff reads labels, policies and policy calls as facts and reports their names and locations. Snapshots and reports hold no value of a run.

## Diagnostics

No `U` code is added. UQ9105 to UQ9109 join the `UQ` registry of `spec/32 §18`, each with an explain text, with slice 4.

## Consequences

- A reviewer and an agent see, in one report, every guarantee a change gave up, with the basis of each claim, before any measure of structure.
- A simpler function that dropped a condition can no longer pass as an improvement (the fixture of section J).
- Approvals become reviewable artifacts bound to one change, listed in every report.
- Every relaxed precondition of a public function needs an approval (Question 1): more approvals for library authors who generalize functions.
- The gate needs the solver for contract comparison; without it, comparisons are incomplete and the gate blocks.
- Deferred: the guarantee graph, signed approvals, provenance, comparison of contracts outside the fragment.

## Alternatives considered

- **Diff the text of headers.** Rejected: equivalent rewrites show as changes and changes hidden in a refined kind show as nothing.
- **Diff two build reports only.** Rejected: rule 2; no implication and no rename can be decided from them.
- **Treat an undecided comparison as compatible.** Rejected: it lets every weakening outside the fragment pass silently; the issue forbids it.
- **Report a relaxed precondition without blocking** (classical compatibility). The less restrictive option; rejected in favor of rule 8 because it lets an authorization condition disappear unapproved, and rejected again on acceptance (Decisions of acceptance 1).
- **Approvals as waivers in `quality.toml`, or as a flag.** Rejected: rule 22.
- **Approval by symbol and category, not by change.** Rejected: one approval would cover every later weakening of the same function.
- **Count proven obligations as progress.** Rejected: rule 17.
- **A guarantee graph now.** Rejected by the issue: start with the facts that exist.

## Implementation plan

### J. Slices

Each slice is its own card and pull request, lands its part of `spec/33` from Draft to Normative with fixtures, and states its complexity:

1. **Snapshot and facts** (#1316). `una.guarantee-snapshot/1`, `una diff snapshot`, `una diff --baseline` and `--from/--to`, symbol matching, the fact comparisons of rules 13 to 16, 18, 19 and 21, the human report and `una.guarantee-diff/1`; scaling tests.
2. **Contract comparison** (#1317). Rules 8 to 12: effective conditions, context, signature mapping, implication queries, witnesses, assumptions and locks.
3. **Obligations** (#1318). Rule 17: matching by kind and goal, status regressions, reproved obligations, the counts with denominators.
4. **Approvals and the gate** (#1319). `guarantees.toml`, change ids, approval states, `--gate`, exit codes, SARIF, the codes UQ9105 to UQ9109 in `spec/32 §18` with explain texts, and the seeded-regression suite of the Conformance section, the cosmetic-improvement fixture included.

## Conformance

- **Seeded regressions,** one fixture per category of sections C to F, each blocking under `--gate`, and one approved copy of each that passes.
- **The cosmetic-improvement fixture** (the issue's required test). The baseline has a public function `billing.refunds.issue` whose cognitive complexity is 24, whose test coverage is low, and whose contract carries an authorization condition. The change rewrites it to cognitive complexity 9 with 92% line coverage, all obligations proven, and removes the authorization in one of three forms, each a fixture:
  1. `requires is_support(agent)` removed: `contract.requires.relaxed`, UQ9104;
  2. `ensures result.issued implies is_support(agent)` removed: `contract.ensures.weakened`, UQ9104;
  3. `uses @ledger: Ledger[credit where is_support(agent)]` written without its `where`: `authority.condition.relaxed`, UQ9101.
  Each must make `una diff --gate` exit 1 with no approval, and make the assurance gate of `una quality --gate --baseline` block whatever its structural results.
- **Equivalent rewrites** report `unchanged`: `x < 10` against `x <= 9`; a `requires` moved into a refined parameter kind.
- **Undecided comparisons** are never `unchanged`: an opaque predicate whose body changed gives `review_required`; a query forced to `unknown` gives `inconclusive`; both block without an approval.
- **Renames:** a public function moved to another module with its contract weakened gives one `contract.ensures.weakened` on a `renamed` match, not a removal and an addition.
- **Approvals:** an approval of a different change id is `unused` and blocks nothing it does not cover; an expired one is `lapsed`; one added in the change is marked `added_in_this_change`.
- **Counts:** a change that splits one obligation into two reports the counts and no improvement.
- **Determinism and isolation:** two runs give identical bytes; the tests use no network and run no `git`.
