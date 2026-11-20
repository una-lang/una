# 0139. Edition numbering follows the release that closes it

- Status: Accepted
- Date: 2026-11-06
- Deciders: Una project (the numbering decided on 2026-11-05; this record accepted, first in the order of acceptance, by the project's decision of 2026-11-06)
- Resolves: issue #1620
- Related:
  - ADR 0121 (the terms of a preview edition), whose rules are written for edition `0.4` and whose rule 15 already names `v0.4.0` as the release that makes edition `0.4` stable;
  - ADR 0056 and its amendments of 2026-10-13 (cumulative editions) and of 2026-10-19 (the terms of edition `0.4`);
  - ADRs 0119 (collections) and 0120 (the numeric foundation), which target edition `0.4`, and ADR 0130 (general protocols and typestate), which targets "the first edition after `0.4` (expected `0.5`)";
  - ADRs 0126 (module constants) and 0137 (`const` and `var`), which already target edition `0.5` as issue #1620 numbers it;
  - issue #1621 (the edition split: the code, specification, tests and packages of collections and numerics move to edition `0.5`), issue #1622 (the documents that state the numbering) and issue #1585 (retiring editions `0.1` and `0.2`: the plan for what leaves `main`).
- Target: the numbering of releases, compiler versions and editions, and the plan of which edition each accepted record lands in. No rule of the language changes. Editions `0.1` to `0.3` keep their names, their terms and their promise; the benchmark baseline, pinned to the release line of edition `0.1`, does not move.
- Amends, on acceptance, by dated amendments appended to each record (Appendix A): ADR 0121; the Target lines of ADRs 0119, 0120 and 0130. The specification (`spec/13 §7`, `spec/01 §5`, `spec/28`, `spec/29`), the guide and the code change with issue #1621, and the state, plan, README and release-gate documents with issue #1622.

## Context

Three numbers name the state of Una today, and they do not agree.

- **Releases** are planned on the board as R0.4, R0.5 and so on, each split into increments (R0.4.1 to R0.4.5, R0.5.1 to R0.5.6).
- **Compiler versions** are `v0.1.x` and `v0.3.x` on the release lines `release/v0.1` and `release/v0.3`; `main` is at `0.3.16`.
- **Editions** are what a package names (`spec/13 §7`): `0.1` and `0.2` stable, `0.3` a preview, `0.4` a preview named in `ita.una`.

Edition `0.4` was opened by ADR 0116 rule 0 and holds, by the records that target it, long-running programs (ADR 0116), tasks and time (ADR 0117), collections (ADR 0119), the numeric foundation (ADR 0120), packages and Itá (ADR 0122) and HTTP servers (ADR 0123). Those records do not fit in one release: the release that the board calls R0.4 ships the first group, and collections and numerics are planned for R0.5. Meanwhile ADR 0137 replaces `let` and `let edit` with `const` and `var`, and ADR 0126 adds module constants written `const`; both must land in one edition, since `let` and `const` must never name a binding in the same edition, and that edition cannot be `0.4`, which closes without them.

Without one rule, "0.4" means a release on the board, a version line of the compiler that does not exist yet, and an edition whose contents span two releases. A reader of `STATE.md`, of a changelog or of `una --version` cannot tell which is meant.

ADR 0121 already points at the answer: its rule 15 makes edition `0.4` stable at "a minor release, `v0.4.0`". This record makes that the general rule and moves the records that do not land by `v0.4.0` to the edition of the release that does land them.

## Decision

1. **One number.** Release R0.N ships compiler version `v0.N.0`. Patch releases `v0.N.x` follow it on its release line, as today.

2. **An edition is named by the release that closes it.** Edition `0.N` is closed by release R0.N: `v0.N.0` is the minor release at which edition `0.N` stops being a preview and gains the promise of a stable edition, by the stabilizing record and the gates of ADR 0121 rules 14 to 16. Before that release, edition `0.N` is the current preview, under the terms of ADR 0121 as Appendix A extends them to every preview edition.

3. **Only releases that change syntax create an edition.** A release that changes no form of the language is a toolchain release: it closes no edition, and no edition carries its number. A construct that needs an edition waits for the next release that creates one.

4. **At most two editions on `main`.** `main` serves at most two editions: the stable one (the newest closed edition) and the current preview. An older edition leaves `main` only by a plan that the project accepts for it, with the dated amendments it needs; the plan of issue #1585 does so for editions `0.1` and `0.2`. An edition that leaves `main` stays served by the release lines that already serve it (`release/v0.1`, `release/v0.3`), which this rule does not touch. Until its plan lands, an edition stays on `main` with its terms and its promise unchanged.

5. **The last edition is the stable edition of 1.0.** The last edition closed before release 1.0 becomes the stable edition that 1.0 ships.

6. **The plan.** The accepted records land in these editions. A record moves to another edition only by a dated amendment of its Target line, as Appendix A does for ADRs 0119, 0120 and 0130.

   | Release | Compiler | Edition it closes | Contents |
   | --- | --- | --- | --- |
   | R0.4 | `v0.4.0` | `0.4` | Long-running programs (ADR 0116), tasks, `within` and `first of` (ADR 0117), HTTP servers (ADR 0123), the core of Itá (ADR 0122), and the other records that target edition `0.4` and are not moved here |
   | R0.5 | `v0.5.0` | `0.5` | Collections (ADR 0119) and the numeric foundation (ADR 0120), moved from `0.4`; module constants (ADR 0126) and `const` and `var` (ADR 0137) |
   | R0.6 | `v0.6.0` | `0.6` | Data-driven and multi-step tests (the testing-family records, issues #1590 to #1594, as each is accepted) |
   | R0.7 | `v0.7.0` | none | A toolchain release |
   | R0.8 | `v0.8.0` | `0.8` | General protocols and typestate (ADR 0130), moved from "the first edition after `0.4`"; the laws record of issue #1623 is planned for R0.8, and states its own edition |
   | R0.9 | `v0.9.0` | none | A toolchain release |
   | 1.0 | `v1.0.0` | | The last closed edition becomes the stable edition (rule 5) |

   Editions `0.1`, `0.2` and `0.3` already carry the number of the release that introduced them; edition `0.3` becomes stable no later than edition `0.4` (ADR 0121 rule 14). `F64` (ADR 0114) stays in edition `0.3`, and its rules stay in `spec/29` (ADR 0121 rule 20); only the constructs of ADR 0120 move.

7. **Migration of a preview that loses a construct.** A construct that a release accepted in edition `0.4` and that this plan moves to edition `0.5` is, in edition `0.4`, a removed form under ADR 0121 rule 5: it is U7007, "This form of edition `0.4` changed in release `<version>`: it moved to edition `0.5`", with a repair that raises the package's edition in `ita.una` to `0.5`. That repair is a description, not an edit that `una fix` applies alone, since edition `0.5` also has the changes of ADR 0137, whose own repairs `una fix` then applies. The release that makes the move lists each moved construct under the changelog heading of ADR 0121 rule 6. A construct that no release has accepted in edition `0.4` moves with no diagnostic. Issue #1621 implements this rule.

8. **What does not change.** No rule of the language, no guarantee and no check changes: the move of a construct between two preview editions changes only which `edition` line admits it. ADR 0121 rule 17 binds the move as it binds every release: noninterference, authority checking and memory safety are not weakened for any program, and no trusted base or `out_of_model` entry is added. The benchmark baseline does not move.

## Costs

Time and space complexity: not applicable. This record numbers releases and editions and plans which edition each record lands in; it adds no algorithm and no run-time or check-time work. The edition gating that issue #1621 moves is a lookup in the compiler's existing keyword and ledger tables, whose costs do not change.

## Security

None of the properties of ADR 0121 rule 17 changes. Moving collections and numerics from edition `0.4` to edition `0.5` removes constructs from a preview edition, which ADR 0121 rule 3 allows; a package of edition `0.4` that used one gets U7007, never a silent change of meaning (ADR 0121 rule 4). The trust that ADR 0120 adds for every package (its numeric kernels, ADR 0121 rule 2 (b)) is unchanged by its new target and keeps its review.

## Consequences

- R0.N, `v0.N.0` and edition `0.N` mean one thing in the board, the changelog, `una --version` and the documents (issue #1622).
- Edition `0.4` can close at R0.4 with the records that are ready, and `const` and `var` land with module constants in one edition, `0.5`, so `let` and `const` never coexist.
- Editions `0.7` and `0.9` never exist; a reader sees a gap in the edition numbers where a toolchain release stands.
- Packages of edition `0.4` that use collections or numerics, in the repository's conformance cases, examples and packages, move to edition `0.5` with issue #1621.
- The website reads the preview editions from `spec/13 §7` (ADR 0121, Context); its markers follow when issue #1621 changes that section.
- The rule of two editions on `main` is reached edition by edition, each by its own accepted plan; until then `main` serves more.

## Alternatives considered

- **Keep edition `0.4` as the home of every record that targets it, and number it apart from the releases.** Edition `0.4` would then close at R0.5, `v0.4.0` would not close edition `0.4`, against ADR 0121 rule 15, and module constants would land in `0.4` beside `let`, against the rule that `let` and `const` never coexist.
- **Name an edition by the release that opens it.** The edition a package names would then say where its first construct appeared, not where its contents are frozen; a reader could not tell from the number which release promises it stable.
- **Create an edition for every release.** Editions `0.7` and `0.9` would be identical to the edition before them, and every package would face an edition change with nothing to migrate.

## Appendix A. Amendments to accepted records, appended on acceptance

Each amendment below is appended, with the same text, to the record it amends. The decided text of each record does not change.

**ADR 0121, Amendment of 2026-11-06: the terms apply to every preview edition (ADR 0139).**
- Rules 1 to 19 are written for edition `0.4`, with edition `0.3` as the edition before it. They apply to every preview edition `0.N`, with the edition before it in place of `0.3` (edition `0.6` for edition `0.8`, since there is no edition `0.7`), and with `v0.N.0` in place of `v0.4.0` in rule 15: edition `0.N` becomes stable at release R0.N, compiler version `v0.N.0` (ADR 0139 rules 1 and 2). The heading of rule 6 names the edition it concerns. Rule 13 (edition `0.3`) and rule 14 (edition `0.3` becomes stable no later than edition `0.4`) are unchanged.
- **Glossary.** A release R0.N is the release whose compiler version is `v0.N.0`; its patch releases are `v0.N.x`. An edition is named by the release that closes it, and only a release that changes syntax creates one (ADR 0139 rules 2 and 3).
- **Rule 20.** In the table of chapters, the Edition column of `spec/28` (ADR 0119) reads `0.5`, and that of `spec/29` (ADR 0120) reads "`0.3` (`F64`); `0.5` (the rest)".
- **Records that move.** A construct that a release accepted in edition `0.4` and that ADR 0139 moves to edition `0.5` is a removed form under rule 5, U7007 with the repair of ADR 0139 rule 7.

**ADR 0119, Amendment of 2026-11-06: the target is edition `0.5` (ADR 0139).** The Target line reads: edition `0.5`, under the terms of ADR 0121 as its amendment of 2026-11-06 extends them to every preview edition. Every "edition `0.4`" of this record that names the edition its constructs exist in, including the `Map.update` of its amendment to ADR 0069, reads "edition `0.5`", and each comparison with edition `0.3` (the clashes, the edition test, rule 0's "what this record can break") is made with edition `0.4`, the edition before `0.5`. In editions `0.1` to `0.4`, `import std.collections` is U1001 at the module path, except where ADR 0139 rule 7 gives U7007 in edition `0.4`. No other rule changes.

**ADR 0120, Amendment of 2026-11-06: the target is edition `0.5` (ADR 0139).** The Target line reads: edition `0.5`. Editions `0.1` to `0.4` do not change, except for the U7007 of ADR 0139 rule 7 in edition `0.4`. Every "edition `0.4`" of this record that names the edition its constructs exist in, including rule 1 and the section "Edition `0.4`", reads "edition `0.5`", and each comparison with edition `0.3` is made with edition `0.4`. `F64` and the rules of ADR 0114 stay in edition `0.3`. No other rule changes.

**ADR 0130, Amendment of 2026-11-06: the target is edition `0.8` (ADR 0139).** The Target line reads: edition `0.8`, closed by release R0.8, as a preview under the terms of ADR 0121 as its amendment of 2026-11-06 extends them. Editions `0.1` to `0.6` do not change. No other rule changes.

## Amendment (2026-11-10): release R0.4 ships edition `0.4` as a preview

- Status: Accepted (2026-11-13), by the project's decision of 2026-11-13 on issue #1897. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs. It records the project's decision of 2026-11-10 on the version of release R0.4. No code changes with it: the compiler already serves edition `0.4` as a preview (`Edition::is_preview` in `crates/una-project`, the `preview:` line of `una --version`), and the documents that said `v0.4.0` makes edition `0.4` stable say "preview" with it (#1897).
- Issue: #1897.

**Why.** Rule 2 makes `v0.N.0` the release at which edition `0.N` stops being a preview, "by the stabilizing record and the gates of ADR 0121 rules 14 to 16". For edition `0.4` the two cannot both hold at `v0.4.0`. ADR 0121 rule 15 asks, among its checks, for a quiet period of at least four weeks and at least one release from the release that landed the last construct of `0.4`, and release R0.4 is the release that lands the last of them (HTTP servers, the core of Itá); no stabilizing record exists. A release numbered `v0.4.0` that promised a frozen edition `0.4` would contradict ADR 0121, and one numbered otherwise would contradict rule 1. The project decided on 2026-11-10 that release R0.4 is `v0.4.0` and that it ships edition `0.4` as a preview.

**The change.**

1. Rule 1 is unchanged for R0.4: it ships compiler version `v0.4.0`.
2. Rule 2 does not make edition `0.4` stable at `v0.4.0`. Release `v0.4.0` ships edition `0.4` as a preview, under the terms of ADR 0121 (rules 1 to 19, as its amendment of 2026-11-06 extends them): `una --version` lists it after `preview:`, and the report's `edition` section has `"preview": true` for it. The changelog heading of ADR 0121 rule 6 applies to every later release that changes it.
3. When edition `0.4` becomes stable is not decided here. A later dated amendment of this record names that release; the stabilizing record of ADR 0121 rule 15, with every check and gate it lists, quiet period included, still governs it. Until then, the sentence "That release is a minor release, `v0.4.0`" of ADR 0121 rule 15, and its reading "with `v0.N.0` in place of `v0.4.0`" in the amendment of 2026-11-06 (Appendix A), do not bind edition `0.4` to `v0.4.0`. On acceptance, a dated note with this paragraph is appended to ADR 0121.
4. ADR 0121 rule 14 is unchanged: edition `0.3` becomes stable no later than edition `0.4`. Since edition `0.4` stays a preview at `v0.4.0`, edition `0.3` may stay one too, as the compiler serves it.
5. In the table of rule 6, the R0.4 row reads: release R0.4, compiler `v0.4.0`, edition `0.4` as a preview (this amendment), with the same contents. The release that makes it stable is named by the later amendment of item 3.
6. Rules 3, 4, 5, 7 and 8, and the rows of editions `0.5` and later, are unchanged. The same tension between rule 2 and the quiet period of ADR 0121 rule 15 can arise for a later edition; the amendment of item 3 may settle it for every edition.

**What does not change.** No rule of the language, no guarantee and no check. A package of edition `0.4` is checked and run as before. The preview terms bind it: a later release may change it only as ADR 0121 rules 3 to 6 allow, and never in silence (rule 4). The benchmark baseline does not move.

**Security.** A preview promises less stability, not less safety: ADR 0121 rule 17 binds every release, preview or not, so noninterference, authority checking and memory safety are not weakened for any program, and no trusted base or `out_of_model` entry is added.

**Cost.** Not applicable: no algorithm and no run-time or check-time work.

**The documents.** `spec/13 §7`, the README, the guide page on `[package] edition` and `docs/plan/roadmap.md` say that `v0.4.0` ships edition `0.4` as a preview and cite this amendment as Proposed (#1897). Once it is accepted, they cite it as accepted; if it is refused, the project decides the version of release R0.4 again before any tag.

## Amendment (2026-11-20): each increment publishes the version it is named for

- Status: Accepted (2026-11-20), by the project's decision of 2026-11-20 on issue #2173. The accepted text of this record above this amendment, and the amendment of 2026-11-10, are unchanged; where they differ from this amendment, this amendment governs.
- Issue: #2173.

**Why.** Since 2026-11-18 the board splits each release R0.N into daily increments, and each increment is one published release (`ops/coordination/ROADMAP-INCREMENTS-2026-11-18.md`). Rule 1 numbered those releases as patches of the release before: the increments of R0.5 were planned as `v0.4.1`, `v0.4.2` and so on, and only the last one as `v0.5.0`. The board names them R0.5.1, R0.5.2 and so on. So a reader of the board, the changelog and the website saw a release planned as R0.5 published as `v0.4`, and release `v0.4.0`, published before the increments existed, holds 318 changelog entries where earlier releases hold 3 to 46. The project decided on 2026-11-20 that the number of a release is the number of its increment.

**The change.**

1. **Rule 1.** Release R0.N is published in increments R0.N.0, R0.N.1, R0.N.2 and so on, and increment R0.N.k publishes compiler version `v0.N.k`. The first increment of R0.N publishes `v0.N.0`. An increment that changes nothing in the shipped archive ships with the next one, and the next one keeps its own number; a number that no release carries is a gap, not an error.
2. **Rule 2.** `v0.N.0` no longer closes edition `0.N`. Edition `0.N` is the current preview through the increments of R0.N, under the terms of ADR 0121 as its amendment of 2026-11-06 extends them. The release that makes edition `0.N` stable is named by a later amendment, with the stabilizing record and every check and gate of ADR 0121 rule 15, quiet period included, as item 3 of the amendment of 2026-11-10 already does for edition `0.4`. Until then the readings "`v0.N.0` is the minor release at which edition `0.N` stops being a preview" (rule 2) and "with `v0.N.0` in place of `v0.4.0` in rule 15" (Appendix A) do not bind any edition to a version.
3. **Rule 6.** In the table, the Compiler column reads "`v0.N.0` to `v0.N.k`, one version per increment" for each release from R0.5 on. The contents of each release, and the edition each record lands in, are unchanged.
4. **R0.4 and R0.5.** Release R0.4 was published as `v0.4.0` before the increments existed. It stays as published: it is not split after the fact, since every version is a tag on a commit that passed the release gates (`docs/release-gates.md`) and no earlier commit of R0.4 passed them. The first published increment of R0.5, the work the board planned as R0.5.1 and R0.5.2, is `v0.5.0`, and it ships editions `0.4` and `0.5` as previews. The board renames those two increments R0.5.0, and each later increment R0.5.k becomes R0.5.(k-2), so that each label is the version it publishes. No version `v0.4.1` is published.
5. Rules 3, 4, 5, 7 and 8 are unchanged.

**What does not change.** No rule of the language, no guarantee and no check. Which edition a package names, and what each edition admits, are unchanged; so are the preview terms of ADR 0121 and the benchmark baseline.

**Security.** None of the properties of ADR 0121 rule 17 changes. Numbering a release is not a release gate: every published version still needs every gate PASS on its gated commit.

**Cost.** Not applicable: no algorithm and no run-time or check-time work.

**The documents.** The changelog section and the release notes of the work planned as `v0.4.1` are renamed `v0.5.0`. The plan of increments, the board and the website follow this amendment.
