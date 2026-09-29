# Una

Una is a verification-first programming language for machine-generated software. Agents write implementations; the compiler checks them against explicit specifications of behavior, authority and information flow; humans review the specifications.

**Status:** v0.1 (Una Core). Reference compiler in development.

## Start here

| Document | Purpose |
| --- | --- |
| `docs/00-charter.md` | What Una is, what it proves and assumes, v0.1 scope |
| `docs/README.md` | Documentation map and reading order |
| `docs/plan/milestones-v0.1.md` | Implementation plan with acceptance criteria |
| `CLAUDE.md` | Rules for AI-assisted implementation |
| `docs/guide/una-for-agents.md` | Compact reference for agents writing Una |
| `examples/` | Reference v0.1 programs: `events`, `hello`, `redacted`, `pantry`, `journal`, `greenhouse`, `shortlinks` |

## Build

Requires the stable Rust toolchain pinned in `rust-toolchain.toml` and Z3 at the version pinned in `verification.lock`. `scripts/install-z3.sh DIR` downloads that Z3 release, verifies its SHA-256 digest and installs it as `DIR/bin/z3`; with `DIR/bin` on `PATH`, `scripts/check-solver.sh` prints the detected version and checks it against the lock.

```bash
git clone https://github.com/una-lang/una.git && cd una
scripts/install-z3.sh "$HOME/.local/z3" && export PATH="$HOME/.local/z3/bin:$PATH"
scripts/check-solver.sh
cargo build --workspace
cargo run -p una-cli -- check examples/events
cargo run -p una-cli -- test examples/events
```

## License

Una is licensed under the Apache License, Version 2.0; see [`LICENSE`](LICENSE).
