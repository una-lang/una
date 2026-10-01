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

## 3. The executable

1. **Output contract.** The executable follows the human contract of `una run` (`spec/13 §5`):
   - the lines the program prints go to stdout as it prints them;
   - U6001 (a failure of `main`) and U6003 (stack exhaustion) go to stderr, with exit code 1;
   - exit code 0 when `main` returns.

   It has no `--json` output in v0.2.
2. **Grants.** The executable has the grants model of `una run` (`spec/13 §5`):
   - Its grants are the `[run] grants` embedded at build time, together with its own `--grant <spec>` flags, which take the grant specs of `spec/13 §5`.
   - Before `main` starts, it checks every `uses` of `main` against those grants. A `uses` they do not grant is U3012 on stderr, with exit code 1, and nothing runs.
   - Relative paths, those of embedded grants included, resolve against the executable's current directory, with the path rules of `una run`.
   - A `--grant` value that is not a grant spec, an unreadable or invalid key file, and a `--max-depth` whose stack cannot be allocated are usage errors, exit code 2, as for `una run`.
3. **Depth.** `--max-depth <n>` bounds the call depth as in `spec/11 §7`, with the same default. Exceeding it, or running short of stack before it, aborts with U6003; the executable never overflows its stack.
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
5. **Host capabilities** (`spec/14 §3`) have the interpreter's semantics, grants, path rules and failures.

## 5. Equivalence testing

The repository's CI runs every runnable program of its suites twice: in the interpreter, and as a native executable built on the CI host. This covers the examples, the conformance cases that run, and the adversarial programs of the reviews.

The output, the exit status, the failure and the trace (`--trace`) MUST be identical. A divergence fails CI.
