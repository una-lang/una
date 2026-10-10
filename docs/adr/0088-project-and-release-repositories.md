# 0088. The project repository and the public release repository

- Status: Proposed
- Date: 2026-10-10
- Deciders: the owner
- Resolves: #483
- Related: #246 (public launch), #242 (signed macOS binaries)

## Context

The owner decided on 2026-10-08:
- The current repository stays the **project repository**: private, with its full history, its coordination and review traces, and the release assets of v0.1.x and v0.3.0 to v0.3.2. Those assets carry build paths (S-13), and they stay as they are.
- **v1.0 launches in a separate, public release repository.** It holds no personal path or user name, no handle, no word of the neutrality list, and no internal history.
- From the moment the release repository exists, **every change in either repository is reflected in the other**.
- This architecture must be ready before v1.0.

This ADR designs the pipeline. It creates no repository and changes no remote setting.

**What the project repository holds today** (`git ls-files`, 2026-10-10):

| Paths | Files | Internal content |
| --- | --- | --- |
| `crates/`, `formal/`, `examples/`, `scripts/`, `site/`, top-level files | ~400 | Two `crates/` files and three `formal/` files name a session or the owner. One script holds the S-13 home-path pattern, as its own check. |
| `tests/` | 2,868 | 216 files name a reviewing session, in a test's description line. 29 name the owner. |
| `docs/spec/`, `docs/guide/`, `docs/compiler/`, `docs/00-charter.md` | 32 | 4 spec files name the owner. |
| `docs/adr/` | 88 | 72 name the owner and 30 a session, in Deciders lines and attributions. Accepted ADRs are immutable. |
| `docs/reviews/` | 3,628 | Review runs, agent transcripts and logs, adversarial programs. |
| `docs/plan/`, `docs/history/`, `docs/fixes/`, `docs/releases/` | 38 | Planning, coordination, validation records with machine paths. |
| `CLAUDE.md` | 1 | The agents' working instructions. |

The commit history has 895 commits by two identities, with agent attributions and internal references in their messages.

## Decision

### 1. Three classes of path, in one checked-in mirror map

`scripts/mirror-map.toml` (to be written) lists every path pattern of the project repository in exactly one class. A path that matches no pattern is **never** mirrored, and the gate fails if one appears in the release tree.

| Class | Paths | Sync |
| --- | --- | --- |
| **Two-way** | `crates/`, `tests/`, `examples/`, `formal/`, `docs/spec/`, `docs/guide/`, `docs/compiler/`, `docs/00-charter.md`, `site/`, `.github/`, the release scripts (`release.sh`, `check-release.sh`, `local-release.sh`, `release-gates.sh`, `local-ci.sh`, `install-*.sh`), `README.md`, `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `install.sh`, the Cargo files, `verification.lock`, `rust-toolchain.toml`, `clippy.toml` | Byte-identical in both repositories. They are written neutral at the source (§2). |
| **One-way, scrubbed** | `docs/adr/` and `CHANGELOG.md` | Project to release only. A scrub table (§2) maps names to roles, and the release repository refuses pull requests that touch these paths: a change goes through the project repository. |
| **Never** | `docs/reviews/`, `docs/plan/`, `docs/history/`, `docs/fixes/`, `docs/releases/`, `coordination/`, `CLAUDE.md`, `images/` except the logo, `target/`, every release asset of the project repository | Stays private. A public summary may be written in a two-way path. |

The CHANGELOG starts afresh in the release repository at v1.0, with one summary entry for everything before it. Its later entries are scrubbed one way.

### 2. Neutral at the source, a scrub table only where text is immutable

**Options:**
- **(A) Neutral at the source.** The two-way paths are cleaned once, before the first sync: a test's description says "a reviewer's matrix", not a session's name, and the spec names roles. The project CI then runs the leak gate (§3) on those paths at every merge, so they stay clean. Sync is then a plain copy, the same in both directions.
- **(B) A transform at every sync.** A scrub table rewrites names on the way out. That makes two-way sync a transform with an inverse, which every public change must survive, so it is fragile.
- **(C) Mirror nothing that names anyone.** Leaving out `tests/` and `docs/adr/` would publish a compiler without its conformance suite or its design record.

**Recommendation: (A) for the two-way class, and (B) only for the one-way class.** Accepted ADRs are immutable (CLAUDE.md), so their names cannot be removed at the source. Their mirror is a one-way projection through a small scrub table: the owner's name and each session's name become a role, such as "the owner" or "a reviewer". New ADRs, from this one on, name roles only, so the table does not grow.

The scrub table and the neutrality list are **kept outside every repository**, in the maintainer's configuration, and the gate reads them from a path given as an argument. Writing either into a repository would publish what it guards.

### 3. One leak gate for every sync and every release

`scripts/leak-gate.sh <tree-or-archive> [--lists <dir>]` (to be written) is the only gate. It fails closed: a check that cannot run is a failure. It runs:

1. **The mirror map.** Every file of the tree is in the two-way or one-way class, and nothing else is present.
2. **Paths and user names.** It scans every file, text and binary, for home-directory paths and the maintainer's user names in any form. It generalizes the check of `scripts/check-release.sh` (#382, S-13), which already refuses `/Users/` and `/home/` paths and the builder's user name in the archives.
3. **The neutrality check.** Every file and every commit message must contain no word of the neutrality list. The list is read from `--lists`, never from the repository.
4. **Handles and internal names.** The same scan, with the list of session and person names, read from `--lists`.
5. **Secrets.** Tokens, keys, e-mail addresses other than the public contact, and URLs of private services.
6. **For an archive,** `scripts/check-release.sh`'s S-13 checks as they are.

It runs at these points:
- every forward sync, on the outgoing tree and its commit message;
- every import back, on the patch;
- the project CI, on the two-way paths at each merge;
- every release, on the source tree and on each archive, before the tag.

### 4. History

**Options:**
- **(A) A clean start.** The release repository's first commit is the v1.0 tree, with no parent. After it, each sync is one commit per merged project pull request. Its message is generated from a template (title, issue reference, summary), passes the gate, and carries a neutral author identity.
- **(B) A rewritten history.** `git filter-repo` keeps only the mirrored paths, applies the scrub table, and rewrites author identities. Every old blob, message and identity must then pass the gate. 895 commits of agent-written messages, with old assets in reachable history, are a large surface for one miss, and a public miss cannot be taken back.
- **(C) Replay every commit.** This is (B) without the rewrite tooling: the same surface, more work.

**Recommendation: (A).** The project repository keeps the full history for the team. The release repository's history starts clean at v1.0 and grows one reviewed commit per change. A sync manifest in the project repository (`docs/releases/sync-manifest.md`, never mirrored) records each pair of project SHA and release SHA.

### 5. Outside contributions flow back through the project repository

**Options:**
- **(A) The project repository is the only writer.** A public pull request is reviewed in the release repository, but merged in the project repository:
  1. The maintainer imports it as a project pull request, with its author kept as `Co-authored-by`, since that identity is already public, and a reference to the public pull request.
  2. The leak gate runs on the patch, then the usual local-ci.
  3. After the merge, the forward sync publishes it, and the public pull request is closed as merged by that commit.
  4. Every change, from either side, then lands once and flows one way at a time, so the two trees cannot diverge. "Reflected in the other" holds with a delay of one sync.
- **(B) Both repositories accept merges, and a bot reconciles them.** It is faster for contributors, but two writers on the same files need conflict resolution in automation, and a public merge could bypass the project's gates.

**Recommendation: (A).** Contributors sign their commits off (DCO, `Signed-off-by`), which `CONTRIBUTING.md` will require in the release repository, because the project repository's licensing record must show where each contribution came from. Issues opened in the public repository stay there. The project's board links them.

### 6. Signing and provenance

**Recommendation:**
- Release tags in the release repository are signed, with an SSH or GPG key that only the owner holds.
- Each archive gets a SHA-256 file, as today, plus a signature of the checksum file, with minisign or Sigstore cosign: the owner picks one.
- macOS binaries are signed and notarized (#242).
- The sync commits are signed with a dedicated sync key, which is listed in the release repository's `SECURITY.md`, so a reader can tell a synced commit from a forged one.
- When the release repository has its own CI, it attaches build provenance (a SLSA attestation) to each archive. Until then, the validation record of `scripts/release-gates.sh`, run on the exact SHA, is published with the release, scrubbed through the gate.

### 7. Where releases are cut

**Options:**
- **(A) In the project repository, then mirrored.** The tag lives where the validation ran, but the public SHA differs from the validated one, so a user cannot tie the two.
- **(B) In the release repository, on the synced commit.**
  1. `scripts/release-gates.sh` runs on a checkout of the release repository at the exact SHA to be tagged.
  2. `scripts/local-release.sh` builds the archives from that checkout, and the leak gate runs on each.
  3. The signed tag and the GitHub release are made there.
  4. The project repository records the validation and the SHA pair in the sync manifest.

**Recommendation: (B).** A user verifies what they download against public history, and the archives are built from exactly the public tree, so the S-13 class of leak cannot come from private files. The project repository keeps cutting its own pre-v1.0 tags (v0.x) as it does today, and they are never published.

### 8. Readiness before v1.0

These steps are tracked as issues once this ADR is accepted:
1. `scripts/leak-gate.sh` and `scripts/mirror-map.toml`, with tests: a seeded leak of each kind must fail the gate.
2. The one-time neutral pass over the two-way paths: test descriptions, the spec's 4 mentions, and the five code files.
3. The gate in the project CI, on the two-way paths.
4. A dry run: build the release tree in a scratch directory, run the gate, local-ci and `scripts/release-gates.sh` on it, and compare it with the project tree. No repository is created.
5. The owner creates the release repository and keys, and sets its settings. This ADR asks for no remote change.

**Complexity.** The gate is one linear pass over the tree's bytes for each list, O(n · k) for n bytes and k list entries, or O(n) with one combined automaton (Aho-Corasick). A sync copies the two-way paths, O(n). For about 4,000 files these are seconds.

## Consequences

- The project repository keeps working as it does, with one new rule: two-way paths are written neutral, and its CI enforces that.
- The release repository's history is short and clean, and its archives are built from public sources only.
- Outside contributors wait one sync for their merged change to appear, and they sign off their commits.
- The neutrality list and the scrub table live outside every repository, so losing the maintainer's configuration stops the sync, closed, rather than leaking.

## Alternatives considered

The options of §2, §4, §5 and §7 are given in their sections, each with the reason for the recommendation. Making the project repository itself public was ruled out by the owner's requirement: its history and old assets stay private.
