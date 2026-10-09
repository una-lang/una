# 0085. The file-state label, and the failure of an HTTP request

- Status: Accepted
- Date: 2026-10-09
- Deciders: coordinator under the standing security rule (2026-10-09), for the owner's review; written by Claude (Una-1)
- Resolves: #436 (S-14), #437 (S-15)
- Amends: `spec/07 §7` rule 4, `spec/14 §3` (labels of failures), `spec/24 §5` rule 3

## Context

N-39 and N-40 label the failure of a host operation by the arguments that decide it, capped at what the operation accepts (`spec/07 §7` rule 4). Two channels escaped that rule.

**S-14: the file system is state.** Found by the coordinator, and confirmed by running it in both editions on v0.3.3 and v0.1.5:

```una
let p = @env.get("SECRET_PATH")
  if none use "b.txt"
let w = attempt @files.write(p, content: "x")
let r = attempt @files.read("a.txt")
when r
  is Ok(v) -> @console.print("secret path was a.txt")
  is Err(e) -> @console.print("secret path was not a.txt")
```

- The read's failure was labeled by its own path, which is public, so branching on it was public.
- But whether `a.txt` exists depended on where the `internal` write landed.
- A write also decides whether a later write fails: a write that creates the file `a` makes a later write to `a/b` fail.
- The same holds for a write that runs only under an `internal` condition, and across runs, since files outlive the run that writes them.

**S-15: the server decides some failures.** An `Http` request's failure was labeled by its URL, header names and key name, not by its body or its header values. But the server receives all of them, and `TooLarge`, `Protocol` and `TimedOut` are its decisions. So a failure could reveal an `internal` body or header value to a public branch.

The owner's rule is the most secure option that is precise enough to be usable. The cost of each option was measured as the programs it newly rejects at tier 0, against `main` 817efaf. The corpus is:
- the 1,303 conformance cases and examples;
- suite-d;
- the 64 una-bench task packages, assembled as `harness.bench.validate` assembles them: scaffold, then reference or complying, then oracle.

## Options for Files

| Option | Rule | Corpus | una-bench |
| --- | --- | --- | --- |
| F-A | every `Files` failure is `internal` in a package that calls `Files.write` | 3 | 4 (B13, B14: reference and complying) |
| F-B | the file-state label below, with each write's content too | 3 | 4 (the same) |
| **F-B′** | **the file-state label below, without content** | **1** (`examples/journal`) | **0** |
| F-C | a flow-sensitive file-state label, threaded through the program like the program counter | at most F-B′'s | 0 |

F-A and F-B reject B13's reference, which reads a summary file and writes it back with a line added. That is honest code. The text it writes back is `internal` because a read returns `internal` text.

F-C is more precise, since a read before every labeled write would stay public. But threading a store label through calls, function values and handlers is a larger change, and F-B′ already rejects nothing in una-bench. It is deferred.

## Decision

1. **The file-state label** of a package is the join, over every call of `Files.write` in the package, of:
   - the labels of its path;
   - the program counter at the call;
   - the highest program counter the code around the call can be entered under:
     - for a function, the join of the program counters of its direct calls, each raised by its caller's own, up to a fixpoint over the call graph;
     - for a function used as a value, and for a policy's `via` function, which may be called anywhere: its effect bound (`spec/07 §7` rule 4: the meet of the `accepts` labels of its effects), or `public` when it can fail, since a caller runs it only under a program counter below that;
     - for a handler operation, its effect floor;
     - for a lambda, `internal`;
     - for an example, `public`.

   It is capped at `internal`. `Files.write` accepts at most `internal` for its path (`spec/14 §3`: "`write(path: Text, content: Text) -> Unit` | `FileError` | `internal`"). So a higher path is U4003 and a higher program counter U4002 already; conformance case `security/s14_secret_write_rejected` checks both.

2. **Every `Files` operation carries it.** The failure of every `Files.read` and `Files.write` in the package is also labeled by the file-state label, and so is what depends on the failure: an `attempt`'s `Result`, and a branch on it. `Files` has no other operation (`spec/14 §3`), and neither has a `Bool` or `Option` outcome.

3. **Content is not part of it.**
   - Which files exist, and whether a later operation fails, depends on where writes land and whether they run, not on what they write.
   - Content reaches a later outcome only through resource consumption, such as `Io` on a full disk, and resource consumption is outside the model (`spec/07 §1`).
   - The other failure that content could decide is a read of invalid UTF-8. Una cannot write it: `Files.write` writes the UTF-8 encoding of a `Text`, in both engines.
   - A later read returns the content `internal` anyway.
   - F-B, which counts content, is rejected for what it costs: B13 and B14 in una-bench.

4. **Across runs.** The label is static and joins every write of the package, whatever run it happens in. So a previous run's write to an `internal` path labels this run's reads too. Writes by other programs or people are outside the model (`spec/07 §1`, external systems).

5. **Computing it.** Labels are checked in pass 2, and a read may be checked before the write that decides it. So when pass 1 meets a `Files.write`, pass 2 is first run as a probe, with its diagnostics dropped, to compute the label; then pass 2 runs with it.
   - The probe's label is the least fixpoint. Labels are joins, so what the file-state label raises in pass 2 (a read's failure used as a condition, for instance) adds only itself.
   - After pass 2, a label above the probe's is U8001, never a silent miss.

6. **Http.** Every argument of an `Http` operation labels its failure: URL, headers with their values, body and key name, capped at what the operation accepts.
   - `Header` declares its value `internal`, so every request's failure is `internal`, as its response is.
   - `attempt` gives an `internal` `Result`, and `if error pass` and `if error fail` are U4002.

7. **Editions.** Both. This is a security fix: edition `0.1` gets it too, in v0.1.6 for the una-bench pin.

## Consequences

- The probe program of S-14, and every shape in the conformance cases, is rejected in both editions:
  - a write then a read (`s14_write_then_read`);
  - a write then the count of the files that exist (`s14_write_then_count`);
  - a write performed by a helper called under a condition (`s14_write_under_condition`);
  - a read checked before the write (`s14_read_before_write`);
  - the request failures of S-15 (`s15_*`).

  Every one of them was accepted before. Their twins with public writes, a failing writer, or `attempt` stay accepted.
- `examples/journal` copies its entry to a path that `JOURNAL_COPY` names. It now reads with `if error use ""`, since a previous run's copy could decide whether the file exists.
- Every `Http` request is handled with `attempt`, as the guide's examples already were. The one conformance case that passed a request's failure on (`names/import_std_host_http`) now returns it.
- The bound alone would have been simpler, but too coarse: a helper that only writes and keeps its failure has the bound `internal`, so it would have labeled every read of a package that calls it only at a public program counter. Two native-backend tests of the repository have that shape (`worker()` in `crates/una-cli/tests/native.rs`). The call sites decide instead, and conformance cases `s14_ok_helper_public_call` and `s14_writer_as_value` pin both sides.
- Complexity: one more pass 2, only for packages that call `Files.write`. The fixpoint over the call graph is O(c · h): c direct calls, and h the height of the label lattice up to `internal`, since each round raises some function's label.

## Alternatives considered

- F-A, F-B and F-C, above.
- **Label header values by the expressions that build them**, so a request with public header values keeps a public failure. `Header` declares its value `internal`, so the label at the call is `internal` whatever was put in. Telling them apart would need a different `Header` type. Deferred: no program of the corpus needs it.
