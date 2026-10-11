# The security hunt's generators

Generators, mutators and drivers of the differential security hunt (#513):
- `docs/plan/security-hunt.md`, the plan;
- `docs/plan/security-hunt-generators.md`, the operators and their verdict classes;
- `docs/plan/security-hunt-oracle.md`, the two-run oracle.

They are Python 3 with the standard library only.

## What is here

- `hunt/families/trusted.py`: family T, operators T1 to T12. They target trusted modules and policies, the rules the formal model assumes and only the checker enforces.
- `hunt/refactor.py`: family R, refactors that keep every label. R6 renames, R9 reorders declarations.
- `hunt/drivers.py`: two judges that need only `una check --tier 0`:
  - **must-reject:** an accepted program is a finding;
  - **must-keep:** a refactor that changes the verdict is a finding.
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
```

- **Reproduction.** Every program is rebuilt from its operator, seed and edition. A program's seed comes from the campaign seed, the operator and the index, nothing else. Each manifest row records a digest of the program's files, and the drivers rebuild every program and compare it with that digest before they run it.
- **Machine slots.** `reject` and `keep` run the compiler, which is heavy work. They run only in a machine slot between benchmark runs.
- **Findings.** A finding goes to the coordinator first, with its minimized repro, before any fix.
- **Nothing generated is committed.** Campaigns write outside the repository.
