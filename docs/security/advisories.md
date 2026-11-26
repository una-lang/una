# Security advisories

Every security fix of a published Una release that affected an earlier published release, newest first. Each entry says what an accepted program could do, which releases were affected, which release of each line fixed it, and where the fix is described. The source of every entry is the release's section of `CHANGELOG.md` and the specification and design decisions it cites; this page paraphrases them and links them. To report a vulnerability, see `SECURITY.md`.

A fix made before any release that it would affect is not an advisory. Neither is hardening that no accepted program reaches. Both are in `CHANGELOG.md`, and the cases this page considered and left out are listed at the end.

## Severity

- **Critical:** an accepted program can move `secret` data, or a decision on it, to a public sink through an ordinary construct.
- **High:** an accepted program breaks a stated guarantee, noninterference or authority, in a narrower shape, or the run uses a handler the checker did not check.
- **Medium:** an accepted program reveals `internal`-labeled facts, or one bit decided by them.
- **Low:** details of the machine or account that built a release are exposed in its artifacts.
- **Availability:** a denial of service: an accepted program that reads untrusted input can be made to use resources, time or memory, without bound or superlinearly in the size of that input.

"Affected" names the releases that `CHANGELOG.md` names. Where it names none, the entry says so, rather than infer a range.

## Advisories

### UNA-SA-024: the order of started tasks' public events could follow data above `public`

- **Severity:** High
- **Affected:** v0.4.0 and v0.5.0, in editions `0.4` and `0.5`, under `una run` and in native executables run with `--virtual-time`. `CHANGELOG.md` names no affected release: tasks (`tasks` and `start`) exist only from edition `0.4`, which v0.4.0 first shipped, and the code of both defects is in v0.4.0 and v0.5.0. Neither was reproduced on a published archive.
- **Fixed in:** v0.5.1.
- **Summary:** In a program that starts tasks, the order in which the tasks' public events happen must not depend on data above `public` (`spec/27 §7` rule 4, `spec/27 §8` rule 5). Two paths broke it. A policy call runs its `via`, whose code the label rules do not check, and the checker did not record the call as a check point of the schedule (#2214): a `via` whose work depended on its argument ordered the public prints of started tasks by that argument, `secret` as well as `internal`, and the program was accepted with an empty report. And a `start` in code under a raised program counter took a place of its statement's bound, and quiet code that parked let the queued tasks run first (#1545, the S-40 remainder): the order of other tasks' public events followed `internal` data. Now a policy call is a check point, so a `via` given data above `internal` in a program that starts tasks is refused with U4002, and one given `internal` data is listed in the report's `information.timing`; a task started by quiet code is held by its statement, joins no queue and counts against no bound, so the public events keep the order of the same run without the quiet code, in both engines (ADR 0117 amendment of 2026-11-22). Native code under the real clock is out of model: its order across tasks is real time. A held task that runs longer than a turn while another task is ready remains a residual at `internal` (#1708). Users of an affected release should not release data above `internal` through a policy in a package that starts tasks, and should not start tasks in code that branches on `internal` data.
- **References:** #2214, #1545, #1708, `spec/27 §7`, `spec/27 §8`, ADR 0117.

### UNA-SA-023: a `Files` grant on the current directory was not confined against symbolic links

- **Severity:** High
- **Affected:** every published release from v0.1.0 through v0.4.0 under `una run`, and from v0.3.0 through v0.4.0 in the native executables that `una build` writes, in every edition. The code was read at v0.1.0, v0.1.13, v0.3.0, v0.3.15, v0.3.17 and v0.4.0, and the behavior reproduced on v0.3.15 in both engines.
- **Fixed in:** v0.5.0. The v0.3 and v0.1 lines have no release with the fix.
- **Summary:** `Files` confines every path to its grant: each directory of the path is resolved with its symbolic links followed and must lie inside the granted prefix (`spec/14 §3` rules 2 and 3, `spec/13 §5`). A grant whose path names the directory it is relative to was held as the empty prefix, and the empty prefix skipped that check: a path under it was opened as the program wrote it, relative to the working directory, with no link check. The affected grants are `files:.`, `files:./` and `files:a/..` (any path that leaves no component once joined to its base) in a `--grant` flag of `una run` or of an executable, in an executable's embedded grants, and in `[run] grants` of a package run as `una run` or `una run .`; `files:..` in a package run as `una run <name>` from its parent; and, through the defect below, `files:../..` and its other spellings. A grant on a named directory, and a package run by a longer or absolute path, were confined correctly. Under an affected grant, a program could follow a symbolic link inside the granted directory out of it: read a file outside, create a new file outside, and overwrite an existing one, through a link to a directory or a final link to a file, within the user's file permissions. It takes a link that leaves the directory and a program that opens a path through it, which the path in the program's input can name: the threat that `spec/14 §3` rules 2 and 3 address. `..` and absolute paths in the program's path were refused as always. In the same releases, a grant path with two or more `..` lost one `..` for each later one, so `files:../../x` granted `./x` and `files:../../../x` granted `../x`: the operator's grant named another directory than it said, which this release also fixes. Such a directory was otherwise confined, except in the empty case above. Now, under every grant, a path is resolved inside the grant before it is opened, in both engines. Under a grant on the current directory, a final component that is a symbolic link is `Denied`, even one that stays inside; a dangling link and a link loop are `Denied` where they were `NotFound` or `Io`; a working directory whose canonical path cannot be computed gives `Denied`; and a grant path with two `..` now names the directory it says. An executable built by an affected release keeps the old behavior until it is built again with v0.5.0. Users of an affected release should name the package by an absolute path (`una run "$PWD"`), grant a named subdirectory or an absolute path rather than `.`, and not write a grant path with two `..`. Do not grant `Files` over a directory whose contents an untrusted party controls.
- **References:** #2144, #2149, #2135, #1459, `spec/13 §5`, `spec/14 §3`.

### UNA-SA-022: `Files` confinement opened a path whose directory existed but did not resolve

- **Severity:** High
- **Affected:** `CHANGELOG.md` names no release. The rule that was wrong is in the `Files` host of every tag from v0.1.0 on, so every published release before v0.4.0 is taken as affected: `Files.read` and `Files.write` under `una run`, and in the native executables of the releases that build them, in every edition. It was reproduced on macOS, where a canonical path stops resolving past 1024 bytes, and on Linux with glibc 2.39, past 4096 bytes. Systems with musl were not tested.
- **Fixed in:** v0.4.0. The v0.3 and v0.1 lines have no release with the fix.
- **Summary:** `Files` resolves every intermediate directory of a path with its symbolic links followed and requires it to lie inside the granted prefix (`spec/14 §3` rule 2). When a directory did not resolve and was not itself a symbolic link, confinement took it for a directory that does not exist and left the lexical path to the operation, which then fails with `NotFound`. A directory that exists can also fail to resolve, for example when its canonical path is longer than the host takes although the path the program gave is short. The operation then opened the lexical path, and the operating system followed symbolic links below that directory that confinement never checked. A program granted `files:<dir>:read` or `write` could therefore read or write a file outside `<dir>` when the tree inside `<dir>` was arranged for it, which takes someone who can create directories and links there: the threat that `spec/14 §3` rules 2 and 3 address. The lexical path is now left to the operation only when a component does not exist or is not a directory; any other failure to resolve the directory is `Denied`, in both engines. An executable built by an affected release keeps the old behavior until it is built again. Users of an affected release should not grant `Files` over a directory whose contents an untrusted party controls. That is the behavior of v0.4.0. A later change, released in v0.5.0 and recorded in `CHANGELOG.md` as hardening (#1459), leaves the lexical path to no operation: when a directory does not exist, or a component of it is not a directory, confinement itself answers `NotFound`, or `Io`, and an operation is given only the canonical form of a directory resolved inside the grant, joined with the final component (`spec/14 §3` rule 2).
- **References:** #1380, #1381, ADR 0038 D9, `spec/14 §3`; #1459 for the later change.

### UNA-SA-021: a run's trace showed whether an operation failed although that decision was labeled

- **Severity:** Medium
- **Affected:** `CHANGELOG.md` names no release. The exposure was reproduced on v0.3.17, in the trace of `una run` and in the `--trace` of executables built by `una build`, and on v0.1.13, in the trace of `una run`; the decision has been labeled since v0.3.4 and v0.1.6 (ADR 0085, UNA-SA-008), so the releases of both lines from those on are taken as affected.
- **Fixed in:** v0.4.0. The v0.3 and v0.1 lines have no release with the fix.
- **Summary:** the trace of a run redacted an operation's returned value by its label but wrote, in clear, whether the operation returned or failed, its failure variant and its payload, although the checker labels that decision (ADR 0085). The `internal` path of a `Files.write`, which the write's own event showed as `<redacted:internal>`, could therefore be recovered from that write's outcome or from the outcomes of later `Files.read`s of public paths, one bit per read; and in the v0.3 line the failure variant of an `Http` request, `Denied` or `Unreachable`, told whether an `internal` URL lay inside the grant. The program itself could not observe these values (U4003); only a reader of the trace could. `secret` data are not affected: these operations accept at most `internal`. An outcome whose label, the decision's joined with its failure type's and its result's, is above `public` is now written as `{"kind":"redacted","level":"internal"}`, and no text is built from it, in both engines and every edition. The traces of examples keep their outcomes in clear. Readers of run traces must accept the kind `redacted`: the outcomes of every `Http` request, `Files.read`, `Env.get` and `Keys.hmac`, and of every `Files` operation of a package whose file-state label is above `public`, are now redacted. An executable built by an affected release keeps the old behavior until it is built again. Users of an affected release should treat the run traces of programs that handle `internal` data as `internal`.
- **References:** #1552, #1562, S-45, ADR 0032 (amendment of 2026-11-06), ADR 0085, `spec/11 §5`, `spec/26 §6`.

### UNA-SA-020: writing a long integer as text took time quadratic in its digits

- **Severity:** Availability
- **Affected:** `CHANGELOG.md` names no release. Every published release before v0.4.0 writes an `Int` in decimal without a bound: by interpolation, `to_text` and `to_json`, and in the Una form of traces, outputs and failures.
- **Fixed in:** v0.4.0. The v0.3 and v0.1 lines have no release with the fix.
- **Summary:** an `Int` has no bound, and a few multiplications build one of millions of digits, from a program's own arithmetic or from untrusted numbers it computes with. Writing such an integer in decimal took time quadratic in its digits, so one write could be made to spend that time; UNA-SA-016 bounded the parsing direction only. Both engines now refuse to write an `Int` of more than 4300 digits, the bound of parsing, decided in O(1) from the integer's bit length and before any conversion: the run aborts with U6003 at the interpolation, the call or the operation; the final value or failure of `main` aborts with no position, and a test's report names such a value instead of writing it. A program that writes integers within the bound is unchanged.
- **References:** #923, #1221, `spec/11 §7`. #1223 (ADR 0004 amendment of 2026-11-26, Proposed) replaces the bound with a subquadratic conversion: from the release that includes it, an `Int` of any size is written in O(M(d) log d) for its d digits, the cost class of one multiplication of its size, after reserving the bytes of its text and conversion against the memory budget (U6003 when they do not fit); the bound of parsing (UNA-SA-016) stays.

### UNA-SA-019: the v0.1 proof cache accepted entries it had not written

- **Severity:** High
- **Affected:** every published release of the v0.1 line up to and including v0.1.12, at tier 1 (`una check`, the build report), in edition `0.1`. Releases from v0.2.0-alpha.3 on, including the v0.3 line, authenticate their cache (ADR 0064) and are not affected.
- **Fixed in:** v0.1.13.
- **Summary:** the v0.1 verifier kept the solver's answers in `.una/cache/proofs/` inside the package and reused any entry whose name matched a query. The name is a hash of `verification.lock` and of the query, both derived from the package itself, and the entries carried no authentication. A repository could therefore ship cache entries marking any proof obligation as proven, a false one included. Running `una check` in a clone of it then reported those contracts as proved without running the solver, and the build report counted them the same way. Entries now carry an HMAC-SHA-256 tag under a key that belongs to the user and lives outside the package (the file `UNA_PROOF_KEY` names, else `~/.una/proof-cache.key`, made on first use, readable by the user only). An entry without a valid tag is ignored and its query solved again, so caches written by earlier releases are rebuilt once. The build report names the key file instead of listing the cache as trusted. The key file is used only when it is a regular file with one link, owned by the user, granting nothing to group or others, of 16 to 4096 bytes; otherwise the cache is off.
- **References:** #1164, ADR 0064.

### UNA-SA-018: `una fmt` followed a symbolic link planted at its temporary file

- **Severity:** High
- **Affected:** every published release up to and including v0.3.16 and v0.1.12: `una fmt` in both lines, and `una fix` in the v0.3 line, in every edition.
- **Fixed in:** v0.3.17; v0.1.13.
- **Summary:** `una fmt` replaced a source file by writing a temporary file at a fixed name next to it (`.<name>.fmt-tmp`) and renaming it over the file. The write followed a symbolic link at that name. A repository can commit such a link, so running `una fmt` in a clone of it could overwrite any file the user can write, outside the repository, with text the repository chose, and leave the source file as a link to it. `una fix` in the v0.3 line used the same write, and the proof cache that `una check` keeps in `.una/cache/proofs/` wrote its entries the same way. The temporary file is now created exclusively, under a name unique to the write, so nothing already at that name is written through, and the proof cache stores nothing when `.una`, `.una/cache` or `.una/cache/proofs` is a link that leads outside the package. A source file that is itself a symbolic link is still formatted in place only when the file it resolves to is inside the package; a link leading outside it is refused. Users of an affected release should not run `una fmt` or `una fix` in a repository they do not trust until they update.
- **References:** #1160, #1161, #1162, #1163.

### UNA-SA-017: releasing a deeply nested value overflowed the interpreter's stack

- **Severity:** Availability
- **Affected:** every published release up to and including v0.3.16 and v0.1.12, in the interpreter (`una run`), in every edition. Native executables are not affected.
- **Fixed in:** v0.3.17; v0.1.13.
- **Summary:** the interpreter released a value that a function value closes over by recursion on the native stack, one level per level of nesting. A loop that nests values through function values, such as a kind `L(next: fn() -> F)` built two million levels deep, finished its work and then aborted `una run` with a stack overflow when the value was released, instead of ending normally. A program whose nesting depth followed untrusted input could be made to abort this way. Function values now release the values they close over and the handlers of their capabilities by the same worklist that releases records. The v0.1 line had no worklist at all, so its fix gives one to records and function values together. There, reaching the overflow at the default `--max-depth` took a value of about 650,000 levels, which costs quadratic time to build, and a lower `--max-depth` reached it sooner.
- **References:** #1071, #1076, ADR 0060 rule R5.

### UNA-SA-016: parsing a long integer from text took time quadratic in its digits

- **Severity:** Availability
- **Affected:** every published release up to and including v0.3.15 and v0.1.11: `to_int` in every edition, and `parse_int` from edition 0.2.
- **Fixed in:** v0.3.16; v0.1.12.
- **Summary:** `to_int` and `parse_int` converted a text of d digits to an `Int` in O(d²) time, so a program that parsed untrusted text could be made to spend quadratic time in one call. Both now refuse a text of more than 4300 digits, the sign excluded and leading zeros included, after one O(b) scan and before any conversion: `to_int` returns `None`, and `parse_int` fails with the new `ParseError` variant `TooLong`, the bound that JSON integers already had. A `when` over `ParseError` that names each variant without an `else` arm must now handle `TooLong`. The v0.1 line has no `parse_int`, so its fix bounds `to_int` alone.
- **References:** #914, #922, #1027, `spec/22 §4`, `spec/14 §1`, `spec/23 §3`.

### UNA-SA-015: an interpolated text pattern let a secret choose a branch or a public Bool

- **Severity:** High
- **Affected:** every published release up to and including v0.3.14 and v0.1.10, in every edition.
- **Fixed in:** v0.3.15; v0.1.11.
- **Summary:** a `when` arm `is "…"` and the expression `x is "…"` treated their text literal as a `public` constant, but a literal with interpolation is computed from the values it interpolates. With `s: secret Text`, a `when` over another value with an arm `is "{s}"` could choose the arm that prints, and `x is "{s}"` gave a `public` `Bool`, with no diagnostic. Interpolation in a pattern's literal is now U4008 in every edition; comparing with `==` gives a `Bool` that carries the value's label.
- **References:** #801, #803, `spec/07 §7` rule 3.

### UNA-SA-014: release archives named the account that built them

- **Severity:** Low
- **Affected:** every published release archive through v0.3.13 and v0.1.10.
- **Fixed in:** v0.3.14. The v0.1 line is not fixed.
- **Summary:** each entry of a release archive carried the user and group names of the account that built it. The binaries in the archives were not affected. Archive entries are now owned by `0:0` with no user or group name, and `scripts/check-release.sh` checks the owners.
- **References:** #679, #689.

### UNA-SA-013: a function named as a value ran on the bindings where it was named, unchecked there

- **Severity:** High
- **Affected:** every release before v0.3.13 and v0.1.10.
- **Fixed in:** v0.3.13; v0.1.10.
- **Summary:** the checker took a named function value's `uses` only from the code that called it. A reference with no binding of a name its function uses was accepted and stopped the run, and a reference under one handler that was called under another was checked against the second and ran on the first. A reference now needs the function's `uses` where it is written: U3001, or U3003 where `uses` are declared empty.
- **References:** #565, #673, S-20, ADR 0093 amendment of 2026-10-14, `spec/05 §6` rule 7, `spec/11 §4`.

### UNA-SA-012: a policy call's failure was public whatever the policy's `to` label

- **Severity:** High
- **Affected:** releases before v0.3.7 and v0.1.9; `CHANGELOG.md` names no first affected release.
- **Fixed in:** v0.3.7; v0.1.9.
- **Summary:** a `via` function that failed on its `from` value let the caller see the failure as public, so a public branch on `attempt p(v)` revealed what it decided about a secret. The failure is now labeled `pc ⊔ ℓ(to)`.
- **References:** #496, S-18, ADR 0090.

### UNA-SA-011: a `checked` predicate's failure did not carry the labels of what it read

- **Severity:** High
- **Affected:** releases before v0.3.6 and v0.1.8; `CHANGELOG.md` names no first affected release.
- **Fixed in:** v0.3.6; v0.1.8.
- **Summary:** `checked(a, φ) else fail C` labeled its failure by `a` alone, so a predicate that read a secret, as `raw.len < limit` with a `secret` limit, failed publicly. The failure now carries every label that φ reads.
- **References:** #488, S-17, ADR 0089.

### UNA-SA-010: whether host text was valid UTF-8 could reach a public branch

- **Severity:** Medium
- **Affected:** releases before v0.3.5 and v0.1.7; `CHANGELOG.md` names no first affected release.
- **Fixed in:** v0.3.5; v0.1.7.
- **Summary:** `Files.read` failed and `Env.get` returned `None` on text that was not valid UTF-8, outcomes decided by `internal` content that a public branch could see. Both now abort the run with U6005.
- **References:** #460, S-16, ADR 0086, `spec/14 §3`.

### UNA-SA-009: an HTTP request's failure could reveal internal data the server received

- **Severity:** Medium
- **Affected:** releases before v0.3.4, in edition 0.2; `CHANGELOG.md` names no first affected release. The v0.1 line has no `Http`.
- **Fixed in:** v0.3.4.
- **Summary:** a server could decide `TooLarge`, `Protocol` or `TimedOut` by a request's body or header values, but the failure was labeled by the URL, the header names and the key name only. Every argument now labels it, so a request's failure is `internal`.
- **References:** #437, S-15, ADR 0085, `spec/24 §5`.

### UNA-SA-008: a write to a labeled path leaked through later file failures

- **Severity:** High
- **Affected:** releases before v0.3.4 and v0.1.6; `CHANGELOG.md` names no first affected release.
- **Fixed in:** v0.3.4; v0.1.6.
- **Summary:** whether a later read succeeded revealed whether a secret path had been written, in the same run or a later one. Every `Files` failure now also carries the package's file-state label.
- **References:** #436, S-14, ADR 0085.

### UNA-SA-007: executables and release archives named paths of the machine that built them

- **Severity:** Low
- **Affected:** releases before v0.3.3; `CHANGELOG.md` names no first affected release, and does not name the v0.1 line.
- **Fixed in:** v0.3.3.
- **Summary:** on macOS, `una build` kept the absolute path of the runtime library, usually with the home directory and the user name, in every executable, and release archives named paths of the machine that built them. Executables now link without debug information, and releases remap those paths.
- **References:** #382, S-13.

### UNA-SA-006: in edition 0.1, an annotation dropped a decision on `T` values

- **Severity:** Critical
- **Affected:** v0.1.0 to v0.1.4; edition 0.1 packages checked by releases of the v0.3 line before v0.3.2, whose first affected release `CHANGELOG.md` does not name.
- **Fixed in:** v0.3.2; v0.1.5.
- **Summary:** in a generic body, `let y: T = if a == b then p else q`, or a field `Hold[T]`, dropped the label of the comparison, which a later call could reveal. A decided value flowing into an annotation, a field or a function value's parameter of type `T` is now U4001.
- **References:** #354, #345.

### UNA-SA-005: in edition 0.1, a decision on `T` values given to function values

- **Severity:** Critical
- **Affected:** v0.3.0, for edition 0.1 packages; v0.1.0 to v0.1.4.
- **Fixed in:** v0.3.1; v0.1.5.
- **Summary:** a call that gave function values data decided by comparing values of `T` could reveal the decision, through the prelude's higher-order functions or a generic function of the program. Such a call now runs under the decision and raises its result.
- **References:** #346, #350.

### UNA-SA-004: in edition 0.1, a decision on `T` values through a call of a function value

- **Severity:** Critical
- **Affected:** v0.1.0 to v0.1.3.
- **Fixed in:** v0.3.0; v0.1.4.
- **Summary:** a value chosen by comparing values of `T`, passed to a function value whose result reached a sink, was accepted. It now raises the result and the program counter of the call.
- **References:** #253, #255.

### UNA-SA-003: whether `Files.write` failed revealed an internal path

- **Severity:** Critical
- **Affected:** v0.1.0 to v0.1.3; whether the v0.2.0 pre-releases were affected, `CHANGELOG.md` does not say.
- **Fixed in:** v0.3.0; v0.1.4.
- **Summary:** a host operation's failure is now labeled by the arguments that decide it, so `attempt` on a write to an `internal` path gives an `internal` `Result`.
- **References:** #310.

### UNA-SA-002: in edition 0.2, a decision on `T` values given to an effectful `uses of` parameter

- **Severity:** Critical
- **Affected:** v0.2.0-alpha.3; whether earlier pre-releases were affected, `CHANGELOG.md` does not say.
- **Fixed in:** v0.3.0.
- **Summary:** a value chosen by comparing values of `T`, passed to a `uses of` parameter whose effect revealed it, was accepted. Each caller now runs its argument under the labels of the type argument.
- **References:** #234, ADR 0067.

### UNA-SA-001: in edition 0.2, function values given for `of` parameters lost their labels

- **Severity:** Critical
- **Affected:** v0.2.0-alpha.3. The v0.1 line is not affected.
- **Fixed in:** v0.3.0.
- **Summary:** a function value given for a `uses of` or `fails of` parameter, as `map`, `filter`, `first_where` and `fold` take, lost its labels: a leak, and a false rejection.
- **References:** #210.

## Not advisories

- **#1514 (S-41), #1515 (S-42) and #1544**, the trace of a run showing how many handler installations and operations ran under a branch on data labeled above `public` (v0.4.0): in every published release of the v0.1 and v0.3 lines and in the v0.2 alphas, in every edition they serve, this is the specified behavior. Each operation invocation appends one event, and each handler installed by `with` has the id `with:<handler>#<n>` (`spec/11 §5`); the count behind `<n>` runs across the run, by `spec/19 §7` rule 2 from release 0.2.0-alpha.3 on, and by the implementation in the v0.1 line and in releases 0.2.0-alpha.1 and 0.2.0-alpha.2. Noninterference (`spec/07 §1`) is stated per observer label. An operation of the program's effects under such a branch, directly or through a call, needs `pc ⊑ accepts` (`spec/07 §3` rule 4), so whoever sees its event already sees the branch. A `with` discharges the effects of the capability its handler serves (T-Handler, `spec/10 §6`), so an operation that handler serves is no effect of the program; from release 0.3.8 on, no observer of the formal model sees it (F1 `EvVis`). No released document or proof states that instance ids, `seq` or the number of events are independent of labeled data. "Traces never show labeled data", in ADR 0032 and in the v0.1 line's `README.md`, is about the values a trace renders (ADR 0032 rule 3), and the same `README.md` sentence says that every effect is recorded in the trace: these findings render no labeled value. An argument whose static label is above `public` is redacted (ADR 0032 rule 3), and what varies with the branch is which events occurred, how many, how they are numbered, and which `public` values their arguments carry. Outcomes that the trace wrote in clear are a separate exposure, S-45 (#1552). v0.4.0 makes it a rule, applied in every edition, that quiet code numbers no installation and appends no such event (`spec/26 §6` rule 4).
- **#773 and #775**, native code ending with an allocator abort where the interpreter reports U6003, for a wide range in a `together for` and for a copy between tasks (v0.3.15): memory stayed bounded and no label or authority guarantee was broken, so no severity of the scale applies.
- **#779**, rustls 0.23.46 for four published advisories (v0.3.15): none is reachable in the TLS configuration of `Http`.
- **#799**, the labels of some values in the Core export (v0.3.15): the export is read by the formal re-check, never by a run; no accepted program leaked through it.
- **#323**, the labels of an un-annotated `let edit` (ADR 0075): `CHANGELOG.md` names only v0.2.0 as affected, which was not published.
- **#310's `Http` part, and HTTP credentials kept to their origin** (v0.3.0): `CHANGELOG.md` names no published release that had `Http` before the fix.
- **#186**, `edit` arguments flowing back with their final labels: `CHANGELOG.md` names no affected published release.
- **#216**, defense in depth near `of` parameters: none of its sites leaked.
- **#424**, the runtime checks a task's code before calling it, and **#480**, native code aborts on a `set` outside the list: hardening that no accepted program reaches.
