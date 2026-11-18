# N-64: a fresh agent's dogfood pass on the v0.3.3 archive

- **Issue:** #418.
- **Protocol:** `docs/reviews/v0.3.3/n64-plan.md` (decisions N-64 and N-65), with one amendment, in its first section below.
- **Runner:** Una-1.
- **Agents:**
  - general-purpose subagents with no prior conversation, on Opus 5.5 (`claude-opus-5-5`) at default effort;
  - one subagent per run, each in its own fresh `mktemp -d` directory `$W`;
  - each `$W` holds the v0.3.3 archive for aarch64-apple-darwin, downloaded with `gh release download v0.3.3` and checked against its published sha256.
- **Compared with:** the dogfood pass of #402 (`docs/reviews/v0.3.3/dogfood.md`), 12 `una check` runs by an agent that knew Una.

## Result

| Task | #402 (knew Una) | **Run A** (archive with guides) | **Run B** (archive without `docs/`) | First A (superseded) | First B (voided) |
| --- | --- | --- | --- | --- | --- |
| greet | 2 | 1 | 1 | 1 | 1 |
| orders | 3 | 1 | 1 | 1 | 7 |
| stock | 2 (+1 for its example) | 3 | 5 | 3 | 8 |
| tally | 4 | 1 | 3 | 1 | 3 |
| sensors | 1 | 1 | 1 | 1 | 4 |
| **Total** | **12** | **7** | **11** | 7 | 23 |
| Passed / gave up | 5 / 0 | 5 / 0 | 5 / 0 | 5 / 0 | 5 / 0 |

- **With the guides, a fresh agent is fast and stable:** 7 check runs, twice, against the 12 of an agent that knew Una.
- **Without them it is slower and noisy:** 11 or 23 runs, depending on how it found the library's API.
  - The voided run probed it through compiler errors.
  - The rerun read the prelude source embedded in the `una` binary with `strings`, which the protocol allows, since the binary is inside `$W`.
- **The guides matter:** shipping them in the archive (#410, N-65) is what makes run A possible.

The cost of the same five tasks, in `una check` runs:

| Run | Check runs |
| --- | --- |
| A, archive with guides | 7 and 7 |
| B, archive without `docs/` | 11 and 23 |
| #402, an agent that knew Una | 12 |

## Protocol, amendment and audit

**Amendment.** The first run B wrote its own temporary files in the runner's session scratchpad, outside `$W`:
- a dump of the 83 `una explain` texts, which it made from the archive's binary;
- a wrapper script;
- its check outputs.

It read only those, never a file of the runner's, but any path outside `$W` voids a run. The cause was the subagent environment's standing advice to use a session scratchpad for temporary files, which overrode "work only inside `$W`". Both runs were therefore made again, with one sentence added after "Do not read, list or search any other directory, and do not search the web":

> This includes any scratch or temp directory your environment suggests; keep temp files under $W/tmp.

The first A is kept as superseded, and the first B as voided.

**Audit.** Every tool call of every run was scanned for a path outside `$W`, after the run and before its results were read. The flagged tokens that were URL pieces or `sed` patterns are not counted.

| Run | Tool uses | Outside `$W` | Verdict |
| --- | --- | --- | --- |
| A | 26 Bash, 5 Read | none; temporary files in `$W/tmp` | clean |
| B | 35 Bash, 1 Read | one Read of a Claude Code persisted-output file (see below) | clean, with a harness exception |
| First A | 24 Bash, 2 Read | none | clean, but superseded by the amendment |
| First B | 47 Bash | its own files in the session scratchpad | **void** |

The harness exception, accepted by the coordinator:
- Run B's `una explain` dump of all 83 codes was too large to show inline, so Claude Code saved it to a `tool-results` file outside `$W` and told the agent to read it there.
- The file held exactly the archive binary's `una explain` output, and the agent could not avoid the read.

Z3 5.1.0 came from `PATH`, as the protocol allows.

**Verification.** On every final package of every run, the runner reran `una check` (all exit 0), `una test` (every example passes) and `una run` for greet.
- No obligation was a `wallclock` unknown, though another session's builds ran during the first runs.
- No package uses `checked`.

## Acceptance

Met by A and by B:

- **greet** prints a line about `USER` through a policy in a trusted module, `internal` to `public`.
  - A prints the name's length, "Hello! Your user name has 10 characters.".
  - B prints the name and its length; the output is shown here as `hello, <user>, you have 10 letters in your name`.
- **orders:** the example checks exactly `{"count":2,"names":["a","b"],"total":5}`, writing its JSON with `\{` and `\"`.
- **stock:**
  - The token is `secret` in the function's own signature, and a policy from `secret` to `internal` carries it to `X-Api-Token`.
  - A's policy also strips newlines and carriage returns.
  - The stand-in API answers `{"count": 7}` only for the right token.
- **tally:** contracts over the map, proven on the one-vote helper.
  - A: `ensures counts.has(key: candidate)`, and a `requires counts.has(key: candidate)` that its caller discharges.
  - B: `ensures counts.len >= old(counts).len` and `counts.len <= old(counts).len + 1`.
  - Neither run uses `checked`, and neither weakens a contract.
- **sensors:** both versions use `together for`. The examples show that the first failure in list order is the one reported, and the count when one sensor fails.

## Did the #402 findings recur?

| Finding of #402 | Fixed by | Run A | Run B |
| --- | --- | --- | --- |
| D-1, an unescaped `{` in JSON text | #403 | not met: the guide's escape list was used | not met |
| D-2, printing `Env` data needs a policy | #404 | not met: written right from the guide | not met: found in `una explain U4003` |
| D-3, a contract across a loop | #404 | not met: the guide's helper paragraph was used | met: U5002, worked around, see N-3 |
| D-4, `Map[Text, Nat]` | #404 | not met: the guide's line was used | not met |
| D-5, a `secret` header value | #404 | not met | met: U4001's `key_name` advice misled, see N-4 |
| D-6, D-7, D-8 | #404, #403 | not met | not met |

## New findings, for #419

Ranked by what they would cost an agent, as in `dogfood.md`.

- **N-1. Without the guides, the library has no reference** (B). The JSON, list and map APIs, `together for`, and the `Http` and `Header` signatures are described nowhere but the guides; run B read them from the binary. This is the finding #410 already answers by shipping the guides. Run B also noted that the README promises `docs/guide/`, which run B's archive lacked by the protocol's design.
- **N-2. The HTTP section is incomplete** (A, both runs). It gives no parameter lists for `post`, `put` and `delete`, does not say that a stand-in for `Http` implements all four operations, and has no section in the syntax reference.
- **N-3. U5002's advice for a loop is not enough on its own** (B). "State what one iteration does as the `ensures` of a helper with an `edit` parameter" left the loop's own postcondition unknown. What the solver proves is the helper's contract, written with `old(...)`, and its callers' use of it. The message should say that the loop's own postcondition still cannot be proven, and show `old(counts)`.
- **N-4. U4001's `key_name` advice does not fit a custom header** (B). It suggests passing a secret token as `key_name`, which sends a bearer token, so it cannot fill `X-Api-Token`. The explanation should name the other route, a policy from `secret` to `internal`.
- **N-5. How labels apply inside a type is learnable only by trial** (B): `internal Option[internal Int]`, a label at each position.
- **N-6. Text escapes have no `\r`** (A, both runs). A carriage return is `\u{D}`, which the guide's escape list should say, since §17 asks for control characters to be removed from header values.
- **N-7. There is no example of an example that expects a failure through `do attempt ...`** (B), nor of a `when` arm that fails while binding (first A).
- **N-8. A one-field variant prints without its field name** (A): `Offline("south")`. The guide's §4 shows `Point(x: 1, y: -2)` and gives no rule.
- **N-9. U1001 (unknown name) suggests no close name** (first B). Una-2 takes it in #419.

## Contents

| Folder | What |
| --- | --- |
| `dogfood-fresh/run-A/` | run A: each task's package (`una.toml`, `verification.lock`, `src/`), its `LOG.md`, and `SUMMARY.md` |
| `dogfood-fresh/run-B/` | run B, the same |
| `dogfood-fresh/superseded-A/` | the first run A, before the amendment |
| `dogfood-fresh/voided-B/` | the first run B, voided |

**Redaction:**
- the user name is written `<user>`;
- each run's directory is written `$W`;
- the runner's scratchpad and the harness's `tool-results` directory are named, not given as paths.

The agents' logs are otherwise as they wrote them.
