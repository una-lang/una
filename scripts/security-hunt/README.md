# The security hunt's generators

Generators, mutators and drivers of the differential security hunt (#513):
- `docs/plan/security-hunt.md`, the plan;
- `docs/plan/security-hunt-generators.md`, the operators and their verdict classes;
- `docs/plan/security-hunt-oracle.md`, the two-run oracle.

They are Python 3 with the standard library only.

## What is here

- `hunt/families/trusted.py`: family T, operators T1 to T12. They target trusted modules and policies, the rules the formal model assumes and only the checker enforces.
- `hunt/families/handlers.py` and `keys.py`: families H (handler bodies and the effect floor, H1 to H6) and K (`Keys.hmac`, K1 to K4).
- `hunt/families/payloads.py`, `generics.py`, `concurrency.py`, `sums.py` and `ordering.py`: families P (failure payloads), G (kind parameters and invariance), C (`together`, edition 0.2), S (user sum types, `is` and `in`) and O (`Ord`, `sort`, `min`, `max`), the blind rows of `docs/reviews/security-hunt/coverage.md`.
- `hunt/families/composed.py` and `composed2.py`: the composed families, two blind rows at once: CS (`together` over user sum types), PG (payloads of generic kinds), TC (policies in tasks), CP (payload failures in tasks) and GS (generic sums at secret instances).
- `hunt/families/boundaries.py` and `conditions.py`: families B (authority boundaries, ADR 0093) and Q (conditions on allowed effects and contract locks, ADR 0092), the rules of edition 0.3, generated in editions 0.2, as the edition gate's probe, and 0.3 (`OPERATOR_EDITIONS`).
- `hunt/families/dependencies.py` and `registry.py`: families D (authority, information flow and trust across packages, ADR 0122 rules 49 to 53) and Y (registry packages: digests, provenance, vendoring, yanks and resolution, `spec/30` §9 to §11 and §14), programs of several packages of edition 0.4, generated in 0.3 as the edition gate's probe and in 0.4. Y1 is judged by `una ita lock --check` (`Program.command`), with the run's own Itaoca listing the program's registry as a mirror; every other program by `una check`.
- `hunt/families/timing.py`: family W, tasks and time over code that reads secrets (#973, ADR 0117 S8): `within` (W1), `first of` (W2), `tasks` and `start` (W3) and `std.limit` (W4), edition 0.4, each with one varied secret literal. Accepted forms keep every timing outcome at the secret's label; closed calls miss it, and a correct checker rejects them. It is kept apart from the operators the gate runs (`TIMING_OPERATORS`), so the fixed run is unchanged; `hunt timing` runs it.
- `hunt/families/servers.py`: family N, servers (#1011, ADR 0123 S9), edition 0.4: a server in the language form and a script of connections (`spec/31 §8` rule 6) that stands for its peers, so no run binds a listener. N1 to N4 send malformed requests, by every refusal of `spec/31 §3` and §4, and their well-formed twins, each followed by a request a smuggling peer wants read, whole and split. N5 echoes a request's decoded query value into a header of the reply, CR, LF and NUL included, and writes names the host refuses: such a reply must fail with `BadReply` and send nothing. N6's request tasks read a secret, for the two-run oracle's server mode, whose `internal` observer sees the wire. Kept apart from the gate's operators (`SERVER_OPERATORS`); `hunt servers` runs it.
- `hunt/itaty.py`, `hunt/ed25519.py` and `hunt/packages.py`: the registry builder of families D and Y, written from `spec/30` alone, with no compiler: Ed25519 signing (RFC 8032), signed notes, root metadata, owners, release statements, yanks, the log's tree and tiles, the name map's tiles, cosigned checkpoints, archives, content digests, provenance documents, vendored copies and the lock's lines. A program it builds is a second opinion on the formats: an honest one the client refuses, or a forged one it accepts, is a finding.
- `hunt/families/fragment.py`: family F, programs inside the formal re-check's vocabulary, for part (b).
- `hunt/refactor.py`: family R, refactors that keep every label. R6 renames, interpolations included; R9 reorders declarations. Both rewrite the root package's modules (`src/`) only, never a dependency's pinned files.
- `hunt/drivers.py`: two judges that need only `una check --tier 0`:
  - **must-reject:** an accepted program is a finding;
  - **must-keep:** a refactor that changes the verdict is a finding;
  - **differ** (part b), which also needs `una-core-check` (`formal/.lake/build/bin`): for a program `una check` accepts, a declaration the re-check rejects is a finding, even when the re-check then times out or crashes. `clean` needs every condition: a complete export, a normal exit, a line for every function and every refinement predicate, each predicate's line `outside the flow model` (unless the whole document fell back to one reason), and only known outcomes. Anything else is `unknown`.
- `hunt/oracle.py`: the two-run oracle's part that the release gate runs: for an accepted program, each varied secret literal is replaced by other values of its type, every variant runs twice on the interpreter, and what an observer below the literal's label sees (at `public` and `internal`, events filtered by their operation's `accepts`) must not change. Two runs of one variant that differ, or a run that ends in an unknown way, are errors; a pair where only one side aborts on a resource limit (U6003, U6004, U6005) or the time limit is recorded as a termination pair, outside T6. Its server mode (`serve=True`, #1011) runs each variant over the program's script of connections, with what the peers received in the `internal` view (`docs/plan/security-hunt-oracle.md` §11). Since a run's trace leaves out the operations of quiet code (S-42), each variant also runs once as an example, the probe, whose trace keeps them: its events join the views and the liveness view (`docs/plan/security-hunt-oracle.md` §12, #1911).
- `hunt/native.py`: the oracle's native sample (#792): a stratified, fixed sample of its programs, by the hash of their names and one per compared family, also runs as native executables, every variant, and the exit code, standard output, standard error and trace must equal the interpreter's, by the rules of `crates/una-cli/tests/native.rs`. A difference is a backend divergence. The programs the native backend does not compile are pinned by name in `native-unsupported.txt`.
- `hunt/costclock.py`: the timing run (#973): family W, seed 973, through must-reject and the oracle under the cost clock (`una run --virtual-time --cost-clock`, `spec/27 §8` rule 6), where each check point moves the virtual clock by one millisecond, so a timing label that misses a secret shows as a difference of values; with a canary that must leak under the cost clock and stay clean on the plain virtual clock. `docs/plan/security-hunt-oracle.md` §10.
- `hunt/serving.py`: the server run (#1011): family N, seed 1011, each program run twice with `--serve-script` and `--serve-out` under the sandbox, what its peers received judged against its expectation (the refusal's status alone and the request behind it never read, a twin answered, the split bytes read as whole, the server alive after), with two canaries. It needs a test build of `una` (`cargo build -p una-cli --features test-hooks`, or the `target/debug/una` that `cargo test` builds), whose scripted engine serves with `UNA_TEST_RUN_SERVER=1`; any other stops the run.
- `hunt/gate.py`: the `hunt` release gate (`docs/release-gates.md`): the fixed, seeded run through must-reject, must-keep, the oracle and its native sample, and the re-check comparison (`differ`), its canaries, and `summary.json`. It fails closed.
- `hunt/sandbox.py`: every compiler run happens in a sandbox: `sandbox-exec` on macOS, and on Linux Landlock (`hunt/landlock.py`) inside the hardened container of `scripts/hunt-linux.sh`, which the sandbox verifies before any run (#793). The only writable directory is the run's, there is no network but the mock's port, and the environment is clean. Without its sandbox, nothing runs.
- `tests/`: tests that need no compiler.

## Use

```sh
cd scripts/security-hunt
python3 -m unittest discover -s tests                     # no compiler
python3 -m hunt gen --ops T --count 25 --seed 1 --out DIR  # writes programs and DIR/manifest.jsonl
python3 -m hunt repro --op T4 --seed SEED --edition 0.2 --out DIR
python3 -m hunt reject --manifest DIR/manifest.jsonl --una /abs/path/to/una --root DIR
python3 -m hunt keep --manifest DIR/manifest.jsonl --una /abs/path/to/una --root DIR
python3 -m hunt differ --manifest DIR/manifest.jsonl --una /abs/path/to/una --core-check /abs/path/to/una-core-check --root DIR
python3 -B -m hunt gate --una /abs/path/to/una --out DIR --tip SHA            # the release gate's run
python3 -B -m hunt timing --una /abs/path/to/una --out DIR                     # family W under the cost clock
python3 -B -m hunt servers --una /abs/path/to/test-build/una --out DIR         # family N, scripts of connections
```

- **Reproduction.** Every program is rebuilt from its operator, seed and edition. A program's seed comes from the campaign seed, the operator and the index, nothing else. Each manifest row records a digest of the program's files, and the drivers rebuild every program and compare it with that digest before they run it.
- **Machine slots.** `reject`, `keep` and `differ` run the compiler, which is heavy work. They run only in a machine slot between benchmark runs.
- **Findings.** A finding is reported privately first, with its minimized repro, before any fix.
- **Nothing generated is committed.** Campaigns write outside the repository.
