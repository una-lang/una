//! `una build [path]` (`spec/16 §2`, ADR 0055): checks the package at tier 1, compiles `main`
//! of the `[run] entry` module into an object through `una-codegen`, and links it with the
//! runtime library `una-rt` through the system `cc` into a native executable. A release `una`
//! links the runtime built into it (ADR 0124 step 2, `crate::runtime_archive`); a development
//! build reads it from a file (rule 19).

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
use crate::{ExitStatus, analyze, on_large_stack};

/// The JSON schema of `una build --json` (`spec/16 §2`).
const BUILD_SCHEMA: &str = "una.build/1";

/// The runtime library that every executable links (ADR 0055 rule 5).
const RUNTIME: &str = "libuna_rt.a";

/// What `una build` needs besides the package.
#[derive(Clone)]
pub(crate) struct BuildOptions<'a> {
    /// `--out <file>`.
    pub(crate) out: Option<PathBuf>,
    /// `--debuggable` (ADR 0124 rule 27, Question 7): the signature lets a debugger attach.
    pub(crate) debuggable: bool,
    /// `--json`.
    pub(crate) json: bool,
    /// Whether human diagnostics use color.
    pub(crate) color: bool,
    /// How tier 1 runs the solver (ADR 0064).
    pub(crate) solving: crate::verification::Solving,
    /// The environment variables, for `UNA_RT_LIB` (development builds only, ADR 0124 rule 15).
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
    let package = match crate::package::load_in(
        path,
        &std::collections::BTreeMap::new(),
        options.solving.itaoca.as_ref(),
    ) {
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
    // A program with a `tasks` statement runs `main` as a coroutine (ADR 0117 rule 11).
    let tasks = una_core::has_tasks(&analysis.program);
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
            "tasks": tasks,
            // This backend never elides a check (ADR 0112): `--virtual-time` is accepted.
            "elision": false,
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
        // ADR 0120 rule 5: edition 0.4 adds the binary32 probes.
        if manifest.edition >= una_project::Edition::V0_4
            && let Some(object) = meta.as_object_mut()
        {
            object.insert("float32_probes".to_owned(), json!(true));
        }
        // ADR 0119 rule 9: an executable whose code can make a hash table reads the process key
        // of keyed hashing before `main`; one that makes none never reads it.
        if described.hashing
            && let Some(object) = meta.as_object_mut()
        {
            object.insert("hashing".to_owned(), json!(true));
        }
        // `spec/31 §2` rule 4: an executable of edition 0.4 may take `serve:` grants.
        if manifest.edition >= una_project::Edition::V0_4
            && let Some(object) = meta.as_object_mut()
        {
            object.insert("serve".to_owned(), json!(true));
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
        // The declared budget, which `--max-memory` may lower, never raise (ADR 0109 rule 4).
        if let (Some(max_memory), Some(object)) = (manifest.max_memory, meta.as_object_mut()) {
            object.insert("max_memory".to_owned(), json!(max_memory));
        }
        // ADR 0117 IF4: `L_lim`, which a grant with limits needs at most `internal`, for an
        // executable of edition 0.4 only; one without it takes no limits (`spec/27 §9`).
        if manifest.edition >= una_project::Edition::V0_4
            && let Some(object) = meta.as_object_mut()
        {
            object.insert(
                "limit_label".to_owned(),
                json!(analysis.limit_label.to_string()),
            );
            object.insert(
                "limits_allowed".to_owned(),
                json!(analysis.limit_label.flows_to(&internal_label())),
            );
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
    let linker = match linker(options.environment) {
        Ok(linker) => linker,
        Err(message) => {
            writeln!(stderr, "error: {message}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let written_object = (linker == Linker::SystemCc).then_some(object.as_slice());
    let placed = match placed(&root, options.out.is_some(), &output, written_object) {
        Ok(placed) => placed,
        Err(error) => {
            writeln!(stderr, "error: cannot write `{shown}`: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let linked = match linker {
        Linker::Internal => {
            link_internal(&placed, &object, options.debuggable, options.environment)
        }
        Linker::SystemCc => link(&placed, options.environment),
    };
    if let Err(failure) = linked {
        // Dropping `placed` removes the private directory, with the runtime written there.
        return match failure {
            LinkFailure::Usage(message) => {
                writeln!(stderr, "error: {message}")?;
                Ok(ExitStatus::Usage)
            }
            LinkFailure::Internal(message) => {
                writeln!(stderr, "internal compiler error (U8001): {message}")?;
                Ok(ExitStatus::Internal)
            }
            LinkFailure::Layout(message) => {
                writeln!(stderr, "error[U7015]: {message}")?;
                Ok(ExitStatus::Errors)
            }
        };
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
    /// The private directory, removed when this value is dropped, on every path. A release
    /// `una` writes its runtime here for `cc` (ADR 0124 step 2).
    private: una_diag::safe_write::PrivateDir,
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
fn placed(root: &Path, explicit: bool, output: &Path, object: Option<&[u8]>) -> io::Result<Placed> {
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
    if let Some(object) = object {
        una_diag::safe_write::write_atomically(&write_root, &object_path, object, true, true)?;
    }
    Ok(Placed {
        root: write_root,
        output,
        object: object_path,
        produced,
        private,
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

/// Why a link failed: a usage error (exit code 2, `spec/16 §2` rule 3), U8001 when the runtime
/// built into `una` is missing or damaged or the internal linker refuses Una's own inputs (exit
/// code 3), or U7015 when the executable cannot be laid out (ADR 0124, Diagnostics).
enum LinkFailure {
    Usage(String),
    Internal(String),
    Layout(String),
}

/// Links the object of `placed` with the runtime library into its executable through `cc`, in
/// its private directory.
fn link(placed: &Placed, environment: crate::Environment<'_>) -> Result<(), LinkFailure> {
    let (object, output) = (placed.object.as_path(), placed.produced.as_path());
    let (runtime, embedded_runtime) = runtime(environment, placed)?;
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
    // Only the system libraries the executable imports a symbol from get a load command or a
    // `NEEDED` entry (#1103): ld64 drops the others with `-dead_strip_dylibs`, and GNU ld, with
    // `--as-needed` before the libraries below, records none of them that the link does not use
    // (since glibc 2.34, `libutil`, `librt`, `libpthread` and `libdl` are empty stubs). Some
    // distributions' compilers pass `--as-needed` by default, others do not, so it is explicit.
    // scripts/load-commands.sh fails on any load command outside scripts/load-commands.txt.
    if cfg!(target_os = "macos") {
        command.arg("-Wl,-dead_strip_dylibs");
    } else if cfg!(target_os = "linux") {
        command.arg("-Wl,--as-needed");
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
    let result = command.output().map_err(|error| {
        LinkFailure::Usage(format!("cannot run the system linker `cc`: {error}"))
    })?;
    if result.status.success() {
        Ok(())
    } else {
        let output = String::from_utf8_lossy(&result.stderr);
        let mut message = format!("the system linker `cc` failed:\n{}", output.trim_end());
        // A runtime built into `una` is the one it was built with, so it cannot be stale (ADR
        // 0124 rule 14); the hint is for a runtime read from a file.
        if !embedded_runtime && let Some(hint) = stale_runtime_hint(&output, &runtime) {
            message.push('\n');
            message.push_str(&hint);
        }
        Err(LinkFailure::Usage(message))
    }
}

/// Which program links the executable (ADR 0124 rules 22 and 19).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Linker {
    /// `una-link`, inside `una` (ADR 0124 step 3), on `aarch64-apple-darwin`.
    Internal,
    /// The system `cc`, the linker of every other target until step 4.
    SystemCc,
}

/// Whether this `una` is built for the target of the internal linker.
const INTERNAL_TARGET: bool = cfg!(all(target_os = "macos", target_arch = "aarch64"));

/// The linker of this build: the internal linker on `aarch64-apple-darwin`, the system `cc`
/// elsewhere. `UNA_LINKER` (`internal` or `cc`) chooses in a development build only, for the
/// tests that compare the two (ADR 0124 rule 19); a release `una`, one with the runtime built in,
/// ignores it, as it ignores `UNA_RT_LIB` (rule 15).
fn linker(environment: crate::Environment<'_>) -> Result<Linker, String> {
    let default = if INTERNAL_TARGET {
        Linker::Internal
    } else {
        Linker::SystemCc
    };
    let release = crate::runtime_archive::SHA256.is_some();
    match crate::variable(environment, "UNA_LINKER").as_deref() {
        _ if release => Ok(default),
        None | Some("") => Ok(default),
        Some("internal") if INTERNAL_TARGET => Ok(Linker::Internal),
        Some("internal") => Err(
            "UNA_LINKER=internal: the internal linker builds for aarch64-apple-darwin only"
                .to_owned(),
        ),
        Some("cc") => Ok(Linker::SystemCc),
        Some(other) => Err(format!(
            "UNA_LINKER must be `internal` or `cc`, not `{other}`"
        )),
    }
}

/// The linker that `una build` uses by default, as the report names it (ADR 0124 rule 28):
/// `una <version>` for the internal linker, `cc` for the system's.
pub(crate) fn linker_name() -> String {
    if INTERNAL_TARGET {
        format!("una {}", env!("CARGO_PKG_VERSION"))
    } else {
        "cc".to_owned()
    }
}

/// Links the program's `object` with the runtime through the internal linker (ADR 0124 step 3),
/// and writes the executable, mode 0755, as a new file in the private directory of `placed`. No
/// other program runs, and nothing but the executable is written: the runtime is read in place
/// (rule 13). A refusal of the linker is U8001 or U7015, and leaves no file behind.
fn link_internal(
    placed: &Placed,
    object: &[u8],
    debuggable: bool,
    environment: crate::Environment<'_>,
) -> Result<(), LinkFailure> {
    let embedded = crate::runtime_archive::embedded(environment).map_err(LinkFailure::Usage)?;
    let from_file;
    let runtime: &[u8] = match &embedded {
        Some(embedded) => embedded.verified().map_err(LinkFailure::Internal)?,
        None => {
            let path = runtime_library(environment).map_err(LinkFailure::Usage)?;
            from_file = std::fs::read(&path).map_err(|error| {
                LinkFailure::Usage(format!(
                    "cannot read the native runtime `{}`: {error}",
                    path.display()
                ))
            })?;
            &from_file
        }
    };
    let identifier = placed
        .output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let options = una_link::Options {
        identifier: &identifier,
        debuggable,
    };
    let bytes =
        una_link::link(object, runtime, &options).map_err(|problem| match problem.code {
            una_link::Code::U8001 => LinkFailure::Internal(problem.message),
            una_link::Code::U7015 => LinkFailure::Layout(problem.message),
        })?;
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o755);
    let written = open
        .open(&placed.produced)
        .and_then(|mut file| file.write_all(&bytes).and_then(|()| file.sync_all()));
    written.map_err(|error| LinkFailure::Usage(format!("cannot write the executable: {error}")))
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
        "the native runtime `{RUNTIME}` is not next to this development build of `una` ({}); a source build makes it with `{SOURCE_BUILD}` (without `--release` for target/debug), or set UNA_RT_LIB to its path",
        dir.display()
    )
}

/// The runtime library to link, and whether it is the one built into `una`.
///
/// A release `una` (and a test build with the hook of `crate::runtime_archive::embedded`)
/// checks its embedded runtime against the digest recorded at its build and writes it into the
/// link's private directory (mode 0700, fresh for this build, #1160) as a new file of mode 0600,
/// which goes with the directory on every path. It reads no variable and no other file: neither
/// `UNA_RT_LIB` nor a `libuna_rt.a` beside `una` changes what it links (ADR 0124 rules 14 and
/// 15). A development build searches as ADR 0059 rule 2 does ([`runtime_library`]).
fn runtime(
    environment: crate::Environment<'_>,
    placed: &Placed,
) -> Result<(PathBuf, bool), LinkFailure> {
    let embedded = crate::runtime_archive::embedded(environment).map_err(LinkFailure::Usage)?;
    let Some(embedded) = embedded else {
        return runtime_library(environment)
            .map(|path| (path, false))
            .map_err(LinkFailure::Usage);
    };
    let bytes = embedded.verified().map_err(LinkFailure::Internal)?;
    let taken = [placed.object.as_path(), placed.produced.as_path()];
    crate::runtime_archive::write_into(bytes, placed.private.path(), &taken)
        .map(|path| (path, true))
        .map_err(|error| {
            LinkFailure::Usage(format!(
                "cannot write the native runtime for the system linker: {error}"
            ))
        })
}

/// The runtime library of a development build (ADR 0124 rule 19; ADR 0059 rule 2 and its
/// amendment): `UNA_RT_LIB` when set, else `libuna_rt.a` in the `lib` directory beside the
/// `una` binary, next to the binary, as a Cargo build leaves it, or in `../lib`.
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
    // An archive before ADR 0124 step 2 shipped it in `lib/` beside `una` (ADR 0059); a Cargo
    // build leaves it next to `una`.
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

/// The label `internal`, the most `L_lim` may be for grants with limits (ADR 0117 IF4).
pub(crate) fn internal_label() -> una_core::Label {
    una_core::Label {
        level: una_syntax::ast::Level::Internal,
        ..una_core::Label::public()
    }
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
        let placed = placed(&dir, false, &output, Some(b"object")).unwrap();
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
        placed(root, false, output, Some(b"object")).is_err()
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
