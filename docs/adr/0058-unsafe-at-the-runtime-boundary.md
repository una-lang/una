# 0058. `unsafe` at the runtime's C ABI boundary

- Status: Accepted
- Date: 2026-10-01
- Deciders: Leonardo (accepted on 2026-10-01), on a proposal by Claude (Una-2)
- Resolves: the blocker of issue #111 (native slice 1)
- Amends: ADR 0055 rule 5, `docs/compiler/conventions.md §1`, `docs/compiler/architecture.md §2`

## Context

The workspace forbids `unsafe` (`unsafe_code = "forbid"`, `conventions.md §1`). ADR 0055 makes every native executable link `una-rt`, a Rust static library, with code that Cranelift generates. That boundary cannot be crossed without `unsafe` in three places:
- the runtime exports the functions the generated code calls, which edition 2024 marks `#[unsafe(no_mangle)]`;
- the runtime calls the generated `main` of Una through a function pointer the generated code hands it;
- the runtime reads what the generated code passes by pointer: the process arguments, the embedded description of the program, and text literals.

`forbid` cannot be relaxed for one crate or one module, so ADR 0055 alone cannot be implemented.

## Decision

1. **One module.** `unsafe` is allowed only in `crates/una-rt/src/ffi.rs`, the C ABI boundary between generated code and the runtime.
   - `una-rt` opts out of the workspace lints, keeps every other lint, and sets `unsafe_code = "deny"`.
   - `#[allow(unsafe_code)]` is on the `ffi` module only.
   - Every `unsafe` block has a `// SAFETY:` comment, and every exported `unsafe extern "C"` function states its contract in a `# Safety` section.
2. **The rest is safe.** Everything behind the boundary stays in safe Rust: grants, host operations, traces, and later reference-counted objects, `Text` and arbitrary-precision integers. `una-codegen` stays free of `unsafe`. It only emits an object file, and nothing runs generated code in the compiler: there is no JIT.
3. **Every other crate** keeps `unsafe_code = "forbid"`.
4. `conventions.md §1` and `architecture.md §2` state the exception. `CLAUDE.md`'s Rust rules are updated separately, by the coordinator.

## Consequences

- The native backend can be implemented as ADR 0055 states, with `una-rt` in Rust, sharing the interpreter's grants and trace form.
- The `unsafe` surface is one small module that a reviewer can audit whole.
- The runtime is part of the Trusted zone, with the compiler and the interpreter (charter §3). The build report's `tcb` section has no field for the runtime or this exception: it lists trusted modules of the package, host capabilities, the solver and the proof cache. Adding one changes `una.report/2`, so it is not done here.

## Alternatives considered

- **A runtime written in C, compiled by `cc`.** Not chosen: it contradicts ADR 0055 rule 5, which makes the runtime a Rust library that shares `num-bigint` with the interpreter. Its code would be just as unsafe, only without the lint.
- **No hand-written runtime: everything generated in Cranelift IR.** Not chosen: it does not scale to arbitrary-precision integers, `Text`, host capabilities and traces.

## Amendment (2026-10-18, ADRs 0116 and 0117)

A second place may hold `unsafe` code: a new crate, `una-sys`, under rule 1's comments and lint settings. It holds only the installation of the handlers of SIGINT and SIGTERM with their async-signal-safe bodies (ADR 0116 rule 18), and the stack switch of coroutines with the mapping of their stacks with guard pages (ADR 0117 rule 2), which both the native runtime and the interpreter use. It stays under 800 lines, depends on no crate of the repository, and every change to it gets a security review before it merges. `una-rt`, `una-interp` and `una-cli` depend on it, and stay free of `unsafe` outside `ffi.rs`.

## Amendment (2026-10-25): what `una-sys` holds, and how its size is counted

The amendment of 2026-10-18 limits `una-sys` to the signal handlers and the coroutine stacks. The concurrency, server and package-store records need more bindings to the operating system than those two. Each is written in the crate's own Rust, through the declarations of the `libc` crate (ADR 0116, amendment of 2026-10-19): no foreign code, and no binding to a system library beyond the C library.

**What it may hold.** Bindings to the operating system that the standard library cannot provide, in these modules:

| Module | Purpose | Record | C library functions |
| --- | --- | --- | --- |
| `signals.rs`, `attention.rs` | Signal handlers, the wake pipe, attention words | ADR 0116 rule 18 | `sigaction`, `sigemptyset`, `sigaddset`, `pthread_sigmask`, `raise`, `pipe`, `pipe2`, `write`, `close`, `fcntl`, `_exit`, and the errno accessor (`__errno_location` on Linux, `__error` on macOS) |
| `coroutine.rs` | Coroutine stacks with guard pages, and the stack switch | ADR 0117 rule 2 | `mmap`, `mprotect`, `madvise`, `munmap`, `sysconf`; the switch is assembly in Rust, with no library call |
| `io.rs`, `poll.rs` | The readiness poller, and the descriptor limit | ADR 0117 rule 3 | `poll`, `epoll_create1`, `epoll_ctl`, `epoll_wait` on Linux, `kqueue`, `kevent` on macOS, `getrlimit` |
| `server.rs` | Listening sockets, the run's identity, raising the descriptor limit | ADR 0123 | `socket`, `setsockopt`, `bind`, `listen`, `getegid`, `getgroups`, `setrlimit` |
| `dir.rs` | Descriptor-relative file operations for the package store | ADR 0122 | `openat`, `mkdirat`, `renameat2` with `RENAME_NOREPLACE` on Linux, `renameatx_np` with `RENAME_EXCL` on macOS, `renameat`, `unlinkat`, `geteuid` |

**Descriptor-relative file creation.** The module `dir` creates, opens, renames and removes files and directories relative to an open directory descriptor, for the writes of vendoring and of the Itaoca store. The standard library works by path only: it cannot create relative to a descriptor, create without following a link at the name, rename without replacing what is at the target, remove relative to a descriptor, or read the process's effective user id for the owner check of a fresh directory. Every name passed to the module MUST be one path component, and no call falls back to a weaker one: a file system without an exclusive rename refuses the write, saying so. `geteuid` is bound once, as `dir::effective_uid`; other code of the crate that needs the effective user id, such as the server's identity check, MUST call it rather than bind `geteuid` again. Known limit: no system call checks a name and renames it in one step. A directory planted at the fresh name between the check and the rename is moved into place, the check after the rename fails the write, and every later load verifies and refuses it, so it is a denial of service until removed, never the use of a different package.

**Rules that every binding meets.**
- It has a safe wrapper, with a doc comment that names why the standard library cannot do it.
- It has its own test, in `crates/una-sys/tests/`.
- Every `unsafe` block carries a `// SAFETY:` comment.
- Each function of the C library is bound once. Every caller in the crate goes through that one wrapper; for example `geteuid` lives in `dir.rs`, and `server.rs` calls it there. A helper used by several modules, such as the errno accessor or `fcntl`, is one private function.
- Every change to the crate gets an independent security review before it merges.

**What stays the same.**
- `una-sys` depends on no crate of the repository, and on no external crate but `libc`.
- The crates that may depend on it are `una-rt`, `una-interp`, `una-cli`, `una-http` and `una-project`, and through `una-project`, `una-check`. They stay free of `unsafe`, except `una-rt`'s `ffi.rs`.

**How its size is counted.** The size of `una-sys` is the number of physical lines, comments and blank lines included, of the Rust files under `crates/una-sys/src/`. The tests under `crates/una-sys/tests/` do not count. The count is what this command prints at the root of the repository:

```sh
find crates/una-sys/src -name '*.rs' -exec cat {} + | wc -l
```

On `main` at the time of this amendment the count is 363. This amendment does not change the cap. A later amendment sets it at the exact count of the merged slices, with at most 5% of headroom, and replaces every earlier number. Each pull request that changes the crate states its count, by this command, in its description.
