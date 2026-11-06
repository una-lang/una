# 0138. How a manifest with an invalid entry falls back: key by key, failing closed on trust and grants

- Status: Proposed
- Date: 2026-11-06
- Deciders: Una project (proposed; not accepted)
- Resolves: issue #930
- Related:
  - ADR 0021 (packages and project files) and `spec/13 §7`, the tables and keys of `una.toml`, U7002;
  - ADR 0122 and `spec/30 §2`, `ita.una` of edition `0.4`, U7008, whose rule 5 maps every U7002 of a key to U7008 at the corresponding line;
  - ADR 0093 (`[boundaries]`), ADR 0092 rule 8 (`[contracts]`), ADR 0117 rule 8 (limits on grants), ADR 0123 rule 7 (`serve` grants).
- Target: every edition. It states the behavior the toolchain has for `una.toml`, with one change that fails closed (rule 3).

## Context

`spec/13 §7` lists the tables and keys of `una.toml` and makes a TOML syntax error, an unknown table or key, a missing required key and a value of the wrong type or out of range U7002. It says that "project errors do not stop checking: the modules are still checked, with the default settings in place of an invalid `una.toml`". Read alone, that sentence says any invalid manifest is replaced as a whole by the defaults. The toolchain (`una_project::manifest`) does not do that, and issue #930 asks for the text to state the behavior exactly:

- invalid TOML: every setting takes its default (empty `[labels]`, `[trusted]` and `[run]`, a budget of 0);
- an unknown table or key: U7002 at it, and it is skipped; the rest of the manifest is read;
- a value of the wrong type or out of range: U7002 at it, and the key keeps its default;
- an array (`[labels] categories`, `[trusted] modules`, `[run] grants`) is read element by element: an element that is not a string, a module path or a grant spec is U7002 at it and dropped, and the other elements are kept.

The edition is the reason a key-by-key reading is worth having: with the edition kept, the modules are checked in the edition the author wrote, so the other diagnostics a broken manifest produces are the ones its author expects.

Two of these settings decide authority: `[trusted] modules`, which lets a module declare policies that lower labels (`spec/07 §6`), and `[run] grants`, which grant host capabilities to `main` (`spec/13 §5`). Writing the key-by-key behavior into the spec as it is would make it normative for them too. A manifest whose author wrote three trusted modules, one of them malformed, or five grants, one of them malformed, would be read as a set its author never wrote. Dropping an element of these lists is the safe direction for the element itself, but the set that remains is still a guess at the author's intent, and every tool that reads the manifest of a rejected package (the build report's TCB section, the language server, a future command that acts on a package's grants) would act on that guess. `ita.una` already refuses a guess of this kind in one place: a grant with a malformed limit line is not read at all, "since without its limit it would allow more" (`spec/30 §2` rule 5).

U7002 and U7008 are errors, so a package with any of them is rejected: `una check` fails, and `una run` and `una build`, which check first, run and build nothing. This decision does not change that; it decides what the toolchain reads while it reports.

## Decision

1. **Key by key.** A manifest is read key by key. A TOML syntax error (in `una.toml`), or a syntax error of the file's grammar that leaves its structure unknown (in `ita.una`), replaces every setting with its default. Otherwise each problem is reported at its key or line (U7002 in `una.toml`, U7008 in `ita.una`) and only that key or line takes its default: an unknown table, key, section or line is skipped, and a value of the wrong type or out of range leaves its key at the default.
2. **Lists element by element.** An array of `una.toml` or a section of lines of `ita.una` is read element by element: an element that is invalid is reported at it and dropped, and the others are kept, except as rule 3 states.
3. **Fail closed on trust and grants.** When any entry of `[trusted] modules` or of `[run] grants` (in `ita.una`, of the `trusted` section or of the `grants` of `run`, a grant's own lines included) is invalid, or the key itself has the wrong type, the whole manifest falls back to the defaults, as for invalid TOML: no module is trusted and nothing is granted, and every other setting takes its default too, except the edition (rule 4). The problem is reported at the entry, with the message that the manifest was not read because an entry of `[trusted]` or of the grants is invalid. A trusted module that its manifest names correctly is still checked against its header (U7005), and a grant that is valid is never read from a manifest whose other grants are not.
4. **The edition under rule 3.** The edition that the manifest names is still read when it is valid, so the modules are checked in their edition and the diagnostics of the fallback are those of that edition; only the settings of rule 3's fallback take their defaults. An edition that is itself invalid takes the default, as today.
5. **Text.** `spec/13 §7` replaces "Project errors do not stop checking: the modules are still checked, with the default settings in place of an invalid `una.toml`" with rules 1 to 4, and `spec/30 §2` rule 5 states them for `ita.una` with U7008. The explanation of U7002 and U7008 names rule 3.

## Consequences

- The spec states what the toolchain reads, as #930 asks. Rules 1 and 2 describe today's behavior of `una.toml`; rule 3 changes it for the two lists that decide authority.
- A package with a malformed trusted module or grant gets more diagnostics: with no module trusted, each `trusted module` header is U7005 and each policy outside a trusted module U4004, and under `una run` each `uses` of `main` that no `--grant` grants is U3012. They follow from one entry, which the first diagnostic names. This is the price of failing closed.
- No package that checks today changes its result: rule 3 applies only to a manifest that is already rejected.
- The matching rule for `ita.una` keeps its existing refusals (a grant with a malformed limit line is not read; a `serve` section with a problem is not read), which rule 3 subsumes for the grants.
- Implementation: `una_project::manifest` and `una_project::ita`, a flag set by any problem of the two lists and checked once at the end of parsing, and conformance cases for each table of rule 3 and its valid twin.

## Costs

Parsing stays linear in the size of the manifest. Rule 3 adds one flag and, when it is set, one replacement of the parsed settings by the defaults, O(1) beyond the parse.

## Alternatives considered

- **Replace the whole manifest on any error.** The plain reading of today's sentence. Rejected: a typo in an unrelated key (`license = "MIT"` in `[package]`) would change the edition every module is checked in and bury the one diagnostic that matters under the diagnostics of the wrong edition.
- **Key by key everywhere, the trust and grant lists included.** Today's behavior. Rejected for those two lists: what remains is a set the author never wrote, and the safe direction of dropping one element does not make the remaining set the author's.
- **Fail the whole manifest on any problem of a table that restricts authority** (`[boundaries]`, `[contracts]`, the limits of a grant) as well. Not proposed: a dropped ceiling or lock is reported, and the package is rejected; the question is open for acceptance below.

## Questions for acceptance

1. Should rule 3 also cover the tables that restrict authority, `[boundaries]` (ADR 0093) and `[contracts]` (ADR 0092 rule 8), whose dropped entries leave a module with a wider ceiling or a function with no locked `uses` in what the toolchain reads? The recommendation is yes, as the more secure option; this record proposes the narrower rule that the project's recommendation named, and leaves the extension to this question.
2. Should the message of rule 3 be a separate diagnostic of the fallback, beside the U7002 or U7008 at the entry, or the U7002 or U7008 itself with a longer message? This record proposes the latter: no new code.
