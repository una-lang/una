# 0121. The terms of a preview edition: edition `0.4` adds constructs behind its own opt-in

- Status: Accepted
- Date: 2026-10-19
- Deciders: Una project (accepted on 2026-10-19)
- Resolves: issue #912
- Related:
  - ADR 0056 and its amendments of 2026-10-13 (edition `0.3`, cumulative editions, what "stricter" means) and of 2026-10-16 (ADR 0114's carve-out), which hold the preview rule of `spec/13 §7`;
  - ADR 0114 (`F64`, edition `0.3`), whose rule 15 made the one exception to that rule;
  - ADRs 0116 and 0117 (accepted), whose rule 0 and target line open edition `0.4` "under the terms of `0.3`";
  - the records on collections (ADR 0119, issue #895) and the numeric foundation (ADR 0120, issue #894), each of which asked this question of the project (their Questions 7 and 6).
- Target: the edition rules themselves. Edition `0.4`'s terms change; editions `0.1` and `0.2` keep their stability promise; edition `0.3` keeps its own terms (rule 13). The benchmark baseline, pinned to edition `0.1`, does not move.
- Amends:
  - **Diagnostics and project files:** `spec/13 §4` (one code), `§5` (`una --version`), `§6` (the report's `edition` section), `§7` (the terms of edition `0.4`);
  - **Guide:** `docs/guide/configure/package/edition.md`, and a section "Migrating to edition 0.4" of `docs/guide/una-for-agents.md`;
  - **Release gates:** `docs/release-gates.md` (the summary's edition line; the preview gate of rule 12, before `0.4` leaves preview);
  - **Accepted records:** ADRs 0056, 0116 and 0117 by dated amendments, and ADR 0114 by a dated note (Appendix A).

  These rules live in `spec/13 §7`, which already holds the editions; this record adds no chapter of its own. It also keeps the table of the specification's new chapters (rule 20).

## Questions for acceptance

Each question changes what a programmer can rely on. The project accepted every recommendation on 2026-10-19; each question states its decision, and the text below follows it.

1. **May a preview edition add constructs that let a program do more, not only less?**
   - **Today:** a preview may only add constructs that narrow: delete the construct and you get a program the previous edition accepts (`spec/13 §7`). `for ever`, `within`, `tasks`, `std.collections` and `Matrix` all let a program do things it could not do before, so none of them could land while `0.4` is a preview, and `0.4` could not become stable without them having been tried.
   - **This record:** yes. A package that writes `edition = "0.4"` gets every construct of edition `0.4` that has landed, including those that widen what a program can do. In exchange, edition `0.4` makes no stability promise until it is declared stable (rule 3).
   - **Cost to the programmer:** a package on `0.4` may need edits after an upgrade of `una`, and should rerun `una check` and `una test` after each one. A package that stays on `0.3` or earlier sees none of it.
   - **Recommendation:** accept.
   - **Decided:** accepted.

   ```toml
   [package]
   name = "ticker"
   version = "0.1.0"
   edition = "0.4"     # opts in to for ever, within, tasks... and to their changes
   ```

2. **When a preview release changes a construct, must it fail loudly?**
   - **This record:** yes. A form that a release accepted never silently means something else in a later release. A construct whose meaning changes gets a new form, and the old form becomes **U7007**, "this form of edition `0.4` changed in release X", with the new form as its repair (rules 4 and 5). Every change is listed under a fixed heading of the changelog, `una --version` names the editions that are previews, and the build report lists the preview constructs a package uses (rules 6 to 8).
   - **The one exception you should know about:** an **unsettled point**, a detail of a construct that its record marks, when the construct first lands, as still open, with its reason, such as the order in which the numeric kernels of ADR 0120 add their numbers. Such a detail may change without a new spelling. It cannot be added to a construct later, the changelog lists each change, and the report shows which of a package's constructs have one (rule 5).
   - **How it is checked:** a release gate compares the results of every `0.4` test case with the previous release, and fails on any change that is not one of the allowed kinds, and on a test case deleted or renamed without a reason (rule 12). Until the gate exists, review checks the same rule. When to build it:
     - **(a) Recommended:** at the first release that changes a construct of `0.4`. That is when the promise is first used, and a promise that no machine checks is only as good as the reviewer who reads that release.
     - **(b)** Only before `0.4` leaves preview, as the project's direction to build new gates late would suggest. Until then, each change rests on review.
   - **Cost to the programmer:** none beyond the edit. The cost falls on the project: a change of meaning must also change the spelling, and keep recognizing the old one.
   - **Recommendation:** accept, and build the gate by option (a).
   - **Decided:** accepted, by option (a): the gate is built at the first release that changes a construct of `0.4`.

   ```una
   -- release N accepted:            release N+1 says:
   for ever within 5 * second         -- U7007: `for ever within` changed in 0.4.x;
     serve(@net)                      --   write `for ever, each pass within 5 * second`
   ```

   (The changed spelling is invented for the example; no such change is planned.)

3. **When does edition `0.4` stop being a preview?**
   - **This record:** by a short record of its own, at a minor release, `v0.4.0` (rule 15). Before that:
     - every accepted record that targets `0.4` has landed, or moved to a later edition;
     - every construct has its tests and documents, and every unsettled point is settled;
     - no soundness or security issue is open against a construct of `0.4`;
     - the release gate of Question 2 and the clash checks of rule 11 exist and pass;
     - the formal model's coverage of `0.4` is stated: what it re-checks, and what it does not model;
     - a quiet period has passed: at least four weeks and one release after the last construct landed, with no change to `0.4`.

     Edition `0.3` becomes stable no later than `0.4`. From then on `0.4` has the promise of `0.1` and `0.2`.
   - **Cost to the programmer:** none. A package on `0.4` keeps its `edition` line; its promise grows.
   - **Recommendation:** accept.
   - **Decided:** accepted.

   ```text
   $ una --version          (a release while 0.4 is a preview)
   una 0.3.21
   editions: 0.1, 0.2, 0.3, 0.4
   preview: 0.3, 0.4
   z3: 5.1.0
   ```

4. **Does edition `0.3` get the same terms?**
   - **This record:** no. Packages chose `0.3` under a narrower promise: it only gets stricter for programs that edition `0.2` can express, with the exceptions that ADRs 0103, 0104 and 0114 state. Widening that promise afterwards would break it. New constructs that cannot be erased go to edition `0.4`.
   - **Cost to the programmer:** a team that wants `for ever` or `Matrix` moves its package to `0.4`, one line.
   - **Recommendation:** accept.
   - **Decided:** accepted.

   ```toml
   edition = "0.3"     # stricter, with the stated exceptions of ADRs 0103, 0104 and 0114
   edition = "0.4"     # everything of 0.3, plus constructs that may still change
   ```

5. **When a construct asks you to trust more code, who must agree to it?**
   - **The words.** The *trusted computing base* is the code that the safety guarantees depend on but that the checker does not check: the compiler, the runtime, the solver, and any code a package marks as trusted.
   - **This record:**
     - **Trust a package adds itself** (a trusted module, or a new kind of grant that the report lists per package) needs a second, explicit opt-in in that package, beyond `edition = "0.4"`, which the construct's record names.
     - **Trust the toolchain adds for everyone** (a new part of the runtime, such as the coroutine switch of ADR 0117) and a new entry of the report's `out_of_model` that comes with a construct (such as "uncatchable termination" for `for ever`) need no opt-in, since no package can choose them. They need the record, a security review, and a listing in the record's Security part and in the edition ledger.
     - No general list of feature switches, such as `preview = ["tasks"]`, is added.
     - **No foreign code.** Una admits no foreign code in any form, in any edition: no construct may let a package link or call code written in another language, so no opt-in can admit it (rule 17).
   - **Cost to the programmer:** one more line, only for constructs that let the package trust code of its own choosing.
   - **Recommendation:** accept.
   - **Decided:** accepted, for trusted modules and for grants a package chooses: each needs the second opt-in that its record names.

   ```toml
   [package]
   edition = "0.4"     # every construct of 0.4; the package's trusted base does not grow
   # A construct through which this package trusts code of its own choosing, such as a
   # trusted module, also needs the second opt-in that the construct's record names.
   ```

## Glossary

- **Edition.** A version of the language that a package names in `una.toml` (`[package] edition`). One `una` serves several editions at once (`spec/13 §5`).
- **Release.** A version of the `una` program, such as `v0.3.15`.
- **Stable edition.** An edition with the stability promise: a later release does not change what it accepts or rejects, except to fix a soundness or security error (`spec/13 §7`). Today `0.1` and `0.2`.
- **Preview edition.** An edition still being completed. Today `0.3`, and `0.4` once its first construct lands.
- **Cumulative.** Each edition has every construct, keyword and rule of the one before it, unless a rule says otherwise (ADR 0056, amendment of 2026-10-13).
- **Narrowing construct.** One that only removes behaviors: erasing it gives a program the previous edition accepts, with no fewer allowed behaviors (ADR 0056, second amendment of 2026-10-13). A condition on an operation is one.
- **Widening construct.** One that lets a program do something the previous edition cannot express, such as `for ever` or `F64`.
- **Clash.** A program of the previous edition whose result changes when its `edition` line is raised: a name that became a keyword or a built-in, a closed kind that gained a variant, a stricter rule.
- **Edition test.** The test that checks every edition `0.2` package of the conformance suite in edition `0.3`, and requires byte-identical output except for the listed differences (`crates/una-cli/tests/editions.rs`).
- **Edition ledger.** The one table of the compiler that lists each construct of a preview edition, with its record, its words, its clashes, its trust and the releases that landed and changed it (rule 9).
- **Unsettled point.** A part of a construct that the record landing it declares may still change in meaning while the edition is a preview (rule 5).
- **Trusted computing base.** The code that the safety guarantees depend on but that the checker does not check (the report's `tcb`, `spec/13 §6`).
- **Soundness error.** A bug through which the checks accept a program they should reject.

## Context

**The preview rule today.** `spec/13 §7` states, for edition `0.3`:
- it is cumulative over `0.2`, and a preview until every rule that targets it has landed;
- a later `0.3.x` release may add obligations or errors to an edition `0.3` package: it only gets stricter, for the programs that edition `0.2` can express;
- a later `0.3.x` release may also accept a construct that earlier releases reject as a syntax error, if it only narrows: "erasing it gives a program that edition `0.2` accepts, with no fewer allowed behaviors";
- editions `0.1` and `0.2` keep their stability promise, except to fix a soundness or security error;
- two specific allowances follow: the unit methods of ADR 0104, and the refined type arguments of ADR 0103, which make edition `0.3` accept programs that edition `0.2` rejects.

These rules come from ADR 0056's amendments of 2026-10-13 (both). The edition test enforces them: it lists each edition `0.2` package whose result differs in edition `0.3` as `Syntax` (with its erasure twin), `Stricter` or `Looser`, and a listed package that does not differ fails the test (`crates/una-cli/tests/editions.rs`). It compares two editions within one release; nothing compares a package's result across releases.

**The exceptions so far.**
- ADR 0103 rule 9 lets edition `0.3` accept refined type arguments, on facts that obligations prove (the `Looser` difference of the edition test).
- ADR 0114 rule 15, appended to ADR 0056 on 2026-10-16, exempts `F64` and the constructs only it admits from the erasure clause. A program that edition `0.2` can express gains from them only U1002, for a declaration named `F64` and a package function `to_f64` on integers. `spec/13 §7` does not carry that sentence yet: it lands with the implementation (ADR 0114 Appendix A).

**Edition `0.4`.**
- ADR 0116 rule 0 adds `"0.4"`, "cumulative over `"0.3"` … and a preview until the rules that target it have landed, under the terms of `"0.3"`". ADR 0117 targets the same edition through that rule.
- Their constructs widen: `for until`, `for ever`, `diverges`, `stop`, `skip`, `on stop`, the `[drive]` table, `std.drive` and `HttpError.Interrupted` (ADR 0116); coroutine tasks, `within`, `first of`, `tasks` and `start`, limits on grants, `limiter`, `std.limit` and `HttpError.RateLimited` (ADR 0117). None of them erases to a program of edition `0.3`.
- They also grow what is trusted for everyone: ADR 0116 adds "uncatchable termination" to the report's `out_of_model`, and ADR 0117 adds the crate `una-sys` to the trusted base and rebuilds the native runtime around coroutines, for every edition (ADR 0117, Security and Consequences).
- The records on collections (ADR 0119) and numeric types (ADR 0120) are in the same position, and each asked the project the same question.

So the terms that ADR 0116 cites cannot admit the constructs it adds. Either every such record needs a carve-out of its own, as ADR 0114 had, or edition `0.4` needs terms that fit what it is for.

**Edition state.** Edition `0.3` is still a preview: ADR 0094's implementation (#566) and `F64` (ADR 0114, #819) have not landed (`CHANGELOG.md`, `changelog.d/819-float.md`). The compiler knows editions `0.1` to `0.3` (`crates/una-project/src/manifest.rs`, `Edition::ALL`), and an unknown edition in `una.toml` is U7002 (`spec/13 §7`).

**What reads the list of editions.**
- `una --version` prints `una <version>`, `editions: 0.1, 0.2, 0.3` and `z3: <version>`; with `--json`, `{"schema":"una.version/1","version":…,"editions":[…],"z3":…}` (`spec/13 §5`). A test pins both (`crates/una-cli/tests/cli.rs`, `version_names_the_editions_and_the_pinned_z3`).
- `scripts/site-bundle.sh` takes the `editions:` line, and only that line, into the docs bundle's manifest.
- The website takes the preview editions from `spec/13 §7` (website repository, `scripts/preview/editions.mjs`). For each preview it requires the marker "**Edition `X` is a preview**" and the sentence "Edition `X` has every construct, keyword and rule of edition `W`", for `W` the edition before it, and it fails its build when `spec/13 §7` names as a preview an edition that the release does not serve. Its badge reads "0.3 preview" for every preview alike.
- The build report, `una.report/2`, names no edition (`spec/13 §6`). The Core export does (`spec/13 §8`, `"edition"`).
- The guide's page `docs/guide/configure/package/edition.md` and the Learn chapter on editions (una-by-example, `14-editions/README.md`, sections 14.1 and 14.3) describe a preview as "it only gets stricter".

**Name resolution of variants.** A variant whose kind the context gives (a `when` pattern, `is`, `fail`, a handling clause) resolves by that kind; elsewhere, the variants of an imported kind are in a scope before the prelude, and a name that stays ambiguous within one scope is U1001 (`spec/10 §11` rules 2 and 3). `HttpError` is declared in the prelude (`spec/24 §1`).

**The specification's chapters.** ADR 0114 and ADR 0116 both name `spec/26` for their rules, and ADR 0117 names `spec/27`. The records on collections, numeric types, Itá and HTTP servers each add a chapter. `docs/spec` ends at `spec/25` on main.

## Decision

In one paragraph: edition `0.4`, while it is a preview, may gain constructs that widen what a program can do, behind the package's own `edition = "0.4"`, with no stability promise (rules 1 to 3). It never changes silently: a changed form fails with U7007 and a repair, unsettled points are declared when a construct lands and shown in the report, and every change is announced in the changelog, by `una --version` and in the report (rules 4 to 8). Every record that adds to `0.4` lists its words, clashes and trust in one ledger, and the clash checks are generated from the compiler's own tables (rules 9 to 11). A release gate compares `0.4` results across releases before `0.4` may leave preview (rule 12). Edition `0.3` keeps its terms (rule 13). `0.4` leaves preview by a record of its own at a minor release, and then has the promise of `0.1` and `0.2` (rules 14 to 16). Rules 17 to 19 state what does not change, and rule 20 numbers the new chapters.

### A. The terms of a preview

1. **What a preview edition is.** Edition `0.4` is a preview from the release that first accepts `edition = "0.4"` until a release declares it stable (rule 15). It is cumulative over `0.3`, as ADR 0116 rule 0 states.

2. **The opt-in.**
   - **The edition is the switch.** A construct of edition `0.4` exists only in a package whose `una.toml` says `edition = "0.4"`. In every earlier edition, its form keeps the diagnostic it has today: U0101 for a syntax error, U0103 for a word that `spec/01 §5` already reserves (as `simd`, ADR 0120), U7002 for an unknown table, U1001 for an unknown module; and a new built-in name stays an ordinary name there. A single file keeps edition `0.1`.
   - **(a) Trust a package adds itself: a second opt-in.** A construct through which a package adds to its own trusted computing base, such as a trusted module or a new kind of grant that the report's `tcb` names per package, MUST also need an explicit opt-in that its record names, in `una.toml` or in a grant. Code written in another language is never such a construct: Una admits no foreign code (rule 17).
   - **(b) Trust the toolchain adds for everyone: a record and a review, no opt-in.** Growth of the runtime or the toolchain that every package runs on, such as `una-sys` (ADR 0117) or the numeric kernels of ADR 0120 and their dispatch by processor, and a new `out_of_model` entry that comes with a construct, such as "uncatchable termination" (ADR 0116), need no opt-in, since no package can choose them. Each MUST have:
     - the record that introduces it;
     - a security review before it merges;
     - a listing in the record's Security part and in the ledger's `trust` field (rule 9);
     - for an `out_of_model` entry, a statement of whether the report shows it only for packages that use the construct, or for every package.
   - **No feature switches.** There is no general list of preview features in `una.toml`. A package on `0.4` sees every construct of `0.4` that its release has.

3. **No stability promise while a preview.** While edition `0.4` is a preview, a later release MAY, in a package of edition `0.4`:
   - accept a construct that earlier releases reject, whether it narrows or widens what a program can do;
   - add obligations, errors and keywords;
   - change or remove a construct of edition `0.4`: its syntax, its typing, its meaning, its standard modules and names, its `una.toml` tables, its codes, and its keys in the report and the Core export;
   all under rules 4 to 11. A change of a key of the report or the Core export is listed under the changelog heading of rule 6, so that tools that read `una.report/2` or `una.core/1` are never broken silently. Rules 17 and 18 bind every release.

4. **No silent change of meaning.** A program that one release accepts in edition `0.4` MUST, in a later release, give the same result, or be rejected with a diagnostic. The result is the status, output and trace of each run, as `spec/16 §5` compares runs, and the status of each obligation. The only exceptions:
   - a fix of a soundness or security error, as for every edition;
   - a change at an unsettled point (rule 5).

5. **Changes of meaning.**
   - **A new form.** A construct whose meaning changes gets a new form. The old form is then **U7007** (rule 19), with the new form as its first repair, for as long as edition `0.4` is served. A construct that is removed is U7007 too, with a repair that says what replaces it.
   - **Unsettled points.** The record that lands a construct MAY declare, in its ledger entry, a part of the construct whose meaning may still change while the edition is a preview, with its reason: for example the accumulation order of ADR 0120's kernels (its lane width L and block KB), which that record declares unsettled during the preview. A point can be declared unsettled only there, before any release accepts the construct; a later record cannot make a released, settled point unsettled. A change at an unsettled point needs no new form; the changelog lists it (rule 6), and the record that makes it states the old and new meaning. Security properties are never unsettled points (rule 17).
   - **Stricter is always allowed.** A change that only adds errors or obligations needs no new form: the program is rejected or gains an obligation, which is loud.

6. **The changelog.** Each release that changes edition `0.4` has, in its `CHANGELOG.md` section, an entry under the heading "**Edition `0.4` preview: changes that may need edits**", listing for each change: the construct, the record, what a package accepted before and what it is now, the diagnostic it gives (U7007 or another), its kind (U7007, unsettled point, soundness or security fix, stricter, or a key of the report or the Core export), and the migration. The release's summary paragraph says "changes the edition `0.4` preview" when such an entry exists. A release that only adds to `0.4` lists its additions as usual.

7. **`una --version`.** The text gains a line, after `editions:`, `preview: ` and the preview editions, comma separated, when there is at least one: `preview: 0.3, 0.4`. With `--json`, `una.version/1` gains `"preview": [...]` after `"editions"`, always present, `[]` when there is none; the key is additive and the schema stays `una.version/1`. The `editions:` line keeps its form and lists every edition served, preview or stable.

8. **The report.** `una.report/2` gains an `edition` section after `package`:

   ```json
   "edition": { "name": "0.4", "preview": true,
                "preview_constructs": [
                  { "construct": "for ever", "record": "ADR 0116", "sites": 3, "unsettled": [] },
                  { "construct": "sum", "record": "ADR 0120", "sites": 1,
                    "unsettled": ["the accumulation order of 0120's kernels (L, KB)"] } ] }
   ```

   - `name` is the package's edition, and `preview` whether it is a preview in this release.
   - `preview_constructs` lists, for a package of edition `0.4`, each construct of the ledger (rule 9) the package uses, with its record, the number of sites and the names of its unsettled points, ordered by record, then construct; `[]` otherwise. Its human form is one line, `edition: 0.4 (preview): for ever (ADR 0116, 3 sites), sum (ADR 0120, 1 site; unsettled: the accumulation order of 0120's kernels (L, KB))`.
   - The key is additive, and the schema stays `una.report/2`. A reviewer sees in one place which parts of a package an upgrade may touch, and which of them may change without a new form.

### B. What each record that adds to edition `0.4` must do

9. **The ledger.** The compiler holds one table, the edition ledger, with one entry per construct of edition `0.4`:
   - its name and its record;
   - its reserved and contextual keywords;
   - the names it adds to the prelude, the built-in methods (by receiver type) and the standard modules, and the variants it adds to existing kinds;
   - its `una.toml` tables and its codes;
   - **`clashes`**: each clash with an id, such as `0116-within-name`, the kind of `0.3` program, and its diagnostic in `0.4`;
   - **`trust`**: what it adds under rule 2 (a) with its opt-in, or under rule 2 (b) with its review;
   - its unsettled points (rule 5);
   - the release that landed it and each release that changed it.

   The report's `preview_constructs` (rule 8), the guide's list of the constructs of `0.4`, and the checks of rule 11 read it. A test checks that the guide's list and the ledger agree.

10. **The edition section of a record.** Each record that targets edition `0.4` has a section "Edition `0.4`" that states, for its constructs:
    - **Words.** Each reserved keyword, and each contextual keyword with the positions where it is one. A contextual keyword is preferred; a reserved keyword states why a contextual one does not serve, as ADR 0116 rule 0 does for `diverges` and `within`.
    - **Clashes.** Each kind of edition `0.3` program whose result changes in edition `0.4`, with the diagnostic it gets: a name that became a reserved keyword (U0101), a declaration or function that a new built-in resolves ahead of (U1002, as for `to_f64` in ADR 0114 rule 4.1), a closed kind that gained a variant (U2007 for an exhaustive `when` with no other arm), a rule that became stricter, or "none". A change of message only is listed as such.
    - **Migration.** For each clash, the edit that resolves it, and whether its diagnostic carries it as a mechanical repair that `una fix` applies (`spec/13 §5`).
    - **Unsettled points**, if any (rule 5), and its **trust** under rule 2.
    - **Security.** Its information-flow rules, and what it adds to `tcb` and `out_of_model`.

11. **The edition test of `0.4`, and clash checks from the compiler's tables.**
    - **The pair.** The edition test gains a second pair, `0.3` to `0.4`, with the rules of the first (`crates/una-cli/tests/editions.rs`): every edition `0.3` package of the conformance suite checks byte-identically in edition `0.4`, except those it lists. Its kinds of difference are `New` (a package that writes a construct of `0.4`, with its one diagnostic in `0.3` and its result in `0.4`; unlike `Syntax`, it has no erasure twin, since a widening construct has none), `Stricter` and `Looser`, as today. Each `Stricter` entry cites the ledger id of its clash.
    - **Generated clash checks.** So that a clash a record forgot fails mechanically, a test derives the differences between editions `0.3` and `0.4` from the compiler's own tables, not from the records:
      - **Keywords.** The difference of the lexer's reserved and contextual keyword tables. A keyword missing from the ledger fails the test. For each new reserved keyword, it generates a `0.3` package that uses it as a name, and expects U0101 in `0.4`.
      - **Prelude and built-in methods.** The difference of the prelude's names and of the built-in method names by receiver type (the tables `una_check::builtin_methods` lists). A name missing from the ledger fails the test. For each new name, it generates a package function or declaration of that name on that receiver type, and expects U1002, or the winner its ledger clash states.
      - **Closed kinds.** The variants of every prelude kind in both editions. For each kind that gains a variant, it generates an exhaustive `when` with no other arm and expects U2007, and the same `when` with an `else` arm and expects the `0.3` result.
      - **Coverage.** Every ledger entry has at least one `New` package, and every ledger clash a `Stricter` entry or a generated case; one without fails the test.
    - **Changes.** Each U7007 of rule 5 has a conformance case with the old form, its diagnostic and its repair.
    - **When.** The pair with `New`, `Stricter` and `Looser` lands with the first construct of `0.4`, as the edition test landed with `0.3`. The generated clash checks and the coverage check are built before `0.4` leaves preview, and are a condition of rule 15; until then, the records' edition sections and review carry the clash lists. This follows the project's direction to defer new gates and test machinery until they are needed for a promise.

12. **The preview gate.** A release gate, `preview`, in `docs/release-gates.md`:
    - **Diff.** It diffs the expected outputs of the edition `0.4` conformance cases between the previous release's tag and the candidate: the status, output and trace of each run, and the status of each obligation. The old outputs are in git at the tag.
    - **Justify.** Every changed case MUST be named by one of: a U7007 entry of the ledger; a declared unsettled point; an entry under the heading of rule 6 marked as a soundness or security fix; or an added error or obligation (rule 5, "stricter").
    - **Deleted or renamed cases.** A case of the previous tag that the candidate no longer has, under its path, is a change too. A case renamed with byte-identical files and outputs passes; any other deletion or rename needs one of the justifications above, or, for a case of a construct that the release removed, its U7007 entry.
    - **Fail.** A change that none of these names fails the gate.
    - **Cost.** O(c) for c cases, with a hash map from case to justification.
    - **When.** The gate is built for the first release that changes a construct of `0.4` (Question 2, option (a)), and in any case before `0.4` leaves preview: it is a condition of rule 15, and must be green across the quiet period. Until it exists, a pull request that changes the expected output of a `0.4` conformance case names, in its description, the justification the gate would require, and review checks it.

### C. Edition `0.3`

13. **Edition `0.3` keeps its terms.** The preview rule of `spec/13 §7` for edition `0.3` stays as it is: it only gets stricter for the programs edition `0.2` can express, its new constructs narrow, and the allowances of ADRs 0103, 0104 and 0114 stay the only exceptions. A construct that does not erase targets edition `0.4` or later, not `0.3`. ADR 0114's carve-out is not a precedent for edition `0.3` (Appendix A).

### D. Leaving the preview

14. **Order.** Edition `0.3` becomes stable no later than edition `0.4`: the release that makes `0.4` stable makes `0.3` stable too, if it is not already. A stable `0.4` is cumulative over `0.3`, so `0.4` cannot be frozen while `0.3` may still change.

15. **The stabilizing record.** Edition `0.4` becomes stable by a record of its own, accepted by the project, that names the release. That release is a minor release, `v0.4.0`. The record checks, and the release gates confirm:
    - every accepted record that targets `0.4` has landed, or a dated amendment of it moves it to a later edition;
    - every ledger entry has its conformance cases, its edition-test entries, its guide page with its Edition part, its `explain` texts, and its section in the Learn chapter on editions;
    - every unsettled point is settled by a stated meaning; a settlement that differs from the current meaning is a change under rules 5 and 6;
    - no soundness or security issue is open against a construct of `0.4`;
    - the generated clash checks and the coverage check of rule 11 exist and pass;
    - the preview gate of rule 12 exists, and was green for every release of the quiet period;
    - the coverage of `0.4` by the formal model is stated: what its re-check covers, and what is listed as not modeled, as ADR 0114 did for `F64`;
    - the security hunt has a family that runs in edition `0.4` (`docs/release-gates.md`, "The hunt gate");
    - **a quiet period:** at least four weeks, and at least one release, from the release that landed the last construct of `0.4`, with no entry under the heading of rule 6.

16. **What is frozen then.** From the release of rule 15, edition `0.4` has the promise of editions `0.1` and `0.2`: a later release MUST NOT change what it accepts or rejects, nor the result of a program it accepts, except to fix a soundness or security error. This covers its syntax, its keywords, its rules, the headers of its prelude and standard modules, its `una.toml` tables, the meaning of its constructs, and the keys of the report and the Core export it added. Messages, `explain` texts, repairs, speed and the tools keep improving, as for every edition. U7007 stays, for the forms the preview changed. Its ledger becomes a record of history, and `preview` no longer lists it.

### E. What does not change

17. **Security binds every release.** No release, preview or not, may:
    - weaken noninterference (T6, `spec/10 §10`), authority checking (`spec/05`) or memory safety for a program that uses no construct of edition `0.4`;
    - give a construct of `0.4` an exception to them that its record does not state;
    - add to the trusted base or to `out_of_model` except as rule 2 states;
    - make any part of these properties an unsettled point;
    - admit foreign code in any form: no foreign function interface, no library or archive of code written in another language, and no trusted foreign code. The toolchain's and the runtime's own Rust is not foreign code.

    Growth of the runtime's trusted base under rule 2 (b), such as `una-sys`, is not a weakening when it has passed its security review and the memory-safety gates (`docs/release-gates.md`, `sanitize`). A fix of a soundness or security error MAY change any edition, as today, and is announced as one.

18. **Earlier editions.** Editions `0.1` and `0.2` keep their stability promise; edition `0.3` keeps its terms (rule 13). The cumulative rule binds edition `0.4`: a program that edition `0.3` can express, checked in edition `0.4`, keeps its result or gains errors and obligations, except the clashes that the ledger lists (rule 9) and the edition test pins (rule 11). The benchmark baseline, pinned to edition `0.1`, does not move.

19. **The code U7007.** "This form of edition `0.4` changed in release `<version>`: `<what changed>`". Domain `project`: it is positioned at a form of the source, but what it reports is a property of the package's edition and of the release that serves it, as U7002 reports an edition that a release does not serve. Its first repair writes the new form, as an edit when the rewrite is local and mechanical, so that `una fix` applies it under its usual conditions (`spec/13 §5`), and otherwise as a description. It is an error, exit code 1.

### F. The chapters of the specification

20. **The numbers of the new chapters.** Each record that adds a chapter to `docs/spec` takes its number from this table, which later records extend by a dated note on this record. This record adds no chapter: it amends `spec/13 §7`.

    | Chapter | Record | Edition | Contents |
    | --- | --- | --- | --- |
    | `spec/26` | ADR 0116 | `0.4` | Long-running programs |
    | `spec/27` | ADR 0117 | `0.4` | Tasks and time |
    | `spec/28` | ADR 0119 | `0.4` | Collections |
    | `spec/29` | ADR 0120 | `0.3` (`F64`); `0.4` (the rest) | The numeric foundation, with the rules of `F64` of ADR 0114 |
    | `spec/30` | ADR 0122 | as its record states | Itá, packages and the registry |
    | `spec/31` | ADR 0123 | as its record states | HTTP servers |

    - ADR 0116's header names `spec/26` and ADR 0117's `spec/27`, as the table does.
    - ADR 0114's header also names `spec/26`, which ADR 0116 holds. The rules of `F64` live in `spec/29`, beside `F32` and `Complex` of ADR 0120, since those share its NaN, comparison and environment rules; a dated note on ADR 0114 records it (Appendix A). If `F64` lands before ADR 0120, it opens `spec/29` with its own rules.
    - A later record that adds a chapter takes the next free number, `spec/32` and on, and extends the table by a dated note.

### Examples

A package of edition `0.3` with a function named `within`, raised to `0.4`:

```una
pub within(limit: Int, n: Int) -> Bool = n <= limit
-- edition 0.3: accepted
-- edition 0.4: U0101, `within` is a reserved keyword (ADR 0116 rule 0); rename it, such as `is_within`
```

The report of a package of edition `0.4`, human form:

```text
edition: 0.4 (preview): for ever (ADR 0116, 1 site), tasks (ADR 0117, 1 site)
```

A changelog entry of a release that changes the preview:

```markdown
### Edition `0.4` preview: changes that may need edits
- **`skip when` is now `skip if`** (ADR 0116, amendment of <date>; kind: U7007). `skip when c`
  is U7007, with the edit to `skip if c`; `una fix` applies it.
```

(The change is invented for the example.)

## Verification

No obligation kind is added or changed. Rule 4's comparison includes the status of each obligation, so a preview release that changes a construct's encoding to the solver keeps the results of programs that do not use the construct: the cumulative rule (rule 18) covers their queries, as ADR 0114 rule 15 states for `F64` ("the queries of a program without `F64` are unchanged"). The lock's `encoding_version` (`spec/12 §9`) changes only by its own rules.

## Information flow

These terms add no flow rule. They bound how a preview may change the rules of others (rule 17):
- a program that uses no construct of `0.4` keeps every guarantee of `spec/07` in edition `0.4`, as in `0.3`;
- each construct of `0.4` brings its information-flow rules in its own record, as ADR 0116 (IF1 to IF8) and ADR 0117 do, and its channels outside the model in `out_of_model`, under rule 2 (b);
- a change of a construct's flow rules in a later preview release is announced as rule 6 states, and may only make them stricter unless it fixes a soundness error, since flow is never an unsettled point.

`preview_constructs` names constructs, counts sites and names unsettled points, all public facts of the source: it adds no channel.

## Security

- **Explicit consent where a package chooses.** Only a package that names `edition = "0.4"` can be changed by a preview release, and only a package that also writes the second opt-in can add to what it trusts (rule 2 (a)). A tool cannot opt a package in.
- **Review where no package chooses.** Trust the toolchain adds for every package, and new `out_of_model` entries, need a record, a security review and a listing (rule 2 (b)), and cannot weaken programs that do not use `0.4` beyond what the review and the memory-safety gates accept (rule 17).
- **Loud failure.** A change of meaning is rejected, never applied silently (rules 4 and 5), so a program that was reviewed under one release cannot do something else under the next without a diagnostic. This is the property that matters most for programs written and maintained by agents, which upgrade often and read diagnostics, not changelogs. The preview gate (rule 12) makes it mechanical before `0.4` leaves preview.
- **Unsettled points are fixed in advance and visible.** They are declared only when a construct lands, never added later, and the report names them (rules 5 and 8).
- **Security is settled from the start.** Rule 17 forbids a preview from weakening the model, even for its own constructs, without a record.
- **Visibility.** `una --version`, the report and the validation summary of each release say which editions are previews, so an auditor knows which promise applies.
- **Not chosen for security:** silently changing a preview construct's meaning (alternative B), and a general list of feature switches, which multiplies the dialects that reviews and the security hunt must cover (alternative D).

## Costs

- **The edition check:** O(1) per construct, at parse or name resolution, as today for `0.2` and `0.3`.
- **U7007:** the old forms are entries of a table keyed by token or name, consulted only where the parser or the resolver would otherwise report an error: O(1) per error, nothing on accepted code.
- **`preview_constructs`:** one counter per ledger entry, incremented where the checker already resolves the construct: O(n) in the size of the package, O(k) memory for k entries; sorting k entries, O(k log k), with k a few dozen.
- **`una --version`:** O(e) for e editions.
- **The edition test:** two checks of each conformance package per pair of editions, linear in the size of the suite. A scaling check: the test's run time grows linearly as the suite grows, measured at the suite's current size and at twice it by duplicating the packages in a temporary directory.
- **The generated clash checks:** set differences of the keyword, prelude, method and variant tables, O(t) for t table entries with hash sets, plus one generated package per difference.
- **The preview gate:** O(c) for c cases, one comparison per case and one lookup of its justification in a hash map.

## Diagnostics

One new code, which joins `spec/13 §4` with its implementation. U7007 is free on main at this writing, and the other records in review cite it as this record's.

| Code | Domain | Meaning | Repairs, ranked |
| --- | --- | --- | --- |
| U7007 | project | A form of a preview edition that a release changed or removed | write the new form (an edit when mechanical); a description of what replaces a removed construct |

Existing codes cover the rest: U0101 (a construct of `0.4`, or a new reserved keyword used as a name, in an earlier edition), U0103 (a word already reserved by `spec/01 §5`), U1001 (a module of `0.4` imported in an earlier edition), U1002 (a declaration that a new built-in resolves ahead of), U2007 (a closed kind with a new variant), U7002 (an edition the release does not serve, or a table of `0.4` in an earlier edition).

## Consequences

- **Edition `0.4` can hold what ADRs 0116, 0117, 0119 and 0120 add**, without one carve-out per record.
- **A package on `0.4` may need edits after an upgrade**, always with a diagnostic and a changelog entry, and the report says which of its parts are exposed.
- **The project carries a cost for each change:** a new form, a U7007 entry kept for the life of the edition, a changelog entry, and conformance cases; and, before `0.4` leaves preview, the preview gate and the generated clash checks.
- **Edition `0.3` stays narrow.** New widening work goes to `0.4`.
- **The guide and Learn** describe two kinds of preview: `0.3`, which only gets stricter, and `0.4`, which may change.
- **One numbering** for the new chapters of the specification, with no two records claiming one number.
- **Future work.** Una has no dependencies between packages today. When it gains them (ADR 0122), a rule is needed for a package of one edition that depends on a package of a preview edition; "a tool cannot opt a package in" (Security) then extends to dependencies.

## Alternatives considered

- **A. A carve-out per record, as ADR 0114 did.** Not chosen: the carve-out of ADR 0114 is a paragraph of exceptions, and `0.4` would need one for every record, each restating the same thing. It also keeps a promise ("only narrows") that is no longer true of the edition as a whole, which misleads readers.
- **B. No stability promise and no further rule (the Rust nightly model).** Not chosen: a preview release could then change what an accepted program does without a diagnostic. Rule 4 costs the project a new spelling per change of meaning, and buys the guarantee that a program never silently changes.
- **C. Land widening constructs only when `0.4` becomes stable.** Not chosen: a construct would be frozen before anyone used it. The point of a preview is to try constructs on real programs while they can still change.
- **D. Per-feature switches in `una.toml`, such as `preview = ["tasks", "collections"]`.** Not chosen as a general mechanism: n switches make 2^n dialects for the checker, the edition test and the security hunt, and agents must learn which switch each construct needs. Kept only for trust a package adds itself (rule 2 (a)), where consent per construct is worth it.
- **E. A different spelling while a preview, `edition = "0.4-preview"`.** It makes the status visible in `una.toml`, but every package must change it again at stabilization, a release must accept both spellings, and edition `0.3` set the precedent of one name. The status is shown by `una --version` and the report instead (rules 7 and 8).
- **F. Give edition `0.3` the same terms.** Not chosen (rule 13, Question 4): packages chose `0.3` under a narrower promise.
- **G. A warning on every check of a preview package.** Not chosen: a warning on every run is ignored. The report and `una --version` say it once, where review looks.
- **H. A second opt-in for every growth of trust,** runtime growth included. Not chosen: `una-sys` and the coroutine runtime serve packages of every edition, so no package could opt in or out of them; an opt-in would be a label with no effect. A record, a review and a listing are what such growth can be held to (rule 2 (b)).
- **I. Build the preview gate and the generated clash checks now, before any construct of `0.4` lands.** Not chosen: the project defers new gates and test machinery until they are needed for a promise. The gate is needed from the first release that changes a construct of `0.4`, when rule 4 is first used (Question 2, option (a)); the generated clash checks before `0.4` is frozen.
- **J. Build the preview gate only before `0.4` leaves preview** (Question 2, option (b)). Not recommended: every change between the first one and the freeze would rest on review alone, which is the gap that rule 4 is meant to close.

## Implementation plan

1. **Specification.** `spec/13 §7`: the terms of edition `0.4` (rules 1 to 5, 13 to 18). It carries, verbatim, the two sentences the website reads: "**Edition `0.4` is a preview**" and "Edition `"0.4"` has every construct, keyword and rule of edition `"0.3"`." The marker of `0.3` stays until `0.3` becomes stable. `§5`: the `preview` line and key. `§6`: the `edition` section. `§4`: U7007.
   - **Ordering.** The website fails its build when `spec/13 §7` names as a preview an edition the release does not serve. So the `0.4` marker lands in the same change as `Edition::ALL` gains `0.4` (ADR 0116's implementation), never in a documentation change before it.
2. **The compiler.** The edition `0.4` value, which ADR 0116 rule 0 adds; the ledger (rule 9) beside `Edition` in `crates/una-project`; `preview` in `una --version`; the report's `edition`; the U7007 table, empty until a change; the `explain` text of U7007.
3. **Tests, with the first construct of `0.4`.** The `0.3`-to-`0.4` pair of the edition test with `New`; the version test; the report's schema test; the guide's list against the ledger; a check that the `preview:` line of `una --version` and the markers of `spec/13 §7` name the same editions, so that the website and `una --version` cannot disagree.
4. **The preview gate** (rule 12), in `scripts/release-gates.sh` and `docs/release-gates.md`, for the first release that changes a construct of `0.4`. **Before `0.4` leaves preview:** the generated clash checks and the coverage check (rule 11).
5. **Release gates now.** The validation summary names the stable and preview editions of the release.
6. **Documentation.**
   - `docs/guide/configure/package/edition.md`: edition `0.4` and its terms, the forms, the edge cases, and U7007.
   - `docs/guide/una-for-agents.md`: a section "Migrating to edition 0.4", which gathers the migration notes of each record (rule 10).
   - The Learn chapter on editions (una-by-example, `14-editions`): the table of "Stable editions and previews" gains `0.4`, "Preview; constructs may change between releases, never silently"; section 14.3 explains the two kinds of preview; "When to choose" gains a row for `0.4`.
   - `scripts/site-bundle.sh` keeps reading only the `editions:` line, so the new `preview:` line changes nothing there.
   - **For the website's own change:** its badge reads "0.3 preview" and would read "0.4 preview" in the same words, though the two promises differ; the badge strings, and whether to read `preview` from the version document, are a change of the website, outside this record.
7. **Records.** The amendments and notes of Appendix A, appended on acceptance; ADRs 0119 and 0120 cite this record in place of their edition question and add their edition sections (rule 10), and the records of rule 20's table take their chapter numbers from it.

## Conformance

- **Editions:** a package of edition `0.4` before any construct lands checks as `0.3` does (byte-identical for the whole suite); `edition = "0.5"` is U7002.
- **`una --version`:** the text with `preview: 0.3, 0.4`, and the JSON with `"preview":["0.3","0.4"]` after `"editions"`; a release with no preview prints no `preview:` line and `"preview":[]`.
- **The report:** `edition` for a package of each edition; `preview_constructs` empty for `0.3`, and listing each construct with its site count and unsettled points, in order, for a `0.4` package that uses two constructs of two records; the human line.
- **The edition test:** a `New` entry for each ledger entry; a `Stricter` entry for each listed clash, citing its id, such as a function named `within` (U0101) and an exhaustive `when` over `HttpError` with no other arm (U2007); a listed package that does not differ fails it. Once built, the generated checks: a keyword, a built-in method and a variant added to the tables without a ledger entry each fail the test.
- **Clashes of ADRs 0116 and 0117** (Appendix A):
  - a raised `0.3` program whose `when` over `HttpError` has an `else` arm keeps its result in `0.4`;
  - a `0.3` binding named `ever` in `for ever in xs` keeps its meaning in `0.4`;
  - a `0.3` loop body with a statement made of the bare identifier `stop` or `skip` followed by a newline, if the grammar of `0.3` admits such a statement, keeps its result in `0.4`; if it does not, a case shows that the statement is rejected in `0.3`, so no `0.3` program can clash;
  - a module that imports `std.limit`, with `RateLimited` in a `when` over `HttpError` (resolved by the kind) and as a bare name elsewhere (resolved to `Limited.RateLimited`, the imported scope).
- **U7007:** once a change exists, the old form with its diagnostic, its repair, and `una fix` applying it; with `--json`, its repair's edit.
- **The preview gate,** once built: a changed case with no justification fails it; each of the four justifications passes it; a deleted case and a renamed case with changed files fail it without a justification, and a renamed case with identical files passes it.
- **Edition `0.3`:** unchanged: the existing `0.2`-to-`0.3` pairs pass as they are.

## Appendix A. Amendments and notes to accepted records, appended on acceptance

**ADR 0056, Amendment of 2026-10-19: the terms of edition `0.4` (ADR 0121).**
- Edition `0.4` is a preview from the release that first accepts it until a release declares it stable by a record of its own (ADR 0121 rules 14 and 15). While a preview, it MAY gain constructs that widen what a program may do, behind `edition = "0.4"`, with no stability promise; the erasure clause of the second amendment of 2026-10-13 does not apply to it.
- A program that a release accepts in edition `0.4` keeps its result in a later release or is rejected with a diagnostic, except for a fix of a soundness or security error and a change at an unsettled point declared when its construct landed (ADR 0121 rules 4 and 5).
- No release may weaken noninterference, authority checking or memory safety for a program that uses no construct of `0.4` (ADR 0121 rule 17).
- Edition `0.3` keeps the terms of the amendments of 2026-10-13 and 2026-10-16, and becomes stable no later than `0.4`. Editions `0.1` and `0.2` keep their promise, and the benchmark baseline pinned to edition `0.1` does not move.

**ADR 0116, Amendment of 2026-10-19: the terms of edition `0.4`.** Rule 0's "under the terms of `"0.3"`" reads "under the terms of ADR 0121". The constructs of this record widen what a program may do, which those terms allow. Its edition section, as ADR 0121 rule 10 asks:
- **Words.** Reserved keywords `diverges` and `within`, each U0101 as a name in edition `0.4`, with the repair of renaming it. Contextual keywords `until`, `ever`, `decreases`, `stop`, `skip` and `on`, in the positions rule 0 states; `for ever in xs` stays a loop over `xs` with a variable `ever`. The `stop` and `skip` cases are pinned by ADR 0121's conformance.
- **Variants.** `HttpError` gains `Interrupted` in edition `0.4` (rule 12 of this record). A `when` over `HttpError` that lists each of the eight variants of `spec/24 §1` with no other arm is U2007 in edition `0.4`, resolved by adding the arm. A `when` with an `else` arm absorbs the new variant with no diagnostic, and this is not a silent change: `Interrupted` arises only from a host stop of an interruptible operation in the body of a `for ever` (rule 14.1 of this record), a construct that no `0.3` program contains, so a raised `0.3` program never sees it.
- **Modules.** `std.time` and `std.drive` clash with no package module, since module paths under `std` are reserved (`spec/13 §7`).
- **Messages only.** The message of `while` and `loop` changes (rule 0); this is a change of message, which every release may make.
- **Unsettled points:** none.
- **Trust,** under ADR 0121 rule 2 (b): the entry "uncatchable termination" of `out_of_model` (rule 17 of this record), shown in the report of packages of the edition that has `for ever`, edition `0.4` and every later edition, since only those editions promise the cleanup that the entry qualifies. In the ordered list of `out_of_model` (`spec/13 §6`), it follows "termination and divergence", the entry it qualifies; the signal handlers, in the trusted crate `una-sys` (ADR 0058 amendment of 2026-10-18), shared with ADR 0117. No second opt-in.
- **Chapter.** Its rules live in `spec/26`, as its header states (ADR 0121 rule 20).

**ADR 0117, Amendment of 2026-10-19: the terms of edition `0.4`.** The target "edition `0.4` (ADR 0116 rule 0)" is under the terms of ADR 0121. Its edition section, as ADR 0121 rule 10 asks:
- **Words.** The contextual keywords `first`, `tasks` and `start`, in the positions rules 6 and 7 state; elsewhere they are identifiers, and the prelude's `first` stays a method.
- **Variants.** `HttpError` gains `RateLimited` in edition `0.4` (rule 8), with the U2007 clash and repair that ADR 0116's amendment states for `Interrupted`. A `when` with an `else` arm absorbs it with no diagnostic, and this is not a silent change: `RateLimited` arises only from a grant that carries limits, which only a package of edition `0.4` can write.
- **A second `RateLimited`.** `std.limit` declares `pub kind Limited = RateLimited` (rule 9), beside `HttpError.RateLimited`. In a `when` pattern, an `is` test, after `fail` or in a handling clause, the name resolves by the kind the context gives (`spec/10 §11` rule 3). Elsewhere, in a module that imports `Limited`, a bare `RateLimited` is `Limited.RateLimited`, since imported variants are in a scope before the prelude, where `HttpError` is declared (`spec/10 §11` rule 2). ADR 0121's conformance pins both.
- **Names and modules.** `limiter` is a name that only `std.limit` provides, with no clash in the prelude; `std.limit` and the additions to `std.time` clash with no package module, since module paths under `std` are reserved (`spec/13 §7`).
- **Messages only.** The message of `async` and `await` changes in edition `0.4`; a change of message.
- **Unsettled points:** none.
- **Trust,** under ADR 0121 rule 2 (b): the trusted crate `una-sys` with the stack switch of coroutines, and the native runtime rebuilt around coroutines, for packages of every edition; each change to `una-sys` gets a security review (ADR 0058 amendment of 2026-10-18). No second opt-in.
- **Chapter.** Its rules live in `spec/27`, as its header states (ADR 0121 rule 20).

**ADR 0114, Note of 2026-10-19.** The carve-out of rule 15 stays the only exception to the erasure clause in edition `0.3`, besides the allowances of ADRs 0103 and 0104, and is not a precedent there: a construct that does not erase targets edition `0.4`, under the terms of ADR 0121. In edition `0.4`, `F64` is inherited by cumulativity and needs no carve-out. **Chapter:** `spec/26`, which this record's header names, is the chapter of ADR 0116; this record's rules live in `spec/29`, the numeric foundation, beside `F32` and `Complex` of ADR 0120 (ADR 0121 rule 20).
