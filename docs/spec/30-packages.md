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
4. **What this release loads.** Path dependencies, by §12. A need of a registry or of a workspace member is refused, U7008 at the need, so no import reaches a package that is not checked with the program.

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
2. **Commands of this release.** `una ita lock [--check] [--accept-authority] [--accept-trust <package>] [path]` (§4 rule 4, §12 rule 6, §13 rule 2); `una ita tree [path]` and `una ita remove <name> [path]` (§12 rule 9).
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
5. **Itaoca.** Fetched packages are stored in `packages/sha256/<hex>/` under `~/.una/itaoca`, or the directory `UNA_ITAOCA` names. A package is unpacked once into a fresh directory, each file created exclusively and made read-only, then renamed into place. It is verified on every load: its files are read again, no symbolic link is followed, an entry that is not a regular file or a directory refuses it, and the digest of what was read must be the one asked for (U7010 otherwise), so a store changed after the fetch never gives a different program.

## 10. Versions and resolution

1. **Versions.** Semantic versioning 2.0.0 without build metadata, each numeric part at most 2^31 − 1, ordered by semver precedence. The identity of a package is its source, its name and its major line: `MAJOR` when it is at least 1, `0.MINOR` otherwise. Two major lines of one package are two identities, which may both be in one program.
2. **Requirements.** A need names the least version it admits on its line. A pre-release is selected only if some requirement names a pre-release of that identity. There are no upper bounds and no ranges.
3. **Minimal version selection.** The build list holds, for each identity reachable from the root, the highest of the versions the reached needs ask for, each read as the least release at or above it; never a newer one. The result does not depend on the order of any list, and a release published later changes nothing until a need asks for it.
4. **Raises.** A selection that is yanked, named by an advisory not fixed in it, or avoided by the root is raised to the least release of its identity above it that is none of these, is not a pre-release unless one is named, and was logged at least the cooldown before the resolution point, 72 hours of witness time by default. An advisory signed by the threshold that names the fix by digest lifts the cooldown for that fix. With no such release, resolution fails, naming the newest candidate the cooldown holds back. The walk resumes with the needs of the release taken.
5. **Bounds.** 100,000 releases visited, 10,000 packages in the list, 1,000 needs per release, 10,000 releases read per name; a release that needs its own identity is refused.
6. **Costs.** O(N + R) time and space for N releases reached and R need edges, with hash maps; O(log K + S) per raise for an identity of K releases of which S are skipped; O(P log P) to sort the list. There is no backtracking and no search.

This release resolves in memory only; the commands that read release statements and per-name state join as the registry client lands.

## 11. Verifying a registry's documents

The registry client verifies what it fetches with these rules, each a pure function of the bytes; the commands that fetch join with the client.

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
8. **Effective ceilings and the report.** The effective ceiling of a package bounds what it can ever exercise: the least fixpoint of `eff(p) = ⋃ over the packages d that need p of (ceiling_d(p) ∩ eff(d))`, computed by a worklist in O((P + R) · C) for P packages, R needs and C capability operations, cycles included (ADR 0122 rule 51). The root may be granted any host capability with `--grant`, so `eff(root)` is taken as every operation, and a direct dependency's effective ceiling is the ceiling the root writes for it: a bound never narrower than what the package can exercise. The report (`spec/13 §6`) holds `dependencies`, after `handler_authority`, one entry per package of the program other than the root, in name order: `package`, `version`, `local`, `source` (`"path"`), `path`, `digest` (the lock's, or `null`), `signed` (`null` for a path package), `replaced`, `edition`, `profile`, `granted_by` (per dependent, its name, its `source`, `null` for the root, and the ceiling it writes as `may`), `effective_ceiling`, `authority` (the union of the `uses` of its functions and handlers) and `trust` (`modules`). A package with no dependencies has `"dependencies": []`. `tcb` gains `registries` (each registry source with the digest of its root metadata; none while only path dependencies load) and `dependency_trust` (§13). The human report prints one line per dependency: `util 0.2.0 (path deps/util): may Console[print] (by demo); uses Console[print]; trust none`.
9. **`una ita tree [path]`** checks the program at tier 0 and prints the root's name and version, then one line per dependency, in the form of the report's human line; with `--json`, one `una.ita/1` document with `schema`, `command` (`"tree"`), `root`, `packages`, the report's `dependencies`, and `diagnostics`. **`una ita remove <name> [path]`** removes the need of `name` from `ita.una`, with its indented lines, and the `needs` section when it empties, through a temporary file and a rename, then writes the lock as `una ita lock` does. A name the root does not need, or an `ita.una` that does not parse, is a usage error and nothing is written. Neither opens a connection.

## 13. Trust a dependency adds

1. **A dependency's trusted modules** (its `trusted` section and `trusted module` headers) are trusted in the program only if a trust line of the root names the package and each of them by its module path in the package (U7012 at the need otherwise). A trust line names a package the root needs directly by its name alone, its major line and source being the need's; a path package has no source, so a trust line with `from` names a registry package, never a path one.
2. **Consent is pinned to the whole package.** The lock records, for the package, `consented "<digest>"` with its content digest, and `trusted modules ...`. Loading compares the recorded digest with the package's content as it is now, and every trusted module with those recorded; any difference, a change of any file of the package included, is U7012 until `una ita lock --accept-trust <package>` records it again. A policy's `via` may call any function of its package, so pinning the trusted module's file alone would not pin the code it runs.
3. **An extent of its own** (rule 53.1). A consented trusted module `m` of the dependency `<local>` is the program's trusted module `<local>.m`, and an extent of its own inside its package's: calls into it from outside it, the package's other modules included, are crossings, granted with `with` (U3020). A release whose policy it declares is not a release into a boundary, whoever reaches it (no U3024); every other release stays U3024.
4. **One ceiling, for now.** Until a crossing's ceiling depends on its direction, the package's extent and its trusted extents share one ceiling: the intersection of the root's `may` lines for the package and those under its trust line. It is never wider than either, so a program this accepts is one rule 53.1 accepts.
5. **The report** lists each dependency's consented trusted modules under its `trust`, and `tcb.dependency_trust` gives them by package, with their line counts; `tcb.trusted_modules` keeps the root's own.
