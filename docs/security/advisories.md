# Security advisories

Every security fix of a published Una release that affected an earlier published release, newest first. Each entry says what an accepted program could do, which releases were affected, which release of each line fixed it, and where the fix is described. The source of every entry is the release's section of `CHANGELOG.md` and the specification and design decisions it cites; this page paraphrases them and links them. To report a vulnerability, see `SECURITY.md`.

A fix made before any release that it would affect is not an advisory. Neither is hardening that no accepted program reaches. Both are in `CHANGELOG.md`, and the cases this page considered and left out are listed at the end.

## Severity

- **Critical:** an accepted program can move `secret` data, or a decision on it, to a public sink through an ordinary construct.
- **High:** an accepted program breaks a stated guarantee, noninterference or authority, in a narrower shape, or the run uses a handler the checker did not check.
- **Medium:** an accepted program reveals `internal`-labeled facts, or one bit decided by them.
- **Low:** details of the machine or account that built a release are exposed in its artifacts.

"Affected" names the releases that `CHANGELOG.md` names. Where it names none, the entry says so, rather than infer a range.

## Advisories

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

- **#323**, the labels of an un-annotated `let edit` (ADR 0075): `CHANGELOG.md` names only v0.2.0 as affected, which was not published.
- **#310's `Http` part, and HTTP credentials kept to their origin** (v0.3.0): `CHANGELOG.md` names no published release that had `Http` before the fix.
- **#186**, `edit` arguments flowing back with their final labels: `CHANGELOG.md` names no affected published release.
- **#216**, defense in depth near `of` parameters: none of its sites leaked.
- **#424**, the runtime checks a task's code before calling it, and **#480**, native code aborts on a `set` outside the list: hardening that no accepted program reaches.
