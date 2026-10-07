# Validation

Status: Normative for the project's workflow (#232). GitHub Actions is not used. Merges, tags and releases rest on local runs whose results are recorded, so anyone can see what was checked, on which commit, and with which outcome.

## 1. The rules

1. **A merge into `main`** needs a `scripts/local-ci.sh` PASS on macOS on the pull request's exact tip, written into the pull request body with the SHA.
   - Linux is not checked per pull request. The next gate run confirms it on `main` (`scripts/local-ci.sh --linux`, in Docker), and a Linux FAIL there is reported at once and fixed before anything else merges.
2. **A tag** `v<version>` needs a run of `scripts/release-gates.sh` in which every gate is PASS, on the gated SHA: the commit of `main` after the release pull request.
   - Its record is committed as `docs/releases/<version>-validation.md` before the tag is made.
   - **The tagged commit is the record commit**, not the gated SHA, since `install.sh` reads the record from the tag's tree, and gating the record commit again would be circular. So:
     - The tagged commit's own change, against its first parent, is exactly the record file.
     - Between the gated SHA and the tagged commit, only files under `docs/` change: `git diff --name-only <gated> <tag>` lists nothing else. The code, build files, scripts and `install.sh` of the tag are then those of the gated SHA.
     - Anything else that merges in between, such as a change of code, needs a new gate run, and the gated SHA becomes that commit.
   - A tag whose record has a FAIL or is INCOMPLETE is not made.
   - The owner's word is still needed for every tag and release.
3. **A release** publishes the archives that the `release` gate built (`OUT/clone/target/local-release/v<version>/`), whose digests the record lists. A build is not assumed to be byte-reproducible, so the archives are not rebuilt after the record. The record and the published `.sha256` files MUST agree.
4. **`.github/workflows` is not a gate.** The files stay in the repository, but no merge, tag or release waits for them or reads their results.

## 2. The gates

`scripts/release-gates.sh SHA OUT [GATE...]` runs the gates on exactly one commit, given as a full 40-digit SHA. It runs all of them when none is named.

- It makes a fresh clone of the commit for the run, `OUT/clone`, from the remote `origin` (or `UNA_GATES_SOURCE`).
- Before every gate, it checks that the clone is still that commit, with no change and no untracked file.
- A failing gate does not stop the next one.

| Gate | What it runs | Needs |
| --- | --- | --- |
| `clone` | A fresh clone, checked out at the SHA and checked to be exact | Git |
| `build` | `scripts/check-solver.sh`, then, cold, `cargo build --locked --release -p una-cli -p una-rt` and `cargo test --workspace --locked` | Rust, Z3 |
| `mac` | `scripts/local-ci.sh`: solver, `fmt`, `clippy -D warnings`, tests, the host's release archive packed and checked | macOS |
| `linux` | `scripts/local-ci.sh --linux`: the same steps in a `linux/amd64` container of CI's Ubuntu image | Docker |
| `linux-arm64` | `scripts/local-ci.sh --linux-arm64`: the same steps in a `linux/arm64` container, native on an arm64 Mac (#241) | Docker |
| `release` | `scripts/local-release.sh v<version>` on a tag made in the clone only (never pushed): the three archives (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), each checked with `scripts/check-release.sh` | macOS, Docker |
| `determinism` | The conformance suite twice with equal summaries and no failure; `una --json check` of every conformance case and example twice, the outputs hashed and compared; the tracked files unchanged | Rust, Z3 |
| `native` | The equivalence test of `spec/16 §5`, `every_runnable_program_runs_natively_as_in_the_interpreter`: every runnable program of the repository (the examples, the conformance cases that run, the review corpora), in the interpreter and as a native executable, compared by status, output and trace | Rust, Z3 |

The pinned Z3 comes from `UNA_GATES_Z3`, else this checkout's `target/z3`, else the clone's `target/z3`. If none is there, the `build` gate installs it with `scripts/install-z3.sh`.

At the end, the script removes the Docker volumes that `--linux` and `--linux-arm64` made for the clone's target directory, so volumes do not pile up.

## 3. The record

The script writes into `OUT`:

- `results.tsv`: a header line, then one line per gate run, tab-separated: `gate`, `sha`, `result` (`PASS` or `FAIL`), `seconds`, `log_sha256`.
- `logs/<gate>.log`: each gate's full output. Its SHA-256 is in `results.tsv` and in the record.
- `record.md`, the validation record:
  - the commit, its subject, the run's date, platform, `rustc` and Docker, and the SHA of the script that ran;
  - a table of the gates, with result, seconds and log digest;
  - the archives that the `release` gate packed, with their SHA-256;
  - the verdict:
    - `PASS (every gate)`;
    - `FAIL (<gates>)`;
    - `INCOMPLETE (not run: <gates>)`.

For a tag, `record.md` is committed as `docs/releases/<version>-validation.md`, unchanged. The logs stay with the person who ran the gates. Their digests in the record identify them.

## 4. The procedure for a release

1. Merge the release pull request (version and `CHANGELOG.md`) under rule 1, and note the SHA of `main`.
2. Run `scripts/release-gates.sh <sha> <out>` on that SHA, with Docker running and nothing else using it.
3. If the verdict is PASS, commit `<out>/record.md` as `docs/releases/<version>-validation.md` in a pull request that changes nothing else, under rule 1.
4. Check rule 2 on the merged record commit: its own change is the record, and `git diff --name-only <gated> <record commit>` lists only files under `docs/`. With the owner's word, tag `v<version>` on the record commit and push the tag.
5. Publish the archives from `<out>/clone/target/local-release/v<version>/`, checking that their digests match the record's, with the `gh release create` command that `logs/release.log` prints.
