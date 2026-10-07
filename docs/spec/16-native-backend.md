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
   - U6001 (a failure of `main`) and U6003 (call depth, stack or memory exhausted) go to stderr, with exit code 1;
   - exit code 0 when `main` returns.

   It has no `--json` output in v0.2.
2. **Grants.** The executable has the grants model of `una run` (`spec/13 §5`):
   - Its grants are the `[run] grants` embedded at build time, together with its own `--grant <spec>` flags, which take the grant specs of `spec/13 §5`.
   - Before `main` starts, it checks every `uses` of `main` against those grants. A `uses` they do not grant is U3012 on stderr, with exit code 1, and nothing runs.
   - Relative paths, those of embedded grants included, resolve against the executable's current directory, with the path rules of `una run`.
   - A `--grant` value that is not a grant spec, a `--max-depth` or `--max-memory` that is not a number, a `--max-memory` of `0`, and a `--max-depth` whose stack cannot be allocated are usage errors, exit code 2, as for `una run`. Key files are read after the check of `uses`, as `una run` reads them, and an unreadable or invalid one is then a usage error too.
3. **Depth and memory.** `--max-depth <n>` bounds the call depth as in `spec/11 §7`, with the same default, on a stack capped at 1 GiB. Exceeding it, or running short of stack before it, aborts with U6003; the executable never overflows its stack. (v0.2, ADR 0061.) `--max-memory <MiB>` bounds the heap of the run as in `spec/11 §7`, with the same default, and exceeding it aborts with U6003, `this call exceeds the memory limit of N MiB`.
4. **Traces.** `--trace <file>` writes the trace of `spec/11 §5` to `<file>` as a JSON array of events. Without the flag the executable records no trace.

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
   - **Which updates:** `append` and `set` on a list, and a record or variant rebuilt from one of its kind that dies there, as a field edit is. (v0.2, ADR 0069.) Also `set` and `remove` on a map, kept in Rust's standard `BTreeMap` (`spec/21 §5`). (v0.2, ADR 0070.) Also a text rebound to an interpolation that begins with it, which appends in place (`spec/22 §6`).
   - **Never moved:** a value something borrows from, which is a `when` scrutinee, a `let (...)` tuple or a list a loop visits. Nor a value a `together` task captures, or one passed to a function, including a `uses of` parameter's argument.
   - **Not observable:** in-place updates change no output, failure, abort or trace.
8. **Trusted base.** `una-rt` is in the Trusted zone (charter §3). Its only `unsafe` code of the repository is `ffi.rs` (ADR 0058), and its heap budget (§3, ADR 0061) depends on the `cap` allocator crate, whose `unsafe` code is outside that boundary and is trusted with it (ADR 0061 amendment of 2026-10-06; Leonardo, 2026-10-06).

## 5. Equivalence testing

The repository's CI runs every runnable program of its suites twice: in the interpreter, and as a native executable built on the CI host. This covers the examples, the conformance cases that run, and the adversarial programs of the reviews.

The output, the exit status, the failure and the trace (`--trace`) MUST be identical. A divergence fails CI. (v0.2, ADR 0065.) A run that starts concurrent tasks is compared as `spec/19 §8` rule 4 says: the lines that tasks print may interleave in any order, and the trace is identical unless a task after a failed one had started.
