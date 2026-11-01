//! `cargo run -p una-link --example link -- PROGRAM.o LIBUNA_RT.a OUT [--debuggable]`: links as
//! `una build` does, for tests and investigation.
#![allow(clippy::print_stderr)]

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(program), Some(runtime), Some(out)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: link PROGRAM.o LIBUNA_RT.a OUT [--debuggable]");
        return std::process::ExitCode::from(2);
    };
    let debuggable = args.iter().any(|a| a == "--debuggable");
    let read = |path: &str| std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"));
    let result = read(program)
        .and_then(|p| read(runtime).map(|r| (p, r)))
        .and_then(|(p, r)| {
            let identifier = std::path::Path::new(out)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            una_link::link(
                &p,
                &r,
                &una_link::Options {
                    identifier: &identifier,
                    debuggable,
                },
            )
            .map_err(|problem| problem.to_string())
        });
    match result {
        Ok(bytes) => {
            if let Err(error) = std::fs::write(out, bytes) {
                eprintln!("cannot write {out}: {error}");
                return std::process::ExitCode::FAILURE;
            }
            let _ = std::process::Command::new("chmod")
                .arg("755")
                .arg(out)
                .status();
            std::process::ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("link: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}
