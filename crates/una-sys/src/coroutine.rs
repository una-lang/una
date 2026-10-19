//! Coroutines (ADR 0117 rule 2): stacks mapped with a guard page, and a switch between stacks
//! for each target, behind a safe interface.
//!
//! - A [`Coroutine`] owns its [`Stack`] and its body, and is neither `Send` nor `Sync`: it runs
//!   only on the thread that made it, so thread-local storage read on its stack is its thread's.
//! - The lifetime `'a` keeps a coroutine from outliving what its body borrows.
//! - A panic that escapes a body aborts the process: nothing unwinds across a switch.
//! - Dropping a suspended coroutine aborts the process, running no code on its stack. Freeing
//!   that stack could leave dangling whatever still points into it (a pinned value, or locals
//!   lent to scoped threads), and running its destructors would mean unwinding it
//!   (`docs/plan/0117-s1-design.md`, decision D1). A finished coroutine, or one never started,
//!   gives its stack back.
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

fn page_size() -> usize {
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
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE;
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
        // SAFETY: the first `guard` bytes lie inside the mapping just made, which nothing else
        // uses yet; on failure `stack`'s drop unmaps it.
        if unsafe { libc::mprotect(base.as_ptr().cast(), guard, libc::PROT_NONE) } != 0 {
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
}

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

/// A body that runs on its own stack, and that can suspend itself with [`suspend`] at any depth
/// and be resumed later on the same thread.
pub struct Coroutine<'a> {
    shared: NonNull<Shared>,
    stack: Option<Stack>,
    run: Run,
    /// Bound by `'a`, and neither `Send` nor `Sync`.
    _marker: PhantomData<(&'a (), *mut ())>,
}

impl<'a> Coroutine<'a> {
    /// A coroutine that will run `body` on `stack` when first resumed.
    pub fn new(stack: Stack, body: impl FnOnce() + 'a) -> Coroutine<'a> {
        let body: Box<dyn FnOnce() + 'a> = Box::new(body);
        // SAFETY: only the lifetime bound changes. The body runs only inside `resume`, which
        // borrows the coroutine, itself bound by `'a`; a body that never ran is dropped by the
        // coroutine's drop, within `'a`; and a suspended coroutine is never resumed after its
        // drop (which aborts) or its `mem::forget`.
        let body: Box<dyn FnOnce() + 'static> = unsafe { std::mem::transmute(body) };
        let shared = Box::new(Shared {
            resumer: Cell::new(0),
            coroutine: Cell::new(0),
            body: Cell::new(Some(body)),
            finished: Cell::new(false),
        });
        let shared = NonNull::from(Box::leak(shared));
        let mut coroutine = Coroutine {
            shared,
            stack: Some(stack),
            run: Run::NotStarted,
            _marker: PhantomData,
        };
        coroutine.prepare();
        coroutine
    }

    /// Writes the first frame on the stack, as a switch away from it would have saved it: the
    /// shared record and the entry function in callee-saved registers, a zero frame pointer,
    /// and the trampoline as the return address.
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
        // SAFETY: `shared` comes from `Box::leak` in `new` and is freed only in `drop`.
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
            Resumed::Finished
        } else {
            self.run = Run::Suspended;
            Resumed::Suspended
        }
    }

    /// The stack back, for a coroutine that finished or never started; the coroutine itself
    /// otherwise.
    pub fn into_stack(mut self) -> Result<Stack, Coroutine<'a>> {
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
        if matches!(self.run, Run::Suspended | Run::Running) {
            // Decision D1: no code runs on a suspended stack, and it is not freed.
            std::process::abort();
        }
        // SAFETY: `shared` comes from `Box::leak` in `new`, and the coroutine is not running,
        // so nothing else refers to it; a body that never ran is dropped here, on this stack.
        drop(unsafe { Box::from_raw(self.shared.as_ptr()) });
    }
}

/// Switches from the innermost coroutine the thread runs back to its resumer, whose `resume`
/// returns [`Resumed::Suspended`]; returns `true` once the coroutine is resumed again. Outside
/// a coroutine, does nothing and returns `false`.
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
        [
            0x037F_0000_1F80,
            0,
            0,
            0,
            shared,
            entry,
            0,
            trampoline as unsafe extern "C" fn() as usize,
        ]
    }

    /// Saves the callee-saved registers and control words of System V on the current stack and
    /// its pointer through `save`, then loads the stack pointer `load` and what is saved there.
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
    #[unsafe(naked)]
    unsafe extern "C" fn trampoline() {
        core::arch::naked_asm!("mov rdi, r12", "call rbx", "ud2")
    }
}
