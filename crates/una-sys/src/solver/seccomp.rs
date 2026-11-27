//! The solver child's system-call filter on Linux (ADR 0124 rule 7, Question 8). The filter
//! checks the architecture first and, on x86-64, refuses a call with the x32 bit; allows the
//! calls the front end needs, listed below per architecture; fails `open`, `openat` and
//! `openat2` with `EACCES`, so that no file is opened and the C library takes its fallback;
//! kills the process at once on the other dangerous families (creating files, sockets,
//! starting programs, reading other processes' memory, signals to another process, executable
//! memory); and fails every other call with `ENOSYS`, so that it never runs and a newer C
//! library takes the fallback it has for older kernels (`clone3` is in this class, so the C
//! library falls back to `clone`).
//!
//! Opening a file is `EACCES` rather than a tripwire because probe P6 showed that it is
//! needed (rule 7, #1455): glibc 2.39 reads `/sys/devices/system/cpu/online` again each time
//! it counts processors, during the query, so reading it once before the filter is not
//! enough. Either way the call never runs: the child opens nothing after the filter, and reads
//! only the descriptors it already holds, its three pipes.
//!
//! The filter is classic BPF over `seccomp_data`, built as a flat list: each check is a
//! comparison that falls through when it does not match, so no jump crosses another check and
//! every offset is a small constant. It is installed after `PR_SET_NO_NEW_PRIVS`, for every
//! thread of the process (`SECCOMP_FILTER_FLAG_TSYNC`), by the `seccomp` system call alone:
//! a kernel without it (before 3.17) is older than any C library the project supports, and a
//! fallback through `prctl` would filter one thread only. [`Mode::Log`] replaces every kill
//! and error by `SECCOMP_RET_LOG`, for probe P6, which lists the calls a query makes outside
//! the allowed list; it confines nothing and is for that probe alone. The standard library has
//! no interface to seccomp.
#![allow(unsafe_code)]

use std::fmt;
use std::io;

/// Whether the filter enforces, or only logs what it would have refused (probe P6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Kill, refuse and allow as rule 7 says.
    Enforce,
    /// Log every call the enforcing filter would kill or refuse, and allow it. For probe P6
    /// only: a process under this mode is not confined.
    Log,
}

/// Why the filter could not be installed.
#[derive(Debug)]
pub enum InstallError {
    /// `PR_SET_NO_NEW_PRIVS` failed.
    NoNewPrivs(io::Error),
    /// The filter was refused by the `seccomp` system call, or not applied to every thread.
    Filter(io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNewPrivs(e) => write!(f, "cannot set no-new-privileges: {e}"),
            Self::Filter(e) => write!(f, "cannot install the system-call filter: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// The calls the front end needs: reading and writing its pipes, memory, threads, signals to
/// itself (and `rt_sigtimedwait`, by which the child's own thread takes its `SIGXCPU`,
/// [`crate::solver::processor_time`]: it only dequeues a signal sent to this process), clocks, randomness, the processor count, and exit. The numbers are the C library's
/// declarations for this architecture. Not here, by design, and decided by probe P6 if a C
/// library turns out to need them: `newfstatat` and `statx`, which take a path the caller
/// chooses and would leak the metadata of any file; `membarrier`, whose global command is a
/// system-wide synchronization; `getcwd`, `ioctl`, `fcntl` and `uname`, which fail with
/// `ENOSYS` and whose callers cope.
mod numbers {
    use libc::c_long;

    /// Allowed outright.
    pub(super) const ALLOWED: &[c_long] = &[
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_fstat,
        libc::SYS_lseek,
        libc::SYS_munmap,
        libc::SYS_madvise,
        libc::SYS_mremap,
        libc::SYS_brk,
        libc::SYS_futex,
        libc::SYS_rseq,
        libc::SYS_set_robust_list,
        libc::SYS_set_tid_address,
        libc::SYS_sigaltstack,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigtimedwait,
        libc::SYS_restart_syscall,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        libc::SYS_getrandom,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_yield,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getrlimit,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ];

    /// Opening a file, by any name: refused with `EACCES`, the call never running (rule 7,
    /// probe P6). `creat`, which only creates, stays a tripwire.
    #[cfg(target_arch = "x86_64")]
    pub(super) const OPEN: &[c_long] = &[libc::SYS_open, libc::SYS_openat, libc::SYS_openat2];
    #[cfg(target_arch = "aarch64")]
    pub(super) const OPEN: &[c_long] = &[libc::SYS_openat, libc::SYS_openat2];

    /// The other tripwires, which kill the process.
    #[cfg(target_arch = "x86_64")]
    pub(super) const TRIPWIRES: &[c_long] = &[
        libc::SYS_creat,
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_fork,
        libc::SYS_vfork,
    ];
    #[cfg(target_arch = "aarch64")]
    pub(super) const TRIPWIRES: &[c_long] = &[
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
    ];

    /// Allowed only with the flags of a thread (`CLONE_THREAD`); a process otherwise.
    pub(super) const CLONE: c_long = libc::SYS_clone;

    /// Allowed only toward this process: the first argument is its own process id.
    pub(super) const SIGNALS: &[c_long] = &[libc::SYS_kill, libc::SYS_tkill, libc::SYS_tgkill];

    /// Allowed only without `PROT_EXEC` in the protection argument, the third.
    pub(super) const MAPPINGS: &[c_long] = &[libc::SYS_mmap, libc::SYS_mprotect];

    /// Allowed only as a read of this process's own limits: the first argument 0 or its own
    /// process id, and the third, the new limit, null. A write, which the C library's
    /// `setrlimit` makes through it, or another process's limits, is refused with `ENOSYS`:
    /// the kernel would let a child of the same user lower any process's limits.
    pub(super) const PRLIMIT: c_long = libc::SYS_prlimit64;

    /// The architecture the filter accepts (`AUDIT_ARCH_*` of the kernel's audit interface).
    #[cfg(target_arch = "x86_64")]
    pub(super) const ARCH: u32 = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    pub(super) const ARCH: u32 = 0xc000_00b7;
}

/// The x32 bit of an x86-64 system-call number (`__X32_SYSCALL_BIT`).
#[cfg(target_arch = "x86_64")]
const X32_BIT: u32 = 0x4000_0000;

/// Offsets into `seccomp_data`: the number, the architecture, and the low and high words of
/// argument `i` (the arguments are 64-bit, and these targets are little-endian).
const NR: u32 = 0;
const ARCH: u32 = 4;
const fn arg_low(i: u32) -> u32 {
    16 + 8 * i
}
const fn arg_high(i: u32) -> u32 {
    20 + 8 * i
}

/// A BPF statement.
const fn stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

/// A BPF jump.
const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

const LOAD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
const JEQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
const JSET: u16 = (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16;
const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;

/// The program for `mode`, over the lists above, for `pid` (the child's own, for the signal
/// and limit checks).
fn program(mode: Mode, pid: u32) -> Vec<libc::sock_filter> {
    let kill = match mode {
        Mode::Enforce => libc::SECCOMP_RET_KILL_PROCESS,
        Mode::Log => libc::SECCOMP_RET_LOG,
    };
    let refuse = |errno: i32| match mode {
        Mode::Enforce => libc::SECCOMP_RET_ERRNO | (errno.cast_unsigned() & libc::SECCOMP_RET_DATA),
        Mode::Log => libc::SECCOMP_RET_LOG,
    };
    let allow = libc::SECCOMP_RET_ALLOW;
    let enosys = refuse(libc::ENOSYS);
    let number = |n: libc::c_long| u32::try_from(n).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(160);
    // The architecture, then the number, which stays in the accumulator through every check
    // that does not match, since a check loads an argument only after its number matched, and
    // every such block ends in a return.
    out.push(stmt(LOAD, ARCH));
    out.push(jump(JEQ, numbers::ARCH, 1, 0));
    out.push(stmt(RET, libc::SECCOMP_RET_KILL_PROCESS));
    out.push(stmt(LOAD, NR));
    #[cfg(target_arch = "x86_64")]
    {
        out.push(jump(JSET, X32_BIT, 0, 1));
        out.push(stmt(RET, libc::SECCOMP_RET_KILL_PROCESS));
    }
    for n in numbers::ALLOWED {
        out.push(jump(JEQ, number(*n), 0, 1));
        out.push(stmt(RET, allow));
    }
    // `clone` with the flags of a thread: the flags are the first argument.
    out.push(jump(JEQ, number(numbers::CLONE), 0, 4));
    out.push(stmt(LOAD, arg_low(0)));
    out.push(jump(JSET, libc::CLONE_THREAD.cast_unsigned(), 0, 1));
    out.push(stmt(RET, allow));
    out.push(stmt(RET, kill));
    // Signals toward this process only: the first argument is the process id, compared as the
    // 32-bit `pid_t` it is, so that 0, -1 and a process group never match.
    for n in numbers::SIGNALS {
        out.push(jump(JEQ, number(*n), 0, 4));
        out.push(stmt(LOAD, arg_low(0)));
        out.push(jump(JEQ, pid, 0, 1));
        out.push(stmt(RET, allow));
        out.push(stmt(RET, kill));
    }
    // Memory mappings and protections without execution: the protection is the third argument.
    for n in numbers::MAPPINGS {
        out.push(jump(JEQ, number(*n), 0, 4));
        out.push(stmt(LOAD, arg_low(2)));
        out.push(jump(JSET, libc::PROT_EXEC.cast_unsigned(), 0, 1));
        out.push(stmt(RET, kill));
        out.push(stmt(RET, allow));
    }
    // `prlimit64` as a read of this process's limits only: the first argument 0 or this pid,
    // and both words of the third, the new limit, zero.
    out.push(jump(JEQ, number(numbers::PRLIMIT), 0, 9));
    out.push(stmt(LOAD, arg_low(0)));
    out.push(jump(JEQ, 0, 1, 0));
    out.push(jump(JEQ, pid, 0, 5));
    out.push(stmt(LOAD, arg_low(2)));
    out.push(jump(JEQ, 0, 0, 3));
    out.push(stmt(LOAD, arg_high(2)));
    out.push(jump(JEQ, 0, 0, 1));
    out.push(stmt(RET, allow));
    out.push(stmt(RET, enosys));
    let open = refuse(libc::EACCES);
    for n in numbers::OPEN {
        out.push(jump(JEQ, number(*n), 0, 1));
        out.push(stmt(RET, open));
    }
    for n in numbers::TRIPWIRES {
        out.push(jump(JEQ, number(*n), 0, 1));
        out.push(stmt(RET, kill));
    }
    out.push(stmt(RET, enosys));
    out
}

/// Installs the filter for `mode` on this process and all its threads, after
/// `PR_SET_NO_NEW_PRIVS`. It cannot be removed.
///
/// # Errors
///
/// [`InstallError`]: no-new-privileges refused, or the filter refused by the `seccomp` system
/// call, or not applied to every thread (a positive return names the thread that could not be
/// synchronized; the filter is then not installed at all).
pub fn install(mode: Mode) -> Result<(), InstallError> {
    // SAFETY: `prctl` with `PR_SET_NO_NEW_PRIVS` and zero arguments touches no memory.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(InstallError::NoNewPrivs(io::Error::last_os_error()));
    }
    // SAFETY: `getpid` has no arguments and cannot fail.
    let pid = unsafe { libc::getpid() }.cast_unsigned();
    let filter = program(mode, pid);
    let prog = libc::sock_fprog {
        len: u16::try_from(filter.len()).unwrap_or(u16::MAX),
        filter: filter.as_ptr().cast_mut(),
    };
    // SAFETY: `prog` names `filter`, which outlives the call and which the kernel copies.
    let installed = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_TSYNC,
            &raw const prog,
        )
    };
    match installed {
        0 => Ok(()),
        -1 => Err(InstallError::Filter(io::Error::last_os_error())),
        thread => Err(InstallError::Filter(io::Error::other(format!(
            "thread {thread} could not be synchronized"
        )))),
    }
}

/// The number of instructions of the enforcing program, for the tests: every jump offset in
/// it is a small constant, so the count bounds nothing but shows the lists were included.
pub fn instruction_count() -> usize {
    program(Mode::Enforce, 1).len()
}

/// The enforcing program for `pid`, for the tests, which run it over every call number to
/// show what each one gets. It installs nothing.
pub fn enforcing_program(pid: u32) -> Vec<libc::sock_filter> {
    program(Mode::Enforce, pid)
}
