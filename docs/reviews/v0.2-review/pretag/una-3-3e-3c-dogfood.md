# Pre-tag review v0.2.0: areas 3e, 3c and the native dogfood (Una-3)

Date: 2026-10-06. Candidate: `ef34f2f604657572da0c8ec97612f2e923549ef8`. Baseline: v0.1.3 (`df2c495`, the Gate 1 pin). Built from `git archive` in a scratch directory with its own target (`cargo build --release --locked -p una-cli -p una-rt`), on aarch64-apple-darwin with Z3 5.1.0. Nothing in either repository was edited during the runs.

## Summary

| Area | Result | Findings |
| --- | --- | --- |
| 3e, Gate 1 references | Identical to v0.1.3 | none |
| 3e, edition 0.1 conformance diff | 684 of 692 identical; 7 differences expected, 4 not | 3e-1 (Minor, #256, N-03) |
| 3c, streams | Engines agree on exit code, stdout and trace in every combination | 3c-1 (#259, after the tag, N-06), 3c-2 (documented in spec/16 §3, N-06) |
| Native dogfood | 51/51 identical, as on 3f3cad1 | none |
| Scaling probes | Linear except text concatenation | S-1 (#260, ADR 0071, N-07), S-2 (#261, ADR 0070 X10, N-08) |

## 3e. Edition 0.1 isolation

### Gate 1 references and Suite D packets

The comparison covered 150 packages:
- the 48 tasks as reference (scaffold, reference and hidden oracle), stub (scaffold and oracle) and scaffold alone;
- the Una packets P1, P4 and P7 (`after/`);
- the 3 frozen Suite D base scaffolds.

`una --json check .` and `una --json test .` give byte-identical stdout and identical exit codes under v0.1.3 and the candidate on all 150.

The una-bench harness (`harness.bench.oracle.run_oracle`, Una side) gives the same result under both binaries on all 48 tasks:
- the visible check passes;
- 571 of 571 hidden cases pass;
- every case result, trace and predicate is identical;
- the stub results are identical.

### Conformance diff

`una --json check` covered 692 edition 0.1 targets of the candidate tree: every single-file conformance case (a single file is edition 0.1), and every package of `tests/conformance`, `tests/demo` and `examples` whose `una.toml` says `edition = "0.1"`. It ran under v0.1.3 and under the candidate, each target in its own copy. 684 targets are byte-identical.

Expected differences:

| Case | Change | Source |
| --- | --- | --- |
| `security/decided_argument_to_function_0_1` | 0 to 1, new U4003 at 14 | #253, N-01 (new case) |
| `syntax/reserved_result_as_binding.una` | U0101 at the same position; new message saying `result` is reserved; rename repair | #206 |
| `syntax/keyword_as_name.una` | U0101 at the same position; new message "`if` is a reserved keyword"; rename repair | #206 (06a86df) |
| `types/variant_compared_without_fields.una` | U2009 unchanged; new repair "write `e is Overdrawn`" | #205 |

#211 changes none of the edition 0.1 targets.

**Finding 3e-1 (Minor; decided N-03; fixed by #256 before the tag).** The U3004 message changed in edition 0.1, in commit 539b4ca (Refs #36). It affects `authority/lambda_in_list.una`, `authority/lambda_into_pure_param.una`, `authority/attenuated_binding_repair_call.una` and `types/effect_polymorphism_in_0_1`. The code, the position and the exit code are unchanged.

| | Message |
| --- | --- |
| v0.1.3 | "`@log` is used here, but this code declares no `uses`, so it is pure; declare `uses @log: ...` in its header" |
| Candidate | "`@log` is used in a lambda whose type declares no `uses`, so the lambda is pure; pass it directly as an argument for a parameter whose type allows the effect" |

ADR 0062 says that edition 0.1 keeps today's rules. The new advice also misleads in 0.1: in `effect_polymorphism_in_0_1` the lambda is already passed directly to `map`, which is pure in edition 0.1. Delta check for #256: only these 4 cases may change, back to the v0.1.3 text plus a hint about edition 0.2.

## 3c. Runtime and CLI streams

### Matrix

| Dimension | Values |
| --- | --- |
| Programs | prints 3 lines and returns; prints a line then fails (U6001); fails without printing; returns without output; one passing and one failing example |
| Commands | `una run`, `una --json run`, the native executable with `--trace <file>`, `una test`, `una --json test` |
| stdout | open, closed (fd closed, EBADF), broken pipe (EPIPE) |
| stderr | open, closed (EBADF), broken pipe (EPIPE) |

All 9 stream combinations ran for every program.

### Results

- **Exit codes:** `una run` and the native executable agree in all 36 run cells.
- **Stdout:** identical wherever it is observable.
- **Traces:** the native trace equals the `una --json run` trace wherever the latter is observable.
- **Closed stdout:** the native trace file stays valid JSON, so there is no fd reuse into the trace.
- **EPIPE on a stream that is written:** every engine prints "error: cannot write the output: Broken pipe (os error 32)" and exits 2 (spec/13 §5).

**Finding 3c-1 (decided N-06; #259, fixed after the tag; known issue in the CHANGELOG).** A closed stdout or stderr (EBADF) is treated as success by every engine. spec/13 §5 says "A failure to write output exits with code 2", and EPIPE already does that.
- `una run . 1>&-` on a program that prints exits 0. The native executable does the same, and its trace records each print as ok.
- `una --json run . 1>&-` and `una test . 1>&-` lose the whole document silently.
- With `2>&-`, U6001 is lost silently, still with exit 1.
- On EPIPE, the run continues after the failed write, and the trace records the prints as ok before the exit 2.

**Finding 3c-2 (decided N-06: allowed, documented in spec/16 §3).** The engines render U6001 differently on stderr:
- `una run` prints the snippet form (`[U6001] Error: ...` with a source frame and "Note: run `una explain U6001` ...");
- the native executable prints `[U6001] Error: ...`, `  at src/s/main.una:5:5` and `  note: ...`.

The first line, the code, the position and the failure are identical.

## Native dogfood rerun

`scripts/native-dogfood` from una-bench was rerun on the candidate. Every task and Una packet gets a generated `main` that runs each example. The results are identical to the run on 3f3cad1 (una-bench `findings/v0.2-native-dogfood.md`):
- `una build` succeeds on all 51 drivers;
- 834 examples give the same stdout, exit code and trace under `una run` and native;
- the 19 file-system run cases of B13 and B14 have the same exit code, output, trace and file tree;
- the negative control on the stubs agrees.

One P4 example is unobservable from a public `main`. It depends on a `secret` value (U4001), as designed.

Timing on the 51 packages:

| Measure | v0.1.3 | Candidate |
| --- | --- | --- |
| `una check`, sum | 1.5 s | 1.6 s |
| `una test`, sum | 1.2 s | 1.2 s |

The drivers take 2.1 s in the interpreter and 0.5 s natively.

## Scaling probes (edition 0.2)

Each value is the best of 3 runs after a warm-up, in seconds. A warm-up is needed because macOS checks a freshly built executable on its first run, which adds about 0.35 s.

| Probe | Sizes | Interpreter (s) | Native (s) |
| --- | --- | --- | --- |
| `xs = xs.append(item: i)` in a `for` loop | 80k / 160k / 320k | 0.041 / 0.107 / 0.138 | 0.022 / 0.042 / 0.075 |
| `xs.at(index: i)` over every index | 20k / 40k / 80k | 0.050 / 0.063 / 0.111 | 0.016 / 0.025 / 0.048 |
| `set` in a loop, with the guard | 20k / 40k / 80k | 0.048 / 0.070 / 0.134 | 0.026 / 0.040 / 0.059 |
| `fold` | 20k / 40k / 80k | 0.024 / 0.038 / 0.069 | 0.012 / 0.015 / 0.023 |
| `t = "{t}x"` in a `for` loop | 40k / 80k / 160k / 320k | 0.14 / 0.44 to 0.80 / 1.83 / 4.43 | 0.06 / 0.23 to 0.31 / 1.16 / 2.33 |

**Finding S-1 (decided N-07; ADR 0071, #260, P0 for Beta; v0.2.0 documents the limit).** The natural in-place update loop fails verification:

```
pub bump(edit xs: List[Int]) -> Unit
=
  for i in 0..<xs.len
    set(edit xs, index: i, item: i + 1)
```

- It gives U5001 "precondition `0 <= i and i < xs.len` of this call is refuted", with the counterexample `i = 0, xs.len = 0`.
- The cause is spec/17 §5 rule 3: a binding the loop updates is fresh at its declared type in the body, so its length is unknown there.
- The workaround is a redundant guard written as a statement: `if i < xs.len`, with `set` on the next line.

**Finding S-2 (decided N-08; ADR 0070 X10, accepted but not yet implemented; #261, P0 for Una-2 after the tag; a known limit in v0.2.0).** Building text by concatenation in a loop is about quadratic in both engines. From 40k to 320k steps (8x), the time grows 32x interpreted and 37x native, while the same number of list appends stays linear. The expected cost, under the average-complexity rule, is amortized O(1) per step for a uniquely referenced `let edit` text, as for `append`.

## Reproduction

The scripts live in Una-3's scratch directory and are available on request:
- `cmpgate.py` and `harnessgate.py`: the Gate 1 comparison;
- `cmp3e.py`: the conformance diff;
- `streams/matrix.py`: the stream matrix;
- `scale.py`: the scaling probes, committed next to this file as #261's acceptance probe. Usage: `python3 scale.py <una> 40000,80000,160000,320000 text_concat,append`. It needs Z3 at `~/Projects/una-lang/una/target/z3/bin` and the una-bench lock file it copies, so adjust `LOCK` and `ENV` for another checkout.

The dogfood scripts are in una-bench at `scripts/native-dogfood/`.
