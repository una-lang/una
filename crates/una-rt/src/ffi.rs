//! The C ABI boundary between compiled Una code and the runtime (ADR 0058). This is the only
//! module of the repository that may use `unsafe`: it exports the functions the generated code
//! calls, which edition 2024 marks `#[unsafe(no_mangle)]`, and it reads what the generated code
//! passes by pointer. Every `unsafe` block states why it holds; everything else is in
//! [`crate::runtime`] and [`crate::values`], in safe Rust.
//!
//! Values cross the boundary as words (`i64`), as [`crate::values`] describes them.

use std::ffi::{CStr, c_char, c_int};

use crate::{runtime, values};

/// Starts the executable: the generated `main` calls it with the process arguments, the
/// embedded description of the program (`meta`, `meta_len` bytes of UTF-8 JSON) and the
/// compiled `main` of Una (`entry`), and returns its result as the exit code.
///
/// # Safety
///
/// `argv` holds `argc` pointers to NUL-terminated strings, as the C runtime passes them to
/// `main`, and `meta` points to `meta_len` bytes that live for the whole process.
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
    runtime::start(&args, &meta, move || entry())
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
    values::int_literal(digits as usize, &text)
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
    values::text_literal(bytes as usize, &text)
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
    una_rt_sum => values::sum, (xs);
    una_rt_or_else => values::or_else, (o, default);
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

/// Enters the call at `site` (U6003 past the maximum depth).
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_enter(site: i64) {
    runtime::enter(usize::try_from(site).unwrap_or(usize::MAX));
}

/// Leaves a call.
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_leave() {
    runtime::leave();
}

/// `Console.print(line)`, with the line as a text handle.
#[unsafe(no_mangle)]
pub extern "C" fn una_rt_console_print(line: i64) {
    runtime::console_print(line);
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
