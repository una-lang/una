# 0086. Host text that is not valid UTF-8 aborts the run

- Status: Accepted
- Date: 2026-10-09
- Deciders: coordinator under the standing security rule (2026-10-09), for the owner's review; written by Owner (Una-1)
- Resolves: #460 (S-16)
- Amends: `spec/14 §3` (host text; `Files` rules 6 and 7), `spec/13 §1` (U6005, and its explanation without an example), `spec/13 §5` (`aborted`), `spec/07 §1` (aborts), `spec/16 §3` (stderr)

## Context

A `Text` is valid UTF-8 (`spec/22`). Two host operations make one from bytes the program does not control:
- `Files.read` returns a file's content;
- `Env.get` returns a variable of the process environment.

Both hosts, the interpreter of `una run` and the runtime of `una build`, decode those bytes with the standard library. On bytes that are not UTF-8, they did this:
- `Files.read` failed with `FileError.Io`;
- `Env.get` returned `None`.

**S-16: the content decided a public outcome.** Found by Una-1 while reviewing M5, and confirmed on v0.3.4. A package that never writes a file has a `public` file-state label (ADR 0085), so a read's failure is `public`. This program printed, to the public console, whether an `internal` file is valid UTF-8:

```una
let r = attempt @files.read("f.txt")
when r
  is Ok(t) -> @console.print("valid utf-8")
  is Err(e) -> @console.print("invalid utf-8")
```

- `una check` accepted it.
- `una run` printed `valid utf-8` for a file holding `hello`, and `invalid utf-8` for the bytes `FF FE`.

The content is `internal` (`spec/14 §3`). `spec/14 §3` labels a read's failure by its path and the file-state label, "not the content". Yet the content decided the failure. `Env.get` had the same channel: its result is `Option[internal Text]`, whose tag is `public`, and an environment value that is not UTF-8 turned `Some` into `None`.

The leak is one bit per read. The bytes come from the host, not from the program's own secrets, since a program's own writes are always valid UTF-8. It is still a flow from `internal` to `public` without a policy, and M5's hypothesis H1 (`docs/plan/formal-track-m5.md` §2) does not hold for a host that decides a read's failure by content.

**The audit of every host value the program does not control:**

| Operation | Bytes the program does not control | Before | Leak |
| --- | --- | --- | --- |
| `Files.read` | the file's content | `FileError.Io` | yes: the failure's label is the path and the file state, not the content |
| `Env.get` | the variable's value | `None` | yes: the `Option`'s tag is `public` |
| `Http` | the response body | `HttpError.Protocol` (`spec/24 §1`) | no: every failure of a request is at least `internal` (ADR 0085 decision 7), as the response is |
| `Keys.hmac` | the key file | `una run` stops before the program starts | no: key material never enters a value, and `hmac` fails only on an unknown `public` key name |
| `Clock.now`, `Console.print` | none | | no |

## Decision

1. **The rule.** When a host operation would make a `Text` from bytes that are not valid UTF-8, the run aborts with U6005 at the operation, as for U6004 (`spec/24 §1`). This covers:
   - the content of `Files.read`;
   - the value of `Env.get`.

   The operation neither fails nor returns: no handler, `attempt` or `if error` sees it. The abort is outside the noninterference model (`spec/07 §1`), as termination is. In a task, the abort stops the task and its block reports it, as for U6004.
2. **Both hosts and both editions.** The interpreter of `una run` (examples never reach a host, `spec/11 §6`) and the native runtime (`una build`) behave the same. Edition `0.1` gets the rule in the v0.1 line (v0.1.7), and edition `0.2` in v0.3.5.
3. **What does not change.** `una check`'s output is unchanged, because the rule is about runs only. A file or a variable that is valid UTF-8 is read exactly as before. `FileError` keeps its variants, and `Io` remains for the other input and output errors, which the shape decides.
4. **U6005** is a new runtime code: "a host value is not valid UTF-8". Its message names the operation, as in `the content of this file is not valid UTF-8` or `the value of this environment variable is not valid UTF-8`. It goes to stderr with exit code 1, and `una run --json` reports it with `status` `aborted`.

**Complexity.** Validating UTF-8 takes O(n) time in the bytes read, with no extra space: the bytes are read once and checked in place, as before (`String::from_utf8`).

## Consequences

- M5's hypothesis H1 now holds for the real host. Whether a read fails depends on its path and the file system's shape only: whether the file exists, the links and directories around it, and its permissions.
- **Out of the model, as for races and hard links (`spec/14 §3` rules 4 and 5, now with rules 6 and 7):**
  - A write can fail on resources the environment sets: `ENOSPC`, a quota, or a file size limit (`EFBIG`). Such a failure can depend on the content's length, which M5's hypothesis H2 excludes.
  - Another process, a server on the same machine included, can change the granted directory between operations. M5 models the file system and the servers as disjoint.

  These are environmental, like a race: they depend on limits and processes outside the package, which the model does not describe.
- A program that read text in a legacy encoding now aborts where it used to get `Io`. Such a file was never readable as `Text`, so no program lost a value it had.

## Alternatives considered

- **B: decode lossily**, replacing ill-formed bytes with U+FFFD, as the lexer does for source (`spec/01` rule 5). The read would never fail on content. But it silently alters the data an agent reads, which harms integrity: a program could not tell a file it read faithfully from one it read changed.
- **C: label every read's failure `internal` at least.** This is sound, but every `if error pass` of a read in a public context becomes U4002, and so does a public branch on `attempt`. It breaks most programs that read a file, to close a channel that only ill-formed files open.
- **Fail with a new `FileError` variant labeled `internal`.** The variant's label would still need its own rule, and `Env.get` has no failure to carry it. The abort covers both with one rule.
