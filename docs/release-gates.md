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
| `deps` | `scripts/deps-audit.sh`, the dependency audit (#770): `cargo-deny` at the version that `supply-chain/tools.lock` pins, against the RustSec advisory database fetched for the run, checks advisories, licenses, bans and sources by `deny.toml`; then the supply-chain review record `supply-chain/crates.tsv` must list exactly the crates.io packages of `Cargo.lock` | `cargo-deny` at its pinned version; network access to the advisory database |
| `mac` | `scripts/local-ci.sh`: solver, the released sections of `CHANGELOG.md` against their tags (#645): the gate's clone has the tags of its source, and the gates check only the tags the gated commit contains (`UNA_RELEASED_SECTIONS_SCOPE=ancestors`), so a past release's commit can be gated again after newer tags exist, `fmt`, `clippy -D warnings`, tests, the host's release archive packed and checked | macOS |
| `linux` | `scripts/local-ci.sh --linux`: the same steps in a `linux/amd64` container of CI's Ubuntu image, emulated on an arm64 Mac; its tests run one at a time on one thread (`RAYON_NUM_THREADS=1`, `--test-threads=1`), since processes spawned from many threads at once can hang under the emulation (#286). It checks the amd64 build, tests and archive. The released sections of `CHANGELOG.md` are checked on the host before the container starts, since a worktree mounted alone has no tags there, and the container's run skips them, visibly | Docker |
| `linux-arm64` | `scripts/local-ci.sh --linux-arm64`: the same steps in a `linux/arm64` container, native on an arm64 Mac (#241), with the tests at full parallelism: the primary evidence of the tests on Linux (#286) | Docker |
| `release` | `scripts/local-release.sh v<version>` on a tag made in the clone only (never pushed): the three archives (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), each checked with `scripts/check-release.sh` | macOS, Docker |
| `reproducible` | `scripts/local-release.sh v<version>` a second time, from cold: in a new checkout at a new path, with new Docker volumes for the Linux target directories. Each archive's sha256 is compared with the one the `release` gate built, and every pair must be equal (#679). Its cost is a second `release` gate: three release builds, one of them emulated | macOS, Docker |
| `determinism` | The conformance suite twice with equal summaries and no failure; `una --json check` of every conformance case and example twice, the outputs hashed and compared; the tracked files unchanged | Rust, Z3 |
| `native` | The equivalence test of `spec/16 §5`, `every_runnable_program_runs_natively_as_in_the_interpreter`: every runnable program of the repository (the examples, the conformance cases that run, the review corpora), in the interpreter and as a native executable, compared by status, output and trace | Rust, Z3 |
| `sanitize` | `scripts/sanitize.sh` (#766), with the checking toolchain that `scripts/check-toolchain.toml` pins, a nightly Rust used only for checking and never to build an artifact. Miri runs the unit tests of `una-rt`, whose `ffi.rs` holds the workspace's only `unsafe`; with permissive provenance, since the runtime's handles are integers (a known weakening of Miri's checks). The tests it does not run are ignored with their reason, and the script fails if that set changes, by name (`scripts/sanitize-miri-ignored.txt`), or if a test of `una-rt` neither runs nor is in it. Then `una-rt` is built with AddressSanitizer, which instruments it, its crate dependencies and the generic standard-library code compiled into them, but not the precompiled standard library, whose libc calls are only intercepted; its unit tests run at their full sizes under it, and the equivalence test of the `native` gate runs every runnable program against it; the link adds the AddressSanitizer runtime through a `cc` placed on that one command's `PATH`, so `una` has no hook for it, and a canary fails the gate unless AddressSanitizer is active and the linked runtime is the instrumented one. A missing or different checking toolchain fails the gate, never skips it. macOS only for now | macOS; the checking toolchain (`scripts/install-check-toolchain.sh`) |
| `hunt` | The security hunt's fixed run (`scripts/security-hunt`, `python3 -m hunt gate`, #767), with the clone's release `una`. See "The hunt gate" below | macOS (`sandbox-exec`), Python 3 |
| `site` | `scripts/doc-links.py`, after its self-test: every link of the repository's documents, an image included, names a file or directory that git tracks, and every `#fragment` to one of those documents names a heading of it, by the id the website gives that heading (#708). Then (#584): `scripts/site-bundle.sh` writes the docs bundle, the documents with a manifest and the release binary's help texts, and the record lists its digest; `scripts/site-website.sh` checks that the website gives the documents' headings the ids that `scripts/doc-links.py` uses, then builds the website pinned by `site/website.lock` from that bundle, and fails on a page that does not build, a broken budget or a code sample that does not check | Python 3; Node.js 20.19 or later with npm; access to the website's repository |
| `formal` | `scripts/formal.sh`: the Lean development of the formal track (`formal/`, `docs/plan/formal-track.md`) builds with the toolchain that `formal/lean-toolchain` pins and the dependencies that `formal/lake-manifest.json` pins, and `#print axioms` of every theorem that `formal/Theorems.lean` lists shows no `sorry` and no axiom beyond `propext`, `Classical.choice` and `Quot.sound` (#46). `scripts/local-ci.sh` runs the same step, on macOS and in both Linux containers | Lean 4.34.0, from its pinned release |

**The hunt gate** (#767). A fixed, seeded run of the security hunt's generators: seed 767, every operator, 50 programs per operator in editions `0.1` and `0.2`, so 5,000 programs, each rebuilt from its seed and checked against its digest. Measured on an Apple M1 Max (10 cores, 32 GB), four compiler runs at a time, the run took 106 seconds on a quiet machine, and 151 seconds under another load (2026-10-15).

- **Must-reject:** a program built to break a flow rule must be rejected by `una check --tier 0`. An accepted one is a finding.
- **Must-keep:** two refactors that keep every label (a rename, a reordering of declarations) must not turn a rejected program into an accepted one.
- **The two-run oracle:** for an accepted program with a literal labeled above `public`, the literal is replaced by up to five other values of its type, in a fixed order, and every variant runs on the interpreter (`una run --json`) twice, each run in a directory of its own. What an observer below the literal's label sees, at `public` and at `internal`, must be the same in every variant that completes (`spec/07 §1`):
  - `status`, every `output` line, `failure` (its variant and its public fields), and the diagnostics by severity and code;
  - the `trace` events of the operations whose `accepts` the observer sees, as the model defines what an observer sees, in order. The `accepts` of host operations is the prelude's, pinned by a test; that of the program's own capabilities comes from its Core export. An operation the oracle cannot classify is an error, and an `accepts` it cannot read exactly counts as visible;
  - of each such event, every field but `seq`, which numbers the hidden events too, and `outcome`, which is what the host returned to the program, an input of the run and not something the program sends; the arguments that decide it are compared. Of `Keys.hmac`, only `key_name`, its one sink, is compared.

  Every variant that completes is compared with the program's first end that completed, the base's or a variant's, so an aborted base hides nothing. An argument of a visible event that the run redacted at a level above the observer is a finding by itself, whatever the other variants show: the run says that a value above what the operation accepts reached it. A program whose policy releases to `public` is set aside and counted, since the release explains a difference.
- **Termination is recorded, not judged.** A pair of variants where one completes (`ok` or `failed`) and the other aborts with U6003, U6004 or U6005, or reaches the run's time limit, is outside the noninterference guarantee (`spec/07 §1`, ADR 0061). The gate prints such pairs, `summary.json` lists them, and the record carries their count. Two variants that complete with different statuses are compared as public output.
- **It fails closed.** The gate fails:
  - on any finding;
  - on any check or run that ends in another way: a timeout of `una check`, an internal error, unreadable output, a signal, a completed run with a diagnostic other than its failure's errors, or a variant the checks reject;
  - on two runs of one variant whose documents differ in any byte, exit code or timeout;
  - when a driver judged a number of cases other than the pinned one, or none, or the oracle's comparisons are under their floors;
  - when its output directory is not empty.
- **Its size.**
  - **Pinned counts:** 5,000 programs judged by must-reject, 8,594 (program, refactor) pairs by must-keep, and 434 programs with a varied literal by the oracle (`PINNED` in `scripts/security-hunt/hunt/gate.py`, checked against the generators by a unit test).
  - **Floors on the oracle**, about 90% of the measured run, against a run that compares little: only comparisons of two complete variants count. At least 300 programs with such a comparison (measured: 334), 1,000 such comparisons (measured: 1,202), and 90 live programs (measured: 100). A program is live when two of its complete variants differ at the literal's own level, where every event is visible with its outcome: its literal decides something, so its comparisons are not vacuous. The run document redacts every value above `public`, so liveness sees only what a literal decides in the control flow, such as whether an operation runs: of the measured run's programs, those of H3 are live, and those of K4, P1 and T8, whose secret reaches only redacted values, are not counted live although they may be compared well. 100 programs were set aside: 74 whose policy releases to `public` and 26 the checks reject.
- **Canaries.** Four planted faults run through the same code, and a canary that is not caught fails the gate:
  - a program the checks accept, labeled must-reject, which the must-reject driver must report;
  - a program that prints a public literal its manifest marks as secret, whose variants the oracle must tell apart at `public`;
  - a program that calls an operation accepting `secret` under a secret branch, which the gate plants as accepting `internal`: the oracle must report it at `internal` only;
  - a program that sends a secret to an operation accepting `secret`, which the gate plants as accepting `public`: every variant shows the same redacted argument, and the oracle must report the redacted sink by itself.
- **Evidence:** `OUT/hunt/summary.json` (the verdict, the seed, the case counts, the `una` tip, the findings, errors, termination pairs and canaries) and one result file per driver. The record's `## Hunt` section summarizes it.
- **Scope.** macOS only: every compiler run is under `sandbox-exec`, with one writable directory, no network and a clean environment, and there is no unsandboxed fallback. The oracle varies source literals only, runs the interpreter only and observes at `public` and at `internal`; host inputs, the files a program writes, the native backend and the re-check comparison are not part of the gate (#782).

The pinned Z3 comes from `UNA_GATES_Z3`, else this checkout's `target/z3`, else the clone's `target/z3`. If none is there, the `build` gate installs it with `scripts/install-z3.sh`.

**The dependency audit** (#770). The `deps` gate fails closed on each of these:
- `cargo-deny` is missing or at another version than `supply-chain/tools.lock` pins. Install it with `cargo install --locked cargo-deny@<version>`. The log records its version and the SHA-256 of its binary. That digest is a record, not a pin: `cargo install` builds are not reproducible, so the gate compares only the version.
- The advisory database cannot be fetched, offline included: the gate fails and is never skipped. A fetch can also succeed and leave an old database, from a cached clone or a stalled mirror, so the gate fails when the database's last commit is more than 7 days old by the current UTC time, and prints its age. The commit and date of the database it fetched go into the record and the public summary, so the run can be repeated against the same advisories.
- An advisory of that database applies to a package of `Cargo.lock`, a yanked version or an unmaintained crate included. `deny.toml` ignores none. An ignore needs its own pull request with a reason, an expiry date and a recorded ruling, and a finding is reported before any ignore is written. A fix updates the dependency (S-26, #779).
- A license outside `deny.toml`'s allowlist, a wildcard version requirement on a registry crate, or a package from any source but crates.io. The workspace's own crates depend on each other by path; `allow-wildcard-paths` admits those, and only because every workspace crate has `publish = false`.
- A package of `Cargo.lock` with no source that is not a member of the workspace, by name and version (`cargo metadata --no-deps`), or a member whose manifest is outside the repository: a path dependency outside the workspace, or a vendored copy, has no source and no checksum, so neither cargo-deny's sources check nor the record would see it.
- A crates.io package of `Cargo.lock` that `supply-chain/crates.tsv` does not list with the same version and checksum, or a line of the record that `Cargo.lock` no longer has. `crates/una-cli/tests/dependencies.rs` checks this offline, so `scripts/local-ci.sh` catches a new or moved dependency before the gate does. A change to the record is the review: its pull request names each crate added or moved and why, and someone other than its author reviews it. The record started with the 158 crates of `Cargo.lock` when the gate was added, after S-26's update, as the baseline, not as a review of each of them.

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
  - the dependency audit: the `cargo-deny` version and the advisory database's commit and date that the `deps` gate used;
  - the verdict:
    - `PASS (every gate)`;
    - `FAIL (<gates>)`;
    - `INCOMPLETE (not run: <gates>)`.
- `summary.md`, the public summary, which `scripts/release-summary.py` writes from `record.md` by a fixed template (#717):
  - the commit, the run's date, operating system and architecture, and the `rustc` and Docker versions;
  - the table of the gates, with result and seconds;
  - the archives with their SHA-256;
  - the dependency audit's tool and advisory database, as in the record;
  - the verdict, as in the record.

  Nothing else of the record is copied: no subject, no notes, no log digest. Each field is checked against its shape before it is written, and a field out of shape stops the summary, so no free text reaches it. The summary must not match the privacy pattern: `UNA_PRIVACY_PATTERN` when it is set, else the macOS Keychain item `una-privacy-pattern`, which `scripts/release-gates.sh` reads into a shell variable that it never exports, unsetting the caller's variable, so that no gate inherits it, and hands only to the summary step; it never prints, logs or writes it. Without either, the script stops before any gate; `UNA_GATES_UNCHECKED=1` runs the gates without the pattern: such a run's summary opens with "Unchecked: written without the privacy pattern; not for publication", and no publish command is printed.
- `release-notes.md`: the summary, then the version's `CHANGELOG.md` section that `scripts/local-release.sh` wrote.

For a tag, `summary.md` is committed as `docs/validation/<version>.md`, unchanged. The full record and the logs stay private. `docs/validation/repacked.md` lists the macOS archives of v0.3.3 to v0.3.13, which were packed again after their release, each with its gated and its published digest and the check that paired them, and the withdrawn macOS archives of v0.3.0 to v0.3.2.

## 4. The procedure for a release

1. Merge the release pull request under rule 1, and note the SHA of `main`. The release pull request sets the version, runs `scripts/changelog-assemble.sh`, which writes every fragment of `changelog.d/` under `## Unreleased` in a fixed order (the issue number, then the file name) and removes the fragments (#744), and renames `## Unreleased` to `## v<version>` with the release's introduction, leaving an empty `## Unreleased` above it.
2. Run `scripts/release-gates.sh <sha> <out>` on that SHA, with Docker running and nothing else using it.
3. If the verdict is PASS and the summary was written with `UNA_PRIVACY_PATTERN` set, commit `<out>/summary.md` as `docs/validation/<version>.md` (and, until the repository is public, `<out>/record.md` as `docs/releases/<version>-validation.md`) in a pull request that changes nothing else, under rule 1.
4. Check rule 2 on the merged record commit: its own change is the summary, and `git diff --name-only <gated> <record commit>` lists only files under `docs/`. Once the release is approved, tag `v<version>` on the record commit and push the tag.
5. Publish the archives from `<out>/clone/target/local-release/v<version>/`, checking that their digests match the summary's, with the `gh release create` command that `scripts/release-gates.sh` prints at its end (`scripts/release-publish.sh`). A pre-release, a version with a suffix such as `0.4.0-alpha.1`, is published with `--prerelease --latest=false`. The summary is the release's notes, followed by the `CHANGELOG.md` section, and is attached as `una-<version>-validation.md`. No command is printed for an unchecked summary, and an unchecked summary is not published.

Once tagged, a release's `## v<version>` section of `CHANGELOG.md` is frozen: its release notes were published from the tag, and `scripts/check-released-sections.sh` fails when the section on a branch differs from the tag's (#645). A correction to a released version, a security advisory included, goes in a changelog fragment whose bullet starts `- **Errata for v<version>: …**`, and ships with the next release's notes. Changing a released section needs a decision recorded as an ADR, which also changes the check.
