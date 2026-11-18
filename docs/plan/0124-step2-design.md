# ADR 0124 step 2: the runtime inside `una` (design note)

- Issue: #1207 (part of #891, ADR 0124 rules 12 to 15, 18 to 20 and 28, Implementation plan step 2)
- Base: step 1 (#1206): `una-z3`'s build script and the build record, `una --version`'s `development build` line, `scripts/release.sh`'s embedding of the Z3 library by digest, `scripts/check-release.sh`'s refusal of a development build.
- Scope: no construct of the language changes. `una build` keeps linking through the system `cc` until step 3; what changes is where the runtime comes from. The existing suite passes unchanged in development builds.

This note fixes how step 2 is built, and lists the decisions it takes where the record leaves a choice. Each decision takes the most secure option the record allows.

## 1. What step 2 delivers

1. The release `una` carries `libuna_rt.a`, built from the same commit and the same build as `una`, without debug sections (rule 12), and its SHA-256 is part of the build record (rule 20) and of the report (rule 28).
2. `una build` of a release `una` reads the runtime from the binary, never from a file: `UNA_RT_LIB` and the search of ADR 0059 rule 2 hold for development builds only (rule 15). The failure of #203, a runtime older than `una`, cannot occur in a release build (rule 14).
3. Until step 3, a release `una` writes the embedded runtime into the private directory of the link, created for that build, and removes it with the directory after the link (Implementation plan step 2).
4. The archive holds `una`, `LICENSE`, `README.md` and the guides, and no `lib/` (ADR 0059 amendment); from such an archive the installer installs no `lib/libuna_rt.a` and records none. It still installs and records the `lib/libuna_rt.a` of an archive of an earlier release, whose `una` reads its runtime from there, and keeps the record of one an earlier installation put there, so that `--uninstall` removes it (as built in #1354; the removal is the migration of step 5).
5. `scripts/release.sh` builds the runtime first, then `una` (rule 18); `scripts/check-release.sh` checks the archive's new contents and that a release `una` ignores `UNA_RT_LIB`.

Not in step 2: the symbol index per member (rule 12), which only the internal linker reads; it is built with step 3, from the same embedded bytes, so that step 2 embeds nothing it does not use.

## 2. The build order and the embedding

**Decision D1: the runtime is embedded by `una-cli`'s own build script, from a file whose digest the release script states, never built by that script.** `scripts/release.sh` runs, in this order:

1. `cargo build --release --locked -p una-rt` into the release target directory, with the remapped paths of today;
2. the debug sections are removed from the archive (`strip -S` on macOS, `strip --strip-debug` on Linux, both from the pinned toolchains' systems; the tool is recorded in the validation summary), so that no path or line table of the building machine reaches an executable and the embedded bytes are smaller (rule 12);
3. `UNA_EMBED_RUNTIME=<path> UNA_EMBED_RUNTIME_SHA256=<sha256 of the stripped archive>` are exported, beside `UNA_EMBED_Z3` and `UNA_EMBED_Z3_SHA256` of step 1;
4. `cargo build --release --locked -p una-cli`.

`crates/una-cli/build.rs` (new, the only build script of `una-cli`; `una-z3`'s stays as it is) reads `UNA_EMBED_RUNTIME` and `UNA_EMBED_RUNTIME_SHA256`: with both, it reads the file, checks its SHA-256 against the given digest and fails the build on a mismatch, copies the bytes into `OUT_DIR/libuna_rt.a`, and sets `cargo:rustc-cfg=una_runtime_embedded` and `cargo:rustc-env=UNA_EMBED_RUNTIME_SHA256=<digest>`; with one of the two variables, it fails; with neither, it sets nothing, and the build is a development build. It declares `rerun-if-env-changed` for both and `rerun-if-changed` for the file. It runs no program, downloads nothing and compiles nothing (rule 18). The crate embeds the bytes with `include_bytes!(concat!(env!("OUT_DIR"), "/libuna_rt.a"))` under `cfg(una_runtime_embedded)`, in a new module `crates/una-cli/src/runtime_archive.rs`, which is the only place that names the bytes.

- **Why the build script, and not a copy at packing time.** Rule 18 makes the digest check part of the build: a runtime that is not the one the release script hashed cannot be linked into `una`, and the digest the binary records is the digest the build checked.
- **Why a copy into `OUT_DIR`.** `include_bytes!` takes a path fixed at compile time; the copy makes the embedded file a build artifact of `una-cli`, so a later `cargo build` with another `UNA_EMBED_RUNTIME` re-embeds (the `rerun-if` lines), and a file edited under the same path is caught by the digest.
- **Development builds** (rule 19) embed nothing: `una --version` ends with `development build`, which from this step means "no release solver or no release runtime"; `crate::development_build()` becomes `!una_z3::BUILD.release || !cfg!(una_runtime_embedded)`.

## 3. How `una build` uses the embedded runtime

**Decision D2: the runtime is written into the link's private directory, which already exists for the executable, with `create_new` (O_EXCL) and mode 0600, and the directory is removed on every path.** `crates/una-cli/src/build.rs`:

- `runtime_library(environment)` becomes `runtime(environment, private: &Path) -> Result<PathBuf, String>`:
  - under `cfg(una_runtime_embedded)`: writes `runtime_archive::BYTES` to `private/libuna_rt.a` through `OpenOptions::new().write(true).create_new(true).mode(0o600)` and returns that path; it reads no variable and no other file, so `UNA_RT_LIB`, a `lib/` beside `una` or a runtime next to it change nothing (rule 15; conformance: "`UNA_RT_LIB` naming a runtime that prints an extra line has no effect on a release `una`");
  - otherwise: today's search, `UNA_RT_LIB`, then `lib/` beside `una`, next to `una`, `../lib` (ADR 0059 rule 2, development builds only), with #203's hint.
- The private directory is `una_diag::safe_write::private_dir(&write_root, &output)` of `placed` (#1160): mode 0700, fresh for each build, inside `target/una` of the package or the typed `--out` directory, and dropped after the executable is installed or on any error, so the written runtime goes with it. `cc` reads the runtime from there. Nothing else is written to disk (rule 13 applies in full only from step 3, which links in memory).
- The link command is unchanged: `cc <object> <runtime> -o <produced> -Wl,-S -Wl,-dead_strip|--gc-sections` and the Linux system libraries.
- **Cost:** one write of the runtime's bytes per `una build` (about 10 MB, see rule 30's estimate) and one read by `cc`, O(size of the runtime); step 3 removes the write.

## 4. The record and the report

- The build record (`una_cli::build_record`, rule 20) gains `runtime_sha256`, the digest the build script checked, empty in a development build; the handshake of step 1 hashes the record, so a release with another runtime has another handshake, as rule 4 intends.
- The report's `native` section (rule 28, `spec/13 §6`) gains `runtime_sha256`, present when the runtime is embedded; `linker` and `debuggable` come with step 3. The key is additive, and the schema stays `una.report/2`.
- `una --version` keeps its text and keys; the `development build` line covers the runtime too (section 2).

## 5. The archive, the installer and the gates

- `scripts/release.sh`: no `lib/`; the archive holds `una`, `LICENSE`, `README.md` and `docs/guide/*.md`; the packing rules of #679 are unchanged.
- `scripts/check-release.sh`: expects `LICENSE README.md docs una`; the marker and AddressSanitizer checks run on `una` alone; the native build of every example uses the packed `una` with no `lib/` and no `UNA_RT_LIB`; a new check sets `UNA_RT_LIB` to a file that is no archive and builds `examples/hello`: a release `una` must still link, since it reads no variable (rule 15); and the two-places build of #382 keeps its meaning with no runtime file beside `una`.
- `install.sh`: installs `bin/una` and the documents, records no `lib/`, and checks that the archive holds `una` alone among binaries; its tests expect no `lib/libuna_rt.a` in a fresh installation. The removal of an earlier installation's `lib/libuna_rt.a` (and `z3/`) is the migration of step 5.
- `scripts/local-ci.sh` and the tests: development builds keep reading the runtime from `target/debug` or `UNA_RT_LIB`, as `crates/una-cli/tests/common/native_build.rs` arranges; no test changes its mechanism. The `native` gate, the `release` and `reproducible` gates (same bytes from the same inputs; the embedded runtime is one more input with a stated digest) and the `determinism` gate keep running as they are. The `toolchain-free` gate of step 1 does not yet run `una build` (that is step 3).

## 6. Records amended

- ADR 0059: the amendment of Appendix A of ADR 0124 ("Release archives no longer ship the runtime: it is built into `una`; rule 2's search order holds for development builds only"), appended dated.
- `docs/security/tcb.md` §3.5: the runtime is pinned (by the build's digest) and recorded (by the report) from this step; the `cc` and linker gap stays open until step 3; §3.3's "the build report's `tcb` section does not list the runtime" closes.
- `docs/release-gates.md`: the `release` row's archive contents; `install.sh`'s header.
- Changelog fragment, and the installer's and the guide's text where they name `lib/libuna_rt.a`.

## 7. Conformance of step 2 (from the record's "Runtime" list)

- `UNA_RT_LIB` naming a runtime that prints an extra line has no effect on a release `una` (the `check-release.sh` check), and is used by a development build (today's test `una_build_finds_the_runtime_through_a_symlinked_una` and its neighbors, unchanged);
- `una --version` of a development build ends with `development build`; a release's does not (`check-release.sh`);
- an archive with a `lib/` entry is refused by `scripts/check-release.sh`;
- a fresh installation holds `bin/una` and the documents and no `lib/`.

The `--external-linker` and `dev-external-link` items of that list belong to step 3.

## 8. Checklist (the issue's, refined)

- [ ] 2a. `scripts/release.sh`: build `una-rt` first, strip its debug sections, export `UNA_EMBED_RUNTIME` and `UNA_EMBED_RUNTIME_SHA256`, then build `una-cli`; `crates/una-cli/build.rs` with the digest check, the copy into `OUT_DIR`, the cfg and the env; `runtime_archive.rs`; the build record's `runtime_sha256`; `development_build()` covering the runtime.
- [ ] 2b. `crates/una-cli/src/build.rs`: the embedded runtime written into the link's private directory with `create_new` and mode 0600 under the cfg, the development search otherwise; the report's `native.runtime_sha256`; `spec/13 §6`.
- [ ] 2c. The archive without `lib/` (`release.sh`, `check-release.sh` with the `UNA_RT_LIB` check and the `lib/` refusal), `install.sh` and its tests, ADR 0059's amendment, `tcb.md` §3.3 and §3.5, `docs/release-gates.md`, the changelog fragment.
