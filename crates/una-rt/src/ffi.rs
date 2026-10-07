//! The C ABI boundary between compiled Una code and the runtime (ADR 0058). This is the only
//! module of the repository that may use `unsafe`: it exports the functions the generated code
//! calls, which edition 2024 marks `#[unsafe(no_mangle)]`, and it reads what the generated code
//! passes by pointer. Every `unsafe` block states why it holds; everything else is in
//! [`crate::runtime`] and [`crate::values`], in safe Rust.
//!
//! Values cross the boundary as words (`i64`), as [`crate::values`] describes them.

use std::ffi::{CStr, c_char, c_int};

use crate::{generic, host, runtime, tasks, values};

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

/// `SIGPIPE` and `SIG_IGN`, which are 13 and 1 on both targets (`aarch64-apple-darwin`,
/// `x86_64-unknown-linux-gnu`).
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

/// The operations of [`crate::values`] that return a word, by word.
macro_rules! exported {
    ($($name:ident => $function:path, ($($arg:ident),*);)*) => {$(
        #[doc = concat!("[`", stringify!($function), "`], for the generated code.")]
        #[unsafe(no_mangle)]
        pub extern "C" fn $name($($arg: i64),*) -> i64 {
            $function($($arg),*)
        }
    )*};
}

exported! {
    una_rt_int_add => values::add, (a, b);
    una_rt_int_sub => values::sub, (a, b);
    una_rt_int_mul => values::mul, (a, b);
    una_rt_int_div => values::div, (a, b);
    una_rt_int_mod => values::rem, (a, b);
    una_rt_int_neg => values::neg, (a);
    una_rt_int_compare => values::compare, (a, b);
    una_rt_int_text => values::int_text, (a);
    una_rt_bool_text => values::bool_text, (a);
    una_rt_text_concat => values::concat, (a, b);
    una_rt_text_equal => values::text_equal, (a, b);
    una_rt_text_compare => values::text_compare, (a, b);
    una_rt_text_len => values::text_len, (a);
    una_rt_data => values::data, (tag);
    una_rt_list => values::list, ();
    una_rt_closure => values::closure, (code);
    una_rt_field => values::field, (object, index);
    una_rt_tag => values::tag, (value);
    una_rt_closure_code => values::closure_code, (value);
    una_rt_count => values::count, (list);
    una_rt_list_len => values::list_len, (list);
    una_rt_equal => values::equal, (a, b);
    una_rt_abs => values::abs, (x);
    una_rt_min => values::min, (a, b);
    una_rt_max => values::max, (a, b);
    una_rt_is_digits => values::is_digits, (t);
    una_rt_starts_with => values::starts_with, (t, prefix);
    una_rt_trim => values::trim, (t);
    una_rt_to_int => values::to_int, (t);
    una_rt_is_empty => values::is_empty, (xs);
    una_rt_first => values::first, (xs);
    una_rt_at => values::at, (xs, index);
    una_rt_append => values::append, (xs, item);
    una_rt_contains => values::contains, (xs, item);
    una_rt_set => values::set, (xs, index, item);
    una_rt_append_owned => values::append_owned, (xs, item);
    una_rt_set_owned => values::set_owned, (xs, index, item);
    una_rt_reuse => values::reuse, (old, tag);
    una_rt_sum => values::sum, (xs);
    una_rt_generic_compare => generic::compare, (desc, a, b);
    una_rt_generic_equal => generic::equal, (desc, a, b);
    una_rt_generic_to_int => generic::to_int, (desc, x);
    una_rt_generic_min => generic::min, (desc, a, b);
    una_rt_generic_max => generic::max, (desc, a, b);
    una_rt_generic_contains => generic::contains, (desc, xs, item);
    una_rt_generic_sum => generic::sum, (desc, xs);
    una_rt_generic_sort => generic::sort, (desc, xs);
    una_rt_generic_write => generic::write, (desc, x);
    una_rt_write => runtime::write, (ty, word);
    una_rt_descriptor_composite => generic::composite, (ty, params);
    una_rt_or_else => values::or_else, (o, default);
    una_rt_op_code => values::op_code, (instance, index);
    una_rt_fixed => values::fixed, (op, int, a, b);
    una_rt_range => values::range, (low, high, inclusive);
    una_rt_console_print => host::console_print, (line);
    una_rt_clock_now => host::clock_now, ();
    una_rt_env_get => host::env_get, (name);
    una_rt_files_read => host::files_read, (path);
    una_rt_files_write => host::files_write, (path, content);
    una_rt_keys_hmac => host::keys_hmac, (key_name, data);
    una_rt_host_failed => host::host_failed, ();
    una_rt_host_instance => host::host_instance, (capability);
    una_rt_handler_instance => host::handler_instance, (name);
    una_rt_event => host::event, (site, instance);
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

/// Checks the memory budget at an iteration of the loop at `site` (U6003 over it): 1 when
/// the task must stop instead ([`runtime::budget`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_budget(site: i64) -> i64 {
    runtime::budget(usize::try_from(site).unwrap_or(usize::MAX))
}

/// 1 when the task must stop before an operation ([`runtime::poll`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_poll() -> i64 {
    runtime::poll()
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

/// How the last block of tasks ended ([`tasks::status`]).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_together_status() -> i64 {
    tasks::status()
}

/// What compiled code returns: its value, and 0, 1 when the value is a failure, or 2 when the
/// task stopped (`spec/16 §4` rule 3). Two words, which the C calling conventions of both
/// targets return in two registers, as Cranelift returns a body's two results.
#[repr(C)]
struct Returned {
    value: i64,
    status: i64,
}

/// Calls the closure `closure`, whose code is at `code`, with `arg` when it takes one, on the
/// thread of a task ([`tasks::Invoke`]).
pub(crate) fn invoke(code: i64, closure: i64, arg: Option<i64>) -> (i64, i64) {
    if code == 0 {
        return (0, 2);
    }
    let address = code as usize as *const ();
    let returned = match arg {
        None => {
            // SAFETY: `code` is the address of a compiled lambda that `una_rt_closure` stored
            // in the closure: generated code with the default calling convention, that takes
            // the closure and returns two words (`Emitter::lambda`, with no parameter).
            let function: extern "C" fn(i64) -> Returned = unsafe { std::mem::transmute(address) };
            function(closure)
        }
        Some(arg) => {
            // SAFETY: as above, for a lambda with one parameter (`together for`).
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
