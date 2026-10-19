//! Coroutines (ADR 0117 rule 2): stacks mapped with a guard page, and a switch between stacks
//! for each target, behind a safe interface.
//!
//! - A [`Coroutine`] owns its [`Stack`] and its body, and is neither `Send` nor `Sync`: it runs
//!   only on the thread that made it, so thread-local storage read on its stack is its thread's.
//! - Coroutines are made only inside [`scope`], which owns them as `std::thread::scope` owns its
//!   threads: a handle borrows its scope, and the scope aborts the process rather than return
//!   while one of its coroutines is unfinished, a leaked one included. So no suspended coroutine
//!   outlives its thread, nor anything its frames borrow, thread-locals included.
//! - A body is `'static`: it borrows nothing of its caller.
//! - A panic that escapes a body aborts the process: nothing unwinds across a switch.
//! - Dropping a suspended coroutine aborts the process, running no code on its stack: freeing it
//!   could leave dangling what still points into it, and running its destructors would mean
//!   unwinding it (decision D1, ADR 0117 amendment of 2026-10-19).
//! - The float control state is switched on x86-64 (MXCSR, x87) but not on AArch64 (FPCR): every
//!   task writes the one environment of ADR 0114 rule 6.4; a per-task one must switch FPCR.
//!
//! ```compile_fail
//! let local = vec![1];
//! let stack = una_sys::coroutine::Stack::new(65536).unwrap();
//! una_sys::coroutine::scope(|s| drop(s.spawn(stack, || drop(local.len())))); // borrows `local`
//! ```
//!
//! ```compile_fail
//! let stack = una_sys::coroutine::Stack::new(65536).unwrap();
//! let escaped = una_sys::coroutine::scope(|s| s.spawn(stack, || {})); // outlives its scope
//! ```
#![allow(unsafe_code)]

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The guard below each stack: at least this many bytes, and at least one page.
const GUARD: usize = 64 * 1024;

/// A stack: one mapping, whose lowest `guard` bytes are inaccessible, so that running off the
/// end faults instead of writing memory past it.
#[derive(Debug)]
pub struct Stack {
    base: NonNull<u8>,
    len: usize,
    guard: usize,
}

/// Why a stack could not be mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackError;

/// The page size of the system, at least 4 KiB.
pub fn page_size() -> usize {
    // SAFETY: `sysconf` reads a constant of the system and touches no memory of the process.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(page).unwrap_or(4096).max(4096)
}

fn round_up(bytes: usize, page: usize) -> Option<usize> {
    bytes.checked_next_multiple_of(page)
}

impl Stack {
    /// Maps a stack with at least `usable` bytes above its guard, rounded up to whole pages.
    pub fn new(usable: usize) -> Result<Stack, StackError> {
        let page = page_size();
        let guard = round_up(GUARD, page).ok_or(StackError)?;
        let usable = round_up(usable.max(page), page).ok_or(StackError)?;
        let len = guard.checked_add(usable).ok_or(StackError)?;
        if len > isize::MAX as usize {
            return Err(StackError);
        }
        #[cfg(target_os = "linux")]
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE | libc::MAP_STACK;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
        let protection = libc::PROT_READ | libc::PROT_WRITE;
        // SAFETY: an anonymous private mapping at an address the system chooses: it replaces
        // no existing mapping, and `len` was checked to be positive and at most `isize::MAX`.
        let base = unsafe { libc::mmap(std::ptr::null_mut(), len, protection, flags, -1, 0) };
        if base == libc::MAP_FAILED {
            return Err(StackError);
        }
        let Some(base) = NonNull::new(base.cast::<u8>()) else {
            return Err(StackError);
        };
        let stack = Stack { base, len, guard };
        // Stacks are `MAP_NORESERVE` on Linux, as thread stacks are: under overcommit, a touch
        // of a page past the system's memory ends in its OOM killer, not in an error here.
        // SAFETY: the first `guard` bytes lie inside the mapping just made, which nothing else
        // uses yet; on failure `stack`'s drop unmaps it.
        if unsafe { libc::mprotect(base.as_ptr().cast(), guard, libc::PROT_NONE) } != 0 {
            return Err(StackError);
        }
        // No transparent huge pages on Linux, so the pages a stack holds are the pages its task
        // touched, which the memory budget charges; `MAP_STACK` implies it from Linux 6.7, and
        // older kernels need the advice. A stack the advice fails on is not used (fail closed),
        // except for `EINVAL`, which a kernel built without transparent huge pages returns: it
        // has no huge pages to give the stack.
        // SAFETY: the advice covers exactly the mapping just made, and changes no data in it.
        #[cfg(target_os = "linux")]
        if unsafe { libc::madvise(base.as_ptr().cast(), len, libc::MADV_NOHUGEPAGE) } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL)
        {
            return Err(StackError);
        }
        Ok(stack)
    }

    /// The bytes above the guard, which code running on the stack may use.
    pub fn usable(&self) -> usize {
        self.len - self.guard
    }

    /// The address the stack grows down from: the end of the mapping.
    pub fn top(&self) -> usize {
        self.base.as_ptr().addr() + self.len
    }

    /// Gives the usable pages back to the system, which reads them as zeros next time, so a
    /// pooled stack holds no memory and no data of the task that used it. On failure, the
    /// caller drops the stack instead of pooling it.
    pub fn discard(&mut self) -> Result<(), StackError> {
        let usable = self
            .base
            .as_ptr()
            .wrapping_add(self.guard)
            .cast::<libc::c_void>();
        // SAFETY: `usable` is the mapping past its guard, owned by `self`, on which nothing runs
        // (a coroutine gives its stack back only once it finished or never started); `madvise`
        // drops the pages of an anonymous private mapping, which then read as zeros.
        #[cfg(target_os = "linux")]
        let done = unsafe { libc::madvise(usable, self.usable(), libc::MADV_DONTNEED) } == 0;
        // SAFETY: as above; a fixed anonymous mapping replaces exactly that range with zero
        // pages (`MADV_FREE` on macOS neither frees at once nor zeroes).
        #[cfg(not(target_os = "linux"))]
        let done = unsafe {
            let (protection, flags) = (libc::PROT_READ | libc::PROT_WRITE, libc::MAP_FIXED);
            let flags = flags | libc::MAP_PRIVATE | libc::MAP_ANON;
            libc::mmap(usable, self.usable(), protection, flags, -1, 0) == usable
        };
        done.then_some(()).ok_or(StackError)
    }
}

// SAFETY: a stack is memory that the value owns, with no tie to a thread; the coroutine that
// runs on it is what stays on one thread.
unsafe impl Send for Stack {}

impl Drop for Stack {
    fn drop(&mut self) {
        // SAFETY: `base` and `len` are exactly the mapping `new` made, and nothing runs on it:
        // a coroutine gives its stack back only when it has finished or never started.
        unsafe { libc::munmap(self.base.as_ptr().cast(), self.len) };
    }
}

/// What a coroutine and its resumer share: the stack pointer each side saved at its last
/// switch, and the body until it starts. Only one side runs at a time.
struct Shared {
    resumer: Cell<usize>,
    coroutine: Cell<usize>,
    body: Cell<Option<Box<dyn FnOnce()>>>,
    finished: Cell<bool>,
}

thread_local! {
    /// The innermost coroutine whose stack the thread is running, or null.
    static CURRENT: Cell<*const Shared> = const { Cell::new(std::ptr::null()) };
}

/// Where a coroutine stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    NotStarted,
    Suspended,
    Running,
    Finished,
}

/// How a call of [`Coroutine::resume`] came back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resumed {
    /// The body called [`suspend`].
    Suspended,
    /// The body returned.
    Finished,
}

/// The owner of the coroutines made in it, as `std::thread::scope` is of its threads: made only
/// by [`scope`], which never returns while a coroutine it made is unfinished. A coroutine leaked
/// while suspended (`mem::forget`, an `Rc` cycle) keeps its count up, so [`scope`] aborts the
/// process instead of returning: no suspended coroutine outlives the frame that runs its
/// scope, so none outlives its thread or the thread-locals its frames may borrow.
pub struct Scope {
    /// The coroutines made in this scope that have not finished or been dropped unstarted.
    live: Cell<usize>,
    /// Neither `Send` nor `Sync`.
    _marker: PhantomData<*mut ()>,
}

/// Runs `f` with a [`Scope`] in which it makes coroutines, and returns its result once every
/// coroutine made in it has finished or was dropped before it started. Otherwise, at the end of
/// `f` or while a panic unwinds out of it, aborts the process.
pub fn scope<R>(f: impl FnOnce(&Scope) -> R) -> R {
    /// Checks the count when the scope ends, a panic's unwinding included.
    struct End<'s>(&'s Scope);
    impl Drop for End<'_> {
        fn drop(&mut self) {
            if self.0.live.get() != 0 {
                std::process::abort();
            }
        }
    }
    let scope = Scope {
        live: Cell::new(0),
        _marker: PhantomData,
    };
    let end = End(&scope);
    let result = f(end.0);
    drop(end);
    result
}

impl Scope {
    /// A coroutine of this scope that will run `body` on `stack` when first resumed. A body is
    /// `'static`, so it borrows nothing of its caller.
    pub fn spawn(&self, stack: Stack, body: impl FnOnce() + 'static) -> Coroutine<'_> {
        let body: Box<dyn FnOnce()> = Box::new(body);
        let shared = Box::new(Shared {
            resumer: Cell::new(0),
            coroutine: Cell::new(0),
            body: Cell::new(Some(body)),
            finished: Cell::new(false),
        });
        let shared = NonNull::from(Box::leak(shared));
        self.live.set(self.live.get() + 1);
        let mut coroutine = Coroutine {
            shared,
            stack: Some(stack),
            run: Run::NotStarted,
            scope: self,
            _marker: PhantomData,
        };
        coroutine.prepare();
        coroutine
    }
}

/// A body that runs on its own stack, and that can suspend itself with [`suspend`] at any depth
/// and be resumed later on the same thread. It borrows the [`Scope`] that made it.
pub struct Coroutine<'s> {
    shared: NonNull<Shared>,
    stack: Option<Stack>,
    run: Run,
    scope: &'s Scope,
    /// Neither `Send` nor `Sync`.
    _marker: PhantomData<*mut ()>,
}

impl Coroutine<'_> {
    /// Writes the first frame on the stack, as a switch away from it would have saved it: the
    /// shared record and the entry function in callee-saved registers, a zero frame pointer,
    /// and the trampoline as the return address (`arch::initial_frame`).
    fn prepare(&mut self) {
        let Some(stack) = &self.stack else {
            return;
        };
        let top = stack.top() & !15;
        let entry = entry as extern "C" fn(*const Shared) as usize;
        let slots = arch::initial_frame(self.shared.as_ptr().addr(), entry);
        let sp = top - slots.len() * size_of::<usize>();
        // The frame is at most 160 bytes, and a stack has at least one page above its guard.
        let offset = sp - stack.base.as_ptr().addr();
        // SAFETY: `sp .. top` lies in the usable part of the mapping, which nothing else uses
        // until the coroutine starts; `sp` is 16-byte aligned, so each slot is aligned.
        unsafe {
            let frame = stack.base.as_ptr().add(offset).cast::<usize>();
            for (index, value) in slots.iter().enumerate() {
                frame.add(index).write(*value);
            }
        }
        self.shared().coroutine.set(sp);
    }

    fn shared(&self) -> &Shared {
        // SAFETY: `shared` comes from `Box::leak` in `spawn` and is freed only in `drop`.
        unsafe { self.shared.as_ref() }
    }

    /// Runs the body until it suspends or returns. Resuming a finished coroutine does nothing.
    pub fn resume(&mut self) -> Resumed {
        match self.run {
            Run::Finished => return Resumed::Finished,
            Run::Running => std::process::abort(),
            Run::NotStarted | Run::Suspended => {}
        }
        self.run = Run::Running;
        let shared = self.shared.as_ptr().cast_const();
        let previous = CURRENT.with(|current| current.replace(shared));
        let (save, load) = (
            self.shared().resumer.as_ptr(),
            self.shared().coroutine.get(),
        );
        // SAFETY: `load` is the stack pointer the coroutine saved at its last switch, or the
        // frame `prepare` wrote; its stack is alive, owned by `self`. `save` points into the
        // shared record, alive as long as `self`. The coroutine switches back to the pointer
        // saved here, so this call returns as an ordinary call.
        unsafe { arch::switch(save, load) };
        CURRENT.with(|current| current.set(previous));
        if self.shared().finished.get() {
            self.run = Run::Finished;
            self.scope.live.set(self.scope.live.get() - 1);
            Resumed::Finished
        } else {
            self.run = Run::Suspended;
            Resumed::Suspended
        }
    }

    /// The stack back, for a coroutine that finished or never started; the coroutine itself
    /// otherwise.
    pub fn into_stack(mut self) -> Result<Stack, Self> {
        if matches!(self.run, Run::Finished | Run::NotStarted)
            && let Some(stack) = self.stack.take()
        {
            return Ok(stack);
        }
        Err(self)
    }
}

impl std::fmt::Debug for Coroutine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coroutine")
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

impl Drop for Coroutine<'_> {
    fn drop(&mut self) {
        match self.run {
            // Decision D1: no code runs on a suspended stack, and it is not freed.
            Run::Suspended | Run::Running => std::process::abort(),
            Run::NotStarted => self.scope.live.set(self.scope.live.get() - 1),
            Run::Finished => {}
        }
        // SAFETY: `shared` comes from `Box::leak` in `spawn`, and the coroutine is not running,
        // so nothing else refers to it; a body that never ran is dropped here, on this stack.
        drop(unsafe { Box::from_raw(self.shared.as_ptr()) });
    }
}

/// Switches from the innermost coroutine the thread runs back to its resumer, and returns `true`
/// once resumed again; outside a coroutine, does nothing and returns `false`.
pub fn suspend() -> bool {
    let current = CURRENT.with(Cell::get);
    if current.is_null() {
        return false;
    }
    // SAFETY: `CURRENT` is set by `resume` only while that coroutine's stack is the running one,
    // and its shared record lives as long as the coroutine, which `resume` borrows.
    let shared = unsafe { &*current };
    // SAFETY: the resumer saved its stack pointer in `resumer` when it switched here, and waits
    // in `resume`; the next resume switches back to the pointer saved in `coroutine`.
    unsafe { arch::switch(shared.coroutine.as_ptr(), shared.resumer.get()) };
    true
}

/// Whether the calling code runs on a coroutine's stack.
pub fn inside() -> bool {
    !CURRENT.with(Cell::get).is_null()
}

/// The first function on a coroutine's stack, called by the trampoline with the shared record:
/// runs the body, catching any panic to abort, then switches back for the last time.
extern "C" fn entry(shared: *const Shared) {
    // SAFETY: the trampoline passes the record `prepare` wrote into the first frame, alive as
    // long as the coroutine, which `resume` borrows while this runs.
    let shared = unsafe { &*shared };
    if let Some(body) = shared.body.take()
        && let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
    {
        // Nothing unwinds across a switch; the payload's drop could panic again, so it is
        // forgotten.
        std::mem::forget(payload);
        std::process::abort();
    }
    shared.finished.set(true);
    let mut ended = 0usize;
    // SAFETY: the resumer waits in `resume`, its stack pointer saved in `resumer`; nothing
    // ever switches back to `ended`.
    unsafe { arch::switch(&raw mut ended, shared.resumer.get()) };
    std::process::abort();
}

#[cfg(target_arch = "aarch64")]
mod arch {
    /// The first frame: x19 to x30 then d8 to d15, as `switch` loads them; x19 holds the
    /// shared record, x20 the entry, x29 (the frame pointer) zero, x30 the trampoline.
    pub(super) fn initial_frame(shared: usize, entry: usize) -> [usize; 20] {
        let mut frame = [0usize; 20];
        frame[0] = shared;
        frame[1] = entry;
        frame[11] = trampoline as unsafe extern "C" fn() as usize;
        frame
    }

    /// Saves the callee-saved registers on the current stack and its pointer through `save`,
    /// then loads the stack pointer `load` and the registers saved there, and returns on it.
    /// x18, the platform register of macOS, is not touched.
    ///
    /// # Safety
    ///
    /// `save` is writable; `load` was saved by `switch`, or written by `initial_frame`, on a
    /// stack still mapped, which only this thread runs.
    #[unsafe(naked)]
    pub(super) unsafe extern "C" fn switch(save: *mut usize, load: usize) {
        core::arch::naked_asm!(
            "sub sp, sp, #160",
            "stp x19, x20, [sp, #0]",
            "stp x21, x22, [sp, #16]",
            "stp x23, x24, [sp, #32]",
            "stp x25, x26, [sp, #48]",
            "stp x27, x28, [sp, #64]",
            "stp x29, x30, [sp, #80]",
            "stp d8, d9, [sp, #96]",
            "stp d10, d11, [sp, #112]",
            "stp d12, d13, [sp, #128]",
            "stp d14, d15, [sp, #144]",
            "mov x2, sp",
            "str x2, [x0]",
            "mov sp, x1",
            "ldp x19, x20, [sp, #0]",
            "ldp x21, x22, [sp, #16]",
            "ldp x23, x24, [sp, #32]",
            "ldp x25, x26, [sp, #48]",
            "ldp x27, x28, [sp, #64]",
            "ldp x29, x30, [sp, #80]",
            "ldp d8, d9, [sp, #96]",
            "ldp d10, d11, [sp, #112]",
            "ldp d12, d13, [sp, #128]",
            "ldp d14, d15, [sp, #144]",
            "add sp, sp, #160",
            "ret",
        )
    }

    /// Calls the entry with the shared record; the entry never returns here.
    ///
    /// # Safety
    ///
    /// Reached only by `switch`'s `ret` on a frame from `initial_frame`.
    #[unsafe(naked)]
    unsafe extern "C" fn trampoline() {
        core::arch::naked_asm!("mov x0, x19", "blr x20", "brk #1")
    }
}

#[cfg(target_arch = "x86_64")]
mod arch {
    /// The first frame, as `switch` loads it: MXCSR and the x87 control word, r15, r14, r13,
    /// r12 (the shared record), rbx (the entry), rbp (zero), then the trampoline.
    pub(super) fn initial_frame(shared: usize, entry: usize) -> [usize; 8] {
        let trampoline = trampoline as unsafe extern "C" fn() as usize;
        [0x037F_0000_1F80, 0, 0, 0, shared, entry, 0, trampoline]
    }

    /// Saves the callee-saved registers and control words of System V on the current stack and
    /// its pointer through `save`, then loads the stack pointer `load` and what is saved there.
    ///
    /// # Safety
    ///
    /// As on AArch64: `save` is writable; `load` comes from `switch` or `initial_frame`.
    #[unsafe(naked)]
    pub(super) unsafe extern "C" fn switch(save: *mut usize, load: usize) {
        core::arch::naked_asm!(
            "push rbp",
            "push rbx",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "sub rsp, 8",
            "stmxcsr dword ptr [rsp]",
            "fnstcw word ptr [rsp + 4]",
            "mov qword ptr [rdi], rsp",
            "mov rsp, rsi",
            "ldmxcsr dword ptr [rsp]",
            "fldcw word ptr [rsp + 4]",
            "add rsp, 8",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbx",
            "pop rbp",
            "ret",
        )
    }

    /// Calls the entry with the shared record, the stack aligned; the entry never returns here.
    ///
    /// # Safety
    ///
    /// Reached only by `switch`'s `ret` on a frame from `initial_frame`.
    #[unsafe(naked)]
    unsafe extern "C" fn trampoline() {
        core::arch::naked_asm!("mov rdi, r12", "call rbx", "ud2")
    }
}
