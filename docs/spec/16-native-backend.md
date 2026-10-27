# 16. Native backend

Status: Normative, v0.2 (ADR 0055).

## 1. Scope and refinement

1. `una build` compiles a package ahead of time into an executable through Cranelift and the system linker (`cc`). The executable runs `pub main() -> Unit` of the entry module, the module `una run` would run (`spec/13 §5`).
2. The supported targets are `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`. Each host builds for itself; there is no cross-compilation in v0.2.
3. The backend compiles every construct the interpreter runs, including kinds, lists, `Option` and `Result`, failures, handlers and host capabilities.
4. **Refinement.** The backend refines the UAM (`spec/11 §1`): for every program and host behavior, the executable's output, failure, abort and trace MUST be ones the UAM produces. The interpreter is the reference.
5. **Input.** Its input is the Core IR of a package that checks at tier 1. `una build` refuses a package with errors, as `una run` does: it reports them and writes no executable.

## 2. `una build`

1. `una build [path]` checks the package at tier 1, as `una check` does (`spec/13 §5`). That includes U3012 for each `uses` of `main` that no host capability, or no grant of `[run] grants` when it is listed, can provide. It then embeds `[run] grants` in the executable as its default grants.
2. It writes the executable to `target/una/<name>` under the package root. `<name>` is the `[package] name` of `una.toml`, or the module name for a single file. `--out <file>` writes it to `<file>` instead.
3. A package without an entry, or an entry without `pub main() -> Unit`, is a usage error (exit code 2), as for `una run`. So is a failure of the system linker, whose output goes to stderr.
4. Exit codes follow `spec/13 §5` (ADR 0036): `0` when the executable is written, `1` when the checks report errors, `2` for a usage error, `3` for an internal compiler error (U8001).
5. With `--json`, `una build` prints, with keys in this order:

   ```json
   {"schema":"una.build/1","tier":1,"status":"built","target":"aarch64-apple-darwin","output":"target/una/hello","diagnostics":[]}
   ```

   - `status` is `built`, or `rejected` when the checks reported errors, which `diagnostics` holds; then `output` is `null`.
   - `target` is the host target.
6. Without `--json`, `una build` prints `built <output>` on stdout.
7. **Slices.** Until the backend compiles the whole language (#26), a construct it does not compile yet is a usage error, exit code 2. `una build` names the construct on stderr, as in "the native backend does not compile calls of functions yet", and writes no executable.

## 3. The executable

1. **Output contract.** The executable follows the human contract of `una run` (`spec/13 §5`):
   - the lines the program prints go to stdout as it prints them;
   - U6001 (a failure of `main`), U6003 (call depth, stack or memory exhausted), U6004 (a header value with a control character, `spec/24 §1`) and U6005 (host text that is not valid UTF-8, `spec/14 §3`) go to stderr, with exit code 1;
   - exit code 0 when `main` returns.
   - (v0.2, #259.) A failure to write output exits with code 2, as in `una run`. A closed stdout or stderr is not one: what the executable writes there is discarded, as `/dev/null` discards it, and the exit code is the one an open stream would give. A broken pipe (EPIPE), a reader that went away, is a failure to write.

   It has no `--json` output in v0.2.

   Its rendering of U6001, U6003, U6004 and U6005 on stderr is compact: the first line, the code, the position and the failure are those of `una run`, followed by `  at <file>:<line>:<column>` and a `  note:` line instead of a source frame. The comparison of §5 covers stdout, the exit status, the failure and the trace, not that rendering.
2. **Grants.** The executable has the grants model of `una run` (`spec/13 §5`):
   - Its grants are the `[run] grants` embedded at build time, together with its own `--grant <spec>` flags, which take the grant specs of `spec/13 §5`.
   - Before `main` starts, it checks every `uses` of `main` against those grants. A `uses` they do not grant is U3012 on stderr, with exit code 1, and nothing runs.
   - Relative paths, those of embedded grants included, resolve against the executable's current directory, with the path rules of `una run`.
   - (Edition 0.4 preview, ADR 0116.) For a package whose `ita.una` has a `drive` section, the executable runs the driver of `spec/26 §2`, checks the `uses` of `init`, `step` and `on_stop` as it checks those of `main`, and takes `--stop-after-iterations <n>` as `una run` does (`spec/13 §5`). Every executable takes `--virtual-time` as `una run` does (`spec/13 §5`, `spec/26 §8`): its clock is then the virtual one.
   - (Edition 0.4 preview, ADR 0116.) Executables take `--deadline <seconds>`, from 1 to 31,536,000, and `--stop-grace <seconds>`, from 1 to 86,400 (default 10), as `una run` does, and follow the host stops of `spec/26 §6`: an executable whose `main` says `diverges`, or that runs a driver, installs handlers of SIGINT and SIGTERM, and any other keeps their default actions. U6007 goes to stderr in the compact form of rule 1, with exit code 1, and the trace file is written.
   - A `--deadline` or `--stop-grace` that is not a number or is out of its range, a `--grant` value that is not a grant spec, a `--max-depth`, `--max-memory`, `--max-response` or `--http-timeout` that is not a number, a `--max-memory`, `--max-response` or `--http-timeout` of `0`, an `--http-timeout` over 86400, a `--max-depth` whose stack cannot be allocated, and a `--workers`, `--task-depth`, `--io-threads` or `--file-threads` that is not a number or is out of its range (`spec/27 §3`) are usage errors, exit code 2, as for `una run`. Key files are read after the check of `uses`, as `una run` reads them, and an unreadable or invalid one is then a usage error too.
3. **Depth and memory.** `--max-depth <n>` bounds the call depth as in `spec/11 §7`, with the same default, on a stack capped at 1 GiB. Exceeding it, or running short of stack before it, aborts with U6003; the executable never overflows its stack. (v0.2, ADR 0061.) `--max-memory <MiB>` bounds the heap of the run as in `spec/11 §7`, with the same default, and exceeding it aborts with U6003, `this call exceeds the memory limit of N MiB`. (v0.2, ADR 0113, 2026-10-15.) The copies a `together` block makes check the budget before each allocation, and a refusal aborts with U6003 at the block, `this call exceeds the memory limit of N MiB`. A task whose stack cannot be mapped aborts with U6003 at the block, ``a task of `together` could not run``. The threads of a program that holds a task construct or a `for ever` start before `main` (`spec/27 §1` rule 4), so no task waits on a thread's start: when one of them cannot start, or a stack of the pool that is filled with them cannot be mapped, the run stops before `main` with U6003, exit code 1, no position and the message `the threads and stacks of this run's tasks could not be made`. (ADR 0117, amendment of 2026-10-25.) The stack each task uses counts toward `--max-memory` (`spec/27 §4`). An export of more than 4294967295 objects aborts with U6003 at the block, and a retain past 4294967295 references to one value ends the process with U6003, exit code 1, when the run's heap and stacks hold at least (2^32 − 1) × 8 bytes, else with U8001, exit code 3, with one line on stderr, no site and no trace file. A panic of the runtime during a copy is U8001, exit code 3.
4. **Traces.** `--trace <file>` writes the trace of `spec/11 §5` to `<file>` as a JSON array of events. Without the flag the executable records no trace. (Edition 0.4, ADR 0117 rule 10.) The file is made before `main` starts, a file that cannot be made being a usage error, and each event is written as it is, `]` closing the array when the run ends (`spec/27 §8`).

## 4. Runtime representation

The representation is not observable; these rules fix what the runtime library `una-rt`, which every executable links, provides.

1. **Values.** Heap values (records, variants with payloads, lists, `Text`, arbitrary-precision integers, closures) are reference counted, non-atomically.
   - A value is freed when its last reference is released.
   - Values are immutable, and the language has no mutable aliasing, so no reference cycle can form, and nothing leaks.
   - `Text` is UTF-8.
2. **`Int`.** An `Int` is a tagged 63-bit integer in a machine word, and an operation whose result does not fit promotes it to an arbitrary-precision integer.
   - A value whose range a refinement proves MAY be a plain machine integer, with no tag and no check.
   - Arithmetic is mathematical, with Euclidean division and remainder, as in `spec/11`.
3. **Failures** are returned values, a tag and a payload. Nothing unwinds.
4. **Capabilities** are passed explicitly, as Core IR passes them (`spec/09`). Handlers and `with` compile to environment passing, and a capability is never a runtime value a program can store.
   - (v0.2, ADR 0066.) A function with constrained type parameters also takes a hidden type descriptor for each of them, and is still compiled once (`spec/20 §7`).
5. **Host capabilities** (`spec/14 §3`) have the interpreter's semantics, grants, path rules and failures.
6. (v0.2, ADR 0065.) **Tasks.** Each task of a `together` block runs on its own thread with its own heap; values are copied at the fork and the join, so reference counts stay non-atomic (`spec/19 §8`).
7. (v0.2, #30.) **Updates in place.** Within the freedom of `spec/17 §7` rule 3, an update MAY happen in place when the code gives up the value at its last use and its reference count shows nothing else holds it. Otherwise the value is copied.
   - **Which updates:** `append` and `set` on a list, and a record or variant rebuilt from one of its kind that dies there, as a field edit is. (v0.2, ADR 0069.) Also `set` and `remove` on a map, kept in Rust's standard `BTreeMap` (`spec/21 §5`). (v0.2, ADR 0070; implemented after v0.2.0 (#261).) Also a text rebound to an interpolation that begins with it, which appends in place (`spec/22 §6`). (v0.2, ADR 0072.) Also an argument that moves into its callee, which the callee may update in place.
   - **Never moved:** a value something borrows from, which is a `when` scrutinee, a `let (...)` tuple or a list a loop visits. Nor a value a `together` task captures.
   - **Arguments** (v0.2, ADR 0072). An argument moves into the callee when the caller never reads its binding again, including in the code that a failure of the call reaches (a handling clause, or what follows an `attempt`), and when it is not passed twice in that call. Any other argument is borrowed, and the callee copies it to update it.
     - A function of the package owns each parameter it consumes: one that reaches an update in place, an owned parameter of another function, or a loop state that does, directly or through an alias. The owned parameters are a least fixpoint over the package (borrow inference). Its other parameters are borrowed.
     - At a call, an argument for an owned parameter moves when it can, and is retained otherwise. The callee releases an owned parameter at the end of its body, or moves it at its last use.
     - (v0.2.) A lambda owns every parameter, and so does the lambda that a function used as a value stands for, whose call of the function moves its arguments into the function's owned parameters and releases the others. So a call through a function value moves or retains each argument as a call of a function does. `fold` moves its state into the step, and `map`, `filter`, `first_where` and `fold` retain each item they give it. A call through a function value whose target only borrows costs one retain at the caller, when the argument is still needed there, and one release in the lambda.
     - The parameters of a handler's operations are borrowed.
   - **Not observable:** in-place updates change no output, failure, abort or trace.
8. **Trusted base.** `una-rt` is in the Trusted zone (charter §3). Its only `unsafe` code of the repository is `ffi.rs` (ADR 0058), and its heap budget (§3, ADR 0061) depends on the `cap` allocator crate, whose `unsafe` code is outside that boundary and is trusted with it (ADR 0061 amendment of 2026-10-06).

## 5. Equivalence testing

The repository's CI runs every runnable program of its suites twice: in the interpreter, and as a native executable built on the CI host. This covers the examples, the conformance cases that run, and the adversarial programs of the reviews.

The output, the exit status, the failure and the trace (`--trace`) MUST be identical. A divergence fails CI. (v0.2, ADR 0065.) A run that starts concurrent tasks is compared as `spec/19 §8` rule 4 says: the lines that tasks print may interleave in any order, and the trace is identical unless a task after a failed one had started. (Edition 0.4 preview, ADR 0116 rule 19.) Two runs on the real clock read `Clock.now` at different moments, so their comparison sets those readings aside. Under `--virtual-time` in both engines, it does not: the readings MUST be identical too (`spec/26 §8`). (Edition 0.4, ADR 0117 rule 12.) Under `--virtual-time`, a run with `first of` MUST give the same winner, output, failure and trace in both engines, which run its branches in index order, each to its end, up to the first that returns (`spec/27 §6`); under the real clock, a run in which the winner of a `first of` decides the outcome is compared only for being an outcome the UAM allows under some timing. Under `--virtual-time`, a run with `tasks` and `start` MUST give the same output, exit status, failure and trace in both engines, which follow the schedule of `spec/27 §8`; an executable built with check elision refuses `--virtual-time`. Under the real clock, the events of started tasks are in the order of real time, so a run with them is compared task by task: each task's events and lines in its own order, the same set of tasks, and the same exit status and failure.
