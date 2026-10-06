# Review of the native backend and runtime: summary

- **Issue:** #176. Branch `review/176-native`, 2026-10-06.
- **Reviewed:**
  - the native backend, `una build` and the runtime (`spec/16`, ADRs 0055, 0058, 0059, 0061), first on `main` at `0e080a3`;
  - native `together` (PR #175, #162);
  - the fixes of the first round's findings: PRs #191 (MEM-1), #192 (MEM-2), #194 (INT-1), #195 (HOST-1) and #196 (HOST-2).
- **Binaries:** `main` at `9a4ab9c` for #191, #192 and #175 (both memory fixes merged); each open PR at its head for #194 (`6593ff8`), #195 (`a94778d`) and #196 (`57a7ae8`). Every case was rechecked on `main` at `7bfb838` before this commit.
- **Method:** each case is a package that passes `una check` at tier 1 in canonical form. The interpreter is the reference (`spec/16 §1` rule 4): a case compares `una run` with the native executable on stdout, stderr, exit code and trace (`adversarial/host/cmp.sh`).
- **Evidence:** only what this reviewer reproduced.

## Verdicts

| Item | Verdict | Open findings |
| --- | --- | --- |
| PR #191, MEM-1 (#181) | **Clean** | none |
| PR #192, MEM-2 (#182) | **Clean** | none |
| PR #194, INT-1 (#180) | **Clean** at `6593ff8` | none; the owner's rule A' (below) changes the PR, so its new head needs a recheck |
| PR #195, HOST-1 (#183) | **Clean** | none |
| PR #196, HOST-2 (#184) | **Not clean (minor)** | #198: a failed write to stderr |
| PR #175, native `together` (#162) | **Not clean (minor)** | #200: a cancelled handler operation traced as `aborted` |
| **Native backend for v0.2.0** | **Not ready** until #200, #198 and #194 land | 0 Critical, 0 Major open; #200 (P1), #198 |

The first round found three Criticals (INT-1, MEM-1, MEM-2) and two Majors (HOST-1, HOST-2), filed as #180 to #184, with the minors in #185. All five have fixes, and no fix let a new variant of its class through, except the stderr half of HOST-2.

## First-round findings and their fixes

### MEM-1 (#181, Critical): in-place reuse through a dead alias

`reusable()` checked only the first name holding a word, so after `let z = a` a constructor could reuse `z`'s object while `z` was live. Fixed by #191: a word is reused or moved only when no alias of it is live.

- **Reruns:** `memory/stale_alias_reuse`, `_option`, `_freed_fields` and `_cycle` match.
- **New variants, all matching:**
  - `memory/recheck_aliases`: an alias through `take`, an alias bound in a `when` arm, lambdas capturing an alias that escape into a list, an alias in a list element, in `Some` and in `Ok`, and an alias stored by a function before an in-place edit.
  - `memory/recheck_joins`: an alias that is the value of an `if` or a `when`, a field projection whose holder dies, loop state that keeps itself on some iterations, and captures in `together` branches and in a `together for`.
  - `memory/recheck_inplace`: in-place updates through an alias from an `if`, from a list's head, beside a lambda capture, after `take`, through an `edit` call, through a nested field, and on loop state appended to a list.
  - `memory/recheck_handlers`: a handler instance whose argument is an alias, while points of the same kind are built and edited.

### MEM-2 (#182, Critical): a loop's range bound released while the loop compares with it

Fixed by #192: a range loop keeps its high bound for the whole loop.

- **Rerun:** `memory/loop_bound_is_state` matches.
- **New variants, all matching:**
  - `memory/recheck_bounds`: a high bound from a field the body updates, bounds from calls, empty `..` and `..<` ranges, nested loops over big integers where both bounds change, a failure out of the inner loop, a `together for` over a big range, `U8` `250..255` with the low binding reassigned, and `I64` loops at the maximum and the minimum.
  - `memory/recheck_lists`: a `for` over a list field the body replaces, a list the body appends to, and an inner list emptied during its own loop.

### INT-1 (#180, Critical): machine arithmetic in reflect predicate bodies

The verifier makes no overflow obligation in a `predicate reflect` body and the interpreter computes it exactly, but native compiled it at machine width. PR #194 compiles reflect predicates, and the predicate of `checked`, exactly.

The owner decided the rule as A': a reflect body makes no overflow obligation and no U2022, and its arithmetic is exact; U2001 still applies. #194 is being changed to it, so this verdict holds for `6593ff8` and the new head needs a recheck.

- **Reruns:** `integers/reflect-predicate-fixed-overflow` and `integers/checked-predicate-fixed-overflow` match.
- **Probes:**
  - A non-reflect predicate must be `opaque`, and the verifier walks its body with overflow obligations: `predicate opaque p(x: U32) -> Bool = x * 4294967295 > 4294967295` is U5001. So machine width is sound there.
  - Native never evaluates `where`, `requires` or `ensures` (`spec/18 §2.4`); only `checked` evaluates a predicate at run time.
  - A reflect predicate calls only reflect predicates (an opaque one is U2016). In `checked`, arithmetic is `Int`, so an out-of-range value cannot reach a fixed-width parameter (U2001).
  - Lambdas, `if` and generics are outside the refinement language (U2014, U2015), so a lambda inheriting exactness is not reachable from source.
- **New variants, all matching:** `integers/recheck_exact_widths` (U64, `I64` minimum through negation and `/ -1`, U8, I8 `mod`, I16, nested reflect calls), `recheck_predicate_values` (predicates as function values, in `together` branches and `together for`), `recheck_refined_params` (out-of-range `Int` values from `checked` into refined parameters, at 1e20 and at the 2^62 tagged-word boundary) and `recheck_checked_in_tasks`.

### HOST-1 (#183, Major): the native grant check ignored operations

Fixed by #195: each operation of `main` is checked against the grants before `main` starts.

- **Rerun:** `host/grant-ops` matches.
- **New variants, all matching:**
  - `host/recheck_grant_modes`, under 12 grant sets: none; each capability missing; files read, write, `read,write`, read and write as two grants, on different prefixes, a narrower prefix and `..`. Exit code, stdout, first diagnostic and files written match.
  - `host/recheck_split_uses` (`Files[read]` and `Files[write]` declared apart), `recheck_indirect_ops` (the write reached through a helper, an effectful lambda, `together` and `together for`) and `recheck_handled_ops` (`write` served by a handler, so a read grant is enough).
  - `keys`, `redaction`, `limits`, `limits-paths` and `files-paths` still match.

### HOST-2 (#184, Major): native executables died on SIGPIPE

Fixed by #196 for stdout: the runtime ignores `SIGPIPE`, and a failed write takes the existing path, exit code 2 with the trace complete.

- **Reruns:** `host/sigpipe` under `| head -n1` and `| true` matches.
- **New variants matching:** `host/recheck_pipe_tasks` (prints from `together` tasks) and `recheck_pipe_big_line` (a 1 MiB line).
- **Open, #198:** with stderr closed, the interpreter exits 2 and the executable exits 1, because native ignores a failed write to stderr. Repro: `host/recheck_pipe_then_fail` (or `host/grant-ops` with `--grant console`), stdout to `/dev/null`, stderr on a pipe whose read end is already closed:

  ```sh
  python3 -c 'import os,subprocess,sys; r,w=os.pipe(); os.close(r); print(subprocess.run(sys.argv[1:],stdout=subprocess.DEVNULL,stderr=w).returncode)' <command>
  ```

## Native `together` (PR #175)

All the earlier cases, `tasks/*` and `equiv/tog*`, match on exit code and failure. The other differences are ones `spec/19 §8` rule 4 allows, or known: interleaved stdout, extra events of runs that are not ordered, native's compact U6003 text (`spec/16 §3`) and the debug build's "copied" note. `tasks/thread_limit` runs its full 16,384 tasks natively in 15 s at 150 MB.

- **New variants, all matching:** `tasks/recheck_abort_race` and `recheck_abort_mirror` (a failure racing a depth-limit U6003, in both orders), `recheck_fixed_ranges` (`together for` over `U8` `0..255`, empty and reversed ranges, `I8`, `I64` and `U64` at both ends), `recheck_nested_cancel` (nested blocks, handlers, `attempt` in a task, secret values in task events) and `recheck_task_memory` (16 tasks under `--max-memory` 4096, 200 and 50: always U6003, never a kill by the system).
- **Open, #200 (P1):** a task cancelled while one of its operations runs in a Una handler stops at the handler body's first call. The operation's event keeps the placeholder outcome `{"kind":"aborted"}` and is followed by `cancelled`. `spec/19 §6` rule 4 and `§7` rule 3 say an operation in progress completes and keeps its own outcome, so no UAM run produces this trace. Repro: `equiv/togcancel` (4 native runs of 5) and `tasks/recheck_nested_cancel` (task `3.2`).

## Other findings filed from this review

- #185: the first round's minors HOST-3, HOST-4, HOST-5, MEM-3 and INT-2 (fixed by #193), and the notes INT-3 (fixed by #197), INT-4 and MEM-4.
- #199: with its stdout pipe closed, `una run` exits 2 and prints nothing on stderr, not even the U6001 of a failing `main` (interpreter side; exit codes match).
- #201: a sequential loop of 400,000 `append`s hits U6003 under `--max-memory 4096` with 2.9 GB resident.
- #202: the `spec/19 §1` example `fetch(id) if error pass` as a `together for` body does not parse.

## Cases whose result changed since they were written

- `equiv/refinefn` passes a lambda with a refined parameter, `fn(x: Small) -> x + x`, where `fn(Int) -> Int` is expected. `main` at `0e080a3` accepted it; `main` at `7bfb838` rejects it with U5001 at the lambda's parameter, which is the sound result. It no longer runs, and is kept as a record.
