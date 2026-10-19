# 30. Packages and dependencies

Status: Normative, edition 0.4 preview (ADR 0122).

From edition `0.4`, a package describes itself in `ita.una`, a declarative file in Una's lexical syntax, and is pinned by `ita.lock`, which only the tool writes. A repository may group packages in a workspace with `itaty.una`. Itá, the package manager, is the command `una ita` of the `una` binary. The rules of ADR 0122 join this chapter as they land, each with its conformance cases. Until the rules that load dependencies have landed, a package may write `needs`, which is read and checked, but every need is refused (U7008), so no import ever reaches a package that is not checked with the program.

**Editions.** The rules of this chapter apply only to packages of edition `0.4` and later. Editions `0.1` to `0.3` keep `una.toml` and `verification.lock` (`spec/13 §7`), and a single file keeps edition `0.1` and has no dependencies.

## 1. Lexical rules

1. **The lexer of `spec/01`.** `ita.una`, `ita.lock` and `itaty.una` are read by the lexer of `spec/01`: UTF-8 without a byte order mark, two-space indentation, `--` comments, no tabs, the control characters of `spec/01 §1` rule 6 rejected, identifiers, and integer and text literals as `spec/01 §4` and `§6` state. Two differences hold in these files only.
   - **A version token.** `version_lit = num "." num [ "." num ] [ "-" pre { "." pre } ]`, where `num` is `0` or digits without a leading zero, and `pre` is `[0-9A-Za-z]+`, a numeric `pre` without a leading zero. It is matched before integers, so `1.4` and `0.1.0` are versions here. A malformed version, such as `01.2`, `1.2.3.4` or `1.2.3+build`, is an error.
   - **Text without interpolation.** A text literal MUST NOT contain `{` or `}` other than as `\{` and `\}`: nothing is interpolated.
2. **No keywords.** No word of these files is a keyword or a reserved word: `from`, `in`, `trusted` and `profile` are words of the grammar where it names them. No word of these files becomes a keyword of a module.
3. **Nothing runs.** There are no expressions, no interpolation, no conditions, no includes and no environment variables, and nothing else runs when a package is read, checked or built: there are no install scripts, build scripts or hooks of any kind.
4. **Bounds.** `ita.una` and `itaty.una` hold at most 1 MiB, and `ita.lock` at most 16 MiB; past that, the file is refused whole, at its start.

## 2. `ita.una`

1. **The package.** A package of edition `0.4` is a directory with `ita.una` and `src/`; its modules are `src/**/*.una`, as in `spec/13 §7`. A directory with both `ita.una` and `una.toml` is U7008 at `ita.una`, so a package never has two manifests that could disagree.
2. **Grammar.**

   ```ebnf
   manifest     = head { section } ;
   head         = "ita" pkg_name NEWLINE INDENT head_line { head_line } DEDENT ;
   head_line    = "version" version_lit NEWLINE
                | "edition" version_lit NEWLINE
                | "profile" ( "app" | "systems" ) NEWLINE
                | "about" text_lit NEWLINE
                | "license" text_lit NEWLINE
                | "home" text_lit NEWLINE
                | "publish" "to" lower_ident NEWLINE ;
   section      = sources | needs | replace | trust | labels | trusted | escapes
                | boundaries | contracts | run | drive ;
   sources      = "sources" NEWLINE INDENT source { source } DEDENT ;
   source       = lower_ident text_lit NEWLINE INDENT "root" text_lit NEWLINE
                  [ "owners" lower_ident { "," lower_ident } NEWLINE ] DEDENT ;
   needs        = "needs" NEWLINE INDENT need { need } DEDENT ;
   need         = pkg_name [ version_lit ] [ "as" lower_ident ] [ origin ] NEWLINE
                  [ INDENT need_line { need_line } DEDENT ] ;
   pkg_name     = lower_ident [ "/" lower_ident ] ;
   origin       = "from" ( lower_ident | "path" text_lit | "workspace" ) ;
   need_line    = "may" cap_type { "," cap_type } NEWLINE
                | "avoid" version_lit NEWLINE
                | "accept" "name" NEWLINE ;
   cap_type     = qualified_upper "[" lower_ident { "," lower_ident } "]" ;
   qualified_upper = { lower_ident "." } upper_ident ;
   replace      = "replace" NEWLINE INDENT
                  { pkg_name version_lit "with" origin NEWLINE } DEDENT ;
   trust        = "trust" NEWLINE INDENT { trust_line } DEDENT ;
   trust_line   = pkg_name [ major ] [ "from" lower_ident ]
                  "modules" module_path { "," module_path } NEWLINE
                  [ INDENT { "may" cap_type { "," cap_type } NEWLINE } DEDENT ] ;
   major        = int_lit | version_lit ;
   labels       = "labels" NEWLINE INDENT { upper_ident NEWLINE } DEDENT ;
   trusted      = "trusted" NEWLINE INDENT { module_path NEWLINE } DEDENT ;
   escapes      = "escapes" NEWLINE INDENT "checked" "per" "module" int_lit NEWLINE DEDENT ;
   boundaries   = "boundaries" NEWLINE INDENT { module_path NEWLINE
                  [ INDENT { "may" cap_type { "," cap_type } NEWLINE } DEDENT ] } DEDENT ;
   contracts    = "contracts" NEWLINE INDENT { module_path NEWLINE
                  INDENT uses_item { uses_item } DEDENT } DEDENT ;
   run          = "run" NEWLINE INDENT [ "entry" module_path NEWLINE ]
                  [ "grants" NEWLINE INDENT grant { grant } DEDENT ] DEDENT ;
   grant        = ( "console" | "clock" | "env"
                  | "keys" text_lit
                  | "files" text_lit file_mode { "," file_mode }
                  | "http" text_lit [ http_method { "," http_method } ] ) NEWLINE
                  [ INDENT limit_line { limit_line } DEDENT ] ;
   file_mode    = "read" | "write" ;
   http_method  = "get" | "post" | "put" | "delete" ;
   drive        = "drive" NEWLINE INDENT "module" module_path NEWLINE
                  "ticks" int_lit NEWLINE [ "budget" int_lit NEWLINE ] DEDENT ;
   ```

3. **Closed.** Every section and every line has one of the forms above; anything else is U7008 at the line. The head's lines appear at most once each, `version` and `edition` required. Each section appears at most once, in the order of the production `section`; a repeated or misplaced section is U7008. There is no `foreign` section and no foreign grant, and no `git` origin: each is U7008.
4. **The head.**
   - `ita <name>`: `name` is a `lower_ident` with no `__` and no trailing `_`, of at most 64 bytes, or `owner/name` with an owner of 2 to 39 bytes by the same rule.
   - `version` is a full version, `MAJOR.MINOR.PATCH` with an optional pre-release, each numeric part at most 2^31 − 1, and no build metadata.
   - `edition` is `0.4` or a later edition the release serves. An earlier edition is U7008: that package keeps `una.toml`.
   - `profile` is `app`, the default. `systems` is U7008 in this release, which does not implement profiles: checking a `systems` package as `app` would leave out the obligations that profile adds.
   - `about` and `license` are non-empty texts, `home` an `https://` URL, and `publish to` names `itaty` or a source of `sources`.
5. **The sections that `una.toml` has.** Each has the meaning and the checks of `spec/13 §7` and of the records it names. Every error that `spec/13 §7` makes U7002 for a key of `una.toml` is U7008 at the corresponding line of `ita.una`; the codes of other domains stay (U1005 and U1002 for categories, U7005 for trusted modules).
   - `labels`: the categories, one per line.
   - `trusted`: the trusted modules, one per line.
   - `escapes`: `checked per module <n>`, `[escape_budget] checked_per_module`.
   - `boundaries` (ADR 0093): a module path, then its ceiling as `may` lines; no `may` line is the empty ceiling.
   - `contracts` (ADR 0092 rule 8): a function's qualified name, then its locked `uses` clauses, one per line, each parsed by the production a header uses.
   - `run`: `entry`, then `grants`, one grant per line, each the grant spec of `spec/13 §5` it corresponds to: `console`, `clock`, `env`, `keys "<file>"`, `files "<path>" read, write`, and `http "<origin>" get, post`. Paths resolve against the package root.
   - The limit lines under a grant (ADR 0117) and the `drive` section (ADR 0116) are U7008 in this release, which implements neither: a grant read without its limit would allow more than written, and a driver section would be ignored.
6. **Canonical form.** `una fmt ita.una` and `una fmt itaty.una` rewrite the file in its canonical form: two spaces per level; one blank line between top-level blocks and none inside them; the head's lines in the order of the production `head_line`; one space between words, none before `,`, `.`, `:`, `/` or `]`, none after `[`, `.` or `/`, and none between a capability and its `[`; a comment kept before the line it precedes, or two spaces after the line it ends. A line whose tokens this spacing would join into others, such as `1.` before `2`, keeps one space between every two tokens. `una fmt --check ita.una` reports U7001 where the file differs. `una fmt` of a package directory does not touch `ita.una`, and `una fix` never edits `ita.una`, `ita.lock` or `itaty.una`.

## 3. Needs and sources

1. **`needs`** names the packages the package depends on.
   - **The requirement** is a version of two or three parts, the least version that the need admits: `json 2.1` admits `2.1.0` and later on the line `json 2`, `json 0.3` admits `0.3.0` and later on the line `json 0.3`. A pre-release follows a full version. It is required, except with `from path` and `from workspace`, where it MUST NOT be written: the dependency's own `version` is used.
   - **The local name** is the `as` name, or the last segment of the package's name. Two needs with one local name are U7008. A local name, and the last segment of the name of any package, MUST NOT be a reserved name or have the skeleton of one (U7008).
     - The reserved names are the keywords and reserved words of `spec/01 §5` of every edition, `std`, `prelude`, `una`, `ita`, `itaty`, `itaoca`, `itauna`, `path`, `git`, `workspace`, and the host capabilities' names in lower case: `console`, `clock`, `env`, `files`, `keys`, `http`.
     - A name's skeleton removes `_`, maps `0` to `o`; `1` and `i` to `l`; `2` to `z`; `3` to `e`; `4` to `a`; `5` to `s`; `7` to `t`; `8` to `b`; `9` to `g`; then `rn` to `m` and `vv` to `w`, and collapses each run of one letter. So `std_` and `s7d` are refused as `std` is.
   - **`from`** gives the origin: `itaty`, the default, predeclared; a source of `sources`; `path "<dir>"`, a directory relative to the package; or `workspace`. Any other source is U7008, and there are no git dependencies.
   - **`may`** lines write the ceiling that the package grants the dependency at its calls (ADR 0093 rule 1). An operation named twice, or a capability named twice in one ceiling, is U7008.
   - **`avoid`** lines name full versions that resolution never selects.
   - **`accept name`** is written only under a path need (U7008 otherwise).
   - **One identity once.** A package needed twice on one major line from one source is U7008.
2. **`sources`** declares a registry other than `itaty`: its name, its `https://` base URL, the digest of its root metadata, `sha256:` and 64 lowercase hexadecimal digits, and the owners it serves. A source's name is not `itaty`, `path`, `git` or `workspace`; two sources with one name, or two sources that list one owner, are U7008. A need of a name whose owner a source lists names that source with `from` (U7008 otherwise), so a private owner's package never resolves at another registry.
3. **`replace`** takes every package of a name at a full version from another origin; `trust` consents to a dependency's trusted modules by identity: its name, its major line (`2`, or `0.3` for the line `0.3`), and its source. The major line and the source may be left out only for a package the root needs directly from a registry (U7008 otherwise).
4. **This release loads no dependency.** Each need is read and checked by the rules above, and then refused, U7008 at the need: no import can reach a package that is not checked with the program.

## 4. `ita.lock`

1. **Who writes it.** Only `una ita` and `una migrate` write `ita.lock`, through a temporary file and a rename. Every other command only reads it, and `una fix` and `una fmt` never edit it. It is committed with the package.
2. **Grammar.** The lexical rules of §1.

   ```ebnf
   lock        = "lock" int_lit NEWLINE solver { registry } { package } ;
   solver      = "solver" NEWLINE INDENT
                 "encoding" text_lit NEWLINE "name" text_lit NEWLINE
                 "version" text_lit NEWLINE "logic" text_lit NEWLINE
                 "steps" int_lit NEWLINE "seed" int_lit NEWLINE DEDENT ;
   registry    = "registry" lower_ident NEWLINE INDENT
                 "root" text_lit NEWLINE "checkpoint" int_lit text_lit NEWLINE DEDENT ;
   package     = "package" pkg_name version_lit NEWLINE INDENT
                 "from" lock_origin NEWLINE
                 "digest" text_lit NEWLINE
                 [ "signed" text_lit NEWLINE "logged" int_lit NEWLINE ]
                 [ "replaced" NEWLINE ]
                 { "needs" lower_ident pkg_name version_lit NEWLINE }
                 { "granted" "by" pkg_name [ major "from" lower_ident ] "may" cap_type { "," cap_type } NEWLINE }
                 [ "consented" text_lit NEWLINE ]
                 { "trusted" "modules" module_path { "," module_path } NEWLINE }
                 DEDENT ;
   lock_origin = lower_ident | "path" text_lit | "workspace" text_lit ;
   ```

   - `lock` is the format's version, `1`; a release reads every format version it ever wrote.
   - **The solver section** has the six keys of `spec/12 §9`, in this order and with the same values: `encoding "una-smt-1"`, `name "z3"`, a non-empty `version`, `logic "QF_UFLIA"`, `steps` of at least 1, and `seed`. A solver of another version stays U5003.
   - **Canonical.** Packages are sorted by name, then version, each once, and a package's `needs` lines by name. Every digest is `sha256:` and 64 lowercase hexadecimal digits. A package has both `signed` and `logged`, or neither. A `git` origin is an error.
   - Every problem of the file is U7009 at its line.
3. **What reading checks.** Every command that loads the package checks, with no network:
   - **the lock** exists and parses (U7009 at the start of `ita.una` when it is missing, with the repair `una ita lock`);
   - **consistency:** every need of `ita.una`, and every `needs` line of every locked package, is met by a locked package of its name on its major line at a version at least the one required; each package is locked once on its major line; and every locked package is reachable from the root by those needs. Each problem is U7009, at the need or at the package line, with the repair `una ita lock`. The walk costs O((n + p + r) log p) for n needs, p packages and r `needs` lines.
4. **`una ita lock [path]`** writes the lock of the package at `path`, `.` by default. The solver section is the existing lock's, or the release's default when the package has none yet. An invalid `ita.una` or `ita.lock` is reported and nothing is written, so a pin is never replaced silently. In this release a package with `needs` is refused (§3 rule 4). `una ita lock --check` writes nothing, and reports where the lock is missing or not consistent (U7009).

## 5. Workspaces: `itaty.una`

1. **Grammar.**

   ```ebnf
   workspace = "itaty" NEWLINE INDENT "members" NEWLINE INDENT { text_lit NEWLINE } DEDENT DEDENT
               [ sources ] [ replace ] [ trust ] ;
   ```

2. Each member is a directory relative to the workspace, without `.` or `..` segments, listed once. The `sources`, `replace` and `trust` sections have the forms of §2 and §3; a trust line of `itaty.una` names the package's major line and source. Every problem is U7008. The members' loading, with one lock for the workspace, joins this chapter with the rules that load dependencies.

## 6. Commands

1. **`una ita`.** The package manager is the command `una ita` of the `una` binary. The same binary invoked under the name `ita` (its `argv[0]` basename, `ita` or `ita.exe`) behaves as `una ita`: `ita lock` is `una ita lock`. With `--json`, each command prints one document, `una.ita/1`: `schema` first, then `command`, `written` (the files it wrote) and `diagnostics`. The exit code is 1 when a diagnostic is an error, and 2 for a usage error, such as a directory without `ita.una`.
2. **Commands of this release.** `una ita lock [--check] [path]` (§4 rule 4).
3. **No network, no other program.** No command of this release opens a connection, and Itá never starts another program.

## 7. The proof cache

For a package of edition `0.4`, the key of a proof cache entry (`spec/12 §9`) is the hex SHA-256 of the domain tag `una-proof-cache/2`, then, for each of the canonical text of the lock's `solver` section and the query, its length in decimal, a NUL byte, and the text. It does not cover the packages of the lock, so adding a dependency invalidates no entry, and the tag and the length prefixes keep it apart from the key of editions `0.1` to `0.3`. The entry's HMAC of ADR 0064 authenticates it either way. The build report's lock digest is the SHA-256 of `ita.lock` as written.

## 8. Migration: `una migrate`

1. **Edition 0.4 is not written in `una.toml`.** `una.toml` naming `edition = "0.4"` is U7002 at the value, with the repair `una migrate`; the package is still checked in edition `0.4`. No released `una` accepted that form, so this changes no released form (ADR 0122 Question 8.1).
2. **`una migrate [path] [--dry-run] [--name <name>]`** moves the package at `path`, `.` by default, from `una.toml` and `verification.lock`, of any edition from `0.1` to `0.4`, to `ita.una` and `ita.lock` of edition `0.4`, and nothing else: a directory that already has `ita.una`, or that is not a package with `una.toml` and `src/`, is a usage error (exit code 2).
3. **What it maps.**

   | `una.toml` / `verification.lock` | `ita.una` / `ita.lock` |
   | --- | --- |
   | `[package] name = "events-example"` | `ita events_example`: `-` becomes `_`; a name that is still not a package name needs `--name` |
   | `version`, `edition` | head lines; `edition` becomes `0.4`; a version that is not `MAJOR.MINOR.PATCH` is refused |
   | `[labels] categories` | `labels`, one per line |
   | `[trusted] modules` | `trusted` |
   | `[escape_budget] checked_per_module` | `escapes`, `checked per module <n>`, when not 0 |
   | `[boundaries]` | `boundaries`, with `may` lines |
   | `[contracts]` | `contracts`, the clause strings as lines |
   | `[run] entry`, `grants` | `run`, `entry`, `grants` with one line per spec, as `files "data/" read, write` |
   | `verification.lock` | the `solver` section of `ita.lock`; the release's default when the package has none |

   A text is written with `"`, `\`, `{` and `}` escaped, and every control or bidirectional character as `\u{…}`.
4. **Comments.** A run of comment lines of `una.toml` directly before a table header or a key is carried, as `--` comments, above the line it becomes. Every other comment is listed in the output, with its line.
5. **Refusals.** An invalid `una.toml`, other than its edition `0.4`, or an invalid `verification.lock` is refused, with each problem printed, and nothing is written (exit code 2).
6. **Writing.** It writes `ita.una` and `ita.lock`, each through a temporary file and a rename, then removes `una.toml` and `verification.lock`.
7. **The clashes of the edition.** It then checks the package at tier 0 in edition `0.4`, and prints every diagnostic the change of edition causes, those the package has in edition `0.4` and not in its own, with their repairs, such as a function named `within` (U0103). It applies none: `una fix` does that. Its exit code is 0: the diagnostics are the next steps of the migration.
8. **Output.** `--dry-run` prints the two files and the diagnostics, and writes nothing. `--json` prints one document, `una.migrate/1`: `schema` first, then `dry_run`, `written`, `removed`, `name` (`from`, `to`), `edition` (`from`, `to`), `default_lock`, `comments_not_carried` (`line`, `text`), `ita_una` and `ita_lock` (the files' texts with `--dry-run`, `null` otherwise), and `diagnostics`.
