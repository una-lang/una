# Pre-tag review of v0.2.0, Section 4: docs and release consistency

- Candidate: `ef34f2f604657572da0c8ec97612f2e923549ef8` (main after #255)
- Release PR checked: #230 at `0ff7b0c`, merge base `45423f3` (before #255)
- Reviewer: Una-1
- Severity: Critical (unsound, disallowed native divergence, release gate failure), Major (meaning-changing diagnostic or repair, lost trace or exit parity, failing docs example, meaning-changing doc error), Minor (everything else)

## Summary

| Severity | Count | Findings |
| --- | --- | --- |
| Critical | 0 | |
| Major | 3 | S4-01, S4-02, S4-03 |
| Minor | 8 | S4-04 to S4-11 |

S4-02 is a release gate failure waiting to happen: if #230 merges unchanged, the `build` gate fails on the tag SHA. It is Major here because it is not on the candidate yet, and it becomes Critical if #230 merges as it is.

## What was checked and found consistent

- **ADRs 0055 to 0070.** Every one is `Status: Accepted`. Spot checks of the Amends targets of ADRs 0065 to 0068 found the text in place: `spec/19` and `spec/20` exist; ADR 0067's rules are in `spec/07:95`, `spec/10:112` and `spec/14:61`, and `spec/13` has `result labels` and `generic_results`; ADR 0068 is in `spec/10 §12` rule 14, `spec/11:119` and `:124` (the `<hashed>` amendment), `spec/20:16` and both guides. Amendments are appended as dated sections, never edits.
- **Diagnostic codes.** The 84 codes of `spec/13` are exactly the codes in `crates/` (the only extras, `U0000` and `U9999`, are test placeholders for invalid codes). `una explain` answers all of them except U4006 and U6002, which `spec/13:181` documents as unassigned gaps.
- **CLI flags in the docs.** Every `una` flag named in `README.md`, the guides and `spec/11`, `13`, `16`, `19` exists in the subcommand help (`--tier`, `--check`, `--json`, `--grant`, `--max-depth`, `--max-memory`, `--solver-wallclock`, `--out`, `--dump-core`, `--color`). The others are flags of `cargo`, `local-ci.sh` and `install.sh`, or of the native executable (`--trace`), see S4-07.
- **Guide and README examples.** `crates/una-cli/tests/guide.rs` checks every `una` block of both guides as a package (tier 1, canonical form, and the commands they show), and `the_readme_shows_only_checked_code` covers the README. They run in the `build` gate (`cargo test --workspace`), so their verdict is that gate's result on the candidate and on the tag SHA.
- **Linux.** #230 states that v0.2.0 ships macOS arm64 and Linux x86_64, with Linux arm64 after the tag (#241, decision N-02).

## Findings

### S4-01 (Major): `spec/21` and `spec/22` are normative for edition 0.2, but v0.2.0 does not implement them

- **Evidence.**
  - `docs/spec/21-maps.md:3` and `docs/spec/22-text.md:3` say `Status: Normative, v0.2`.
  - Cross references written as present tense rules of edition 0.2:
    - `spec/03:22` ("Edition `0.2` adds `Map[K, V]`");
    - `spec/12:91` (the sort `i!Map_<K>_<V>`);
    - `spec/14:45` and `:47` ("the prelude gains `empty_map()`", "the text functions of `spec/22 §2`");
    - `spec/16:63` (in-place `set` and `remove` on a map, in-place text append);
    - `spec/20:22` (`Map` satisfies `Eq`).
  - On the candidate, an edition 0.2 package with `let counts: Map[Text, Int] = empty_map()` and `split("a,b", on: ",")` gets three U1001 errors (`cannot find type Map`, `cannot find name empty_map`, `cannot find name split`).
  - The CHANGELOG of #230 says only "no map type" under Known limits, and says nothing about the text operations.
- **Why Major.** The spec is the source of truth (`CLAUDE.md`). An agent that reads it writes programs that v0.2.0 rejects, and the spec says that the compiler is wrong.
- **Suggested fix**, docs only, which the freeze allows as a fix for this finding:
  - change both Status lines to "Accepted (ADR 0069), not implemented in v0.2.0: normative from the release that implements #236" (and #237 for `spec/22`);
  - mark each cross reference above "(not in v0.2.0, #236)" or "(#237)";
  - in #230, add a "Specified, not yet implemented" section, as v0.2.0-alpha.3 did, listing ADR 0069 and ADR 0070.

### S4-02 (Major; Critical if #230 merges unchanged): #230 bumps the version without `install.sh`, and a test fails

- **Evidence.**
  - #230 changes `Cargo.toml` to `0.2.0` and leaves `install.sh:42` at `DEFAULT_VERSION=0.2.0-alpha.3`. `git show pr230:install.sh` still has it.
  - `crates/una-cli/tests/install.rs:37` asserts that `DEFAULT_VERSION` equals the workspace version.
  - The `build` gate runs `cargo test --workspace`, so it fails on the tag SHA. The installer would also default to alpha.3, which the CHANGELOG itself supersedes for a Critical issue.
  - #230's merge base is `45423f3`, before #255 (#253) and #252 (`install.sh`). Its "Still to do" list predates both.
- **Suggested fix.**
  - Rebase #230 onto `ef34f2f`.
  - Set `DEFAULT_VERSION=0.2.0` in `install.sh`.
  - Run `cargo test -p una-cli --test install`, then the full local CI, on the rebased tip before merge.

### S4-03 (Major): #230's CHANGELOG still lists #253 as pending, and says edition 0.1 accepts the leak

- **Evidence.** The #230 CHANGELOG, Security section, has "Pending: a decision on `T` values through a call of a function-typed parameter in edition 0.1 (#253, Critical) ... is accepted ... on this release's edition 0.1". On the candidate, #255 merged that fix: edition 0.1 now rejects the shape, so 0.1 programs that v0.1.3 accepted are rejected.
- **Why Major.** The release notes would state the opposite of the shipped behavior on a security fix, and they would not mention a new rejection in edition 0.1, the edition Gate 1's benchmark uses.
- **Suggested fix.**
  - Make it a fix line: "Fixed in edition 0.1 (#253, Critical): ... now U4003; programs v0.1.0 to v0.1.3 accepted with this shape are rejected; edition 0.2 is unchanged."
  - State whether a v0.1.4 patch follows (the owner's call), and keep "v0.1.x is affected" accurate for the pinned v0.1.3.

### S4-04 (Minor): the CHANGELOG does not say which changes reach edition 0.1

- **Evidence.**
  - #205, #206 and #211 change messages and repairs in edition 0.1 too. Their conformance cases are single-file edition 0.1 cases: `tests/conformance/types/variant_compared_without_fields.una`, `syntax/reserved_result_as_binding.una`, and `syntax/handling_clause_on_its_line_repair/una.toml`.
  - #230 lists them under "Interpreter and tools" with no edition.
- **Suggested fix.** Add a short "Changes in edition 0.1" subsection that lists #205, #206, #211 and #253 (S4-03), for users of the v0.1.x pin.

### S4-05 (Minor): the CHANGELOG omits the installer and the release gates

- **Evidence.**
  - Missing from #230:
    - `install.sh` and its verification by the validation record (#240, #252), which the README presents as new from v0.2.0 (`README.md:378`);
    - `scripts/release-gates.sh` and `docs/releases/validation.md` (#232, #249, #254).
  - Known limits says the release was "built and checked locally (`scripts/local-release.sh`)". It is now gated by `release-gates.sh`, and the record is `docs/releases/0.2.0-validation.md`.
- **Suggested fix.** Add both under Tools, and point Known limits at the validation record.

### S4-06 (Minor): the CHANGELOG's list of ADRs stops at 0068

- **Evidence.** The #230 "Architecture decisions" list ends at ADR 0068, but ADRs 0069 and 0070 are Accepted on the candidate.
- **Suggested fix.** List them as accepted and specified, not implemented (with S4-01).

### S4-07 (Minor): the agent guide says that `una run` takes `--trace`

- **Evidence.**
  - `docs/guide/una-for-agents.md:780` says "`--grant`, `--max-depth`, `--max-memory` and `--trace <file>` work as for `una run`".
  - `una run --help` has no `--trace`; `una run` gives its trace through `--json`. `--trace <file>` is a flag of the native executable only (`spec/16:44`).
  - `syntax-reference.md:2074` states it correctly.
- **Suggested fix.** "... and `--max-memory` work as for `una run`, and `--trace <file>` writes the trace as a JSON array".

### S4-08 (Minor): `install.sh` calls `una --version`, which the CLI rejects

- **Evidence.** `install.sh:195` runs `"$home/bin/una" --version 2>/dev/null || echo "una $version"`. On the candidate, `una --version` exits 2 with "unexpected argument '--version'", so the fallback always runs. The message is right, but the check is dead code.
- **Suggested fix.** Drop the call and print `una $version`. A `--version` flag would be new CLI surface, which the spec does not define.

### S4-09 (Minor): the README release table and CI paragraph are stale

- **Evidence.**
  - `README.md:405-406` lists the `0.1.2` archives.
  - `README.md:43` is the v0.1.2 status paragraph, which #230 keeps above its new v0.2.0 one.
  - `README.md:606` describes GitHub Actions as running CI and releases, while `docs/releases/validation.md` §1 rule 4 says it is not a gate. `README.md:608-610` describes `local-release.sh`, not `release-gates.sh`.
- **Suggested fix.** In #230:
  - list the `0.2.0` archives;
  - point to `docs/releases/validation.md` and `release-gates.sh`;
  - shorten the v0.1.2 paragraph to a line.

### S4-10 (Minor): backlog rows do not reflect v0.2

- **Evidence**, in `docs/plan/backlog.md`:
  - Shipped items are still listed with no status, some under "Deferred": #44 (proof cache) and #45 (memory limit), shipped in alpha.3; #41 (`filter` and `first_where` over labeled elements) and #92 (result summaries), shipped in v0.2.0.
  - Rows targeted at `v0.2` that do not ship need a new target: #27, #31, #33, #34, #39, #40, #42 and #82.
  - Map (#236) and Text (#237) have no row.
  - The intro paragraph lists ADRs only up to 0066.
- **Suggested fix.** Add a status column, or move shipped rows to a "Shipped in v0.2.0" section. Retarget the rest. Add the #236 and #237 rows.

### S4-11 (Minor): D-133 to D-159 are still unanswered

- **Evidence.**
  - In `docs/history/v0.1/open-decisions.md`, 26 entries (D-133 to D-159, except D-137) have `- Answer needed` and no `- Answer:` line.
  - The file's header says they "await the owner's answer". Their `DECISION D-NNN` tags are still in `crates/` and `tests/`.
  - D-137 and D-141 are answered (switch to B in edition 0.2, ADR 0067).
- **Why Minor.** These are decisions of the v0.1 closure run, not v0.2 behavior, and none blocks the tag.
- **Suggested fix.** Put them to the owner as one batch, or mark them "deferred past v0.2.0" in the header.
