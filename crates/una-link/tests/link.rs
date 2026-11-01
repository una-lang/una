//! The internal linker on hand-made inputs (ADR 0124 rules 23, 24, 25, 27, 29 and 34): links
//! that run, the must-reject cases and the canaries that fail closed with their diagnostic, and
//! determinism. Running an executable and reading it with the system's tools (`codesign`,
//! `otool`, `lldb`) happens on `aarch64-apple-darwin` only; the linker itself never runs them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod common;

use common::*;
use una_link::{Code, Options, link};

fn options(identifier: &str) -> Options<'_> {
    Options {
        identifier,
        debuggable: false,
    }
}

/// A program whose `main` returns `code`.
fn returning(code: u16) -> Object {
    Object {
        sections: vec![text(&[mov_w0(code), RET])],
        symbols: vec![global("_main", 1, 0)],
    }
}

/// A member defining `name`, which returns `code`.
fn function(name: &str, code: u16) -> Object {
    Object {
        sections: vec![text(&[mov_w0(code), RET])],
        symbols: vec![global(name, 1, 0)],
    }
}

/// A program whose `main` calls `callee` and returns its result.
fn calling(callee: &str) -> Object {
    let mut code = text(&[PROLOGUE[0], PROLOGUE[1], BL, EPILOGUE[0], EPILOGUE[1]]);
    code.relocs.push(reloc(8, 1, BRANCH26));
    Object {
        sections: vec![code],
        symbols: vec![global("_main", 1, 0), undefined(callee)],
    }
}

fn runtime() -> Vec<u8> {
    archive(&[("unused.o", &function("_unused", 1))])
}

fn failure(program: &Object, runtime: &[u8]) -> una_link::Problem {
    match link(&program.bytes(), runtime, &options("t")) {
        Ok(_) => panic!("the link succeeded"),
        Err(problem) => problem,
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod running {
    use std::path::PathBuf;
    use std::process::Command;

    /// Writes `bytes` as an executable in a fresh directory and returns its path.
    pub fn write(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("una-link-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        std::os::unix::fs::PermissionsExt::set_mode(
            &mut std::fs::metadata(&path).unwrap().permissions(),
            0o755,
        );
        let status = Command::new("chmod")
            .arg("755")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    /// `codesign --verify --strict` accepts the file.
    pub fn signed(path: &PathBuf) {
        let out = Command::new("codesign")
            .args(["--verify", "--strict", "-v"])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn a_program_runs_and_its_exit_status_is_main_s_result() {
    let bytes = link(&returning(7).bytes(), &runtime(), &options("seven")).unwrap();
    let path = running::write("seven", &bytes);
    running::signed(&path);
    let status = std::process::Command::new(&path).status().unwrap();
    assert_eq!(status.code(), Some(7));
}

#[test]
fn a_call_loads_the_member_that_defines_it_and_no_other() {
    let helper = function("_helper", 42);
    let mut other = function("_other", 3);
    other
        .sections
        .push(data("__TEXT", "__const", 0, b"UNUSED-MARKER"));
    other.symbols.push(local("_marker", 2, 0));
    let runtime = archive(&[("other.o", &other), ("helper.o", &helper)]);
    let bytes = link(&calling("_helper").bytes(), &runtime, &options("helper")).unwrap();
    assert!(!bytes.windows(13).any(|w| w == b"UNUSED-MARKER"));
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let path = running::write("helper", &bytes);
        running::signed(&path);
        assert_eq!(
            std::process::Command::new(&path).status().unwrap().code(),
            Some(42)
        );
    }
}

#[test]
fn an_unreachable_function_of_a_loaded_member_is_removed() {
    // The member defines `_helper` and `_dead` in one section; only `_helper` is reached.
    let mut member = Object {
        sections: vec![text(&[
            mov_w0(42),
            RET,
            0xd420_0000,
            0xd420_0000,
            0xd420_0000,
            RET,
        ])],
        symbols: vec![global("_helper", 1, 0), global("_dead", 1, 8)],
    };
    member.sections[0].data.extend_from_slice(&[0; 0]);
    let runtime = archive(&[("helper.o", &member)]);
    let bytes = link(&calling("_helper").bytes(), &runtime, &options("strip")).unwrap();
    let brk = 0xd420_0000u32.to_le_bytes();
    let run = [brk, brk, brk].concat();
    assert!(
        !bytes.windows(12).any(|w| w == run.as_slice()),
        "_dead was kept"
    );
}

/// The program of `hi`: `write(1, "hi\n", 3)` twice, once through a stub and a pointer in
/// `__DATA` (a rebase), once through the GOT (a bind), then returns 0.
fn hi() -> Object {
    let mut code = text(&[
        0xa9bf_7bfd, // stp x29, x30, [sp, #-16]!
        0x9100_03fd, // mov x29, sp
        0x5280_0020, // mov w0, #1
        0x9000_0008, // adrp x8, _ptr@PAGE
        0xf940_0101, // ldr x1, [x8, _ptr@PAGEOFF]
        0xd280_0062, // mov x2, #3
        BL,          // bl _write
        0x9000_0010, // adrp x16, _write@GOTPAGE
        0xf940_0210, // ldr x16, [x16, _write@GOTPAGEOFF]
        0x5280_0020, // mov w0, #1
        0xd280_0062, // mov x2, #3
        0x9000_0001, // adrp x1, l_str@PAGE
        0x9100_0021, // add x1, x1, l_str@PAGEOFF
        0xd63f_0200, // blr x16
        0x5280_0000, // mov w0, #0
        0xa8c1_7bfd, // ldp x29, x30, [sp], #16
        RET,
    ]);
    code.relocs = vec![
        reloc(12, 3, PAGE21),
        reloc(16, 3, PAGEOFF12),
        reloc(24, 1, BRANCH26),
        reloc(28, 1, GOT_LOAD_PAGE21),
        reloc(32, 1, GOT_LOAD_PAGEOFF12),
        reloc(44, 2, PAGE21),
        reloc(48, 2, PAGEOFF12),
    ];
    let string = Section {
        align: 0,
        ..data("__TEXT", "__cstring", 0x2, b"hi\n\0")
    };
    let mut pointer = data("__DATA", "__data", 0, &[0; 8]);
    pointer.relocs.push(reloc(0, 2, UNSIGNED));
    Object {
        sections: vec![code, string, pointer],
        symbols: vec![
            global("_main", 1, 0),
            undefined("_write"),
            local("l_str", 2, 0),
            local("_ptr", 3, 0),
        ],
    }
}

#[test]
fn imports_are_bound_through_stubs_and_the_got_and_pointers_rebased() {
    let bytes = link(&hi().bytes(), &runtime(), &options("hi")).unwrap();
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let path = running::write("hi", &bytes);
        running::signed(&path);
        let out = std::process::Command::new(&path).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\nhi\n");
        let fixups = std::process::Command::new("xcrun")
            .args(["dyld_info", "-fixups"])
            .arg(&path)
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&fixups.stdout);
        assert!(listing.contains("bind  libSystem/_write"), "{listing}");
        assert!(listing.contains("rebase"), "{listing}");
        let otool = std::process::Command::new("otool")
            .arg("-l")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            otool.status.success(),
            "{}",
            String::from_utf8_lossy(&otool.stderr)
        );
        let commands = String::from_utf8_lossy(&otool.stdout);
        for kind in [
            "LC_DYLD_CHAINED_FIXUPS",
            "LC_MAIN",
            "LC_UUID",
            "LC_BUILD_VERSION",
            "LC_CODE_SIGNATURE",
        ] {
            assert!(commands.contains(kind), "{kind}: {commands}");
        }
        assert!(commands.contains("minos 13.0"), "{commands}");
        assert!(!commands.contains("LC_RPATH") && !commands.contains("LC_SOURCE_VERSION"));
    }
    let _ = bytes;
}

/// The load commands of an executable, by kind, and the fields the tests read.
struct Commands {
    kinds: Vec<u32>,
    chained_at: usize,
    uuid_at: usize,
    signature_at: usize,
    flags: u32,
    segments: Vec<(String, u32, u32)>,
    segment_at: Vec<usize>,
}

fn commands(bytes: &[u8]) -> Commands {
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let count = word(16) as usize;
    let mut at = 32;
    let mut out = Commands {
        kinds: Vec::new(),
        chained_at: 0,
        uuid_at: 0,
        signature_at: 0,
        flags: word(24),
        segments: Vec::new(),
        segment_at: Vec::new(),
    };
    for _ in 0..count {
        let (cmd, size) = (word(at), word(at + 4) as usize);
        out.kinds.push(cmd);
        match cmd {
            0x1b => out.uuid_at = at + 8,
            0x8000_0034 => out.chained_at = word(at + 8) as usize,
            0x1d => out.signature_at = word(at + 8) as usize,
            0x19 => {
                let name = String::from_utf8_lossy(&bytes[at + 8..at + 24])
                    .trim_end_matches('\0')
                    .to_owned();
                out.segments.push((name, word(at + 56), word(at + 68)));
                out.segment_at.push(at);
            }
            _ => {}
        }
        at += size;
    }
    out
}

#[test]
fn the_load_commands_are_those_of_rule_25_and_the_segments_are_hardened() {
    let bytes = link(&hi().bytes(), &runtime(), &options("hi")).unwrap();
    let parsed = commands(&bytes);
    let expected = [
        0x19,
        0x19,
        0x19,
        0x19,
        0x19,
        0x8000_0034,
        0x8000_0033,
        0x2,
        0xb,
        0xe,
        0x1b,
        0x32,
        0x8000_0028,
        0xc,
        0x26,
        0x29,
        0x1d,
    ];
    assert_eq!(parsed.kinds, expected);
    // MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_PIE; never MH_ALLOW_STACK_EXECUTION.
    assert_eq!(parsed.flags & 0x0020_0085, 0x0020_0085);
    assert_eq!(parsed.flags & 0x2_0000, 0);
    for (name, prot, flags) in &parsed.segments {
        assert!(prot & 0x6 != 0x6, "{name} is writable and executable");
        if name == "__DATA_CONST" {
            assert_eq!(flags & 0x10, 0x10, "__DATA_CONST is SG_READ_ONLY");
        }
    }
    assert!(!bytes.windows(8).any(|w| w == b"__DWARF\0"));
    assert!(!bytes.windows(9).any(|w| w == b"/Users/\0\0"[..9].as_ref()));
}

#[test]
fn the_same_inputs_give_the_same_bytes_and_a_name_changes_only_the_signature() {
    let program = hi().bytes();
    let runtime = runtime();
    let first = link(&program, &runtime, &options("one")).unwrap();
    let again = link(&program, &runtime, &options("one")).unwrap();
    assert_eq!(first, again);
    let other = link(&program, &runtime, &options("two")).unwrap();
    assert_eq!(first.len(), other.len());
    let parsed = commands(&first);
    for (at, (a, b)) in first.iter().zip(&other).enumerate() {
        if a != b {
            assert!(
                at >= parsed.signature_at,
                "byte {at:#x} differs outside the signature"
            );
        }
    }
    assert_ne!(first[parsed.signature_at..], other[parsed.signature_at..]);
    // Rule 27: the UUID is the digest of the file with the UUID and the signature zeroed, so the
    // identifier, inside the signature, does not change it.
    assert_eq!(
        first[parsed.uuid_at..parsed.uuid_at + 16],
        other[parsed.uuid_at..parsed.uuid_at + 16]
    );
    // A version-4 UUID.
    assert_eq!(first[parsed.uuid_at + 6] >> 4, 4);
}

#[test]
fn an_undefined_symbol_outside_libsystem_is_u8001_naming_it() {
    let problem = failure(&calling("_no_such_function"), &runtime());
    assert_eq!(problem.code, Code::U8001);
    assert!(
        problem.message.contains("_no_such_function"),
        "{}",
        problem.message
    );
    assert!(problem.message.contains("libSystem"), "{}", problem.message);
    // A symbol that libSystem exports only after macOS 13 is refused as well.
    let problem = failure(&calling("_os_sync_wait_on_address"), &runtime());
    assert_eq!(problem.code, Code::U8001, "{}", problem.message);
}

#[test]
fn a_duplicate_strong_symbol_is_u8001_naming_both_inputs() {
    let mut program = calling("_helper");
    program.sections.push(text(&[RET]));
    program.symbols.push(global("_twice", 2, 0));
    let mut member = function("_helper", 1);
    member.symbols.push(global("_twice", 1, 4));
    member.sections[0]
        .data
        .extend_from_slice(&RET.to_le_bytes());
    let runtime = archive(&[("helper.o", &member)]);
    let problem = failure(&program, &runtime);
    assert_eq!(problem.code, Code::U8001);
    assert!(problem.message.contains("_twice"), "{}", problem.message);
    assert!(
        problem.message.contains("the program's object"),
        "{}",
        problem.message
    );
    assert!(problem.message.contains("helper.o"), "{}", problem.message);
}

#[test]
fn an_input_that_defines_the_header_symbol_is_u8001() {
    let mut program = returning(0);
    program.symbols.push(global("__mh_execute_header", 1, 0));
    let problem = failure(&program, &runtime());
    assert_eq!(problem.code, Code::U8001);
    assert!(
        problem.message.contains("__mh_execute_header"),
        "{}",
        problem.message
    );
}

#[test]
fn an_unsupported_relocation_type_is_u8001_naming_it() {
    let mut program = calling("_helper");
    program.sections[0].relocs[0].kind = 11;
    let problem = failure(&program, &archive(&[("helper.o", &function("_helper", 1))]));
    assert_eq!(problem.code, Code::U8001);
    assert!(
        problem
            .message
            .contains("ARM64_RELOC_AUTHENTICATED_POINTER"),
        "{}",
        problem.message
    );
    assert!(
        problem.message.contains("__TEXT,__text"),
        "{}",
        problem.message
    );
}

#[test]
fn corrupted_inputs_are_u8001() {
    let program = returning(0).bytes();
    for broken in [&program[..40], b"not an object at all".as_slice(), &[][..]] {
        let problem = link(broken, &runtime(), &options("t")).unwrap_err();
        assert_eq!(problem.code, Code::U8001, "{}", problem.message);
    }
    let problem = link(&program, b"!<arch>\ngarbage", &options("t")).unwrap_err();
    assert_eq!(problem.code, Code::U8001);
    // Every single-byte change of a valid object either links or fails with a diagnostic.
    for at in (0..program.len()).step_by(3) {
        let mut mutated = program.clone();
        mutated[at] ^= 0xa5;
        let _ = link(&mutated, &runtime(), &options("t"));
    }
}

#[test]
fn an_absolute_pointer_in_text_is_u8001() {
    let mut program = returning(0);
    let mut constant = data("__TEXT", "__const", 0, &[0; 8]);
    constant.relocs.push(reloc(0, 0, UNSIGNED));
    program.sections.push(constant);
    program.symbols.push(local("_table", 2, 0));
    // `main` reaches the table, so it is live.
    program.sections[0] = text(&[0x9000_0008, 0x9100_0108, mov_w0(0), RET]);
    program.sections[0].relocs = vec![reloc(0, 1, PAGE21), reloc(4, 1, PAGEOFF12)];
    let problem = failure(&program, &runtime());
    assert_eq!(problem.code, Code::U8001);
    assert!(problem.message.contains("__TEXT"), "{}", problem.message);
}

#[test]
fn an_unaligned_pointer_in_data_is_u8001() {
    let mut program = hi();
    program.sections[2] = data("__DATA", "__data", 0, &[0; 12]);
    program.sections[2].relocs.push(reloc(4, 2, UNSIGNED));
    let problem = failure(&program, &runtime());
    assert_eq!(problem.code, Code::U8001);
    assert!(problem.message.contains("8-aligned"), "{}", problem.message);
}

#[test]
fn a_relocation_past_its_section_is_u8001() {
    // The canary of rule 34: a place 8 bytes past the end of its section.
    let mut program = calling("_helper");
    let end = program.sections[0].data.len() as u32;
    program.sections[0].relocs[0].address = end + 8;
    let problem = failure(&program, &archive(&[("helper.o", &function("_helper", 1))]));
    assert_eq!(problem.code, Code::U8001);
    assert!(
        problem.message.contains("outside its section"),
        "{}",
        problem.message
    );
}

#[test]
fn a_symbol_past_its_section_is_u8001() {
    let mut program = returning(0);
    program.symbols.push(local("_beyond", 1, 64));
    let problem = failure(&program, &runtime());
    assert_eq!(problem.code, Code::U8001);
    assert!(problem.message.contains("_beyond"), "{}", problem.message);
}

#[test]
fn a_branch_beyond_128_mib_is_u7015() {
    let far = 130u64 << 20;
    let mut code = text(&[BL]);
    code.data.resize(far as usize, 0);
    code.data.extend_from_slice(&RET.to_le_bytes());
    code.relocs.push(reloc(0, 1, BRANCH26));
    let program = Object {
        sections: vec![code],
        symbols: vec![global("_main", 1, 0), global("_far", 1, far)],
    };
    let problem = failure(&program, &runtime());
    assert_eq!(problem.code, Code::U7015, "{}", problem.message);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn the_hardened_runtime_refuses_injected_libraries_and_debuggers_unless_debuggable() {
    use std::process::{Command, Stdio};
    let program = hi().bytes();
    let plain = running::write(
        "plain",
        &link(&program, &runtime(), &options("plain")).unwrap(),
    );
    let debuggable = running::write(
        "debuggable",
        &link(
            &program,
            &runtime(),
            &Options {
                identifier: "debuggable",
                debuggable: true,
            },
        )
        .unwrap(),
    );
    running::signed(&plain);
    running::signed(&debuggable);
    // libgmalloc prints a banner when it is loaded; the hardened runtime does not load it.
    let injected = Command::new(&plain)
        .env("DYLD_INSERT_LIBRARIES", "/usr/lib/libgmalloc.dylib")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&injected.stdout), "hi\nhi\n");
    assert!(!String::from_utf8_lossy(&injected.stderr).contains("GuardMalloc"));
    // The signature's layout: one CodeDirectory with adhoc, runtime and linker-signed; the
    // entitlements only with `debuggable`.
    let shown = |path: &std::path::Path| {
        let out = Command::new("codesign")
            .args(["-d", "-vvv", "--entitlements", "-"])
            .arg(path)
            .output()
            .unwrap();
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    let plain_shown = shown(&plain);
    assert!(
        plain_shown.contains("flags=0x30002(adhoc,runtime,linker-signed)"),
        "{plain_shown}"
    );
    assert!(plain_shown.contains("v=20500"), "{plain_shown}");
    assert!(!plain_shown.contains("get-task-allow"), "{plain_shown}");
    assert!(shown(&debuggable).contains("com.apple.security.get-task-allow"));
    // lldb is refused at once without the entitlement.
    let mut lldb = Command::new("lldb")
        .args(["--batch", "-o", "process launch"])
        .arg(&plain)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while lldb.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = lldb.kill();
    let out = lldb.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("Not allowed to attach"), "{text}");
}

/// The canaries of rule 34 that concern an executable: each must make the verifier refuse it.
#[test]
fn the_verifier_refuses_each_canary() {
    let bytes = link(&hi().bytes(), &runtime(), &options("canary")).unwrap();
    assert_eq!(una_link::verify(&bytes, false), Ok(()));
    let parsed = commands(&bytes);
    let refused = |mutated: &[u8], why: &str| {
        let problem = una_link::verify(mutated, false).unwrap_err();
        assert_eq!(problem.code, Code::U8001);
        assert!(problem.message.contains(why), "{why}: {}", problem.message);
    };
    // A segment both writable and executable.
    let data = parsed
        .segments
        .iter()
        .position(|(name, _, _)| name == "__DATA")
        .unwrap();
    let mut wx = bytes.clone();
    wx[parsed.segment_at[data] + 60..parsed.segment_at[data] + 64]
        .copy_from_slice(&7u32.to_le_bytes());
    refused(&wx, "writable and executable");
    // An executable stack.
    let mut stack = bytes.clone();
    let flags = parsed.flags | 0x2_0000;
    stack[24..28].copy_from_slice(&flags.to_le_bytes());
    refused(&stack, "stack");
    // One wrong page hash: the last byte of the signature's first code hash.
    let mut hash = bytes.clone();
    let cd = parsed.signature_at + 20;
    let hash_offset = u32::from_be_bytes(hash[cd + 16..cd + 20].try_into().unwrap()) as usize;
    hash[cd + hash_offset] ^= 1;
    refused(&hash, "hash of page 0");
    // An import outside libSystem's list.
    let mut import = bytes.clone();
    let at = parsed.chained_at
        + import[parsed.chained_at..]
            .windows(7)
            .position(|w| w == b"_write\0")
            .unwrap();
    import[at + 3] = b'l';
    refused(&import, "export list");
    // A __DATA_CONST that is writable after the fixups.
    let constant = parsed
        .segments
        .iter()
        .position(|(name, _, _)| name == "__DATA_CONST")
        .unwrap();
    let mut writable = bytes.clone();
    writable[parsed.segment_at[constant] + 68] = 0;
    refused(&writable, "SG_READ_ONLY");
}

#[test]
fn an_input_with_a_writable_code_section_is_u8001() {
    let mut program = returning(0);
    program.sections.push(Section {
        flags: 0x8000_0400,
        ..data("__DATA", "__code", 0, &RET.to_le_bytes())
    });
    let problem = failure(&program, &runtime());
    assert_eq!(problem.code, Code::U8001);
    assert!(
        problem.message.contains("__DATA,__code"),
        "{}",
        problem.message
    );
}

#[test]
fn input_sections_that_overlap_are_u8001() {
    let mut program = returning(0);
    program.sections.push(data("__DATA", "__data", 0, &[0; 8]));
    let mut bytes = program.bytes();
    // The second section's address: the first's (the 72-byte segment command, then the section).
    let second = 32 + 72 + 80 + 32;
    bytes[second..second + 8].copy_from_slice(&0u64.to_le_bytes());
    let problem = link(&bytes, &runtime(), &options("t")).unwrap_err();
    assert_eq!(problem.code, Code::U8001);
    assert!(problem.message.contains("overlap"), "{}", problem.message);
}

/// A program whose `main` takes the address of `_big`, the start of a zero-fill `__bss` of 16
/// bytes, so the section is live; `patch` then rewrites the object's section headers.
fn with_bss(patch: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
    let mut code = text(&[0x9000_0008, 0x9100_0108, mov_w0(0), RET]);
    code.relocs = vec![reloc(0, 1, PAGE21), reloc(4, 1, PAGEOFF12)];
    let bss = Section {
        zerofill: 16,
        ..data("__DATA", "__bss", 0x1, &[])
    };
    let mut bytes = Object {
        sections: vec![code, bss],
        symbols: vec![global("_main", 1, 0), local("_big", 2, 0)],
    }
    .bytes();
    patch(&mut bytes);
    bytes
}

/// The offset of a field of section `index`'s header in a [`common::Object`]'s bytes.
fn section_field(index: usize, field: usize) -> usize {
    32 + 72 + 80 * index + field
}

#[test]
fn a_section_too_large_or_out_of_reach_is_refused_without_wrapping() {
    assert!(link(&with_bss(|_| {}), &runtime(), &options("t")).is_ok());
    for size in [0xffff_ffff_ffff_fff0u64, u64::MAX, 1 << 32] {
        let bytes = with_bss(|b| {
            b[section_field(1, 40)..section_field(1, 48)].copy_from_slice(&size.to_le_bytes())
        });
        let problem = link(&bytes, &runtime(), &options("t")).unwrap_err();
        assert_eq!(problem.code, Code::U8001, "{size:#x}: {}", problem.message);
        assert!(problem.message.contains("4 GiB"), "{}", problem.message);
    }
    // Two sections at the top of the address space, which a sum would wrap.
    let bytes = with_bss(|b| {
        b[section_field(0, 32)..section_field(0, 40)]
            .copy_from_slice(&0x8000_0000_0000_0000u64.to_le_bytes());
        b[section_field(1, 32)..section_field(1, 40)]
            .copy_from_slice(&0x8000_0000_0000_0010u64.to_le_bytes());
    });
    let problem = link(&bytes, &runtime(), &options("t")).unwrap_err();
    assert_eq!(problem.code, Code::U8001, "{}", problem.message);
    // A zero-fill section within 4 GiB that does not fit the image beside the rest is U7015.
    let bytes = with_bss(|b| {
        b[section_field(1, 40)..section_field(1, 48)].copy_from_slice(&0xffff_ff00u64.to_le_bytes())
    });
    let problem = link(&bytes, &runtime(), &options("t")).unwrap_err();
    assert_eq!(problem.code, Code::U7015, "{}", problem.message);
}

/// The verifier's other refusals (rule 34), each on one mutation of a valid executable.
#[test]
fn the_verifier_refuses_every_structural_canary() {
    let bytes = link(&hi().bytes(), &runtime(), &options("canary")).unwrap();
    let parsed = commands(&bytes);
    let word = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let quad = |b: &[u8], at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
    let refused = |mutated: &[u8], why: &str| {
        let problem = una_link::verify(mutated, false).unwrap_err();
        assert_eq!(problem.code, Code::U8001);
        assert!(problem.message.contains(why), "{why}: {}", problem.message);
    };
    // The offset of the load command of kind `cmd`.
    let command = |cmd: u32| {
        let mut at = 32;
        loop {
            if word(&bytes, at) == cmd {
                return at;
            }
            at += word(&bytes, at + 4) as usize;
        }
    };
    let segment_fileoff = |name: &str| {
        let index = parsed
            .segments
            .iter()
            .position(|(n, _, _)| n == name)
            .unwrap();
        quad(&bytes, parsed.segment_at[index] + 40) as usize
    };
    // Not position-independent.
    let mut b = bytes.clone();
    let flags = parsed.flags & !0x20_0000;
    b[24..28].copy_from_slice(&flags.to_le_bytes());
    refused(&b, "position-independent");
    // A second library: LC_DATA_IN_CODE turned into an LC_LOAD_DYLIB.
    let mut b = bytes.clone();
    let at = command(0x29);
    b[at..at + 4].copy_from_slice(&0xcu32.to_le_bytes());
    refused(&b, "load");
    // An unknown load command.
    let mut b = bytes.clone();
    b[at..at + 4].copy_from_slice(&0x99u32.to_le_bytes());
    refused(&b, "outside rule 25");
    // The first GOT slot, a bind to `_write`: an ordinal that names no import; a chain that
    // leaves its page; a chain to a place that is not 8-aligned.
    let got = segment_fileoff("__DATA_CONST");
    let slot = quad(&bytes, got);
    assert_eq!(slot >> 63, 1, "the first GOT slot is a bind");
    let mut b = bytes.clone();
    b[got..got + 8].copy_from_slice(&((slot & !0xff_ffff) | 7).to_le_bytes());
    refused(&b, "does not exist");
    let mut b = bytes.clone();
    b[got..got + 8].copy_from_slice(&((slot & !(0xfff << 51)) | (0xfff << 51)).to_le_bytes());
    refused(&b, "outside its page or not 8-aligned");
    let mut b = bytes.clone();
    b[got..got + 8].copy_from_slice(&((slot & !(0xfff << 51)) | (1 << 51)).to_le_bytes());
    refused(&b, "outside its page or not 8-aligned");
    // The pointer of `__DATA`, a rebase: a target outside the image.
    let data = segment_fileoff("__DATA");
    let pointer = quad(&bytes, data);
    assert_eq!(pointer >> 63, 0, "the first pointer of __DATA is a rebase");
    let mut b = bytes.clone();
    b[data..data + 8].copy_from_slice(&((pointer & !((1 << 36) - 1)) | 0x10).to_le_bytes());
    refused(&b, "outside the image");
    // A UUID that is not the file's digest.
    let mut b = bytes.clone();
    b[parsed.uuid_at] ^= 1;
    refused(&b, "UUID");
    // A signature that is not the file's last bytes.
    let mut b = bytes.clone();
    b.extend_from_slice(&[0; 16]);
    refused(&b, "last bytes");
    // A code limit that is not the signature's offset, and an executable segment that is not
    // __TEXT.
    let cd = parsed.signature_at + 20;
    let mut b = bytes.clone();
    let limit = u32::from_be_bytes(b[cd + 32..cd + 36].try_into().unwrap()) - 4096;
    b[cd + 32..cd + 36].copy_from_slice(&limit.to_be_bytes());
    refused(&b, "code limit");
    let mut b = bytes.clone();
    let exec = u64::from_be_bytes(b[cd + 72..cd + 80].try_into().unwrap()) + 0x4000;
    b[cd + 72..cd + 80].copy_from_slice(&exec.to_be_bytes());
    refused(&b, "executable segment");
    // An identifier that is not a name inside the CodeDirectory.
    let mut b = bytes.clone();
    b[cd + 20..cd + 24].copy_from_slice(&0x7fff_0000u32.to_be_bytes());
    refused(&b, "identifier");
    // A section whose bytes are not at its segment's file offset.
    let mut b = bytes.clone();
    let text = parsed.segment_at[1] + 72;
    let offset = word(&b, text + 48) + 8;
    b[text + 48..text + 52].copy_from_slice(&offset.to_le_bytes());
    refused(&b, "file range");
    // A debuggable executable verified as a default build: its entitlement is refused.
    let debuggable = link(
        &hi().bytes(),
        &runtime(),
        &Options {
            identifier: "canary",
            debuggable: true,
        },
    )
    .unwrap();
    assert_eq!(una_link::verify(&debuggable, true), Ok(()));
    refused_by(&debuggable, "blobs");
}

fn refused_by(bytes: &[u8], why: &str) {
    let problem = una_link::verify(bytes, false).unwrap_err();
    assert!(problem.message.contains(why), "{why}: {}", problem.message);
}

#[test]
fn two_relocations_at_one_place_are_u8001() {
    let mut program = calling("_helper");
    let first = program.sections[0].relocs[0].clone();
    program.sections[0].relocs.push(first);
    let problem = failure(&program, &archive(&[("helper.o", &function("_helper", 1))]));
    assert_eq!(problem.code, Code::U8001);
    assert!(
        problem.message.contains("two relocations"),
        "{}",
        problem.message
    );
}
