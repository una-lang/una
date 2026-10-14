# Release gates

Status: Normative for the project's workflow (#232). GitHub Actions is not used. Merges, tags and releases rest on local runs whose results are recorded, so anyone can see what was checked, on which commit, and with which outcome.

## 1. The rules

1. **A merge into `main`** needs a `scripts/local-ci.sh` PASS on macOS on the pull request's exact tip, written into the pull request body with the SHA.
   - Linux is not checked per pull request. The next gate run confirms it on `main` (`scripts/local-ci.sh --linux`, in Docker), and a Linux FAIL there is reported at once and fixed before anything else merges.
   - Except when the pull request changes what the Linux run runs: `scripts/local-ci.sh`, `scripts/local-ci/` (its Docker image), or a script that a step of it calls, such as `scripts/formal.sh` or `docs/reviews/formal-m2-replay/check.sh`, or makes a step need a tool it did not need before. Such a pull request also needs a `scripts/local-ci.sh --linux-arm64` PASS (native, a few minutes) on its exact tip, written into its body with the SHA (#605). A step that needs a tool the image lacks passes on macOS, which has it, and fails only in the container: #528's replay needed `python3`, and the first gate run of v0.3.8 failed on it (#606).
2. **A tag** `v<version>` needs a run of `scripts/release-gates.sh` in which every gate is PASS, on the gated SHA: the commit of `main` after the release pull request.
   - Its public summary (§3) is committed as `docs/validation/<version>.md` before the tag is made.
   - **The tagged commit is the record commit**, not the gated SHA, since `install.sh` reads the summary from the tag's tree, and gating the record commit again would be circular. So:
     - The tagged commit's own change, against its first parent, is exactly the summary file and, until the repository is public, the full record as `docs/releases/<version>-validation.md`.
     - Between the gated SHA and the tagged commit, only files under `docs/` change: `git diff --name-only <gated> <tag>` lists nothing else. The code, build files, scripts and `install.sh` of the tag are then those of the gated SHA.
     - Anything else that merges in between, such as a change of code, needs a new gate run, and the gated SHA becomes that commit.
   - A tag whose summary has a FAIL or is INCOMPLETE is not made.
   - Every tag and release also needs an explicit approval, recorded in the release issue before the tag.
3. **A release** publishes the archives that the `release` gate built (`OUT/clone/target/local-release/v<version>/`), whose digests the summary lists. The archives are reproducible (#679): two builds of one commit give the same bytes for each target, which the `reproducible` gate checks with a second build from cold. A published archive can therefore be rebuilt from its commit and checked against the summary's digest. The summary and the published `.sha256` files MUST agree.
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
| `mac` | `scripts/local-ci.sh`: solver, the released sections of `CHANGELOG.md` against their tags (#645): the gate's clone has the tags of its source, and the gates check only the tags the gated commit contains (`UNA_RELEASED_SECTIONS_SCOPE=ancestors`), so a past release's commit can be gated again after newer tags exist, `fmt`, `clippy -D warnings`, tests, the host's release archive packed and checked | macOS |
| `linux` | `scripts/local-ci.sh --linux`: the same steps in a `linux/amd64` container of CI's Ubuntu image, emulated on an arm64 Mac; its tests run one at a time on one thread (`RAYON_NUM_THREADS=1`, `--test-threads=1`), since processes spawned from many threads at once can hang under the emulation (#286). It checks the amd64 build, tests and archive. The released sections of `CHANGELOG.md` are checked on the host before the container starts, since a worktree mounted alone has no tags there, and the container's run skips them, visibly | Docker |
| `linux-arm64` | `scripts/local-ci.sh --linux-arm64`: the same steps in a `linux/arm64` container, native on an arm64 Mac (#241), with the tests at full parallelism: the primary evidence of the tests on Linux (#286) | Docker |
| `release` | `scripts/local-release.sh v<version>` on a tag made in the clone only (never pushed): the three archives (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), each checked with `scripts/check-release.sh` | macOS, Docker |
| `reproducible` | `scripts/local-release.sh v<version>` a second time, from cold: in a new checkout at a new path, with new Docker volumes for the Linux target directories. Each archive's sha256 is compared with the one the `release` gate built, and every pair must be equal (#679). Its cost is a second `release` gate: three release builds, one of them emulated | macOS, Docker |
| `determinism` | The conformance suite twice with equal summaries and no failure; `una --json check` of every conformance case and example twice, the outputs hashed and compared; the tracked files unchanged | Rust, Z3 |
| `native` | The equivalence test of `spec/16 §5`, `every_runnable_program_runs_natively_as_in_the_interpreter`: every runnable program of the repository (the examples, the conformance cases that run, the review corpora), in the interpreter and as a native executable, compared by status, output and trace | Rust, Z3 |
| `site` | `scripts/doc-links.py`, after its self-test: every link of the repository's documents, an image included, names a file or directory that git tracks, and every `#fragment` to one of those documents names a heading of it, by the id the website gives that heading (#708). Then (#584): `scripts/site-bundle.sh` writes the docs bundle, the documents with a manifest and the release binary's help texts, and the record lists its digest; `scripts/site-website.sh` checks that the website gives the documents' headings the ids that `scripts/doc-links.py` uses, then builds the website pinned by `site/website.lock` from that bundle, and fails on a page that does not build, a broken budget or a code sample that does not check | Python 3; Node.js 20.19 or later with npm; access to the website's repository |
| `formal` | `scripts/formal.sh`: the Lean development of the formal track (`formal/`, `docs/plan/formal-track.md`) builds with the toolchain that `formal/lean-toolchain` pins and the dependencies that `formal/lake-manifest.json` pins, and `#print axioms` of every theorem that `formal/Theorems.lean` lists shows no `sorry` and no axiom beyond `propext`, `Classical.choice` and `Quot.sound` (#46). `scripts/local-ci.sh` runs the same step, on macOS and in both Linux containers | Lean 4.34.0, from its pinned release |

The pinned Z3 comes from `UNA_GATES_Z3`, else this checkout's `target/z3`, else the clone's `target/z3`. If none is there, the `build` gate installs it with `scripts/install-z3.sh`.

At the end, the script removes the Docker volumes that `--linux` and `--linux-arm64` made for the clone's target directory, so volumes do not pile up.

**The job count in the containers** (#701). By default, Cargo inside a container runs one job per CPU of the Docker VM. On a VM with 8 GB and 10 CPUs, ten rustc and linker processes at once have run it out of memory: the kernel killed the linker with signal 9, the test binaries did not link, and no test ran.

`scripts/local-ci.sh --linux` and `--linux-arm64` therefore pass a job count into the container, which `scripts/container-jobs.sh` computes:
- The build runs one job per 2 GiB of the VM's memory (`docker info`'s `MemTotal`), at most one per CPU and at least one. That is 3 jobs on the VM above.
- A `CARGO_BUILD_JOBS` set on the host can lower that count, never raise it. A higher value, such as one meant for the host's own builds, is capped, and the run says so.
- `RUST_TEST_THREADS`, when set on the host, goes in too.

The run prints the job count before the container starts, and the container's peak memory (`memory.peak` of its cgroup) at the end. The job count changes only the build's peak memory and its time, never what is built or tested.

Why 2 GiB: a measured run of `--linux-arm64`, cold, on that VM (7834 MiB, 10 CPUs) with 5 jobs (one per 1.5 GiB) passed with a peak of 6841 MiB, 87% of the VM. That figure includes page cache, which the kernel reclaims before it kills a process, but the margin is narrow. These runs are evidence: a linker killed for memory costs a full rerun and leaves a failed run in the record, which outweighs the slower build of 3 jobs. A lower `CARGO_BUILD_JOBS` set on the host is passed in as it is, for a one-off run that wants even less memory. The VM's memory is the host's setting, and the script does not change it.

**Reproducible archives** (#679). The binaries were already reproducible: `scripts/release.sh` remaps the build's paths (#382), and the linkers derive the macOS binary's `LC_UUID` and the Linux binary's build ID from its content, so both are equal when the inputs are. Two builds of one commit, from fresh clones at different paths and cold target directories, gave byte-identical binaries on `aarch64-apple-darwin` and, in Docker, on `aarch64-unknown-linux-gnu`. Three things made two archives of one commit differ, and `scripts/release.sh` now fixes each:

- each tar entry had the time the files were copied, so every entry now takes the commit's time, `SOURCE_DATE_EPOCH` when it is set, else `git log -1 --format=%ct`. A build that finds neither stops;
- each entry named the account and group that built it, so every entry is now owned by `0:0` with no user or group name, which `scripts/check-release.sh` checks;
- the gzip header had the packing time, so `gzip -n` now stores none.

The entries are also packed in byte order of their paths, with modes 755 for directories and `una` and 644 for the other files, so the order of a directory listing and the umask do not matter. On macOS, extended attributes, ACLs and resource forks are left out. The containers of the Linux builds cannot read the commit's time when the checkout is a worktree, so `scripts/local-release.sh` and `scripts/local-ci.sh` pass `SOURCE_DATE_EPOCH` in. The `reproducible` gate costs a second `release` gate, so a full run of the gates takes about that much longer.

## 3. The record

The script writes into `OUT`:

- `results.tsv`: a header line, then one line per gate run, tab-separated: `gate`, `sha`, `result` (`PASS` or `FAIL`), `seconds`, `log_sha256`.
- `logs/<gate>.log`: each gate's full output. Its SHA-256 is in `results.tsv` and in the record.
- `record.md`, the full validation record:
  - the commit, its subject, the run's date, platform, `rustc` and Docker, and the SHA of the script that ran;
  - a table of the gates, with result, seconds and log digest;
  - the archives that the `release` gate packed, with their SHA-256;
  - the verdict:
    - `PASS (every gate)`;
    - `FAIL (<gates>)`;
    - `INCOMPLETE (not run: <gates>)`.
- `summary.md`, the public summary, which `scripts/release-summary.py` writes from `record.md` by a fixed template (#717):
  - the commit, the run's date, operating system and architecture, and the `rustc` and Docker versions;
  - the table of the gates, with result and seconds;
  - the archives with their SHA-256;
  - the verdict, as in the record.

  Nothing else of the record is copied: no subject, no notes, no log digest. Each field is checked against its shape before it is written, and a field out of shape stops the summary, so no free text reaches it. When `UNA_PRIVACY_PATTERN` is set, the summary must not match it; when it is unset, the summary is marked unchecked.
- `release-notes.md`: the summary, then the version's `CHANGELOG.md` section that `scripts/local-release.sh` wrote.

For a tag, `summary.md` is committed as `docs/validation/<version>.md`, unchanged. The full record and the logs stay private. `docs/validation/repacked.md` lists the macOS archives of v0.3.3 to v0.3.13, which were packed again after their release, each with its gated and its published digest and the check that paired them, and the withdrawn macOS archives of v0.3.0 to v0.3.2.

## 4. The procedure for a release

1. Merge the release pull request under rule 1, and note the SHA of `main`. The release pull request sets the version, runs `scripts/changelog-assemble.sh`, which writes every fragment of `changelog.d/` under `## Unreleased` in a fixed order (the issue number, then the file name) and removes the fragments (#744), and renames `## Unreleased` to `## v<version>` with the release's introduction, leaving an empty `## Unreleased` above it.
2. Run `scripts/release-gates.sh <sha> <out>` on that SHA, with Docker running and nothing else using it.
3. If the verdict is PASS and the summary was written with `UNA_PRIVACY_PATTERN` set, commit `<out>/summary.md` as `docs/validation/<version>.md` (and, until the repository is public, `<out>/record.md` as `docs/releases/<version>-validation.md`) in a pull request that changes nothing else, under rule 1.
4. Check rule 2 on the merged record commit: its own change is the summary, and `git diff --name-only <gated> <record commit>` lists only files under `docs/`. Once the release is approved, tag `v<version>` on the record commit and push the tag.
5. Publish the archives from `<out>/clone/target/local-release/v<version>/`, checking that their digests match the summary's, with the `gh release create` command that `scripts/release-gates.sh` prints at its end. The summary is the release's notes, followed by the `CHANGELOG.md` section, and is attached as `una-<version>-validation.md`. No command is printed for an unchecked summary, and an unchecked summary is not published.

Once tagged, a release's `## v<version>` section of `CHANGELOG.md` is frozen: its release notes were published from the tag, and `scripts/check-released-sections.sh` fails when the section on a branch differs from the tag's (#645). A correction to a released version, a security advisory included, goes in a changelog fragment whose bullet starts `- **Errata for v<version>: …**`, and ships with the next release's notes. Changing a released section needs a decision recorded as an ADR, which also changes the check.
