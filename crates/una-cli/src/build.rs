//! `una build [path]` (`spec/16 §2`, ADR 0055): checks the package at tier 1, compiles `main`
//! of the `[run] entry` module into an object through `una-codegen`, and links it with the
//! runtime library `una-rt` through the system `cc` into a native executable.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::json;
use una_codegen::CodegenError;
use una_diag::{Diagnostic, render_human};
use una_syntax::ast::ClauseKind;

use crate::contract::Modules;
use crate::runtime::{driven_init, entry_main, rejected_status, takes_stops};
use crate::verification::tier_one;
use crate::{ExitStatus, analyze, load, on_large_stack};

/// The JSON schema of `una build --json` (`spec/16 §2`).
const BUILD_SCHEMA: &str = "una.build/1";

/// The runtime library that every executable links (ADR 0055 rule 5).
const RUNTIME: &str = "libuna_rt.a";

/// What `una build` needs besides the package.
#[derive(Clone)]
pub(crate) struct BuildOptions<'a> {
    /// `--out <file>`.
    pub(crate) out: Option<PathBuf>,
    /// `--json`.
    pub(crate) json: bool,
    /// Whether human diagnostics use color.
    pub(crate) color: bool,
    /// How tier 1 runs the solver (ADR 0064).
    pub(crate) solving: crate::verification::Solving,
    /// The environment variables, for `UNA_RT_LIB`.
    pub(crate) environment: crate::Environment<'a>,
}

/// `una.build/1`, with keys in the order of `spec/16 §2`.
#[derive(Serialize)]
struct BuildDocument<'a> {
    schema: &'static str,
    tier: u8,
    status: &'static str,
    target: String,
    output: Option<String>,
    diagnostics: &'a [Diagnostic],
}

/// `una build [path]`.
pub(crate) fn build_command(
    path: &Path,
    options: &BuildOptions<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    // As `una check` does: U3012 against `[run] grants` when they are listed (`spec/16 §2`).
    let mut analysis = on_large_stack(|| analyze(&package, None));
    if analysis.diagnostics.is_empty() {
        let found = tier_one(&package, &analysis.program, &options.solving);
        analysis.diagnostics = found.diagnostics;
    }
    let target = una_codegen::host_target();
    if !analysis.diagnostics.is_empty() {
        let status = rejected_status(&analysis.diagnostics);
        if options.json {
            let document = BuildDocument {
                schema: BUILD_SCHEMA,
                tier: 1,
                status: "rejected",
                target,
                output: None,
                diagnostics: &analysis.diagnostics,
            };
            print_json(&document, stdout, stderr)?;
        } else {
            write!(
                stderr,
                "{}",
                render_human(
                    &analysis.diagnostics,
                    &package.sources.texts(),
                    options.color
                )
            )?;
        }
        return Ok(status);
    }
    // The module the host drives (ADR 0116 rule 3), whose `init` starts the run; or the entry,
    // whose `main` runs.
    let (manifest, _) = crate::package::project(&package);
    let (entry, main, started) = if let Some(driven) = &manifest.drive {
        let module = driven.module.value.clone();
        match driven_init(&analysis, &package.sources, &module) {
            Ok(init) => (module, init, ["init", "step", "on_stop"].as_slice()),
            Err(message) => {
                writeln!(stderr, "error: {message}")?;
                return Ok(ExitStatus::Usage);
            }
        }
    } else {
        let Some(entry) = analysis.entry.clone() else {
            writeln!(
                stderr,
                "error: una.toml has no `[run] entry`; name the module whose `main` runs"
            )?;
            return Ok(ExitStatus::Usage);
        };
        let Some(main) = entry_main(&analysis, &package.sources, &entry) else {
            writeln!(
                stderr,
                "error: the module `{entry}` has no `pub main() -> Unit` to run"
            )?;
            return Ok(ExitStatus::Usage);
        };
        (entry, main, ["main"].as_slice())
    };
    // The description the executable reads at start: `[run] grants`, and every `uses` of
    // `main`, or of the driven functions, with where it is written, for U3012 (`spec/16 §3`).
    let modules = Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let mut uses: Vec<serde_json::Value> = Vec::new();
    for name in started {
        let Some(function) = analysis
            .program
            .modules
            .iter()
            .filter(|m| m.path == entry)
            .flat_map(|m| &m.decls)
            .find_map(|decl| match decl {
                una_core::Decl::Fun(fun) if fun.name == *name => Some(fun.as_ref()),
                _ => None,
            })
        else {
            continue;
        };
        let clauses: Vec<(String, una_syntax::Span)> = modules
            .function(&format!("{entry}.{name}"))
            .map(|(_, _, function)| {
                function
                    .clauses
                    .iter()
                    .filter_map(|clause| match &clause.kind {
                        ClauseKind::Uses(uses) => Some((
                            analysis.interner.resolve(uses.cap.name).to_owned(),
                            uses.span,
                        )),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        uses.extend(function.uses.iter().map(|need| {
            let span = clauses
                .iter()
                .find(|(name, _)| *name == need.cap)
                .map_or(function.span, |(_, span)| *span);
            json!({
                "function": name,
                "name": need.cap,
                "capability": need.capability,
                "ops": need.ops,
                "location": package.sources.location(span),
            })
        }));
    }
    let grants: Vec<&str> = manifest.grants.iter().map(|g| g.value.as_str()).collect();
    let drive = manifest
        .drive
        .as_ref()
        .map(|drive| json!({ "ticks": drive.ticks, "budget": drive.budget }));
    // What the generated code refers to by index: the calls' locations, for U6003 at the call
    // that exceeds the maximum depth, the types the runtime renders, the operations a trace
    // records, and where `main`, or `init`, is named, for U6001.
    let main_location = package.sources.location(main.span);
    let stops = takes_stops(
        &analysis,
        &package.sources,
        &entry,
        manifest.drive.is_some(),
    );
    let meta = |described: &una_codegen::Described<'_>| {
        let sites: Vec<una_diag::Location> = described
            .sites
            .iter()
            .map(|span| package.sources.location(*span))
            .collect();
        let mut meta = json!({
            "grants_section": match manifest.format.section("[run] grants") {
                section if section.starts_with('`') => section,
                section => format!("`{section}`"),
            },
            "stops": stops,
            "grants": grants,
            "uses": uses,
            "sites": sites,
            "types": described.types,
            "ops": described.ops,
        });
        // ADR 0114 rule 6.4: an executable of edition 0.3 writes and tests the floating-point
        // environment; one of an earlier edition never does.
        if manifest.edition >= una_project::Edition::V0_3
            && let Some(object) = meta.as_object_mut()
        {
            object.insert("float_environment".to_owned(), json!(true));
        }
        // ADR 0119 rule 9: an executable whose code can make a hash table reads the process key
        // of keyed hashing before `main`; one that makes none never reads it.
        if described.hashing
            && let Some(object) = meta.as_object_mut()
        {
            object.insert("hashing".to_owned(), json!(true));
        }
        // ADR 0116 amendment, P8 (#1145): a program with a task construct or a `for ever`
        // starts its workers and helper pools before `main`, so no thread starts inside a
        // construct, at a moment that earlier code could have decided.
        if described.threads
            && let Some(object) = meta.as_object_mut()
        {
            object.insert("threads".to_owned(), json!(true));
        }
        if let (Some(root), Some(object)) = (described.failure, meta.as_object_mut()) {
            object.insert(
                "failure".to_owned(),
                json!({ "location": main_location, "root": root }),
            );
        }
        if let (Some(drive), Some(object)) = (&drive, meta.as_object_mut()) {
            object.insert("drive".to_owned(), drive.clone());
        }
        meta.to_string()
    };
    let compiled = if manifest.drive.is_some() {
        una_codegen::compile_drive(&analysis.program, &entry, &meta)
    } else {
        una_codegen::compile(&analysis.program, &entry, &meta)
    };
    let object = match compiled {
        Ok(object) => object,
        Err(CodegenError::NoMain) => {
            writeln!(
                stderr,
                "error: the module `{entry}` has no `pub main() -> Unit` to run"
            )?;
            return Ok(ExitStatus::Usage);
        }
        Err(error @ CodegenError::Unsupported(_)) => {
            writeln!(stderr, "error: {error} (#26)")?;
            return Ok(ExitStatus::Usage);
        }
        Err(error @ CodegenError::Internal(_)) => {
            writeln!(stderr, "internal compiler error (U8001): {error}")?;
            return Ok(ExitStatus::Internal);
        }
    };
    let root = package_root(&package);
    let (output, shown) = match &options.out {
        Some(out) => (out.clone(), out.display().to_string()),
        None => {
            let name = match (&manifest.name, package.single) {
                (Some(name), false) => name.value.clone(),
                _ => entry.clone(),
            };
            (
                root.join("target").join("una").join(&name),
                format!("target/una/{name}"),
            )
        }
    };
    // #1160: the object and the executable are written only inside the package, or inside the
    // directory `--out` names, never through a link planted there that leads elsewhere. The
    // linker writes into a private directory, and the executable is installed from there.
    let placed = match placed(&root, options.out.is_some(), &output, &object) {
        Ok(placed) => placed,
        Err(error) => {
            writeln!(stderr, "error: cannot write `{shown}`: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    if let Err(message) = link(&placed.object, &placed.produced, options.environment) {
        // Dropping `placed` removes the private directory.
        writeln!(stderr, "error: {message}")?;
        return Ok(ExitStatus::Usage);
    }
    if let Err(error) =
        una_diag::safe_write::install(&placed.root, &placed.produced, &placed.output)
    {
        writeln!(stderr, "error: cannot write `{shown}`: {error}")?;
        return Ok(ExitStatus::Usage);
    }
    drop(placed);
    if options.json {
        let document = BuildDocument {
            schema: BUILD_SCHEMA,
            tier: 1,
            status: "built",
            target,
            output: Some(shown),
            diagnostics: &[],
        };
        print_json(&document, stdout, stderr)?;
    } else {
        writeln!(stdout, "built {shown}")?;
    }
    Ok(ExitStatus::Success)
}

fn print_json(
    document: &BuildDocument<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<()> {
    match serde_json::to_string(document) {
        Ok(text) => writeln!(stdout, "{text}")?,
        Err(error) => writeln!(
            stderr,
            "internal compiler error (U8001): rendering the output as JSON failed: {error}"
        )?,
    }
    stdout.flush()
}

/// Where `una build` writes (#1160): the root its writes stay inside, the checked path of the
/// executable, and the private directory the linker writes in, with the object file and the
/// executable as the linker produces them there.
struct Placed {
    /// The package root, or the directory `--out` names.
    root: PathBuf,
    /// The executable's final path, in its canonical directory.
    output: PathBuf,
    /// The object file, in the private directory.
    object: PathBuf,
    /// The executable as the linker writes it, under its final name in the private directory,
    /// since a linker may record that name in what it writes.
    produced: PathBuf,
    /// The private directory, removed when this value is dropped, on every path.
    _private: una_diag::safe_write::PrivateDir,
}

/// Creates the directory of `output`, a fresh private directory next to it, and the object
/// file in that directory, and returns where the linker reads and writes (#1160).
///
/// The default output, `target/una/<name>`, is placed inside the package root `root`: each
/// level of `target/una` is created one at a time, and a link among them that leads outside the
/// package is refused. An absolute `--out` path (`explicit`), or a relative one with `..`, is
/// placed inside the directory it names, which is created as the user asked; a plain relative
/// one is rooted at the current directory, and its directories are created there as
/// `target/una` is in the package. The executable's path, and `<name>.o` beside it, are
/// checked by [`una_diag::safe_write::checked_output`] before the linker starts, for an early
/// error on a link at either. The linker never writes a name the package could have planted: the object, written
/// through [`una_diag::safe_write::write_atomically`], and the executable both live in a
/// [`una_diag::safe_write::private_dir`] until [`una_diag::safe_write::install`] moves the
/// executable into place.
fn placed(root: &Path, explicit: bool, output: &Path, object: &[u8]) -> io::Result<Placed> {
    let write_root = if explicit {
        let directory = match output.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(parent) => parent.to_path_buf(),
            None => PathBuf::from("."),
        };
        let typed = output.is_absolute()
            || output
                .components()
                .any(|component| component == std::path::Component::ParentDir);
        if typed {
            // An absolute directory, or a relative one with `..`, is a path the user typed on
            // the command line, which a repository cannot inject: created as given, and the
            // root of the writes in it.
            std::fs::create_dir_all(&directory)?;
            directory
        } else {
            // A plain relative one is rooted at the current directory, often the package
            // itself, so it is created one level at a time: a committed `target` link cannot
            // redirect it.
            let here = Path::new(".");
            una_diag::safe_write::create_dir_inside(here, &here.join(&directory))?;
            here.to_path_buf()
        }
    } else {
        let root = if root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            root
        };
        una_diag::safe_write::create_dir_inside(root, &root.join("target").join("una"))?;
        root.to_path_buf()
    };
    let output = una_diag::safe_write::checked_output(&write_root, output)?;
    // The object's former name, `<name>.o` next to the executable, is no longer written, but a
    // link there is refused all the same, as at the executable's name: build outputs are not
    // sources, so a link at one is never written through (R-024 does not apply).
    una_diag::safe_write::checked_output(&write_root, &output.with_extension("o"))?;
    let private = una_diag::safe_write::private_dir(&write_root, &output)?;
    let name = output.file_name().unwrap_or_default();
    let produced = private.path().join(name);
    // `<name>.o`, unless the executable's own name already ends in `.o`.
    let object_path = match produced.with_extension("o") {
        same if same == produced => {
            let stem = same.file_stem().unwrap_or_default().to_string_lossy();
            private.path().join(format!("{stem}-object.o"))
        }
        other => other,
    };
    una_diag::safe_write::write_atomically(&write_root, &object_path, object, true, true)?;
    Ok(Placed {
        root: write_root,
        output,
        object: object_path,
        produced,
        _private: private,
    })
}

/// The directory of `una.toml`, or of the file of a single-file package.
fn package_root(package: &crate::package::Package) -> PathBuf {
    package
        .manifest
        .or_else(|| package.modules.first().copied())
        .and_then(|id| package.disk_paths.get(&id))
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Links `object` with the runtime library into the executable `output` through `cc`.
fn link(object: &Path, output: &Path, environment: crate::Environment<'_>) -> Result<(), String> {
    let runtime = runtime_library(environment)?;
    let mut command = Command::new("cc");
    command.arg(object).arg(&runtime).arg("-o").arg(output);
    // No debug information (#382, S-13): the macOS debug map names each object by the absolute
    // path of `libuna_rt.a`, and DWARF names the directories the runtime was built in, so an
    // executable would carry the paths of the machine that built it and differ between two
    // installs. The behavior of the program does not depend on them. `-S` means this to both
    // ld64 and GNU ld.
    command.arg("-Wl,-S");
    // Only the runtime functions the program calls are kept: an executable that never uses
    // `Http` carries no TLS stack (ADR 0074 H7).
    if cfg!(target_os = "macos") {
        command.arg("-Wl,-dead_strip");
    } else if cfg!(target_os = "linux") {
        command.arg("-Wl,--gc-sections");
    }
    // The system libraries that the Rust standard library inside `una-rt` needs, as
    // `rustc --print native-static-libs` lists them for each target.
    if cfg!(target_os = "linux") {
        command.args([
            "-lgcc_s",
            "-lutil",
            "-lrt",
            "-lpthread",
            "-lm",
            "-ldl",
            "-lc",
        ]);
    }
    let result = command
        .output()
        .map_err(|error| format!("cannot run the system linker `cc`: {error}"))?;
    if result.status.success() {
        Ok(())
    } else {
        let output = String::from_utf8_lossy(&result.stderr);
        let mut message = format!("the system linker `cc` failed:\n{}", output.trim_end());
        if let Some(hint) = stale_runtime_hint(&output, &runtime) {
            message.push('\n');
            message.push_str(&hint);
        }
        Err(message)
    }
}

/// The command that builds `una` with its runtime, for a source build (#203).
const SOURCE_BUILD: &str = "cargo build --release -p una-cli -p una-rt";

/// A hint for a link that fails on a runtime symbol: the runtime library is older than this
/// `una`, as when `cargo build -p una-cli` rebuilt only the compiler (#203).
fn stale_runtime_hint(linker_output: &str, runtime: &Path) -> Option<String> {
    let undefined = linker_output.contains("Undefined symbols")
        || linker_output.contains("undefined reference");
    (undefined && linker_output.contains("una_rt_")).then(|| {
        format!(
            "the runtime `{}` lacks functions this `una` calls, so it is older than `una`: rebuild both with `{SOURCE_BUILD}` (without `--release` for target/debug)",
            runtime.display()
        )
    })
}

/// The message when no runtime library is found beside `una` in `dir` (#203).
fn missing_runtime(dir: &Path) -> String {
    format!(
        "the native runtime `{RUNTIME}` is not next to `una` ({}); a source build makes it with `{SOURCE_BUILD}` (without `--release` for target/debug), or set UNA_RT_LIB to its path",
        dir.display()
    )
}

/// The runtime library: `UNA_RT_LIB` when set, else `libuna_rt.a` in the `lib` directory
/// beside the `una` binary, as a release archive ships it (ADR 0059), next to the binary, as a
/// Cargo build leaves it, or in `../lib`.
fn runtime_library(environment: crate::Environment<'_>) -> Result<PathBuf, String> {
    if let Some(path) = crate::variable(environment, "UNA_RT_LIB") {
        return Ok(PathBuf::from(path));
    }
    // Through a symlink, macOS reports the link's path: the runtime is beside the binary the
    // link resolves to (#185).
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|error| format!("cannot locate the `una` binary: {error}"))?;
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    // A release archive ships it in `lib/` beside `una` (ADR 0059); a Cargo build leaves it
    // next to `una`.
    let candidates = [
        dir.join("lib").join(RUNTIME),
        dir.join(RUNTIME),
        dir.join("..").join("lib").join(RUNTIME),
    ];
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| missing_runtime(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_linker_writes_only_in_a_private_directory_removed_on_every_path() {
        // #1160: `cc -o` names a file in a fresh private directory inside `target/una`, never
        // the final path, and the directory goes away with the placement.
        let dir = std::env::temp_dir().join(format!("una-placed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let output = dir.join("target/una/app");
        let placed = placed(&dir, false, &output, b"object").unwrap();
        let private = placed.produced.parent().unwrap().to_path_buf();
        assert_eq!(placed.output, output);
        assert_ne!(placed.produced, output);
        assert_eq!(placed.produced.file_name(), output.file_name());
        assert_eq!(private.parent(), Some(dir.join("target/una").as_path()));
        assert_eq!(placed.object, private.join("app.o"));
        assert_eq!(std::fs::read(&placed.object).unwrap(), b"object");
        assert!(!dir.join("target/una/app.o").exists());
        drop(placed);
        assert!(!private.exists());
        assert_eq!(
            std::fs::read_dir(dir.join("target/una")).unwrap().count(),
            0
        );
        // A link at the final path is refused before anything is written.
        std::os::unix::fs::symlink(dir.join("elsewhere"), &output).unwrap();
        assert!(placed_err(&dir, &output));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn placed_err(root: &Path, output: &Path) -> bool {
        placed(root, false, output, b"object").is_err()
    }

    #[test]
    fn a_missing_runtime_names_the_command_that_builds_it() {
        let message = missing_runtime(Path::new("/src/una/target/release"));
        assert!(
            message.contains("`cargo build --release -p una-cli -p una-rt`"),
            "{message}"
        );
        assert!(message.contains("UNA_RT_LIB"), "{message}");
    }

    #[test]
    fn a_link_that_misses_a_runtime_function_says_the_runtime_is_stale() {
        let runtime = Path::new("target/debug/libuna_rt.a");
        let macos = "Undefined symbols for architecture arm64:\n  \"_una_rt_operation_begin\", referenced from:";
        let linux =
            "hello.o: in function `una_fn_0': undefined reference to `una_rt_operation_begin'";
        for output in [macos, linux] {
            let hint = stale_runtime_hint(output, runtime).unwrap_or_default();
            assert!(hint.contains("older than `una`"), "{hint}");
            assert!(hint.contains("-p una-cli -p una-rt"), "{hint}");
        }
        // Another link failure gets no hint.
        assert_eq!(
            stale_runtime_hint("Undefined symbols: \"_sqrt\"", runtime),
            None
        );
        assert_eq!(stale_runtime_hint("ld: library not found", runtime), None);
    }
}
