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
- `hunt/families/fragment.py`: family F, programs inside the formal re-check's vocabulary, for part (b).
- `hunt/refactor.py`: family R, refactors that keep every label. R6 renames, interpolations included; R9 reorders declarations.
- `hunt/drivers.py`: two judges that need only `una check --tier 0`:
  - **must-reject:** an accepted program is a finding;
  - **must-keep:** a refactor that changes the verdict is a finding;
  - **differ** (part b), which also needs `una-core-check` (`formal/.lake/build/bin`): for a program `una check` accepts, a declaration the re-check rejects is a finding, even when the re-check then times out or crashes. `clean` needs every condition: a complete export, a normal exit, a line for every function and every refinement predicate, each predicate's line `outside the flow model` (unless the whole document fell back to one reason), and only known outcomes. Anything else is `unknown`.
- `hunt/oracle.py`: the two-run oracle's part that the release gate runs: for an accepted program, each varied secret literal is replaced by other values of its type, every variant runs twice on the interpreter, and what an observer below the literal's label sees (at `public` and `internal`, events filtered by their operation's `accepts`) must not change. Two runs of one variant that differ, or a run that ends in an unknown way, are errors; a pair where only one side aborts on a resource limit (U6003, U6004, U6005) or the time limit is recorded as a termination pair, outside T6.
- `hunt/gate.py`: the `hunt` release gate (`docs/release-gates.md`): the fixed, seeded run through must-reject, must-keep, the oracle and the re-check comparison (`differ`), its canaries, and `summary.json`. It fails closed.
- `hunt/sandbox.py`: every compiler run happens under `sandbox-exec`. The only writable directory is the campaign's root, there is no network, and the environment is clean. Without `sandbox-exec`, nothing runs.
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
```

- **Reproduction.** Every program is rebuilt from its operator, seed and edition. A program's seed comes from the campaign seed, the operator and the index, nothing else. Each manifest row records a digest of the program's files, and the drivers rebuild every program and compare it with that digest before they run it.
- **Machine slots.** `reject`, `keep` and `differ` run the compiler, which is heavy work. They run only in a machine slot between benchmark runs.
- **Findings.** A finding goes to the coordinator first, with its minimized repro, before any fix.
- **Nothing generated is committed.** Campaigns write outside the repository.
