//! The C ABI boundary between compiled Una code and the runtime (ADR 0058). This is the only
//! module of the repository that may use `unsafe`: it exports the functions the generated code
//! calls, which edition 2024 marks `#[unsafe(no_mangle)]`, and it reads what the generated code
//! passes by pointer. Every `unsafe` block states why it holds; everything else is in
//! [`crate::runtime`], in safe Rust.

use std::ffi::{CStr, c_char, c_int};

use crate::runtime;

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
    runtime::start(&args, &meta, &|| entry())
}

/// `Console.print(line)`: the generated code passes the line as `len` bytes of UTF-8 at `line`.
///
/// # Safety
///
/// `line` points to `len` bytes that stay valid during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn una_rt_console_print(line: *const u8, len: usize) {
    // SAFETY: the caller passes `len` bytes at `line` (this function's contract).
    if let Some(line) = unsafe { text(line, len) } {
        runtime::console_print(&line);
    }
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
