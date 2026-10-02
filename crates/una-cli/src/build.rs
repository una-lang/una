//! `una build [path]` (`spec/16 §2`, ADR 0055): checks the package at tier 1, compiles `main`
//! of the `[run] entry` module into an object through `una-codegen`, and links it with the
//! runtime library `una-rt` through the system `cc` into a native executable.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use una_codegen::CodegenError;
use una_diag::{Diagnostic, render_human};
use una_syntax::ast::ClauseKind;

use crate::contract::Modules;
use crate::runtime::{entry_main, rejected_status};
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
    /// The wall-clock guard of the solver.
    pub(crate) wallclock: Duration,
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
        let found = tier_one(&package, &analysis.program, options.wallclock);
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
    // The description the executable reads at start: `[run] grants`, and every `uses` of
    // `main` with where it is written, for U3012 (`spec/16 §3`).
    let (manifest, _) = crate::package::project(&package);
    let modules = Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
    };
    let clauses: Vec<(String, una_syntax::Span)> = modules
        .function(&format!("{entry}.main"))
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
    let uses: Vec<serde_json::Value> = main
        .uses
        .iter()
        .map(|need| {
            let span = clauses
                .iter()
                .find(|(name, _)| *name == need.cap)
                .map_or(main.span, |(_, span)| *span);
            json!({
                "name": need.cap,
                "capability": need.capability,
                "ops": need.ops,
                "location": package.sources.location(span),
            })
        })
        .collect();
    let grants: Vec<&str> = manifest.grants.iter().map(|g| g.value.as_str()).collect();
    // What the generated code refers to by index: the calls' locations, for U6003 at the call
    // that exceeds the maximum depth, the types the runtime renders, the operations a trace
    // records, and where `main` is named, for U6001.
    let main_location = package.sources.location(main.span);
    let meta = |described: &una_codegen::Described<'_>| {
        let sites: Vec<una_diag::Location> = described
            .sites
            .iter()
            .map(|span| package.sources.location(*span))
            .collect();
        let mut meta = json!({
            "grants": grants,
            "uses": uses,
            "sites": sites,
            "types": described.types,
            "ops": described.ops,
        });
        if let (Some(root), Some(object)) = (described.failure, meta.as_object_mut()) {
            object.insert(
                "failure".to_owned(),
                json!({ "location": main_location, "root": root }),
            );
        }
        meta.to_string()
    };
    let object = match una_codegen::compile(&analysis.program, &entry, &meta) {
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
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let object_path = output.with_extension("o");
    std::fs::write(&object_path, object)?;
    let linked = link(&object_path, &output, options.environment);
    let _ = std::fs::remove_file(&object_path);
    if let Err(message) = linked {
        writeln!(stderr, "error: {message}")?;
        return Ok(ExitStatus::Usage);
    }
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
        Err(format!(
            "the system linker `cc` failed:\n{}",
            String::from_utf8_lossy(&result.stderr).trim_end()
        ))
    }
}

/// The runtime library: `UNA_RT_LIB` when set, else `libuna_rt.a` in the `lib` directory
/// beside the `una` binary, as a release archive ships it (ADR 0059), next to the binary, as a
/// Cargo build leaves it, or in `../lib`.
fn runtime_library(environment: crate::Environment<'_>) -> Result<PathBuf, String> {
    if let Some(path) = environment("UNA_RT_LIB") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe()
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
        .ok_or_else(|| {
            format!(
                "the native runtime `{RUNTIME}` is not next to `una` ({}); set UNA_RT_LIB to its path",
                dir.display()
            )
        })
}
