# Foreign code inventory

Issue #1098, part of epic #1097. The rule this inventory serves: Una ships no C, C++ or foreign assembly, not in the `una` binary, not in the runtime (`una-rt`), and not in any linked program. Each finding is fixed by an implementation of our own in Rust. Z3 is the one accepted exception.

This is a snapshot of `main` at `358ab4d1` (workspace version 0.3.16) and of the v0.3.16 release artifacts for `aarch64-apple-darwin`. The Z3 rows are updated for ADR 0124 step 1, which built the solver into `una`, and for the Linux release `una` of 0.4.0 (#1905), from its build scripts rather than from a Linux artifact (section 1).

## Summary

| Item | What it is | Plan |
| --- | --- | --- |
| ring 0.17.14 (and its build dependency cc 1.6.0) | rustls's crypto provider in `una-http`; compiles C and assembly; present in `una`, `libuna_rt.a` and every linked program, even one without `Http` | Our own Rust cryptography for TLS (epic #1097 item "Own cryptography for TLS, replacing ring"); cc goes away with it |
| num-bigint 0.4.8, num-integer 0.1.47, num-traits 0.2.19 | Big integers; their `std` feature calls the C math library (`cbrt`, `exp`, `log`, `log2`) | #1092 turns default features off (no `std`, no libm calls); replaced by our own big integer, `una_core::bigint` (#1093): no build holds them since |
| getrandom 0.2.17 | ring's random source; calls `getentropy` (macOS) and the `getrandom` system call through the C library (Linux); no C compiled | Leaves with ring |
| libc 0.2.189 | Rust declarations of the system interface; no C compiled; adds `#[link]` directives (`iconv` on macOS, `c`, `m`, `pthread`, `rt`, `dl`, `util` on Linux) | Declarations accepted for `una-sys` by ADR 0116 Appendix A; the unused `libiconv` load command of `una` removed by #1103 (`-dead_strip_dylibs`); the library behind it is the open question below |
| cpufeatures 0.2.17 | CPU feature detection for sha2 and curve25519-dalek; calls `sysctlbyname` (macOS) or `getauxval` (Linux) on aarch64; no C compiled | System interface, part of the open question below |
| Rust standard library (std, panic_unwind, std_detect) | The source of most C library symbols in all three artifacts | System interface, part of the open question below |
| compiler_builtins C and assembly objects (from LLVM compiler-rt) | Shipped inside `libuna_rt.a` by the prebuilt standard library; not linked into `una` nor into the hello program | Kept out of linked artifacts by the #1092 symbol check; removing them from the archive needs a standard library built without compiler-builtins' C part (open, toolchain) |
| Unwinder (`_Unwind_*`) | libunwind inside libSystem (macOS); `libgcc_s` on Linux, which `una build` links | Part of the open question below |
| System linker `cc` | `una build` links through the host C compiler driver | A host tool, not code in the artifacts; no issue yet |
| C-emitting release backend (ADR 0106) | Accepted design; no code on `main` | Withdrawn by ADR 0106's amendment of 2026-10-31 (#1099, epic #1097); release builds become Una's own code generation (#1305) |
| Z3 5.1.0 | The SMT solver, a C++ library built into `una` (ADR 0124 step 1), run in a confined child process of `una` itself; pinned by `supply-chain/z3.lock`, its version by `verification.lock` | Accepted exception |
| GNU C++ runtime (`libstdc++`) and `libgcc` | Linked statically into the Linux release `una` for Z3's C++ code (`scripts/release.sh`, `-static-libstdc++ -static-libgcc`); not in `libuna_rt.a` nor in a linked program | Part of the Z3 exception: leaves only with Z3 |

## 1. Method

### Dependency graph

The graph covered is the normal dependency graph of `una-cli` (the `una` binary) and `una-rt` (the runtime library that `una build` links into every program). Dev-dependencies are excluded (rcgen, for example, is a dev-dependency of `una-http` only). Build dependencies are examined as the means of a build script, not as shipped code.

```bash
cargo metadata --locked --format-version 1
cargo tree --locked -e normal -p una-cli -p una-rt --target <triple> --prefix none --format '{p}'
cargo tree --locked -e normal,build -p una-cli -p una-rt --target all --prefix none --format '{p}'
cargo tree --locked -e normal,build -p una-cli -p una-rt --target all -i <crate> --prefix depth
cargo tree --locked -e features -p una-cli --target aarch64-apple-darwin -f '{p} {f}'
```

For the four targets `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu`, the graph has 112 (aarch64) or 114 (x86_64) packages. The target-specific differences:

- x86_64 adds `curve25519-dalek-derive` (a procedural macro for curve25519-dalek's AVX2 backend, run at build time) and `syn` 2 for it.
- `libc` is pulled by `getrandom` 0.2 on all four targets, and by `cpufeatures` and `ring` on aarch64 only (`sysctlbyname` on macOS, `getauxval` on Linux). On x86_64, `cpufeatures` uses the CPUID instruction and needs no `libc`.
- ring compiles a different set of C and assembly files per architecture (see the table).
- With `--target all`, the graph also lists Windows-only crates (`windows-sys`, `windows-targets`, `windows_*`, `windows-link`, `anstyle-wincon`) and `wasi`. The `windows_*` crates carry pregenerated import libraries. None of them is built for the macOS and Linux targets, so they are out of scope here.

`una-sys` (with `libc`) and `una-hash` (with `getrandom` 0.4.3) are workspace members that `una-cli` and `una-rt` do not depend on yet. When they join the graph they add `libc` declarations and the operating system's random source, both system interfaces (see section 4).

### Crate sources

For every package in the graph, the registry source under `~/.cargo/registry/src/*/` was read (never built or executed):

- `build.rs`: whether it compiles anything (`cc`, `cmake`, `nasm`, `Command::new` of a compiler) or only probes `rustc --version` and prints `cargo:rustc-cfg`;
- `Cargo.toml`: the `links =` key and the `[build-dependencies]`;
- the source tree: files ending in `.c`, `.h`, `.cc`, `.cpp`, `.S`, `.s`, `.asm`, `.o`, `.a`, `.obj`, `.lib`;
- `src/`: `#[link(...)]` attributes and `extern "C" { ... }` blocks.

### Artifacts

The three artifacts are the v0.3.16 release for `aarch64-apple-darwin`:

| Artifact | SHA-256 |
| --- | --- |
| `una` | `f6abd9a5963550c5817fa09f6640a7d0c00e0ec71a5f00e25a87fd48f379c26d` |
| `lib/libuna_rt.a` | `8d822664cec0b2c27726a84b8e70f5e85ba9cffb16d0452367c75f8b5dcedc43` |
| `hello`, a hello world built by that release with `una build` | `31c80f792b607f987735c512d8d08d1210c334b54350a57430f86292ec61dc34` |

```bash
otool -L una hello
nm -u una
nm -u hello
llvm-nm -A -u libuna_rt.a                 # undefined symbols, per member object
llvm-nm -g --defined-only -j libuna_rt.a  # symbols the archive defines itself
llvm-nm -A --defined-only libuna_rt.a     # which member defines what
nm una hello                              # which foreign objects reached the binaries
```

Xcode's `nm` reads the two linked binaries but stops on the `std`, `alloc` and `core` members of the archive ("Unknown attribute kind", the members having been produced by a newer LLVM than Xcode's). The archive was therefore read with the `llvm-nm` of the pinned Rust toolchain (1.98.1, `lib/rustlib/aarch64-apple-darwin/bin/llvm-nm`). For the archive, a symbol counts as external when some member leaves it undefined and no member defines it; Rust-mangled symbols (`__R...`, `__ZN...`) are dropped, which leaves 198 C-level symbols. `una` imports 146 symbols and `hello` imports 76.

The artifacts are those of v0.3.16. The artifacts of `main` need a release build to re-check, which the #1092 symbol check does. No Linux artifact was inspected: the Linux release and a Linux-linked program need the same reading, which the #1092 check also covers.

## 2. Crates with foreign code or native links

Every package of the graph was examined. Packages not in this table have no build script, or have a build script that only probes `rustc` and sets `cfg` flags (anyhow, crc32fast, crossbeam-*, curve25519-dalek, generic-array, libm, num-traits, proc-macro2, quote, rustls, serde, serde_core, serde_json, target-lexicon, wasmtime-internal-core, zmij; cranelift-codegen, cranelift-assembler-x64 and cranelift-isle generate Rust source, and cranelift-codegen's runs `git` for version information). None of them has native source files, a `links` key, or a `#[link]` attribute for these targets. In particular these are pure Rust: cranelift-* and regalloc2, object (with crc32fast), gimli, rustls, rustls-webpki, rustls-pki-types, untrusted, webpki-roots, sha2 (its `asm` feature is off), ed25519-dalek, curve25519-dalek and fiat-crypto, libm (a Rust port of a C math library, used by cranelift-codegen), memchr, zmij, serde_json, toml. No zstd, flate2, miniz or other compression crate is in the graph (the archive's `adler2` and `addr2line` members come with the standard library's backtrace code, in Rust).

| Crate | Version | Criterion | Path from una-cli / una-rt | What it does for Una | Artifacts carrying it | Plan |
| --- | --- | --- | --- | --- | --- | --- |
| ring | 0.17.14 | (a) build.rs compiles C with `cc` (`curve25519.c`, `aes_nohw.c`, `montgomery.c`, `montgomery_inv.c`, `ecp_nistz.c`, `gfp_p256.c`, `gfp_p384.c`, `p256.c`, `p256-nistz.c`, `limbs.c`, `mem.c`, `poly1305.c`, the `constant_time_test.c` test source, and on x86_64 also `crypto.c`, `cpu_intel.c`, `curve25519_64_adx.c`) and pregenerated assembly (`pregenerated/*-ios64.S` on aarch64 macOS, `*-linux64.S` on aarch64 Linux, `*-macosx.S` and `*-elf.S` on x86_64, plus `fiat_curve25519_adx_*.S` on x86_64); (b) `links = "ring_core_0_17_14_"`; (c) imports `___stack_chk_fail`, `___stack_chk_guard`, `_bzero`, `_memcmp`, `_memcpy`, `_memmove` (C objects) and `_sysctlbyname` (its Rust code) | una-cli and una-rt → una-http (directly, and through una-interp) → rustls → ring; also rustls → rustls-webpki → ring | The crypto provider of rustls: AES-GCM, ChaCha20-Poly1305, X25519, P-256/P-384 ECDSA, RSA verification, SHA-2, HMAC, for the TLS of the `Http` capability (ADR 0074) | `libuna_rt.a` (11 C objects with exported symbols and 11 assembly objects on aarch64 macOS); `una` (118 symbols of these objects, C and assembly); `hello` (50 symbols, all from the assembly objects: `vpaes_*`, `ChaCha20_512_neon`, `ecp_nistz256_*`), although the program does not use `Http` and is linked with `-dead_strip` | Replaced by our own Rust cryptography for TLS (epic #1097). Finding: the assembly objects reach every linked program today, so ADR 0074 H7 ("an executable that never uses `Http` carries no TLS stack") does not hold for them |
| cc | 1.6.0 | (a) the C compiler driver crate | Build dependency of ring only | Runs the C compiler and assembler for ring | None (build time only) | Removed with ring |
| num-bigint | 0.4.8 | (c) imports `_cbrt`, `_exp`, `_log`, `_log2` through its `std` feature (default) | una-core, una-interp, una-rt (directly), and through una-http, una-check, una-codegen, una-project, una-verify | The arbitrary-precision `Int` of the language | `libuna_rt.a` (`_cbrt`, `_exp`, `_log`, `_log2`); `una` and `hello` (`_log2`, from the radix conversion estimate in `to_radix_digits_le`) | #1092 turns default features off (no `std`); replaced by our own big integer, `una_core::bigint` (#1093). `Cargo.lock` still lists it only inside the optional `x509-parser` feature of rcgen (a test dependency of una-http), which no build enables |
| num-integer | 0.1.47 | (c) imports `_cbrt`, `_exp`, `_log` (the floating-point first guess of `Roots`, under `std`) | num-bigint → num-integer | Integer roots and gcd for num-bigint | `libuna_rt.a` | Same as num-bigint (#1092, replaced by #1093) |
| num-traits | 0.2.19 | (c), indirectly: its `std` feature makes `Float` call the standard library's float methods, which call the C math library; without `std` it would need its `libm` feature | num-bigint → num-traits | Numeric traits for num-bigint | Through num-bigint and num-integer | Same as num-bigint (#1092, replaced by #1093) |
| getrandom | 0.2.17 | (b) `#[link]` and `extern "C"` declarations in its sources; (c) imports `_getentropy`, `___error`, `_strerror_r` | ring → getrandom | The random source of ring: `getentropy` on macOS; the `getrandom` system call through the C library's `syscall` wrapper on Linux, with a `/dev/urandom` fallback (ring's `dev_urandom_fallback` feature) | `libuna_rt.a`, `una` | Leaves with ring; our own TLS takes its randomness from the system interface of section 4 |
| libc | 0.2.189 | (b) `#[link(name = "iconv")]` on Apple targets, `#[link(name = "util" / "rt" / "pthread" / "m" / "dl" / "c")]` on Linux; build.rs only sets `cfg` flags, no C is compiled | getrandom 0.2 → libc (all targets); cpufeatures → libc and ring → libc (aarch64) | Rust declarations of C library functions and constants | No code of its own; up to v0.3.16 its link directives gave `una` a `/usr/lib/libiconv.2.dylib` load command (`otool -L`) although `una` imports no iconv symbol; `hello` has no such load command, because `una build` passes no `-liconv` | Declarations accepted for `una-sys` by ADR 0116 Appendix A (amendment of 2026-10-19). The unused `libiconv` dependency of `una` is removed by #1103: una-cli's build script links `una` with `-Wl,-dead_strip_dylibs` on macOS, `una build` passes the same flag (and `-Wl,--as-needed` on Linux), and `scripts/load-commands.sh` fails a release build on any load command outside `scripts/load-commands.txt`. The C library behind the declarations is the open question of section 4 |
| cpufeatures | 0.2.17 | (c) calls `sysctlbyname` (macOS) or `getauxval` (Linux) through libc on aarch64; no C compiled | sha2 → cpufeatures (sha2 used by una-cli, una-interp, una-project, una-verify); ed25519-dalek → curve25519-dalek → cpufeatures | Selects the SHA-2 and field arithmetic code paths at run time | `una`, `libuna_rt.a` (`_sysctlbyname`, together with ring, the standard library's `std_detect` and compiler-rt's `aarch64.o`) | System interface, part of section 4 |
| rayon-core | 1.13.0 | (b) `links = "rayon-core"` | rayon → rayon-core (una-cli, una-check) | Thread pool | `una` | None needed: the key only guarantees one rayon-core in a build; build.rs links nothing |

Also carried by the artifacts, outside the crate graph:

| Item | What it is | Artifacts | Plan |
| --- | --- | --- | --- |
| Rust standard library (`std`, `panic_unwind`, `std_detect`) | Rust code that calls the C library for every system service | All three; source of most symbols of section 3 | Section 4 |
| compiler_builtins C and assembly objects | The prebuilt standard library ships compiler_builtins with objects compiled from LLVM compiler-rt: 43 C objects (`int_util.o`, `absvdi2.o`, `mulvti3.o`, `muldc3.o`, ...; `int_util.o` imports `___assert_rtn`), the CPU feature detection `aarch64.o` (from compiler-rt's `cpu_model/aarch64.c`; imports `___stack_chk_fail`, `___stack_chk_guard`, `_sysctlbyname`) and 100 objects of outline atomics (`lse_cas1_acq.o`, ..., from compiler-rt's `lse.S`) | `libuna_rt.a` only: none of their symbols is in `una` or `hello` | Kept out of linked artifacts by the #1092 symbol check. Removing them from the archive needs a standard library built without compiler-builtins' C part; open, toolchain |
| The unwinder | `_Unwind_*` functions for panics and backtraces | All three (`panic_unwind`, std's backtrace, `_Unwind_Resume` in every crate's landing pads) | Section 4 (libSystem on macOS, `libgcc_s` on Linux) |
| System linker `cc` | `una build` links the program object and `libuna_rt.a` through the host C compiler driver (`crates/una-cli/src/build.rs`, ADR 0055 NB-1); on Linux it adds `-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc`, after `-Wl,--as-needed` since #1103, so only the libraries the program uses get a `NEEDED` entry | Not in the artifacts; a host tool needed to build | No issue yet |
| C-emitting release backend | ADR 0106 (accepted) chose C compiled by the system C compiler for release builds; no C emission exists in the code of `main` | None | Withdrawn by ADR 0106's amendment of 2026-10-31 (#1099, epic #1097) |
| Z3 5.1.0 | Z3's SMT-LIB front end, built by `scripts/build-z3.sh` from the source `supply-chain/z3.lock` pins and linked into `una` by `crates/una-z3`; each query runs in a child process of `una` itself, in solver mode, confined (`crates/una-cli/src/solver_child.rs`, `crates/una-sys/src/solver/`); the version is pinned by `verification.lock`. No `z3` program is run | `una` (with the system's C++ runtime, `/usr/lib/libc++.1.dylib`, on macOS; on Linux with `libstdc++` and `libgcc` linked into `una` statically, below); not `libuna_rt.a` nor a linked program | Accepted exception |
| GNU C++ runtime on Linux | `crates/una-z3/build.rs` links `stdc++` on Linux (line 123) for Z3's C++ code, and `scripts/release.sh` (line 99) adds `-static-libstdc++ -static-libgcc` to every Linux release build, so the release `una` holds the code of `libstdc++` and `libgcc` itself and asks the dynamic loader for no C++ library: `scripts/load-commands-una.txt` admits only `libm.so.6` beyond `scripts/load-commands.txt` (`libc.so.6`, `libgcc_s.so.1` and the loader), and `scripts/load-commands.sh`, a step of `scripts/local-ci.sh` (so of the `linux-arm64` gate) on the `una` that `scripts/release.sh` packed, fails on any other entry. No Linux release `una` was read for this inventory | `una` on Linux only; not `libuna_rt.a` nor a linked program | Part of the Z3 exception: it leaves with Z3 |

## 3. C library symbols of the artifacts

All three artifacts link only `/usr/lib/libSystem.B.dylib` (plus, up to v0.3.16, the unused `libiconv.2.dylib` for `una`, removed by #1103, section 2). Every symbol below resolves in libSystem. Groups, with the source of each symbol from `llvm-nm -A -u` on the archive:

| Group | `una` | `hello` | `libuna_rt.a` | Source |
| --- | --- | --- | --- | --- |
| C math library | `log2` | `log2` | `cbrt`, `exp`, `log`, `log2` | num-bigint and num-integer through their `std` feature (section 2), removed by #1092 and #1093. cranelift's constant folding uses the Rust `libm` crate and imports nothing |
| Memory and string routines | `bzero`, `memcmp`, `memcpy`, `memmove`, `memset`, `memset_pattern16`, `strlen` | `bzero`, `memcmp`, `memcpy`, `memmove`, `memset`, `strlen` | `bzero`, `memcmp`, `memcpy`, `memmove`, `memset`, `strlen`, `strnlen` | Calls that LLVM emits for copies, fills and comparisons in every Rust crate (on Apple targets compiler-builtins does not provide them, so libSystem's are used); ring's C objects (`bzero`, `memcmp`, `memcpy`); `strlen` and `strnlen` from std and alloc (C strings of the system interface) and una-rt |
| Memory allocation | `malloc`, `calloc`, `realloc`, `free`, `posix_memalign` | same | same | std's `System` allocator, wrapped by the `cap` allocator of una-rt and una-cli |
| Threads and synchronization | `pthread_*` (create, join, detach, attributes, mutexes, condition variables, `pthread_setname_np`, `pthread_threadid_np`, stack queries), `dispatch_semaphore_*`, `dispatch_time`, `dispatch_release`, `sched_yield` | `pthread_*` (create, join, attributes, mutexes, stack queries, name, id), `dispatch_semaphore_*`, `dispatch_release` | as `una`, plus `pthread_cond_timedwait_relative_np` | std (`thread`, `sync`, thread parking on Darwin through dispatch semaphores) |
| Unwinding | 13 `_Unwind_*` functions | 12 `_Unwind_*` functions | 13 `_Unwind_*` functions | `panic_unwind`, std's backtrace, and the landing pads of every crate (`_Unwind_Resume`); the implementation is libunwind inside libSystem |
| Stack protector and assertions | `__stack_chk_fail`, `__stack_chk_guard` | none | `__stack_chk_fail`, `__stack_chk_guard`, `__assert_rtn` | ring's C objects (`aes_nohw.o`, `curve25519.o`, `gfp_p384.o`, `p256-nistz.o`, `poly1305.o`) and compiler-rt's `aarch64.o` (archive only); `__assert_rtn` from compiler-rt's `int_util.o` (archive only) |
| Random bytes | `getentropy`, `CCRandomGenerateBytes` | `CCRandomGenerateBytes` | same as `una` | `getentropy` from getrandom 0.2 (ring); `CCRandomGenerateBytes` from std (the keys of `HashMap`'s `RandomState`) |
| CPU features | `sysctlbyname` | none | `sysctlbyname` | ring's Rust code, std's `std_detect`, cpufeatures; in the archive also compiler-rt's `aarch64.o` |
| Program image, arguments and thread-local storage | `_NSGetArgc`, `_NSGetArgv`, `_NSGetEnviron`, `_NSGetExecutablePath`, `_dyld_*` (4), `_tlv_atexit`, `_tlv_bootstrap`, `dyld_stub_binder` | `_dyld_*` (4), `_tlv_atexit`, `_tlv_bootstrap` | as `una` without `dyld_stub_binder` | std (arguments, environment, `current_exe`, backtrace symbolization, thread-locals); `_tlv_bootstrap` also from una-rt's thread-locals |
| Errors | `__error`, `strerror_r` | same | same | std (`io::Error`), getrandom |
| System calls: files and directories | `open`, `read`, `readv`, `write`, `writev`, `close`, `fstat`, `stat`, `lstat`, `fcntl`, `ioctl`, `isatty`, `opendir`, `readdir_r`, `closedir`, `dirfd`, `mkdir`, `rename`, `unlink`, `linkat`, `chmod`, `chdir`, `chroot`, `getcwd`, `realpath` | `open`, `read`, `write`, `writev`, `close`, `fstat`, `opendir`, `readdir_r`, `closedir`, `dirfd`, `getcwd` | as `una`, plus `openat`, `fstatat`, `fdopendir`, `pread`, `pwrite`, `lseek`, `ftruncate`, `flock`, `readlink`, `symlink`, `rmdir`, `unlinkat`, `mkfifo`, `chown`, `fchown`, `lchown`, `fchmod`, `copyfile_state_*`, `fcopyfile`, `fclonefileat`, `setattrlist`, `fsetattrlist` | std (`fs`, `io`, `env`); `close` also from una-rt and una-http |
| System calls: processes and signals | `fork`, `execvp`, `posix_spawnp` and `posix_spawn*` attributes, `waitpid`, `kill`, `getpid`, `setsid`, `setpgid`, `setuid`, `setgid`, `setgroups`, `pipe`, `dup`, `dup2`, `poll`, `sigaction`, `sigaddset`, `sigemptyset`, `sigaltstack`, `signal`, `pause`, `_exit`, `abort` | `signal`, `sigaltstack`, `pause`, `exit`, `abort` | as `una`, plus `getppid`, `killpg`, `getuid` | std (`process`, stack-overflow guard); `signal` also from una-rt (`ignore_sigpipe` in `ffi.rs`) |
| System calls: network | `socket`, `connect`, `send`, `recv`, `setsockopt`, `getsockopt`, `getaddrinfo`, `freeaddrinfo`, `gai_strerror` | none | as `una`, plus `bind`, `listen`, `accept`, `sendto`, `recvfrom`, `shutdown`, `socketpair`, `getsockname`, `getpeername`, `getpeereid`, `gethostname` | std (`net`), used by una-http |
| Memory mapping | `mmap`, `munmap`, `mprotect` | same | same | std (thread stack guard pages, signal stacks) |
| Time and others | `clock_gettime`, `nanosleep`, `sysconf`, `getenv`, `dlsym` | `sysconf`, `getenv` | as `una`, plus `mach_timebase_info`, `mach_wait_until`, `mach_error_string`, `confstr`, `getpwuid_r`, `setenv`, `unsetenv`, `dlopen`, `dlclose`, `dlerror` | std (`time`, `env`, weak symbol lookup through `dlsym`) |

On Linux the same groups come from glibc (`libc.so.6`, `libm.so.6`, `libpthread`, `librt`, `libdl`, `libutil`) and the unwinder from `libgcc_s.so.1`, which `una build` passes explicitly; Linux artifacts were not read for this inventory (section 1).

## 4. Open question: the system C library

Removing every crate of section 2 still leaves the Rust standard library, the unwinder, and Una's own `una-sys` (ADR 0116) calling the operating system through its C library. Whether that library counts as foreign code is not decided here. The facts and options per platform:

### macOS

- Facts: libSystem (`/usr/lib/libSystem.B.dylib`, which contains the C library, libm, pthread, libdispatch, libunwind and dyld's interfaces) is the only stable interface to the kernel that Apple supports. Apple treats the XNU system call interface as private and subject to change between releases, and does not support statically linked executables (Technical Q&A QA1118). Every Mach-O executable is loaded by dyld, which itself links libSystem. Programs built by Go, Zig and Rust on macOS all link libSystem for this reason.
- Options:
  1. Accept libSystem as the operating system's interface, like the kernel itself: Una ships no C, and calls into the system's library at a boundary listed and checked by the #1092 symbol check (an allowlist of imported symbols).
  2. Same as 1, and reduce the surface: replace the parts of libSystem that are only libraries, not system interface, with our own Rust code (math, `memcpy`/`memset`/`memcmp` and string routines, formatting, allocation on top of `mmap`), so that the imports left are system services.
  3. Raw system calls through our own Rust layer: not a supported ABI on macOS; programs break when Apple renumbers or changes a call.

### Linux

- Facts: the Linux system call ABI is stable and documented per architecture; the C library (glibc on the `-gnu` targets) is a replaceable user-space library. Today `una`, `libuna_rt.a` and linked programs use glibc dynamically and `libgcc_s` for unwinding; `una build` passes `-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc`. Rust's `-musl` targets link musl (C) statically; Rust has no supported `-linux-none` user-space target with a full standard library.
- Options:
  1. Keep glibc as the system interface, with the same allowlist check as macOS option 1.
  2. Link statically against a C library written in Rust (for example the `relibc` project or a library of our own) that implements the subset the standard library and `una-sys` need, and an unwinder in Rust (or `panic = "abort"` for linked programs, which removes the unwinder if the runtime does not rely on unwinding).
  3. A Rust layer over raw system calls (in the manner of the `rustix` crate's `linux_raw` backend) for `una-sys` and the runtime, with a standard library that does not need a C library: possible because the ABI is stable, but the Rust standard library for `*-linux-gnu` and `*-linux-musl` calls the C library, so this needs either a `no_std` runtime or a custom standard library build.
  4. Option 2 or 3 for linked programs only, with the toolchain (`una`) keeping the platform C library.
