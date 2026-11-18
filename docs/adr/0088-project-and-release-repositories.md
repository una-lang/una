# 0088. The project repository and the public release repository

- Status: Accepted (Owner, 2026-10-10, with minisign and the neutral-text change)
- Date: 2026-10-10
- Deciders: Owner
- Resolves: #483
- Related: #246 (public launch), #242 (signed macOS binaries)

## Context

The project decided on 2026-10-08 and 2026-10-10:
- The current repository stays the **project repository**: private, with its full history, its coordination and review traces, and the release assets of v0.1.x and v0.3.0 to v0.3.2. Those assets carry build paths (S-13), and they stay as they are.
- **v1.0 launches in a separate, public release repository.** It holds no personal path or user name, no handle, no word of the neutrality list, and no internal history.
- From the moment the release repository exists, **every change in either repository is reflected in the other**.
- This architecture must be ready before v1.0.
- Every release-facing text is neutral and institutional, with no reference to a person or a role. Internal notes stay in the working version, and never appear in a release from v1.0 on.

This ADR designs the pipeline. It creates no repository and changes no remote setting.

**What the project repository holds today** (`git ls-files`, 2026-10-10):

| Paths | Files | Internal content |
| --- | --- | --- |
| `crates/`, `formal/`, `examples/`, `scripts/`, `site/`, top-level files | ~400 | Two `crates/` files and three `formal/` files name a working session or a person. One script holds the S-13 home-path pattern, as its own check. |
| `tests/` | 2,868 | 216 files name a working session, in a test's description line. 29 name a person. |
| `docs/spec/`, `docs/guide/`, `docs/compiler/`, `docs/00-charter.md` | 32 | 4 spec files name a person. |
| `docs/adr/` | 88 | 72 name a person and 30 a working session, in Deciders lines and attributions. Accepted ADRs are immutable. |
| `docs/reviews/` | 3,628 | Review runs, agent transcripts and logs, adversarial programs. |
| `docs/plan/`, `docs/history/`, `docs/fixes/`, `docs/releases/` | 38 | Planning, coordination, validation records with machine paths. |
| `CLAUDE.md` | 1 | The working instructions of the project's agents. |

The commit history has 895 commits by two identities, with agent attributions and internal references in their messages.

## Decision

### 1. Three classes of path, in one checked-in mirror map

`scripts/mirror-map.toml` (to be written) lists every path pattern of the project repository in exactly one class. A path that matches no pattern is **never** mirrored, and the gate fails if one appears in the release tree.

| Class | Paths | Sync |
| --- | --- | --- |
| **Two-way** | `crates/`, `tests/`, `examples/`, `formal/`, `docs/spec/`, `docs/guide/`, `docs/compiler/`, `docs/00-charter.md`, `site/`, `.github/`, the release scripts (`release.sh`, `check-release.sh`, `local-release.sh`, `release-gates.sh`, `local-ci.sh`, `install-*.sh`), `README.md`, `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `install.sh`, the Cargo files, `verification.lock`, `rust-toolchain.toml`, `clippy.toml` | Byte-identical in both repositories. They are written neutral at the source (§2). |
| **One-way, projected** | `docs/adr/` and `CHANGELOG.md` | Project to release only. A projection (§2) drops every attribution, and the release repository refuses pull requests that touch these paths: a change goes through the project repository. |
| **Never** | `docs/reviews/`, `docs/plan/`, `docs/history/`, `docs/fixes/`, `docs/releases/`, `coordination/`, `CLAUDE.md`, `images/` except the logo, `target/`, every release asset of the project repository | Stays private. A public summary may be written in a two-way path. |

The CHANGELOG starts afresh in the release repository at v1.0, with one summary entry for everything before it. Its later entries are projected one way, as neutral text.

### 2. Neutral at the source, a projection only where text is immutable

**Options:**
- **(A) Neutral at the source.** The two-way paths are cleaned once, before the first sync: a test's description states what it covers, with no name and no role, and the spec names no one. The project CI then runs the leak gate (§3) on those paths at every merge, so they stay clean. Sync is then a plain copy, the same in both directions.
- **(B) A transform at every sync.** A scrub table rewrites names on the way out. That makes two-way sync a transform with an inverse, which every public change must survive, so it is fragile.
- **(C) Mirror nothing that names anyone.** Leaving out `tests/` and `docs/adr/` would publish a compiler without its conformance suite or its design record.

**Recommendation: (A) for the two-way class, and a one-way projection for the one-way class.** Accepted ADRs are immutable, so their attributions cannot be removed at the source. Their mirror is a projection that removes attributions entirely: the Deciders line, any "written by" line, and any line or phrase that names a person or a role are dropped, and nothing replaces them. The CHANGELOG and the release notes are written as neutral, institutional text, in the style of a press release. New ADRs keep their attributions in the project repository only, and the same projection drops them. Internal notes stay in the working version and never appear in the release repository, from v1.0 on.

The projection's patterns, the neutrality list and the list of role words are **kept outside every repository**, in a private configuration, and the gate reads them from a path given as an argument. Writing any of them into a repository would publish what it guards.

### 3. One leak gate for every sync and every release

`scripts/leak-gate.sh <tree-or-archive> [--lists <dir>]` (to be written) is the only gate. It fails closed: a check that cannot run is a failure. It runs:

1. **The mirror map.** Every file of the tree is in the two-way or one-way class, and nothing else is present.
2. **Paths and user names.** It scans every file, text and binary, for home-directory paths and the user names of the machines that build releases, in any form. It generalizes the check of `scripts/check-release.sh` (#382, S-13), which already refuses `/Users/` and `/home/` paths and the builder's user name in the archives.
3. **The neutrality check.** Every file and every commit message must contain no word of the neutrality list. The list is read from `--lists`, never from the repository.
4. **Handles and internal names.** The same scan, with the list of session and person names, read from `--lists`.
5. **Attributions.** Role words used as attributions in mirrored text, such as a role named as the author or the decider of a change, from a list read from `--lists`.
6. **Secrets.** Tokens, keys, e-mail addresses other than the public contact, and URLs of private services.
7. **For an archive,** `scripts/check-release.sh`'s S-13 checks as they are.

It runs at these points:
- every forward sync, on the outgoing tree and its commit message;
- every import back, on the patch;
- the project CI, on the two-way paths at each merge;
- every release, on the source tree and on each archive, before the tag.

### 4. History

**Options:**
- **(A) A clean start.** The release repository's first commit is the v1.0 tree, with no parent. After it, each sync is one commit per merged project pull request. Its message is generated from a template (title, issue reference, summary), passes the gate, and carries a neutral author identity.
- **(B) A rewritten history.** `git filter-repo` keeps only the mirrored paths, applies the projection, and rewrites author identities. Every old blob, message and identity must then pass the gate. 895 commits of agent-written messages, with old assets in reachable history, are a large surface for one miss, and a public miss cannot be taken back.
- **(C) Replay every commit.** This is (B) without the rewrite tooling: the same surface, more work.

**Recommendation: (A).** The project repository keeps the full history for the team. The release repository's history starts clean at v1.0 and grows one reviewed commit per change. A sync manifest in the project repository (`docs/releases/sync-manifest.md`, never mirrored) records each pair of project SHA and release SHA.

### 5. Outside contributions flow back through the project repository

**Options:**
- **(A) The project repository is the only writer.** A public pull request is reviewed in the release repository, but merged in the project repository:
  1. It is imported as a project pull request, with its author kept as `Co-authored-by`, since that identity is already public, and a reference to the public pull request.
  2. The leak gate runs on the patch, then the usual local-ci.
  3. After the merge, the forward sync publishes it, and the public pull request is closed as merged by that commit.
  4. Every change, from either side, then lands once and flows one way at a time, so the two trees cannot diverge. "Reflected in the other" holds with a delay of one sync.
- **(B) Both repositories accept merges, and a bot reconciles them.** It is faster for contributors, but two writers on the same files need conflict resolution in automation, and a public merge could bypass the project's gates.

**Recommendation: (A).** Contributors sign their commits off (DCO, `Signed-off-by`), which `CONTRIBUTING.md` will require in the release repository, because the project repository's licensing record must show where each contribution came from. Issues opened in the public repository stay there. The project's board links them.

### 6. Signing and provenance

**Recommendation:**
- Release tags in the release repository are signed, with an SSH or GPG key held outside every repository and every automation.
- Each archive gets a SHA-256 file, as today, plus a minisign signature of the checksum file. The minisign public key is published in the release repository's `SECURITY.md` and on the release page.
- macOS binaries are signed and notarized (#242).
- The sync commits are signed with a dedicated sync key, which is listed in the release repository's `SECURITY.md`, so a reader can tell a synced commit from a forged one.
- When the release repository has its own CI, it attaches build provenance (a SLSA attestation) to each archive. Until then, the validation record of `scripts/release-gates.sh`, run on the exact SHA, is published with the release, as neutral text, through the gate.

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
5. The neutral rewrite of the commit-message template of sync commits and of the release notes, in the institutional style of §2, checked by the gate.
6. The release repository, its keys and its settings are created by hand, outside the pipeline. This ADR asks for no remote change.

**Complexity.** The gate is one linear pass over the tree's bytes for each list, O(n · k) for n bytes and k list entries, or O(n) with one combined automaton (Aho-Corasick). A sync copies the two-way paths, O(n). For about 4,000 files these are seconds.

## Consequences

- The project repository keeps working as it does, with one new rule: two-way paths are written neutral, and its CI enforces that.
- The release repository's history is short and clean, and its archives are built from public sources only.
- Every release-facing text, mirrored files, the CHANGELOG, release notes and sync commits, is neutral and institutional: it names no person and no role. Attributions and internal notes stay in the project repository.
- Outside contributors wait one sync for their merged change to appear, and they sign off their commits.
- The neutrality list, the role words and the projection's patterns live outside every repository, so losing that private configuration stops the sync, closed, rather than leaking.

## Alternatives considered

The options of §2, §4, §5 and §7 are given in their sections, each with the reason for the recommendation. Making the project repository itself public was ruled out: its history and old assets stay private.

## Amendment of 2026-10-14: this repository becomes the public repository

- Status: Accepted
- Deciders: Una project (accepted on 2026-10-14)
- Refs: #717

**Context.** The decision above keeps this repository private and launches a separate release repository, kept in step with it by a mirror. Its alternatives rule out making this repository public, because of its history and its old assets. On 2026-10-14 the project decided otherwise: this repository itself becomes the public repository.

**Decision.**
1. **One repository.** This repository is the public repository when it is opened. No separate release repository is created, and nothing is mirrored.
2. **No personal data.** The repository holds no personal data, in its files, its commit metadata or its release assets:
   - no personal e-mail address;
   - no local account name;
   - no home directory path;
   - no personal name.

   New commits are authored with a noreply address. A check that blocks new personal data in the tree is added under #717, with its patterns kept outside the repository, as the lists of §3 are.
3. **A one-time history rewrite.** Past content is cleaned by one rewrite of the history, run once, when the repository is opened. It covers author and committer identities, commit messages, and file contents across history, and it is tested on a clone before it runs.
4. **The exception to immutability.** The rewrite changes the text of accepted ADRs and of other immutable records where they hold personal data, and it changes every commit id. This is an exception to the immutability of accepted text and of history, and it covers personal data only. No other change to an accepted text or to history is made under it.
5. **References stay resolvable.** Commit ids and issue references in the repository's texts, such as validation records, release notes and ADRs, stay resolvable. Each rewritten commit id maps to its new one in a map committed to the repository. The rewrite keeps every commit, even one it leaves empty, so the map is one to one and every tag keeps its commit. Issues keep their numbers, so the map of issue references is the identity. Release assets that are repacked to remove personal data are recorded with their old and new digests, so validation records stay checkable.
6. **What stays unpublished.** The paths that §1 never mirrors stay unpublished: `docs/reviews/`, `docs/plan/`, `docs/history/`, `docs/fixes/`, `docs/releases/`, `coordination/`, `CLAUDE.md`, and `images/` except the logo. Before the repository is opened, they are removed from its tree. The one-time rewrite drops them from the history of the opened repository, and they remain only in a private archive. Every other path is published: the list of private paths is explicit, and publishing is the default.
7. **Neutral text.** The rewrite also applies the projection of §2 to the ADRs: their attributions to persons and roles are removed. It works inside lines, replacing each attribution, and deletes no line. Release-facing text stays neutral and institutional, as the decision above requires.

**Consequences.**
- The separate release repository, its mirror map (§1), the sync of §4 and §5, and the cutting of releases in another repository (§7) are no longer needed. Releases are cut, validated and signed in this repository.
- The rule that release-facing text is neutral and institutional is unchanged.
- The release validation records, `docs/releases/validation.md` among them, move out of the published tree to a private location before the repository is opened.
- Any link from a published document into a path of rule 6 is removed or redirected before the repository is opened. Examples are the references of `README.md` and `CONTRIBUTING.md` to the validation records, and the reference of the `CHANGELOG.md` header to `docs/plan/`. Plain mentions of such a path in released sections of `CHANGELOG.md`, such as their closing "Validation:" lines, stay as they are: those sections are frozen, and a mention publishes no content. Once those paths are gone from the tree, the documentation link check (`scripts/doc-links.py`) enforces it.
- The check of personal data runs on this repository's tree, with its lists outside the repository.
- Every commit id changes once, at the rewrite. Tags, releases and validation records are carried over through the maps of rule 5.

## Amendment of 2026-10-14, second: changelog projection and release validation

- Status: Accepted
- Deciders: Una project (accepted on 2026-10-14)
- Refs: #717

**Context.** The first amendment of 2026-10-14 makes this repository the public repository and moves the release validation records to a private location. It leaves three points open: whether the projection of its rule 7 covers `CHANGELOG.md`, what a published release says about its validation once the records are private, and where the gates are defined, since their definitions are in `docs/releases/validation.md`, a path of its rule 6.

**Decision.**
1. **The projection covers `CHANGELOG.md`.** Rule 7 of the first amendment applies to `CHANGELOG.md` as it does to the ADRs, since §1 classes both as one-way, projected. Its attributions to persons and roles are replaced inside lines, and no line is deleted. The released sections are projected with the history, so they still match the text their tags published.
2. **Each release publishes a neutral validation summary.** The summary is generated from a template and names the commit, the toolchains, the table of gates with each result, and the digest of every release asset. It is the release's notes and is also attached to the release as an asset. It is neutral and institutional, as all release-facing text is. The full validation record stays private.
3. **The gate definitions are published.** The definitions of the gates move from `docs/releases/validation.md` to a published document, written as neutral text. The pattern that the personal-data gate checks reaches the gates through an environment variable, set from an item of the system keychain when the gates run. It is never kept in a file of the repository.

**Consequences.**
- A reader of a published release can see what was validated, on which commit, and with which result, without the private record.
- The release gates, and their tests, refer to the published document of the gate definitions in place of `docs/releases/validation.md`.
- The gates fail closed when the personal-data pattern is not set, as they do for any gate that cannot run.
