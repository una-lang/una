# 0124. One binary: the solver, the runtime and the linker inside `una`

- Status: Accepted
- Date: 2026-10-18
- Deciders: Una project (accepted on 2026-10-27)
- Resolves: issue #891
- Related:
  - ADR 0003 (Z3 through SMT-LIB text over a subprocess), whose subprocess becomes a child process of `una` itself;
  - ADR 0055 (the native backend), whose rule NB-1 links through the system `cc`;
  - ADR 0059 (release archives ship the runtime), whose Alternative "embed the library" this record takes up again;
  - ADR 0106 (the optimized release backend), whose rule RB-1 compiles emitted C with the system C compiler. Where that compiler comes from is decided by a dated amendment of ADR 0106 after a measured prototype, not by this record (Question 1);
  - ADR 0041 and ADR 0064 (the proof cache), whose key gains the solver's build (rule 10);
  - ADR 0058 (`unsafe` at the runtime's C ABI boundary) and its amendment for `una-sys`;
  - ADR 0122 (Itá, Proposed), whose rule 1 runs the package manager inside `una`, also as `ita`, whose rule 18 says that the solver pin names the solver built into `una`, and whose rule 48 shares proofs across programs;
  - `docs/security/tcb.md` §3.2, §3.5 and gaps 1, 6 and 7;
  - issue #769 (the report's `tcb` section is incomplete).
- Target: the toolchain. No construct of the language changes, so no edition changes, and no edition-terms question arises (`spec/13 §7`).
- Amends:
  - **Verification:** `spec/12 §5` rules 1, 3, 4 and 6 (the solver in a child process, the proof condition, its limits), `§9` (the meaning of the pin, the cache key), `§10` (more non-deterministic outcomes);
  - **Diagnostics and project files:** `spec/13 §4` (one code), `§5` (`una --version`, `--solver-memory`, `--license`, `una notices`, `una guide`, `una build --debuggable`), `§6` (report fields);
  - **Native backend:** `spec/16 §1` rules 1 and 2 (the internal linker, the executables' targets), `§2` rule 3 (a failure to link), `§3` (RELRO at start-up);
  - **Accepted records:** ADRs 0003, 0035, 0055, 0058 and 0059, by the dated amendments of Appendix A;
  - **Documents:** `docs/compiler/architecture.md §7` (the crates `una-link` and `una-z3`), `docs/security/tcb.md` §3.2, §3.5, §3.9, `docs/release-gates.md` §2, `install.sh`, `scripts/release.sh`, `scripts/check-release.sh`.

  The rules of the internal linker would live in a new section of `spec/16`, "Linking".

> **In plain words.**
> - **What changes for a user.** Installing Una is downloading one file, `una`. With it alone a machine checks, proves, tests, runs and builds native executables. No Z3, no C compiler, no linker and no Rust toolchain are needed. The commands, flags and outputs stay as they are. (Release builds of ADR 0106, not implemented yet, follow when that record's amendment decides where their compiler comes from.)
> - **What is inside `una`.** Z3, built from pinned source; the native runtime, built from the same commit as `una`; and a linker written for Una that writes the executables itself.
> - **What gets safer.** Today `una check` runs whatever `z3` comes first on `PATH`, and `una build` runs whatever `cc` comes first, so a directory earlier on `PATH` can forge proofs or put code into every executable. Neither is looked up any more. The solver runs in a child process confined by the operating system, with limits on time and memory, and a proof now also requires that the solver exited normally.
> - **What it costs.** A larger `una`: about 50 to 70 MB on disk instead of 14 MB plus a 27 MB `z3` and a 28 MB runtime. Measured sizes are recorded for every release (rule 30). On Linux, `una` itself still needs glibc; the executables it writes need nothing.

## Questions for acceptance

Each question states what is being decided, its cost to the programmer, an example and the recommendation this draft follows.

- **Questions 3 to 8 are decided as recommended.**
- **Questions 1 and 2 are decided (2026-10-27) by the project's ruling on toolchain scope.** Una admits no foreign code inside a Una program: no C code is embedded in Una or in the executables `una` writes, and no C compiler goes into `una`. Calls of the operating system through its own interface (libSystem on macOS, the C library's system-call wrappers on Linux) are allowed, and so is hand-written assembly where it serves extreme performance, always with the same safety. Under that ruling the C-emitting backend of ADR 0106 is withdrawn (issue #1099), which closes Question 1, and Question 2 is decided as (a), where musl is the executables' interface to the operating system and nothing more. The rules that follow Question 2 (a) (rules 12, 16, 21, 22, 25 and 29, the musl inputs of rule 24's scan, and the Linux parts of rules 31 and 33 and of the Conformance) stand.

1. **Where does the C compiler of `una build --release` come from? Decided (2026-10-27): no C compiler goes into `una`.** The C-emitting backend of ADR 0106 is withdrawn (issue #1099), by a dated amendment of that record; the release path becomes Una's own code generation. The paragraph that follows records the direction this record stated before that ruling, and is kept for the history of the question. Not decided by this record either way. ADR 0106 (accepted, not yet implemented) runs the system C compiler. The direction this record states is a pinned clang built into `una`, run as a confined child (rule 9). The decision is a dated amendment of ADR 0106, taken after a measured prototype: the size of `una` with LLVM and clang linked in, the start-up of `una --version` and of a solver child with LLVM's static initializers and relocations in the binary, and the build time of the release gates. Steps 1 to 5 of the plan do not wait for it.
   - **Cost to the programmer until then:** release builds, once implemented, need a C compiler; nothing else does.
   - *Example.*

     ```text
     $ una build              # needs nothing but una
     built target/una/server
     $ una build --release    # until ADR 0106's amendment: needs cc
     ```

2. **What do Linux executables link against? Decided (2026-10-27): option (a), as recommended.** Today they link the system's glibc dynamically (`crates/una-cli/src/build.rs`, `link`). Two options:
   - **(a) Recommended: a static, position-independent executable with musl's C library inside it,** taken from the pinned Rust toolchain. It runs on any Linux of its architecture, with no shared library, and keeps address randomization. Recommended on three conditions, each a rule of this record: RELRO is really applied at run time (rule 25, probe P2); the runtime's allocator is chosen by measurement, preferring one that adds no C code to the runtime's trusted base, and otherwise reviewed as a C dependency (rule 21); and the name-resolution limit is documented in the guide.
   - **(b)** A dynamic executable against glibc. The internal linker would then write symbol versions and dynamic sections, against a list of glibc's symbols embedded in `una`.
   - **Cost to the programmer under (a):** host names resolve through `/etc/hosts` and the DNS servers of `/etc/resolv.conf` only, without the system's name-service modules (LDAP, mDNS). And `una` itself stays a glibc executable (rule 1): it runs on glibc distributions from the gate image's glibc on, not on musl distributions such as Alpine, although the executables it writes do.
   - *Example.*

     ```text
     $ una build && file target/una/hello
     target/una/hello: ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped
     $ scp target/una/hello alpine-box: && ssh alpine-box ./hello
     hello, world
     ```

3. **The guides and the license that the archive holds today.** The archive holds `LICENSE`, `README.md` and four guides beside `una` (`scripts/release.sh`, #410). With one file, they move inside the binary.
   - **Recommended:** `una --license` prints Una's license and the notices of everything built into `una`; `una notices` prints the notices of everything `una build` puts into an executable (the Rust standard library, the runtime's crates, and on Linux, under Question 2 (a), musl and LLVM's libunwind), for whoever distributes one; `una guide [name]` prints a guide, the list when no name is given. The archive holds only `una`.
   - **Decided:** as recommended.
   - **Cost to the programmer:** a guide is read with a command instead of a file.
   - *Example.*

     ```text
     $ una guide
     getting-started  syntax-reference  una-compact  una-for-agents
     $ una notices > dist/THIRD-PARTY-NOTICES.txt
     ```

4. **A memory limit for the solver.** Today a query may use all the memory of the machine.
   - **Recommended:** each solver child gets a fixed limit, 4 GiB by default, set with `--solver-memory <MiB>`; and the number of children running at once is bounded so that their limits together stay within half of the machine's physical memory (rule 6). The limit of one query never depends on the machine, so its result does not either; only how many run at once does. A query past its limit is `unknown` with the reason `memout`, an error like any `unknown`, never U8001 (rule 8).
   - **Decided:** as recommended.
   - **Cost to the programmer:** a proof that needs more than 4 GiB fails until the flag raises the limit; on a small machine, fewer queries run in parallel. The `limits` gate records the largest query of the repository's obligations.
   - *Example.*

     ```text
     $ una check
     error[U5002]: obligation unknown: the solver ran out of memory (memout, 4096 MiB)
     $ una check --solver-memory 16384
     ```

5. **The oldest macOS for the executables.**
   - **Recommended: macOS 13,** tested: the list of libSystem's symbols is taken from the macOS 13 SDK, so a symbol added later cannot be bound, and each release runs its executables on a macOS 13 virtual machine (rule 31). The Z3 that the installer downloads today is already built for macOS 13.3 (`install.sh`), so no current installation runs `una check` on an older one. `una` itself also targets macOS 13.
   - **Decided:** as recommended, macOS 13.
   - **Cost to the programmer:** an executable built by `una` does not start on macOS 11 or 12.
   - *Example.*

     ```text
     $ una build                       # on macOS 15
     built target/una/hello
     $ vtool -show-build target/una/hello | grep -E 'minos|sdk'
         minos 13.0
           sdk 13.3
     ```

6. **The solver under a sandbox that already confines `una` (macOS).** macOS does not let a sandboxed process apply a second sandbox profile of its own (`sandbox_init` fails). `una` run under `sandbox-exec`, as the project's `hunt` gate does and some agent and continuous-integration harnesses do, could then never confine its solver child.
   - **What this record decides:** when the child's own `sandbox_init` fails, the child asks the system whether it is already sandboxed (`sandbox_check` on itself). If it is not, the run fails with U5003, as any failure to confine does. If it is, the child checks each operation of a fixed list with `sandbox_check` (rule 7) and the report records which the outer profile denies (`tcb.solver_confinement: "outer"`, `tcb.solver_outer_denied`).
   - **The question: must the outer profile deny a minimum?** Recommended: **yes**. The outer sandbox counts as the confinement only if it denies at least outbound and inbound network access and writing files outside the temporary directory; otherwise the run fails with U5003, "the outer sandbox is too permissive for the solver". The alternative is to accept any outer sandbox and only record what it denies.
   - **Decided:** as recommended: yes, the outer profile must deny that minimum.
   - **Cost to the programmer:** none under a strict harness. A harness whose profile allows network access or writes anywhere must tighten it, or not run `una` under it; without this rule, `una check` would fail under every outer sandbox, and with no minimum, a permissive harness would leave the solver unconfined.
   - *Example.*

     ```text
     $ sandbox-exec -f ci.sb una check --json | jq -c '.report.tcb | [.solver_confinement, .solver_outer_denied]'
     ["outer",["file-write-data","network-inbound","network-outbound","process-exec*"]]
     $ sandbox-exec -p '(version 1)(allow default)(deny process-exec (literal "/bin/ls"))' una check
     error[U5003]: the outer sandbox is too permissive for the solver: network-outbound is allowed
     ```

7. **The hardened runtime on macOS executables.** It makes macOS refuse injected libraries (`DYLD_INSERT_LIBRARIES`) in a Una executable, and it also stops debuggers and profilers (`lldb`, Instruments, `dtrace`) from attaching to it.
   - **Recommended:** keep it on every executable, and add `una build --debuggable`, which also writes the entitlement `com.apple.security.get-task-allow`, so that the developer of a program can debug and profile it. The report records the flag, and `--release` refuses it.
   - **Decided:** as recommended.
   - **Cost to the programmer:** profiling a program means building it with `--debuggable`.
   - *Example.*

     ```text
     $ una build --debuggable && xctrace record --template 'Time Profiler' --launch target/una/server
     $ una build --release --debuggable
     error: --debuggable is for development builds; --release refuses it
     ```

8. **What the solver child's system-call filter does with a call it does not list (Linux).** `una` uses the system's glibc, which varies by distribution and version and may make calls the filter's tests never saw.
   - **Recommended:** an unlisted call fails with `ENOSYS`, as on a kernel that lacks it, so the call never runs and the C library takes its fallback; the dangerous families (opening files, sockets, starting programs, reading other processes' memory) kill the child at once, as tripwires; and a child killed by the filter is U5003, "the solver child broke its confinement", never a mere crash (rule 7).
   - **Decided:** as recommended.
   - **Cost to the programmer:** none; killing on every unlisted call would turn a newer glibc into failed proofs.
   - *Example.*

     ```text
     error[U5003]: the solver child broke its confinement (SIGSYS); this is a security event, report it
       --> verification.lock:3:20
     ```

## Glossary

- **Child process.** A process that `una` starts and waits for. Here, `una` itself, started again in an internal mode: re-executed.
- **Solver child.** `una` re-executed in its solver mode (`una __solver`), which runs Z3's SMT-LIB front end on one query.
- **Handshake.** The first line a child writes, naming the build it belongs to, which the parent checks before it reads anything else.
- **rlimit.** A per-process resource limit enforced by the kernel (`setrlimit`): processor time, address space, file size, core dumps, open files.
- **seccomp.** A Linux mechanism by which a process restricts, for itself and for good, the system calls it may make. `ENOSYS` is the error of a call the kernel does not have.
- **`sandbox_init`, `sandbox_check`.** The macOS interfaces by which a process confines itself under a profile, and asks whether a process is sandboxed.
- **Static archive.** A `.a` file: a collection of relocatable objects, from which a linker takes the members that define symbols still needed.
- **Relocatable object.** The output of a compiler for one unit: code and data with holes, the relocations, that the linker fills once addresses are known.
- **Relocation.** An instruction to the linker: write this address, or this distance, computed in this way, at this place. Its **place** is the bytes it writes.
- **Static-pie.** A statically linked executable that the kernel loads at a random address and that relocates itself at start-up.
- **musl.** A C library for Linux (MIT license), which the Rust toolchain ships for its `*-unknown-linux-musl` targets.
- **libSystem.** The C library of macOS. Apple requires every executable to link it dynamically, and makes no stable promise about system calls.
- **GOT.** The global offset table: a table of addresses that code reaches indirectly, filled by the linker or the loader.
- **TLS.** Thread-local storage: data that each thread has its own copy of.
- **Chained fixups.** The compact form, on macOS 12 and later, in which a Mach-O executable tells the loader which pointers to adjust or bind.
- **Ad-hoc signature.** A code signature with no signing identity: a hash of every page of the executable, which Apple silicon requires before it runs any code. Linker-signed: one written by the linker, which a later `codesign` may replace.
- **Hardened runtime.** A code-signature flag under which macOS refuses injected libraries and the attachment of debuggers.
- **RELRO.** Data that the start-up code adjusts and then makes read-only.
- **Oracle.** An independent implementation whose results are compared with the one under test: here, a system linker, and a standalone Z3.
- **Reproducible.** Two builds of the same inputs give the same bytes.

## Context

- **Three things besides `una`.** A user needs, besides `una`:
  - **Z3.** The verifier runs `z3` as found on `PATH`: `Settings::solver` is `PathBuf::from("z3")` (`crates/una-cli/src/verification.rs`, `settings`). It starts one `z3 -in -smt2` process per query, reads the version once per run with `z3 --version`, and kills the process at the wall-clock guard (`crates/una-verify/src/solver.rs`; `spec/12 §5` rules 1, 4 and 6). It reads the answer from stdout and ignores the exit status (`solver.rs`, `run`). A missing solver or another version is U5003.
  - **A C compiler as `cc`.** `una build` runs `cc` to link the object that Cranelift writes with the runtime (`crates/una-cli/src/build.rs`, `link`), with `-Wl,-S` (no debug information, #382), `-Wl,-dead_strip` on macOS or `-Wl,--gc-sections` on Linux, and, on Linux, `-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc`. A failure of the system linker is a usage error, exit code 2 (`spec/16 §2` rule 3).
  - **The runtime, `libuna_rt.a`.** `una build` finds it from `UNA_RT_LIB`, or in `lib/` beside `una`, next to `una`, or in `../lib` (`build.rs`, `runtime_library`; ADR 0059 rule 2). A runtime older than `una` fails the link on an `una_rt_*` symbol (#203).
- **The installer** (`install.sh`) downloads the release archive, checks it against the digest of the validation summary, downloads Z3 5.1.0 from the Z3 project's releases and checks it against a pinned digest, and installs `bin/una`, `lib/libuna_rt.a`, `z3/bin/z3` with a link `bin/z3`, the license, the readme and the guides. It records what it installed in `UNA_HOME/.una-install`, and `--uninstall` removes exactly that.
- **The archive** (`scripts/release.sh`, ADR 0035 rule 3, ADR 0059) holds `una`, `lib/libuna_rt.a`, `LICENSE`, `README.md` and four guides, and is reproducible (#679): the release builds with `--remap-path-prefix` and `-ffile-prefix-map`, and packs with fixed times, modes and owners. `scripts/check-release.sh` checks exactly those entries.
- **Sizes.** In an installation of 0.3.15 on `aarch64-apple-darwin`, `bin/una` is 13.8 MB, `lib/libuna_rt.a` 28.4 MB and `z3/bin/z3` 27.4 MB (file sizes, 2026-10-18).
- **The versions they need.** The Z3 assets that `install.sh` pins are built for macOS 13.3, glibc 2.39 on x86-64 Linux and glibc 2.38 on arm64 Linux.
- **The trusted base.** `docs/security/tcb.md` ranks the system `cc` and linker as unpinned and unrecorded (§3.5, gap 7), and the report's `tcb` section omits the runtime, the linker and their versions (gap 6, #769). The solver is ranked second, "soundness" (§3.2).
- **The `sanitize` gate** links the native equivalence run against an AddressSanitizer runtime "through a `cc` placed on that one command's `PATH`" (`docs/release-gates.md` §2), and the `hunt` gate runs every compiler run under `sandbox-exec` on macOS.
- **`unsafe`.** Only `crates/una-rt/src/ffi.rs` and, by the amendment of ADR 0058 made by ADRs 0116 and 0117, the crate `una-sys` may hold `unsafe` code. Every other crate forbids it. `una-sys`'s limit is 800 lines.
- **The runtime unwinds.** `una-rt` catches panics at its boundary (`crates/una-rt/src/runtime.rs`, `catch_unwind`), so an executable needs its unwind tables.
- **The runtime's allocator** is `Cap<System>`, the system allocator under a counting cap (`crates/una-rt/src/lib.rs`).
- **Name resolution.** `una-http` resolves host names through the standard library's `to_socket_addrs` (`crates/una-http/src/client.rs`), which calls the C library's `getaddrinfo`.
- **The proof cache's key** is SHA-256 over the lock and the query (`spec/12 §9`). ADR 0122 rule 48 (Proposed) makes it domain-tagged, over the lock's `solver` section and the query, and shares it across programs in `~/.una/itaoca/proofs/`.
- **Earlier decisions.**
  - ADR 0003 chose a subprocess over Rust bindings to Z3, so that every query stays text that can be saved, diffed and replayed. That reason stands; the subprocess becomes `una` itself.
  - ADR 0059 rejected embedding the runtime in `una`, because it doubles the binary's size for every command, and Cargo cannot order the runtime's build before `una`'s without a build script. The project has since decided that a bigger binary is acceptable; rule 18 orders the builds.
  - ADR 0106 rejected building LLVM into `una` for its size, and chose the system C compiler because `una build` already needed `cc` as its linker. Neither reason holds once nothing needs `cc`, which is why Question 1 goes to an amendment of ADR 0106.
- **The goal (#891).** One file to install. The project fixed the direction on 2026-10-18: Z3 linked in statically from pinned source (MIT license), run in a child process of `una` itself, so that crashes and limits stay isolated; the runtime embedded; an internal linker for ELF (Linux x86-64 and arm64) and Mach-O (macOS arm64, with an ad-hoc signature), with no `cc`; one file from the installer. A bigger binary is accepted.

### Departures from the issue

1. **The release compiler (Question 1)** is left to an amendment of ADR 0106; the issue does not mention it.
2. **The solver child is confined** by the operating system (rule 7), beyond the isolation of a separate process that the issue asks for, and a proof requires a normal exit (rule 8).
3. **A memory limit for the solver** is new (Question 4).
4. **The guides and the license move inside `una`** (Question 3), so that the archive is one file.
5. **Development builds** may read the runtime from a file and may link through a system `cc` (rule 19), for the `sanitize` gate. Release builds contain neither.
6. **The proof cache's key** gains the solver's build, so caches are verified once again per machine (rule 10).

## Decision

In one paragraph: the release `una` contains Z3, built from a pinned commit by a committed recipe; the native runtime, built from the same commit; for Linux, the C library and start-up objects of the pinned Rust toolchain's musl target; and a linker written for Una. `una check` runs each query in a child process that is `una` re-executed in its solver mode, confined by the operating system or verified to be inside an outer sandbox, and bounded in time and memory, with the number of children bounded too. A proof requires the child's handshake, a normal exit and `unsat`. `verification.lock` keeps its keys: its `solver_version` names the Z3 built into the running `una`, another value is U5003, and the proof cache's key includes the solver's build. `una build` links in memory, with no external program, into a static-pie ELF executable on Linux whose RELRO the runtime protects at start-up, and a Mach-O executable with an ad-hoc signature and the hardened runtime, linking only libSystem, on macOS. The linker refuses relocations it cannot honor in a position-independent executable, keeps every relocation and symbol inside its own section, links only inputs that `una` itself carries, and writes the same bytes on every machine. The installer and the archives ship one file. Each step ships alone, and each adds a test on a machine with no `cc`, no Rust and no Z3.

### A. The solver inside `una`

1. **The build.**
   - **The source** is the commit of Z3's release tag, pinned in `supply-chain/z3.lock` by its commit hash, its git tree hash, and the SHA-256 of a source tarball made from that tree and mirrored in the project's own release storage under its digest. Archives that a hosting service generates on request are not used, since their bytes are not promised to stay the same. The digest is checked before anything is compiled, and a mismatch stops the build.
   - **The recipe** is committed in `supply-chain/z3/`, reviewed like code: the CMake options (`Z3_BUILD_LIBZ3_SHARED=OFF`; `Z3_USE_LIB_GMP=OFF`, so nothing under the LGPL is linked; `Z3_INCLUDE_GIT_HASH=OFF`; `Z3_INCLUDE_GIT_DESCRIBE=OFF`; no bindings, no examples), the compiler flags written out in full, which match the flags of Z3's own release builds plus `-ffile-prefix-map` for the source and build directories, and the build of the shell's SMT-LIB 2 front end (`src/shell/`) as a library whose entry is renamed so that it does not clash with `una`'s `main`, which upstream CMake does not do.
   - **`-ffp-contract=off`** is not used unless the `solver-identity` gate (rule 31) passes with it against the upstream comparison; if adopted, it is a reproducibility choice only. Floating-point results of Z3's heuristics are not claimed to agree across targets.
   - **Toolchain.** On Linux, the C++ compiler, CMake and linker of the pinned image of `scripts/local-ci/`; on macOS, Apple clang of the Xcode version that the release record names. Each version is written into the validation summary. The macOS version is recorded, not pinned, so a third party with another Xcode may not reproduce the bytes, as today.
   - **Built once per pin.** The library is built and its reproducibility shown (two cold builds, equal digests) when the pin or the recipe changes, and stored in the project's release storage under its digest, with the digest in `supply-chain/z3.lock`. Release builds take it from there by digest, so the `release` and `reproducible` gates check a digest instead of rebuilding Z3 (rule 31). A cold build of Z3 takes on the order of 10 to 20 minutes per target on the gate machine, more under emulation; it is measured and recorded in step 1.
   - **What is linked.** On Linux, `libstdc++` and `libgcc` are linked statically, and `una` depends dynamically only on glibc, from the version of the gate image's glibc on; `una` does not run on musl distributions. On macOS, `una` uses the system's `libc++` and targets macOS 13.
2. **Where the call into Z3 lives.** A new crate, `una-z3`, holds only the declaration of the front end's entry, as `extern "C"` (not `"C-unwind"`, so that a C++ exception that escapes aborts the child, which the parent sees as `crash`), the construction of its arguments, and the call. It is a third place where `unsafe` may appear, under the rules of ADR 0058 (Appendix A): at most 150 lines, no dependency on a crate of the repository but `una-sys`, and a security review of every change. The entry refuses to run, and aborts, unless `una-sys` has recorded that the process's confinement succeeded (rule 7), so no path of `una` can reach Z3's engine unconfined. Z3's static initializers still run in every `una` process, the parent and the language server included; they read no input.
3. **The solver mode.** `una __solver` is internal: it is not listed by `--help`, and its interface may change in any release. It runs the steps of rule 7, writes the handshake of rule 4, and then runs Z3's front end exactly as `z3 -in -smt2` runs it, on the script it reads on stdin, with the options of rule 6. Its stdout is read by the parser of `crates/una-verify/src/solver.rs`, unchanged apart from the handshake and the proof condition of rule 8.
4. **The handshake.** The child's first line on stdout is

   ```text
   una-solver 1 z3 5.1.0 <build>
   ```

   where `1` is the version of this protocol and `<build>` the SHA-256 of `una`'s embedded build record (rule 20), in hex. The parent reads at most 256 bytes for it, within 2 seconds counted inside the wall-clock guard, and compares it with its own values before it reads the answer. A line that differs, is too long or late, or is missing is U5003, "the solver child is not this `una`". The handshake detects an accidental replacement, such as an upgrade of `una` during a run; it is not a defense against a malicious one, since its value is public.
5. **Starting the child.** The parent starts its own executable: `/proc/self/exe` on Linux, which names the running file even after it is renamed or replaced, and `std::env::current_exe()` on macOS, guarded by the handshake. On a Linux system without `/proc`, the run fails with U5003, "cannot re-execute `una`: /proc is not mounted".
   - The environment is empty, the working directory is `/`, stdin and stdout are pipes, and stderr is a pipe whose first 4 KiB are kept for the reason of a crash.
   - **Descriptors.** On macOS the child is spawned with `POSIX_SPAWN_CLOEXEC_DEFAULT`, so it inherits only its three pipes. On Linux the child calls `close_range` on every descriptor above 2, or, on a kernel without it, closes the descriptors that `/proc/self/fd` lists; never a loop up to the descriptor limit.
   - One child per query, as today (`spec/12 §5` rule 6).
6. **Limits.**
   - **Time.** The wall-clock guard stays the parent's (`--solver-wallclock`, default 60 s): past it, the child is killed and the answer is `unknown` with reason `wallclock`, as today. As a backstop, the child's processor time is limited to the guard plus 1 s (`RLIMIT_CPU`); a child ended by `SIGXCPU` gives `unknown` with reason `cpu`.
   - **Memory, per child.** `--solver-memory <MiB>`, default 4096, from 256 to 1,048,576 (Question 4). Z3's own limit is set to it (`memory_max_size`). As a backstop, on Linux the address space is limited to the limit plus 1 GiB (`RLIMIT_AS`), except in development builds instrumented by AddressSanitizer, whose shadow memory `RLIMIT_AS` would break. macOS does not enforce `RLIMIT_AS`, so there the parent reads the child's physical footprint (`ri_phys_footprint`, through `proc_pid_rusage`), which counts compressed memory, every 10 ms, and kills it past the limit plus 1 GiB.
   - **Memory, all children together.** The number of children running at once is at most the number of processor cores, and at most `max(1, ⌊M / (2 × L)⌋)`, where `M` is the machine's physical memory, or the memory limit of the container or control group that `una` runs in when that is lower (on Linux, the `memory.max` of its cgroup), and `L` the per-child limit. The per-child limit is fixed, so a query's outcome does not depend on how many run beside it.
   - **Everything else.** No core dump (`RLIMIT_CORE` 0), no file written (`RLIMIT_FSIZE` 0), at most 16 open files (`RLIMIT_NOFILE`).
7. **Confinement.** Before it reads the query, the child, in this order: closes descriptors (rule 5), sets the limits of rule 6, and confines itself:
   - **Linux** (Question 8):
     - `PR_SET_NO_NEW_PRIVS`, then a seccomp filter;
     - the filter first checks the architecture (`AUDIT_ARCH_X86_64` or `AUDIT_ARCH_AARCH64`) and, on x86-64, refuses a call with the x32 bit (`__X32_SYSCALL_BIT`); both kill the process;
     - **allowed:** the calls the front end needs, listed and pinned in `una-sys`, such as `read`, `write`, `close`, `fstat`, memory mapping and protection without execution, `madvise`, `brk`, futexes, `rseq`, `set_robust_list`, `sigaltstack`, `rt_sigaction`, `rt_sigprocmask`, clocks, `getrandom`, `sched_getaffinity`, `clone` with the flags of a thread (`CLONE_THREAD`), signals to its own process only, and exit;
     - **before the filter,** the child does once what the C library would otherwise open a file for during the query, such as reading the number of processors (`/sys/devices/system/cpu/online`) and the time zone (`/etc/localtime`), so that no later call needs a file. Probe P6 shows whether that is enough. If a C library still opens a file during a query, `open`, `openat` and `openat2` move from the tripwires to `SECCOMP_RET_ERRNO(EACCES)`: the call still never runs, and the C library takes its fallback;
     - **tripwires, which kill the process (`SECCOMP_RET_KILL_PROCESS`):** `open`, `openat`, `openat2`, `creat`, `socket`, `socketpair`, `connect`, `bind`, `accept`, `accept4`, `execve`, `execveat`, `ptrace`, `process_vm_readv`, `process_vm_writev`, `fork`, `vfork`, `clone` without `CLONE_THREAD`, `kill`, `tkill` and `tgkill` toward another process, and `mmap` or `mprotect` asking for execution;
     - **every other call fails with `ENOSYS`** (`SECCOMP_RET_ERRNO`), so it never runs, and a newer C library takes the fallback it has for older kernels. `clone3`, whose flags a filter cannot read, is in this class, so the C library falls back to `clone`.
   - **macOS** (Question 6): `sandbox_init` with a profile that denies everything by default and allows only what the front end needs, with no file access, no network and no process creation: `(version 1)(deny default)`, plus the few allowances the front end needs. One is decided: the C++ library asks for the number of processors with `sysctlbyname("hw.ncpu")` when Z3 asks for it, so reading it once before confining would not serve later calls; the profile allows `sysctl-read` of that one name and no other. The rest comes from probe P7 on the real front end. The profile is a string of the sandbox profile language passed to `sandbox_init`, built into `una`; probe P7 checks that every supported macOS accepts it. If `sandbox_init` fails and `sandbox_check` on the child's own process reports that it is already sandboxed, the child checks with `sandbox_check`, one by one, the operations `network-outbound`, `network-inbound`, `file-write-data` at `/` and at a path outside the temporary directory that the parent passes, `file-read-data` at the same paths, `process-exec*` with a path filter naming `/bin/sh` (`sandbox_check` refuses the plain name `process-exec` with `EINVAL`), and `process-fork`, and writes the list of those denied after its handshake. Under Question 6's minimum, the outer sandbox counts as the confinement only if it denies both network operations and both writes; otherwise U5003. The interface is deprecated; it is used because it is the one the system offers to a process confining itself.
   - **What a failure is.** A child that cannot confine itself writes no handshake and exits; the run fails with U5003, "the solver could not be confined". There is no unconfined fallback. A child ended by `SIGSYS` broke the filter: U5003, "the solver child broke its confinement (SIGSYS)", never `crash`.
   - **Where the code lives.** The confinement, the limits, the spawn attributes and the footprint reading are in `una-sys`, behind a Cargo feature `solver` that only `una-cli` enables (rule 35).
   - **Recorded.** The report's `tcb.solver_confinement` is `"own"` or `"outer"`, and, for `"outer"`, `tcb.solver_outer_denied` lists the operations the outer profile denies, in the order above.
8. **The outcome of a query.**
   - **Proven** only when all of these hold: the handshake matched; the child exited normally, by its own exit, with status 0, not by a signal; and its status line is `unsat`. Anything else is not a proof. This is stricter than today, where the exit status is ignored.
   - **Refuted:** a status line `sat` from a child that ended by its own exit, not by a signal, with status 0, or with status 1 when `sat` is followed on stdout by an `(error ...)` line, which is how Z3 5.1.0's front end ends after an error that follows `sat` (probe P4). The rules of `spec/12 §5` rule 6 for an `(error ...)` after `sat` and for missing values are unchanged, so the refutations of ADR 0101, whose counterexample has no values and names the solver's message, stay refutations.
   - **Memout:** Z3 5.1.0's front end reports exceeding `memory_max_size` by exit status 101, with `(error "out of memory")` on stderr and nothing on stdout (probe P4). That exit, a kill by the parent for memory, or an `RLIMIT_AS` failure, gives `unknown` with reason `memout`, never `crash` and never U8001. Both values are pinned for the pinned version, and checked again when the pin changes.
   - **Crash:** any other signal, or an exit without a status line, gives `unknown` with reason `crash`, and the diagnostic names the signal or the exit code. A crash is never a proof, never cached, and never U8001: it is not evidence that the query is invalid.
   - **U8001** stays the outcome of an `(error ...)` line before the status that is not the memout message (ADR 0033).
   - `memout`, `crash`, `cpu` and `wallclock` are never cached, and are marked non-deterministic in the report.
9. **The release compiler, if ADR 0106's amendment builds one in.** It runs as `una __cc1`, under rules 4 to 8, reading the emitted C on stdin and writing one object to stdout, with no file read: the generated header is part of the input, and the compiler's own headers are built into `una`. The amendment decides the rest.
10. **The pin and the cache.**
    - `verification.lock` keeps the keys of `spec/12 §9`. `solver = "z3"` and `solver_version` name the solver built into the running `una`. Any other value is U5003, with no process started, and the message names both versions:

      ```text
      error[U5003]: verification.lock pins z3 5.0.0, and this una (0.4.0) has z3 5.1.0 built in
        --> verification.lock:3:20
      ```

    - **One `una`, one solver.** A package that pins another version is checked with a `una` that has that version, or after its lock is updated, which may change which obligations are proven, as a change of the pinned Z3 does today.
    - **The cache key includes the solver's build.** From step 1, the key of `spec/12 §9` is SHA-256 over the lock, the solver's build digest (rule 20's `z3_library_sha256`) and the query; under ADR 0122 rule 48, the build digest is one more length-prefixed field after the domain tag. Entries of the standalone Z3, or of another build, are then simply not found, and each machine verifies its caches once again. The HMAC of ADR 0064 is unchanged.
    - **Development builds** (rule 19) write and read only the package's own `.una/cache/`, never the shared store of ADR 0122, and their build digest differs from every release's.
    - **`una --version`** keeps its text and its `una.version/1` keys (`spec/13 §5`): `z3: 5.1.0` now names the built-in solver.
11. **No other solver is run.** `una` never looks up `z3` on `PATH`, and no variable names another solver at run time.

### B. The runtime inside `una`

12. **What is embedded,** for the target of the `una` binary:
    - **the runtime:** `libuna_rt.a`, built from the same commit and the same build as `una` (rule 18), without debug sections, which it removes at that build;
    - **on Linux,** from the pinned Rust toolchain's `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` target (the `self-contained` directory of its `rustlib`): musl's `libc.a`, the start-up objects of a static-pie executable (`rcrt1.o`, `crti.o`, `crtn.o`, `crtbeginS.o`, `crtendS.o`) and `libunwind.a`;
    - **on macOS,** the list of symbols that libSystem exports, with the re-exported libraries' symbols, read at that build from the `libSystem.tbd` of the macOS 13 SDK, pinned by its digest, so that no symbol added after macOS 13 can be bound (Question 5);
    - **for each member,** a symbol index computed at that build, so that `una build` finds the member that defines a symbol without reading the others.
13. **How `una build` uses it.** The linker reads the embedded bytes in place. Nothing is written to disk but the executable.
14. **Versioned with the binary.** The embedded runtime is the one `una` was built with, so the failure of #203, a runtime older than `una`, cannot occur in a release build. Its SHA-256 is part of `una`'s build record (rule 20) and of the report (rule 28).
15. **`UNA_RT_LIB`** is read only by development builds (rule 19). A release `una` ignores it: a variable that replaces the runtime of every executable would be a way to put code into them.
16. **The runtime's target.** On Linux, the runtime is built for the musl target (Question 2), and Cranelift compiles for the same triple. On macOS it stays `aarch64-apple-darwin`.
17. **Cross-compilation** would embed other targets' runtimes too. It is not part of this record (`spec/16 §1` rule 2 keeps "each host builds for itself").
18. **Build order.** `scripts/release.sh` builds the runtime first, then `una`, passing the runtime's path and digest in `UNA_EMBED_RUNTIME` and `UNA_EMBED_RUNTIME_SHA256`, and Z3's library, taken by digest (rule 1), in `UNA_EMBED_Z3` and `UNA_EMBED_Z3_SHA256`. The build script of `una-cli` checks each digest and embeds the bytes with `include_bytes!` from its output directory. These variables are read at build time only; a built `una` never reads them. No build script runs Cargo, downloads anything or compiles C.
19. **Development builds.** Without those variables, `una` is built with no embedded runtime and no embedded Z3, and says so: `una --version` prints `development build` on its last line.
    - It reads the runtime as today (`UNA_RT_LIB`, `lib/`, beside `una`, `../lib`), and links with the internal linker.
    - It links Z3 at build time from the library that `scripts/build-z3.sh` builds once from the pinned source and recipe into `target/z3/<digest>/`, named by `UNA_Z3_LIB` at build time; without it, `una check` is U5003, "this development build has no solver built in; run `scripts/build-z3.sh`". It never runs a `z3` from `PATH`.
    - **An external link, for the `sanitize` gate only.** A development build compiled with the Cargo feature `dev-external-link` accepts `una build --external-linker <cc>`, which links through that program as `build.rs` does today, so that the gate can link the AddressSanitizer runtime, a dynamic library on macOS. The feature is absent from release builds, `scripts/check-release.sh` refuses a binary that has it, and the gate keeps its canary that AddressSanitizer is active. This path, and the `sanitize` gate's checking toolchain, are developer-only and are outside the `toolchain-free` gate (rule 31).
    - The release gates refuse a development build (rule 31).
20. **The build record.** `una` embeds a small JSON document: its version, the commit, the Z3 commit and tree hashes, the SHA-256 of Z3's source tarball (`z3_source_sha256`) and of the built library (`z3_library_sha256`), of the runtime, and, on Linux, of each musl input. The handshake hashes it (rule 4), the cache key uses `z3_library_sha256` (rule 10), and the report lists the digests (rule 28).
21. **The allocator on Linux.** Under Question 2 (a), the runtime's global allocator is not musl's `malloc`. It is a pinned allocator, chosen in step 4 of the plan by measurement on the allocation-heavy programs of the release performance benchmarks against the same programs under glibc, preferring one written in Rust, which adds no C code to the runtime's trusted base; a C allocator is reviewed as a C dependency. It is wrapped in the same `Cap` that counts and limits memory (ADR 0061), and added through the supply-chain record and the `deps` gate. musl's `malloc` remains only for musl's own internal allocations.

### C. The internal linker

22. **Scope.** A crate, `una-link`, in safe Rust (`unsafe_code = "forbid"`), links:
    - **inputs:** the program's object from Cranelift; under ADR 0106, the release compiler's object; the embedded runtime; on Linux, musl's `libc.a`, its start-up objects and `libunwind.a`;
    - **outputs:**
      - **Linux, x86-64 and arm64:** a static-pie ELF executable. No interpreter, no shared library, no symbol versions.
      - **macOS, arm64:** a Mach-O executable for macOS 13 and later (Question 5), that links one library dynamically, `/usr/lib/libSystem.B.dylib`, as Apple requires.
    - **never an input:** an object, an archive or a library that a package or the user supplies. Una admits no foreign code, so every input is one of the above, generated by `una` or built by the project and carried inside `una`.
    - **out of scope:** shared libraries, linker scripts, any flag a package supplies, link-time optimization (under ADR 0106 it would happen inside the compiler, before the link), cross-compilation, and debug information (rule 26).
    - **Object reading** uses the `object` crate, already in `Cargo.lock` (0.40.0, through `cranelift-object`), for ELF, Mach-O and `ar`. The writer is `una-link`'s own.
23. **Resolution.**
    - **Order.** The inputs are read in this fixed order: the program's objects, the runtime, then the C library and its start-up objects on Linux.
    - **Archives.** A member is loaded when it defines a symbol that is undefined at that point; loading repeats until nothing changes, over all archives, so their order does not matter for what is found (as `ld.lld` does). Among members that define the same symbol, the first in input order wins.
    - **Duplicates.** Two strong definitions of one symbol in loaded inputs are an error. A weak definition yields to a strong one. COMDAT groups and Mach-O weak definitions keep the first.
    - **Linker-defined symbols.** The linker defines exactly these, and no input may define them: on ELF, `__start_<section>` and `__stop_<section>` for sections whose names are C identifiers, `__init_array_start`, `__init_array_end`, `__fini_array_start`, `__fini_array_end`, `__preinit_array_start`, `__preinit_array_end`, `_DYNAMIC`, `_GLOBAL_OFFSET_TABLE_`, `__ehdr_start` and `__dso_handle`; on Mach-O, `__mh_execute_header`.
    - **What stays undefined** at the end is an error, except, on macOS, a symbol that the embedded export list of libSystem holds, which is bound to libSystem (two-level namespace, ordinal 1). A symbol outside that list is an error, so the runtime cannot come to need another library, or a newer macOS, unnoticed.
    - **Dead code.** Sections, and on Mach-O the atoms that `.subsections_via_symbols` delimits, that are not reachable from the entry point, the initializers and the exported symbols through relocations, are dropped. An `.eh_frame` FDE is kept only if its function is live, and its LSDA and personality routine are kept alive only through a kept FDE. This keeps today's `--gc-sections` and `-dead_strip`: an executable that never uses `Http` carries no TLS stack (ADR 0074 H7).
24. **Relocations.**
    - **The lists are derived, not assumed.** `scripts/scan-relocations` lists every relocation type, by section kind and symbol kind, in the actual inputs: Cranelift 0.136's objects for the repository's programs, the Rust 1.98.1 standard library of each target, `ring` 0.17's assembly, and the runtime. The section "Linking" of `spec/16` lists the result per target, and the scan runs again at every bump of the Rust toolchain, Cranelift or a crate with assembly, before the bump merges; a new type is added with its oracle test, or the bump waits. The expected sets are:
      - **ELF x86-64:** `R_X86_64_64`, `PC32`, `PC64`, `PLT32` (resolved directly, with no PLT), `GOTPCREL`, `GOTPCRELX`, `REX_GOTPCRELX` (relaxed to an address computation when the target is local), `32` and `32S` under the restriction below, and the TLS types `TPOFF32`, `GOTTPOFF`, `TLSGD`, `TLSLD`, `DTPOFF32`, the general- and local-dynamic sequences relaxed to the local-exec model as the psABI describes, both in their form that calls `__tls_get_addr` through `PLT32` and in the form through `GOTPCRELX` that Rust's default of no PLT produces;
      - **ELF arm64:** `R_AARCH64_ABS64`, `PREL64`, `PREL32`, `CALL26`, `JUMP26`, `CONDBR19`, `TSTBR14`, `ADR_PREL_PG_HI21`, `ADR_PREL_LO21`, `ADD_ABS_LO12_NC`, `LDST8` to `LDST128_ABS_LO12_NC`, `ADR_GOT_PAGE`, `LD64_GOT_LO12_NC`, `ABS32` and `MOVW_UABS_G0` to `G3` under the restriction below, and the TLS types `TLSLE_*`, `TLSIE_*` and `TLSDESC_*`, the last two relaxed to the local-exec model;
      - **Mach-O arm64:** `ARM64_RELOC_UNSIGNED`, `SUBTRACTOR`, `BRANCH26`, `PAGE21`, `PAGEOFF12`, `GOT_LOAD_PAGE21`, `GOT_LOAD_PAGEOFF12`, `POINTER_TO_GOT`, `TLVP_LOAD_PAGE21`, `TLVP_LOAD_PAGEOFF12` and `ADDEND`.
    - **Position independence.** The executable is loaded at a random address, so:
      - a narrow absolute relocation (`R_X86_64_32`, `R_X86_64_32S`, `R_AARCH64_ABS32`, `R_AARCH64_MOVW_UABS_G0` to `G3`) is allowed only against an absolute symbol (`SHN_ABS`) or in a section that is not loaded; otherwise it is an error, since no load-time fixup can express it;
      - a 64-bit absolute relocation against a symbol that is not absolute (`R_X86_64_64`, `R_AARCH64_ABS64`) is allowed only in a writable or RELRO section, where it becomes a relative fixup; in any other loaded section it is an error, the equivalent of `-z text`;
      - on Mach-O, an `ARM64_RELOC_UNSIGNED` pointer is allowed only in `__DATA` or `__DATA_CONST` and at an 8-byte-aligned place, as chained fixups require; otherwise it is an error. A pair of `ARM64_RELOC_SUBTRACTOR` and `ARM64_RELOC_UNSIGNED`, which writes the distance between two symbols and needs no fixup, is exempt, in `__TEXT,__eh_frame` and `__TEXT,__gcc_except_tab` as anywhere else it is resolved at link time.
    - **Bounds.** For every input: the place of every relocation, with its width, lies inside its own input section, and is applied to that section's bytes only; every defined symbol of a section satisfies `value + size ≤ size of the section`; and input sections never overlap in the output. A violation is an error. Applying a relocation computes its place from its own section's output offset, never from an offset into the whole output buffer.
    - **Sections that are not loaded.** Relocations in sections that the output does not contain (`.debug_*`, `.comment`, `__DWARF`) are ignored, never refused, so that inputs with debug information link.
    - **Never supported:** `R_*_IRELATIVE` and indirect functions (musl has none), copy relocations, text relocations, and anything that needs code to be written at load time.
    - **Errors.** An unsupported type, a value that does not fit its field, or a violation of this rule is an error that names the type, the input, the section and the symbol (Diagnostics).
    - **Range.** A branch whose target is beyond its reach (±128 MiB on arm64) is U7015. There are no range-extension thunks in this record; no program of the repository comes within a factor of 50 of that size.
25. **Layout and loading.**
    - **Alignment.** ELF segments are aligned to 64 KiB on arm64 (`p_align` 0x10000), so that the executable loads on kernels with 4, 16 or 64 KiB pages, and to 4 KiB on x86-64. Mach-O segments are aligned to 16 KiB.
    - **ELF.** Segments, in order: read-only data, code, RELRO data, writable data and `.bss`, none both writable and executable. `PT_PHDR`, `PT_LOAD`, `PT_DYNAMIC` (for the relative relocations only), `PT_TLS`, `PT_GNU_EH_FRAME` with a synthesized `.eh_frame_hdr` and its sorted search table, `PT_GNU_STACK` not executable, and `PT_GNU_RELRO`, whose end is padded to the segment alignment so that a page protection covers exactly it. Relative relocations are written as `DT_RELA`, which musl's `rcrt1.o` applies at start-up; `DT_RELR` only if a gate shows that the embedded musl applies it. `.init_array` and `.fini_array` keep their order and priorities, since the Rust standard library uses them.
    - **RELRO at run time.** Writing `PT_GNU_RELRO` does not protect anything unless the start-up code does. glibc's static-pie start-up does; musl's is not known to (probe P2). So the runtime's start-up, in `ffi.rs`, as the first thing its `main` does, finds `PT_GNU_RELRO` through the program headers (`getauxval(AT_PHDR)`) and makes that range read-only with `mprotect`, failing the run (U8001) if it cannot. Before that, only the C library's start-up and the standard library's initializers have run, and no input has been read. If probe P2 shows that musl does protect the range, the runtime's call stays as a check that it is read-only. The claim "full RELRO" holds only with the conformance test that reads `/proc/self/maps` at run time.
    - **Mach-O.** `__PAGEZERO` (4 GiB); `__TEXT` with code, stubs, constants, strings, `__unwind_info` (built from the inputs' `__compact_unwind`, with the GOT entries of its personality routines) and `__eh_frame` for the entries that need it; `__DATA_CONST`, marked `SG_READ_ONLY`, with the GOT and constants, read-only after the fixups; `__DATA` with data, the thread-local descriptors `__thread_vars` (each bound to libSystem's `__tlv_bootstrap`), `__thread_data` followed by `__thread_bss`, and zero-fill; `__LINKEDIT`. Load commands: `LC_SEGMENT_64`, `LC_DYLD_CHAINED_FIXUPS`, `LC_DYLD_EXPORTS_TRIE`, `LC_SYMTAB`, `LC_DYSYMTAB`, `LC_LOAD_DYLINKER`, `LC_UUID`, `LC_BUILD_VERSION` (minimum macOS 13, and the `sdk` field set to the SDK version whose export list is embedded, since libSystem keys some behavior on it), `LC_MAIN`, `LC_LOAD_DYLIB` (libSystem), `LC_FUNCTION_STARTS`, `LC_DATA_IN_CODE` and `LC_CODE_SIGNATURE`. Every import is bound at load, none lazily, so no writable stub table exists.
    - **The entry.** On Linux, `_start` of musl's `rcrt1.o`, which relocates the executable before it calls `_start_c`; on macOS, `LC_MAIN` naming `_main`. The runtime's `main` stays the one that calls the program (`spec/16 §3`).
26. **Symbols and debug information.** As today (`-Wl,-S`, #382), executables carry no debug information: no DWARF sections and no debug map, which would name the paths of the machine that built `una`. They keep a symbol table of function names, so that profilers and crash reports name functions (on macOS, with `--debuggable`, Question 7). No input path, user name, time or random value is written.
27. **The ad-hoc signature (macOS).** The last part of `__LINKEDIT` is a code signature: a SuperBlob holding one CodeDirectory of version 0x20500, with SHA-256 page hashes of 4 KiB pages over the file up to the signature, the executable segment's range, the flag of a main executable, the flags `adhoc` and `linker-signed`, and the flag `runtime` (hardened runtime, Question 7), with its runtime version field. `ld64` does not write `linker-signed` together with `runtime`; probe P3 showed that macOS enforces the hardened runtime on this combination (on macOS 26.6.2, `DYLD_INSERT_LIBRARIES` was refused and `codesign -v` accepted the file). The probe's signature had a requirements slot, which this layout does not, so the conformance test checks the exact layout of this rule; if macOS refuses that layout, the signature gains an empty requirements slot rather than dropping the hardened runtime. With `--debuggable`, the signature also holds an entitlements blob with `com.apple.security.get-task-allow`. The identifier is the output file's name, without its directory. There are no requirements. `LC_UUID` is the first 16 bytes of the SHA-256 of the file with the UUID and the signature zeroed, marked as a version-4 UUID. A user who distributes an executable may replace the signature with their own (`codesign --force --sign`); notarization is theirs.
28. **What is recorded.** The build report's `native` section gains `linker` (`"una <version>"`), `runtime_sha256` and `debuggable`; the `tcb` section gains `solver_build` (rule 20's two Z3 digests), `solver_confinement` and `solver_outer_denied` (rule 7). `solver` keeps its text, `z3 <version>`. Every key is additive, and the schema stays `una.report/2` (ADR 0076). This closes the parts of #769 that concern the runtime and the linker.
29. **Determinism.** The executable is a function of the inputs' bytes, the `una` version, the output's file name and `--debuggable`: two builds, on any two machines of the same target, from any directories, by any users, write the same bytes. Inputs, sections and symbols are processed in a fixed order, never in the order of a hash map or of threads; parallel work, such as hashing the signature's pages, writes to fixed offsets. `una.build/1` keeps its keys; its `target` now names the executable's target, `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` on Linux.

### D. Size, start-up, installation and release

30. **Size and start-up.**
    - **Size.** Expected, on `aarch64-apple-darwin`: `una` about 14 MB today, plus Z3 (its standalone binary is 27.4 MB), plus the runtime without debug sections (an estimate of 8 to 12 MB of the 28.4 MB archive, whose code is about 4.9 MB), so about 50 to 70 MB. Each release records the size of `una` per target in its validation summary, and a growth of more than 10% over the previous release is listed there with its reason.
    - **Start-up.** `una --version` must not take more than 2 ms longer than in the release before step 1, and a solver child must reach its handshake within 5 ms, both as medians of 50 runs on the gate machine. The child's figure includes compiling its macOS sandbox profile. Today each query already starts `z3`, a 27 MB C++ executable; the solver child does the same work, plus `una`'s own relocations at start-up, which on Linux are packed (`-z pack-relative-relocs`) where the build image's linker and glibc support it.
    - **If the child's start-up misses its budget,** a later record may add a fork server: one confined child started per run, which forks a fresh process per query, keeping one process per query.
31. **The release gates.** Every existing gate keeps running, and these are added (`docs/release-gates.md` §2):
    - **`solver-identity`,** on each target, over every query of the repository's obligations (the conformance suite, the examples, the hunt's fixed run), each query run twice in each comparison:
      - **front-end identity:** the solver child against a standalone `z3` that the project builds from the same library, recipe and flags; the outputs must be equal byte for byte, and any difference fails the gate;
      - **upstream comparison:** the solver child against the standalone release that the Z3 project publishes for the pinned version, compared by status; a difference is listed in the validation summary and reviewed, and does not by itself fail the gate;
      - it also records the largest memory a query used.
    - **`link-oracle`:** rule 33.
    - **`toolchain-free`,** on each target, the release `una` alone, with no compiler, linker, Rust or Z3, in two phases:
      - **build phase:** `una check` proves a package, `una test` runs its tests, and `una build` builds `examples/hello` and a program that makes an `Http` request over TLS to a local test server. On Linux, in a container of a minimal image with none of those tools, where Landlock grants execution only to `una`'s file and to the glibc loader it needs; on macOS, under `sandbox-exec` with a profile that denies `process-exec` for every file but the `una` under test, with a `PATH` that holds only an empty directory, the solver child taking the outer sandbox as its confinement (Question 6);
      - **run phase:** each executable built runs, under a second restriction that grants execution to exactly those files (on Linux, with no loader, since they are static), and gives the interpreter's output;
      - any attempt to start another program fails the gate. The `sanitize` gate's toolchain and the external link of rule 19 are developer-only and outside this gate.
    - **`macos-oldest`:** the executables of the `native` gate's corpus, built on the gate machine, run on a macOS 13 virtual machine with the interpreter's outputs (Question 5).
    - **Native on each target.** The `limits` gate and the confinement cases of the Conformance run natively on each target, on an arm64 Mac, in a native `linux/arm64` container and on a native x86-64 Linux machine, never under emulation, before each release.
    - **`limits`:** a query that allocates without bound ends as `memout`, one that loops ends as `wallclock`, one killed by `SIGXCPU` as `cpu`, a child killed by `SIGSEGV` as `crash`, and a child that opens a file as U5003, each with the parent intact; and the canaries of rule 34.
    - **`size-startup`:** rule 30's measures, written into the summary.
    - **Z3's library:** the `release` and `reproducible` gates check that the library linked has the digest of `supply-chain/z3.lock`, and rebuild it only when the pin or the recipe changed (rule 1).
    - **Archive checks:** `scripts/check-release.sh` expects exactly one entry, `una`, and refuses a development build and a build with `dev-external-link` (rule 19).
32. **One file to install.**
    - **The archive.** `una-<version>-<target>.tar.gz` holds one entry, `una`, with mode 755, packed as today (#679). Its digest is beside it and in the validation summary.
    - **The installer** checks the archive against the summary as today, and installs `UNA_HOME/bin/una` and, under ADR 0122, the link `UNA_HOME/bin/ita` to it. It downloads nothing else and needs only `curl` or `wget`, `tar` and a SHA-256 tool; `unzip` is no longer needed.
    - **Uninstall** removes exactly what the record lists, `bin/una` and `bin/ita`, and the directories it created when they are empty. It never touches Itaoca (ADR 0122) or the proof-cache key (`~/.una/proof-cache.key` or the file `UNA_PROOF_KEY` names, ADR 0064), and prints the path of each that exists, for the user to remove if they wish.
    - **Path checks.** Every path of a record is refused when it is absolute, contains `..`, or has a component that is a symbolic link, before anything is removed. This tightens the checks of today's `install.sh`.
    - **Migration.** Run over an earlier installation, the installer reads the earlier record. After the new `una` is in place, it removes the files of that record that the new installation does not hold (`bin/z3`, `z3/bin/z3`, `lib/libuna_rt.a`, `README.md`, `LICENSE`, `docs/guide/*.md`), with the same path checks, then the directories of that record that are left empty, and writes the new record. It prints that `z3` was removed and is no longer needed by `una`, for users who used it otherwise. A `z3` installed outside `UNA_HOME` is not touched, and no longer used. An existing `verification.lock` that pins the version built in keeps working; its proof cache is verified once again (rule 10).
    - **Downgrades.** An older installer does not know `bin/ita`, so the documented way to downgrade is `--uninstall`, then the older installer. A `una` invoked as `ita` that has no Itá is a usage error that says so.
    - **Upgrades** are a new run of the installer, as today.

### E. Testing the linker

33. **The oracle and fuzzing.** On the gate machines, the system linkers serve as oracles: `ld.lld` and GNU `ld` on Linux, Apple's `ld` on macOS. The outputs are not expected to be byte-identical; their meaning is:
    - **Per relocation type:** for each supported type, a test object in the repository, written in assembly, is linked by both. At each place a relocation patched, the instruction or datum is decoded, and the address it reaches is mapped back to a symbol and an offset. Both outputs must reach the same symbol and offset at every place.
    - **Must-reject cases,** refused by both (or by `una-link` where the system linker is laxer, which the case notes): `R_X86_64_32` against a local symbol in `.text`; `R_AARCH64_ABS32` and `MOVW_UABS_G0` against a defined symbol; `R_X86_64_64` in `.rodata`; on Mach-O, an `UNSIGNED` pointer in `__TEXT,__const` and one at an unaligned place.
    - **Per program:** every runnable program of the repository, linked by both, runs with the same output, failure and trace, and the `native` gate's equivalence with the interpreter holds for the internal linker's output (`spec/16 §5`).
    - **Generated programs:** the programs of the hunt's generators, beyond the repository's own, linked by both and run, compared the same way.
    - **Structure:** every output is read back by `una-link`'s verifier (rule 34) and by `llvm-readobj`; on macOS `codesign --verify --strict` accepts it.
    - **Fuzzing,** in the `sanitize` gate: the readers of `ar`, ELF and Mach-O, and the whole link, with mutated objects and archives as inputs and the verifier of rule 34 as the oracle: an input either fails with a diagnostic or yields an executable the verifier accepts.
34. **The verifier.** After writing, `una-link` reads the executable back with what it knows of every input, and refuses it (U8001) unless:
    - **hardening:** no segment is both writable and executable; the stack is not executable; RELRO covers the GOT and relocated constants on ELF, and `__DATA_CONST` is read-only after fixups on Mach-O; no text relocation exists; every relocation was applied, inside its own section (rule 24); every kept input section is placed once, with no overlap; on macOS the signature's hashes match the file;

    Canaries, each of which must make the verifier or the gate fail: an object with a writable and executable section; an `R_X86_64_IRELATIVE` relocation; an undefined symbol outside libSystem's list; a signature with one wrong page hash; an executable whose `PT_GNU_STACK` is executable; a relocation whose place lies 8 bytes past the end of its section, into the following code; a symbol whose value lies past its section's end; and two input sections placed overlapping.

### F. The trusted base

35. **The `unsafe` code.** Three places, each under ADR 0058's rules:
    - `ffi.rs`, which gains the RELRO protection at start-up (rule 25, about 40 lines);
    - `una-sys`, which gains, behind a Cargo feature `solver` that only `una-cli` enables, the solver child's confinement and limits: the seccomp filter and its installation (about 250 lines), `sandbox_init` and `sandbox_check` (about 80), the rlimits (about 60), the spawn attributes and descriptor closing (about 60), the footprint reading (about 40), about 500 lines in all. The crate's limit becomes 800 lines of base code, plus 600 behind `solver`. A test runs `cargo tree -p una-rt -e features` and `cargo tree -p una-interp -e features`, failing if `solver` is active;
    - `una-z3` (rule 2).

    `una-link` and the rest of `una-cli` stay free of `unsafe`.

### G. Before acceptance

36. **Probes.** These are run, read-only for the repository, and their results recorded on the issue, before this record is accepted:
    - **P1:** a seccomp filter installed by an x86-64 process under Rosetta, in the `linux` gate's emulated container, and under qemu-user: whether it installs, which architecture it sees, and whether the solver child's tests pass. If it cannot work there, the solver's tests in the emulated `linux` gate are marked as such, and the confinement tests run natively on an x86-64 Linux machine before each release (rule 31);
    - **P2:** whether the musl of Rust 1.98.1's self-contained targets protects `PT_GNU_RELRO` in a static-pie executable;
    - **P3, done (2026-10-18, macOS 26.6.2, arm64):** macOS enforces the hardened runtime on a signature that is both `linker-signed` and `runtime`: `DYLD_INSERT_LIBRARIES` was refused, and `codesign -v` accepted the file. The probe's signature had a requirements slot; the conformance test covers rule 27's exact layout, without one;
    - **P4, done (2026-10-18, macOS 26.6.2, arm64, Z3 5.1.0):** exceeding `memory_max_size` ends with exit status 101, `(error "out of memory")` on stderr and nothing on stdout; an error after `sat` ends with exit status 1, with `sat` and then the `(error ...)` line on stdout. Rule 8 pins both;
    - **P5, done (2026-10-18, macOS 26.6.2, arm64):** under the `hunt` gate's profile, a nested `sandbox_init` fails with `EPERM`, which leads to the check of rule 7 as designed, and `sandbox_check` reports outbound and inbound network access denied and writes at `/` and outside the temporary directory denied, with reads, `process-exec*` and `process-fork` allowed: the profile meets Question 6's minimum. `sandbox_check` refuses the operation name `process-exec` (`EINVAL`); rule 7 uses `process-exec*` with a path filter;
    - **P6:** the solver's queries of the conformance suite, under the filter in log mode (`SECCOMP_RET_LOG` in place of every `ERRNO` and kill action), on the oldest glibc the project supports for `una`, on the gate image's glibc, and on a current one: every call made outside the allowed list, and in particular every file opened, which decides between opening those files before the filter and `EACCES` (rule 7);
    - **P7, partial (2026-10-18, macOS 26.6.2, arm64):** `(version 1)(deny default)` is accepted by `sandbox_init` in-process; allocation, threads, clocks and reading stdin work under it; opening a file, `fork`, `connect`, `bind` and `sysctlbyname("hw.ncpu")` are denied (hence rule 7's one `sysctl-read` allowance). As an approximation of the front end, 110 of 110 solver queries gave identical answers under a deny-default profile. Still open: macOS 13, and the real front end inside `una`.

    P1, P2 and P6 are pending. The record was accepted on 2026-10-27 with these three still pending: they are run and recorded on issue #891 before the implementation steps they condition, P1 and P6 before the solver's confinement ships on Linux (rules 7 and 31) and P2 before the static executables of Question 2 (a) ship (rule 25).

## Security

- **Closing two hijacks.** Today a directory earlier on `PATH` than the real `z3` can supply one that answers `unsat` to every query, which forges every proof, and one earlier than `cc` can add code to every executable `una build` writes. Anyone who can write `UNA_HOME/lib/libuna_rt.a` or set `UNA_RT_LIB` can do the same to the runtime. After this record, none of them is looked up: changing the solver, the runtime or the linker means changing `una` itself (rules 11, 15, 19).
- **The proof condition.** Today a `z3` that prints `unsat` and then exits with an error counts as a proof. A proof now needs the handshake, a normal exit with status 0 and `unsat` (rule 8).
- **The supply chain of Z3.** Z3's source is pinned by commit, tree and tarball digest, mirrored by the project, and built by a committed recipe, reproducibly, with the toolchain recorded (rule 1). This is stronger than today, where the installer trusts a binary built by the Z3 project on its own machines, checked only against the digest the same project publishes. Z3 has no advisory feed of its own; a change of the pin is reviewed by its source diff, and the `solver-identity` gate runs on every release. Z3 is MIT-licensed, and no LGPL code is linked.
- **The solver as an attack surface.** Queries come from the package's source, which, with a registry (ADR 0122), may be someone else's. A bug in Z3's parser or engine reached by a hostile query now lands in a child that holds no file descriptor but its pipes, has no environment and no new privileges, cannot open a file or a socket, start a program or read another process, writes no core dump with the query's constants, and is bounded in time and memory, with the number of children bounded too (rules 5 to 7). A forbidden call kills it and is reported as a security event (U5003). What it can still do is answer wrongly, which is today's trust in the solver: `unsat` is trusted without a certificate (`docs/security/tcb.md` §3.2, #66). A crash or a limit is `unknown`, an error, never a proof (rule 8).
- **Under an outer sandbox (macOS),** the child's confinement is the outer one's. `una` checks that it exists, checks each operation of a fixed list against it, refuses one that allows network access or writes outside the temporary directory (Question 6), and records what it denies in the report.
- **The linker as a new part of the trusted base.** A bug in `una-link` can produce a wrong executable: a wrong address is a wrong result or a crash, and a wrong segment permission weakens hardening. It replaces a component that was unpinned and unrecorded (`tcb.md` gap 7). Mitigations:
  - safe Rust only;
  - relocation types derived from a scan of the real inputs, with position independence and bounds enforced on each (rule 24);
  - the verifier that reads every output back (rule 34);
  - the oracle tests per relocation type, per program and on generated programs, and fuzzing of the whole link (rule 33);
  - the equivalence of every runnable program with the interpreter, which already catches a wrong result (`spec/16 §5`).
- **No foreign code at the link.** Una admits no foreign code: every input of the link is generated by `una` or built by the project and carried inside `una` (rule 22), and no package or user can supply an object, an archive, a library or a flag. A bug in an input, such as a relocation aimed past its own section, which could patch the following code, or a symbol aimed past its section, which could alias it, is still refused by rule 24 and checked by rule 34.
- **Hardening of the executables,** at least that of today's default `cc`: position-independent with address randomization on both targets; RELRO protected by the runtime at start-up and tested at run time (rule 25); every import bound at load; non-executable stacks; no segment both writable and executable; on macOS, the hardened runtime (Question 7).
- **Code signing.** An ad-hoc signature proves integrity, not identity. An executable built on a machine runs there, since a locally written file carries no quarantine attribute. One sent to another Mac and downloaded there is subject to Gatekeeper, as an executable linked by Apple's `ld` with an ad-hoc signature is today; its author signs it with their own identity. `--debuggable` adds an entitlement that lets any debugger of the same user attach; the report records it, and `--release` refuses it. Signing `una` itself with a developer identity and notarizing it is a separate decision of the release process.
- **Licenses.** An executable now contains musl (MIT) and LLVM's libunwind (Apache-2.0 with LLVM exception) on Linux, as well as the Rust standard library (MIT or Apache-2.0) and the runtime's crates as today. `una notices` prints their notices, and `spec/16` lists what an executable contains. musl's and libunwind's advisories are followed through the Rust toolchain's bumps, in the supply-chain record.

## Verification

- **Proof results do not change, except to fail closed.** The query text, its preamble and its options are unchanged (`spec/12 §4`, `§5`). The solver is the same version of Z3, running the same front end that `z3 -in -smt2` runs; the `solver-identity` gate checks byte equality against a project-built standalone `z3` and compares statuses against the upstream release (rule 31). The only change of outcome is stricter: a `unsat` from a child that did not exit normally is no longer a proof (rule 8).
- **Determinism** (`spec/12 §10`): for identical sources, `una.toml` and `verification.lock`, tiers 0 and 1 produce identical diagnostics, statuses and reports, except obligations marked `unknown` with reason `wallclock`, `cpu`, `memout` or `crash`, which are marked non-deterministic in the report and never cached. How many children run at once never changes a query's limits (rule 6).
- **The pin** (`spec/12 §9`): `solver_version` is compared with the version built in, with no process started (rule 10); the handshake checks that the child is the same build (rule 4); the cache key binds answers to the solver's build.
- **Nothing changes in the formal model** (`formal/`), which does not model the solver, the runtime's linking or the installer.

## Information flow

No label, rule or theorem of `spec/07` changes: this record moves where tools run, not what a program may observe.
- **Queries hold program constants,** some of which may be labeled above `public`. As today, they go to the solver over a pipe and nowhere else. The child cannot open or write a file and writes no core dump (rules 6, 7), so they do not reach the disk, which today's `z3`, unconfined, could.
- **Executables carry no data of the machine that built them:** no path, user name or time (rules 26, 29). The signature's identifier is the output's file name, which the user chose.
- **The crash reason** that the parent keeps is at most 4 KiB of the child's stderr, shown only in the diagnostic of that obligation, on the machine that ran it.

## Costs

| Operation | Time | Memory |
| --- | --- | --- |
| A solver query | as today: one process start, which reaches its handshake within 5 ms (rule 30), then Z3's own work, unchanged | Z3's own, bounded by `--solver-memory` |
| Admission of a child | O(1): a counter against the bound of rule 6 | O(1) |
| Comparing the pin | O(1), with no process started | O(1) |
| Finding a runtime member that defines a symbol | O(1) expected, through the precomputed index (rule 12) | the index, O(number of symbols), inside `una` |
| Reading inputs | O(total size of the loaded members); the embedded runtime is read in place | O(size of the loaded members) |
| Resolution | O(number of symbols) expected, with a hash table, over the members loaded | O(number of symbols) |
| Dead-code removal | O(sections + relocations), one traversal from the roots | O(sections) |
| Layout | O(sections), plus O(S log S) for the sorted orders of rule 29, with S sections | O(sections) |
| Applying relocations, with their bounds and position checks | O(relocations) | O(1) each |
| `.eh_frame_hdr` and `__unwind_info` | O(F log F), sorting F functions | O(F) |
| The signature | O(size of the executable), SHA-256 of each 4 KiB page, in parallel over pages, each written at its fixed offset | O(pages × 32 bytes) |
| The verifier | O(size of the executable + relocations) | O(sections) |
| RELRO at start-up | O(number of program headers), one `mprotect` | O(1) |

The whole link is O(N + S log S) for N bytes of loaded input, the bound of the fastest production linkers (`mold`, `ld.lld`), and needs no external process. Today's link spends a process start and the reading of the whole `libuna_rt.a` from disk.

**Scaling tests.** Synthetic programs of 1,000, 2,000, 4,000 and 8,000 functions are linked, with medians of 9 runs: each doubling may at most multiply the time by 2.3 (linear, with the log factor and noise). The same test runs on the number of relocations per function. And, for the solver, the time per query from the parent's start to the answer, on the conformance suite, may not exceed today's by more than 10% in the median (rule 30).

**Project costs.** A cold build of Z3 takes on the order of 10 to 20 minutes per target, more under emulation; it happens once per pin or recipe change, not in every gate run (rule 1).

## Diagnostics

One new code, which joins `spec/13 §4` with its implementation. U7007 to U7014 are taken by ADRs 0121, 0122 and 0123 (Proposed); U7015 is free on `main`, in every open pull request and in every local draft.

| Code | Domain | Meaning |
| --- | --- | --- |
| U7015 | project | The executable cannot be laid out: a branch beyond its target's range, or a section larger than its format allows |

Existing codes cover the rest:
- **U5003:** a lock that pins another solver or version (rule 10); a child whose handshake differs, is too long or late, or is missing (rule 4); no `/proc` on Linux (rule 5); a child that cannot be confined, whose outer sandbox is too permissive, or that broke its confinement (rule 7); a development build with no solver (rule 19).
- **U5002:** `unknown` with reason `memout`, `cpu` or `crash` (rules 6, 8).
- **U8001:** a link that fails on Una's own inputs: an unsupported or position-dependent relocation, a relocation or symbol outside its section, a duplicate or undefined symbol, a symbol outside libSystem's list, a refusal of the verifier (rules 23, 24, 34); a failure to protect RELRO at start-up (rule 25).

Usage errors, exit code 2: `--solver-memory` takes an integer from 256 to 1,048,576; `una guide` with a name that is not a guide; `--debuggable` with `--release`; `--external-linker` in any build without `dev-external-link`. A failure to write the executable's file stays a usage error, as today (`spec/16 §2` rule 3).

## Consequences

- **One file.** Installing, upgrading and uninstalling Una is one file, plus the `ita` link of ADR 0122. On Linux, `una` needs glibc from the gate image's version on.
- **No external tool** is run by `una`, except `git` for git sources under ADR 0122 rule 61.1, and, until ADR 0106's amendment, the C compiler of release builds.
- **Executables** on Linux run on any distribution of their architecture, with no shared library (Question 2), and are the same bytes on every machine.
- **`una` is larger** (rule 30), and every command carries Z3 and the runtime on disk; pages that a command does not touch are not read into memory.
- **One `una`, one Z3.** Changing the solver version means changing `una`, and a package that pins another version needs the `una` that has it.
- **Proof caches are verified once again** on each machine after the update of step 1 (rule 10).
- **The project builds more.** Z3 is built from source once per pin, which requires a C++ toolchain on the gate machines; contributors build it once per pin (`scripts/build-z3.sh`).
- **The linker is the project's to maintain:** the scan of rule 24 runs at every toolchain bump, and a new relocation type is added to `spec/16` with its oracle test before the bump merges.
- **Debugging on macOS** needs `--debuggable` (Question 7).
- **`una.build/1`'s `target`** on Linux becomes the musl triple (rule 29).

## Alternatives considered

- **A. Keep Z3, `cc` and the runtime separate.** Not chosen: it is the state that #891 replaces, with the two hijacks of the Security section.
- **B. Z3 in the `una` process, through its C API.** Not chosen: a crash of Z3 ends `una`, including the language server; Z3's global state and memory stay in the process across queries; memory and processor limits cannot be applied to one query; and confinement would confine `una` itself.
- **C. One long-lived solver child, reset between queries.** Not chosen: state that `(reset)` does not clear could make one query's answer depend on an earlier one, against `spec/12 §10`. A fork server (rule 30) keeps one process per query if start-up ever needs it.
- **D. Embed `ld.lld` or `mold`.** Not chosen: each is a large C++ program that would parse every input with no memory safety, inside the trusted base, and neither is built as a library.
- **E. A Rust linker crate.** Not chosen for now: at the time of writing, none covers both ELF and Mach-O with a code signature. If one comes to, it goes through the supply-chain record and the `deps` gate, and rules 33 and 34 test it the same way.
- **F. Write the runtime to a cache directory and keep `cc`.** ADR 0059's alternative. Not chosen: it keeps `cc`.
- **G. Dynamic glibc executables** (Question 2 (b)). Not chosen: the linker would need symbol versions, dynamic sections and an embedded list of glibc's symbols, and executables would depend on the glibc of the machine that runs them.
- **H. A Zig-based toolchain (`zig cc`)** as a bundled C compiler and linker. Not chosen: a second language toolchain, larger than this record's linker, inside the trusted base.
- **I. Confining the solver with Landlock only.** Not chosen: Landlock limits files, not system calls, and needs a kernel from 5.13 on. seccomp filters exist in every Linux kernel since 3.5, and forbid opening any file at all.
- **J. A seccomp filter that kills on every unlisted call.** Not chosen (Question 8): `una` uses the machine's glibc, so a call that a newer glibc makes, and the filter's tests never saw, would end every query as a failure. `ENOSYS` is as confining, since the call never runs.
- **K. Fail closed under an outer macOS sandbox.** Not chosen (Question 6): `una check` would fail under every harness that uses Seatbelt, the project's `hunt` gate included, and the macOS gates would need redesigning.
- **L. Linking `una` itself statically on Linux,** which would pin its system calls and let it run on musl distributions. Not chosen for now: Z3 is C++, and a static musl build of it and of `libstdc++` needs a musl C++ toolchain on the gate machines; with Question 8's `ENOSYS` default, the dynamic glibc is safe. It remains open.
- **M. Release builds' compiler decided here.** Not chosen: the size and start-up cost of LLVM inside `una` should be measured first (Question 1).

## Implementation plan

Each step ships in a release, keeps every existing gate (archive scan, validation records, determinism, reproducibility), and adds the `toolchain-free` test for what it removes. The probes P1 to P7 of rule 36 run before acceptance.

1. **Z3 inside `una`.** `supply-chain/z3.lock` and `supply-chain/z3/`, the mirrored tarball, `scripts/build-z3.sh`, the library stored by digest, the crate `una-z3`, the solver mode with its handshake, limits, admission control and confinement in `una-sys` behind `solver`, `--solver-memory`, the proof condition and the reasons `memout`, `cpu` and `crash`, the cache key with the solver's build, the build record, the report's `tcb.solver_build` and `tcb.solver_confinement`, and the gates `solver-identity` and `limits`. The installer stops downloading Z3. `toolchain-free` runs `una check` with no `z3` on `PATH`.
2. **The runtime inside `una`.** The build order of rule 18, the embedded runtime and its index, `UNA_RT_LIB` for development builds only. Until step 3, `una build` writes the embedded runtime into a private directory of mode 0700, created for the build, with `O_EXCL`, for `cc`, and removes it after the link. The archive drops `lib/`.
3. **The internal linker, macOS arm64.** `una-link` with Mach-O output, chained fixups, unwind information, the signature with the hardened runtime and `--debuggable`, the relocation scan, the verifier, the oracle and fuzzing, the external link for the `sanitize` gate, the `macos-oldest` gate, and U7015. `toolchain-free` runs `una build` on macOS.
4. **The internal linker, Linux x86-64 and arm64.** Static-pie ELF with 64 KiB alignment on arm64, the musl inputs, the runtime and Cranelift retargeted to musl, RELRO at start-up with its run-time test, and the allocator of rule 21 with its measurement on the release performance benchmarks. `toolchain-free` runs `una build` on Linux. This step follows the project's decision on Question 2.
5. **One file.** The archive's single entry, `una --license`, `una notices`, `una guide`, the installer with its tightened path checks and its migration, and the `size-startup` gate.

The release compiler (Question 1) follows ADR 0106's amendment, on its own plan.

## Conformance

- **Solver:**
  - every obligation of the conformance suite and the examples gives the same output, byte for byte, as the project-built standalone `z3`, and the same status as the upstream release, or a reviewed difference;
  - a lock pinning `5.0.0` is U5003 at its `solver_version`, naming both versions, with no child started;
  - a child whose handshake names another build, one whose line exceeds 256 bytes, and one that writes none within 2 seconds, are U5003;
  - a child that prints `unsat` and exits with status 1, and one that prints `unsat` and is killed by `SIGKILL`, are not proven;
  - a child that exits with status 101, `(error "out of memory")` on stderr and nothing on stdout, gives `unknown`, `memout`; a child that exits with status 101 and another stderr is `crash`;
  - a query that allocates without bound is `unknown`, `memout`, at `--solver-memory 256`, never U8001, and `una` stays alive;
  - children killed by `SIGSEGV` and by `SIGKILL` give `unknown`, `crash`; one ended by `SIGXCPU` gives `cpu`; none is cached;
  - in a test build whose front end is replaced by one that tries each: opening `/etc/passwd`, creating a socket, starting a process, writing another process's memory, and an `mmap` with execution each kill the child on Linux, reported as U5003 "broke its confinement", and are denied on macOS; a call outside both lists (such as `uname`) fails with `ENOSYS` and the child goes on; a call with the x32 bit kills it;
  - `una check` under an outer `sandbox-exec` profile like the `hunt` gate's proves the package and reports `tcb.solver_confinement` `"outer"` with the operations it denies; under an outer profile that allows network access it is U5003; with no outer sandbox it reports `"own"`;
  - a refutation whose solver writes `sat`, then an `(error ...)` line, on stdout and exits with status 1, as in ADR 0101, stays refuted; `sat` with status 1 and no `(error ...)` line is `crash`;
  - inside a cgroup whose `memory.max` is 8 GiB, on a machine with more, at most 1 child of 4 GiB runs at once;
  - with a per-child limit of 4 GiB on a machine with 16 GiB, at most 2 children run at once, and every query's outcome equals that of a run with one child at a time;
  - `--solver-memory 255` and `--solver-memory x` are usage errors;
  - a file descriptor left open and inheritable in `una` is closed in the child;
  - `una check` with a fake `z3` first on `PATH` that answers `unsat` to everything still refutes a false obligation;
  - a proof cache entry written by a build with another `z3_library_sha256` is not used;
  - calling `una-z3`'s entry without the confinement flag aborts the process.
- **Runtime:**
  - `UNA_RT_LIB` naming a runtime that prints an extra line has no effect on a release `una`, and is used by a development build;
  - `una --version` of a development build ends with `development build`; a release's does not;
  - `--external-linker` is a usage error in a release `una`, and `scripts/check-release.sh` refuses a binary built with `dev-external-link`.
- **Linker:**
  - each supported relocation type, by the oracle of rule 33, and each must-reject case;
  - an object with an unsupported relocation is U8001 naming the type; an object with DWARF sections and their relocations links;
  - a duplicate strong symbol and an undefined symbol are U8001 naming the symbol and the inputs; an input that defines `__dso_handle` is U8001;
  - on macOS, a reference to a symbol outside libSystem's list is U8001, including a symbol that libSystem exports only after macOS 13;
  - a branch to a target beyond 128 MiB, in a synthetic object, is U7015;
  - `examples/hello` built twice, from two directories, by two users, gives the same bytes; built under two output names, it differs only in the signature's identifier and the hashes and UUID that follow from it;
  - an executable that does not use `Http` contains no symbol of `rustls`;
  - a panic inside the runtime is caught at its boundary, which needs the unwind tables, and gives U8001 as today;
  - on macOS, the signature has exactly rule 27's layout (one CodeDirectory, no requirements slot, an entitlements blob only with `--debuggable`); `codesign --verify --strict` accepts every output; an executable run with `DYLD_INSERT_LIBRARIES` naming a library does not load it; one built with `--debuggable` can be attached to by `lldb`, and one without cannot;
  - on Linux, every `PT_LOAD` of an arm64 executable has `p_align` 0x10000; `PT_GNU_STACK` is not executable; the executable loads at a different address on two runs; it runs in a container with no shared library; and, at run time, the executable reads `/proc/self/maps` and the page of its GOT is not writable;
  - the canaries of rule 34.
- **Installer:**
  - a fresh install creates `bin/una` and `bin/ita` and nothing else, with no `unzip` on `PATH`;
  - an install over 0.3.15 removes `bin/z3`, `z3/`, `lib/libuna_rt.a`, `README.md`, `LICENSE` and `docs/guide/`, prints the notice about `z3`, and keeps a file the user added under `UNA_HOME`;
  - `--uninstall` removes `bin/una` and `bin/ita`, and keeps `~/.una/itaoca` and `~/.una/proof-cache.key`, printing their paths;
  - a record naming a path through a symbolic-link directory is refused before anything is removed;
  - an archive with a second entry is refused by `scripts/check-release.sh`.
- **Toolchain-free,** on each target: the `toolchain-free` gate of rule 31, both phases.

## Appendix A. Amendments to records

To be appended, dated, when this record is accepted:

**ADR 0003, Amendment.** The subprocess is `una` itself, re-executed in its solver mode, running the Z3 built into it at the pinned version, confined and bounded (ADR 0124 rules 1 to 11). Queries stay SMT-LIB text, which can be saved, diffed and replayed. A proof requires a normal exit with status 0.

**ADR 0035, Amendment.** Rule 3's archive holds one entry, `una` (ADR 0124 rule 32).

**ADR 0055, Amendment.** NB-1: `una build` links with the internal linker of ADR 0124, not the system `cc`. On Linux the executables are static-pie and their target is the musl triple.

**ADR 0058, Amendment.** The amendment of ADRs 0116 and 0117 says that `una-sys` holds only the signal handlers and the coroutine stacks. It now holds: that base code (at most 800 lines); and behind the feature `solver`, which only `una-cli` enables, the solver child's confinement, limits, spawn attributes and footprint reading of ADR 0124 rules 5 to 7 (at most 600 lines). A third place may hold `unsafe` code: the crate `una-z3`, holding only the declaration of Z3's front-end entry, the construction of its arguments and the call, at most 150 lines, depending only on `una-sys`. `ffi.rs` also holds the RELRO protection at start-up (ADR 0124 rule 25). Each is under rule 1's comments and lint settings, and every change gets a security review.

**ADR 0059, Amendment.** Release archives no longer ship the runtime: it is built into `una` (ADR 0124 rules 12 to 19). Rule 2's search order holds for development builds only.

## Amendment (2026-10-27): the size of `una-sys`

Rule 35 and the ADR 0058 entry of Appendix A say that `una-sys`'s base code is at most 800 lines. That number predates ADR 0058's amendment of 2026-10-25, which defines how the crate's size is counted (physical lines of the Rust files under `crates/una-sys/src/`), keeps the cap of 1,200 lines until a later amendment sets it at the exact count of the merged slices, and replaces every earlier number. On `main` at this date the count is 1097. ADR 0058's amendment of 2026-10-25 therefore governs the base code's count and cap; this record's 800 is not a cap. The 600 lines behind `solver` and the 150 lines of `una-z3` stand as written, counted the same way.

## Amendment (2026-10-27): the solver files' size

Rule 35 states the base code of `una-sys` as 800 lines and the code behind `solver` as 600 lines,
estimates from its writing. ADR 0058's amendment of 2026-10-25 governs the count and the cap of
the base code (1,200 lines until its exact-count amendment); rule 35's 800 is not a cap. The
files behind the feature `solver` (`crates/una-sys/src/solver/`) are capped at their measured
physical count, comments and blank lines included, as that amendment counts, plus at most 5%:
1,166 lines at 3010a0867, so the cap is 1,224. `una-z3`'s 150 lines stand.

## Amendment (2026-10-27): the outer sandbox's minimum (rule 7, Question 6)

A limit on the file size stops writing data but not creating, removing or renaming files. The
outer sandbox counts as the confinement only if it also denies `file-write-create` and
`file-write-unlink` at `/` and at a path outside the temporary directory, beside the network
operations and `file-write-data` of Question 6; otherwise U5003, "the outer sandbox is too
permissive for the solver". The fixed list of rule 7 gains those four operations, in the order
the report keeps.

## Amendment (2026-10-27): the proof condition (rule 8)

Z3 5.1.0's front end answers the script's `(get-model)` and `(get-value ...)` after `unsat` with
`(error "line N column M: model is not available")` and exits with status 1, so "exited with
status 0" can never hold for a proof. A proof requires: the handshake matched; exactly one answer
to `(check-sat)`, `unsat`; and then either exit status 0, or exit status 1 where every line after
`unsat` on stdout is the pinned solver's exact "model is not available" error text (pinned per
solver build, compared byte for byte but for the line and column numbers) or the reply to the
script's `(get-info :reason-unknown)`, with nothing on stderr and no other output of any kind; no
other status is excused. Anything else is `unknown`, never cached. The alternative, a dialogue
with the child that sends `(get-model)` only after `sat`, was rejected: rule 6's whole script
and the solver-identity comparison stay as they are. The conformance test is the real
transcript and its near misses (crates/una-verify/src/solver.rs).

## Note (2026-10-31): read through ADR 0106's amendment (#1311)

ADR 0106's amendment of 2026-10-31 withdraws the C-emitting backend and closes Question 1, as its §5 states: no release compiler is built into `una`. No rule of this record changes. Its passages on the release compiler read through that amendment: the header's relation to ADR 0106, Question 1's history and its cost line, the Context's and the first Departure's reference to an amendment, rule 9 (which has no subject), rule 22's "the release compiler's object" (the release path's object), the Consequences' exception for "the C compiler of release builds" (none: no external tool is run for release builds either) and the Implementation plan's last line. `docs/plan/c-emission-inventory.md` lists each passage by line.

## Amendment (2026-11-01): the outer sandbox's probes (rule 7, Question 6, #1363, #1364)

Rule 7 asked an outer sandbox about `network-outbound` and `network-inbound` with no filter and
about three writes at `/` and at the parent's working directory, or at `/` again when that
directory was under the temporary directory. `sandbox_check` with no filter answers "denied"
for an operation that the profile allows only under a filter, so a profile that allowed
connections to `localhost` was accepted (#1363); and a profile that allowed writes under the
home directory, or everywhere but two paths, or every write but data, create and unlink, was
accepted too (#1364). The check of rule 7 becomes the following fixed list, in the order the
report keeps; the outer sandbox counts as the confinement only if it denies every required
probe, otherwise U5003, "the outer sandbox is too permissive for the solver: <probe> is
allowed".

- **The network, required.** `network-outbound`, `network-inbound` and `network-bind`, each
  asked with no filter; toward `127.0.0.1` and `::1` over TCP and over UDP, by
  `sandbox_check_network` on a socket of each kind at the ports 0, 1, 22, 53, 80, 443, 631,
  3000, 5000, 5432, 6379, 8000, 8080, 8443, 9000 and 65535 (a profile's address filter names
  every port or one); and toward local sockets, by `sandbox_check` with a path filter at each
  place below and at `/private/var/run`. They are named `<operation>`,
  `<operation> <tcp|udp> <address>` and `<operation> unix-socket`.
  `sandbox_check_network` is looked up at run time: where the system has none, the check fails
  closed with U5003.
- **Writes, required.** `file-write-data`, `file-write-create`, `file-write-unlink`,
  `file-write-xattr`, `file-write-mode`, `file-write-flags`, `file-write-times`,
  `file-write-owner`, `file-write-acl` and `file-write-finderinfo` (the last two caught a
  profile that denied only the first eight and left ACLs through `acl_set_file` and Finder info
  through `setattrlist` open, #1365), each at `/`, `/Users`, `/private/var`, `/private/tmp`,
  `/Volumes`, the home directory of the user's account (its passwd entry, not `HOME`), named
  `(home)`, and the user's temporary directory (`confstr(_CS_DARWIN_USER_TEMP_DIR)`, not
  `TMPDIR`), named `(temporary directory)`, each by its resolved path. All ten are covered by a
  profile's `file-write*`, so a harness that denies that denies them all. No place comes from
  the working directory or the environment, and the child's command line no longer carries a
  path: `una __solver <cpu-seconds> <memory-mib> [--no-address-space-backstop]`. `sandbox_check`
  answers "denied" for a path that does not exist whatever the profile, so a place that does
  not resolve is U5003. They are named `<operation> <place>`.
- **Recorded, not required.** `file-read-data` at `/` and at `(home)`, `file-revoke` at every
  place (the `revoke` call; a profile's `file-write*` does not cover it, so requiring it would
  reject a harness that denies only `file-write*`, and it is irrelevant to a child that opens
  nothing), `process-exec*` and `process-fork`.
- **An answer that is neither "denied" nor "allowed"**, such as the -1 the library gives for an
  operation it does not know, is U5003, "cannot ask the outer sandbox about <probe>", and never
  counts as a denial.
- **No descriptor after the check.** Once the outer sandbox is taken, the child sets its
  open-file limit to zero, soft and hard: no socket or file can be opened from then on, so a
  filter of the outer profile that the probes do not ask about (another port, another socket
  path) is never reached by the solver. Under the child's own profile, rule 6's limit of 16
  stands.
- **A denied probe is denied under the outer sandbox**, by the profile or by the system: the
  system denies some writes at some places whatever the profile (a create at `/`, on the sealed
  system volume, for example), so `tcb.solver_outer_denied` listing a probe does not, on its
  own, say the profile denied it. The report makes no such claim; `spec/13 §6` says the same.
- **Denial at the temporary directories is stricter than Question 6's wording, and accepted.**
  Question 6 speaks of "writing files outside the temporary directory", yet `/private/tmp` and
  the user's temporary directory are among the required places: the outer sandbox must deny
  writes there too. A solver child writes nothing (rule 7), so it needs no temporary directory;
  denying writes there as well removes a place where a compromised child could drop a file for
  another process, and the project's own sandboxes already deny it. The stricter reading is the
  one taken.
- **A probe needs the place's metadata and the home directory.** `sandbox_check` with a path
  filter reads the path's metadata, so a harness that denies `file-read-metadata` on one of the
  places, or an account with no home directory, makes a place fail to resolve and the run fail
  with U5003 ("cannot find <place> ..."). This is availability only: a run that would prove
  under the outer sandbox fails closed instead of proving; it never weakens the confinement.

**Accepted residuals.** Two things the probes cannot close are accepted, and the project's own
sandboxes (the `hunt` gate and `scripts/toolchain-free.sh`) deny them in their profiles so the
project never relies on the residual:

- **Signals to other processes.** An outer `(allow default)` profile lets the child send a
  signal to another process of the same user, and the open-file limit does not stop it. This is
  a denial-of-service nuisance, not a breach of the verification's confidentiality or integrity:
  the child holds nothing but its pipes, and a signal reveals nothing and changes no proof.
  `sandbox_check` cannot be made to answer for `(deny signal (target others))` (it answers
  "allowed" with no filter whether or not the profile denies it), and `kill` with signal 0 is
  not mediated, so there is no sound empirical probe that does not risk a real process; a real
  signal to a process guaranteed to be external and to survive is not available. The project
  adds `(deny signal (target others))` to the `hunt` and toolchain-free profiles; a harness
  that wants the residual closed does the same.
- **Writes that need no descriptor, outside the seven directories.** A profile that denies
  writes only at the seven named places (as literals) still lets `rename`, `unlink`,
  `truncate`, `symlink`, `link` and `mkdir` succeed at any other path, since none needs an open
  descriptor and the probes cannot enumerate every path (`sandbox_check` answers "denied" for a
  path that does not exist, so a probe at an invented path is meaningless, and a canary has no
  path that a compliant profile both lets it create and lets the residual be tested at). The
  project accepts this residual: a harness that confines the solver denies `file-write*` without
  a place filter, as the `hunt` and toolchain-free profiles do, which leaves no write of any
  kind outside the one writable directory.

- **Cost.** A fixed number of system calls per solver child, about 350, O(1) in the query.
- **Size.** The files behind the feature `solver` count 1,687 lines with this change (1,274
  before it, already above the cap of 1,224 that the amendment of 2026-10-27 set). The cap
  becomes this count plus 5%: 1,768, enforced by `crates/una-sys/tests/solver_size.rs` so it
  cannot drift; a later change that needs more raises the cap with a dated amendment stating the
  new count. ADR 0058's note of 2026-11-01 records the same.

## Amendment (2026-11-09): the handshake's window (rule 4)

- Status: Accepted (2026-11-10), by the project's decision of 2026-11-10 on issue #1791. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs. The implementation applies it since #1791.
- Issue: #1791.

**Why.** Rule 4 has the parent wait 2 seconds of wall clock for the child's handshake, counted inside the wall-clock guard, and makes a later one U5003. The window is a liveness bound: it ends a child that is not this `una` or does not start, and it is not part of the proof condition, which rule 8 states on its own. On a loaded machine (a full test run, a load average of tens) a child's start-up stretched past 2 seconds: five tests of the verifier failed together with "no handshake within 2 seconds" in one run, and a processor-time scaling test, whose measured check starts a solver child per query, had to run alone (#1776). Rule 30's target, a handshake within 5 ms as the median of 50 runs, shows how far 2 seconds already was from the normal case; the misses were the machine, not the child.

**The change.** In rule 4, "within 2 seconds counted inside the wall-clock guard" reads "within 10 seconds counted inside the wall-clock guard". The conformance item "one that writes none within 2 seconds" reads "within 10 seconds", and the message of a late handshake ends with "no handshake within 10 seconds".

**What does not change.**

- A handshake that is late, missing, too long or different is still U5003, "the solver child is not this `una`", an error and never a proof; the child is ended. Solver `unknown` stays an error.
- The 256 bytes, the protocol, the proof condition of rule 8 and the wall-clock guard (default 60 s, `--solver-wallclock`) are unchanged. The window stays inside the guard: the deadline is checked first, so with a guard shorter than 10 seconds a child that has written nothing by the guard ends as `unknown` with reason `wallclock`, as one did before with a guard shorter than 2 seconds; neither is a proof.
- Rule 30's start-up target (5 ms) is unchanged.

**Security.** The handshake detects an accidental replacement of `una` during a run; rule 4 already says it is no defense against a malicious one. A longer window lets a stalled child hold its admission seat (rule 6) for at most 8 more seconds before U5003, bounded by the guard as before; it gives a child no new input, output or authority.

**Cost.** O(1) per query: the same polling loop, with a larger bound. A run with no stalled child takes the same time.

**Note for `spec/12 §5` rule 6.** Its sentence "the parent reads at most 256 bytes for it within 2 seconds of the guard" reads "within 10 seconds of the guard" once this amendment is accepted; until then the spec carries a Proposed note at that rule.

## Amendment (2026-11-13): the unwinder library on Linux (rule 1, #1958)

Rule 1's "What is linked" says that on Linux `una` depends dynamically only on glibc. The Rust standard library of the `-gnu` targets links the unwinder of panics and backtraces (`_Unwind_*`) as a shared library, `libgcc_s.so.1`, which stays a NEEDED entry of `una` although `libgcc` itself is linked statically (`-static-libgcc`). The rule reads: on Linux, `una` depends dynamically on glibc (`libc.so.6`, `libm.so.6` for the solver's math imports, and the dynamic loader) and on `libgcc_s.so.1`, and on nothing else. This amendment concerns `una` itself; what an executable that `una build` writes may load is unchanged.

- **Where it is checked.** `scripts/load-commands.txt` lists what a released `una` and an executable that `una build` links may ask the dynamic loader for, and `scripts/load-commands-una.txt` what `una` alone may add (`libm.so.6` on Linux); `scripts/load-commands.sh` fails on anything else. Besides glibc's own libraries (`libc.so.6`, `libm.so.6`, `ld-linux-x86-64.so.2`, `ld-linux-aarch64.so.1`), `libgcc_s.so.1` is the only library those lists allow on Linux.
- **What does not change.** `una` still does not run on musl distributions, and needs glibc from the gate image's version on. No C++ runtime is a NEEDED entry (`libstdc++` is linked statically, #1956). No foreign code enters a Una program, and nothing is added to what `una` loads: `libgcc_s.so.1` was already a NEEDED entry of every Linux `una`, allowed by `scripts/load-commands.txt` and listed in `docs/foreign-code-inventory.md` (the unwinder, sections 2 to 4).
- **Cost.** None: no algorithm, build step or run-time work changes.

## Amendment (2026-11-26): glibc's own libraries before 2.34, and the build image (rule 1, #2105)

- Status: Accepted (2026-11-26), by the Owner's decision of 2026-11-26 on issue #2105, for v0.5.2. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs. The implementation applies it since #2105.

Rule 1's "What is linked" and the amendment of 2026-11-13 name glibc's own libraries as `libc.so.6`, `libm.so.6` and the dynamic loader, and rule 1's toolchain is "the pinned image of `scripts/local-ci/`". From #2105 `scripts/local-ci/` holds two images: the Ubuntu 24.04 image of the tests, the formal track and the hunt (`Dockerfile`), and the release image that builds the Z3 library, `una` and the Linux archive (`Dockerfile.release`, AlmaLinux 8, glibc 2.28). Rule 1's image is the release image, and "from the gate image's glibc on" reads from the release image's glibc on, so that `una` runs on glibc 2.28 and later. The tests stay on Ubuntu because the pinned Lean toolchain needs glibc 2.29 and the Z3 project's Linux executable 2.38. Before glibc 2.34, threads and `dlsym` are libraries of their own, so a `una` linked there also names `libpthread.so.0` and `libdl.so.2`, and so does an executable that `una build` links on a glibc before 2.34. Since glibc 2.34 both are empty stubs that the loader still provides.

- **The rule reads.** On Linux, `una` depends dynamically on glibc (`libc.so.6`, `libm.so.6`, `libpthread.so.0`, `libdl.so.2` and the dynamic loader) and on `libgcc_s.so.1`, and on nothing else.
- **Where it is checked.** `scripts/load-commands.txt` lists `libpthread.so.0` and `libdl.so.2` as glibc's; `scripts/load-commands.sh` still fails on any other entry. `scripts/check-release.sh` refuses a packed `una`, or an executable it builds, that asks for a glibc symbol version above the floor of `install.sh` (2.28).
- **Where the image is checked.** `scripts/check-toolchain.sh` compares the release image's packages (`rpm -q`) and the files its linker puts into `una` with `supply-chain/z3.lock`, and `scripts/local-ci.sh --linux-arm64` packs and checks the archive in the release image after the tests.
- **What the tests link.** The Linux test suite, the timing oracles, the formal track and the hunt run in the Ubuntu image and link a Z3 library built there from the same pinned source and recipe, whose digest the lock does not record. The shipped library is exercised by the archive's check in the release image (the examples under the solver, `una build`, the C math library, FMA, load-command and glibc-floor checks), by the `reproducible` and `toolchain-free` gates, and by the limits leg (`limits-linux-arm64`), which runs the solver limits and the solver child's confinement cases in the release image after checking that the library it links has the lock's digest.
- **What does not change.** No library outside glibc and `libgcc_s.so.1` is loaded; `libstdc++` and `libgcc` stay linked statically; no foreign code enters a Una program.
- **Cost.** None: no algorithm, build step or run-time work changes.

## Amendment (2026-11-27, Accepted): a hosted runner as the native x86-64 Linux machine (rule 31, #2106)

- Status: Accepted by the Owner on 2026-11-27. The accepted text of this record above this amendment is unchanged; where the two differ, this amendment governs once it is accepted. It follows the Owner's decision of 2026-11-19 on issue #2104 (GitHub-hosted runners are accepted as native evidence for the targets the project's machine cannot run natively).
- Issue: #2106.

**Why.** Rule 31 requires the `limits` gate and the confinement cases of the Conformance "natively on each target, on an arm64 Mac, in a native `linux/arm64` container and on a native x86-64 Linux machine, never under emulation, before each release". The gate machine is an arm64 Mac: its `linux/amd64` containers run under Rosetta, where no seccomp filter installs (probe P1, #1442: EINVAL under Rosetta, ENOSYS under qemu-user), so neither the confinement cases nor any test that starts a solver child can run there, and `x86_64-unknown-linux-gnu` left the shipped targets after v0.3.17. The project has no x86-64 Linux machine of its own.

**The change.** In rule 31's "Native on each target", "on a native x86-64 Linux machine" reads "on a native x86-64 Linux machine, which may be a GitHub-hosted runner as below". A GitHub-hosted runner counts as the native x86-64 Linux machine for that target's evidence when all of these hold, each checked and recorded by the run itself:

1. **The machine.** The runner is GitHub's hosted image named `ubuntu-24.04` (its `ImageOS` and `ImageVersion` recorded), on x86-64 cores: `uname -m` is `x86_64`, `scripts/emulation.sh` finds the shell native (the kernel's executable of the shell is an x86-64 ELF), and the Docker engine's architecture is `x86_64`, so the `linux/amd64` containers run natively. A run where any of these fails is FAIL, never evidence. The runner is a hardware-virtualized machine, as Docker Desktop's virtual machine is on the gate machine; it is not emulated.
2. **What runs there.** The workflow `.github/workflows/linux-x86-64.yml`, at the gated commit itself (the run's head commit, checked out exactly), runs `scripts/hosted-x86-64.sh` for each leg: `linux` (`scripts/local-ci.sh --linux`, natively: the tests, the timing oracles, the archive packed and checked in the release image), `archive` (the release archive built as `scripts/local-release.sh` builds it, checked by `scripts/check-release.sh`, then run through `scripts/toolchain-free.sh --linux`), `limits-linux` (`scripts/local-ci.sh --linux --limits`: the solver limits and the solver child's confinement cases, with the seccomp filter installed, against the library of `supply-chain/z3.lock`) and `z3-library` (the library built twice from cold in the release image, which must give one digest). Nothing is skipped or marked not runnable there; the counts of `scripts/limits.sh` hold as on any native target. Solver `unknown` stays an error.
3. **What is kept.** Each leg uploads an artifact `linux-x86-64-<leg>-<commit>`: its whole log, a manifest (the leg, the commit, the result, the runner image and version, the kernel `uname -srvm`, the processor's model and count, the emulation verdict, the Docker engine's architecture, the run's URL and attempt, the log's SHA-256, and the archive's or the library's SHA-256), and the SHA-256 of each file. No secret is used and the token is read-only; every action is pinned by its full commit hash.
4. **How it joins the release record.** On the gate machine, `scripts/release-gates.sh` takes the gates `linux` and `limits-linux`, and the `x86_64-unknown-linux-gnu` leg of `release` and `toolchain-free`, from that evidence (`scripts/hosted-evidence.sh`) when its own Docker engine is not x86-64: it finds the successful run of the workflow dispatched (`workflow_dispatch`, whose workflow file is the gated commit's own) whose head commit is the gated SHA, downloads the artifact of the leg, and passes the gate only when every digest matches its manifest, the manifest names the gated SHA, the result is PASS, the machine checks of item 1 hold, and, for the archive, the archive that the gate machine builds in the release image has the digest of the one the runner checked natively. The logs, manifests and digests are copied into the run's `OUT/hosted/`, each gate's log holds the runner's whole log and names the run's URL, and the validation record (`record.md`, section "Hosted x86-64 evidence") lists, per gate, the run, its runner image and kernel and the archive's digest; the public summary lists those gates as it lists every other. Anything missing, another commit, another digest or a failed check is FAIL; there is no fallback to the emulated leg. Where the gate machine's Docker engine is itself x86-64, the legs run there as before.

**Trust.** The hosted runner's evidence rests on GitHub's infrastructure (its image, its hypervisor, its artifact store) for the legs it runs, as the gate machine's rests on Docker Desktop's virtual machine and the Mac's own system. That is a party outside the project, so the release record names it, per leg. Two checks keep it from being the only party: the archive shipped is the one the gate machine builds, whose bytes must equal the runner's natively checked archive, and the library's digest in `supply-chain/z3.lock` is shown reproducible by independent cold builds (two on separate runners, and one in the release image on the gate machine under emulation, where building needs no seccomp). What the runner proves is tested evidence on real x86-64 cores, not a proof.

**What does not change.** The seccomp confinement is installed and tested on x86-64, never skipped; the emulated `linux/amd64` leg on the gate machine still cannot pass (`scripts/limits.sh` refuses it); every other gate of each target and of the gate machine is unchanged; GitHub Actions validates nothing else (`docs/release-gates.md`).

**Cost.** No algorithm, run-time work or binary changes. The hosted legs take hours of a runner per gated commit, outside the gate machine; the gate machine adds the archive's build under emulation and an O(n) digest check of the artifacts.
