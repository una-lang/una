# ADR 0124 probe P1: seccomp under Rosetta and qemu-user (results)

- Issue: #1203 (part of #891; ADR 0124 rules 7, 31 and 36)
- Measured: 2026-11-02, read-only for the repository. The probe program, its build and run scripts and every build product stayed outside the tree; this note records the results only.
- Question: does a seccomp filter installed by an x86-64 process work in the `linux` gate's emulated container (Rosetta for Linux) and under qemu-user? Which architecture does the filter see, does the x32 check of rule 7 behave, do `SECCOMP_RET_ERRNO(ENOSYS)` and `SECCOMP_RET_KILL_PROCESS` act as on a native kernel, and can the solver child's confinement cases pass there?

## Answer

No. Neither emulator lets an x86-64 process install a seccomp filter, and a filter that a native process installs sees the emulator's own aarch64 calls, never x86-64 ones.

| Item | arm64, native container | x86-64 under Rosetta (`linux` gate) | x86-64 under qemu-user 8.2.2 and 10.0.13 |
|---|---|---|---|
| 1. `prctl(PR_SET_NO_NEW_PRIVS)` | succeeds | succeeds (`NoNewPrivs: 1`) | succeeds (`NoNewPrivs: 1`) |
| 1. `seccomp(SECCOMP_SET_MODE_FILTER)` | succeeds, with and without `SECCOMP_FILTER_FLAG_TSYNC`; without no-new-privileges a non-root process gets `EACCES`, as the kernel documents | **fails with `EINVAL`** for every filter, even a one-instruction "allow"; also `EINVAL` without no-new-privileges, where the kernel would answer `EACCES` | **fails with `ENOSYS`** for every filter |
| 1. Other entry points | `SECCOMP_GET_ACTION_AVAIL` and `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER)` succeed | both `EINVAL` | `SECCOMP_GET_ACTION_AVAIL` `ENOSYS`; `prctl(PR_SET_SECCOMP)` `EINVAL` |
| 2. Architecture the filter sees, own filter | `AUDIT_ARCH_AARCH64` (0xc00000b7), native numbers (`close` = 57) | none: no filter installs | none: no filter installs |
| 2. Architecture seen by a filter inherited from a native aarch64 parent, for the emulated x86-64 program's calls | (not applicable) | **`AUDIT_ARCH_AARCH64`, aarch64 numbers**: its `close` (x86-64 number 3) reaches the filter as 57 | **same: `AUDIT_ARCH_AARCH64`, `close` as 57** (qemu 8.2.2) |
| 2. A filter that accepts only the other architecture | kills at the first call (`SIGSYS`) | not installable | not installable |
| 2. x32 bit (`__X32_SYSCALL_BIT \| getpid`) | (x86-64 only) | without a filter the emulator answers `ENOSYS`; the filter's x32 check is never exercised | same as Rosetta |
| 3. Unlisted call (`uname`, `getppid`) under the enforcing filter | `-1`, `ENOSYS`, and the process goes on; a thread still starts (`clone3` refused, the C library falls back to `clone` with `CLONE_THREAD`); `malloc` works | not applicable: no filter | not applicable: no filter |
| 3. Tripwires (`socket`, `openat`, `fork`, `mmap` with `PROT_EXEC`, `socket` from a second thread) | the child ends by **`SIGSYS`** (`waitpid`: signaled, 31, no core) before the call runs | not applicable: the probe's children run on unconfined and exit 0 | same as Rosetta |
| 3. `SECCOMP_RET_ERRNO(EACCES)` on `openat` | `-1`, `EACCES` | not applicable | not applicable |
| 4. `una-sys` `tests/solver_confinement.rs`, as uid 1000 | **6 of 6 pass** | **4 of 6**: `the_child_has_no_environment_and_only_its_pipes_after_confining` and `under_the_filter_an_unlisted_call_is_enosys_and_tripwires_kill` fail with `Seccomp(Filter(EINVAL))` | not run (see Limits) |
| 4. The same test, as root (the local-ci container today, #1394) | 5 of 6: the descriptor test fails with `Err(Root)`; the filter test passes | 4 of 6: `Err(Root)`, and the filter test fails with `EINVAL` | not run |

The results were the same as root and as uid 1000, and with Docker's default seccomp profile and without it (`--security-opt seccomp=unconfined`), except where the table says otherwise. Since Rosetta answers `EINVAL` even where the kernel would answer `EACCES`, and qemu answers `ENOSYS` for a call the kernel implements, the refusal comes from the translators, not from the kernel or Docker's profile. qemu-user's refusal matches its design, which disables seccomp in user mode so that a guest cannot block the calls the emulator needs; this probe did not re-read qemu's source, and the measurement is the evidence.

The architecture result shows why no translator could honor such a filter by passing it on: the kernel sees the translator's aarch64 calls, so a filter written for x86-64 (rule 7: `AUDIT_ARCH_X86_64` and x86-64 numbers) would kill the process at its first call if it were installed. Under emulation the confinement fails closed: `confine` returns an error, the child writes no handshake and the run is U5003 (rule 7, no unconfined fallback).

## Decision it feeds

1. **The filter cannot work under emulation.** In the emulated `linux` gate (`scripts/local-ci.sh --linux`, Rosetta), the solver's confinement tests cannot pass, and neither can any test or check that starts a solver child, since every such child fails to confine (U5003). They are marked there as not runnable under emulation, visibly in the log, as the timing oracles already are (`UNA_LOCAL_CI_SERIAL`), and never counted as passed. The mark is keyed to the gate's declared emulation, not to an `EINVAL` seen at run time, so that a native kernel's refusal is never mistaken for emulation and stays U5003.
2. **The confinement cases run natively** (rule 31, "Native on each target"): in the native `linux/arm64` container, where they pass as a non-root user, and on a native x86-64 Linux machine before each release, which is the only place the x86-64 filter, its `AUDIT_ARCH_X86_64` check and its x32 check can be exercised. qemu-user is no substitute: it refuses the filter too.
3. **No unconfined mode for emulation.** Rule 7 stands: the probe gives no reason for a fallback, and the emulated gate keeps checking the amd64 build, the tests that start no solver child, and the archive.
4. **#1394 is necessary but not sufficient for the emulated gate.** Running the container as a non-root user fixes the `Err(Root)` failures on the native arm64 leg (6 of 6 pass as uid 1000). Under Rosetta the filter still cannot be installed, so the solver tests of the `linux` gate stay marked as in item 1.
5. **Order.** ADR 0124 asks for P1 before the solver's Linux confinement ships. The filter (`crates/una-sys/src/solver/seccomp.rs`) and its tests are already on `main` (at 8e1087b8f); this result does not change them, only where their tests can run.

## Environment

| | arm64 | x86-64, Rosetta | x86-64, qemu-user |
|---|---|---|---|
| Kernel | Linux 7.0.14-linuxkit, aarch64 (Docker Desktop 4.94.0, engine 29.8.2, `DOCKER_CONTEXT=desktop-linux`, Apple silicon host, macOS 26.6.2) | same kernel | same kernel |
| Emulator | none | Rosetta for Linux, the host's `RosettaLinux/rosetta` (SHA-256 `da8c4ac7…`, package `RosettaUpdateAuto` 1.0.0.0.1786586848; the binary prints no version), mapped as `/run/rosetta/rosetta`, CPU "VirtualApple" | `qemu-x86_64-static` 8.2.2 (Ubuntu `1:8.2.2+ds-0ubuntu1.18`) and `qemu-x86_64` 10.0.13 (Debian `1:10.0.13+ds-0+deb13u1`), aarch64 static binaries started explicitly in the arm64 container |
| Container | `una-local-ci:1.98.1-arm64` (`linux/arm64`) | `una-local-ci:1.98.1` (`linux/amd64`) | `una-local-ci:1.98.1-arm64`, running the x86-64 probe |
| Page size | 4096 | 4096 | 4096 |
| Libraries | Ubuntu 24.04.5, glibc 2.39, gcc 13.3.0, rustc 1.98.1 | same | (the probe is static) |

## Method

The probe is a small measurement program in C, a host tool outside the tree (not part of Una or its toolchain), compiled statically inside each container with the image's gcc and glibc. Each scenario runs in a child made by `fork` before any filter, and the parent records how the child ended (`waitpid`) and what it wrote on a pipe:

1. no-new-privileges, then a one-instruction "allow" filter, plain and with `TSYNC`; the filter without no-new-privileges; `SECCOMP_GET_ACTION_AVAIL`; `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER)`;
2. a filter that answers `close(magic + k)` with `SECCOMP_RET_ERRNO` carrying byte `k` of `seccomp_data.arch`, or the low bits of `seccomp_data.nr`, and allows every other call, so that nothing is killed whatever the architecture; the same filter installed by the native aarch64 probe, which then executes the x86-64 probe (under Rosetta through the VM's `binfmt_misc`, and under qemu through a shell wrapper) to read what the inherited filter sees; an x32 call with no filter;
3. an enforcing filter of rule 7's shape for the binary's architecture: architecture check and, on x86-64, the x32 bit, both killing; an allowed list; `clone` only with `CLONE_THREAD`; `mmap` and `mprotect` without `PROT_EXEC`; tripwires that kill; everything else `ERRNO(ENOSYS)`; a filter accepting only the other architecture.

The repository's own test, `cargo test --locked -p una-sys --features solver --test solver_confinement` at `main` 8e1087b8f, was built in each container (as root in the amd64 one, whose toolchain lacked the musl targets that `rust-toolchain.toml` names and installed them into the throwaway container) and run as uid 1000 and as root.

## Limits

- No native x86-64 Linux kernel was available: the x86-64 filter, its `AUDIT_ARCH_X86_64` check and its x32 check were measured nowhere. That is the native x86-64 run of rule 31.
- `una-sys`'s test was not run under qemu-user: it re-executes itself for each child, and qemu-user started explicitly (not through `binfmt_misc`) hands an executed x86-64 program to the kernel, whose handler for x86-64 in this VM is Rosetta, so the children would have run under Rosetta. Changing the VM's `binfmt_misc` registration would change state other containers share, so it was not done. The C probe, which forks without executing, stands for qemu-user.
- `crates/una-cli/tests/abi.rs` was not run: it needs a full `una` with Z3 built under Rosetta. Its solver tests are expected to fail in the emulated gate with U5003 for the reasons above, as root and as non-root alike; this is an inference from `confine`, not a measurement.
- The inherited-filter reading under qemu-user was taken with 8.2.2 only. qemu 8.2.2 crashes (internal `SIGSEGV`) emulating a read of `/proc/self/maps`, so that read was skipped under qemu; 10.0.13 does not crash.
- Only 4 KiB page kernels were measured.

## Evidence

The probe source, the scripts, every run's output and the digests of the binaries are in the project's coordination folder for this issue (`runs/1203-probe-p1`), outside the repository.
