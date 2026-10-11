# The two-run oracle of the security hunt

- Issue: #513, part (e) of `docs/plan/security-hunt.md`
- Date: 2026-10-10
- Status: specification (L3), no code yet

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
3. **Runs.** Every variant runs on the interpreter, `una run --json` with the grants of §6. A sample of about 5% is also built with `una build` and run natively, with the same grants and flags.
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

## 6. Sandbox (the coordinator's condition 1)

Every run happens inside a sandbox:

- **A fresh directory per run** under the oracle's own temporary root, created with `mktemp -d` and removed by a `${VAR:?}`-guarded path when the run's evidence has been copied. The only `Files` grant is `files:<that directory>:read,write`; the grant's prefix rules (`spec/14 §3`) confine every path to it.
- **No network but the mock.** The process runs under `sandbox-exec` with a profile that denies all network traffic except loopback to the mock server's port, and denies file writes outside the run's directory and the oracle's temporary root. Grants for `Http` name only `http://127.0.0.1:<port>`.
- **The mock Http server** is local, binds to loopback only, and answers from a table that the variation sets: deterministic statuses, headers and bodies per request. It logs every request for the internal view. It never forwards a request.
- **A clean environment.** Each run starts with `env -i`, plus only the variables that the variation defines. Key files are written fresh into the run's directory.
- **Limits.** Each run has a wall-clock timeout, and `--max-depth` and `--max-memory` set low enough that a run cannot hurt the machine.

## 7. Triage of a candidate

1. **Minimize.** Delta debugging removes declarations, statements and expressions from the source. A step is kept while the program stays accepted, the variation still differs, and the difference stays in the same channel.
2. **Classify:**
   - a real leak, which goes to the coordinator first, with the minimized repro, before any fix;
   - a channel outside the model that the projection did not exempt, which is a fix to the projection, with a note;
   - a release that a policy explains, which is a fix to the generator.
3. **Record.** Every candidate is kept with its seed, the program, the variation and both runs' evidence.
4. **A confirmed leak** follows the S-xx process: a repro under `tests/conformance/security`, an ADR when a rule changes, the fix in both editions, and patch releases.

## 8. Cost

The machine work is per program and variation: one check, then for each variant two determinism runs and one compared run (two when `Files.write` is used), and natively for the sample. With about four values per varied position, a program costs about 13 interpreter runs. Runs are small, so a round's cost is dominated by the number of runs, and is measured on the first 500 programs before any round is planned. All of it is heavy work (the coordinator's condition 3): only in a machine slot between benchmark runs.

## 9. Limits

- It finds only what its inputs exercise. Its strength depends on the generators of part (a) and on the values of §2.
- The final-tree view of `Files.write` does not see the order of writes (§3).
- Nondeterministic programs are set aside, not tested.
- A difference that is outside the model by `spec/07 §1` is never a finding, even when it is real: timing, termination, resources, aborts and the order of effects across tasks.
