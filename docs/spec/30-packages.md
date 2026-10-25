# 30. Packages and dependencies

Status: Normative, edition 0.4 preview (ADR 0122).

From edition `0.4`, a package describes itself in `ita.una`, a declarative file in Una's lexical syntax, and is pinned by `ita.lock`, which only the tool writes. A repository may group packages in a workspace with `itaty.una`. Itá, the package manager, is the command `una ita` of the `una` binary. The rules of ADR 0122 join this chapter as they land, each with its conformance cases. A package loads its path dependencies (§12) and the releases of the sources its `sources` section declares, verified by the registry client (§14); every other need is refused (U7008), so no import ever reaches a package that is not checked with the program.

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
   - The limit lines under a grant (ADR 0117) are U7008 in this release, which does not implement them: a grant read without its limit would allow more than written. The `drive` section names the module the host drives (`spec/26 §2`): `module`, `ticks` and an optional `budget`, each of `ticks` and `budget` from 1 to 86,400,000 milliseconds; a missing key, a value out of range, a `run entry` beside it and a module that is not in the package are U7008.
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
3. **`replace`** takes every package of a name at a full version from another origin; `trust` consents to a dependency's trusted modules by identity: its name, its major line (`2`, or `0.3` for the line `0.3`), and its source. The major line and the source may be left out only for a package the root needs directly (U7008 otherwise); they are then those of that need (§13).
4. **What this release loads.** Path dependencies, by §12, and the releases of the sources of `sources` that the lock pins and that load verified, by §14 rule 10. A need of `itaty`, whose root metadata is not built into this release, or of a workspace member is refused, U7008 at the need, so no import reaches a package that is not checked with the program. `una ita` reports a need of `itaty` as U7013 (§10 rule 7).

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
   - **presence, integrity and provenance** of each registry package, as §9 rule 9 states (U7011, U7010).
4. **`una ita lock [path]`** resolves the package at `path`, `.` by default, as §10 states, and writes its lock. The solver section is the existing lock's, or the release's default when the package has none yet. An invalid `ita.una` or `ita.lock`, or a need with no resolution (U7013), is reported and nothing is written, so a pin is never replaced silently. `una ita lock --check` writes nothing, and reports where the lock is missing or not the one `una ita lock` writes (U7009). `--dry-run` prints the review of §10 rule 8 and writes nothing.

## 5. Workspaces: `itaty.una`

1. **Grammar.**

   ```ebnf
   workspace = "itaty" NEWLINE INDENT "members" NEWLINE INDENT { text_lit NEWLINE } DEDENT DEDENT
               [ sources ] [ replace ] [ trust ] ;
   ```

2. Each member is a directory relative to the workspace, without `.` or `..` segments, listed once. The `sources`, `replace` and `trust` sections have the forms of §2 and §3; a trust line of `itaty.una` names the package's major line and source. Every problem is U7008. The members' loading, with one lock for the workspace, joins this chapter with the rules that load dependencies.

## 6. Commands

1. **`una ita`.** The package manager is the command `una ita` of the `una` binary. The same binary invoked under the name `ita` (its `argv[0]` basename, `ita` or `ita.exe`) behaves as `una ita`: `ita lock` is `una ita lock`. With `--json`, each command prints one document, `una.ita/1`: `schema` first, then `command`, `written` (the files it wrote) and `diagnostics`; a command that resolves (`lock`, `update`, `add`, `remove`) also gives `changes`, the review of §10 rule 8, after `written`, and `tree` gives `root` and `packages` (§12 rule 9). The exit code is 1 when a diagnostic is an error, and 2 for a usage error, such as a directory without `ita.una`.
2. **Commands of this release.** `una ita lock [--check] [--dry-run] [--accept-authority] [--accept-trust <package>] [--accept-key <package>] [path]` (§4 rule 4, §10 rules 7 and 8, §12 rule 6, §13 rule 2, §14 rule 8); `una ita update [<name>…] [--cooldown <hours>] [--now] [--dry-run] [--accept-authority] [--accept-trust <package>] [--accept-key <package>]` and `una ita add <name> [<version>] [--as <local>] [--from <source>] [--now] [--dry-run]`, on the package in the current directory (§10 rules 9 and 10); `una ita tree [path]` and `una ita remove <name> [path]` (§12 rule 9); `una ita fetch [path]` (§14 rule 9); `una ita vendor [path]` (§9 rule 8); `una ita verify [path]` (§10 rule 11); `una ita publish [path]`, `una ita yank <version> [path]`, `una ita audit [path]`, `una ita watch <owner> [<name>…] [--from <source>]`, `una ita bundle <file> [path]` and `una ita unbundle <file> [path]` (§15). `una ita key` joins later.
3. **No network, no other program.** No command of this release opens a network connection: a registry is read from a local mirror directory (§14 rule 1), a submission to an `https://` source stops before anything is sent (§15 rule 1), and a bundle goes through a file. Itá never starts another program.

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
## 9. Package content, digest and archive

1. **What a package holds.** Exactly: `ita.una`; every `src/**/*.una`; `README.md`, `CHANGELOG.md`, and files named `LICENSE` or starting with `LICENSE-` or `LICENSE.`. Nothing else: no library, archive or source file of another language, since Una admits no foreign code, and not `ita.lock`. Every path:
   - is relative, of segments `[A-Za-z0-9_.-]+`, none `.` or `..`, none starting with `.`, none ending with `.`, at most 32 segments and 255 bytes;
   - has no segment that is a reserved device name of Windows, with or without an extension, compared ignoring case: `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`;
   - is unique ignoring ASCII case, and is not, ignoring case, a directory prefix of another path (`a` beside `a/b`);
   - names a regular file; no mode bits are kept.

   A package holds at most 10,000 files and 256 MiB.
2. **The listing and the content digest.** The listing of a package is, for each file in ascending byte order of its path: the path's bytes, a NUL byte, the file's length in decimal, a NUL byte, the lowercase hex SHA-256 of its contents, and a line feed. The content digest is `sha256:` followed by the hex SHA-256 of the listing. It depends only on the files' paths and bytes.
3. **The archive.** A package travels as one file: the line `ita-archive/1`, the listing's length in decimal and a line feed, the listing, then each file's bytes in listing order, and nothing after. There is no compression. The reader reads the archive once into memory and parses and hashes those same bytes; it refuses another first line, a length with a leading zero, above 256 MiB or past the end, a listing whose paths are not in strictly ascending byte order or that rule 1 refuses, a file whose hash differs from the listing's, and trailing bytes.
4. **Costs.** The listing sorts the files, O(f log f); the digest and the reader are one pass over the bytes, O(B), with memory O(listing + archive).
5. **Itaoca.** Fetched packages are stored in `packages/sha256/<hex>/` under `~/.una/itaoca`, or the directory `UNA_ITAOCA` names when it is set and not empty; that directory itself MUST NOT be a symbolic link. A package is written once, from the bytes of its archive as rule 3 read and hashed them, into a fresh private directory:
   - the directory has a name of its own, is created with `mkdir`, which fails when anything is there, and has mode 0700; each directory below it is created the same way, and each file with `O_CREAT`, `O_EXCL` and `O_NOFOLLOW`;
   - each file's permissions are set through its open descriptor (read-only in Itaoca), never by path, and each file and each directory is synced;
   - the fresh directory MUST still be the one created, by the device and inode of its descriptor, before each file is written and before it is renamed into place; otherwise the write fails, and a write that fails removes the fresh directory only while it is still the one created, and leaves it otherwise.

   What is written is then loaded as rule 6 states. The provenance of a release at a resolution point (rule 7) is stored once in `registry/<source>/releases/<release>/checkpoint-<size>.provenance`, where `<release>` is the release's directory name of rule 8 and `<size>` the checkpoint's size, written as one file in a fresh private directory and renamed into place; another document for the same release and point is refused.
6. **Reading.** A package is read from Itaoca or a vendor directory, on every load and never once for all:
   - every component below the store's or the root's directory, which is resolved once, MUST be a directory and not a symbolic link;
   - the package is listed: every entry is a directory or a regular file with one link; a symbolic or hard link, a device, a FIFO or a socket refuses the package, and so does every path that rule 1 refuses, before any byte of it is read; at most 20,000 entries are listed;
   - each listed file is read once into memory, through a descriptor opened with `O_NOFOLLOW`, and on macOS also `O_NOFOLLOW_ANY`, which MUST be the entry listed, by device and inode;
   - the digest of the bytes read MUST be the lock's before any of them is used, and every later step (the provenance's check, vendoring, checking and building) MUST use those bytes and MUST NOT read a file of the package by its path again.

   So an entry swapped while the package is read can only make the load fail.
   A path dependency's files (§12 rule 1) are each opened with `O_NOFOLLOW` and checked to be the regular file listed. On a platform whose flag values `una` does not know, no package is read or written. Any failure is U7010; nothing is repaired.
7. **Provenance.** Beside each registry package, in Itaoca (rule 5) or the vendor directory (rule 8), its provenance is a document of these parts, in this order: the line `ita-provenance/1`; one to 64 blocks `root <length>`, each a line followed by that many bytes of root metadata (§11 rule 8), the version that the source's pin names first and each next version after it; a block `checkpoint <length>`, the checkpoint of the lock's resolution point with its signature and cosignatures (§11 rule 4); a block `statement <length>`, the release statement (§11 rule 6), whose bytes are its log leaf; `index <n>`, the statement's log index; zero to 64 lines `proof <base64 hash>`, its inclusion proof; and `key <name> <base64 key>`, the publisher key the lock names. Lengths and the index are decimals without a leading zero, each block at most 64 KiB, the whole at most 1 MiB, and nothing follows the key line; a document that is not in this one encoding is refused. It verifies, offline and with nothing taken on the lock's word, when:
   - the first root metadata has the digest the lock records for the source, which MUST be the root that `ita.una`'s `sources` pins, and the threshold of its own root keys; each next version meets §11 rule 8; and the last has not expired at the checkpoint's witness time (§11 rule 5);
   - the checkpoint's size and root hash, `sha256:` and its hex, are the lock's `checkpoint` line; its origin is the name of the root's log key; it is signed by that key; and it is cosigned by at least the witness threshold of distinct witnesses of the root;
   - the statement names the lock's source, name and version and gives the lock's digest; its `manifest` is the digest of the package's `ita.una`, `sha256:` and the hex SHA-256 of its bytes; it is signed by the key stored, whose `signed` form is the lock's: `ed25519:`, the name's owner (`itauna` for a name without one), `:`, and the hex SHA-256 of the public key; and its leaf is included in the checkpoint at the lock's `logged` index.

   The owner's key chain and the per-name state at the resolution point (ADR 0122 rules 36.1 and 36.3) are verified by the registry client when the release is resolved and when it is fetched (§14), with the key the statement is signed by valid at its log position; they are not stored beside the package in this release, so loading verifies the provenance above and does not walk them again. A package of `itaty`, whose root metadata is built into `una` and is not in this release, does not verify.
8. **Vendoring.** `una ita vendor [path]` copies each registry package of the lock from Itaoca into `vendor/ita/<source>/<release>/` of the package at `path`, `.` by default, where `<release>` is `<owner>+<name>-<version>`, or `<name>-<version>` for a name without an owner, with its provenance beside it in `vendor/ita/<source>/<release>.provenance`. The copy is written from the verified bytes of Itaoca's copy (rule 6) into a fresh private directory as rule 5 writes, with its files writable, renamed into place, and loaded as every command loads it (rule 9); the provenance is written the same way, as one file. A symbolic link at `vendor`, `vendor/ita` or `vendor/ita/<source>` refuses the copy (U7010), and nothing is written through it. A copy already there that verifies is kept; one whose files verify and whose provenance is missing or does not verify takes Itaoca's provenance; one whose files do not verify is U7010 and is left as it is. A package Itaoca does not hold is U7011. Path dependencies are not copied: they are read from their directory. An invalid `ita.una` or `ita.lock` is reported and nothing is written. With `--json` it prints one `una.ita/1` document whose `written` lists the directories and files written, relative to the package.
9. **What loading checks.** Every command that loads a package of edition `0.4` whose lock reads checks each registry package of the lock, with no network, at its line of `ita.lock`:
   - **presence:** it is in the vendor directory or in Itaoca; otherwise U7011. The commands that check, run, build, test or report a package, and the language server, look in Itaoca; the others look in the vendor directory only;
   - **integrity and provenance:** read from the vendor directory when it holds the package, else from Itaoca, it meets rules 6 and 7; otherwise U7010. A vendored copy that fails is not replaced by Itaoca's.

   A package of `itaty` is not checked in this release: its root is not built in yet, so it cannot verify, and its need is refused (§3 rule 4), so it never loads. The check applies to it once that root lands. A release that loads verified and that a need of the root names is mounted (§14 rule 10).

   So a repository with its vendor directory and lock committed builds with `una` alone, and a change that edits a vendored package together with its digest, key and checkpoint does not load: it would need the root's, the log's and the witnesses' keys.
10. **No connection.** `una check`, `una run`, `una build`, `una test`, `una report`, `una core`, `una lsp`, `una migrate`, `una fmt` and `una fix` never open a connection for a package or read a registry; only `una ita lock`, `update`, `add` and `fetch` read one, from a mirror directory (§14). A program whose locked packages are present gives the same result with or without a network. Only a program that `una run` grants `http` reaches the network, through its `Http` capability (`spec/24`).

## 10. Versions and resolution

1. **Versions.** Semantic versioning 2.0.0 without build metadata, each numeric part at most 2^31 − 1, ordered by semver precedence. The identity of a package is its source, its name and its major line: `MAJOR` when it is at least 1, `0.MINOR` otherwise. Two major lines of one package are two identities, which may both be in one program.
2. **Requirements.** A need names the least version it admits on its line. A pre-release is selected only if some requirement names a pre-release of that identity. There are no upper bounds and no ranges.
3. **Minimal version selection.** The build list holds, for each identity reachable from the root, the highest of the versions the reached needs ask for, each read as the least release at or above it; never a newer one. The result does not depend on the order of any list, and a release published later changes nothing until a need asks for it.
4. **Raises.** A selection that is yanked, named by an advisory not fixed in it, or avoided by the root is raised to the least release of its identity above it that is none of these, is not a pre-release unless one is named, and was logged at least the cooldown before the resolution point, 72 hours of witness time by default. An advisory signed by the threshold that names the fix by digest lifts the cooldown for that fix. With no such release, resolution fails, naming the newest candidate the cooldown holds back. The walk resumes with the needs of the release taken.
5. **Bounds.** 100,000 releases visited, 10,000 packages in the list, 1,000 needs per release, 10,000 releases read per name; a release that needs its own identity is refused.
6. **Costs.** O(N + R) time and space for N releases reached and R need edges, with hash maps; O(log K + S) per raise for an identity of K releases of which S are skipped; O(P log P) to sort the list. There is no backtracking and no search.

7. **What resolution reads.** `una ita` resolves over each path dependency, as its directory holds it, the one release of its identity, whose source is its directory; and over the releases of each source of `sources` that the root needs, read from the registry at a fresh resolution point and verified (§14 rules 2 to 7), each name's whole chain included, so nothing is ever resolved against a partial view of a registry, which could lack a lower release that rule 3 takes or a yank that rule 4 raises past. A need of `itaty`, whose root metadata is not built into this release, is U7013 at the need, and the source is refused as a whole. A need of a workspace member is U7008. An `avoid` line naming the version a path dependency holds has no later release to raise to (U7013). Every bound of rule 5 is U7013, the root's needs counted as a release's. The walk is rule 6's, and the lock's packages are sorted in O(p log p).
8. **The review and consent** (ADR 0122 rule 25). Before a command writes a lock that differs from the one it replaces, it prints one line per package added, changed or dropped, compared by name and origin: `add util 0.2.0 (path deps/util): digest sha256:…; may Console[print]; uses Console[print]`, with `old -> new` for a version, digest or ceiling that changed, and `drop util 0.2.0 (path deps/util)`. `uses` is what the package's code uses in the new graph, checked at tier 0: the union of the `uses` of its functions and handlers. With `--json`, each line is an entry of `changes`: `change` (`add`, `change` or `drop`), `package`, `origin`, `version_from`, `version_to`, `digest_from`, `digest_to`, `ceiling_from`, `ceiling_to` and `authority`. Refused without a flag, and nothing written: a ceiling wider than the one the lock records (`--accept-authority`, U7009) and a trust that no recorded consent covers for the package's content as it is now (`--accept-trust <package>`, U7012). `una ita add` and `una ita remove` take neither flag: their refusals are repaired by `una ita lock` with it.
9. **`una ita update [<name>…]`** raises the named needs, or all, and writes the lock. A name the root does not need, or an `ita.una` that does not parse, is a usage error. `--cooldown <hours>` sets the cooldown of rule 4, 72 by default, and `--now` sets none; the two are one choice. A path dependency is always locked as its directory holds it, so `update` locks it again. A registry need is raised in `ita.una` to the newest release of its identity that is not a pre-release (unless the need names one), not yanked, avoided or signed after its key's stated compromise, and logged at least the cooldown before the resolution point; `update` never changes a major line, and writes `ita.una` and the lock only when the resolution succeeds. Rule 26 of ADR 0122, that a release needs no authority or trust the highest earlier release of its identity did not, is checked from the release statements (§14 rule 6 and rule 8); for a path dependency, what it may exercise stays bounded by the ceiling the lock records, whose widening needs `--accept-authority` (rule 8).
10. **`una ita add <name> [<version>]`** adds the need `<name> <version> [as <local>] [from <source>]` at the end of `needs`, or in a new `needs` section before the first section that follows it in the canonical order, and locks; it writes no `may` and no `trust` line. Each word is checked before it is written, the name as `name` or `owner/name` in lower case, the version as a version of two or three parts, and `--as` and `--from` as lower-case names (a usage error otherwise); the edited `ita.una` is read as any other (U7008), and it is written, then the lock, only when the resolution succeeds. Without a version, it takes the newest release of the newest major line that is not a pre-release, not signed after its key's stated compromise, and logged at least the cooldown before a fresh resolution point, from the registry's verified state (§14), and writes it as a full version; with none, U7013, naming the newest release the cooldown holds back. Without `--from`, a name whose owner a source of `sources` serves is needed from that source, and `add` writes the `from`. A need of `itaty` has no resolution in this release (rule 7), and nothing is written.
11. **`una ita verify [path]`** reads each registry package of the lock again, in the vendor directory and in Itaoca, wherever it is, and verifies each copy as §9 rule 9 states: U7010 for a copy that does not verify, U7011 for a package in neither place. It prints `verified <name> <version> in vendor/ita and Itaoca`, naming where it verified, and writes and repairs nothing. A path dependency is the root author's working tree, which `una ita lock --check` compares.

## 11. Verifying a registry's documents

The registry client verifies what it reads with these rules, each a pure function of the bytes; how it reads a registry and applies them is §14.

1. **The log's tree** is that of RFC 9162 §2.1: a leaf's hash is SHA-256 of `0x00` and the leaf, a node's of `0x01` and its children's hashes. An inclusion proof (§2.1.3.2) and a consistency proof (§2.1.4.2) are verified as RFC 9162 states; a consistency proof from a tree of no leaf is empty, and one between equal sizes needs equal roots. Each costs O(log L) hashes for a log of L leaves.
2. **Ed25519 is verified strictly**, by one rule: a key of 32 bytes and a signature of 64; the public key and `R` canonical encodings (`y` below p) of points not of small order; `S` below the group order; and the cofactorless equation.
3. **A signed note** (C2SP `signed-note`) is a text of lines without control characters, ending with a line feed, a blank line, and lines `— <name> <base64>`, whose base64 is canonical and decodes to a 4-byte key hash and the signature. The key hash of an Ed25519 key is the first four bytes of SHA-256 of its name, a line feed, `0x01` and the key; a note is signed by a key when a line of its name and key hash verifies the text. A note is at most 64 KiB.
4. **A checkpoint** (C2SP `tlog-checkpoint`) is a note whose text is the origin, the size in decimal without a leading zero, the root hash in base64, and non-empty extension lines. **A cosignature/v1** (C2SP `tlog-cosignature`) of a checkpoint by a witness is a signature line whose key hash uses `0x04`, holding an 8-byte big-endian timestamp and a signature of `cosignature/v1`, `time <t>`, each on its line, then the checkpoint's text.
5. **Witness time** (ADR 0122 rule 36.4). With k the witness threshold:
   - an entry is **logged at** the k-th earliest of the distinct witnesses' earliest cosignatures on checkpoints that include it;
   - the **witness time of a checkpoint** is the k-th latest of the distinct witnesses' cosignatures on it;
   - a wait is the witness time of the later checkpoint minus the logged-at time of the entry;
   - a witness whose timestamps decrease as the log grows is refused (U7010);
   - freshness alone compares a witness time with the local clock, and a local clock more than one hour behind the newest witness time is clock skew (U7010).

   Fewer than k witnesses can move none of these times in an attacker's favor.
6. **A release statement** is a signed note whose text is, in this order and each once: `ita-release/1`; `source <source>`; `name <name>`; `version <full version>`; `previous <index> <leaf hash>`, or `previous none` for a name's first entry; `digest <content digest>`; `manifest <digest of its ita.una>`; `edition <edition>`; one `needs <source> <name> <version>` line per need, sorted by name, each of the release's own source; `authority` with the capabilities and operations its public functions and handlers use, as `Http[get], Store[find, put]`, or `none`; and `trust modules <path>, ...`, or `trust none`.
7. **Authority is part of the interface** (rule 26). Within one identity, a release needs no capability, operation or trusted module that the highest earlier release did not; a statement that does is refused, by the registry and by every client.
8. **Root metadata** is a signed note whose text is, in this order: `ita-root/1`; `source <source>`; `version <n>`, from 1; `expires <seconds>`; `root <threshold>` and one `key <name> <base64 key>` line per root key; `log <name> <base64 key>`; `witnesses <threshold>` and a key line per witness; `security <threshold>` and a key line per security key; `itauna <threshold>` and a key line per key of the official set. Each threshold is between 1 and its number of keys, and no name or key appears twice. Its digest, which `sources` pins and the lock records, is `sha256:` and the hex SHA-256 of the whole signed note. The first one a client accepts has the pinned digest and the threshold of its own root keys; a version `n + 1` is accepted only with the thresholds of version `n`'s root keys and of its own. Versions passed may have expired; the last one MUST NOT have, in witness time.
9. **The name map** (rule 36.1) is a sparse Merkle tree of depth 256 over SHA-256 of each name, whose leaf is SHA-256 of `0x00`, the key, the index of the name's latest entry (8 bytes, big-endian), that entry's leaf hash, and its count of entries (8 bytes); an empty subtree of height 0 is 32 zero bytes, and a node and an empty subtree above it are SHA-256 of `0x01` and its two children. A checkpoint carries the map's root in its extension line `ita-map <base64 root>`. A proof of a name's leaf, or of its absence, is a bitmap of 256 bits, set where the sibling at that height is not empty, and those siblings from the leaf up; its size is O(log M) hashes for M names, and checking it takes 256 hashes.
10. **A name's chain.** Each entry about a name names the previous one by its log index and leaf hash, or none. A client walks it from the map's head back to the last entry it holds, each link the previous one's, and the number of links walked MUST equal the map's count minus the count held, or the whole count on a first walk; otherwise U7010. So a map or chain that omits a yank, an advisory, a revocation, a recovery or a transfer is caught.
11. **Yanks and advisories** are entries of a name's chain. A yank is `ita-yank/1`, `source`, `name`, `version` and `previous`, signed by the owner's threshold. An advisory is `ita-advisory/1`, `source`, `name`, `previous`, `id` (1 to 64 letters, digits, `-`, `_` or `.`), `line` (the major line), `affected` (full versions of that line), `fixed <version> <digest>` or `fixed none`, `severity` (`low`, `medium`, `high` or `critical`) and a one-line `summary`; it stands when a security key of the root metadata signs it, and lifts the cooldown for the fix it names by digest only when a second security key or the owner's threshold also signs it. Folded oldest first, a name's entries give each release its digest, its logged-at time, whether it is yanked, the advisories that name it, and whether a lifting advisory names it as its fix; a second statement for one version, a yank or advisory of a version never released, or an advisory no security key signs is U7010. `una ita audit` reports a locked release that is yanked or named by an advisory (U7014).
12. **Publishing** (rule 43) refuses, before building anything, a package without `publish to`, `about` or `license`, with a `sources`, `replace` or `trust` section, or with a need of a path, a workspace member or another source than the one it is published to. Its release statement lists its needs at their least versions, each of its own source, sorted by name.

## 12. Path dependencies in the program

1. **Loading.** Each need `from path "<dir>"` names a directory relative to the package, which holds `ita.una` and `src/`. It is read with no symbolic link followed, in it or below it; a directory that is the package itself, lies inside its `src/`, or is named by two needs is not read. Its `ita.lock`, `run` and `sources` are not read. A dependency that cannot be read is U7008 at its need.
2. **What the root accepts.** A path dependency is used only when:
   - its `ita.una` names the package the need names (U7008 otherwise);
   - the need says `accept name`, since a local package carries a name a registry may hold, and no client can tell offline which owners a registry serves (U7012 otherwise);
   - its edition is the package's or earlier (U7012 otherwise);
   - and, in this release, it has no needs of its own, no labels, no trusted modules, no boundaries and no contracts, and the need's `may` lines name host capabilities only (U7008 at the need otherwise).

   A dependency that is not used is not checked, and its modules are not importable.
3. **Modules.** A dependency's module `m` is the module `<local>.m` of the program, where `<local>` is the need's local name: the root writes `import util.text (shout)`. A dependency's modules import their own modules by their own paths, and the standard modules, never a module of the root (U1001). A module of the root whose first path segment is a local name is U1010. Each module's file path is checked against its own package's `src/` (U1008).
4. **One program.** `una check` checks the root's modules and every used dependency's as one program, under the root's solver pin; a diagnostic in a dependency is reported at its file, and counts as any other. `una fix` never edits a dependency's file, `una fmt` of the package formats the root's modules only, and `una test` runs the root's examples only.
5. **Authority.** Every module of a dependency is in its package's extent, a boundary of ADR 0093 whose ceiling is the one the root writes with `may` lines under the need, empty without them. A call into it takes no capability by name (U3020); each is granted with `with`, within that ceiling (U3021), and no release may be reachable (U3024). A dependency's examples are checked as its own, with no consumer ceiling on their `given` lines, since they never run with the root's authority.
6. **The lock.** `una ita lock` locks each used path dependency as `package <name> <version>`, `from path "<dir>"`, its content digest (§9), and `granted by <root> may <ceiling>` for a non-empty ceiling. Loading checks that each path need has its locked package at the version its `ita.una` gives (U7009), and that every `may` line is within the ceiling the lock records as granted by the root (U7009 at the `may` line). `una ita lock` records a wider ceiling only with `--accept-authority`, so every widening is explicit and shows in the lock's diff; `una fix` never applies it. The digest of a path package is recorded for review and compared by `una ita lock --check`; loading does not compare it, since a path package is the root author's own working tree.
7. **The Core export** of a program with dependencies is `una.core/2`, which this release does not write: `una core` refuses it as a usage error.
8. **Effective ceilings and the report.** The effective ceiling of a package bounds what it can ever exercise: the least fixpoint of `eff(p) = ⋃ over the packages d that need p of (ceiling_d(p) ∩ eff(d))`, computed by a worklist in O((P + R) · C) for P packages, R needs and C capability operations, cycles included (ADR 0122 rule 51). The root may be granted any host capability with `--grant`, so `eff(root)` is taken as every operation, and a direct dependency's effective ceiling is the ceiling the root writes for it: a bound never narrower than what the package can exercise. The report (`spec/13 §6`) holds `dependencies`, after `handler_authority`, one entry per package of the program other than the root, in name order: `package`, `version`, `local`, `source` (`"path"`), `path`, `digest` (the lock's, or `null`), `signed` (`null` for a path package), `replaced`, `edition`, `profile`, `granted_by` (per dependent, its name, its `source`, `null` for the root, and the ceiling it writes as `may`), `effective_ceiling`, `authority` (the union of the `uses` of its functions and handlers) and `trust` (`modules`). A package with no dependencies has `"dependencies": []`. `tcb` gains `registries` (each registry source of the program's mounted releases with the digest of its root metadata that the lock records, §14 rule 10) and `dependency_trust` (§13). The human report prints one line per dependency: `util 0.2.0 (path deps/util): may Console[print] (by demo); uses Console[print]; trust none`.
9. **`una ita tree [path]`** checks the program at tier 0 and prints the root's name and version, then one line per dependency, in the form of the report's human line; with `--json`, one `una.ita/1` document with `schema`, `command` (`"tree"`), `root`, `packages`, the report's `dependencies`, and `diagnostics`. **`una ita remove <name> [path]`** removes the need of `name` from `ita.una`, with its indented lines, and the `needs` section when it empties, through a temporary file and a rename, then writes the lock as `una ita lock` does. A name the root does not need, or an `ita.una` that does not parse, is a usage error and nothing is written. Neither opens a connection.

## 13. Trust a dependency adds

1. **A dependency's trusted modules** (its `trusted` section and `trusted module` headers) are trusted in the program only if a trust line of the root names the package and each of them by its module path in the package (U7012 at the need otherwise). A trust line names a package the root needs directly by its name alone, its major line and source being the need's; a path package has no source, so a trust line with `from` names a registry package, never a path one.
2. **Consent is pinned to the whole package.** The lock records, for the package, `consented "<digest>"` with its content digest, and `trusted modules ...`. Loading compares the recorded digest with the package's content as it is now, and every trusted module with those recorded; any difference, a change of any file of the package included, is U7012 until `una ita lock --accept-trust <package>` records it again. A policy's `via` may call any function of its package, so pinning the trusted module's file alone would not pin the code it runs.
3. **An extent of its own** (rule 53.1). A consented trusted module `m` of the dependency `<local>` is the program's trusted module `<local>.m`, and an extent of its own inside its package's: calls into it from outside it, the package's other modules included, are crossings, granted with `with` (U3020). A release whose policy it declares is not a release into a boundary, whoever reaches it (no U3024); every other release stays U3024.
4. **One ceiling, for now.** Until a crossing's ceiling depends on its direction, the package's extent and its trusted extents share one ceiling: the intersection of the root's `may` lines for the package and those under its trust line. It is never wider than either, so a program this accepts is one rule 53.1 accepts.
5. **The report** lists each dependency's consented trusted modules under its `trust`, and `tcb.dependency_trust` gives them by package, with their line counts; `tcb.trusted_modules` keeps the root's own.

## 14. The registry client

A registry is a set of static files below a base (ADR 0122 rule 34). `una ita lock`, `update`, `add` and `fetch` read the registries of the sources the root's `sources` section declares, and verify every byte they use with the rules of §11 and of this section before any of it is used.

1. **Where a source is read** (ADR 0122 rule 40). Each source is read from its places, in order: the mirrors that Itaoca's `mirrors.una` lists for it, then its own URL, the one its `sources` entry gives. `mirrors.una` has one line `mirrors`, below it one line per source, and below each source one location in quotes per line:

   ```una
   mirrors
     acme
       "https://ita-mirror.example.net"
       "file:///srv/ita-mirror"
   ```

   A location is an `https://` URL or a `file://` URL of a local directory, which holds a copy of the registry's files as its store keeps them. Plain `http://`, every other scheme and a bare path are refused, and so is a source or a location listed twice, at most 64 mirrors per source and 1 MiB in all: a `mirrors.una` that does not read is U7008, naming its line, and no source is read from it. This release has no transport for an `https://` location: such a place cannot serve, which the diagnostic says. The first place whose root metadata and checkpoint verify (rule 3) is the one the command reads; a place that serves a copy that does not verify is skipped and named, U7010, and one that cannot serve is skipped. With no place that verifies, the source is U7010, naming each place whose copy did not verify, or U7013 when no place could serve, at the first need of the source. Everything a place serves is verified as from the source itself, by the same rules, so a file changed in a mirror makes the command fail (U7010), never take the change. No command starts another program or opens a network connection.
2. **The files of a registry** are those of ADR 0122 rule 34, at the paths a registry serves: `root/<n>`, `checkpoint` and `checkpoint/<size>`, the log's tiles and entry bundles in the C2SP `tlog-tiles` layout under `tile/`, the name map's tiles under `map/`, and archives at `blobs/sha256/<hex>`. A copy of a registry's store in a directory keeps `checkpoint` as the file `checkpoint/latest`, a name no size can have. Every document other than an archive is at most 16 MiB, a note at most 64 KiB.
3. **Opening a source.** The root metadata whose digest is the pin of `sources` is read among `root/1` to `root/64`, accepted by the threshold of its own root keys, and each next version `root/<n + 1>` that exists is walked by §11 rule 8, at most 64. The checkpoint is accepted when its origin is the last root's log key name, it is signed by that key, and at least the root's witness threshold of distinct witnesses cosign it; its witness time is §11 rule 5's. The last root MUST NOT have expired at that time (U7010).
   - **Freshness.** For `lock`, `update` and `add`, the checkpoint is `checkpoint`, and its witness time MUST be less than 24 hours before the local clock; a local clock more than one hour behind it is clock skew. Both are U7010, and nothing is written.
   - **A lock's point.** For `fetch` and for `lock --check`, the checkpoint is the lock's resolution point, `checkpoint/<size>` with the root hash the lock records, and no freshness is needed.
   - **Floors.** Itaoca keeps, in `registry/<source>/checkpoint`, the largest checkpoint of the source accepted; it is replaced only by a larger one. A checkpoint is accepted only with a consistency proof (§11 rule 1) between it and the stored one, computed from the tiles, in either direction; a resolution never accepts one below the stored one or below the resolution point of the lock it replaces. A failure is U7010: it is evidence that the log forked.
   - **Monotonic witnesses.** Across every cosignature the client holds, a witness's timestamps never decrease as the log grows (U7010).
4. **Entries and chains.** An entry is read from its bundle, and its leaf hash, SHA-256 of `0x00` and its bytes, MUST be included in the checkpoint by an inclusion proof computed from the tiles. A name's state is its head in the name map, read from the map's tiles on the name's path from the root of the checkpoint's `ita-map` extension line, each tile checked against the hash that names it, and proven against that root (§11 rule 9), then its whole chain, walked from the head by §11 rule 10, each entry verified in the log. An entry is a signed note whose text is a release statement (§11 rule 6), an owner's entry (rule 5), or a yank or an advisory (§11 rule 11), read as §15 rule 6 states; an entry of any other kind, a transfer included, refuses the name (U7010), so nothing is resolved from a part of a name's state.
5. **Owners' keys** (ADR 0122 rules 36.3, 38 and 38.1). An owner's chain is the chain of the name-map name `owner:<owner>`, which no package name can be, since a package name has no `:`. Its entries are signed notes of two formats. In each, every line after the format line is `<field> <value>`, one form per line, in the order stated, each once unless stated, and nothing after; an entry longer than a note may be (64 KiB) is refused before it is read. Anything else is refused (U7010).
   - **`ita-owner/1`, the registration,** is the chain's first entry: `source <source>`; `owner <owner>`; `previous none`; `threshold <k>`; one `key <name> <base64 key>` line per key, 1 to 32 of them and at least `k`; then, optionally, the owner's recovery cosigners: one `cosigner <name> <base64 key>` line each, 1 to 32, followed by `cosigners <k>`, `k` from 1 to their number. A key's name is 1 to 128 bytes with no space, control character or `+`; no name or key is listed twice. It is signed by every key it registers.

     ```text
     ita-owner/1
     source itaty
     owner acme
     previous none
     threshold 1
     key acme.laptop 47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=
     cosigner trustee.a 9Pxw7lyLbsDgQ2ib5Wb3sTsXdrWmHYcnbq1Qr6eKXCM=
     cosigners 1
     ```

   - **`ita-key/1`, every later event:** `source <source>`; `owner <owner>`; `previous <index> sha256:<hex>`, the owner's previous entry; one action line; then `threshold <k>`, the owner's threshold once the event takes effect, from 1 to 32:

     | Action line | Signed by | `threshold` |
     | --- | --- | --- |
     | `add <name> <base64 key>` | the owner's threshold of current keys, and the key added | 1 to the keys after |
     | `revoke <name>` or `revoke <name> since <p>` | the owner's threshold, or the cosigners' threshold | 1 to the keys after |
     | `cosigners <k> <name> <base64 key> …`, one or more cosigners, `k` at most their number; or `cosigners none` | the owner's threshold | the current one |
     | `recover <name> <base64 key>`: a pending recovery naming the new key | the log's key, and the cosigners' threshold when the owner has cosigners | 1 |
     | `cancel <index>`: of the pending recovery logged at that index | any one current key of the owner | the current one |
     | `overrule <index>`: of the cancellation logged at that index | the cosigners' threshold | the current one |
     | `dispute <name> <base64 key>`: a registry dispute naming the new key | the log's key; only for an owner with no cosigners | 1 |

     ```text
     ita-key/1
     source itaty
     owner acme
     previous 48211 sha256:0b7e3c110b7e3c110b7e3c110b7e3c110b7e3c110b7e3c110b7e3c110b7e3c11
     revoke acme.laptop since 48190
     threshold 1
     ```

   A key's fingerprint is the hex SHA-256 of its public key. A key added, recovered or disputed is never a name or a key the owner holds or held. The client replays the owner's whole chain: an event governs the entries logged after it. A recovery or a dispute takes effect at the first kept checkpoint whose witness time is at least seven days after the witness time of the first checkpoint that includes it, and governs the entries logged after that checkpoint; the new key is then the owner's only key, with a threshold of one, and the change is a key change (rule 8). A recovery that a current key cancelled before it took effect, unless the cosigners overruled the cancellation, never takes effect; a cancellation of a dispute, of a recovery in effect, or of no recovery changes nothing, and a dispute cannot be cancelled. A key revoked `since <p>` is compromised: not valid after the log position `p`. An entry that is not signed as the table states, or whose `threshold` is not the one stated, refuses the chain (U7010). The unscoped names belong to Itaúna, whose keys and threshold are the root metadata's: it has no chain, no recovery and no dispute.
6. **Releases.** A release of a name is a release statement of its chain, of the source and name it is read for, each version once (a second statement of a version is U7010: a release is immutable), signed by the owner's threshold of keys valid at its log position. A statement whose signers reach the threshold only with a key revoked as compromised before its position is skipped by resolution as a yanked release is. A statement that needs a capability, an operation or a trusted module that the highest release of its identity logged before it did not need breaks rule 26 of ADR 0122, and refuses the name (U7010). The key the lock names is the first signer by fingerprint, as `ed25519:<owner>:<fingerprint>`, `itauna` for an unscoped name.
7. **Logged time.** A release is logged at the k-th earliest of the distinct witnesses' earliest cosignatures on the kept checkpoints that include it, k the witness threshold (§11 rule 5). The kept checkpoints are read by size, `checkpoint/<size>` from the entry's index plus one on, each signed by the log's key and a prefix of the resolution point by a consistency proof, until k witnesses are seen, at most 64 of them; past that the resolution point's cosignatures count too, which can only make the time later and a wait longer; the cooldown of §10 rule 4 is measured from it to the witness time of the resolution point, the earliest of the registries' when a program reads several.
8. **Resolution and the lock.** `una ita lock`, `update` and `add` resolve each registry need by §10 over the releases of rule 6, and lock each release of the build list as `package <name> <version>`, `from <source>`, `digest`, `signed`, `logged <index>`, its `needs` lines from its statement, and, for a need of the root with `may` lines, `granted by <root> may <ceiling>`; each source with a release in the lock is recorded as `registry <source>`, `root "<pin>"`, `checkpoint <size> "<root hash>"`. Consent is §10 rule 8's, and for a release with trusted modules, by its statement's `trust`, as §13 states. Beside it:
   - **Keys.** A locked release whose owner's keys changed since the lock it replaces was written, another signing key, or any key event of the owner logged after its resolution point, a recovery or a dispute that took effect included, is U7014 at the need until `--accept-key <package>` records it; nothing is written.
   - **Rule 26 against the lock.** A release that replaces a locked one of its identity and needs more authority or trust than it did is U7010.
   - **The review** of §10 rule 8 gives a registry release's `uses` from its statement, which its archive is checked against when it is fetched (rule 9).
   - **Floors.** The checkpoint of each registry read is stored in Itaoca when the lock is written.
9. **`una ita fetch [path]`** fills Itaoca with each registry release of the lock that Itaoca does not hold with its provenance: read at the lock's resolution point (rule 3), the release statement at the lock's `logged` index with the lock's digest and signed by the key the lock names, valid at its position; the archive at `blobs/sha256/<hex>`, read and hashed once (§9 rule 3); then the statement is checked against the archive (ADR 0122 rule 37): its `manifest` is the digest of the archive's `ita.una`, which names its package, version and edition; its needs are the archive's, each of the release's own source; its `trust` is the archive's trusted modules; and what the archive's code uses, the union of the `uses` of its functions and handlers, checked at tier 0 from the bytes verified, is within its `authority`. The package is then stored with its provenance (§9 rules 5 and 7) and loaded again as every command loads it. Any failure is U7010, and nothing of that release is stored; a source with no mirror is U7013. It prints `fetched <name> <version>`, or `<name> <version> is in Itaoca` for one held already.
10. **Loading.** A registry release that loads verified (§9 rule 9) and that a need of the root names, by its name, its source and its major line, is mounted as a path dependency is (§12 rules 3 to 5), its files the bytes verified, named in diagnostics below `vendor/ita/<source>/<release>/` or `itaoca/<source>/<release>/`. §12 rule 2 applies to it but `accept name`, which a registry package does not need; in this release a registry release with trusted modules is not mounted either (U7008 at the need). The report gives it with its `source`, `null` for `path`, the lock's `digest` and `signed`, and `tcb.registries` names each source of a mounted release with the digest of its root metadata.
11. **Costs.** Opening a source: O(R) root versions and O(log L) hashes and tiles for a consistency proof in a log of L entries. An entry: O(log L). A name: at most 32 map tiles of at most 12 KiB and O(256 + 8 k) hashes for its head, and O(E log L) for its E entries. An owner's keys: O(E) to replay, O(log E) to find the keys at a position. A logged time: at most 64 kept checkpoints; a recovery's effect: O(log L) probes by binary search over sizes, since witness times never decrease as the log grows. Every proof is computed from tiles fetched at most once each.

## 15. Publishing, yanks, audit, watch and bundles

The client's side of ADR 0122 rules 33, 36.2 and 41 to 43, on the registry client of §14. Each rule here holds for `una ita`; none changes what `una check` reports, so they are tested as tooling.

1. **Reading and writing a registry.** A source is read as §14 states: from the first of its places, its mirrors (§14 rule 1) and then its own URL, whose copy verifies, every byte verified before it is used. A submission is written through the transport of the source's own URL, below its base: a directory's transport writes each file once, through a fresh private file renamed into place, and refuses a second write of other bytes at the same path; an `https://` URL has no transport yet, and a command that would send to one stops with a diagnostic saying so (U7013), and sends nothing.
2. **Private sources** (rule 42). Each need resolves at one source, with no fallback (§3 rule 2, §10). `una ita add <owner>/<name>` names the source that serves `<owner>` itself, and `--from` another source is a usage error, so a private owner's package is never looked for at another registry. Only the root and `itaty.una` name sources (§12 rule 1).
3. **Publishing** (rule 43). `una ita publish [path]`:
   - refuses (U7013 at the head) a package that §11 rule 12 refuses; then one whose directory, or any ancestor of it up to the file system's root, holds an entry `.git`, a directory or a file: the check that a workspace member has no uncommitted change is not available yet, so a package in a git repository is not published, and no program is started to look; then one whose `una check` at tier 1 reports any diagnostic, an obligation not proven included;
   - publishes `ita.una` and the modules exactly as checked, and `README.md`, `CHANGELOG.md` and the license files at the top of the package, each read once with no link followed; every other file stays out, and a file or a bound that §9 rule 1 refuses refuses the publication;
   - sends to the source that the head's `publish to` names: a published package has no `sources` section, so a source other than `itaty` is the one that the `sources` of the nearest workspace whose `itaty.una` lists the package as a member declares, with its URL and its root's pin, and a source no such workspace declares is refused (U7013). `itaty`'s URL and root are built into `una`. The URL is given to the transport of rule 1, so a source served over HTTPS stops here with U7013, and nothing is sent;
   - reads the name's state at a fresh checkpoint (§14 rule 3): the head of its chain, its releases and its yanks; a version already released, and authority or a trusted module beyond the highest earlier release of its line (§11 rule 7), are refused;
   - gives rule 26's advice: each public header of the highest earlier release of the line, a line of one of its modules that starts with `pub ` read up to its ` = `, that the new release no longer has as written, removed or changed, is printed as advice; nothing is refused for it;
   - writes the release statement of §11 rule 6, whose `previous` is the head of the name's chain and whose `authority` is the union of the `uses` of the package's public functions and of its handlers; signs it with the owner's keys of rule 4; and sends the archive, then the statement, as `submit/<leaf>/archive` and `submit/<leaf>/entry` below the source's base, where `<leaf>` is the hex of the statement's leaf hash (RFC 9162: SHA-256 of `0x00` and the signed note); then records `registry/<source>/submitted/<leaf>`, holding the source's URL, in Itaoca, for rule 8.
4. **The owner's keys** (rule 38). The keys that sign a release or a yank are chosen in `.una/keys` of `HOME`, against the owner's keys and threshold valid at the checkpoint read (§14 rule 5):
   - the owner's key files are the regular files named `<owner>-<id>.key`, `<id>` lowercase hexadecimal, and `itauna-<id>.key` for a name without an owner;
   - every one of them must read: a link, a file of more than 4 KiB, one that a group or others can read, one encrypted with a passphrase, and one that does not parse each refuse the command (U7013), so no key file of the owner is silently left out or taken;
   - a key signs only when its name and its public key are those of a key of the owner, so its signature line carries the name and the key hash that a client looks for (§11 rule 3); every such key signs, and fewer distinct ones than the owner's threshold refuse the command (U7013).

   A key file is read in the form written without a passphrase:

   ```text
   ita-keyfile/1
   name acme.publisher
   secret <base64 of the 32-byte Ed25519 secret>
   ```

   A key whose public key fails §11 rule 2 on a signature of its own is refused. A key file encrypted with a passphrase starts with `ita-keyfile-encrypted/1`; it is refused with the reason that passphrase encryption is not implemented yet, and never read as clear.
5. **Promises** (rule 36.2). The registry answers a submission with a promise, written at `submit/<leaf>/promise`: a note signed by the log's key whose text is `ita-promise/1`, `source <source>`, `leaf sha256:<hex>` and `by <seconds>`, each on its line. `by` is the registry's own commitment and no time of anything else. A promise is kept when its entry appears in a chain followed, and broken when a checkpoint whose witness time is past `by` does not hold its entry.
6. **Yanks and advisories** (rule 41, §11 rule 11, and the amendment of ADR 0122 on the yank command).
   - `una ita yank <version> [path]` writes the yank statement of a release of the package, whose `previous` is the head of the name's chain, signs it with the owner's keys of rule 4, at least the owner's threshold, and sends it as `submit/<leaf>/entry` to the source that the head's `publish to` names, through the same transport as `una ita publish` (rule 3), where it is logged in the name's chain; a version not released, or already yanked, is refused (U7013).
   - The client reads a name's yanks and advisories from its chain (§14 rule 4), in log order. A yank stands only when it is of the name and source it is read for, names a release logged before it, and is signed by the owner's threshold of keys valid at its log position. An advisory stands only when it is of that name and source, at least one security key of the root signs it, and every version it names is a release logged before it; it lifts the cooldown for the fix it names by digest only with a second security key or the owner's threshold besides. Any other refuses the name (U7010).
   - Resolution (§10 rule 4) reads a release as yanked when a yank of it stands, or when it counts only with a key revoked as compromised before its position; as advised when an advisory names it; and with no cooldown when a lifting advisory names it, by digest, as its fix. The cooldown is a wait in witness time: the witness time of the resolution point's checkpoint minus the release's logged-at time (§14 rule 7).
7. **`una ita audit [path]`** opens each source of the lock's registry packages at a fresh checkpoint (§14 rule 3) and reads, for each package, the verified state of its name and of its owner. It reports U7014, once per package with every finding, at the root's need when the root needs the package and otherwise at its line of `ita.lock`, for a release that is yanked; named by an advisory, with the advisory's identifier, severity and summary; that counts only with a key revoked as compromised since a log entry before it; for a recovery or a dispute pending for its owner; and for a key change of its owner from a log position after the lock's resolution point. A state that cannot be had verified at a fresh checkpoint is U7010 at the package, so the audit calls nothing clean that it did not verify. The exit code is 1 on any. `una build` never calls it. Costs: per package, one state of its name and one of its owner, each read once per source, then O(A + K) for its advisories and its owner's key events.
8. **`una ita watch <owner> [<name>…] [--from <source>]`**, `itaty` by default, reads the source the package in the current directory, or its workspace, declares, at a fresh checkpoint, and follows the owner's chain `owner:<owner>` and the chains of the names named and of those the position holds, from the position stored in Itaoca at `registry/<source>/watch/<owner>.position`:

   ```text
   ita-watch/1
   source acme
   owner acme 12
   name acme/ledger 5
   ```

   the format line, the source, the owner with the count of its entries seen, and one line per name, sorted, with its count, in one encoding; a position that does not read is a usage error. It prints each new entry, its chain, format, log index and leaf hash; a chain whose count is below the one stored is U7010 against the registry. For each submission recorded in Itaoca (rule 3), it reads the promise at the place it was sent, verifies it with the log's key of the last root, and prints it kept or broken; a broken promise exits with code 1, and is evidence the owner can publish. It then stores the new position. Costs: one chain per name followed, each verified as §14 rule 4 states.
9. **Bundles** (rule 33). `una ita bundle <file> [path]` reads each source of the lock's registry packages at the lock's resolution point (§14 rule 3), from its first place that verifies, and writes into one file:

    ```text
    ita-bundle/1
    <length of the listing>
    <listing: path NUL length NUL hex SHA-256, one line per entry, in ascending byte order>
    <each entry's bytes, in listing order>
    ```

    whose entries are `packages/<hex>`, the archive (§9 rule 3) of the package of digest `sha256:<hex>`; `provenance/<source>/<hex>/checkpoint-<size>`, its provenance at the lock's resolution point (§9 rule 7); and `registry/<source>/<path>`, every file of the source's registry, but its archives, that the client read to verify those releases: the root chain, the checkpoints, the log's tiles and entry bundles, and the map's tiles of each name and owner. A release the registry does not hold at that point as the lock names it, by log index and digest, is U7010, and a source no place serves verified U7010 or U7013; then nothing is written, and the file is written through a fresh file and a rename. At most 100,000 entries and 4 GiB. `una ita unbundle <file> [path]` reads the bundle once into memory and checks its listing (paths of these forms, strictly ascending, each entry's hash, nothing after the last). Then, for each registry package of the lock that Itaoca does not hold verified already, it verifies its archive's digest against the lock and its provenance as every load does (§9 rule 7, with `ita.una`'s pin), and opens the bundle's copy of its source with the client at the lock's resolution point, with no freshness and against the checkpoint Itaoca stores for the source (§14 rule 3): the release the client reads MUST be the lock's, by log index and digest, and its name's state, its yanks and advisories and its owner's keys included, MUST verify. Only when every one verifies does it store them in Itaoca, and then the source's checkpoint as its floor, which is replaced only by a larger one, so unbundling never moves it backwards. A package the bundle does not hold is U7011, any other failure U7010, and then nothing is written. Entries that no locked package uses are not stored. Costs: O(B) for a bundle of B bytes, O(e log e) to sort its e entries, and the client's costs of §14 rule 11.
