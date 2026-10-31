# 0131. The format and extension of Una's own configuration files

- Status: Proposed
- Date: 2026-10-31
- Deciders:
- Resolves: issue #1302
- Related:
  - ADR 0021 (packages and project files: `una.toml`, `verification.lock`), the TOML files of editions `0.1` to `0.3`;
  - ADR 0122 (Itá: `ita.una`, `ita.lock`, `itaty.una`, `mirrors.una`), whose rule 3 reads its files "by Una's lexer and by the grammar of rules 5, 16 and 19, which is part of the syntax crate", and whose Question 8.1 gives edition `0.4` "one manifest format and one parser"; issue #893;
  - ADR 0127 (Una Quality, Proposed) rule 20 and `spec/32 §14`: `quality.toml` is interim, and its schema is a data model independent of the file format;
  - ADR 0063 (`una lsp`); issue #1295 (the Language Pack); epic #1292 and issue #1304 (the quality configuration).
- Target: the files of edition `0.4` and later, and the configuration of tools that serve every edition (`una quality`). Editions `0.1` to `0.3` keep `una.toml` and `verification.lock` as ADR 0021 states.
- Amends, on acceptance and with the implementation: `spec/30 §1` (the lexical rules, as the shared layer of every configuration file), `§8` (`una migrate`), `spec/32 §14` (the file), `spec/13 §5` (commands given a configuration file). ADR 0122 by the dated amendment appended to it in this change.

> **In plain words.**
> - **One family.** Every configuration file a person writes for Una is written in Una's own lexical syntax, with a closed grammar, and ends in `.una`: `ita.una`, `itaty.una`, `mirrors.una`, and `quality.una`, which replaces `quality.toml`. A file that only the tool writes keeps its own extension (`ita.lock`). Machine documents (reports, caches, SARIF) stay JSON.
> - **Telling them apart.** A module lives under `src/` and starts with `module`. A configuration file lives at a fixed name outside `src/` and starts with its own word (`ita`, `quality`), which is also its name.
> - **Nothing runs.** Configuration files have no expressions, no calls, no capabilities and no includes.
> - **Until then.** `quality.toml` stays the format while this record is Proposed and until its slice lands; `una migrate` then rewrites it mechanically.

## Context

### What exists

| File | Who writes it | Format today | Record |
| --- | --- | --- | --- |
| `una.toml` | people | TOML | ADR 0021, editions `0.1` to `0.3` |
| `verification.lock` | people, rarely | TOML | ADR 0021, editions `0.1` to `0.3` |
| `ita.una` | people (`una ita` edits needs) | Una lexical syntax, closed grammar | ADR 0122, edition `0.4` |
| `ita.lock` | `una ita` and `una migrate` only | Una lexical syntax, closed grammar, extension `.lock` | ADR 0122 rule 16 |
| `itaty.una` | people | Una lexical syntax, closed grammar | ADR 0122 rule 19 |
| `~/.una/itaoca/mirrors.una` | people | Una lexical syntax, closed grammar | ADR 0122 rule 40 |
| `quality.toml` | people | TOML, interim | ADR 0127 rule 20, `spec/32 §14` |
| the key file of a `keys:` grant | the operator | TOML table | `spec/13 §5` |
| `.una/cache/proofs/*` | the tool | JSON | `spec/12 §9` |
| reports, snapshots, SARIF | the tool | JSON with a `schema` key, SARIF 2.1.0 | `spec/13 §6`, `spec/32 §17` |

ADR 0122 already chose Una's own syntax for edition `0.4`'s project files, for three reasons that hold for every configuration file: one syntax for agents to learn, no second parser in the trusted base for every package, and lines such as `contracts` that embed Una forms parsed by the production a header uses (ADR 0122, alternative A). `quality.toml` is the one new file that did not follow, by an explicit interim decision (ADR 0127 rule 20), pending this record.

### What #1302 asks

1. Which configuration files exist or are planned, and the format and extension of each.
2. One answer for the extension and naming scheme, and the rule that tells a reader or a tool what kind of file it is looking at.
3. Whether configuration is read by the same parser as source, and what the closed grammar forbids.
4. The interim policy for `quality.toml`.
5. Migration (`una migrate`), and the effect on editor support, the language server and the Language Pack.
6. The trade-off of keeping a TOML reader for the interim.

## Decision

### A. The family

1. **Three kinds of files.**
   - **Configuration** is what people write and review to configure the language or a tool: today `ita.una`, `itaty.una`, `mirrors.una`, and `quality.una` (rule 9). It is written in **Una data syntax** (rule 3) and ends in `.una`.
   - **Locks** are what only a tool writes and people review in diffs: today `ita.lock`. A lock is written in Una data syntax too, and keeps an extension of its own, `.lock`, which says "do not edit by hand". A lock has a head with its format version (`lock 1`).
   - **Machine documents** are what a tool writes for another program: reports, snapshots, caches, SARIF, the `--json` output of every command. They stay JSON, each with its `schema` key (or SARIF's own), since their readers are other programs and other languages. They are not configuration, and nothing reads them as such.
2. **New files join the family by a record.** A future policy, preset or other configuration file is configuration: its record names its head word (rule 5), its place, its closed schema and its diagnostic code. No configuration file in another format is added. In particular:
   - **Presets** are built into `una quality` (`spec/32 §14.3`); a user preset, if one is ever added, is a configuration file of this family.
   - **CI files.** Una adds none. A pipeline passes a file by path (`--config`), as ADR 0127 rule 20 states; the tool knows no environments.
   - **Key files** of `keys:` grants hold secrets the operator supplies at run time; they are inputs of a run, not configuration of a package, and a package never holds one (ADR 0122 rule 27). Their format is left as `spec/13 §5` states, and a separate card decides it (Open items), so that a secret never gets an extension that the tools treat as a package file.

### B. One syntax, one parser

3. **Una data syntax** is the shared layer of every configuration file and lock. It is the lexical layer that `spec/30 §1` already states for `ita.una`, `ita.lock` and `itaty.una`, now named and shared:
   - the lexer of `spec/01`: UTF-8 without a byte order mark, two-space indentation, `--` comments, no tabs, the control characters of `spec/01 §1` rule 6 rejected, identifiers, and integer and text literals;
   - the version token of `spec/30 §1` rule 1, where a file's grammar names it;
   - text without interpolation: `{` and `}` only as `\{` and `\}`;
   - **the shape:** a file is a head line, then a tree of lines. Each line is a sequence of tokens: words, literals, versions and the punctuation `,`, `.`, `:`, `/`, `[` and `]`. A block of lines opens by indentation under the line that names it. No line holds an operator outside a production that a grammar embeds by name (rule 4);
   - **bounds:** each grammar states a size limit (`ita.una` 1 MiB, `ita.lock` 16 MiB, `quality.una` 1 MiB) and a nesting depth of at most 8 levels. Past either, the file is refused whole.
4. **One parser.** Every configuration file and lock is read by Una's lexer and by a grammar of the syntax crate, as ADR 0122 rule 3 states for its files. One lexer and one parser crate serve source and configuration.
   - **Each kind has its own closed grammar**, a set of productions over lines. It is not the grammar of a module: a configuration file is never parsed as a module, and a module is never parsed as a configuration file.
   - **Embedded forms are shared, not copied.** Where a line holds a form of the language, such as an operation's condition in a `contracts` line (`cap_type_with_conditions`, `spec/05 §3` rule 4), the grammar calls the same production a header uses. No configuration grammar has a second parser for any part of the language.
   - **No expression production is reachable** from a configuration grammar except through such a named form, so "no calls" holds by construction, not by a later check.
5. **What tells a file's kind.**
   - **Place decides discovery.** A package's modules are exactly `src/**/*.una` (`spec/13 §7`, `spec/30 §2`). A configuration file is found at a fixed name outside `src/`: `ita.una` and `quality.una` at a package root, `itaty.una` at a workspace root, `mirrors.una` in Itaoca, or a path that a flag names (`--config`). A file under `src/` is always a module: `src/ita.una` is the module `ita`.
   - **The head word decides the kind.** A module's first token, after comments and blank lines, is `module` or `trusted` (`spec/02`, `module_header`). A configuration file's first token is its kind's **head word**, which is also the stem of its fixed name: `ita`, `itaty`, `mirrors`, `quality`; a lock's is `lock`. The head words are words of the configuration grammars, never keywords of a module (`spec/30 §1` rule 2).
   - **Agreement.** A file found at a configuration name whose head word is another kind's, or a module's, is that kind's error at its first line: U7008 for Itá's files, UQ0001 for `quality.una`. A file named to a command by path is classified by its head word alone.
   - **Cost.** Classifying a file reads its first token: O(1) after the lexer's start, with no second pass.
6. **What a closed grammar forbids.** Every configuration grammar, now and later:
   - **no expressions** outside a named embedded form: no operators, no conditions on values, no arithmetic, no interpolation;
   - **no calls**: nothing names a function to run, and no line is evaluated;
   - **no capabilities as authority**: a line MAY name a capability type and operations as data, as `may` and `contracts` lines do, and that names a ceiling or a lock, never a grant to code. Host grants stay in `run grants` of the root's `ita.una` only (ADR 0122 rule 52);
   - **no execution**: no scripts, hooks, plugins, build steps or commands, before, during or after reading;
   - **no includes, inheritance, environment variables or conditions**: one file is one complete configuration, so its digest describes it;
   - **closed keys**: every section and line has one of the grammar's forms; an unknown, repeated or misplaced line is the kind's error at the line.

   These are the rules ADR 0122 rule 3 states for Itá's files, made the rule of the family.
7. **Canonical form.** `una fmt <file>` formats a configuration file by the canonical form of `spec/30 §2` rule 6, generalized: two spaces per level, one blank line between top-level blocks, one space between words, the grammar's order of sections. `una fmt --check <file>` reports U7001. `una fmt` of a package directory formats its modules only, as today; `una fix` never edits a configuration file or a lock (ADR 0122 rule 14, ADR 0127 rule 22).
8. **Commands given a configuration file.** `una check <file>` with a configuration file reads it by its grammar and schema and reports its diagnostics, with the exit codes of `spec/13 §5`. `una run`, `una test` and `una build` given one are usage errors (exit code 2), "`quality.una` is a configuration file, not a module".

### C. `quality.toml` and `quality.una`

9. **`quality.una`.** The configuration of `una quality` is the file `quality.una`, head word `quality`, in Una data syntax. It carries the data model of `spec/32 §14.2` unchanged: the same keys, values, defaults and checks, each TOML table or key becoming a section or a line. An illustration of the mapping (a proposal; `spec/32 §14` fixes the grammar with the implementation):

   ```una
   quality 1
     preset server

   analysis
     mode verified
     incomplete block
     required not_measured, partial, inconclusive, timeout

   scope
     generated "src/gen"
       reason "parser tables"

   limits
     mutants 2000
     seconds per phase 600

   metric complexity.cyclomatic
     severity block
     warn 10
     block 15
     scope new

   evolution complexity.cognitive
     severity warn
     rule no_increase

   waivers
     max days 90

   waiver UQ1001 billing.invoice.total
     reason "table-driven dispatch, to be split"
     approver "billing maintainers"
     expires "2027-01-31"
     fingerprint "sha256:..."

   output
     max recommendations 5
   ```

   - The head `quality 1` carries the schema version that TOML's `schema = "una.quality-config/1"` carried.
   - A metric id is a dotted name of lower words, which the lexer reads as words and dots.
   - A date is a text literal in the form `YYYY-MM-DD`, checked by the schema, since the lexer has no date token and this record adds none.
   - Every error that `spec/32 §14.1` rule 3 makes UQ0001 stays UQ0001, at the line.
   - `quality.una` serves packages of every edition, since `una quality` does: the format of a tool's configuration follows the tool, not the package's edition. A package of edition `0.1` to `0.3` therefore has `una.toml` and `quality.una` side by side, which is the expected state until it migrates.
10. **The interim policy.** While this record is Proposed, and after its acceptance until its slice lands, `quality.toml` is the format, exactly as ADR 0127 rule 20 and `spec/32 §14` state. Nothing changes for the quality epic's phases meanwhile; they implement the data model, which both formats carry.
11. **After the slice lands.**
    - `una quality` reads `quality.una` by default, and a `--config` path by its head word.
    - A package root that holds `quality.toml` without `quality.una` is UQ0001, "`quality.toml` is no longer read; run `una migrate`", with the repair `una migrate`. A root with both is UQ0001 at `quality.toml`, so a package never has two quality configurations that could disagree. No compatibility period is kept: there is no compatibility promise before v1.0.
    - The configuration digest that the report shows (`spec/32 §14`) is the SHA-256 of the file's bytes, so a migrated file has a new digest. The guarantee diff (#1291) reports the format change once, as a change of configuration with an equal data model, which `una migrate` states in its output.

### D. Migration

12. **`una migrate` converts `quality.toml`.** Besides its edition move (ADR 0122 rule 64, `spec/30 §8`), `una migrate [path]` converts `quality.toml` at the package root to `quality.una`, whatever the package's edition:
    - on a package of edition `0.1` to `0.3`, `una migrate` does both, the edition move and the quality file; `una migrate --only quality` converts the quality file alone and leaves `una.toml`, `verification.lock` and the edition as they are;
    - on a package that already has `ita.una`, which ADR 0122 rule 64 makes a usage error today, `una migrate` converts `quality.toml` when there is one, and is a usage error only when there is nothing to convert (the amendment of ADR 0122 appended in this change);
    - the conversion is mechanical: it reads the TOML file against the schema, refuses with UQ0001 and writes nothing if the file is invalid, and writes `quality.una` in canonical form through a temporary file and a rename, then removes `quality.toml`. TOML comments before a key or table are carried above the line it becomes; others are listed in the output, as ADR 0122 rule 64 does for `una.toml`;
    - `--dry-run` and `--json` (`una.migrate/1`, with the converted file in `written`) behave as ADR 0122 rule 64 states;
    - **cost:** linear in the file's size.
13. **Round trip.** The slice includes a test that, for every key of `spec/32 §14.2` and every preset, converting a `quality.toml` and reading the resulting `quality.una` gives the same data model, and the same report, as reading the TOML file.

### E. Editors, the language server and the Language Pack

14. **Editor support.** An editor's syntax coloring for `.una` files applies to configuration files as is, since they share the lexer: comments, words, literals and versions color the same. No new file type is needed in an editor. Coloring that needs the kind, such as section words, comes from the language server's semantic tokens.
15. **The language server** (`una lsp`, ADR 0063) classifies each open document by rule 5 and serves it by its kind:
    - **diagnostics** of the kind's grammar and schema (U7008, U7009, UQ0001), as `una check <file>` reports them;
    - **formatting** by rule 7;
    - **completion** of the section and line words its closed grammar allows at the cursor, and of metric ids and preset names for `quality.una`;
    - **hover** with the meaning of a line, from the schema's documentation;
    - **no code runs**, as for modules: the server reads, checks and formats.

    Cost: one classification per document open, O(1) (rule 5); a configuration file's check is linear in its size.
16. **The Language Pack** (#1295) gains one entry per configuration kind: its head word, place, grammar, an example, and rule 5's way to tell it from a module, so an agent never edits a configuration file as code, or the reverse. The entry for `quality.una` replaces the one for `quality.toml` when the slice lands.

### F. The TOML reader

17. **The trade-off.** Keeping a TOML reader in `una` for `quality.toml` conflicts with ADR 0122's goal of one manifest format and one parser in edition `0.4` (Question 8.1). This record **accepts it for the interim**, for these reasons:
    - **No new dependency.** `una` holds a TOML reader anyway for the files of editions `0.1` to `0.3` (`una.toml`, `verification.lock`) and for the key files of `keys:` grants, and ADR 0127 rule 20 uses that same reader. The interim adds no crate to the trusted base and no code path that another file does not already exercise.
    - **The cost is to agents, and short.** An edition `0.4` package with `quality.toml` shows two syntaxes until the slice lands; rule 11 ends that state with one command.
    - **The end state.** After the slice, no file of an edition `0.4` package is TOML. The TOML reader stays in `una` only while it serves editions `0.1` to `0.3` and key files; removing it is a decision of the record that retires those editions, not of this one.

## Consequences

- One syntax and one parser for every file a person writes to configure Una, from edition `0.4` on, and for `quality.una` in every edition.
- A reader, a tool or an agent tells a module from a configuration file by its place and its first word, at O(1) cost.
- Every future configuration file is closed, declarative and bounded by rule, so none can become a way to run code.
- `quality.toml` keeps working, unchanged, until its slice lands; then `una migrate` converts it, and the reader refuses the old file with a repair.
- **Harder:** tools outside Una that read TOML cannot read `quality.una`. The JSON documents of `una quality` (`una.quality/1`) and SARIF stay their interface.
- **Deferred:** the key file's format (Open items), and the removal of the TOML reader with editions `0.1` to `0.3`.

## Alternatives considered

- **Keep TOML for configuration, Una syntax for the manifest only.** Not chosen: two syntaxes in an edition `0.4` package forever, a second parser that every configuration-reading tool keeps, and quality waivers, metric ids and future policy lines written in a format whose strings no Una production checks. ADR 0122's alternative A rejected TOML for the same reasons.
- **A distinct extension for configuration** (such as `.unc` or `.ita`). Not chosen: a reader would need to learn a second extension for the same lexical syntax, every editor would need a second file type, and the extension would add nothing that the place and the head word do not already decide. `ita.una` set the precedent, and it is accepted.
- **A generic data language in Una syntax with no per-kind grammar** (any tree of keys and values, checked by a schema afterwards). Not chosen as the reader's model: closed grammars report a misplaced or unknown line at the parser, with the kind's code, and embed language forms by production. The shared shape of rule 3 keeps the tooling benefit of a generic layer (one lexer, one formatter, one classifier) without opening the grammar.
- **Configuration as Una code** (a module whose functions return the configuration). Rejected: it would run code to configure the build, which ADR 0122 rule 3 and #893 forbid, and it would need the checker and the interpreter to read a manifest.
- **JSON for configuration.** Not chosen: no comments, and its strings would carry Una forms that a second parser reads. JSON stays the format of machine documents (rule 1).
- **Moving `quality.toml` now, before this record is accepted.** Not chosen: the quality epic would depend on an undecided grammar. The data model is format independent (`spec/32 §14`), so waiting costs one mechanical migration.
- **Reading both `quality.toml` and `quality.una` after the slice.** Not chosen: two files that could disagree, and no compatibility promise before v1.0 to keep.

## Slices

Each slice is a card in Needs spec, to start after this record is accepted:

1. **Spec:** (#1333) `spec/30 §1` names Una data syntax and rules 3 to 8 as the family's shared rules; `spec/32 §14` gives the grammar of `quality.una`; `spec/13 §5` states rule 8. Conformance cases first.
2. **Parser:** (#1334) the grammar of `quality.una` in the syntax crate, sharing the lexer and the classifier of rule 5; the size and depth bounds; a fuzz target.
3. **`una migrate` and the quality reader:** (#1335) the conversion of rule 12 with its round-trip test (rule 13); `una quality` reading `quality.una`; UQ0001 for `quality.toml`.
4. **Editor support:** (#1336) `una lsp` classification, diagnostics, formatting, completion and hover for configuration files (rule 15); the Language Pack entries (rule 16).
5. **Record: the key file's format** (#1337) (Open items).

## Open items

- **Key files.** Whether the key file of a `keys:` grant moves to Una data syntax, and under which name and extension, so that a secret file is never taken for a package file. Until it is decided, it stays a TOML table (`spec/13 §5`).
- **The quality grammar's exact words** (rule 9 shows a proposal of the mapping).
