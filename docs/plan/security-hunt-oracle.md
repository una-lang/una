# The two-run oracle of the security hunt

- Issue: #513, part (e) of `docs/plan/security-hunt.md`
- Date: 2026-10-10
- Status: specification (L3); the part the `hunt` release gate runs is `scripts/security-hunt/hunt/oracle.py` (#767): V-secret, the interpreter, the `public` and `internal` observers. The rest is #782. The cost-clock mode for tasks and time is §10 (#973)

## 1. What it checks

T6 (`spec/07 §1`) says: two runs whose inputs differ only in values labeled above an observer's label `o` produce the same values and effects observable at `o`, unless a policy released information to `o`. The oracle tests that statement directly, by running programs that `una check` accepts, with no model in the loop. A difference it finds is a concrete witness: two inputs, two runs, and what the observer saw differ.

It complements the re-check (part (b)), which sees only the 18% of the corpus inside F1, and it sees every construct: handlers, `Map`, `together`, `div`, records, payloads, `Http`.

## 2. Variations

A **variation** is a set of inputs to one accepted program that differ only at a known label `v`. The observers it tests are every `o` with `v ⋢ o`; with the levels the corpus uses, those are:

| Variation | Varied inputs | Observers tested |
|---|---|---|
| V-secret | the literal behind `let s: secret T = <literal>` (and `confidential`, `restricted`) | `public` and `internal` |
| V-internal | the literal behind `let s: internal T = <literal>` | `public` |
| V-host | `Files.read` contents, `Env.get` values, key material of `Keys`, `Http` response bodies, statuses and headers (all `internal`, `spec/14 §3`, `spec/24 §5`) | `public` |

- **The edge that flows `public` into a label is legal**, so a program stays accepted when one of its literals moves behind an annotated `let`. The generators of part (a) write these bindings: a source-level variation never changes anything but the literal.
- **V-host needs no change to the source.** Every runnable program that uses `Files`, `Env`, `Keys` or `Http` is a candidate, the examples included.
- **What stays fixed.** It is fixed in every variation because it is `public` input:
  - which environment variables exist (`spec/14 §3`, #487);
  - which files exist, and their kinds, since `Files` failures carry the path's label and the file-state label (ADR 0085), not the content's;
  - the key names;
  - the grants and the flags.
- **Values.** For each varied position, the generator draws:
  - for `Int`, `0`, `1`, `-1`, a large value, and `c - 1`, `c` and `c + 1` for every integer constant `c` of the program;
  - for `Text`, the empty text, one character, the program's text constants, a long text, and non-ASCII text;
  - for `Bool`, both values.
  For host text, invalid UTF-8 is drawn too, and an abort it causes (U6005) is exempt (§5).

## 3. What an observer sees

An observer at `o` sees, in the model, the run's result and failure where they are labeled at most `o`, and the events it can observe (`EvVis`). An event is a call of a capability operation, with its arguments: what the program sends to the host. What a host returns is input, not observation. The oracle's projection follows that.

| Channel | Public observer | Internal observer |
|---|---|---|
| Standard output (`Console.print` accepts `public`) | every line, in order | the same |
| The run's outcome (`una run --json`'s `status`) | `ok` or `failed`; for `failed`, the variant and the `public` fields that `failure` holds | the same |
| Events of operations whose arguments accept `public`: `Console.print`, `Clock.now`, `Env.get`, `Files.read`, `Keys.hmac` with `key_name` only | the operation and its `public` arguments, in order, from `trace` (`spec/11 §5`) | the same |
| `Files.write` (accepts `internal`) | not seen | the final tree of the granted directory: names, kinds and contents |
| `Http` (every argument accepts `internal`, `spec/24 §5`) | not seen | the mock server's ordered log of requests: method, URL, headers, body |

- **Outcomes are not compared.** A trace event's `outcome` is the host's answer, which V-host varies on purpose.
- **Writes have no order.** The internal view of `Files.write` is the final tree, not the sequence of writes. Two runs that write the same files in different orders, or write and then restore a file, are not told apart. The report says so, and the trace's write events, whose arguments are redacted at `internal`, give their count and position for a later refinement.
- **Redaction already projects to `public`.** The trace prints an argument labeled above `public` as `<redacted:level>`. The public view keeps the redaction marker, because which level is printed is a static fact, the same in both runs. A marker that differs between runs is a finding.
- **Policies.** A release (`spec/07 §5`) is allowed to make runs differ at the policy's `to`. The generators either write no policy into a program they vary, or vary a value only where no policy call depends on it, which the generator knows by construction. A difference that a policy explains is triaged as such and never counted.

## 4. Runs

For every program and variation:

1. **Check.** Run `una check --json`. Only an accepted program goes on. A rejected one feeds part (c).
2. **Determinism check.** Each variant runs twice, alone. If a variant's two runs differ in what it shows any observer, it is nondeterministic (for example `Clock.now`, task order in `together`, a network response), and the program is set aside with the reason. Nothing it shows is a finding.
3. **Runs.** Every variant runs on the interpreter, `una run --json` with the grants of §6. A sample of about 5% is also built with `una build` and run natively, with the same grants and flags. The `hunt` release gate runs this sample since #792 (`scripts/security-hunt/hunt/native.py`, `docs/release-gates.md`).
4. **State that outlives a run.** For programs that use `Files.write`, each variant also runs a second time on the directory its first run left. S-15's channel, where a write decides a later run's failure, is visible only that way.
5. **Comparison.** Two variants are compared by the projection of §3, for every observer the variation tests. Separately, the interpreter's and the native run of one variant are compared on everything they show. A mismatch there breaks `spec/16 §5` and is reported as a backend divergence, which is a conformance bug, and a leak only if the native run is the one that differs where the interpreter does not.

## 5. Outcomes

| Result | Class | What happens |
|---|---|---|
| The projections differ, both runs finished as `ok` or `failed`, the program is deterministic | **candidate leak** | minimized and triaged (§7) |
| One run ends by timeout, and the other does not | termination channel, outside the model (`spec/07 §1`) | logged and counted, not a finding |
| One run aborts with U6003, U6004 or U6005 | abort, outside the model | logged and counted, not a finding |
| A run exits with code 3 (U8001), or the compiler crashes | robustness bug | an issue, not a leak |
| The interpreter and native disagree | backend divergence | an issue (§4 step 5) |
| A variant is nondeterministic | set aside | counted with its reason |

The aborts and timeouts are counted, so that a program whose variants keep aborting does not hide the rest of its coverage.

## 6. Sandbox (condition 1 of the plan's approval)

Every run happens inside a sandbox:

- **A fresh directory per run** under the oracle's own temporary root, created with `mktemp -d` and removed by a `${VAR:?}`-guarded path when the run's evidence has been copied. The only `Files` grant is `files:<that directory>:read,write`; the grant's prefix rules (`spec/14 §3`) confine every path to it.
- **No network but the mock.** The process runs under `sandbox-exec` with a profile that denies all network traffic except loopback to the mock server's port, and denies file writes outside the run's directory and the oracle's temporary root. Grants for `Http` name only `http://127.0.0.1:<port>`.
- **The mock Http server** is local, binds to loopback only, and answers from a table that the variation sets: deterministic statuses, headers and bodies per request. It logs every request for the internal view. It never forwards a request.
- **A clean environment.** Each run starts with `env -i`, plus only the variables that the variation defines. Key files are written fresh into the run's directory.
- **Limits.** Each run has a wall-clock timeout, and `--max-depth` and `--max-memory` set low enough that a run cannot hurt the machine.
- **On Linux** (#793), the gate runs in a hardened container (`scripts/hunt-linux.sh`: no network, read-only root, no capability, not root), and Landlock replaces `sandbox-exec`: each run may write only under its own directory and connect only to the mock's port. `docs/release-gates.md` ("The Linux sandbox") states the rules and when the gate refuses to run.

## 7. Triage of a candidate

1. **Minimize.** Delta debugging removes declarations, statements and expressions from the source. A step is kept while the program stays accepted, the variation still differs, and the difference stays in the same channel.
2. **Classify:**
   - a real leak, which is reported privately first, with the minimized repro, before any fix;
   - a channel outside the model that the projection did not exempt, which is a fix to the projection, with a note;
   - a release that a policy explains, which is a fix to the generator.
3. **Record.** Every candidate is kept with its seed, the program, the variation and both runs' evidence.
4. **A confirmed leak** follows the S-xx process: a repro under `tests/conformance/security`, an ADR when a rule changes, the fix in both editions, and patch releases.

## 8. Cost

The machine work is per program and variation: one check, then for each variant two determinism runs and one compared run (two when `Files.write` is used), and natively for the sample. With about four values per varied position, a program costs about 13 interpreter runs. Runs are small, so a round's cost is dominated by the number of runs, and is measured on the first 500 programs before any round is planned. All of it is heavy work (condition 3 of its approval): only in a machine slot between benchmark runs.

## 9. Limits

- It finds only what its inputs exercise. Its strength depends on the generators of part (a) and on the values of §2.
- The final-tree view of `Files.write` does not see the order of writes (§3).
- Nondeterministic programs are set aside, not tested.
- A difference that is outside the model by `spec/07 §1` is never a finding, even when it is real: timing, termination, resources, aborts and the order of effects across tasks. Deadlines, races and limits are in the model through their timing labels: the cost clock of §10 tests them.

## 10. The cost clock (#973)

ADR 0117 (implementation plan step 7, S8) adds a mode of the oracle for the constructs of time: `within`, `first of`, `tasks` and the limits of `std.limit`. On the real clock their outcomes depend on how long work takes, which the oracle cannot vary reproducibly; on the plain virtual clock work takes no time, so no deadline passes and a timing label is never put to the test.

- **The clock.** Every run of the mode is `una run --virtual-time --cost-clock` (`spec/27 §8` rule 6): each check point of the interpreter, each call, operation and loop iteration, moves the virtual clock by one millisecond. A deadline then passes, and a token is left at a limit, by how much work the run did. When a secret decides that work, the timing label `τ` of the construct (`spec/27 §5` rule 8, §6 rule 8, §10 rule 4) must carry it; a checker that missed it would accept a program whose public values differ between two secrets, and the comparison of §4 sees the difference.
- **What is not compared.** The trace's events with no operation whose outcome is `timeout` or `cancelled`: whether a `within` timed out and which tasks its cut or a race cancelled are the timing information the trace records by design (`spec/27 §5` rule 7, `spec/26 §7` rule 4). Every other part of §3 is compared, outputs, failures and the events of operations included.
- **Set aside.** A program that names `Clock`: under the cost clock `Clock.now` reads the run's work, timing outside the model (`spec/07 §1`).
- **Interpreter only.** Executables have no cost clock, so the mode has no native sample.
- **Its programs.** Family W (`hunt/families/timing.py`), edition `0.4`: W1 `within`, W2 `first of`, W3 `tasks` and `start`, W4 `std.limit`, each with one varied secret literal. Accepted forms keep every timing outcome at the secret's label; closed calls are the same programs with that label missed, which a correct checker rejects (an acceptance is a finding of must-reject). Every loop on the secret is clamped, so no varied value makes a run long.
- **The run.** `python3 -B -m hunt timing --una PATH --out DIR` (`hunt/costclock.py`): seed 973, 40 programs per operator, through must-reject and the oracle in this mode, with two canaries: a program whose `within` runs a loop on a public literal that its manifest marks as secret must be a leak under the cost clock and clean on the plain virtual clock, so a cost clock that does not move fails the run. It fails closed, as the gate does, and every operator with an accepted form must compare at least one program. It is not the release gate's fixed run, and leaves it unchanged: family W is kept apart from the operators that run (`TIMING_OPERATORS`).
- **Limits.** Under virtual time a `first of` runs its branches in index order, each to its end (`spec/27 §6` rule 4), so the cost clock does not choose its winner: W2 tests a race through the `within` around it, and through the timing floor of its branches. The task switch after 1024 check points (`spec/27 §8` rule 2) counts check points, not the clock, and is the same in both modes.

## 11. Servers (#1011)

ADR 0123 (implementation plan step 10, S9) asks for a run of the oracle over handlers that read labeled data. A mode of the oracle runs servers (`oracle.py`, `serve=True`):

- **The runs.** Every run serves the program's script of connections, `una run --virtual-time --serve-script script.jsonl --serve-out ...` (`spec/31 §8` rule 6), with `UNA_TEST_RUN_SERVER=1`. Only a test build of `una` serves a script, and none binds a listener: the scripted engine stands for the peers, and the sandbox denies every network operation.
- **What an observer sees.** `reply` accepts `internal` (ADR 0123 IF2), so the observer at `internal` sees every line of `--serve-out`, the bytes each peer received and their instants, and the one at `public` does not. The operations of `HttpServer` join the table of `accepts` (`next` and `reply` at `internal`, `close` at `public`), in this mode only. Two runs of one variant must agree on the wire too.
- **Set aside.** A program whose policy releases to `internal` is compared at `public` only: what its peers see is what the policy released. As before, one that releases to `public` is set aside.
- **Interpreter only.** The native scripted engine is a runtime test hook (`spec/31 §8` rule 6), so the mode has no native sample.
- **Its programs.** Family N6 (`hunt/families/servers.py`), edition `0.4`: a server in the language form whose request task reads a `secret` balance from a capability of the program, whose handler returns one varied secret literal, or binds a secret literal itself. Accepted forms keep the reply at `internal`: the balance read and dropped, a local secret computed on and dropped, or the balance released through a policy to `internal`. Closed calls miss the label: the balance in the body or in a header (U4001), a status it chooses (U4001), a reply under its condition (U4003). A correct checker rejects them; one that accepted them would put the secret on the wire.
- **The run.** `python3 -B -m hunt servers` (`hunt/serving.py`) sends N6 through must-reject and the oracle in this mode, beside the wire judges of N1 to N5, with a canary: a server whose reply carries a `public` literal that its manifest marks as secret must be a leak at `internal`, and only there, so the wire is compared and no other view shows it. Every accepted form must compare at least one program.
- **Limits.** The script is fixed, so the oracle varies source literals only, never the requests (V-host); what a peer measures of timing stays outside the model (ADR 0123 IF5).

## 12. The example probe (#1911)

Since S-41 and S-42 (`spec/11 §5`, `spec/26 §6` rule 4; #1514, #1515, #1544) a run's trace is public by construction: an operation performed in quiet code, under a program counter above `public`, appends no event, and neither does any operation of a handler instance that quiet code installed. The trace of `una run --json` therefore no longer shows what an observer at the literal's own level sees of such operations, in any variant, and the views of §3 read from it alone can neither find a program live (§5) nor see an operation that a checker wrongly let run under a secret branch: the runtime hides it whatever the checker said. An example's trace keeps the operations performed in quiet code, for `expect performed` (`spec/08 §2` rule 7), though never those of instances quiet code installed, which no trace shows.

- **The probe.** Each variant also runs once as an example (`oracle.run_probe`): the entry module (`[run] entry`) gets a handler of `Console` that does nothing and a test (`example` in editions 0.1 and 0.2, `test` from 0.3) that gives it as `@console` and runs `main()`, and `una test --json` reports its trace, with the limits and the solver flags of the runs, in the sandbox, in a directory of its own.
- **What it adds to a view.** The probe's events join every observer's view, projected by `accepts` exactly as the run's trace is (§3, `EvVis`), their redacted sinks counted as violations, and the liveness view at the literal's own level, with their outcomes. The test's own result does not join any view: the test report of edition 0.2 and later writes the labeled fields of a failure unredacted (ADR 0078), so a failure's payload would show the literal at `public` where the run redacts it. A policy that releases to `internal` keeps the probe's events out of the `internal` view, as the server mode compares such a program at `public` only (§11). The run's own views are unchanged: a leak the run shows is still a leak, and the probe's is one more.
- **Unavailable.** A program whose `main` needs a host capability other than `Console` has no probe: an example cannot use host capabilities (U3014, `spec/08 §2` rule 1). The result records the reason; the gate pins the reasons and their counts (`PINNED_PROBE_UNAVAILABLE`), so a probe lost for a new reason, or for more programs, fails it. A program with no `[run] entry` has none either.
- **Errors.** A probe whose check the solver did not decide (U5002, U5003), that aborts or times out while the run completed, whose report is not a test document or does not hold exactly one probe test, that reports a diagnostic other than an error, or that is available for the base and not for a variant (or the reverse) is an error of the program: the probe is a view of the program, and its availability cannot depend on the literal.
- **Modes.** The cost-clock mode (§10) and the server mode (§11) run no probe: an example has neither clock nor peers; their views are as before.
- **Liveness.** A program is live when two of its complete ends differ at the literal's own level, in the run's view or in the probe's. Of the gate's forms, those whose secret decides which operations of an instance installed under a public program counter run (`H3:reach-under-pc`, `C6:secret-fan-out`) are live through the probe; `H3:install-under-pc`, whose secret decides an installation and nothing else any trace shows, is compared and not live (`docs/release-gates.md`, the hunt gate).
- **Canary.** The `internal` canary of the gate installs its handler in `main` and performs the planted operation under a secret branch: only the probe can show it, at `internal` alone, so a probe that goes missing leaves the canary uncaught and fails the gate.
