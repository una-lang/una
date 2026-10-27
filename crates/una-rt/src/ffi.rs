//! The C ABI boundary between compiled Una code and the runtime (ADR 0058). This is the only
//! module of the repository that may use `unsafe`: it exports the functions the generated code
//! calls, which edition 2024 marks `#[unsafe(no_mangle)]`, and it reads what the generated code
//! passes by pointer. Every `unsafe` block states why it holds; everything else is in
//! [`crate::runtime`] and [`crate::values`], in safe Rust.
//!
//! Values cross the boundary as words (`i64`), as [`crate::values`] describes them.

use std::ffi::{CStr, c_char, c_int};

use crate::{decimal, generic, host, json, runtime, tasks, values};

/// The allocator of a debug build (ADR 0113, I3): the release build's [`cap::Cap`], wrapped to
/// keep the most bytes the process ever held, its high-water mark, and the bytes each thread
/// allocated, so that the tests check the bound of the copies by counters rather than by
/// sampling. A release build uses `Cap<System>` itself, unchanged (`lib.rs`).
#[cfg(debug_assertions)]
pub(crate) struct Peak {
    inner: cap::Cap<std::alloc::System>,
    peak: std::sync::atomic::AtomicUsize,
}

#[cfg(debug_assertions)]
thread_local! {
    /// The bytes this thread allocated, never decreased by a deallocation. A constant
    /// initializer and no destructor, so reading it in the allocator allocates nothing and is
    /// valid at any point of the thread's life.
    static THREAD_ALLOCATED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(debug_assertions)]
impl Peak {
    /// The wrapper of `inner`, with no peak yet.
    pub(crate) const fn new(inner: cap::Cap<std::alloc::System>) -> Peak {
        Peak {
            inner,
            peak: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The allocator it wraps, which counts the bytes held and enforces the ceiling.
    pub(crate) fn inner(&self) -> &cap::Cap<std::alloc::System> {
        &self.inner
    }

    /// The most bytes the process held at any point so far, in tests.
    #[cfg(test)]
    pub(crate) fn peak(&self) -> usize {
        // A monotone maximum: `Relaxed` suffices, since no other memory is published through it.
        self.peak.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Records an allocation of `size` bytes that the inner allocator made: the bytes held now,
    /// read after it, raise the peak, and the thread's own count grows. A deallocation on
    /// another thread between the two may hide a peak, so with several threads allocating the
    /// mark is a lower bound; in a quiet phase (ADR 0113, I3) it is exact.
    fn note(&self, size: usize) {
        // A monotone maximum: `Relaxed` suffices, since no other memory is published through it.
        self.peak
            .fetch_max(self.inner.allocated(), std::sync::atomic::Ordering::Relaxed);
        THREAD_ALLOCATED.with(|bytes| bytes.set(bytes.get().saturating_add(size)));
    }
}

/// The bytes the calling thread allocated so far, in a debug build.
#[cfg(debug_assertions)]
pub(crate) fn thread_allocated() -> usize {
    THREAD_ALLOCATED.with(std::cell::Cell::get)
}

#[cfg(test)]
std::thread_local! {
    /// What a test lambda holds on its task until the task ends: a stand-in for memory the task
    /// keeps, which `sched` swaps and drops with the task's other thread-locals.
    pub(crate) static FILLER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Exchanges the thread's [`FILLER`] with a parked task's, in tests.
#[cfg(test)]
pub(crate) fn swap_filler(filler: &mut Vec<u8>) {
    FILLER.with(|cell| std::mem::swap(&mut *cell.borrow_mut(), filler));
}

/// Exchanges the thread's count of allocated bytes with a parked task's (`sched`), so that each
/// task's count is its own, as on a thread of its own.
#[cfg(debug_assertions)]
pub(crate) fn swap_thread_allocated(allocated: &mut usize) {
    *allocated = THREAD_ALLOCATED.with(|cell| cell.replace(*allocated));
}

// SAFETY: every method forwards to `Cap<System>`, a `GlobalAlloc`, with the caller's arguments
// unchanged, so each keeps the contract the caller met for it; the wrapper only reads counters
// after the inner call returns, and allocates nothing itself.
#[cfg(debug_assertions)]
unsafe impl std::alloc::GlobalAlloc for Peak {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: the caller's `layout` meets `alloc`'s contract, which `Cap::alloc` has too.
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            self.note(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: the caller's `layout` meets `alloc_zeroed`'s contract, which `Cap`'s has too.
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() {
            self.note(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: the caller passes a block this allocator returned with `layout`, and every
        // block this allocator returns is the inner allocator's, so `Cap::dealloc`'s contract
        // holds.
        unsafe { self.inner.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller passes a block this allocator returned with `layout`, which is the
        // inner allocator's, and a `new_size` that meets `realloc`'s contract, so
        // `Cap::realloc`'s contract holds.
        let moved = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            self.note(new_size.saturating_sub(layout.size()));
        }
        moved
    }
}

/// The address of a compiled lambda, as a closure holds it. Its field is private to this
/// module, so only this module makes one: [`una_rt_closure`], from what the generated code
/// passes, and [`Code::NONE`]. A task's thread receives a copy of it. So no other code of the
/// runtime can put an address into a closure that [`invoke`] would call (ADR 0058, #424).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Code(i64);

impl Code {
    /// No code: [`invoke`] refuses it.
    pub(crate) const NONE: Code = Code(0);

    /// The address as a word, for the generated code that calls a function value itself
    /// (`una_rt_closure_code`).
    pub(crate) fn word(self) -> i64 {
        self.0
    }
}

/// Starts the executable: the generated `main` calls it with the process arguments, the
/// embedded description of the program (`meta`, `meta_len` bytes of UTF-8 JSON) and the
/// compiled `main` of Una (`entry`), and returns its result as the exit code.
///
/// # Safety
///
/// `argv` holds `argc` pointers to NUL-terminated strings, as the C runtime passes them to
/// `main`, and `meta` points to `meta_len` bytes that live for the whole process. `entry` is
/// the compiled `main` that `una build` generated with `meta` (#185): the runtime calls it once,
/// on a thread of its own with the stack `--max-depth` needs, after the grants are checked, and
/// it may call every exported function of this module with the words they expect, which is
/// the contract of the generated code (ADR 0058).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn una_rt_start(
    argc: c_int,
    argv: *const *const c_char,
    meta: *const u8,
    meta_len: usize,
    entry: extern "C" fn() -> i64,
) -> c_int {
    let count = usize::try_from(argc).unwrap_or(0);
    let mut args = Vec::with_capacity(count);
    // The program name, `argv[0]`, is not an option.
    for index in 1..count {
        // SAFETY: the C runtime passes `argc` valid pointers in `argv`; `index < argc`.
        let pointer = unsafe { *argv.add(index) };
        if pointer.is_null() {
            break;
        }
        // SAFETY: each `argv[i]` is a NUL-terminated string that lives for the whole process.
        let arg = unsafe { CStr::from_ptr(pointer) };
        args.push(arg.to_string_lossy().into_owned());
    }
    // SAFETY: the caller passes `meta_len` bytes at `meta` (this function's contract).
    let meta = unsafe { text(meta, meta_len) }.unwrap_or_default();
    ignore_sigpipe();
    runtime::start(&args, &meta, move || entry())
}

/// `SIGPIPE` and `SIG_IGN`, which are 13 and 1 on the three targets (`aarch64-apple-darwin`,
/// `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, whose Linux numbers are the
/// generic ones).
const SIGPIPE: c_int = 13;
const SIG_IGN: usize = 1;

unsafe extern "C" {
    /// `signal(2)` of the C library, which every executable links.
    fn signal(signum: c_int, handler: usize) -> usize;
}

/// Ignores `SIGPIPE`, as a Rust program's start does and as `una` does for `una run`: a write
/// to a closed pipe then fails with `EPIPE`, which the runtime reports at the end (exit code 2),
/// after writing the trace, instead of the signal ending the process (#184). The executable's
/// `main` is the generated C `main`, so Rust's start never runs.
fn ignore_sigpipe() {
    // SAFETY: `signal` with a valid signal number and `SIG_IGN` only changes the disposition
    // of `SIGPIPE` for the process; it runs before any thread of the runtime starts.
    unsafe {
        signal(SIGPIPE, SIG_IGN);
    }
}

/// The word of the integer literal of `len` decimal digits at `digits`, with an optional `-`,
/// borrowed: the runtime keeps it for the whole run.
///
/// # Safety
///
/// `digits` points to `len` bytes of the generated code's read-only data, which live for the
/// whole process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn una_rt_int_literal(digits: *const u8, len: usize) -> i64 {
    // SAFETY: the caller passes `len` bytes at `digits` (this function's contract).
    let text = unsafe { text(digits, len) }.unwrap_or_default();
    values::int_literal(&text)
}

/// The handle of the text literal of `len` UTF-8 bytes at `bytes`, borrowed: the runtime keeps
/// it for the whole run.
///
/// # Safety
///
/// `bytes` points to `len` bytes of the generated code's read-only data, which live for the
/// whole process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn una_rt_text_literal(bytes: *const u8, len: usize) -> i64 {
    // SAFETY: the caller passes `len` bytes at `bytes` (this function's contract).
    let text = unsafe { text(bytes, len) }.unwrap_or_default();
    values::text_literal(&text)
}

/// Writes the default floating-point environment of the calling thread (ADR 0114 rule 6.4):
/// round to nearest, ties to even, every exception masked, no flush-to-zero and no
/// denormals-are-zero, and on AArch64 no default-NaN mode. One instruction.
pub(crate) fn write_float_environment() {
    #[cfg(target_arch = "x86_64")]
    {
        let mxcsr: u32 = 0x1F80;
        // SAFETY: `ldmxcsr` reads 4 bytes from the address of `mxcsr`, a live local of that
        // size; it writes no memory and touches no stack, and 0x1F80 is the default MXCSR, so
        // no reserved bit is set and no exception can be raised by the load.
        unsafe {
            core::arch::asm!(
                "ldmxcsr [{}]",
                in(reg) &mxcsr,
                options(nostack, readonly, preserves_flags)
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `msr fpcr` writes the floating-point control register with 0: RMode round to
        // nearest, FZ, DN and AHP off, every trap enable off. It touches no memory.
        unsafe {
            core::arch::asm!("msr fpcr, {}", in(reg) 0_u64, options(nomem, nostack, preserves_flags));
        }
    }
}

/// The operations of [`crate::values`] that return a word, by word.
macro_rules! exported {
    ($($name:ident => $function:path, ($($arg:ident),*);)*) => {
        $(
            #[doc = concat!("[`", stringify!($function), "`], for the generated code.")]
            #[unsafe(no_mangle)]
            pub extern "C" fn $name($($arg: i64),*) -> i64 {
                $function($($arg),*)
            }
        )*

        /// Each entry point of the table: its name, its function, its arity, and the function
        /// called on words taken in order from a slice, missing ones 0 (#766: every entry point
        /// is called with words the generated code never passes). Also built with
        /// `--cfg una_fuzz`, for the `words` fuzz target (`fuzz/`), and never otherwise.
        #[cfg(any(test, una_fuzz))]
        #[doc(hidden)]
        pub fn exported_table() -> Vec<(&'static str, &'static str, usize, fn(&[i64]) -> i64)> {
            vec![$(
                (
                    stringify!($name),
                    stringify!($function),
                    0 $(+ { let _ = stringify!($arg); 1 })*,
                    |words: &[i64]| {
                        let next = std::cell::Cell::new(0_usize);
                        let _ = (words, &next);
                        $function($({
                            let _ = stringify!($arg);
                            let at = next.get();
                            next.set(at + 1);
                            words.get(at).copied().unwrap_or(0)
                        }),*)
                    },
                ),
            )*]
        }
    };
}

exported! {
    una_rt_int_add => values::add, (a, b);
    una_rt_int_sub => values::sub, (a, b);
    una_rt_int_mul => values::mul, (a, b);
    una_rt_int_div => values::div, (a, b);
    una_rt_int_mod => values::rem, (a, b);
    una_rt_int_neg => values::neg, (a);
    una_rt_int_compare => values::compare, (a, b);
    una_rt_int_text => runtime::int_text, (site, a);
    una_rt_bool_text => values::bool_text, (a);
    una_rt_text_concat => values::concat, (a, b);
    una_rt_text_append_owned => values::text_append_owned, (t, part);
    una_rt_text_equal => values::text_equal, (a, b);
    una_rt_text_compare => values::text_compare, (a, b);
    una_rt_text_len => values::text_len, (a);
    una_rt_data => values::data, (tag);
    una_rt_list => values::list, ();
    una_rt_map_empty => values::map_empty, ();
    una_rt_field => values::field, (object, index);
    una_rt_tag => values::tag, (value);
    una_rt_closure_code => values::closure_code, (value);
    una_rt_count => values::count, (list);
    una_rt_list_len => values::list_len, (list);
    una_rt_equal_at => values::equal, (site, a, b);
    una_rt_abs => values::abs, (x);
    una_rt_min => values::min, (a, b);
    una_rt_max => values::max, (a, b);
    una_rt_is_digits => values::is_digits, (t);
    una_rt_starts_with => values::starts_with, (t, prefix);
    una_rt_trim => values::trim, (t);
    una_rt_split => values::split, (t, on);
    una_rt_join => values::join, (parts, separator);
    una_rt_ends_with => values::ends_with, (t, suffix);
    una_rt_contains_text => values::contains_text, (t, part);
    una_rt_find => values::find, (t, text);
    una_rt_replace => values::replace, (site, t, pattern, by);
    una_rt_slice => values::slice, (site, t, start, end);
    una_rt_to_upper => values::to_upper, (t);
    una_rt_to_lower => values::to_lower, (t);
    una_rt_chars => values::chars, (t);
    una_rt_trim_start => values::trim_start, (t);
    una_rt_trim_end => values::trim_end, (t);
    una_rt_bytes => values::bytes_of, (t);
    una_rt_parse_failure => values::parse_failure, (t);
    una_rt_parse_int => values::parse_int, (t);
    una_rt_f64_literal => values::f64_literal, (bits);
    una_rt_f64_op => values::f64_op, (op, a, b, c);
    una_rt_f64_from_bits => values::f64_from_bits, (bits);
    una_rt_f32_op => values::f32_op, (op, a, b, c);
    una_rt_f32_from_bits => values::f32_from_bits, (bits);
    una_rt_parse_f32 => values::parse_f32, (t);
    una_rt_parse_f64_failure => values::parse_f64_failure, (t);
    una_rt_parse_f64 => values::parse_f64, (t);
    una_rt_as_f64 => values::as_f64, (j);
    una_rt_json_number => values::json_number, (x);
    una_rt_to_int => values::to_int, (t);
    una_rt_is_empty => values::is_empty, (xs);
    una_rt_first => values::first, (xs);
    una_rt_at => values::at, (xs, index);
    una_rt_append => values::append, (xs, item);
    una_rt_contains_at => values::contains, (site, xs, item);
    una_rt_set => values::set, (site, xs, index, item);
    una_rt_append_owned => values::append_owned, (xs, item);
    una_rt_set_owned => values::set_owned, (site, xs, index, item);
    una_rt_map_len => values::map_len, (map);
    una_rt_map_has => values::map_has, (map, key);
    una_rt_map_get => values::map_get, (map, key);
    una_rt_map_with => values::map_with, (map, key, value);
    una_rt_map_with_owned => values::map_with_owned, (map, key, value);
    una_rt_map_without => values::map_without, (map, key);
    una_rt_map_without_owned => values::map_without_owned, (map, key);
    una_rt_map_list => values::map_list, (map, part);
    una_rt_collection_len => values::collection_len, (c);
    una_rt_seq_empty => values::seq_empty, ();
    una_rt_seq_of => values::seq_of, (xs, stack);
    una_rt_seq_op => values::seq_op, (op, s, item);
    una_rt_seq_op_owned => values::seq_op_owned, (op, s, item);
    una_rt_set_empty => values::set_empty, ();
    una_rt_set_of => values::set_of, (xs);
    una_rt_set_op => values::set_op, (op, s, a, b);
    una_rt_set_op_owned => values::set_op_owned, (op, s, a, b);
    una_rt_priority_empty => values::priority_empty, (highest);
    una_rt_priority_op => values::priority_op, (op, q, item, priority);
    una_rt_priority_op_owned => values::priority_op_owned, (op, q, item, priority);
    una_rt_bit_set_empty => values::bit_set_empty, (site, size);
    una_rt_bit_set_op => values::bit_set_op, (site, op, b, x);
    una_rt_bit_set_op_owned => values::bit_set_op_owned, (site, op, b, x);
    una_rt_hash_empty => values::hash_empty, (site, set);
    una_rt_hash_of_list => values::hash_of_list, (site, set, desc, xs);
    una_rt_hash_map_op => values::hash_map_op, (site, op, desc, m, a, b);
    una_rt_hash_map_op_owned => values::hash_map_op_owned, (site, op, desc, m, a, b);
    una_rt_hash_set_op => values::hash_set_op, (site, op, desc, s, a);
    una_rt_hash_set_op_owned => values::hash_set_op_owned, (site, op, desc, s, a);
    una_rt_reuse => values::reuse, (old, tag);
    una_rt_sum => values::sum, (xs);
    una_rt_generic_compare => generic::compare, (desc, a, b);
    una_rt_generic_equal_at => generic::equal, (site, desc, a, b);
    una_rt_generic_to_int => generic::to_int, (desc, x);
    una_rt_generic_min => generic::min, (desc, a, b);
    una_rt_generic_max => generic::max, (desc, a, b);
    una_rt_generic_contains_at => generic::contains, (site, desc, xs, item);
    una_rt_generic_sum => generic::sum, (desc, xs);
    una_rt_generic_sort => generic::sort, (desc, xs);
    una_rt_generic_write => generic::write, (site, desc, x);
    una_rt_write => runtime::write, (site, ty, word);
    una_rt_descriptor_composite => generic::composite, (ty, params);
    una_rt_or_else => values::or_else, (o, default);
    una_rt_op_code => values::op_code, (instance, index);
    una_rt_fixed => values::fixed, (op, int, a, b);
    una_rt_decimal => decimal::decimal, (op, scales, a, b, r);
    una_rt_parse_decimal_failure => decimal::parse_decimal_failure, (t, scale);
    una_rt_parse_decimal => decimal::parse_decimal, (t, scale);
    una_rt_as_decimal => decimal::as_decimal, (j, scale);
    una_rt_range => values::range, (site, low, high, inclusive);
    una_rt_console_print => host::console_print, (line);
    una_rt_clock_now => host::clock_now, ();
    una_rt_env_get => host::env_get, (site, name);
    una_rt_files_read => host::files_read, (site, path);
    una_rt_files_write => host::files_write, (path, content);
    una_rt_keys_hmac => host::keys_hmac, (key_name, data);
    una_rt_http_get => host::http_get, (site, url, headers, key_name);
    una_rt_http_post => host::http_post, (site, url, headers, body, key_name);
    una_rt_http_put => host::http_put, (site, url, headers, body, key_name);
    una_rt_http_delete => host::http_delete, (site, url, headers, key_name);
    una_rt_host_failed => host::host_failed, ();
    una_rt_parse_json => json::parse_json, (t, site);
    una_rt_to_json => json::to_json, (j, site);
    una_rt_json_stopped => json::json_stopped, ();
    una_rt_quote_json => json::quote_json, (t);
    una_rt_is_json_number => json::is_json_number, (t);
    una_rt_host_instance => host::host_instance, (capability);
    una_rt_handler_instance => host::handler_instance, (name);
    una_rt_limiter_instance => host::limiter_instance, (at_most, per, waiting);
    una_rt_limit_acquire => host::limit_acquire, (site, instance);
    una_rt_event => host::event, (site, instance);
}

/// A new closure of the compiled lambda at `code`, which takes the closure and `arity`
/// parameters ([`values::closure`]).
///
/// # Safety
///
/// `code` is the address of a lambda that `una build` compiled into this executable, with the
/// default calling convention, taking the closure and `arity` words and returning two words
/// (`Lowerer::closure`, `Emitter::lambda`): [`invoke`] calls it with those arguments when the
/// arity matches its call. Only the generated code calls this function; no code of the runtime
/// does, so a closure's code is always one the compiler made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn una_rt_closure(code: i64, arity: i64) -> i64 {
    values::closure(Code(code), arity)
}

/// Adds the code of an operation to an instance being built ([`values::push_op`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_push_op(instance: i64, code: i64) {
    values::push_op(instance, code);
}

/// Records an argument of an operation's event ([`host::event_arg`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_event_arg(site: i64, slot: i64, index: i64, value: i64) {
    host::event_arg(site, slot, index, value);
}

/// Ends an operation's event ([`host::event_end`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_event_end(site: i64, slot: i64, value: i64, failed: i64) {
    host::event_end(site, slot, value, failed);
}

/// Adds a reference to a value ([`values::retain`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_retain(value: i64) {
    values::retain(value);
}

/// Releases a reference to a value ([`values::release`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_release(value: i64) {
    values::release(value);
}

/// Adds a field, item or captured value to an object being built ([`values::push`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_push(object: i64, value: i64) {
    values::push(object, value);
}

/// Enters the call at `site` (U6003 past the maximum depth): 1 when the task must stop
/// instead ([`runtime::enter`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_enter(site: i64) -> i64 {
    runtime::enter(usize::try_from(site).unwrap_or(usize::MAX))
}

/// Reports that `main` failed with `failure` (U6001), and releases it.
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_main_failed(failure: i64) {
    runtime::main_failed(failure);
}

/// Called by the C `main` of a program that performs an operation of `Http`, before
/// `una_rt_start`: lets the run build the client's TLS settings before `main` when its grants
/// include `Http` ([`runtime::enable_http`]). A program without `Http` never calls it, so the
/// linker drops the TLS stack from its executable (ADR 0074 H7).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_http_enable() {
    runtime::enable_http();
}

/// Checks the memory budget at an iteration of the loop at `site` (U6003 over it): 1 when
/// the task must stop instead ([`runtime::budget`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_budget(site: i64) -> i64 {
    runtime::budget(usize::try_from(site).unwrap_or(usize::MAX))
}

/// Enters a `for ever within` loop whose deadline is the borrowed integer `ms`
/// ([`crate::deadline::enter`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_deadline_enter(ms: i64) {
    crate::deadline::enter(ms);
}

/// Starts an iteration of a `for ever within` loop ([`crate::deadline::iteration`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_deadline_iteration() {
    crate::deadline::iteration();
}

/// Leaves a `for ever within` loop ([`crate::deadline::leave`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_deadline_leave() {
    crate::deadline::leave();
}

/// Enters `e within d`, whose deadline is the borrowed integer `ms`, and returns the frame's id
/// ([`crate::deadline::within_enter`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_within_enter(ms: i64) -> i64 {
    crate::deadline::within_enter(ms)
}

/// Ends the `within` of the frame `id`, after `e` returned or failed (`stopped` 0) or after its
/// task stopped inside it (`stopped` 1): 0 when `e`'s outcome stands, 1 when it timed out, 2 when
/// the task stops on ([`crate::deadline::within_end`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_within_end(id: i64, stopped: i64) -> i64 {
    crate::deadline::within_end(id, stopped)
}

/// The next tick of a driven program's loop, or 0 when it stops ([`runtime::drive_tick`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_drive_tick() -> i64 {
    runtime::drive_tick()
}

/// 1 when a `for ever` stops at the iteration that starts now ([`runtime::ever_start`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_ever_start() -> i64 {
    runtime::ever_start()
}

/// Leaves the body of a `for ever` ([`runtime::ever_end`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_ever_end() {
    runtime::ever_end();
}

/// 1 when the task must stop before an operation ([`runtime::poll`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_poll() -> i64 {
    runtime::poll()
}

/// 1 when the task the thread runs stopped in an operation ([`runtime::stopped`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_stopped() -> i64 {
    runtime::stopped()
}

/// Begins an operation in progress, which a cancellation does not stop
/// ([`runtime::operation_begin`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_operation_begin() {
    runtime::operation_begin();
}

/// Ends the operation in progress ([`runtime::operation_end`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_operation_end() {
    runtime::operation_end();
}

/// Runs the tasks of `together` at `site` ([`tasks::together`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_together(closures: i64, site: i64) -> i64 {
    tasks::together(closures, usize::try_from(site).unwrap_or(usize::MAX))
}

/// Runs the tasks of `together for` at `site` ([`tasks::together_for`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_together_for(closure: i64, items: i64, site: i64) -> i64 {
    tasks::together_for(closure, items, usize::try_from(site).unwrap_or(usize::MAX))
}

/// Runs the branches of `first of` at `site` ([`tasks::first_of`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_first_of(closures: i64, site: i64) -> i64 {
    tasks::first_of(closures, usize::try_from(site).unwrap_or(usize::MAX))
}

/// Opens the `tasks` statement at `site`, bounded by the integer of the borrowed word `bound`
/// when `bounded` is 1: 0, or 2 when the task stops instead ([`crate::started::tasks_begin`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_tasks_begin(bound: i64, bounded: i64, site: i64) -> i64 {
    crate::started::tasks_begin(bound, bounded, usize::try_from(site).unwrap_or(usize::MAX))
}

/// Starts a task for the closure `closure`, which the caller keeps, at the `start` at `site`:
/// 0, or 2 when the task stops instead ([`crate::started::start`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_start_task(closure: i64, site: i64) -> i64 {
    crate::started::start(closure, usize::try_from(site).unwrap_or(usize::MAX), invoke)
}

/// Ends the innermost `tasks` statement once its block has ended, cancelling its tasks first
/// when `cancel` is 1, and waits for them ([`crate::started::tasks_end`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_tasks_end(cancel: i64) {
    crate::started::tasks_end(cancel);
}

/// How the last block of tasks ended ([`tasks::status`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_together_status() -> i64 {
    tasks::status()
}

/// What compiled code returns: its value, and 0, 1 when the value is a failure, or 2 when the
/// task stopped (`spec/16 §4` rule 3). Two words, which the C calling conventions of the three
/// targets return in two registers, `rax` and `rdx` on x86-64 System V and `x0` and `x1` on
/// AArch64, Apple's and the standard one alike, as Cranelift returns a body's two results with
/// the target's default convention (`Emitter::declare`). `together_two_words` checks it on
/// each target, through the native gate.
#[repr(C)]
struct Returned {
    value: i64,
    status: i64,
}

/// Calls the closure `closure` with `arg` when its lambda takes one, on the thread of a task of
/// the block at `site` ([`tasks::Invoke`]). The code and its arity are read here, from the
/// closure, so the call's soundness rests on this module alone (ADR 0058, #424). A word that is
/// no closure, a closure without code, or one whose lambda takes another number of parameters
/// than the call gives is an internal error: nothing is called, and the task aborts with U8001,
/// which ends the run with exit code 3 (`spec/13 §5`).
pub(crate) fn invoke(closure: i64, arg: Option<i64>, site: usize) -> (i64, i64) {
    let given = usize::from(arg.is_some());
    let code = match values::closure_target(closure) {
        Some((code, arity)) if code != Code::NONE && arity == given => code,
        _ => {
            values::fault();
            let value = runtime::abort_at(
                site,
                una_diag::Code::U8001,
                "internal error: a task of `together` holds no compiled lambda that takes its arguments".to_owned(),
            );
            return (value, 2);
        }
    };
    let address = code.word() as usize as *const ();
    let returned = match arg {
        None => {
            // SAFETY: `code` is a `Code`, which only this module makes: `una_rt_closure`, whose
            // contract is that the generated code passes the address of a compiled lambda with
            // the default calling convention, taking the closure and `arity` words and returning
            // two words (`Lowerer::closure`, `Emitter::lambda`, `Emitter::declare`); a task's
            // thread receives it unchanged (`values::export`, `values::import`). It is not
            // `Code::NONE`, and its arity is 0, both checked above.
            let function: extern "C" fn(i64) -> Returned = unsafe { std::mem::transmute(address) };
            function(closure)
        }
        Some(arg) => {
            // SAFETY: as above, for a lambda whose arity is 1, checked above (`together for`).
            let function: extern "C" fn(i64, i64) -> Returned =
                unsafe { std::mem::transmute(address) };
            function(closure, arg)
        }
    };
    (returned.value, returned.status)
}

/// Leaves a call.
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_leave() {
    runtime::leave();
}

/// The text of `len` bytes at `pointer`; `None` for a null pointer or bytes that are not
/// UTF-8.
///
/// # Safety
///
/// `pointer`, when not null, points to `len` initialized bytes that stay valid during the call.
unsafe fn text(pointer: *const u8, len: usize) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: `pointer` is not null, and points to `len` initialized bytes (this function's
    // contract).
    let bytes = unsafe { std::slice::from_raw_parts(pointer, len) };
    String::from_utf8(bytes.to_vec()).ok()
}

/// The words of [`una_rt_int_literal`] and [`una_rt_text_literal`] for `bytes`, for the
/// `literal` fuzz target (`fuzz/`, #766): the generated code passes its read-only data, the
/// target any bytes. Built only with `--cfg una_fuzz`.
#[cfg(una_fuzz)]
#[doc(hidden)]
pub fn fuzz_literals(bytes: &[u8]) -> (i64, i64) {
    // SAFETY: `bytes` is a slice, so its `len` bytes are initialized and stay valid during the
    // call; both functions copy them (`text`) before returning, so the lifetime their contract
    // asks for, the whole process, is not needed here.
    unsafe {
        (
            una_rt_int_literal(bytes.as_ptr(), bytes.len()),
            una_rt_text_literal(bytes.as_ptr(), bytes.len()),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::process::Command;

    use super::{Code, Returned, invoke};
    use crate::{host, runtime, tasks, values};

    thread_local! {
        /// How many times the test lambdas below ran on this thread.
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }

    /// A lambda with no parameter, as `together` makes, returning a value that no other
    /// word has and the status of a failure, so that two words crossed in order show.
    extern "C" fn no_parameter(_closure: i64) -> Returned {
        CALLS.with(|calls| calls.set(calls.get() + 1));
        Returned {
            value: 0x0123_4567_89ab_cdef,
            status: 1,
        }
    }

    /// A lambda with one parameter, as `together for` makes.
    extern "C" fn one_parameter(_closure: i64, arg: i64) -> Returned {
        CALLS.with(|calls| calls.set(calls.get() + 1));
        Returned {
            value: arg.wrapping_mul(3),
            status: 0,
        }
    }

    /// A lambda with one parameter that says on stderr that it ran, for the scenario that runs
    /// in a process of its own, where no other trace of a call survives the abort.
    extern "C" fn marked(_closure: i64, _arg: i64) -> Returned {
        eprintln!("{MARK}");
        Returned {
            value: 0,
            status: 0,
        }
    }

    /// What [`marked`] says.
    const MARK: &str = "una-rt test: the refused lambda ran";

    fn none() -> Code {
        Code(i64::try_from(no_parameter as extern "C" fn(i64) -> Returned as usize).unwrap())
    }

    fn one() -> Code {
        Code(i64::try_from(one_parameter as extern "C" fn(i64, i64) -> Returned as usize).unwrap())
    }

    #[test]
    #[cfg(debug_assertions)]
    fn i3_the_debug_high_water_mark_rises_with_an_allocation_and_never_falls() {
        // ADR 0113, I3: the debug allocator keeps the most the process held, and each thread's
        // own allocations. A block of 1 MiB adds at least that to this thread's count, the mark
        // never falls, freeing the block included, and it is at least the block. (What other
        // tests allocate at once only raises the mark.)
        let (before, mine) = (crate::peak(), super::thread_allocated());
        let block: Vec<u8> = Vec::with_capacity(1 << 20);
        assert!(super::thread_allocated() - mine >= 1 << 20);
        let during = crate::peak();
        assert!(
            during >= before && during >= 1 << 20,
            "{during} after {before}"
        );
        drop(block);
        assert!(crate::peak() >= during);
    }

    #[test]
    fn a_closure_is_called_with_the_arguments_its_arity_takes_and_returns_two_words_in_order() {
        // #424 (F3): the value and the status come back as the two words the code returned.
        assert_eq!(
            invoke(values::closure(none(), 0), None, 0),
            (0x0123_4567_89ab_cdef, 1)
        );
        assert_eq!(invoke(values::closure(one(), 1), Some(14), 0), (42, 0));
        assert_eq!(CALLS.with(Cell::get), 2);
    }

    #[test]
    fn a_closure_of_another_arity_or_a_word_that_is_no_closure_is_refused_without_a_call() {
        // #424 (F1): `invoke` checks what it calls before the transmute; each refusal is a
        // fault and stops the task, and no code runs. Here, outside a run, the abort it asks
        // for has no state to record it in.
        let faults = values::faults();
        let refused = [
            invoke(values::closure(none(), 0), Some(5), 0),
            invoke(values::closure(one(), 1), None, 0),
            // A list is no closure, and neither is a word that names no object.
            invoke(values::list(), None, 0),
            invoke(7, None, 0),
            // A closure without code, and one whose arity no lambda has.
            invoke(values::closure(Code::NONE, 0), None, 0),
            invoke(values::closure(none(), -1), None, 0),
        ];
        assert_eq!(refused, [(0, 2); 6]);
        assert_eq!(CALLS.with(Cell::get), 0);
        assert!(
            values::faults() >= faults + 6,
            "every refused call is a fault"
        );
    }

    /// The name of [`a_refused_task_aborts_the_run`]'s scenario, run in a process of its own
    /// since an abort ends the process.
    const SCENARIO: &str = "ffi::tests::a_refused_task_aborts_the_run_scenario";

    #[test]
    #[ignore = "run by `a_refused_task_aborts_the_run`, in a process of its own"]
    fn a_refused_task_aborts_the_run_scenario() {
        // A block of one task whose closure takes a parameter, which `together` does not give.
        let meta = r#"{"grants":[],"uses":[],"sites":[{"file":"src/t.una","start":{"line":3,"col":3},"end":{"line":4,"col":10}}]}"#;
        let marked =
            Code(i64::try_from(marked as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        runtime::start(&[], meta, move || {
            let closures = values::list();
            let closure = values::closure(marked, 1);
            values::push(closures, closure);
            values::release(closure);
            let value = tasks::together(closures, 0);
            values::release(closures);
            value
        });
    }

    /// The scenarios of [`a_text_call_outside_its_precondition_aborts_the_run`], each run in a
    /// process of its own since an abort ends the process, with the message each aborts with.
    const TEXT_SCENARIOS: [(&str, &str); 4] = [
        (
            "ffi::tests::a_slice_before_the_text_scenario",
            values::SLICE_OUTSIDE,
        ),
        (
            "ffi::tests::a_slice_past_the_text_scenario",
            values::SLICE_OUTSIDE,
        ),
        (
            "ffi::tests::a_slice_ending_before_it_starts_scenario",
            values::SLICE_OUTSIDE,
        ),
        (
            "ffi::tests::a_replace_of_an_empty_pattern_scenario",
            values::REPLACE_EMPTY,
        ),
    ];

    /// `slice(t, start, end)` of "abc" in a scenario: it aborts, so the mark never prints.
    fn slice_scenario(start: i64, end: i64) {
        runtime::start(&[], SET_META, move || {
            let t = values::text_word("abc".to_owned());
            let taken = values::slice(
                0,
                t,
                values::int_word(&start.into()),
                values::int_word(&end.into()),
            );
            eprintln!("{MARK}: slice returned {taken}");
            0
        });
    }

    #[test]
    #[ignore = "run by `a_text_call_outside_its_precondition_aborts_the_run`, in a process of its own"]
    fn a_slice_before_the_text_scenario() {
        slice_scenario(-1, 1);
    }

    #[test]
    #[ignore = "run by `a_text_call_outside_its_precondition_aborts_the_run`, in a process of its own"]
    fn a_slice_past_the_text_scenario() {
        slice_scenario(0, 4);
    }

    #[test]
    #[ignore = "run by `a_text_call_outside_its_precondition_aborts_the_run`, in a process of its own"]
    fn a_slice_ending_before_it_starts_scenario() {
        slice_scenario(2, 1);
    }

    #[test]
    #[ignore = "run by `a_text_call_outside_its_precondition_aborts_the_run`, in a process of its own"]
    fn a_replace_of_an_empty_pattern_scenario() {
        runtime::start(&[], SET_META, || {
            let t = values::text_word("abc".to_owned());
            let pattern = values::text_word(String::new());
            let by = values::text_word("x".to_owned());
            let replaced = values::replace(0, t, pattern, by);
            eprintln!("{MARK}: replace returned {replaced}");
            0
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_text_call_outside_its_precondition_aborts_the_run() {
        // #693: `slice` outside `0 <= start <= end <= t.len` and `replace` with an empty pattern
        // abort at the call with U8001, the interpreter's message and exit code 3, as `set` does
        // (#480); native code used to clamp the positions and replace the empty pattern.
        for (scenario, message) in TEXT_SCENARIOS {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    scenario,
                    "--nocapture",
                    "--test-threads",
                    "1",
                ])
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(3), "{scenario}: {stderr}");
            assert!(stderr.contains("U8001"), "{scenario}: {stderr}");
            assert!(stderr.contains("src/s.una"), "{scenario}: {stderr}");
            assert!(stderr.contains(message), "{scenario}: {stderr}");
            assert!(!stderr.contains(MARK), "{scenario}: it returned: {stderr}");
        }
    }

    #[test]
    fn slice_and_replace_inside_their_preconditions_compute() {
        let t = values::text_word("abc".to_owned());
        let at = |n: i64| values::int_word(&n.into());
        assert_eq!(values::text_value(values::slice(0, t, at(1), at(3))), "bc");
        assert_eq!(values::text_value(values::slice(0, t, at(3), at(3))), "");
        assert_eq!(values::text_value(values::slice(0, t, at(0), at(0))), "");
        // Positions count scalars: the end of "a\u{e9}" is 2, inside a text of three bytes.
        let accented = values::text_word("a\u{e9}".to_owned());
        assert_eq!(
            values::text_value(values::slice(0, accented, at(1), at(2))),
            "\u{e9}"
        );
        let pattern = values::text_word("b".to_owned());
        let by = values::text_word("x".to_owned());
        assert_eq!(
            values::text_value(values::replace(0, t, pattern, by)),
            "axc"
        );
    }

    /// The names of [`a_set_outside_the_list_aborts_the_run`]'s scenarios, each run in a process
    /// of its own since an abort ends the process.
    const SET_COPIED: &str = "ffi::tests::a_set_outside_the_list_copied_scenario";
    const SET_OWNED: &str = "ffi::tests::a_set_outside_the_list_owned_scenario";

    /// The site of the `set` call in the scenarios.
    const SET_META: &str = r#"{"grants":[],"uses":[],"sites":[{"file":"src/s.una","start":{"line":7,"col":3},"end":{"line":7,"col":33}}]}"#;

    #[test]
    #[ignore = "run by `a_set_outside_the_list_aborts_the_run`, in a process of its own"]
    fn a_set_outside_the_list_copied_scenario() {
        runtime::start(&[], SET_META, || {
            let xs = values::list();
            values::push(xs, values::int_word(&1.into()));
            // A second holder makes `set` copy.
            values::retain(xs);
            let tuple = values::set(0, xs, values::int_word(&1.into()), 0);
            eprintln!("{MARK}: set returned {tuple}");
            0
        });
    }

    #[test]
    #[ignore = "run by `a_set_outside_the_list_aborts_the_run`, in a process of its own"]
    fn a_set_outside_the_list_owned_scenario() {
        runtime::start(&[], SET_META, || {
            let xs = values::list();
            values::push(xs, values::int_word(&1.into()));
            let tuple = values::set_owned(0, xs, values::int_word(&(-1).into()), 0);
            eprintln!("{MARK}: set returned {tuple}");
            0
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_set_outside_the_list_aborts_the_run() {
        // #480, `spec/17 §7.3`: `set`'s `requires` keeps its index in range, and outside it the
        // interpreter aborts with a fault. Native code used to leave the list unchanged and go
        // on; now both the copying and the in-place update abort at the call with U8001, the
        // interpreter's message, and exit code 3, and nothing after them runs.
        for scenario in [SET_COPIED, SET_OWNED] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    scenario,
                    "--nocapture",
                    "--test-threads",
                    "1",
                ])
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(3), "{scenario}: {stderr}");
            assert!(stderr.contains("U8001"), "{scenario}: {stderr}");
            assert!(stderr.contains("src/s.una"), "{scenario}: {stderr}");
            assert!(
                stderr.contains(
                    "evaluation stopped: `set` at an index outside the list, which its precondition excludes"
                ),
                "{scenario}: {stderr}"
            );
            assert!(!stderr.contains(MARK), "{scenario}: set returned: {stderr}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_refused_task_aborts_the_run() {
        // #424 (from its review): a refused call is an internal error, not a quiet cancellation:
        // the run ends with U8001 at the block and exit code 3, and the lambda never ran.
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                SCENARIO,
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(3), "{stderr}");
        assert!(stderr.contains("U8001"), "{stderr}");
        assert!(stderr.contains("src/t.una"), "{stderr}");
        assert!(!stderr.contains(MARK), "the lambda ran: {stderr}");
        assert!(
            stderr
                .contains("a task of `together` holds no compiled lambda that takes its arguments"),
            "{stderr}"
        );
    }

    /// A xorshift generator, so that the words are the same on every run (#766).
    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// The number of words of [`word_pool`]: 26 chosen, then 16 drawn.
    const WORD_POOL_LEN: usize = 42;

    /// One call of the word tests: the entry point's name, its function on words, the words.
    type WordCase = (&'static str, fn(&[i64]) -> i64, Vec<i64>);

    /// The calls of the word tests (#766), in a fixed order: every entry point of the table but
    /// the host operations, whose effects (output, files, network) need a run's grants and are
    /// tested with them. All tuples of `pool` for an arity of two or less, and `drawn` tuples
    /// drawn with a fixed seed above. `range` takes its words from `bounds` when given.
    fn word_cases(pool: &[i64], bounds: Option<&[i64]>, drawn: usize) -> Vec<WordCase> {
        let mut state: u64 = 0x766;
        let mut cases = Vec::new();
        for (name, function, arity, call) in super::exported_table() {
            if function.starts_with("host") {
                continue;
            }
            let words = match bounds {
                Some(bounds) if name == "una_rt_range" => bounds,
                _ => pool,
            };
            let mut tuples: Vec<Vec<i64>> = Vec::new();
            match arity {
                0 => tuples.push(Vec::new()),
                1 => tuples.extend(words.iter().map(|&a| vec![a])),
                2 => {
                    for &a in words {
                        tuples.extend(words.iter().map(|&b| vec![a, b]));
                    }
                }
                _ => {
                    for _ in 0..drawn {
                        tuples.push(
                            (0..arity)
                                .map(|_| {
                                    let pick = xorshift(&mut state) % words.len() as u64;
                                    words[usize::try_from(pick).unwrap_or(0)]
                                })
                                .collect(),
                        );
                    }
                }
            }
            cases.extend(tuples.into_iter().map(|tuple| (name, call, tuple)));
        }
        cases
    }

    /// The words of the word tests: boundary words, live handles of each kind, a freed handle,
    /// handles one slot off a live one, and 16 words of a fixed seed. Made in the same order
    /// on every call, so a run made again holds the same words. The test under Miri picks words
    /// by their position here: a word added or moved changes [`WORD_POOL_LEN`] and those
    /// positions with it.
    fn word_pool() -> Vec<i64> {
        use num_bigint::BigInt;

        let int = |value: i64| values::int_word(&BigInt::from(value));
        let freed = values::text_word("freed".to_owned());
        values::release(freed);
        let text = values::text_literal("abc");
        let some_list = values::append(values::list(), text);
        let mut pool = vec![
            0,
            1,
            2,
            3,
            -1,
            -2,
            i64::MIN,
            i64::MAX,
            i64::MIN + 1,
            i64::MAX - 1,
            1 << 32,
            1 << 62,
            -(1 << 62),
            // A handle beyond the table, one of a freed slot, and those one slot off a live one.
            1 << 40,
            freed,
            some_list.wrapping_add(2),
            some_list.wrapping_sub(2),
            int(7),
            int(-7),
            values::int_word(&(BigInt::from(i64::MAX) * 4)),
            text,
            values::text_word(String::new()),
            values::list(),
            some_list,
            values::data(3),
            values::map_empty(),
        ];
        let mut state: u64 = 0x0766_0766;
        for _ in 0..16 {
            pool.push(xorshift(&mut state).cast_signed());
        }
        pool
    }

    #[test]
    fn every_entry_point_takes_any_word_without_a_panic() {
        // #766, step 1: a lowering bug passes the runtime a word it never should. Each entry
        // point of the table must then return a word or count a fault, never panic, and the
        // runtime must stay usable. Outside a run, an abort returns 0 rather than ending the
        // process (`runtime::abort_at`), so every call returns here; `range` gets small bounds,
        // since outside a run no memory budget bounds its list. The same calls inside a run
        // are `every_entry_point_takes_any_word_in_a_run`.
        std::thread::spawn(|| {
            use num_bigint::BigInt;

            let int = |value: i64| values::int_word(&BigInt::from(value));
            // Under Miri (#778), where each allocation is slow, a pool of one word of each kind
            // (zero, a tagged integer, the extremes, a handle beyond the table, a freed one and
            // the two a slot off a live one, a big integer, a text, a list, a variant and a map)
            // and 20 drawn tuples, about 10,000 calls rather than more than 100,000. The
            // indices are positions in `word_pool`, whose length is checked so that they move
            // with it.
            // The pool's length is checked on every run, under Miri or not.
            assert_eq!(word_pool().len(), WORD_POOL_LEN);
            let (pool, drawn, least) = if cfg!(miri) {
                let all = word_pool();
                let kinds = [0, 3, 6, 7, 13, 14, 15, 16, 19, 20, 23, 24, 25];
                (kinds.iter().map(|&k| all[k]).collect(), 20, 5_000)
            } else {
                (word_pool(), 600, 10_000)
            };
            let small: Vec<i64> = (-3..=3).map(int).collect();
            let cases = word_cases(&pool, Some(&small), drawn);
            assert!(cases.len() > least, "{}", cases.len());
            let mut failures = Vec::new();
            for (name, call, tuple) in &cases {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(tuple)));
                if result.is_err() {
                    failures.push(format!("{name}({tuple:?})"));
                }
            }
            assert!(
                failures.is_empty(),
                "{} panics:\n{}",
                failures.len(),
                failures.join("\n")
            );
            // The runtime still works.
            let text = values::text_literal("still");
            assert_eq!(values::text_len(text), int(5));
        })
        .join()
        .unwrap();
    }

    /// The scenario of [`every_entry_point_takes_any_word_in_a_run`], run in a process of its own
    /// since an abort ends the process.
    const WORDS_IN_A_RUN: &str = "ffi::tests::words_in_a_run_scenario";

    /// Where the parent test writes the first call the scenario makes, by the parent's process.
    fn words_from(parent: u32) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("una-rt-words-{parent}"))
    }

    /// What the scenario prints before each call, and once all are made.
    const WORDS_CALL: &str = "una-rt words: call";
    const WORDS_DONE: &str = "una-rt words: done";

    #[test]
    #[ignore = "run by `every_entry_point_takes_any_word_in_a_run`, in a process of its own"]
    fn words_in_a_run_scenario() {
        let from: usize = std::fs::read_to_string(words_from(std::os::unix::process::parent_id()))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let args = ["--max-memory".to_owned(), "8".to_owned()];
        runtime::start(&args, SET_META, move || {
            let pool = word_pool();
            // Fewer drawn tuples than outside a run: each call that ends the run costs a process.
            let cases = word_cases(&pool, None, 100);
            for (index, (name, call, tuple)) in cases.iter().enumerate().skip(from) {
                eprintln!("{WORDS_CALL} {index} {name}{tuple:?}");
                call(tuple);
            }
            // The run still works.
            let text = values::text_literal("still");
            assert_eq!(values::text_len(text), values::int_word(&5.into()));
            eprintln!("{WORDS_DONE} {}", cases.len());
            0
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn every_entry_point_takes_any_word_in_a_run() {
        // #766, step 1, inside a started run, with its heap, its sites and a memory budget of
        // 8 MiB: the same calls, `range` with every word. A call either returns, or ends the
        // run as the model says (exit code 1 or 3 with its diagnostic: U6003 over the budget,
        // U8001 for a `set` outside its list), and the next run resumes after it. A panic, a
        // signal, another exit, or a run that does not end within `DEADLINE` (a call that hangs,
        // itself a defect) is a failure that names the call, its entry point and its words.
        // The slowest run takes about 0.7 s on a recent machine, so a minute leaves room for
        // slower machines and for Linux under emulation.
        const DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
        let from = words_from(std::process::id());
        let log = std::env::temp_dir().join(format!("una-rt-words-{}.log", std::process::id()));
        let mut next = 0_usize;
        let mut ended = 0_usize;
        let mut failures = Vec::new();
        let mut slowest = std::time::Duration::ZERO;
        loop {
            std::fs::write(&from, next.to_string()).unwrap();
            // The child's stderr goes to a file, which never blocks it as a full pipe would.
            let file = std::fs::File::create(&log).unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    WORDS_IN_A_RUN,
                    "--nocapture",
                    "--test-threads",
                    "1",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(file)
                .spawn()
                .unwrap();
            let started = std::time::Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break Some(status);
                }
                if started.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            };
            slowest = slowest.max(started.elapsed());
            let stderr = std::fs::read_to_string(&log).unwrap_or_default();
            if stderr.contains(WORDS_DONE) {
                break;
            }
            let last = stderr
                .lines()
                .filter_map(|line| line.strip_prefix(WORDS_CALL))
                .filter_map(|rest| {
                    let mut parts = rest.trim().splitn(2, ' ');
                    let index: usize = parts.next()?.parse().ok()?;
                    Some((index, parts.next().unwrap_or("").to_owned()))
                })
                .next_back();
            let Some((last, call)) = last else {
                failures.push(format!("no call made from {next}: {stderr}"));
                break;
            };
            match status {
                None => failures.push(format!("call {last}: {call}: hung for {DEADLINE:?}")),
                Some(status) => {
                    let in_model = matches!(status.code(), Some(1 | 3))
                        && (stderr.contains("U6003") || stderr.contains("U8001"))
                        && !stderr.contains("panicked");
                    if !in_model {
                        failures.push(format!("call {last}: {call}: {status:?}: {stderr}"));
                    }
                }
            }
            ended += 1;
            next = last + 1;
        }
        let _ = std::fs::remove_file(&from);
        let _ = std::fs::remove_file(&log);
        eprintln!("{ended} runs ended in the model; the slowest took {slowest:?}");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        assert!(ended < 5_000, "{ended} runs ended");
    }

    /// The meta of the count-limit scenarios: one site, which no report of them names.
    const LIMIT_META: &str = r#"{"grants":[],"uses":[],"sites":[{"file":"src/c.una","start":{"line":2,"col":3},"end":{"line":2,"col":9}}]}"#;

    /// The trace a count-limit scenario asks for, which its stop must not write: next to the
    /// test executable, so that the test and the scenario it runs name the same file, and runs
    /// from two checkouts never share one.
    fn limit_trace(scenario: &str) -> std::path::PathBuf {
        let name = scenario.rsplit("::").next().unwrap_or(scenario);
        std::env::current_exe()
            .unwrap()
            .with_file_name(format!("una-rt-{name}.json"))
    }

    /// Runs `main` with a trace at `trace` and the count limit lowered to `limit`.
    fn limit_scenario(trace: &str, limit: u32, entry: impl FnOnce() -> i64 + Send + 'static) {
        values::TEST_COUNT_LIMIT.store(limit, std::sync::atomic::Ordering::SeqCst);
        runtime::start(&["--trace".to_owned(), trace.to_owned()], LIMIT_META, entry);
    }

    /// A lambda with no parameter that takes a fresh text to its count's limit of 2.
    extern "C" fn to_the_limit(_closure: i64) -> Returned {
        let x = values::text_word("x".to_owned());
        values::retain(x);
        values::retain(x);
        eprintln!("{MARK}");
        Returned {
            value: 0,
            status: 0,
        }
    }

    /// Two tasks that reach the limit together, after both arrive here.
    static TOGETHER: std::sync::LazyLock<std::sync::Barrier> =
        std::sync::LazyLock::new(|| std::sync::Barrier::new(2));

    /// Says on stdout, flushed, that the task is about to reach the limit, then waits for the
    /// other task and reaches it.
    extern "C" fn to_the_limit_together(closure: i64) -> Returned {
        {
            use std::io::Write;
            println!("{BEFORE_LIMIT}");
            let _ = std::io::stdout().flush();
        }
        TOGETHER.wait();
        to_the_limit(closure)
    }

    /// What each of the two tasks says before it reaches the limit.
    const BEFORE_LIMIT: &str = "una-rt test: a task reaches the limit";

    /// What `main` says once a count reached the shipped limit without passing it.
    const AT_LIMIT: &str = "una-rt test: the count reached 4294967295";

    /// Runs `scenario` in a process of its own with a trace path, and returns its exit code,
    /// its stderr, whether the trace was written, and its stdout.
    fn run_limit_scenario(scenario: &str) -> (Option<i32>, String, bool, String) {
        let trace = limit_trace(scenario);
        let _ = std::fs::remove_file(&trace);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                scenario,
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .output()
            .unwrap();
        let written = trace.exists();
        let _ = std::fs::remove_file(&trace);
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            written,
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    const INTERNAL: &str = "[U8001] Error: internal error: a reference count of the native runtime reached its limit (4294967295) with less memory held than that many references take";
    const OVER: &str = "[U6003] Error: the native runtime's reference count of a value reached its limit (4294967295)";

    #[test]
    #[ignore = "run by `i2_a_count_at_its_limit_ends_the_process_and_never_returns`"]
    fn i2_count_limit_in_main_scenario() {
        limit_scenario(
            &limit_trace("i2_count_limit_in_main_scenario").to_string_lossy(),
            2,
            || {
                let x = values::text_word("x".to_owned());
                values::retain(x);
                values::retain(x);
                eprintln!("{MARK}");
                0
            },
        );
    }

    #[test]
    #[ignore = "run by `i2_a_count_at_its_limit_ends_the_process_and_never_returns`"]
    fn i2_count_limit_shipped_scenario() {
        // The limit the runtime ships, 4294967295: a count one below it takes one more
        // reference without overflow, and the next retain is the stop.
        limit_scenario(
            &limit_trace("i2_count_limit_shipped_scenario").to_string_lossy(),
            values::COUNT_LIMIT,
            || {
                let x = values::text_word("x".to_owned());
                values::set_count_for_test(x, values::COUNT_LIMIT - 1);
                values::retain(x);
                {
                    use std::io::Write;
                    println!("{AT_LIMIT}");
                    let _ = std::io::stdout().flush();
                }
                values::retain(x);
                eprintln!("{MARK}");
                0
            },
        );
    }

    #[test]
    #[ignore = "run by `i2_a_count_at_its_limit_ends_the_process_and_never_returns`"]
    fn i2_count_limit_with_the_memory_it_needs_scenario() {
        runtime::TEST_COUNT_LIMIT_BYTES.store(0, std::sync::atomic::Ordering::SeqCst);
        limit_scenario(
            &limit_trace("i2_count_limit_with_the_memory_it_needs_scenario").to_string_lossy(),
            2,
            || {
                let x = values::text_word("x".to_owned());
                values::retain(x);
                values::retain(x);
                eprintln!("{MARK}");
                0
            },
        );
    }

    #[test]
    #[ignore = "run by `i2_a_count_at_its_limit_ends_the_process_and_never_returns`"]
    fn i2_count_limit_in_a_task_scenario() {
        let lambda =
            Code(i64::try_from(to_the_limit as extern "C" fn(i64) -> Returned as usize).unwrap());
        limit_scenario(
            &limit_trace("i2_count_limit_in_a_task_scenario").to_string_lossy(),
            2,
            move || {
                let closures = values::list();
                let closure = values::closure(lambda, 0);
                values::push(closures, closure);
                values::release(closure);
                let value = tasks::together(closures, 0);
                eprintln!("{MARK}: the block returned");
                value
            },
        );
    }

    #[test]
    #[ignore = "run by `i2_a_count_at_its_limit_ends_the_process_and_never_returns`"]
    fn i2_count_limit_during_an_import_scenario() {
        let marked =
            Code(i64::try_from(marked as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        limit_scenario(
            &limit_trace("i2_count_limit_during_an_import_scenario").to_string_lossy(),
            values::COUNT_LIMIT,
            move || {
                // A list holding one text three times: its import into the task's heap retains the
                // text once per element, and the third finds the lowered limit of 3.
                let x = values::text_word("x".to_owned());
                let inner = values::list();
                for _ in 0..3 {
                    values::push(inner, x);
                }
                values::release(x);
                let items = values::list();
                values::push(items, inner);
                values::release(inner);
                let closure = values::closure(marked, 1);
                values::TEST_COUNT_LIMIT.store(3, std::sync::atomic::Ordering::SeqCst);
                let value = tasks::together_for(closure, items, 0);
                eprintln!("{MARK}: the block returned");
                value
            },
        );
    }

    #[test]
    #[ignore = "run by `i2_a_count_at_its_limit_ends_the_process_and_never_returns`"]
    fn i2_count_limit_in_two_tasks_at_once_scenario() {
        let lambda = Code(
            i64::try_from(to_the_limit_together as extern "C" fn(i64) -> Returned as usize)
                .unwrap(),
        );
        runtime::TEST_EXPECT_PARKED.store(1, std::sync::atomic::Ordering::SeqCst);
        limit_scenario(
            &limit_trace("i2_count_limit_in_two_tasks_at_once_scenario").to_string_lossy(),
            2,
            move || {
                let closures = values::list();
                for _ in 0..2 {
                    let closure = values::closure(lambda, 0);
                    values::push(closures, closure);
                    values::release(closure);
                }
                let value = tasks::together(closures, 0);
                eprintln!("{MARK}: the block returned");
                value
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i2_a_count_at_its_limit_ends_the_process_and_never_returns() {
        // ADR 0113 rule 5: a retain that finds a count at its limit ends the process, on any
        // thread, inside a copy or not, with one line on stderr and no trace: U8001 when the
        // heap and the stacks hold less than such a count needs, U6003 when they hold it. A
        // retain that returned without counting would leave the count one short.
        let cases = [
            ("ffi::tests::i2_count_limit_shipped_scenario", 3, INTERNAL),
            ("ffi::tests::i2_count_limit_in_main_scenario", 3, INTERNAL),
            (
                "ffi::tests::i2_count_limit_with_the_memory_it_needs_scenario",
                1,
                OVER,
            ),
            ("ffi::tests::i2_count_limit_in_a_task_scenario", 3, INTERNAL),
            (
                "ffi::tests::i2_count_limit_during_an_import_scenario",
                3,
                INTERNAL,
            ),
            (
                "ffi::tests::i2_count_limit_in_two_tasks_at_once_scenario",
                3,
                INTERNAL,
            ),
        ];
        for (scenario, code, line) in cases {
            let (status, stderr, traced, stdout) = run_limit_scenario(scenario);
            assert_eq!(status, Some(code), "{scenario}: {stderr}");
            assert_eq!(stderr, format!("{line}\n"), "{scenario}: one line only");
            assert!(!traced, "{scenario}: the stop wrote a trace");
            if scenario.ends_with("shipped_scenario") {
                // The count reached the shipped limit without passing it; the next retain stopped.
                assert!(stdout.contains(AT_LIMIT), "{scenario}: {stdout}");
            }
            if scenario.ends_with("two_tasks_at_once_scenario") {
                // Both tasks reached the limit, and the one that did not end the process parked:
                // one line on stderr, and one stop parked behind it.
                assert_eq!(
                    stdout.matches(BEFORE_LIMIT).count(),
                    2,
                    "{scenario}: {stdout}"
                );
                assert!(
                    stdout.contains("una-rt test: parked stops: 1"),
                    "{scenario}: {stdout}"
                );
            }
        }
    }

    /// A block of one `together for` task over a list of three texts, whose lambda says it ran:
    /// the fork's scenarios, each with a hook set before `main`.
    fn fork_scenario(before: impl FnOnce()) {
        before();
        let marked =
            Code(i64::try_from(marked as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        runtime::start(&[], LIMIT_META, move || {
            let items = values::list();
            let element = values::list();
            for text in ["a", "b", "c"] {
                let word = values::text_word(text.to_owned());
                values::push(element, word);
                values::release(word);
            }
            values::push(items, element);
            values::release(element);
            let closure = values::closure(marked, 1);
            let value = tasks::together_for(closure, items, 0);
            eprintln!("{MARK}: the block returned");
            value
        });
    }

    #[test]
    #[ignore = "run by `i6_i11_a_failed_fork_ends_the_block_and_runs_no_task`"]
    fn i6_fork_spawn_failure_scenario() {
        fork_scenario(|| tasks::TEST_FAIL_SPAWN.store(true, std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    #[ignore = "run by `i6_i11_a_failed_fork_ends_the_block_and_runs_no_task`"]
    fn i6_fork_import_refused_scenario() {
        fork_scenario(|| {
            values::TEST_REFUSE_IMPORT.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    #[test]
    #[ignore = "run by `i6_i11_a_failed_fork_ends_the_block_and_runs_no_task`"]
    fn i11_fork_import_panic_scenario() {
        fork_scenario(|| {
            values::TEST_PANIC_IN_IMPORT.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    #[test]
    #[ignore = "run by `i6_i11_a_failed_fork_ends_the_block_and_runs_no_task`"]
    fn i6_fork_export_limit_scenario() {
        // The element holds four objects, past a limit of 2.
        fork_scenario(|| values::TEST_EXPORT_LIMIT.store(2, std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    #[ignore = "run by `i11_a_panic_in_the_join_s_import_is_u8001_at_the_block`"]
    fn i11_join_import_panic_scenario() {
        fork_scenario(|| {
            tasks::TEST_PANIC_IN_JOIN.store(true, std::sync::atomic::Ordering::SeqCst)
        });
    }

    /// The meta of the scenario of a refused export in a later batch: the block's site, and one
    /// operation that each task records in the trace.
    const BATCH_META: &str = r#"{"grants":[],"uses":[],"sites":[{"file":"src/c.una","start":{"line":2,"col":3},"end":{"line":2,"col":9}}],"ops":[{"capability":"Console","op":"print","params":[],"args":[],"result":{"type":0}}]}"#;

    /// A lambda that records one event of the operation of [`BATCH_META`].
    extern "C" fn evented(_closure: i64, _arg: i64) -> Returned {
        let instance = values::instance("console".to_owned());
        host::event(0, instance);
        values::release(instance);
        Returned {
            value: 0,
            status: 0,
        }
    }

    #[test]
    #[ignore = "run by `i6_a_refused_export_of_a_later_batch_keeps_the_events_of_the_tasks_that_ran`"]
    fn i6_later_batch_export_refused_scenario() {
        // 65 tasks: 64 over integers, which the first batch forks and runs, and a last one over a
        // list of two texts, three objects, past an export limit of 2.
        values::TEST_EXPORT_LIMIT.store(2, std::sync::atomic::Ordering::SeqCst);
        let trace = limit_trace("ffi::tests::i6_later_batch_export_refused_scenario");
        let evented =
            Code(i64::try_from(evented as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        runtime::start(
            &["--trace".to_owned(), trace.to_string_lossy().into_owned()],
            BATCH_META,
            move || {
                let items = values::list();
                for number in 0..64 {
                    values::push(items, (number << 1) | 1);
                }
                let element = values::list();
                for text in ["a", "b"] {
                    let word = values::text_word(text.to_owned());
                    values::push(element, word);
                    values::release(word);
                }
                values::push(items, element);
                values::release(element);
                let closure = values::closure(evented, 1);
                tasks::together_for(closure, items, 0)
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i6_a_refused_export_of_a_later_batch_keeps_the_events_of_the_tasks_that_ran() {
        // ADR 0113 rule 2: a refused export does not end `main` inside the copy. The block's
        // fork turns it into the outcome of the task it was for, and the join adopts the events
        // of the tasks of earlier batches, which ran, before the stop ends the run at the block.
        let scenario = "ffi::tests::i6_later_batch_export_refused_scenario";
        let trace = limit_trace(scenario);
        let _ = std::fs::remove_file(&trace);
        let (status, stderr) = run_scenario(scenario);
        let written = std::fs::read_to_string(&trace).unwrap_or_default();
        let _ = std::fs::remove_file(&trace);
        assert_eq!(status, Some(1), "{stderr}");
        assert!(
            stderr.starts_with("[U6003] Error: this copy holds more objects"),
            "{stderr}"
        );
        assert!(stderr.contains("src/c.una:2:3"), "at the block: {stderr}");
        let events: serde_json::Value = serde_json::from_str(&written).unwrap();
        let tasks: std::collections::BTreeSet<String> = events
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|event| event["task"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(tasks.len(), 64, "the first batch's events: {written}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i11_a_panic_in_the_join_s_import_is_u8001_at_the_block() {
        // ADR 0113 rule 2 and I11: the join's copies run inside `guarded` too, so a panic there
        // ends the run with U8001 at the block, exit code 3, never an abort through the
        // generated code's `extern "C"` frames.
        let (status, stderr) = run_scenario("ffi::tests::i11_join_import_panic_scenario");
        assert_eq!(status, Some(3), "{stderr}");
        // The task ran, and said so, before its value reached the join.
        assert!(stderr.starts_with(MARK), "{stderr}");
        assert!(
            stderr.contains("\n[U8001] Error: internal error: the runtime panicked while it copied a value: a test made the join's import panic\n"),
            "{stderr}"
        );
        assert!(stderr.contains("src/c.una:2:3"), "at the block: {stderr}");
        assert!(!stderr.contains("panicked at"), "{stderr}");
    }

    /// A lambda with no parameter that returns a list of three texts: four objects, past an
    /// export limit of 2, so that a transfer of its value is refused.
    extern "C" fn returns_big(_closure: i64) -> Returned {
        let value = values::list();
        for text in ["a", "b", "c"] {
            let word = values::text_word(text.to_owned());
            values::push(value, word);
            values::release(word);
        }
        Returned { value, status: 0 }
    }

    /// A lambda with no parameter that records one event of [`BATCH_META`]'s operation, then
    /// fails with a small text.
    extern "C" fn fails_small(_closure: i64) -> Returned {
        let instance = values::instance("console".to_owned());
        host::event(0, instance);
        values::release(instance);
        Returned {
            value: values::text_word("small".to_owned()),
            status: 1,
        }
    }

    /// A lambda with no parameter that aborts its task with U6004 at the block's site.
    extern "C" fn aborts(_closure: i64) -> Returned {
        runtime::abort_at(
            0,
            una_diag::Code::U6004,
            "una-rt test: the task aborted".to_owned(),
        );
        Returned {
            value: 0,
            status: 2,
        }
    }

    /// Whether [`waits_for_cancel`] recorded its event, in the process of a scenario.
    static RECORDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// A lambda with no parameter that waits until [`waits_for_cancel`] has started and
    /// recorded its event, then fails as [`fails_small`] does: so the task after it is started
    /// when the failure cancels it, whatever the scheduling.
    extern "C" fn fails_after_recorded(closure: i64) -> Returned {
        while !RECORDED.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        fails_small(closure)
    }

    /// A lambda with no parameter that records one event, then waits until its task is
    /// cancelled, and stops.
    extern "C" fn waits_for_cancel(_closure: i64) -> Returned {
        let instance = values::instance("console".to_owned());
        host::event(0, instance);
        values::release(instance);
        RECORDED.store(true, std::sync::atomic::Ordering::SeqCst);
        while runtime::poll() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Returned {
            value: 0,
            status: 2,
        }
    }

    /// A lambda with no parameter that records one event and returns.
    extern "C" fn records(_closure: i64) -> Returned {
        let instance = values::instance("console".to_owned());
        host::event(0, instance);
        values::release(instance);
        Returned {
            value: values::text_word("r".to_owned()),
            status: 0,
        }
    }

    /// A lambda with no parameter that waits until [`waits_for_cancel`] has started and recorded
    /// its event, then runs [`inner_import_panics`]: so the lower task is started when the
    /// defect cancels it, whatever the scheduling.
    extern "C" fn inner_import_panics_after_recorded(closure: i64) -> Returned {
        while !RECORDED.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        inner_import_panics(closure)
    }

    /// Whether [`inner_import_panics`]'s block ended, in the process of a scenario.
    static INNER_ENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Whether [`fails_after_inner`] started, in the process of a scenario.
    static FIRST_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// A lambda with no parameter that waits until [`fails_after_inner`] has started, then runs
    /// [`inner_import_panics`]: so its defect never cancels that task before it starts.
    extern "C" fn inner_import_panics_after_first(closure: i64) -> Returned {
        while !FIRST_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        inner_import_panics(closure)
    }

    /// A lambda with no parameter that waits until [`inner_import_panics`]'s block ended, then
    /// fails: so the task after it is not cancelled before its defect, whatever the scheduling.
    extern "C" fn fails_after_inner(closure: i64) -> Returned {
        FIRST_STARTED.store(true, std::sync::atomic::Ordering::SeqCst);
        while !INNER_ENDED.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        fails_small(closure)
    }

    /// A lambda with no parameter that runs a block of its own whose fork import panics.
    extern "C" fn inner_import_panics(_closure: i64) -> Returned {
        values::TEST_PANIC_IN_IMPORT.store(true, std::sync::atomic::Ordering::SeqCst);
        let items = values::list();
        values::push(items, 1);
        let marked =
            Code(i64::try_from(marked as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        let closure = values::closure(marked, 1);
        let value = tasks::together_for(closure, items, 0);
        let status = tasks::status();
        // Cleared at once, so that no later copy panics: the defect is this block's alone, and
        // only its precedence can decide the outer block.
        values::TEST_PANIC_IN_IMPORT.store(false, std::sync::atomic::Ordering::SeqCst);
        values::release(closure);
        values::release(items);
        INNER_ENDED.store(true, std::sync::atomic::Ordering::SeqCst);
        Returned { value, status }
    }

    /// The code of a lambda with no parameter.
    fn code0(lambda: extern "C" fn(i64) -> Returned) -> Code {
        Code(i64::try_from(lambda as usize).unwrap())
    }

    /// Runs `main` with a trace for `scenario`: a `together` of the closures of `lambdas`, then
    /// says on stderr how the block ended.
    fn block_scenario(scenario: &str, lambdas: Vec<extern "C" fn(i64) -> Returned>) {
        block_scenario_under(scenario, &[], lambdas);
    }

    /// [`block_scenario`], with the further command-line arguments `args`.
    fn block_scenario_under(
        scenario: &str,
        args: &[&str],
        lambdas: Vec<extern "C" fn(i64) -> Returned>,
    ) {
        let trace = limit_trace(scenario);
        let mut arguments = vec!["--trace".to_owned(), trace.to_string_lossy().into_owned()];
        arguments.extend(args.iter().map(|&arg| arg.to_owned()));
        runtime::start(&arguments, BATCH_META, move || {
            let closures = values::list();
            for lambda in lambdas {
                let closure = values::closure(code0(lambda), 0);
                values::push(closures, closure);
                values::release(closure);
            }
            let value = tasks::together(closures, 0);
            let status = tasks::status();
            if status == 1 {
                eprintln!("{MARK_END}: failed with {}", values::text_value(value));
            } else {
                eprintln!("{MARK_END}: status {status}");
            }
            values::release(value);
            values::release(closures);
            0
        });
    }

    /// What a block scenario's `main` says once its block ended.
    const MARK_END: &str = "una-rt test: the block ended";

    /// Runs a block scenario in a process of its own: its exit code, its stderr, and the tasks
    /// of its trace's events, in order, with `cancelled` after a cancelled task's.
    fn run_block_scenario(scenario: &str) -> (Option<i32>, String, Vec<String>) {
        let trace = limit_trace(scenario);
        let _ = std::fs::remove_file(&trace);
        let (status, stderr) = run_scenario(scenario);
        let written = std::fs::read_to_string(&trace).unwrap_or_default();
        let _ = std::fs::remove_file(&trace);
        let events: serde_json::Value = serde_json::from_str(&written).unwrap_or_default();
        let tasks = events
            .as_array()
            .map(|events| {
                events
                    .iter()
                    .map(|event| {
                        let task = event["task"].as_str().unwrap_or("").to_owned();
                        if event["outcome"]["kind"] == "cancelled" {
                            format!("{task} cancelled")
                        } else {
                            task
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        (status, stderr, tasks)
    }

    #[test]
    #[ignore = "run by `i7_a_deciding_abort_or_failure_copies_no_other_task_s_value`"]
    fn i7_abort_scenario() {
        values::TEST_EXPORT_LIMIT.store(2, std::sync::atomic::Ordering::SeqCst);
        block_scenario("ffi::tests::i7_abort_scenario", vec![returns_big, aborts]);
    }

    #[test]
    #[ignore = "run by `i7_a_deciding_abort_or_failure_copies_no_other_task_s_value`"]
    fn i7_failure_scenario() {
        values::TEST_EXPORT_LIMIT.store(2, std::sync::atomic::Ordering::SeqCst);
        block_scenario(
            "ffi::tests::i7_failure_scenario",
            vec![returns_big, fails_small],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i7_a_deciding_abort_or_failure_copies_no_other_task_s_value() {
        // ADR 0113 rule 3, I7: a block that aborts copies nothing at its join, and a failing
        // block copies only the deciding failure value. The first task's value is past the
        // export limit, so a copy of it would stop the run with that limit's U6003 instead.
        let (status, stderr, _) = run_block_scenario("ffi::tests::i7_abort_scenario");
        assert_eq!(status, Some(1), "{stderr}");
        assert!(
            stderr.starts_with("[U6004] Error: una-rt test: the task aborted"),
            "{stderr}"
        );
        let (status, stderr, _) = run_block_scenario("ffi::tests::i7_failure_scenario");
        assert_eq!(status, Some(0), "{stderr}");
        assert!(
            stderr.contains(&format!("{MARK_END}: failed with small")),
            "{stderr}"
        );
    }

    #[test]
    #[ignore = "run by `i10_a_refused_transfer_stops_at_the_block_with_every_task_s_events`"]
    fn i10_refused_transfer_scenario() {
        values::TEST_EXPORT_LIMIT.store(2, std::sync::atomic::Ordering::SeqCst);
        block_scenario(
            "ffi::tests::i10_refused_transfer_scenario",
            vec![records, returns_big, records],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i10_a_refused_transfer_stops_at_the_block_with_every_task_s_events() {
        // ADR 0113 rule 3: every task returned, the batch's events are adopted first, then the
        // second task's transfer is refused: U6003 at the block, with the events of all three.
        let (status, stderr, tasks) =
            run_block_scenario("ffi::tests::i10_refused_transfer_scenario");
        assert_eq!(status, Some(1), "{stderr}");
        assert!(
            stderr.starts_with("[U6003] Error: this copy holds more objects"),
            "{stderr}"
        );
        assert!(stderr.contains("src/c.una:2:3"), "at the block: {stderr}");
        assert_eq!(
            tasks,
            ["1", "3"],
            "the events of the tasks that recorded one"
        );
    }

    #[test]
    #[ignore = "run by `i10_the_trace_holds_a_cancelled_task_s_events_then_its_cancelled_event`"]
    fn i10_cancelled_task_scenario() {
        block_scenario(
            "ffi::tests::i10_cancelled_task_scenario",
            vec![records, fails_after_recorded, waits_for_cancel],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i10_the_trace_holds_a_cancelled_task_s_events_then_its_cancelled_event() {
        // ADR 0113 I10, `spec/19 §6` rules 3 to 5: task 2 fails, task 3, already started, is
        // cancelled. The trace holds the events of tasks 1, 2 and 3 in task order, then task 3's
        // `cancelled` event, and the block fails with task 2's value.
        let (status, stderr, tasks) = run_block_scenario("ffi::tests::i10_cancelled_task_scenario");
        assert_eq!(status, Some(0), "{stderr}");
        assert!(
            stderr.contains(&format!("{MARK_END}: failed with small")),
            "{stderr}"
        );
        assert_eq!(tasks, ["1", "2", "3", "3 cancelled"]);
    }

    #[test]
    #[ignore = "run by `i11_a_copy_s_panic_decides_its_block_before_a_lower_indexed_failure`"]
    fn i11_outranked_panic_scenario() {
        block_scenario(
            "ffi::tests::i11_outranked_panic_scenario",
            vec![fails_after_inner, inner_import_panics_after_first],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i11_a_copy_s_panic_decides_its_block_before_a_lower_indexed_failure() {
        // ADR 0113 I11: a panic during a copy ends the run with U8001, exit code 3, even when a
        // lower-indexed task of an enclosing block failed: the defect decides the block first.
        let (status, stderr, _) = run_block_scenario("ffi::tests::i11_outranked_panic_scenario");
        assert_eq!(status, Some(3), "{stderr}");
        assert!(
            stderr.starts_with("[U8001] Error: internal error: the runtime panicked while it copied a value: a test made the import panic"),
            "{stderr}"
        );
        assert!(!stderr.contains(MARK_END), "{stderr}");
    }

    /// How many tasks of [`inner_big_after_cancel`]'s block started, in the process of a
    /// scenario.
    static INNER_STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A lambda with one parameter that says it started, waits until its task is cancelled, then
    /// returns a value past an export limit of 2, as a task that ignored the cancellation would.
    extern "C" fn big_after_cancel(closure: i64, _arg: i64) -> Returned {
        INNER_STARTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        while runtime::poll() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        returns_big(closure)
    }

    /// A lambda with no parameter that runs a block of two [`big_after_cancel`] tasks.
    extern "C" fn inner_big_after_cancel(_closure: i64) -> Returned {
        let items = values::list();
        values::push(items, 1);
        values::push(items, 3);
        let code = Code(
            i64::try_from(big_after_cancel as extern "C" fn(i64, i64) -> Returned as usize)
                .unwrap(),
        );
        let closure = values::closure(code, 1);
        let value = tasks::together_for(closure, items, 0);
        let status = tasks::status();
        values::release(closure);
        values::release(items);
        Returned { value, status }
    }

    /// A lambda with no parameter that fails once both of [`inner_big_after_cancel`]'s tasks
    /// started, so that both return their values after the cancellation, whatever the
    /// scheduling.
    extern "C" fn fails_after_inner_started(closure: i64) -> Returned {
        while INNER_STARTED.load(std::sync::atomic::Ordering::SeqCst) < 2 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        fails_small(closure)
    }

    #[test]
    #[ignore = "run by `i7_i10_a_block_whose_own_task_was_cancelled_copies_nothing_and_ends_as_cancelled`"]
    fn i7_cancelled_block_scenario() {
        values::TEST_EXPORT_LIMIT.store(2, std::sync::atomic::Ordering::SeqCst);
        block_scenario(
            "ffi::tests::i7_cancelled_block_scenario",
            vec![fails_after_inner_started, inner_big_after_cancel],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i7_i10_a_block_whose_own_task_was_cancelled_copies_nothing_and_ends_as_cancelled() {
        // ADR 0113 rule 3, I7 and I10: task 1 fails while task 2's inner block runs, which
        // cancels task 2. The inner tasks return values past the export limit, but the inner
        // join sees its block's task cancelled and copies nothing, so no U6003; task 2 ends as
        // cancelled, with its `cancelled` event, and the outer block fails with task 1's value.
        let (status, stderr, tasks) = run_block_scenario("ffi::tests::i7_cancelled_block_scenario");
        assert_eq!(status, Some(0), "{stderr}");
        assert!(!stderr.contains("U6003"), "{stderr}");
        assert!(
            stderr.contains(&format!("{MARK_END}: failed with small")),
            "{stderr}"
        );
        assert_eq!(tasks, ["1", "2 cancelled"]);
    }

    use super::FILLER;

    /// A lambda with one parameter that records one event, says it started, waits until its
    /// task is cancelled, then holds all but 4 bytes of what the budget leaves until its thread
    /// ends, and returns a small text.
    extern "C" fn fills_after_cancel(_closure: i64, _arg: i64) -> Returned {
        let instance = values::instance("console".to_owned());
        host::event(0, instance);
        values::release(instance);
        INNER_STARTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        while runtime::poll() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let room = runtime::test_room().saturating_sub(4);
        FILLER.with(|filler| *filler.borrow_mut() = Vec::with_capacity(room));
        Returned {
            value: values::text_word("full".to_owned()),
            status: 0,
        }
    }

    /// A lambda with no parameter that runs a block of one [`fills_after_cancel`] task.
    extern "C" fn inner_fills_after_cancel(_closure: i64) -> Returned {
        let items = values::list();
        values::push(items, 1);
        let code = Code(
            i64::try_from(fills_after_cancel as extern "C" fn(i64, i64) -> Returned as usize)
                .unwrap(),
        );
        let closure = values::closure(code, 1);
        let value = tasks::together_for(closure, items, 0);
        let status = tasks::status();
        values::release(closure);
        values::release(items);
        Returned { value, status }
    }

    /// A lambda with no parameter that fails, with its event, once the task of
    /// [`inner_fills_after_cancel`]'s block started.
    extern "C" fn fails_after_one_inner_started(closure: i64) -> Returned {
        while INNER_STARTED.load(std::sync::atomic::Ordering::SeqCst) < 1 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        fails_small(closure)
    }

    #[test]
    #[ignore = "run by `i10_a_block_cancelled_near_the_budget_keeps_its_batch_s_events_and_its_cancelled_event`"]
    fn i10_cancelled_block_near_budget_scenario() {
        block_scenario_under(
            "ffi::tests::i10_cancelled_block_near_budget_scenario",
            &["--max-memory", "8"],
            vec![fails_after_one_inner_started, inner_fills_after_cancel],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i10_a_block_cancelled_near_the_budget_keeps_its_batch_s_events_and_its_cancelled_event() {
        // ADR 0113 rule 3 and its Amendment of 2026-10-16, I10: task 1 fails while task 2's
        // inner block runs, which cancels task 2. The inner task then holds all but 4 bytes of
        // the budget's rest in its heap and returns. Its block's own task was cancelled, so its
        // thread exits, its heap freed, before the adoption reserves: the adoption fits, the
        // inner task's event is kept, task 2 ends as cancelled with its `cancelled` event, and
        // the outer block fails with task 1's value.
        let (status, stderr, tasks) =
            run_block_scenario("ffi::tests::i10_cancelled_block_near_budget_scenario");
        assert_eq!(status, Some(0), "{stderr}");
        assert!(!stderr.contains("U6003"), "{stderr}");
        assert!(
            stderr.contains(&format!("{MARK_END}: failed with small")),
            "{stderr}"
        );
        assert_eq!(tasks, ["1", "2.1", "2 cancelled"]);
    }

    #[test]
    #[ignore = "run by `i11_a_defect_ends_the_run_while_a_lower_task_would_run_on`"]
    fn i11_defect_cancels_lower_scenario() {
        block_scenario(
            "ffi::tests::i11_defect_cancels_lower_scenario",
            vec![waits_for_cancel, inner_import_panics_after_recorded],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i11_a_defect_ends_the_run_while_a_lower_task_would_run_on() {
        // ADR 0113 I11: task 1 would run until cancelled; task 2's inner block panics in a copy.
        // The defect cancels task 1, a lower one, so the run ends with U8001, exit code 3, and the
        // trace holds task 1's event and its `cancelled` event.
        let (status, stderr, tasks) =
            run_block_scenario("ffi::tests::i11_defect_cancels_lower_scenario");
        assert_eq!(status, Some(3), "{stderr}");
        assert!(
            stderr.starts_with("[U8001] Error: internal error: the runtime panicked while it copied a value: a test made the import panic"),
            "{stderr}"
        );
        assert_eq!(tasks, ["1", "1 cancelled"]);
    }

    /// How many times [`counted`] ran, in the process of a scenario.
    static COUNTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A lambda with one parameter that counts its runs and fails on the first element.
    extern "C" fn counted(_closure: i64, arg: i64) -> Returned {
        COUNTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if arg == 1 {
            Returned {
                value: values::text_word("first".to_owned()),
                status: 1,
            }
        } else {
            Returned {
                value: 0,
                status: 0,
            }
        }
    }

    #[test]
    #[ignore = "run by `i8_a_failure_in_the_first_batch_leaves_the_later_batches_unforked`"]
    fn i8_first_batch_failure_scenario() {
        let counted =
            Code(i64::try_from(counted as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        runtime::start(&[], LIMIT_META, move || {
            let items = values::list();
            for number in 0..200 {
                values::push(items, (number << 1) | 1);
            }
            let closure = values::closure(counted, 1);
            let value = tasks::together_for(closure, items, 0);
            eprintln!(
                "{MARK_END}: status {} after {} runs",
                tasks::status(),
                COUNTED.load(std::sync::atomic::Ordering::SeqCst)
            );
            values::release(value);
            values::release(closure);
            values::release(items);
            0
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i8_a_failure_in_the_first_batch_leaves_the_later_batches_unforked() {
        // ADR 0113 rules 1 and 3, I8: the first element's task fails, so the block fails with
        // its value and no task of a later batch is forked: at most the first batch's 64 tasks
        // ran, of 200.
        let (status, stderr) = run_scenario("ffi::tests::i8_first_batch_failure_scenario");
        assert_eq!(status, Some(0), "{stderr}");
        let runs: usize = stderr
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{MARK_END}: status 1 after ")))
            .and_then(|rest| rest.strip_suffix(" runs"))
            .and_then(|runs| runs.parse().ok())
            .unwrap_or_else(|| panic!("{stderr}"));
        assert!((1..=64).contains(&runs), "{runs} runs");
    }

    /// The meta of the budget scenarios: the block's site.
    const BUDGET_META: &str = LIMIT_META;

    #[test]
    #[ignore = "run by `i1_i3_a_fork_past_its_budget_stops_with_u6003_and_held_never_passes_the_ceiling`"]
    fn i3_fork_past_budget_scenario() {
        // A closure that holds a list of 400000 integers, forked under `--max-memory 8`: its
        // portable form alone takes 6.4 MB, past what the budget leaves.
        runtime::TEST_PRINT_PEAK.store(true, std::sync::atomic::Ordering::SeqCst);
        let marked =
            Code(i64::try_from(marked as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        runtime::start(
            &["--max-memory".to_owned(), "8".to_owned()],
            BUDGET_META,
            move || {
                let list = values::list();
                for number in 0..400_000 {
                    values::push(list, (number << 1) | 1);
                }
                let closure = values::closure(marked, 1);
                values::push(closure, list);
                values::release(list);
                let items = values::list();
                values::push(items, 1);
                let value = tasks::together_for(closure, items, 0);
                values::release(items);
                values::release(closure);
                value
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i1_i3_a_fork_past_its_budget_stops_with_u6003_and_held_never_passes_the_ceiling() {
        // ADR 0113, I1 and I3: every step of the fork is reserved and checked before it
        // allocates, so the copy that would pass the budget stops with U6003 at the block, before
        // any task runs, and the debug high-water mark of the heap never passed the ceiling.
        let (status, stderr) = run_scenario("ffi::tests::i3_fork_past_budget_scenario");
        assert_eq!(status, Some(1), "{stderr}");
        assert!(
            stderr.starts_with("[U6003] Error: this call exceeds the memory limit of 8 MiB"),
            "{stderr}"
        );
        assert!(!stderr.contains(MARK), "a task ran: {stderr}");
        let (peak, ceiling) = stderr
            .lines()
            .find_map(|line| line.strip_prefix("una-rt test: peak "))
            .and_then(|rest| rest.split_once(" ceiling "))
            .map(|(peak, ceiling)| {
                (
                    peak.parse::<usize>().unwrap(),
                    ceiling.parse::<usize>().unwrap(),
                )
            })
            .unwrap_or_else(|| panic!("{stderr}"));
        assert!(
            peak <= ceiling,
            "the peak {peak} passed the ceiling {ceiling}"
        );
    }

    #[test]
    #[ignore = "run by `i1_a_block_whose_starts_pass_the_budget_stops_with_u6003_before_any_task_runs`"]
    fn i1_starts_past_budget_scenario() {
        // A `together for` of 64 tasks under `--max-memory 1`, with all but 40 KiB of the budget
        // held: each start, and what its thread keeps while the batch forks, eats into the rest.
        let marked =
            Code(i64::try_from(marked as extern "C" fn(i64, i64) -> Returned as usize).unwrap());
        runtime::start(
            &["--max-memory".to_owned(), "1".to_owned()],
            BUDGET_META,
            move || {
                let room = runtime::test_room();
                let filler: Vec<u8> = Vec::with_capacity(room.saturating_sub(40 * 1024));
                let items = values::list();
                for number in 0..64 {
                    values::push(items, (number << 1) | 1);
                }
                let closure = values::closure(marked, 1);
                let value = tasks::together_for(closure, items, 0);
                drop(filler);
                values::release(items);
                values::release(closure);
                value
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i1_a_block_whose_starts_pass_the_budget_stops_with_u6003_before_any_task_runs() {
        // ADR 0113 rules 2 and 5, with a real budget: the block's bookkeeping and its tasks'
        // starts, which the threads keep while the batch forks, pass what the budget leaves; a
        // step is refused, U6003 at the block, and no task runs. (Whether the step refused is a
        // start or the next fork's export depends on how much each start's thread keeps.)
        let (status, stderr) = run_scenario("ffi::tests::i1_starts_past_budget_scenario");
        assert_eq!(status, Some(1), "{stderr}");
        assert!(
            stderr.starts_with("[U6003] Error: this call exceeds the memory limit of 1 MiB"),
            "{stderr}"
        );
        assert!(stderr.contains("src/c.una:2:3"), "at the block: {stderr}");
        assert!(!stderr.contains(MARK), "a task ran: {stderr}");
    }

    #[test]
    #[ignore = "run by `i11_a_panic_outside_a_copy_prints_as_before`"]
    fn i11_panic_outside_a_copy_scenario() {
        runtime::start(&[], LIMIT_META, || {
            let _ = std::thread::spawn(|| {
                #[allow(clippy::panic)]
                {
                    panic!("una-rt test: a panic outside a copy");
                }
            })
            .join();
            0
        });
    }

    /// Runs `scenario` in a process of its own. A scenario that runs past a minute is killed
    /// and fails the test, so a hang never holds the run. Its stderr goes to a file, which a
    /// full pipe cannot block.
    fn run_scenario(scenario: &str) -> (Option<i32>, String) {
        let exe = std::env::current_exe().unwrap();
        let path = exe.with_file_name(format!("{scenario}.stderr"));
        let mut child = Command::new(&exe)
            .args([
                "--ignored",
                "--exact",
                scenario,
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&path).unwrap())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{scenario} ran past a minute");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let stderr = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        (status.code(), stderr)
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i6_i11_a_failed_fork_ends_the_block_and_runs_no_task() {
        // ADR 0113 rule 2: a thread that cannot start, a refused step of an import or of an
        // export, and a panic during a copy each end the block before any task runs, with no
        // thread left waiting: U6003 at the block for the first three, with the message that
        // names the cause, and U8001 for the panic, whose message the panic hook keeps.
        let cases = [
            (
                "ffi::tests::i6_fork_spawn_failure_scenario",
                1,
                "[U6003] Error: a task of `together` could not run",
            ),
            (
                "ffi::tests::i6_fork_import_refused_scenario",
                1,
                "[U6003] Error: this call exceeds the memory limit of 4096 MiB",
            ),
            (
                "ffi::tests::i6_fork_export_limit_scenario",
                1,
                "[U6003] Error: this copy holds more objects than the native runtime copies at once (4294967295)",
            ),
            (
                "ffi::tests::i11_fork_import_panic_scenario",
                3,
                "[U8001] Error: internal error: the runtime panicked while it copied a value: a test made the import panic",
            ),
        ];
        for (scenario, code, first) in cases {
            let (status, stderr) = run_scenario(scenario);
            assert_eq!(status, Some(code), "{scenario}: {stderr}");
            assert!(stderr.starts_with(first), "{scenario}: {stderr}");
            assert!(
                stderr.contains("src/c.una:2:3"),
                "{scenario}: at the block: {stderr}"
            );
            assert!(!stderr.contains(MARK), "{scenario}: a task ran: {stderr}");
            assert!(!stderr.contains("panicked at"), "{scenario}: {stderr}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn i11_a_panic_outside_a_copy_prints_as_before() {
        // ADR 0113 rule 5: the runtime's panic hook keeps a panic's message only inside a copy,
        // and passes every other panic to the hook before it, which prints it.
        let (_, stderr) = run_scenario("ffi::tests::i11_panic_outside_a_copy_scenario");
        assert!(
            stderr.contains("una-rt test: a panic outside a copy"),
            "{stderr}"
        );
    }
}
