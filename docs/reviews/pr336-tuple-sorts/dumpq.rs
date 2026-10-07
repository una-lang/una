//! Scratch tool (not part of una): writes every SMT query of each program to OUT/<n>/<i>.smt2.
//! Usage: dumpq <lock file> <out dir> <program path>...
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let text = std::fs::read_to_string(&args[1]).unwrap();
    let (lock, _) = una_project::parse_lock(&text);
    let lock = una_verify::Lock { text, solver_version: lock.solver_version, step_budget: lock.step_budget, random_seed: lock.random_seed };
    let out = PathBuf::from(&args[2]);
    for (n, program) in args[3..].iter().enumerate() {
        let path = Path::new(program);
        let Ok(checked) = una_cli::check(path, una_cli::Tier::Zero) else { continue };
        if !checked.diagnostics.is_empty() { continue; }
        let obligations = std::thread::scope(|scope| {
            std::thread::Builder::new().stack_size(64 * 1024 * 1024)
                .spawn_scoped(scope, || una_verify::obligations(&checked.program, &checked.sources))
                .unwrap().join().unwrap()
        });
        let dir = out.join(format!("{n:05}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("program.txt"), program).unwrap();
        for (i, q) in una_verify::queries(&obligations, &lock).iter().enumerate() {
            std::fs::write(dir.join(format!("{i:04}.smt2")), q).unwrap();
        }
    }
}
