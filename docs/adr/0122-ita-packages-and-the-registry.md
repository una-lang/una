# 0122. Itá: packages and dependencies, the project files `ita.una` and `ita.lock`, and the Itaty registry

- Status: Accepted
- Date: 2026-10-19
- Deciders: Una project (accepted on 2026-10-19)
- Resolves: issue #893 (a sub-issue of epic #840: steps 1 and 3 of the epic, the record for its phase 1 and its phase 2, in one record)
- Related:
  - ADR 0021 (packages and project files: `una.toml`, `verification.lock`), which this record replaces for edition `0.4`;
  - ADR 0093 (authority at module boundaries), whose rule 2 already states the secure default for code from outside the package;
  - ADR 0085 (the file-state label), whose decision 5 states what must change "if packages may ever import other packages";
  - ADR 0109 (profiles), which deferred "profiles across package dependencies";
  - ADR 0116 (the `[drive]` table) and ADR 0117 (limits on grants), which name `ita.una` (#893) for their forms;
  - ADR 0121 (draft, PR #915: the terms of edition `0.4`), whose rule 2 (a) governs the trust a package adds, and whose rule 10 asks for this record's section "Edition `0.4`";
  - issue #891 (one binary), inside which Itá ships;
  - ADR 0064 (the authenticated proof cache), `docs/security/tcb.md` (the risk profile of the trusted base), and `docs/plan/formal-track-m5.md` (the document-level flag that assumes one package).
- Target: edition `0.4`, under the terms of ADR 0121 (draft). Editions `0.1` to `0.3` keep `una.toml` and `verification.lock`, with no change. A single file keeps edition `0.1` and has no dependencies. The benchmark baseline, pinned to edition `0.1`, does not move.
- Amends:
  - **Names:** `spec/10 §11` rules 1, 2, 4, 8 and 10 (modules of dependencies);
  - **Information flow:** `spec/07 §6` (trusted modules of dependencies), `§7` rule 4 (the file-state label of a program with dependencies);
  - **Verification:** `spec/12 §9` (the lock and the key of the proof cache), `§10` (determinism);
  - **Diagnostics and project files:** `spec/13 §4` (codes), `§5` (`una ita`, `una migrate`, `una fix`), `§6` (report), `§7` (the project files of edition `0.4`, and the exception to its U7002 for a trusted module inside a boundary, rule 53.1), `§8` (Core export, `una.core/2`);
  - **Charter:** `docs/00-charter.md §3`, the Trusted zone's row;
  - **Accepted records:** ADRs 0021, 0085, 0093, 0109, 0116 and 0117, by the dated amendments of Appendix A;
  - **Draft records:** ADR 0121, by a note appended on its acceptance (Appendix B).

  The rules live in a new chapter, `spec/30-packages.md`.

> **In plain words.**
> - **What changes for a programmer.** A package of edition `0.4` describes itself in `ita.una`, a short file in Una's own syntax, and may name other packages it needs. `una ita add json` adds one. `ita.lock`, written by the tool, pins the exact content of every package the program uses.
> - **What a dependency can do.** Nothing it is not given. It runs no code when it is fetched. It holds no capability unless the program passes one at a call, within a limit that `ita.una` writes per dependency. It may declassify data only if the program says so by name. It carries no foreign code: a package holds Una sources only. The build report lists, for each dependency, what it may do and what it does.
> - **What is checked again.** The program re-checks and re-proves its dependencies' code itself. Nothing a publisher claims about its own package is trusted.
> - **What is trusted.** That the bytes in the lock are the bytes the publisher signed and logged. A dependency's module that the program marks as trusted (its policies) is trusted as the program's own trusted code is.

## Questions for acceptance

Questions 1 to 7 change what a programmer can rely on or must write. Question 8 asks the project to confirm three assumptions on edition terms. Questions 1 to 6 were accepted as recommended, Question 7 was rejected, and the three assumptions of Question 8 were confirmed (2026-10-19). The text below follows each decision.

1. **Who may publish a package under a short name, such as `http`?**
   - **Decided:** accepted as recommended. Unscoped names are Itaúna's; every other package is `owner/name`.
   - **This record:** only Itaúna, the official set the project maintains. Every other package is named `owner/name`, as `acme/ledger`, and an owner name is registered once (rules 44 to 46).
   - **Owners belong to their source.** A private source declares the owners it serves (`owners acme`). A need of an `acme/…` name must then name that source, and `una ita add` picks it (rule 42), so `acme/ledger` never resolves at Itaty by default.
   - **Itaúna's keys** are named in the registry's root metadata, and change only by a new root version, signed by the offline root threshold. The registry cannot recover them (rule 38.1).
   - **Why:** short generic names are what typosquatters and name squatters take first, and a private name that a public package can shadow is how dependency confusion works.
   - **Cost to the programmer:** a longer name in `needs` for packages outside the official set. Imports use the short local name.
   - **Recommendation:** accept.

   ```una
   sources
     acme "https://ita.acme.example"
       root "sha256:5e0d1b7a…"
       owners acme
   needs
     json 2.0                       -- Itaúna, from Itaty
     acme/ledger 0.3 from acme      -- imported as `ledger`; without `from acme`: U7008
   ```

2. **May a dependency ever receive the program's capabilities by name?**
   - **Decided:** accepted as recommended. No inheritance by name; a dependency receives a capability only at a call, with `with`, within its caller's ceiling.
   - **Today:** ADR 0093 rule 2 makes every module of a dependency a boundary with an empty ceiling, "unless the consumer's `una.toml` lists the dependency under a table `[inherit]`".
   - **This record:** no. `[inherit]` is not created. A dependency receives a capability only at a call, with `with` (rule 51, and the amendment of ADR 0093 in Appendix A).
   - **Which ceiling applies.** The ceiling at a call is the one the *calling* package writes, with `may`, for the package it calls. A call that reaches a package its caller does not need, such as a function value of a transitive dependency handed back to the root, has the empty ceiling. So a dependency's own `ita.una` never sets the ceiling of a grant the root makes.
   - **Consent is in the lock.** The lock records every ceiling and trust line it was written under. A wider `may` line in `ita.una` is U7009 until `una ita lock --accept-authority` records it, so every widening shows in the lock's diff and in the review (rule 17).
   - **Cost to the programmer:** a `with` at each call into a dependency that needs a capability, a `may` line per capability, and the flag when a ceiling widens. Pure libraries, such as `json`, need none of it.
   - **Recommendation:** accept.

   ```una
   -- ita.una                         -- src/store.una
   needs                              import http.client (get_text)
     http 1.4                         ...
       may Http[get]                  let page = get_text(url) with @http = @net[get]
   ```

3. **Does the program prove its dependencies again?**
   - **Decided:** accepted as recommended. The root checks and proves every package of the program.
   - **This record:** yes. `una check` of a package type-checks, flow-checks and verifies every module of every dependency, under the package's own solver pin, as one program (rule 48). Nothing a publisher claims is used.
   - **The cache.** Obligations are cached by query across packages, under a domain-tagged key that cannot collide with the key of editions `0.1` to `0.3`, so a dependency is proven once per machine and solver version.
   - **Cost to the programmer:** the first check after adding or updating a large dependency takes longer. Its time is bounded by the dependencies' obligations, each under the step budget of the lock (`spec/12 §9`); there is no other limit.
   - **Recommendation:** accept. Accepting a publisher's proofs needs checkable certificates (issue #66), which a later record may add.

   ```text
   $ una ita update http
   http 1.4.2 -> 1.5.0 (itaty)
   $ una check
   checked 4 packages (store, http 1.5.0, json 2.1.0, text_utf8 1.0.3): 212 obligations proven (198 cached)
   ```

4. **Is a version that needs more authority a breaking change?**
   - **Decided:** accepted as recommended. Authority growth needs a new major version, enforced by the registry and by clients.
   - **This record:** yes. Within a major version, a release MUST NOT need a capability, an operation or a trust that the previous release did not (rule 26).
   - **Who enforces it.** The registry refuses to log a release statement whose `authority` or `trust` is not within the previous release's of the same identity, and every client verifies the same thing from the two statements when it locks or updates (U7010). The statements' authority is checked against the archives (rule 37), so a publisher with a modified client cannot get around it. A consumer's ceiling refuses it anyway (U3021).
   - **Not enforced:** the compatibility of headers. `una ita publish` checks it, as advice to an honest publisher.
   - **Cost to the programmer:** a library author bumps the major version to start sending email.
   - **Recommendation:** accept.

   ```text
   $ una ita publish
   error[U7013]: acme/mailer 2.2.0 needs Http[post], which 2.1.0 did not need
     --> src/send.una:12:3
     = repair: publish it as 3.0.0
   ```

5. **How are publishers' keys held?**
   - **Decided:** accepted as recommended, option (a): owner-held Ed25519 keys under a witnessed transparency log.
   - **(a) Recommended:** each owner holds Ed25519 keys of its own, encrypted on disk by default. Every key event and every release is an entry of a public, append-only transparency log that independent witnesses co-sign (rules 36 to 38). No identity provider is in the trust base. With it:
     - **Recovery** of an owner that lost its keys is pending for seven days of witness time, which clients check themselves, and any current key can cancel it (rule 38.1).
     - **Itaúna's keys** are recoverable only by the root threshold.
     - **A revocation** can state "compromised since log entry p", and `una ita audit` then flags every release the key signed after p.
     - **Verification** is strict Ed25519, one rule for every implementation.
     - **Time** is the witnesses' co-signature time, as an order statistic over the threshold of witnesses, so fewer than the threshold can move it; never the publisher's or the registry's clock (rule 36.4).
     - **A thief holding one key** of an owner with a threshold above one cannot block recovery forever: recovery co-signers, or a logged registry dispute after the same delay, overrule it (rule 38.1).
   - **(b)** Keyless signing, as Sigstore offers: a short-lived certificate tied to an account at an identity provider. The identity provider and the certificate authority would join the trust base, and a stolen account would sign.
   - **Cost to the programmer:** a publisher keeps a key file and a passphrase, and loses nothing but a waiting period if it is lost.
   - **Recommendation:** (a). A later record MAY add (b) as a second signature, never as a replacement.

   ```text
   $ una ita key new                # once per publisher machine
   passphrase for ~/.una/keys/acme-4f9c.key: ********
   key ed25519:acme:4f9c…e1 signed by ed25519:acme:a03d…7b; registered (log entry 48211)
   ```

6. **Should Itá wait before taking a new release?**
   - **Decided:** accepted as recommended. Every selection waits a 72-hour cooldown in witness time.
   - **This record:** yes. No resolution selects a release logged less than 72 hours earlier, measured in witness time (rules 24, 25, 36.4). That is a window in which many malicious releases have been found and removed.
   - **It covers every way in.** `una ita update`, `una ita add`, and a raise past a yanked or advised release (rule 24) all wait. A stolen key cannot shorten the wait by publishing a release and then yanking the one before it.
   - **The one exception:** a fix that an advisory names by its digest, when the advisory is signed by two security keys of the root metadata, or by the security key and the owner's threshold (rule 41).
   - `--now` takes a newer release anyway, explicitly.
   - **Cost to the programmer:** an update reaches a new feature three days later.
   - **Recommendation:** accept.

   ```text
   $ una ita update json
   json 2.1.0 -> 2.1.3 (itaty)
     2.1.4 was logged 9 hours ago, within the 72-hour cooldown; `--now` takes it
   authority: unchanged (nothing); trust: unchanged (none); key: unchanged
   ```

7. **May a package depend on a git repository?**
   - **Decided:** rejected. There are no git dependencies, in any phase: no `from git` origin, no git origin in the lock, and Itá never runs the system `git` or any other external program (rules 7 and 61).
   - **Why:** fetching a git source would run the system `git` unconfined, with the user's privileges, against a server that may be hostile, and would add a tool besides `una` (#891).
   - **What covers the need instead:**
     - **local development:** path dependencies, written only by the root and workspace members (rules 7 and 42), and workspaces (rule 19);
     - **organizations:** a private Itá source, declared in the root's `sources` with the owners it serves (rules 8 and 42), or packages vendored into the repository (rule 32).
   - **Cost to the programmer:** code shared before it is published is shared by path, by workspace, or through a private source.

   ```una
   sources
     acme "https://ita.acme.example"
       root "sha256:5e0d1b7a…"
       owners acme
   needs
     acme/ledger 0.3 from acme
     util from path "../util"
   ```

8. **Edition terms: the project is asked to confirm three assumptions** (ADR 0121, draft, and `spec/13 §7`). This record does not decide edition terms; it is written against the assumption stated in each item.
   - **Decided:** the three assumptions below are confirmed.
   - **8.1. `una.toml` in edition `0.4`.** ADRs 0116 and 0117 put `[drive]` and limits in `una.toml` for edition `0.4`. **Assumed:** edition `0.4` reads only `ita.una`, so it has one manifest format and one parser.
     - **If no released `una` has accepted edition `0.4` in `una.toml`** when this record's files land, `una.toml` naming edition `0.4` is a plain rejection, U7002, with `una migrate` as its repair. No form of a released edition changes.
     - **Otherwise** it is U7007 (ADR 0121 rule 5), an entry of the edition ledger, with the same repair.
     - **ADR 0121** reads the edition switch and its opt-ins from `una.toml` (its rule 2) and builds the `0.3`-to-`0.4` pair of the edition test from `una.toml` packages (its rule 11). A note on ADR 0121, appended on its acceptance (Appendix B), states that in edition `0.4` both read `ita.una`, and that the edition test runs `una migrate` on each `0.3` package before comparing.
   - **8.2. Editions across dependencies.** ADR 0121 notes that "a rule is needed for a package of one edition that depends on a package of a preview edition". **Assumed** (rule 49): a package MAY depend only on packages whose edition is its own or earlier. A U7007 inside a dependency, after an upgrade of `una`, is reported at the dependency's line with the repair "update the dependency".
   - **8.3. Itá is preview material.** The manifest grammar, the lock format and the registry protocol may change under ADR 0121's rules while `0.4` is a preview. The lock and the protocol carry their own format versions (`lock 1`, rule 16; `ita-release/1`, rule 37), so a later format is read beside the old one. **Assumed:** yes, with no unsettled point.

## Glossary

- **Itá** (Tupi for "stone"). A package, and the package manager, `una ita`, also invoked as `ita` (rule 1).
- **Itaty** ("many stones", a quarry). The public registry of packages.
- **Itaoca** ("stone house"). The local store of fetched packages, `~/.una/itaoca`.
- **Itaúna** ("black stone"). The official set: the standard library, which stays inside `una`, and the packages the project maintains on Itaty under unscoped names.
- **Package.** A directory with `ita.una` and `src/`, or the same content fetched as an archive (rule 27).
- **Root.** The package a command runs on. Its dependencies, direct and transitive, and the root together are the **program**.
- **Owner.** An account of a registry that holds names, as `acme` in `acme/ledger`, and the keys that sign their releases (rules 38 and 44).
- **Need.** A line of `ita.una` naming a package the root depends on, with a minimum version (rule 7).
- **Local name.** The name under which a package's own modules import a dependency's modules, the last segment of its name unless `as` gives another (rule 47).
- **Identity.** What makes two packages the same: the source, the name, and the major version (rule 21).
- **Minimal version selection (MVS).** The resolution of rule 23: for each identity, the highest of the minimum versions that the program's needs ask for. Never the newest published.
- **Content digest.** The SHA-256 of a package's canonical file listing (rule 28). Two packages with the same digest have the same files, byte for byte.
- **Ceiling.** The most authority a package grants a dependency at its calls, written with `may` (ADR 0093 rule 1, rule 51). The ceiling of a call is the calling package's.
- **Resolution point.** The checkpoint at which a lock was resolved; yanks, advisories, cooldowns and keys are evaluated as of it (rule 15).
- **Name map and chain.** The entries about one name, linked by hash, and a map from each name to its latest entry committed in every checkpoint, so a client can tell that it sees all of a name's entries (rule 36.1).
- **Witness time.** The timestamp of a witness's co-signature, the only clock the rules use for durations (rule 36.4).
- **Trust line.** A line of the root's `trust` section that consents to a dependency's trusted modules (rule 53).
- **Transparency log.** An append-only Merkle tree of every release and key event, whose signed checkpoints let a client prove that an entry is in the log and that the log was only appended to (rule 36).
- **Checkpoint.** A signed statement of the log's size and root hash (C2SP `tlog-checkpoint`).
- **Witness.** An independent party that co-signs a checkpoint after checking that it extends the last one it saw, so the registry cannot show two different logs to two clients.
- **Yank.** A release marked as not to be selected by new resolutions. It stays in the log and remains fetchable (rule 41).
- **Advisory.** A signed, logged statement that releases of a package have a security defect (rule 41).
- **Cooldown.** The witness time every resolution waits after a release is logged before selecting it (rules 24 and 25).
- **Trusted computing base.** The code that the safety guarantees depend on but that the checker does not check (the report's `tcb`, `spec/13 §6`).
- **T6.** The noninterference theorem of `spec/10 §10`.

## Context

- **One package today.** A package is `una.toml`, `verification.lock` and `src/**/*.una` (`spec/13 §7`, ADR 0021). An import names a module of the package or one of the standard modules, `std.host` and `std.testing`; anything else is U1001 (`spec/10 §11` rules 4 and 10). Module identity is the module path within the package (`spec/10 §11` rule 8).
- **Project files are TOML.** `una.toml` holds the tables `[package]` (`name`, `version`, `edition`, and from edition `0.3` `profile`, ADR 0109), `[labels]`, `[trusted]`, `[escape_budget]`, `[run]`, and in edition `0.3` `[contracts]` (ADR 0092) and `[boundaries]` (ADR 0093). Unknown keys are U7002. `verification.lock` holds six keys (`spec/12 §9`), U7004 when invalid. Both are read by the `toml` crate (`crates/una-project/src/document.rs`, `Cargo.toml`). ADR 0116 adds `[drive]` and ADR 0117 adds limits to grants for edition `0.4`, each saying that its form moves to `ita.una` with #893.
- **Names of packages.** `name` is any non-empty string (ADR 0021 rule 2), so `events-example` is valid today, and it never appears in source.
- **Authority across packages is already decided.** ADR 0093 rule 2: "When packages gain dependencies, every module of a dependency is a boundary with an empty ceiling, unless the consumer's `una.toml` lists the dependency under a table `[inherit]`." Calls into a boundary are granted with `with` (rule 3), checked against the ceiling (U3021, rule 5), and may not reach a release (U3024, rule 8).
- **What assumes a single package.**
  - ADR 0085 decision 5: the file-state label is computed over "the whole program's" call graph, which is one package; "if packages may ever import other packages, every `pub` function ... must take its effect bound ... unless the whole program is known when the label is computed. ADR 0085 must be amended with that change."
  - ADR 0116's `conc(f)` "is computed once over the call graph of the whole program, which today is one package with the standard library".
  - ADR 0092's case (d) of its lock rule relies on a function being "not public", which across packages is visible only if the whole program is.
  - The formal track's document-level flag: "No package imports another ... If packages may ever import other packages, the flag must go" (`docs/plan/formal-track-m5.md`).
  - ADR 0109: "Per-module profiles, a profile for libraries, and profiles across package dependencies are deferred."
- **`Keys` handlers do not assume one package.** A user handler of `Keys` is checked against the worst caller (`spec/14 §3`, `spec/07 §7` rule 4), whoever calls it.
- **The proof cache.** Its key is SHA-256 over the lock file's contents and the query text; its entries carry an HMAC under the user's key (`spec/12 §9`, ADR 0064).
- **`una fix`** never edits `una.toml` or `verification.lock` (`spec/13 §5`).
- **The dependency audit of the toolchain itself.** The `deps` release gate checks `Cargo.lock` against advisories, licenses and sources, and against the review record `supply-chain/crates.tsv` (`docs/release-gates.md`, #770). `sha2` and `ring` (through `rustls`) are already dependencies (`Cargo.toml`).
- **What the trust base is.** `docs/00-charter.md §3` lists the Trusted zone: the compiler, the interpreter, the solver, the standard library, `trusted` modules, the host capabilities, the runtime and the proof cache's key. `docs/security/tcb.md` ranks the risks; its gap 9 is "no provenance for archives".
- **The direction of epic #840.** Capability grants per dependency, no code at install, contracts re-verified by the consumer, a lock with content hashes, reproducible builds, the CLI integrated in `una`; phase 1 with git and path sources and no server, phase 2 a central registry with name ownership, signing and a transparency log, "the phase 1 design must allow this move without breaking locks".
- **The direction of #893** (keep): the names Itá, Itaty, Itaoca and Itaúna; `ita.una` declarative in Una syntax, read by Una's parser, with closed keys and no code; `ita.lock` written by the tool with the solver pin and every dependency's version and content digest; `itaty.una` for workspaces; no install scripts; content-addressed and signed packages; no capability the program does not grant; each dependency's authority in the report; `una migrate`, edition-gated.

### Departures from the issues

- **Ceilings.** The issue's example writes `http 1.4` with no ceiling. Under ADR 0093 rule 2, `http` would then receive no capability at all, so a `may` line is added wherever a dependency needs one (rule 7).
- **Owner names.** The issue writes unscoped names; here unscoped names belong to Itaúna, and `http` and `json` in the example are read as Itaúna packages (Question 1).
- **The epic's `una add` and `una.lock`** are `una ita add` and `ita.lock`, as #893 decided. The epic's grants "in `una.toml`" are the `may` ceilings of `ita.una`.
- **No git sources.** The epic's phase 1 has git and path sources. This record has path and workspace sources only (Question 7): local development uses path dependencies, and organizations use a private Itá source.
- **One record for both phases.** The epic asks for two records. Rules 34 to 46 (the registry) are this record's phase 2, implemented after phase 1, and the lock of rule 16 serves both, so the move to the registry breaks no lock.

## Decision

In one paragraph: in edition `0.4`, a package is described by `ita.una`, a declarative file in Una's lexical syntax with a closed grammar, and pinned by `ita.lock`, written only by `una ita`, which holds the solver pin and, for every package of the program, its version, source and content digest. Versions are a subset of semantic versioning, and resolution is minimal version selection, linear in the size of the dependency graph and reproducible without a lock. A package is fetched as a canonical archive whose digest is its identity, into Itaoca, and no code of it runs. The root checks and proves every package of the program as one program. Every dependency is a boundary of ADR 0093 with the ceiling its `need` writes; it receives capabilities only through `with`; its trusted modules need a trust line in the root, and it carries no foreign code. The registry, Itaty, is a set of static files: a transparency log of signed releases and key events, with a hash chain per name and a name map in every checkpoint, co-signed by witnesses, under a root of offline keys embedded in `una`; every selection waits a cooldown measured in witness time; mirrors are untrusted transport. Names outside Itaúna are scoped by owner, and owners are bound to the source that serves them; only the root names sources and paths. Trust lines bind identities, and the lock records every ceiling and trust it was written under. The build report lists each dependency, its ceiling, its authority and its trust. `una migrate` moves a package from `una.toml` to `ita.una`.

### A. Names and files

1. **Itá in `una`.** The package manager is the command `una ita` of the `una` binary (#891). The same binary invoked under the name `ita` (its `argv[0]` basename) behaves as `una ita`: `ita add json` is `una ita add json`. The installer makes `ita` a symbolic link to `una`; uninstalling removes both. No other program is installed.

2. **The files of a package of edition `0.4`.**
   - `ita.una` at the root, the manifest; `ita.lock` at the root, the lock; sources under `src/`, as today. `ita.una` replaces `una.toml`, and `ita.lock` replaces `verification.lock`.
   - **A package of edition `0.4` is a directory with `ita.una` and `src/`.** `ita.una` that names an edition before `0.4` is U7008. A directory with both `ita.una` and `una.toml` is U7008, so a package never has two manifests that could disagree. Under the assumption of Question 8.1, `una.toml` naming edition `0.4` is rejected with the repair `una migrate`: U7002 if no released `una` accepted that form, U7007 otherwise.
   - **Editions `0.1` to `0.3`** keep `una.toml` and `verification.lock` as `spec/13 §7` states, and have no dependencies.
   - **A single file** keeps edition `0.1`, the default lock and no dependencies.

3. **Read by Una's parser, and no code.** `ita.una`, `ita.lock` and `itaty.una` are read by Una's lexer (`spec/01`), with the one token of rule 4, and by the grammar of rules 5, 16 and 19, which is part of the syntax crate.
   - **Declarative.** There are no expressions, no interpolation, no conditions, no includes and no environment variables. Nothing in these files runs, and nothing else runs when a package is fetched, resolved, checked or built: there are no install scripts, build scripts or hooks of any kind, and no foreign code (rule 60).
   - **Closed.** Every section and every line has one of the forms below. Anything else is U7008, at the line.
   - **Bounded.** `ita.una` and `itaty.una` hold at most 1 MiB and `ita.lock` at most 16 MiB; past that, U7008 or U7009 at the start of the file.

### B. `ita.una`

4. **Lexical rules.** Those of `spec/01`: UTF-8 without byte order mark, two-space indentation, `--` comments, no tabs, the control characters of `spec/01 §1` rule 6 rejected, identifiers and integer and text literals as `spec/01 §4` and `§6` state. Two differences, in these files only:
   - **A version token.** `version_lit = num "." num [ "." num ] [ "-" pre { "." pre } ]`, where `num` is `0` or a digit sequence without a leading zero and `pre` is `[0-9A-Za-z]+` (a numeric `pre` has no leading zero). It is matched before the float and integer literals, so `1.4` and `0.1.0` are versions here, never `F64` literals.
   - **Text without interpolation.** A text literal MUST NOT contain `{` other than as `\{` (U7008): nothing is interpolated.

   Reserved keywords of `spec/01 §5`, such as `trusted`, `in` and `from`, are words of this grammar where it names them. No word of these files becomes a keyword of a module.

5. **Grammar.**

   ```ebnf
   manifest     = head { section } ;
   head         = "ita" pkg_name NEWLINE INDENT head_line { head_line } DEDENT ;
   head_line    = "version" version_lit NEWLINE
                | "edition" version_lit NEWLINE
                | "profile" ( "app" | "systems" ) NEWLINE
                | "about" text_lit NEWLINE
                | "license" text_lit NEWLINE
                | "home" text_lit NEWLINE
                | "publish" "to" lower_ident NEWLINE ;

   section      = sources | needs | replace | trust | labels | trusted | escapes
                | boundaries | contracts | run | drive ;

   sources      = "sources" NEWLINE INDENT source { source } DEDENT ;
   source       = lower_ident text_lit NEWLINE INDENT "root" text_lit NEWLINE
                  [ "owners" lower_ident { "," lower_ident } NEWLINE ] DEDENT ;

   needs        = "needs" NEWLINE INDENT need { need } DEDENT ;
   need         = pkg_name [ version_lit ] [ "as" lower_ident ] [ origin ] NEWLINE
                  [ INDENT need_line { need_line } DEDENT ] ;
   pkg_name     = lower_ident [ "/" lower_ident ] ;
   origin       = "from" ( lower_ident
                         | "path" text_lit
                         | "workspace" ) ;
   need_line    = "may" cap_type { "," cap_type } NEWLINE
                | "avoid" version_lit NEWLINE
                | "accept" "name" NEWLINE ;
   cap_type     = qualified_upper "[" lower_ident { "," lower_ident } "]" ;
   qualified_upper = { lower_ident "." } upper_ident ;

   replace      = "replace" NEWLINE INDENT
                  { pkg_name version_lit "with" origin NEWLINE } DEDENT ;

   trust        = "trust" NEWLINE INDENT { trust_line } DEDENT ;
   trust_line   = pkg_name [ major ] [ "from" lower_ident ]
                  "modules" module_path { "," module_path } NEWLINE
                  [ INDENT { "may" cap_type { "," cap_type } NEWLINE } DEDENT ] ;
   major        = int_lit | version_lit ;          (* 2, or 0.3 for the line 0.3 *)

   labels       = "labels" NEWLINE INDENT { upper_ident NEWLINE } DEDENT ;
   trusted      = "trusted" NEWLINE INDENT { module_path NEWLINE } DEDENT ;
   escapes      = "escapes" NEWLINE INDENT "checked" "per" "module" int_lit NEWLINE DEDENT ;

   boundaries   = "boundaries" NEWLINE INDENT { module_path NEWLINE
                  [ INDENT { "may" cap_type { "," cap_type } NEWLINE } DEDENT ] } DEDENT ;
   contracts    = "contracts" NEWLINE INDENT { module_path NEWLINE
                  INDENT uses_item { uses_item } DEDENT } DEDENT ;
   uses_item    = cap_ref ":" cap_type_with_conditions NEWLINE ;      (* spec/05 §3 rule 4 *)

   run          = "run" NEWLINE INDENT [ "entry" module_path NEWLINE ]
                  [ "grants" NEWLINE INDENT grant { grant } DEDENT ] DEDENT ;
   grant        = ( "console" | "clock" | "env"
                  | "keys" text_lit
                  | "files" text_lit file_mode { "," file_mode }
                  | "http" text_lit [ http_method { "," http_method } ] ) NEWLINE
                  [ INDENT limit_line { limit_line } DEDENT ] ;
   file_mode    = "read" | "write" ;
   http_method  = "get" | "post" | "put" | "delete" ;
   limit_line   = (* ADR 0117 rule 8 *) ;

   drive        = "drive" NEWLINE INDENT "module" module_path NEWLINE
                  "ticks" int_lit NEWLINE [ "budget" int_lit NEWLINE ] DEDENT ;
   ```

   - **Order and repetition.** The head's lines appear at most once each, `version` and `edition` required. Each section appears at most once, in the canonical order of the production `section`. A repeated or misplaced line is U7008. There is no `foreign` section and no `git` origin: either is an unknown line, U7008. `una fmt` writes the canonical order and spacing, and `una fmt --check` reports U7001, as for a module.
   - **`module_path`, `cap_ref`, `lower_ident`, `upper_ident`, `int_lit`, `text_lit`** are those of `spec/01` and `spec/02`. `cap_type_with_conditions` is the type of a `uses` clause with ADR 0092's conditions (`spec/05 §3` rule 4), parsed by the same production as in a function header.

6. **The head.**
   - `ita <name>` names the package. `<name>` is `lower_ident` with no `__` and no trailing `_` (U1005's rule), at most 64 bytes, or `owner/name` for a package published under an owner (rule 44). The name is never written in source; the local name of rule 47 is.
   - `version`: a full version, `MAJOR.MINOR.PATCH` with an optional pre-release (rule 20). `edition`: `0.4` or a later edition the release serves (U7002's rule for editions becomes U7008 here).
   - `profile`: `app` (the default) or `systems` (ADR 0109).
   - `about`, `license` and `home`: a one-line description, an SPDX license expression, and an `https` URL. `una ita publish` requires `about` and `license`. A text may hold any character through an escape, so every command that prints one of a dependency's texts writes the characters that `spec/01 §1` rule 6 rejects in source as `\u{…}` escapes, never raw, so a bidirectional control in a dependency's `about` cannot disguise what a review shows.
   - `publish to <source>`: the one source this package may be published to. **Without it, `una ita publish` refuses**, so private code is never published by accident.

   ```una
   ita acme/ledger
     version 0.3.1
     edition 0.4
     profile app
     about "Double-entry ledgers with verified balances"
     license "Apache-2.0"
     publish to itaty
   ```

7. **`needs`.** Each `need` names a package the root's modules may import.
   - **The requirement** is a minimum version (rule 22). It is required, except with `from path` and `from workspace`, where the dependency's own `version` is used.
   - **`as`** gives the local name (rule 47). Two needs with the same local name are U7008.
   - **Reserved local names.** A local name, and the last segment of the name of any package of the program, whatever its origin, MUST NOT be a reserved name of rule 45 or have the skeleton of one (`std`, `prelude`, `std_`, `s7d` included). Otherwise U7008 at the need, or U7012 at the lock's package line for a transitive package, so `import std.host` can never be ambiguous.
   - **`from`** gives the source:
     - `itaty`, the default when `from` is absent and the name's owner is not one a `sources` entry serves (rule 42), and predeclared;
     - a source of the root's `sources` section;
     - `path`, a directory relative to the package. Only the root and workspace members write it. In a workspace member, the path stays inside the workspace root; in a root outside a workspace, it is the root author's own layout (`"../util"` is allowed). A path dependency's own path needs are U7013 (rule 42);
     - `workspace` (rule 19).
   - **`may`** lines write the ceiling that this package grants the dependency at its calls (rule 51), in the form of a `uses` type: a capability of the prelude, or one the dependency declares, written with its local name and module path, as `ledger.store.Store[put]`. No `may` line: the empty ceiling. A capability or operation that does not resolve is U7008.
   - **`avoid`** lines exclude versions from selection (rule 24). They are honored only in the root.
   - **`accept name`** (root and workspace members only): the root accepts that a `path` package carries an unscoped name, or a name whose owner is served by a source (rule 42). Without it, such a package is U7012.

   ```una
   needs
     http 1.4
       may Http[get]
     json 2.0
     acme/ledger 0.3 from acme
       may ledger.store.Store[put, find]
       avoid 0.3.4
     util from path "../util"
   ```

8. **`sources`.** A source other than `itaty`: a name, the registry's base URL (`https`), the digest of its root metadata (rule 35), which pins its trust on first use, and the owners it serves.
   - A source's name is `lower_ident`; `itaty`, `path`, `git` and `workspace` are reserved (U7008).
   - **`owners`.** A need of a name whose owner a source lists resolves only at that source, and MUST say `from` it (U7008 otherwise); `una ita add` writes the `from`. Two sources that list one owner are U7008.
   - **Only the root and `itaty.una` declare sources.** A `sources` section in any other package is not read, and a package that has one is refused at publish (rule 43). A dependency can therefore never make a program contact a host, or trust a root, that the root did not name (rule 42).

   ```una
   sources
     acme "https://ita.acme.example"
       root "sha256:5e0d1b7a…"
       owners acme
   ```

9. **`replace`** (root only; ignored with no diagnostic in a dependency, as Go's `replace` is): every package of an identity at a version is taken from another origin instead, such as a patched copy. The lock records the replacement and its digest, and the report marks the package `replaced`.

   ```una
   replace
     json 2.1.0 with path "patches/json"
   ```

10. **`trust`** (root only; rule 53): consent to a dependency's trusted modules, listed by module path.
    - **A trust line names an identity:** the package's name, its major line and its source, as `acme/privacy 1 from itaty`. The major and the source may be left out only for a package that the root needs directly; they are then those of that need. A transitive package needs a full trust line of its own.
    - **The lock records the consent, pinned to the whole package:** for each package with a consented trusted module, the lock records the package's whole content digest as consented (rule 16). A policy's `via` may call any function of its package (`spec/07 §5`), so pinning the trusted module's own file would not pin the code it runs. Any change of such a package, a new version included, and any trusted module that no recorded consent covers, whether of a new package, a new major or another source, is a trust change: U7012 until `una ita lock --accept-trust <package>` records it (rule 25).
    - **`may` lines under a `modules` trust line** write the ceiling of the trusted modules it names, as an extent of their own (rule 53.1). None: the empty ceiling.
    - A trust line in a dependency's own `ita.una` binds nothing for the root.

    ```una
    trust
      acme/privacy 1 from itaty modules privacy.keys
        may Keys[hmac]
    ```

11. **The sections that `una.toml` had.** Same meaning and the same checks as in `spec/13 §7` and the records named, in this form:
    - `labels`: the categories, one per line (U1005, U1002).
    - `trusted`: the trusted modules (U7005).
    - `escapes`: `checked per module <n>` (`[escape_budget] checked_per_module`).
    - `boundaries` (ADR 0093): a module path, then its ceiling as `may` lines; no `may` line is the empty ceiling.
    - `contracts` (ADR 0092 rule 8): a function's qualified name, then its locked `uses` clauses, one per line, parsed as a header's.
    - `run`: `entry`, and `grants`, one grant per line (rule 12).
    - `drive` (ADR 0116 rule 1): `module`, `ticks`, `budget`, with that rule's ranges and errors.

    ```una
    labels
      PII

    trusted
      events.keys

    escapes
      checked per module 1

    boundaries
      gallery.thumbnailer
        may Files[read]
      webhooks.template

    contracts
      pricing.update.update_prices
        @catalog: Catalog[price, set_price where not dry_run]
    ```

12. **Grants.** One per line under `run grants`, each with the meaning of the grant spec of `spec/13 §5` it corresponds to: `console`, `clock`, `env`, `keys "<file>"`, `files "<path>" read, write`, and `http "<origin>" get, post` with ADR 0117's limit lines below it. There is no foreign grant. Paths resolve against the package root. The `--grant` flag keeps the text form of `spec/13 §5`. A dependency's `run` and `drive` sections are not read: only the root is run.

    ```una
    run
      entry store
      grants
        console
        files "data/" read
        http "https://api.payments.example" get, post
          at most 100 per minute
          at most 8 at once
    ```

13. **Errors.** A syntax error, a missing required line, a repeated or unknown line, a value of the wrong form or out of range, and every error that `spec/13 §7` and the records named make U7002 for the corresponding `una.toml` key, are **U7008** at the line in `ita.una`. As today, project errors do not stop checking: the modules are still checked with default settings, and no dependency is loaded from an invalid manifest.

### C. `ita.lock`

14. **Who writes it.** Only `una ita` (rule 57) and `una migrate` write `ita.lock`, through a temporary file and a rename. `una check`, `una run`, `una build`, `una test`, `una lsp` and every other command only read it. `una fix` and `una fmt` never edit `ita.lock`, `ita.una` or `itaty.una` (amending `spec/13 §5`; `una fmt` formats `ita.una` only when asked by `una fmt ita.una`). It is committed with the package.

15. **What it holds.**
    - The solver pin of `spec/12 §9`.
    - For each registry source the program uses: its root digest, and the checkpoint the lock was resolved at. That checkpoint is the lock's **resolution point**: yanks, advisories, cooldowns and key states are evaluated as of it (rules 23 and 24), and it is a floor no client goes below (rule 39).
    - For every package of the program except the root: its name, version, origin, content digest and its own resolved needs, with their sources; for a registry package, the publisher's key and the log index of its release statement.
    - **The consent it was written under:** for each package, the ceiling each dependent wrote for it (its `may` lines), and for each trust line, the content digest of the package whose trusted modules it consented to (rules 10 and 51).

16. **Grammar.** The lexical rules of rule 4.

    ```ebnf
    lock        = "lock" int_lit NEWLINE solver { registry } { package } ;
    solver      = "solver" NEWLINE INDENT
                  "encoding" text_lit NEWLINE "name" text_lit NEWLINE
                  "version" text_lit NEWLINE "logic" text_lit NEWLINE
                  "steps" int_lit NEWLINE "seed" int_lit NEWLINE DEDENT ;
    registry    = "registry" lower_ident NEWLINE INDENT
                  "root" text_lit NEWLINE "checkpoint" int_lit text_lit NEWLINE DEDENT ;
    package     = "package" pkg_name version_lit NEWLINE INDENT
                  "from" lock_origin NEWLINE
                  "digest" text_lit NEWLINE
                  [ "signed" text_lit NEWLINE "logged" int_lit NEWLINE ]
                  [ "replaced" NEWLINE ]
                  { "needs" lower_ident pkg_name version_lit NEWLINE }     (* source, name, version *)
                  { "granted" "by" pkg_name [ major "from" lower_ident ] "may" cap_type { "," cap_type } NEWLINE }
                  [ "consented" text_lit NEWLINE ]                          (* the package digest consented *)
                  { "trusted" "modules" module_path { "," module_path } NEWLINE }
                  DEDENT ;
    lock_origin = lower_ident | "path" text_lit | "workspace" text_lit ;
    ```

    - `lock` is the format's version, `1`. A release reads every format version it ever wrote.
    - Packages are sorted by name, then version; `needs` lines by name. So the lock is canonical, and its diffs are reviewable line by line.
    - `digest` is `sha256:` and 64 lowercase hex digits (rule 28); `signed` is `ed25519:`, the owner and the key's fingerprint (rule 38).
    - `granted by` records, per dependent, the ceiling it wrote; a dependent other than the root is named with its major and source, so two majors of one package are told apart. `consented` records the content digest under which the package's trust was accepted, and `trusted` what was trusted in it.

    ```una
    -- Written by `una ita`; do not edit.
    lock 1

    solver
      encoding "una-smt-1"
      name "z3"
      version "5.1.0"
      logic "QF_UFLIA"
      steps 5000000
      seed 0

    registry itaty
      root "sha256:9f2c41d07a…"
      checkpoint 48211 "sha256:0b7e3c…"

    package http 1.4.2
      from itaty
      digest "sha256:3b1f8e0c…"
      signed "ed25519:itauna:7c0a91…"
      logged 48190
      needs itaty json 2.1.0
      granted by store may Http[get]

    package json 2.1.0
      from itaty
      digest "sha256:d41a77e2…"
      signed "ed25519:itauna:7c0a91…"
      logged 47002
    ```

17. **What reading checks, offline.** Every command that loads the package checks, with no network:
    - **the lock** exists and parses (U7009 otherwise, as a missing `verification.lock` is U7004 today; `una ita lock` writes it, with the solver section of the release's default lock when there is none yet);
    - **consistency:** every need of `ita.una` and every `needs` line of every locked package is met by a locked package of the same identity at a version at least the one required; every locked package is reachable from the root; each identity is locked once; `replace` entries match (U7009 at the need, with the repair `una ita lock`);
    - **consent:** every `may` line of every package of the program is within the ceiling the lock records for it, and every trusted module is covered by a recorded consent. A wider ceiling is U7009 at the `may` line, with the repair `una ita lock --accept-authority`, marked as an authority change and never applied by `una fix`; an uncovered trust is U7012 (rule 10). So a widening is visible in the lock's diff. The lock is a control of visibility, not of protection: an agent can write it as easily as `ita.una`. Like the `may` and `trust` lines, it is protected by a repository's review rules, as ADR 0093 rule 1 says of `una.toml`;
    - **integrity:** each package, read from the vendor directory or from Itaoca (rule 30), has the locked digest, computed from the bytes read (U7010 otherwise; never repaired automatically);
    - **provenance, for registry packages,** from what is stored beside the package (rules 30, 32), with nothing taken on the lock's word (U7010 for any failure):
      - the root: the lock's `registry … root` equals the `sources` pin, or for Itaty is reached by the root chain built into `una`, walked from the built-in version;
      - the checkpoint of the lock's resolution point: its log signature, and co-signatures by at least the threshold of distinct witnesses, all under that root's keys;
      - the release statement: its inclusion in that checkpoint, its digest equal to the lock's, and its signature by the locked key;
      - the key: a key of the package's owner valid at the statement's log position, from the owner's stored chain and its map proof at that checkpoint (rules 36.1, 36.3);
      - at a cost of O(log L) hashes and a few signatures per package, and O(log M + E) per owner.

      So a change that edits a vendored package, its digest, its key and its checkpoint together does not load: it would need the root's or the witnesses' keys;
    - **presence:** a locked package that is in neither place is U7011 at its need, with the repair `una ita fetch`.

    Consistency is not minimality. Whether the lock is exactly the resolution of rule 23 at its resolution point is checked by `una ita lock --check`, offline: every package's needs are in the locked packages, and the per-name state of rule 36.1 at the resolution point (yanks, advisories, logged times) is stored beside them in Itaoca or the vendor directory.

18. **The solver pin.** The `solver` section has the six keys of `spec/12 §9`, the same values and the same checks (U7009 in place of U7004; a solver of another version stays U5003). With #891, `version` names the solver built into `una`.

### D. Workspaces

19. **`itaty.una`.** A repository MAY hold, at its root, `itaty.una`, listing the packages of a workspace:

    ```ebnf
    workspace = "itaty" NEWLINE INDENT "members" NEWLINE INDENT { text_lit NEWLINE } DEDENT DEDENT
                [ sources ] [ replace ] [ trust ] ;
    ```

    ```una
    itaty
      members
        "store"
        "tools/report"

    trust
      acme/privacy 1 from itaty modules privacy.keys
    ```

    - Each member is a directory, relative and without `..`, with its own `ita.una` and `src/`, of edition `0.4` or later. A member has no `ita.lock`; the workspace has one, at its root, resolved for every member together (U7009 if a member has its own).
    - A member names another member with `from workspace`: `store from workspace`. Without `from workspace`, a need never resolves to a member, even with the same name, so a member can never shadow a registry package or the reverse.
    - `sources`, `replace` and `trust` of `itaty.una` apply to every member, as if each member's root had written them; a member that writes its own is U7008.
    - A command run inside a member finds the workspace by walking up to the nearest `itaty.una` that lists it; at most 64 levels, and never past a directory that holds `.git`.

### E. Versions and resolution

20. **Versions.** Semantic versioning 2.0.0, with two restrictions: no build metadata (`+…`), since it does not order; and each numeric part at most 2^31 − 1. Order is semver precedence. A release is immutable: once logged, a name and version always mean one digest (rule 36).

21. **Identities and major versions.** The identity of a package is its source, its name, and its major line: `MAJOR` when it is at least 1, and `0.MINOR` otherwise. `json 1.x` and `json 2.x` are two identities and may both be in one program, each a distinct set of modules and kinds; a package that needs both gives one a local name with `as`.

22. **Requirements.** `json 2.1` in a need means "at least `2.1.0`, in the identity `json 2`". `json 0.3` means at least `0.3.0` in `json 0.3`. A pre-release is selected only if some requirement names a pre-release of that identity. There are no upper bounds and no ranges: a need states the least version its author tested.

23. **Minimal version selection.** The program's build list is computed so:

    ```text
    sel  := empty map  identity -> version
    seen := empty set  of (identity, version)
    work := the root's needs (after `replace`)
    while work is not empty:
      (id, v) := take from work
      if (id, v) in seen: continue
      add (id, v) to seen
      sel[id] := max(sel[id], v)
      for each need (id2, v2) of the release (id, v): put (id2, v2) in work
    raise (rule 24), which may add to work and repeat the loop
    list := the identities reachable from the root by the needs of the releases in sel
    ```

    - **What it gives.** For each identity, the highest of the minimum versions that any reached release asks for, never a newer one, except where rule 24 raises past a yanked, advised or avoided release.
    - **Reproducible.** The result is a function of the needs and of the per-name state (yanks, advisories, logged times) at one checkpoint, the lock's resolution point (rule 15). It does not depend on the order of the work list, and a release published later changes nothing until a need asks for it or a later resolution point is chosen.
    - **Where needs come from.** For a registry release, from the release statement's `needs`, each with its source (rule 37), so resolution downloads no archive. For a path or workspace origin, from the `ita.una` read at that origin.
    - **Complexity.** O(N + R) time and space, for N releases reached and R need edges among them, with hash maps for `sel` and `seen`, and O(N) fetches of release statements, done concurrently. There is no backtracking and no search: version solving with ranges and upper bounds is NP-complete, and MVS avoids it.
    - **Bounds** (U7013 past any): 100,000 releases visited; 10,000 packages in the list; 1,000 needs per release; 10,000 releases read per name; 64 KiB per release statement and 16 MiB per other registry document; 4 GiB of archives per program; 60 seconds per fetch without progress. A need on the package's own identity is U7013, so a chain of releases that each need the next one of their own identity cannot drive N.

24. **Yanks, advisories and `avoid`.** After the loop, `sel[id]` is raised when, at the resolution point, it is yanked, is named by an advisory not fixed in it, or is avoided by the root.
    - **To what.** The least release of the same identity above it that is none of these, is not a pre-release (unless a requirement names a pre-release of that identity, rule 22), and was logged at least the cooldown before the resolution point (rule 25, measured as rule 36.4 states). The loop then resumes with that release's needs.
    - **The advisory exception.** An advisory that names its fix by digest, and is signed by two security keys of the root metadata, or by the security key and the owner's threshold (rule 41), lets the raise take that fix whatever its age.
    - **No such release:** U7013 naming the identity and the newest candidate held back by the cooldown, with `--now` as the explicit override.
    - Raising only increases versions, and each release is visited at most once, so the bound of rule 23 holds.
    - **A lock already written is never changed by a yank:** it keeps building, and `una ita audit` reports it (rule 41).

25. **Changing the lock.** Only explicitly:
    - `una ita lock` resolves `ita.una` at a fresh resolution point and writes the lock. It never raises a need written in `ita.una`; it may raise a selection by rule 24.
    - `una ita add <name> [<version>]` adds a need at the given version, or at the newest release of the newest major past the cooldown, then locks. An explicit version logged within the cooldown is held like any other (U7013), and taken only with `--now`: naming a version is not consent to skip the wait. It writes no `may` and no `trust` line: a dependency that needs authority or trust is then rejected by `una check` (U3021, U7012) until the programmer writes them.
    - `una ita update [<name>…]` raises the named needs, or all, to the newest release of their identity that is not a pre-release (unless the need names one), is not yanked, avoided or under advisory, and was logged at least the **cooldown** before the resolution point: 72 hours by default, `--cooldown <hours>`, `--now` for none. Only rule 24's advisory exception takes a younger release. It never changes a major version.
    - **Every change prints a review** before writing: per package, its source, the old and new version, digest and publisher key; and, by checking the new graph at tier 0, the change in its authority (the union of its `uses`), in its trusted modules, and in its ceiling violations.
    - **Refused without a flag:** a widened ceiling (`--accept-authority`), a trust change of rule 10 (`--accept-trust <package>`), and a key change, transfer or recovery of a locked package's owner (`--accept-key <package>`). Each flag names what it accepts, and the lock records it.
    - `--dry-run` writes nothing. `--offline` resolves from Itaoca's stored per-name state only, at its stored resolution point.

26. **Authority is part of the interface.** Within one identity, a release needs no authority the highest earlier release did not: no new capability or operation in the `uses` of a public function or handler, no new host capability, no new trusted module.
    - **The registry enforces it.** It refuses to log a release statement whose `authority` or `trust` (rule 37) is not within those of the highest earlier release of its identity (U7013 to the publisher, with the repair "publish it as the next major version").
    - **Clients verify it** from the two statements when they lock or update (U7010 for a logged statement that breaks it), and every statement's `authority` and `trust` are checked against the archive (rule 37). So neither a modified publishing client nor the registry alone can slip authority into a minor release.
    - **Header compatibility** (every public function and kind of the earlier release exists with a header that accepts what it accepted) is checked by `una ita publish` as advice to an honest publisher, and is not enforced.
    - A release that needs more authority than a consumer's ceiling is U3021 in the consumer anyway.

### F. Package content and digest

27. **What a package holds.** Exactly: `ita.una`; every `src/**/*.una`; `README.md`, `CHANGELOG.md`, and files named `LICENSE` or starting with `LICENSE-` or `LICENSE.`. Nothing else: no library, archive or source file of another language, since Una admits no foreign code (rule 60). Not `ita.lock`: a dependency's lock is not read (rule 23 uses its `ita.una`). Every path:
    - is relative, of segments `[A-Za-z0-9_.-]+`, none `.` or `..`, none starting with `.` (hidden entries, as `spec/13 §7` excludes them), none ending with `.`, at most 32 segments and 255 bytes;
    - has no segment that is a reserved device name of Windows, with or without an extension, compared ignoring case: `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`;
    - is unique ignoring ASCII case, and is not, ignoring case, a directory prefix of another path (`a` beside `a/b`), so no file system can merge two entries or make a file a directory;
    - names a regular file: no symbolic or hard links, devices or directories as entries, and no mode bits are kept, so nothing is executable.

    At most 10,000 files and 256 MiB. A violation is U7010 when reading and refuses `una ita publish`.

28. **The content digest.** The listing of a package is, for each file in ascending byte order of its path: the path's bytes, a NUL byte, the file's length in decimal ASCII, a NUL byte, the lowercase hex SHA-256 of its contents, and a line feed. The digest is `sha256:` followed by the hex SHA-256 of the listing. It depends only on the files' paths and bytes: not on timestamps, owners, modes, order of creation or the archive that carried them.

29. **The archive.** A package travels as one file: the ASCII line `ita-archive/1`, a line feed, the listing's length in decimal and a line feed, the listing, then each file's bytes in listing order, and nothing after. It has no compression, so there is no decompressor in the trust base and no decompression bomb. The reader:
    - reads the archive once into memory, and parses and hashes those same bytes;
    - rejects a listing whose paths are not in strictly ascending byte order, a length with a leading zero or above 256 MiB, and any path that rule 27 rejects;
    - checks every length and hash against the listing as it reads, rejects trailing bytes, and never trusts a length beyond the file's size.

### G. Itaoca, vendoring and offline builds

30. **Itaoca.** `~/.una/itaoca`, or the directory `UNA_ITAOCA` names, holds:
    - `packages/sha256/<hex>/`: each fetched package, unpacked once, its files read-only;
    - `registry/<source>/`: per source, the root chain, the accepted checkpoints with their co-signatures, the release statements with their inclusion proofs, and the per-name chains and map proofs of rule 36.1 that a client has verified, up to the last resolution point of each lock it served;
    - `proofs/`: the shared proof cache of rule 48.

    - **Unpacking** writes into a fresh directory through directory descriptors with `O_NOFOLLOW` and `O_EXCL` on every component, then renames it into place.
    - **Reading,** from Itaoca, a vendor directory or a path dependency, opens every component with `O_NOFOLLOW` through a directory descriptor, rejects every entry that is not a regular file or a directory, reads each file once, and parses and hashes the same bytes.
    - **A package is verified on every load,** not once: the checker hashes the bytes it reads and compares the digest with the lock (rule 17). A store tampered with after the fetch gives U7010, never a different program.

31. **Only `una ita` uses the network.** `una check`, `una run`, `una build`, `una test`, `una report`, `una core`, `una lsp` and `una migrate` never open a connection. A program whose locked packages are present builds with no network and gives the same result with or without one.

32. **Vendoring.** `una ita vendor` copies each locked package into `vendor/ita/<source>/<name>-<version>/` of the root (an owner's `/` written as `+`), verifying each digest. Beside each registry package it stores the release statement, its inclusion proof and the per-name state of rule 36.1 at the lock's resolution point, as a bundle does (rule 33). Loading prefers the vendor directory to Itaoca, and verifies both the same way (rule 17), provenance included. A repository with its vendor directory and lock committed builds anywhere with `una` alone.

33. **Air-gapped machines.**
    - `una ita bundle <file>` writes the locked packages, as archives, with their release statements, the inclusion proofs of their log entries, the per-name state of rule 36.1 at the lock's resolution point, the co-signed checkpoints that prove when each entry was logged (rule 36.4), the resolution point's checkpoint with its co-signatures, and the registry's root chain, into one file of the format of rule 29.
    - `una ita unbundle <file>` on the isolated machine verifies all of it against the roots built into `una` (rules 35 to 39), with no clock freshness required, and fills Itaoca.
    - An organization MAY also run a full mirror inside its network (rule 40).

### H. The Itaty registry

34. **Static files.** A registry is a set of files served over HTTPS at fixed paths below its base URL. Nothing is computed per request, so any web server or content delivery network can serve it, and any mirror can copy it.
    - `root/<n>`: the root metadata, version `n` (rule 35).
    - `checkpoint`, `checkpoint/<size>` and `tile/…`: the transparency log in the C2SP `tlog-tiles` layout, with every co-signed checkpoint kept by size (rules 36 and 36.4).
    - `map/…`: the tiles of the name map of rule 36.1.
    - `blobs/sha256/<hex>`: the archive of rule 29 with that digest.

    A name's releases, yanks, advisories and key events are found through the name map and the name's chain (rule 36.1), never through an unverified index.

35. **The root of trust.**
    - **Root metadata** names, with an expiry date: the root keys and their threshold; the log's key; the witnesses' keys and how many co-signatures a checkpoint needs; the security keys that sign advisories, and how many are needed for rule 24's exception; and the keys of Itaúna with their threshold (rule 38.1). It is a C2SP `signed-note` signed by at least the threshold of root keys.
    - **Itaty's root** has five Ed25519 keys held offline, threshold three. **Its first root metadata, and its digest, are built into `una`.** A new version `n + 1` is accepted only if signed by the threshold of the keys of version `n` and of its own. A client walks the chain from the version it knows; versions it passes may have expired, the last one MUST NOT have (U7010 otherwise, against the witnesses' time of rule 36.4). So a root key compromise below the threshold cannot move the root, and keys are rotated without a release of `una`.
    - **An old `una`** whose built-in root has expired, and that cannot reach a newer root version, can still build every lock whose provenance it stored (rule 17); its network commands for that source stop with U7010, naming the expiry, until it reaches a newer root or is upgraded.
    - **Another source's root** is pinned by its digest in `sources` (rule 8), and walked the same way.
    - The online keys (log, security) are replaced by a new root version, without the root keys ever being online.

36. **The transparency log.**
    - Every release statement (rule 37), yank, advisory, key event (rule 38) and transfer (rule 46) is a leaf of one append-only Merkle tree, in the format of RFC 9162, served as C2SP tiles.
    - The log signs checkpoints. **A client accepts a checkpoint only with co-signatures from at least the number of witnesses the root names** (Itaty: three of at least five, operated by parties other than the registry). A witness co-signs only after verifying a consistency proof from the last checkpoint it signed, and the name map's update (rule 36.1). So two different logs, or a map that hides an entry, cannot both be co-signed.
    - **A client verifies, for every entry it uses, an inclusion proof** against an accepted checkpoint, and, **for every checkpoint it accepts, a consistency proof** from the last one stored in Itaoca. A failure is U7010, naming the registry, and stops every network command for that source until the user resolves it: it is evidence that the registry misbehaved, or of an attack.
    - **Immutability follows:** a name and version are bound to one digest by its first logged statement. A second statement for the same name and version is invalid, and since every client sees a name's whole chain (rule 36.1), every client rejects it.
    - **Monitoring.** `una ita watch <owner>` follows the chains of the owner's names from a stored position and prints every entry, so an owner learns of a release or key event it did not make. The registry also notifies owners, but nothing relies on it.

    36.1. **Completeness: per-name chains and the name map.** An inclusion proof shows that an entry is in the log, not that nothing was left out. So:
    - **Every entry about a name** (an owner name for its key events, recoveries and disputes; a package name for its releases, yanks, advisories and transfers) carries the log index and leaf hash of the previous entry about that name, or none for the first. The entries of a name form a hash chain. A transfer of a package is an entry of the package's chain, and is also referred to by an entry in the chains of both owners.
    - **The name map.** Each checkpoint carries, in a signed extension line, the root of a map from SHA-256(name) to the index and leaf hash of the name's latest entry and its count of entries: a sparse Merkle tree of depth 256, stored as tiles (rule 34).
    - **Witnesses** replay the entries since the last checkpoint they co-signed before co-signing (rule 36), at a cost linear in the new entries. For each new entry they MUST verify that its `previous` equals the name's map head just before it, and that the name's count rises by exactly one; then they recompute the map's root. Otherwise they refuse to co-sign.
    - **A client** that needs a name's state verifies the map's proof for that name against an accepted checkpoint, then walks the chain from the head back to the last entry it already holds, verifying each link and each entry's inclusion. **The number of links it walks MUST equal the new count minus the count it held**, or the full count on a first walk; a mismatch is U7010. A map that omits a yank, an advisory, a revocation, a recovery or a transfer is caught at that walk or by the witnesses, and is U7010. So is a name a client reads with no map entry while its chain exists.
    - **Cost:** O(log M) hashes for the map's proof, in practice about 2·log₂ M for M names, plus O(E) for the E entries of the name not seen before; then O(1) per later lookup.

    36.2. **Censorship.** The registry answers each submission (release, yank, key event) with a signed promise to log it within one hour. A promise that is not met is evidence against the registry: `una ita watch` reports it, and the owner can publish it. A registry can delay an owner's yank only by breaking a promise it signed.

    36.3. **Integrity of order.** A key event takes effect at its log position, and a statement is valid only if its signing key is valid at its own position, which the chain of the owner's name (rule 36.1) gives completely.

    36.4. **Time.** No rule uses a publisher's or the registry's clock, and no rule uses a single witness's. With k the witness threshold of the root metadata, every time is an order statistic over k distinct witnesses, so fewer than k witnesses can move none of them in an attacker's favor.
    - **Monotonic witnesses.** A witness MUST NOT co-sign with a timestamp below its own previous one. A client checks it across every co-signature of one witness it holds, and a decrease is U7010 against that witness.
    - **Logged at,** the start of a wait, where an attacker wants an earlier time. For each distinct witness, its earliest co-signature timestamp (C2SP `cosignature/v1`) on a checkpoint that includes the entry; the entry is logged at the k-th earliest of these. A client proves it with the co-signed checkpoints at `checkpoint/<size>` and the entry's inclusion proofs in them. A witness that back-dates moves nothing unless k witnesses do.
    - **The witness time of a checkpoint,** the end of a wait and the measure of freshness, where an attacker wants a later time: the k-th latest of the distinct witnesses' co-signature timestamps on that checkpoint. A time T is reached at a checkpoint only if at least k distinct witnesses co-signed it at or after T.
    - **Durations** (the cooldown of rules 24 and 25, the recovery delay of rule 38.1) are the witness time of the later checkpoint minus the logged-at time of the entry, never a local reading.
    - **Freshness** (rule 39) alone compares a witness time with the local clock. A local clock more than one hour behind the witness time of the newest checkpoint a client holds is reported as clock skew, U7010, and network commands stop until it is corrected; a clock ahead only makes freshness stricter.

37. **Release statements.** A release is the signed note:

    ```text
    ita-release/1
    source itaty
    name acme/ledger
    version 0.3.1
    previous 47755 sha256:aa04…      (the previous entry of this name, rule 36.1)
    digest sha256:3b1f8e0c…
    manifest sha256:77aa10f3…
    edition 0.4
    needs itaty json 2.1.0
    needs itaty text_utf8 1.0.3
    authority Http[get]
    trust modules ledger.keys
    ```

    with one Ed25519 signature line per signing publisher key.
    - `needs` repeats the release's needs, each with its source, which is always the release's own source (rule 42).
    - `authority` is the union of its public functions' and handlers' `uses` and host capabilities; `trust` its trusted modules.
    - **A client verifies that the archive's `ita.una` and computed authority and trust agree with the statement** when it fetches the archive (U7010 otherwise), so the statement cannot advertise less than the code needs.
    - **The registry logs a statement** only if it is signed by keys of the name's owner valid at that point of the log, by the owner's threshold (rule 38); its needs name releases already logged at its source; and it meets rule 26.

38. **Owners' keys.**
    - **Keys.** Ed25519, verified strictly, by one rule in every implementation: RFC 8032 encodings, a canonical `S` (below the group order), public keys and `R` of small order rejected, and the cofactorless equation. A key's fingerprint is the SHA-256 of its public key; `signed` in the lock is `ed25519:<owner>:<fingerprint>`. A C2SP signed-note names a key by a 4-byte identifier, so a client tries every key of the owner with that identifier, and accepts the note only if one of them verifies it.
    - **Key files** live in `~/.una/keys/`, readable by the user only, **encrypted with a passphrase by default** (`--no-passphrase` for a machine whose disk is protected otherwise).
    - **Registration.** An owner's first key is registered when the owner name is created, after the registry verifies the account (an email address, a second factor, and for an organization optionally a DNS record of its domain); the event is logged. Every later key is added by a statement signed by the owner's threshold of current keys.
    - **Threshold.** An owner MAY require k of its n keys on every release and key event.
    - **Rotation and revocation** are logged statements signed by the owner's threshold. A revocation MAY state "compromised since log entry p": releases the key signed after p are then flagged by `una ita audit` (U7014), and new resolutions skip them as yanked. Without that clause, releases it signed before the revocation stay valid.

    38.1. **Recovery.**
    - An owner that lost its keys asks the registry, which logs a **pending recovery** naming the new key. An owner MAY designate recovery co-signers in advance; then the request also needs their threshold.
    - **Any current key of the owner can cancel it**, by a logged statement, until it takes effect.
    - **A thief who holds one key** of an owner whose threshold is above one could cancel every recovery, while the owner, below its threshold, cannot revoke that key. The fallbacks: recovery co-signers designated in advance, whose threshold can revoke a key and overrule a cancellation; otherwise a registry dispute, logged in the owner's chain, taking effect after the same seven days of witness time and shown to every client as a key change. Itaúna has neither: its keys change only by a root version.
    - **It takes effect** only on a checkpoint whose witness time is at least seven days after the witness time of the checkpoint that first included the request (rule 36.4), with no cancellation in the owner's chain between them. Each client checks this itself; the registry's word is not used.
    - **Pending recoveries** of the owners of locked packages are reported by `una ita audit` and `una ita update` (U7014). A recovery that took effect is a key change (rule 25).
    - **Itaúna has no registry recovery.** Its keys and threshold are named in the root metadata (rule 35), and change only by a new root version.
    - **So account takeover alone publishes nothing:** publishing needs the owner's keys, and a recovery is public, delayed, verifiable and cancelable by the keys it would replace.

39. **What a client verifies, and freshness.**
    - For every registry package it writes into a lock or fetches: the root chain; a checkpoint with enough witness co-signatures; the consistency proof from its stored checkpoint; the name map's proof and the name's chain (rule 36.1); the inclusion proof of the release statement; the statement's signatures against the owner's keys valid at its log position; the archive's digest against the statement; the statement's needs, authority and trust against the archive (rule 37); and rule 26 against the previous release.
    - **Freshness.** Resolution (`una ita lock`, `add`, `update`) and `una ita audit` require a checkpoint whose witness time (rule 36.4: at least k distinct witnesses co-signed that checkpoint) is less than 24 hours old by the local clock, so neither a mirror, a network attacker, nor fewer than k witnesses re-co-signing an old checkpoint can freeze a client on an old view. `una ita fetch` of a lock needs no freshness: the digests and the stored provenance of the lock decide.
    - **Floors.** A client never accepts a checkpoint smaller than the largest it stored, or than the resolution point of a lock it reads, even with an empty Itaoca; `unbundle` never moves the stored checkpoint backwards.
    - **Witness outages.** If too few witnesses co-sign for more than 24 hours, resolution and audit stop with U7010 naming the cause; builds from a lock are unaffected.
    - **TLS** protects the confidentiality of a private registry's credentials, not integrity, which rests on digests, signatures and proofs alone.

40. **Mirrors.** `~/.una/itaoca/mirrors.una` lists, per source, URLs to try before the source's own:

    ```una
    mirrors
      itaty
        "https://ita-mirror.example.net"
    ```

    A mirror serves the same paths. Everything it serves is verified as from the source itself, the name map and chains included (rule 36.1), so it cannot hide an entry of a name from a checkpoint it serves. It can refuse service, or serve an old checkpoint, which rule 39's freshness bounds. A mirror that serves a failing proof is skipped, and U7010 names it.

41. **Yanks and advisories.**
    - **Yank.** An owner yanks a release by a logged statement. New resolutions skip it (rule 24); existing locks keep it. Nothing is deleted: the statement stays in the log, and the archive stays served. If a court order removes an archive, the registry serves a logged withdrawal in its place, and locks that hold the digest still build from Itaoca, a vendor directory or a mirror.
    - **Advisory.** A signed, logged statement in the chain of the name it concerns: an identifier, the affected identity and versions, the fixed version and its digest if any, a severity and a summary. It is signed by a security key of the root metadata. Only an advisory that also carries a second security key's signature, or the owner's threshold, can lift the cooldown for the fix it names (rule 24); a single security key can hold back a release, never hurry one in.
    - **`una ita audit`** checks the lock against the per-name state at a fresh checkpoint: yanks, advisories, revocations with "compromised since", pending recoveries, key changes and transfers since the lock's resolution point. It exits with code 1 on any (U7014 at the need, one per package). It is the command a project's release gate runs. `una build` stays offline and deterministic, and does not call it.

42. **Sources of the program, and dependency confusion.**
    - **One source per need.** Every need resolves at exactly one source, and resolution never asks two sources for one name and never falls back from one to another. The lock records each package's source, and a package changes source only by an edit of `ita.una` or `itaty.una`.
    - **Owners are bound to their source.** A need of a name whose owner a root's source lists (rule 8, `owners`) resolves only there, and must say so; `una ita add` picks it. So `una ita add acme/ledger` never goes to Itaty when the root serves `acme` from its own registry.
    - **Only the root names sources.** Only the root and `itaty.una` declare sources (rule 8). A package that is not the root:
      - if it comes from a registry, may need only packages of that same registry, and its statement says so (rule 37), which the registry and the client verify; it has no `path` or `workspace` need;
      - if it comes from `path`, may need only registry packages that the root lists and `workspace` members;
      - a need of a transitive registry package that omits `from` resolves at that package's own source, never at Itaty by default; a need of a path package that omits `from` resolves at Itaty, and one that names a source names it by the root's source names, so a path package's `from internal` binds to whatever the root calls `internal`, which `una ita add` and every review print.

      A need that breaks this is U7013. So no dependency makes a program contact a host or trust a root that the root did not name, or read a local directory.
    - **Names of local packages.** A `path` package whose name is unscoped, or whose owner a source of the root serves (Itaty's owners included), is U7012 unless the root's need says `accept name` (rule 7). A transitive one cannot be accepted, since it cannot exist (above).
    - **The report, `una ita tree` and every review** print each package's source beside its name. A package of an owner name that the root binds to another source, such as an Itaty `acme/util` needed by a public dependency while the root serves `acme` from its own registry, is a distinct identity and harmless to imports, but is marked "same owner name as source acme" in every review, in the tree, and in the report (`"owner_elsewhere": "acme"`).

43. **Publishing.** `una ita publish`:
    - refuses without `publish to` in the head, without `about` and `license`, with a `sources`, `replace` or `trust` section, with a need of another source or of a `path` or `workspace` origin (rule 42), from a workspace member with uncommitted changes (when the package is in a git repository), or when `una check` of the package at tier 1 reports any diagnostic or any obligation not proven;
    - builds the archive, checks rule 27, and checks header compatibility against the previous release of the identity (rule 26, advice);
    - writes the release statement, signs it with the owner's keys, and uploads both.

    The registry checks the archive's format, the name's owner and keys, the statement against the archive (rule 37), that every need is of its own source and already logged, rule 26, and the limits; then it logs the statement and returns its promise and, once logged, its index. It does not run the package; whether it checks is the consumer's to verify (rule 48).

### I. Names

44. **Name forms.**
    - **Unscoped names**, such as `json`, belong to Itaúna and are published only with the Itaúna keys under the project's release gates.
    - **Owner names**, such as `acme`, are `lower_ident` of 2 to 39 bytes; **package names** under an owner, `acme/ledger`, are `lower_ident` of at most 64 bytes. Both follow U1005's rule (no `__`, no trailing `_`).
    - The local name of `acme/ledger` is `ledger`.

45. **Reserved and confusable names.**
    - **Reserved:** every keyword of `spec/01 §5` of any edition, `std`, `prelude`, `una`, `ita`, `itaty`, `itaoca`, `itauna`, `path`, `git`, `workspace`, and the names of the host capabilities in lower case. The registry refuses them and names with their skeletons; clients refuse them as local names and as the last segment of any package's name, whatever its origin (rule 7).
    - **Skeletons.** A name's skeleton removes `_`, maps `0` to `o`, `1` and `i` to `l`, `2` to `z`, `3` to `e`, `4` to `a`, `5` to `s`, `7` to `t`, `8` to `b`, `9` to `g`, `rn` to `m`, `vv` to `w`, and collapses repeated letters. A new owner name whose skeleton equals an existing one's is refused, and so is a new package name under one owner whose skeleton equals another of that owner's, or an unscoped name's.
    - **Distance.** A new owner name at edit distance 1 from one of the 1,000 owners with the most dependents is held for review by the registry. The check uses a deletion index (each name with one character deleted), so it costs O(L) lookups for a name of length L, not a scan.

46. **Transfers.** A package name moves to another owner only by a logged transfer in the package's own chain (rule 36.1), signed by the old owner's threshold, or with its owner by a recovery (rule 38.1). Clients treat it as a key change (rule 25). Names are never reassigned for inactivity.

### J. Dependencies in the language

47. **Modules and imports.** A package's modules import, besides its own modules and the standard modules, the modules of its direct dependencies, under their local names: the module `client` of the dependency `http` is `http.client` in the importing package (`import http.client (get)`).
    - **Only direct dependencies.** A module may import from a package only if its own package names it in `needs`. A transitive dependency is not importable (U1001), so no package relies on a package it did not ask for.
    - **No overlap.** A module of a package whose first path segment equals one of its dependencies' local names is **U1010** at the module header, with the repair of renaming the need with `as`. A local name is never `std` or `prelude` (rule 7).
    - **Identity** (`spec/10 §11` rule 8) becomes the package's identity and version, with the module path and the name. Kinds of two identities, such as `json 1` and `json 2`, are different kinds. The report and the Core export write a declaration of a dependency as `<local name>.<module>.<name>` with a `package` field (rule 58).
    - **Inside a dependency**, its modules see its own modules and its own direct dependencies, by its own local names, exactly as when it is the root.

48. **One program, checked and proven by the root.**
    - `una check` of the root checks every module of every package of the program, tiers 0 and 1, each under its own package's edition and profile, with the root's solver pin. A diagnostic in a dependency is reported at its file in Itaoca or the vendor directory, and counts as any other: the program does not check.
    - **Nothing is taken from the publisher**: no cached result, no proof, no report. The release statement's `authority` and `trust` are compared with what the checker computes (rule 37), never used in its place.
    - **The proof cache across packages.** Obligations of every package are cached in `~/.una/itaoca/proofs/` as well as in the root's `.una/cache/`, under the key of `spec/12 §9`, with the HMAC of ADR 0064 under the user's own key. An obligation is the same query in every program that contains it, so a dependency is proven once per machine and solver pin. **The key** is SHA-256 over the domain tag `una-proof-cache/2`, then, for each of the canonical text of the lock's `solver` section and the query, its length in decimal, a NUL byte and the text. It is not over the whole lock, so adding a dependency invalidates nothing; its tag and length prefixes keep it apart from the key of editions `0.1` to `0.3`, SHA-256 over `verification.lock` and the query (amending `spec/12 §9`). The HMAC of ADR 0064 authenticates it either way.
    - `una fix` never edits a file of a dependency: its repairs there are skipped with the reason `not a module` (`spec/13 §5`).

49. **Editions across packages.** Each package is checked under its own edition. A package MAY depend only on packages whose edition is its own or earlier (U7012 at the need otherwise), so no dependency opts a package into a preview it did not name (Question 8.2). The cumulative rule of ADR 0056 makes a declaration of an earlier edition usable from a later one.

50. **Profiles across packages.** Each package is checked under its own profile (ADR 0109). A dependency whose profile is `systems` used by an `app` root is checked as `systems`, which only adds obligations (ADR 0109 rule 2). What a dependency's profile cannot change is the trust it adds: its trusted modules need the root's trust line (rule 53), whatever either profile is. Constructs gated on `systems` that add no trust, such as SIMD in draft ADR 0120, need nothing of the root.

51. **Authority: every dependency is a boundary.**
    - Every module of every dependency is in a boundary of ADR 0093, one extent per package (rule 9 of that record), except the trusted modules of rule 53.1, which are extents of their own.
    - **The ceiling at a call** into a package is the one that the calling package writes for it, with `may` (rule 7). A call into a package that its caller does not need, such as a function value or a handler of a transitive dependency that a direct one hands back, has the empty ceiling. A dependency's `ita.una` therefore sets the ceilings of its own calls, never of the root's.
    - A call into a dependency, a function of a dependency used as a value, and a handler of a dependency installed outside it, are calls into a boundary: every capability it needs is granted with `with` (U3020), within that ceiling (U3021), and no release may be reachable except as rule 53.1 allows (U3024), as ADR 0093 rules 3 to 15 state. There is no inheritance by name (Question 2).
    - **The effective ceiling** of a package bounds what it can ever exercise, and is reported (rule 58): the least fixpoint of `eff(p) = ⋃ over the packages d that need p of (ceiling_d(p) ∩ eff(d))`, with `eff(root)` everything the root's grants allow. A worklist computes it in O((P + R) · C) for P packages, R needs and C capability operations, cycles included.
    - **Repairs.** U3020 and U3021 keep ADR 0093 rule 6: a repair never widens a ceiling, and `una fix` never edits `ita.una`.

52. **Grants are the root's.** Only the root's `run grants` and `--grant` give host capabilities, and only to `main` or the driver of the root's entry (`spec/05 §7`). A dependency's `run` and `drive` sections are not read.
    - **Tests.** `una test` runs the examples and tests of the root's modules only.
    - **A dependency's examples and tests** are checked in the root's program as the dependency's own: under its own `boundaries`, with no consumer ceiling applied to their `given` lines, since they are never run with the root's authority. A ceiling is never widened to make a dependency's tests check.

53. **Trust a dependency adds needs the root's consent** (ADR 0121 rule 2 (a), draft).
    - **Trusted modules.** A dependency's trusted module, one that its own `trusted` section and header declare (`spec/07 §6`), is trusted in the program only if a trust line of the root names it, by identity and module path (rule 10). Otherwise U7012 at the root's need, or at the package's line in the lock for a transitive dependency.
    - **Consent is by identity and pinned to the whole package.** The lock records the content digest of each package whose trust was consented (rules 10 and 16), so no byte of it can change, and no new trusted module can appear in the program, without `--accept-trust` (rule 25).
    - **The registry and the packages themselves** are trust the package adds by writing `needs`: the root's `needs` section is that opt-in, and the report names each registry root and each package's digest and publisher key in `tcb` (rule 58).

    53.1. **A trusted module of a dependency is an extent of its own.** ADR 0093 makes a boundary module never trusted (its amendment rule 8), and `spec/13 §7` makes a trusted module inside a boundary U7002. For a module that a trust line of the root names, and for it alone:
    - it is its own extent, inside its package's boundary: calls into it from outside it, its package's other modules included, are boundary crossings, granted with `with` (U3020). Their ceiling (U3021) is the one written under the trust line (rule 10), and for a call from another package also within that caller's ceiling for the package (rule 51). The root alone sets what a trusted module of a dependency may receive;
    - U3024 does not apply to a release whose policy is declared in it, whoever reaches it. It still applies to every other release the package can reach, the root's own policies included, as ADR 0093 states;
    - **calls from it into the rest of its package** are crossings out of the trusted extent: every capability they need is granted with `with` (U3020), within the ceiling written under the trust line (U3021), empty without `may` lines, so the trusted module passes on nothing it was not given. The code those calls run is pinned with the rest of the package (rule 10);
    - every other module of the package stays in the package's boundary, untrusted, with ADR 0093 intact.

    The amendment of ADR 0093 (Appendix A) and of `spec/13 §7` (its U7002 for "a module of `[trusted] modules` inside a boundary" excepts a dependency's module that the root trusts by name) state the same.

54. **Labels across packages.** The categories of the program are the union, by name, of every package's `labels`. A module may write only the categories its own package declares (U4007). Merging by name never accepts a flow it would reject, since categories form a powerset (`spec/07 §1`) and every flow is checked in the whole program; a dependency's policy that releases a category is trust the root must consent to (rule 53). **A known limitation:** two packages' unrelated categories that share a name, such as two meanings of `PII`, become one. Categories qualified by package may come in a later record.

55. **Whole-program analyses** run over the program's call graph, every package included:
    - **The file-state label** (ADR 0085) joins every `Files.write` of the program. The whole program is known when the label is computed, which is the condition ADR 0085 decision 5 states, so `pub` functions keep the label their callers give them (Appendix A).
    - **`conc(f)`** (ADR 0116) and the effect bounds of ADR 0092's case (d) are computed over the same graph.
    - **Consequence:** adding a dependency, or adding code to the root, may raise a label inside a dependency, which then fails to check. That is sound, and it is reported at the dependency's line with the root's write as the related span.
    - **The formal track.** The Core export of a root is one document for the whole program (rule 59), so "every caller of a declaration is in the document" holds, and the document-level flag of `docs/plan/formal-track-m5.md` stays sound per document. The authority-confinement theorem of ADR 0093 rule 7 is proved (#562); until the open obligation for its amendment rules 9 to 15 (#629) is discharged, calls into dependencies are reported as not re-checked where those rules apply, as calls into boundaries are.

56. **Assumptions.** A capability a dependency declares, with its `ensures`, is listed in the report's `assumptions` with its package. A handler of it, wherever written, is verified against those `ensures` (`spec/05 §5`), as today.

### K. Commands

57. **`una ita`.** Each command follows `spec/13 §5`: `--json` prints one document, `una.ita/1`, with `schema` first; exit code 1 when its output holds an error diagnostic; 2 for a usage error.

    | Command | What it does | Network |
    | --- | --- | --- |
    | `una ita add <name> [<version>] [--as <n>] [--from <source>]` | adds a need and locks (rule 25) | yes |
    | `una ita remove <name>` | removes a need and locks | no |
    | `una ita lock [--check] [--accept-authority] [--accept-trust <p>] [--accept-key <p>]` | writes the lock at a fresh resolution point, or checks it at its own (rules 17, 25) | to write, unless `--offline`; `--check` never |
    | `una ita update [<name>…] [--cooldown <h>] [--now]` | raises needs past the cooldown and locks (rule 25), with the same `--accept-…` flags | yes |
    | `una ita fetch` | fills Itaoca with the locked packages | yes |
    | `una ita vendor` | copies them into `vendor/ita/` | no |
    | `una ita bundle <file>`, `unbundle <file>` | the air-gap file (rule 33) | no |
    | `una ita tree` | the program's packages, with versions, ceilings, authority and trust | no |
    | `una ita verify` | re-verifies Itaoca and the vendor directory against the lock and the stored proofs | no |
    | `una ita audit` | yanks, advisories, compromised keys, pending recoveries, key changes and transfers (rule 41) | yes |
    | `una ita publish`, `yank <version>` | rule 43, rule 41 | yes |
    | `una ita key new`, `key rotate`, `key revoke <fp>` | rule 38 | yes |
    | `una ita watch <owner>` | rule 36 | yes |

    `una migrate` is rule 64.

### L. The report and the Core export

58. **The report.**
    - Each entry of `authority` and `handler_authority` gains `package`, the package's name and version, after `function` or `handler`.
    - A new section `dependencies`, after `handler_authority`, one entry per package of the program other than the root, in name order:

      ```json
      "dependencies": [
        { "package": "http", "version": "1.4.2", "local": "http", "source": "itaty",
          "digest": "sha256:3b1f…", "signed": "ed25519:itauna:7c0a…", "replaced": false,
          "edition": "0.4", "profile": "app",
          "granted_by": [ { "package": "store", "source": null, "may": ["Http[get]"] } ],
          "effective_ceiling": ["Http[get]"],
          "authority": ["Http[get]"], "trust": { "modules": [] } } ]
      ```

      `granted_by` lists, per dependent, the ceiling it writes, which is the ceiling of that dependent's calls (rule 51); `source` is `null` for the root. `effective_ceiling` is rule 51's fixpoint, and `authority` the union of the `uses` of its functions and handlers, always within the effective ceiling. The human form is one line per package, with its source beside its name: `http 1.4.2 (itaty, signed itauna): may Http[get] (by store); uses Http[get]; trust none`.
    - `tcb` gains `registries`, each source's name and root digest, and `dependency_trust`, the trusted modules of dependencies, with line counts, by package. `trusted_modules` keeps the root's own.
    - Every key is additive, and the schema stays `una.report/2`. A package with no dependencies has `"dependencies": []`.

59. **The Core export** of a root is one document for the whole program.
    - A document with more than one package is `una.core/2`: it gains `packages`, after `edition`, each package's name, source, version, digest, edition and profile, and each module gains `package`. A module path is unique within a package, and the pair is unique in the document.
    - A document of a package with no dependencies stays `una.core/1`, unchanged, so a consumer of `una.core/1` never meets two modules with one path.
    - The formal re-check reads `una.core/2` from the release that writes it (`spec/13 §8`).

### M. No foreign code

60. **A package is Una code only.** Una admits no foreign code in any form, and a dependency is no exception.
    - A package holds only the files of rule 27: no library, archive, object file or source of another language is part of it, and none is covered by its digest.
    - `ita.una` has no `foreign` section and `run grants` has no foreign grant (rule 5): either is U7008.
    - Nothing of a package is compiled or run by anything but the checker, the interpreter and the compiler of `una`.

### N. One binary (#891)

61. **Nothing else to install.** Itá is code of `una`: the manifest and lock parsers, the resolver, the archive reader, the verification of signatures and proofs (Ed25519 through `ring`, SHA-256 through `sha2`, both already dependencies), and the HTTPS client of `una-http`. Itaty's first root metadata is compiled in. The solver pin names the solver built into `una`.
    - **No external program.** Itá runs no other program, `git` included: there are no git sources (Question 7). Every source is a registry, a path or a workspace member.

### O. Edition `0.4` and migration

62. **The edition.** `ita.una` exists from edition `0.4` (rule 2). Every rule of this record applies only to packages of edition `0.4` and later; earlier editions, `una.toml` and `verification.lock` do not change.

63. **What migration maps.**

    | `una.toml` / `verification.lock` | `ita.una` / `ita.lock` |
    | --- | --- |
    | `[package] name = "events-example"` | `ita events_example` (`-` becomes `_`; a name that is still not a `lower_ident` needs `--name`) |
    | `version`, `edition`, `profile` | head lines; `edition` becomes `0.4` |
    | `[labels] categories` | `labels`, one per line |
    | `[trusted] modules` | `trusted` |
    | `[escape_budget] checked_per_module` | `escapes` / `checked per module` |
    | `[boundaries]` | `boundaries`, with `may` lines |
    | `[contracts]` | `contracts`, the clause strings as lines |
    | `[run] entry`, `grants` | `run`, `entry`, `grants` with one line per spec, limit suffixes as limit lines |
    | `[drive]` | `drive` |
    | `verification.lock` | the `solver` section of `ita.lock` |

64. **`una migrate [path]`.**
    - **Edition-gated.** It moves a package of edition `0.1` to `0.3` to edition `0.4`, and nothing else; a package that already has `ita.una` is a usage error.
    - It writes `ita.una` and `ita.lock`, then removes `una.toml` and `verification.lock`. Comments of `una.toml` that precede a key or table are carried above the line it becomes; others are listed in its output.
    - It then checks the package at tier 0 in edition `0.4` and prints every diagnostic that the change of edition causes, with its repairs, such as a function named `within` (U0101, ADR 0116). It applies none: `una fix` does that.
    - `--dry-run` prints the files and the diagnostics, and writes nothing. `--json` prints `una.migrate/1`.

    ```text
    $ una migrate
    wrote ita.una, ita.lock; removed una.toml, verification.lock
    name: events-example -> events_example
    edition 0.2 -> 0.4: 1 diagnostic
      src/events/limits.una:4:5 U0101 `within` is a reserved keyword; rename it
    ```

### Example

The issue's package, with the ceiling this record adds, its lock, and its report line:

```una
ita store
  version 0.1.0
  edition 0.4

needs
  http 1.4
    may Http[get]
  json 2.0

run
  entry store
  grants
    console
    files "data/" read
    http "https://api.example.com" get
```

```una
-- src/store.una
module store

import http.client (get_text)      -- the body, or "" when the request fails

pub main() -> Unit
  uses @console: Console[print]
  uses @net: Http[get]
=
  let body = get_text("https://api.example.com/items") with @http = @net[get]
  @console.print(body)
```

```text
dependencies:
  http 1.4.2 (itaty, signed itauna): may Http[get] (by store); uses Http[get]; trust none
  json 2.1.0 (itaty, signed itauna): may nothing (by store, http); uses nothing; trust none
```

## Edition `0.4`

As ADR 0121 rule 10 (draft) asks:

- **Words.** None in the language: no reserved or contextual keyword of a module is added. The words of `ita.una`, `ita.lock` and `itaty.una` belong to their own grammar (rule 5) and are not keywords anywhere else.
- **Clashes.**
  - **U1010** (rule 47) arises only in a package with `needs`, which edition `0.3` cannot write: no edition `0.3` program changes result.
  - Under the assumption of Question 8.1, **a `una.toml` naming edition `0.4`** is rejected with `una migrate` as the repair. **Landing order:** if this record's files land in the release that first accepts edition `0.4`, or before any release accepts `0.4` in `una.toml`, no released form changes, and the rejection is a plain U7002 with no ledger entry. Otherwise it is U7007, a change of a `0.4` form under ADR 0121 rule 5, with its ledger entry. Only edition `0.4` packages, which accepted the preview's terms, can see either.
  - **A U7007 inside a dependency**, after an upgrade of `una`, is reported at the dependency's line in the root, with the repair "update the dependency" (Question 8.2).
  - A package raised from `0.3` to `0.4` by `una migrate` meets the clashes of the other `0.4` records, which `una migrate` prints (rule 64).
- **Migration.** `una migrate` (rule 64). Its edits are not `una fix` repairs, since they rewrite project files, which `una fix` never touches.
- **Unsettled points:** none. The formats carry their own versions (rules 16, 29, 37), so a later format is read beside the old one.
- **Trust.**
  - **Rule 2 (a):** the registry and the packages a root names are trust the package adds itself, and its `needs` section and `sources` section are the opt-in; no dependency can add a source (rule 42). A dependency's trusted modules need a second, named opt-in, the root's `trust` lines, bound to an identity and pinned by digest in the lock (rules 10 and 53). The report names each (rule 58).
  - **Rule 2 (b):** the toolchain grows for every package of edition `0.4` by the parser of the three files. It grows for packages with dependencies by the archive reader, the resolver, the verification of signatures, roots, checkpoints and proofs, Itaty's built-in root metadata, and the use of `una-http` by `una ita`. Each needs the security review of ADR 0121 rule 2 (b) and a listing in the ledger's `trust` field. No `out_of_model` entry is added.
- **Security.** The information-flow rules of this record are in "Information flow" below.

## Security

**What a dependency can do, and what stops it.**

| Threat | Control |
| --- | --- |
| Code that runs at install or build | None exists: no scripts, hooks, build programs or macros (rule 3). |
| A dependency that reaches the network, files or keys | It holds no capability unless a call grants it with `with`, within the ceiling the *calling* package writes (rule 51, U3020, U3021); a dependency never sets the ceiling of the root's grants. Only the root is granted host capabilities (rule 52). |
| An agent that widens a ceiling to make a check pass | The lock records the consented ceilings; a wider `may` line is U7009 until `--accept-authority` records it, visible in the lock's diff (rule 17). |
| A dependency that declassifies the program's data | Its policies are in trusted modules, which need a trust line naming its identity and module (rules 10, 53, U7012); only that module is exempt from U3024 (rule 53.1), never its siblings. |
| A dependency that brings foreign code | None is admitted: a package holds only Una sources, and `ita.una` has no foreign section or grant (rules 5, 27, 60). |
| Trust consented for one package used by another | Trust lines bind source, name and major, and the lock pins the content digest of each package whose trust was consented; any change is a trust change (rules 10, 25). |
| Proofs the publisher fakes | None is read: the root proves everything (rule 48). |
| Typosquatting, including names an agent invents | Unscoped names are Itaúna's; owner names are unique by skeleton and reviewed near popular owners (rules 44, 45); reserved names are refused as local names too (rule 7). A wrong package has no capability or trust without lines the programmer writes, and `una ita add` prints the source, owner, age, dependents and authority of what it adds. |
| Dependency confusion | One source per need, no fallback; owners bound to the source that serves them, with `from` required (rule 42); a local package with a registered name only with `accept name`; workspace members only by `from workspace` (rule 19). |
| A dependency that adds a source or a local path | Only the root and `itaty.una` name sources and paths; published packages need only their own registry (rules 8, 42, 43). |
| A compromised account | Publishing needs the owner's keys, not the account (rule 38); a recovery is pending seven days of witness time, checked by each client, and any current key cancels it; Itaúna has no registry recovery (rule 38.1). |
| A stolen publisher key | Every release is public in the log and in the name's chain (rules 36, 36.1); MVS never takes a release no need asks for (rule 23); every selection, the raises past yanks included, waits out the cooldown in witness time (rules 24, 25, 36.4); authority growth in a minor release is refused by the registry and by clients (rule 26); a revocation can mark releases since the compromise (rule 38). |
| A malicious update of an honest package | Same as above; `una ita update` prints the source, authority and trust change and refuses a ceiling, trust or key change without an explicit flag (rule 25). |
| A stolen security key | One key can hold a release back with an advisory, never hurry one in: lifting the cooldown needs two security keys, or one and the owner's threshold, and names the fix by digest (rules 24, 41). |
| A compromised registry or mirror | It cannot forge an owner's signature; it cannot rewrite or fork the log, or hide an entry of a name, without the witnesses, who recompute the name map (rules 36, 36.1); it cannot change a locked package (digest and stored provenance, rule 17). It can refuse service, which vendoring and bundles survive (rules 32, 33). |
| A freeze or rollback attack | Witness co-signatures less than 24 hours old for resolution and audit; checkpoints never move backwards, and the lock's resolution point is a floor (rule 39). |
| Censorship of a yank or key event | A signed promise to log within an hour, which an owner can publish if broken (rule 36.2). |
| A root key compromise | Three of five offline keys are needed; root rotation without a release of `una` (rule 35). |
| Denial of service through the graph | Bounds on releases visited, needs, documents, bytes and time; no self-need (rule 23). |
| A malformed archive (path traversal, links, case collisions, device names, bombs) | The closed layout and the reader of rules 27 and 29: no links, no modes, no compression, sorted listing, exact lengths, read once; descriptors with `O_NOFOLLOW` (rule 30). The parsers get fuzz targets (Implementation plan). |
| A store or vendor directory tampered with | Every load re-hashes, and registry packages are re-verified against their stored statements and proofs (rule 17, U7010). |
| A poisoned proof cache | Entries are authenticated with the user's own key (ADR 0064), under a domain-tagged key (rule 48). |

**What Itá cannot do.**
- **Pure code can be wrong.** A dependency with no capability can still compute a wrong value, loop, or exhaust memory. Its contracts, proven by the root, are what a programmer relies on; termination and resource consumption stay outside the model (`spec/07 §1`).
- **A trusted dependency is trusted.** A trust line gives that module the standing of the root's own trusted code.
- **A colluding registry and threshold of witnesses** could show one client a forked log or a map that hides an entry. Monitors outside both see it.
- **Withholding.** A registry can delay logging only by breaking a promise it signed (rule 36.2); it can refuse service entirely, and serve a client a checkpoint up to the freshness window old.
- **The registry learns what is fetched.** Fetches reveal a program's dependencies to the registry or mirror, though no program data. Vendoring, bundles and an internal mirror avoid it.
- **The HTTP client** runs with the user's privileges; a vulnerability in it is code execution outside these rules, though not a loss of integrity. Itá runs no external program (rule 61).

**Trusted computing base.** See "Edition `0.4`", Trust. Before the first implementation step, `docs/security/tcb.md` gains a component "Package integrity and the registry client", with this table, and its gap 9 ("no provenance for archives") gains the release statements as the pattern a later record can reuse for `una`'s own releases.

## Verification

- **No new obligation kind.** The obligations of a dependency are its own, generated and solved as for any module, under the root's solver pin.
- **A dependency proven by its publisher under another solver version** may leave obligations unknown under the root's pin: they are reported as any unknown obligation (U5002) at the dependency's file. The release statement does not carry the publisher's solver, since the root never uses its results.
- **The cache key** of `spec/12 §9` becomes a domain-tagged, length-prefixed hash of the solver section and the query, so obligations shared by programs are proven once per machine (rule 48).
- **Check time** is bounded by the obligations of the program's packages, each under the lock's step budget; there is no other limit.
- **Determinism** (`spec/12 §10`): for identical sources, `ita.una`, `ita.lock` and locked package contents (by digest), tiers 0 and 1 give identical diagnostics, statuses and reports, except `unknown: wallclock`.

## Information flow

- **No new flow rule.** Every flow of the program is checked as one program (rules 48, 55), under each package's edition.
- **Categories** are a union by name (rule 54).
- **Whole-program labels:** the file-state label (ADR 0085) and `conc` (ADR 0116) join over every package, so a dependency's reads are labeled by the root's writes and the reverse.
- **Boundaries:** a dependency receives authority only by `with`, within its caller's ceiling, and reaches no release except through a policy module the root trusts by name (rules 51, 53.1).
- **Project files and the report** carry names, versions, digests, keys, capabilities and module paths, all public facts of the sources. Resolution reads no program data, and `una ita` sends a registry only names, versions and digests.

## Costs

- **Parsing `ita.una`, `ita.lock`, `itaty.una`:** O(n) in their size, one pass of the lexer.
- **Loading a program:** O(B) SHA-256 over the B bytes of the packages' files, which the checker reads anyway; about 1 GB/s on one core, done per package in parallel.
- **The consistency check of a lock:** O(P + R) for P packages and R needs, with hash maps.
- **MVS:** O(N + R) time and space (rule 23), and O(N) concurrent fetches of statements, at most 16 at once per source. Scaling test: random graphs of 10^3, 10^4 and 10^5 releases with five needs each; time per edge stays within a constant factor across sizes, and the test fails on growth faster than linear.
- **The archive reader and the digest:** O(size), with memory O(listing + largest file). Scaling test: 1 MiB, 16 MiB and 256 MiB packages.
- **Log verification:** an inclusion proof costs O(log L) hashes for a log of L entries; a consistency proof O(log L); a signature O(1), about 50 µs. Scaling test: proofs on logs of 2^10 to 2^30 leaves, checking that the work grows by one hash per doubling.
- **A name's state** (rule 36.1): O(log M) hashes for the map's proof over M names, plus O(E) for the E entries of the name not seen before, then O(1); a witness's map update is linear in the new entries. Scaling test: maps of 2^10 to 2^24 names, the proof's work growing by one hash per doubling.
- **Offline provenance on load** (rule 17): one signature and O(log L) hashes per registry package.
- **Name checks at the registry:** the skeleton, one hash lookup; distance 1 against the top owners, O(L) lookups in a deletion index (rule 45).
- **Effective ceilings:** a least fixpoint by worklist, O((P + R) · C) for C capability operations, since each set only grows and has at most C elements; cycles included (rule 51).
- **The consent check of the lock:** O(P + R) set comparisons (rule 17).
- **Checking the program:** linear in the program's size, as for one package today; the dependencies' obligations are solved once per machine and pin (rule 48).
- **The report's `dependencies`:** O(P log P) to sort.

## Diagnostics

New codes, which join `spec/13 §4` with their implementation. ADR 0121 (draft) takes U7007. U1010 and U7008 to U7014 are free on main, in the open pull requests and in the local worktrees, at this writing.

| Code | Domain | Meaning | Repairs, ranked |
| --- | --- | --- | --- |
| U1010 | names | A module of a package whose first path segment is a dependency's local name | give the need another local name with `as` |
| U7008 | project | Invalid `ita.una` or `itaty.una`: syntax, unknown or repeated line, wrong value, edition before `0.4`, both `ita.una` and `una.toml`, a reserved local name, a need of an owner served by a source without its `from` | per line, as U7002 |
| U7009 | project | Missing, invalid or inconsistent `ita.lock`, or a ceiling wider than the one it records | run `una ita lock`; for a wider ceiling, `una ita lock --accept-authority` (an authority change, never applied by `una fix`) |
| U7010 | project | A package, archive, statement, root, checkpoint, name chain or map, or log proof that fails verification; a statement against its archive; a breach of rule 26 in the log; an expired root; clock skew | none mechanical: the text names what failed and where it came from |
| U7011 | project | A locked package that is neither in the vendor directory nor in Itaoca | run `una ita fetch` |
| U7012 | project | A dependency the root may not use as written: trust that no recorded consent covers, an edition later than the dependent's, a local package with a registered name without `accept name`, a reserved name of a transitive package | add the trust line and accept it (an authority change, never applied by `una fix`); use an earlier version |
| U7013 | project | No resolution: no release meets a need, a raise held back by the cooldown, a source or name that does not exist, a need on its own identity, an origin rule 42 forbids, a bound of rule 23 exceeded, or a publication that rule 26 or 43 refuses | name the release that would do; `--now` for the cooldown |
| U7014 | project | A locked release that is yanked, under an advisory, or signed after a stated compromise; a pending recovery, a key change or a transfer of its owner (`una ita audit`) | run `una ita update <name>` |

Existing codes cover the rest:
- **U1001:** an import of a package that is not a direct dependency.
- **U3020, U3021, U3024:** calls into dependencies (ADR 0093).
- **U4007:** a category the module's own package does not declare.
- **U5003:** a solver other than the pin.
- **U7001:** non-canonical `ita.una`.
- **U7002 or U7007:** `una.toml` in edition `0.4`, by the landing order of the section "Edition `0.4`".

## Consequences

- **Packages can depend on packages**, from edition `0.4`, with no change to earlier editions or to single files.
- **A dependency is safe to add in a precise sense:** it runs nothing, receives nothing it is not given at a call within a written ceiling, releases nothing unless the root names it, carries no foreign code, and its code is proven by the root. What remains is the correctness of pure code, which its proven contracts address.
- **Calls into a capability-using dependency are written with `with`,** and a wider ceiling is accepted in the lock. This is the price of Question 2.
- **The first check after an update is slower**, then cached per machine (Question 3).
- **Labels are whole-program**, so a dependency can fail to check in one program and not in another (rule 55).
- **Library authors** keep authority growth for major versions (Question 4), and publish with keys they hold.
- **The project** operates Itaty, its log, its name map and its root keys, recruits at least five witnesses, which run Itá-specific witness software since they replay entries and check chain links and the name map (rule 36.1), and holds security keys for advisories; Itaúna packages go through the release gates, and Itaúna's keys change only by root versions.
- **Every selection waits 72 hours of witness time,** a raise past a yanked release included, unless a threshold-signed advisory names the fix.
- **TOML remains** for editions `0.1` to `0.3` and for the `Keys` key file (`spec/14 §3`), which is a secret outside the package; moving the key file is a separate decision.
- **Edition `0.4` packages written against `una.toml`** move with `una migrate` (Question 8.1).
- **The `toml` crate** stays a dependency of `una` for as long as editions `0.1` to `0.3` are served.

## Alternatives considered

- **A. Keep TOML.** Not chosen (#893): a second syntax for agents to learn, a TOML parser in the base for every package, and ceilings and contracts written as strings that a second parser reads. In Una syntax, `may` and `contracts` lines are parsed by the production a header uses.
- **B. A manifest or build step that runs code** (build scripts, install hooks, plugins). Not chosen: the commonest way packages attack their users, and against #893.
- **C. Range-based solving with upper bounds** (a SAT or PubGrub solver). Not chosen: NP-complete in general, reproducible only through the lock, and it selects the newest matching release, so a malicious release reaches every fresh resolution at once.
- **D. Newest compatible by default,** without a solver. Not chosen, for the last reason of C.
- **E. Unscoped names for everyone.** Not chosen (Question 1).
- **F. Names as URLs** (a domain and path per package). Not chosen: identity would follow domain ownership, which expires and changes hands, and a host outage would break resolution; mirrors would need rewriting.
- **G. Signatures without a log** (keys and metadata only). Not chosen: the registry could show different releases to different users, and a stolen key could sign in private.
- **G2. Completeness by scanning the log** (each client reads every new tile since its stored checkpoint). Not chosen: O(new entries) per resolution, growing with the whole registry's activity rather than with the program, for every client. The name map of rule 36.1 costs O(log M + E) per name.
- **G3. An unverified per-name index.** Not chosen: a mirror or network attacker could hide yanks, advisories, revocations and recoveries from it (rule 36.1).
- **H. Keyless signing only** (Question 5, option b). Not chosen as the only form.
- **I. Trusting a publisher's proofs.** Not chosen: a proof that is not checked is a claim. Certificates (issue #66) may later let a root check a proof instead of redoing it.
- **J. Inheritance by name for chosen dependencies** (ADR 0093's `[inherit]`). Not chosen (Question 2): it is the B10 to B12 failure of ADR 0092 across packages.
- **K. Install scripts in a sandbox.** Not chosen: a sandbox is a large addition to the base, and the scripts are not needed.
- **L. Compressed archives.** Not chosen: a decompressor in the base and the bomb it allows, for sources that are small text.
- **M. Reading a dependency's lock.** Not chosen: the root's resolution would depend on files it does not control; MVS needs only `ita.una`.
- **N. Git sources in phase 1.** Rejected (Question 7): fetching would run the system `git` unconfined against a possibly hostile server, and add a tool besides `una`. Path dependencies cover local development, and a private Itá source covers organizations.
- **O. Registry recovery for every owner, Itaúna included.** Not chosen: whoever controls the registry or an account's email could take the most-depended-on names (rule 38.1).
- **P. The union of dependents' ceilings at every call.** Not chosen: a dependency's own `ita.una` would set the ceiling of the root's grants (rule 51).

## Implementation plan

0. **The risk profile.** `docs/security/tcb.md` gains the component and gaps of Security, before the first implementation step.
1. **Phase 1, files.** The lexer's version token and the grammar of `ita.una`, `ita.lock` and `itaty.una` in the syntax crate, with fuzz targets; U7008, U7009; `una fmt` for `ita.una`; the reading of every section that `una.toml` has, and a test that every check of `una.toml` has its `ita.una` twin; `una migrate`. The crate `una-project` reads both formats behind one `Manifest`.
2. **Phase 1, packages in the checker.** Path and workspace sources and the origin rules of rule 42; module identity with packages; U1010 and reserved local names; whole-program checking with each package's edition and profile; boundaries per dependency with per-caller ceilings and the effective-ceiling fixpoint; trusted modules of dependencies as extents of their own (rule 53.1), with the U7002 exception of `spec/13 §7`; trust lines by identity, consents in the lock, and U7012; dependencies' examples in their own context; labels; the whole-program analyses of rule 55; the report's `dependencies` and `tcb` keys; `una.core/2`; the shared proof cache and its tagged key.
3. **Phase 1, content.** The listing, digest and archive (rules 27 to 29); Itaoca with verified loads through descriptors; vendoring with stored provenance; U7010, U7011.
4. **Phase 1, resolution.** MVS with its bounds and scaling test; the consent check and `--accept-…` flags; `una ita add`, `remove`, `lock` (and `--check`), `update` (offline sources), `fetch`, `vendor`, `tree`, `verify`.
5. **Phase 2, the registry client.** Root metadata, its chain and expiry; tiles, co-signed checkpoints kept by size, witness time, freshness and floors; the name map and per-name chains; inclusion and consistency proofs; release statements and their check against archives, rule 26 from statements; strict Ed25519; keys, pending recoveries and cancellations; publishing and its promises; yanks, advisories with their thresholds, the cooldown in witness time, `audit`, `watch`, mirrors, private sources with `owners`, bundles. Each parser of a signed or fetched document has a fuzz target.
6. **Phase 2, the service.** Itaty itself, in a repository of its own: static storage, the log and the name map, the name rules of rules 44 to 46, rule 26, the logging promises, notifications; at least five witnesses that recompute the map; the offline root ceremony and the security keys. Its own security review.
7. **Security.** The review of ADR 0121 rule 2 (b) for each step; a hunt family with programs of several packages (`docs/plan/security-hunt.md`); the `deps` gate for any new crate.
8. **Documentation.** The new chapter of the specification, `spec/30-packages.md`; `spec/13 §4` to `§8`; the guide's pages on packages, dependencies, publishing and offline builds; "Migrating to edition 0.4" in `docs/guide/una-for-agents.md`; the Learn chapter on editions.

## Conformance

- **`ita.una`:** each section and line in canonical form; each U7008 (unknown line, repeated section, version `1.4` where a full version is required, interpolation in a text, edition `0.3`, both manifests, a `may` capability that does not resolve, a member's `path` leaving the workspace, `as std`, `as s7d`, a need of `acme/x` without `from acme` when a source lists `acme`, two sources listing one owner, a `from git` need, a `foreign` section, a `foreign` grant); `una fmt --check` on a misordered file (U7001).
- **`ita.lock`:** a lock that parses; U7009 for a missing lock, a need not met, an unreachable package, an identity locked twice, and a `may` line wider than the recorded ceiling, accepted after `--accept-authority`; U7010 for a changed byte in a stored package, for a vendored package and its digest edited together (its stored statement fails), and for a lock whose registry root differs from the `sources` pin; U7011 for an absent one; `una ita lock` on a package with no lock writes the default solver section; `una ita lock --check` offline, at the lock's resolution point.
- **Digest and archive:** the same files give the same digest whatever their order of creation, timestamps and modes; a file renamed changes it; U7010 for trailing bytes, a length past the end or with a leading zero, a listing out of order, a link, a `..`, a hidden entry, a segment ending in `.`, `CON` or `nul.txt`, two paths equal ignoring case, and `a` beside `a/b`.
- **MVS:** the classic examples (a diamond, a cycle, two majors of one package side by side), an `avoid` and a yank raising a selection, a `replace`, and a lock unchanged by a later release; a raise held back by the cooldown (U7013) and taken with `--now`; a raise that skips a pre-release; a threshold advisory naming its fix by digest lifting the cooldown, and a single-key advisory not lifting it; a need on its own identity (U7013); each bound of rule 23; the scaling test.
- **Origins:** a registry package with a `sources` section or a path need, refused at publish and, when crafted, U7013 at resolution; a path package's own path need (U7013); a transitive need without `from` resolved at its package's own source; a path package named `json` (U7012), accepted with `accept name` in the root; a lock with a `git` origin, U7009; no command of `una ita` starts an external program.
- **Modules:** `import http.client (get)` resolves; an import of a transitive dependency is U1001; a root module `http` beside the need `http` is U1010; two majors of `json` give two kinds, and mixing them is a type error.
- **Authority:** a call into a dependency without `with` (U3020); a grant beyond the caller's `may` (U3021); a function value of a transitive dependency handed back to the root, with the empty ceiling even though the direct dependency's `may` for it is wide; a dependency's handler installed by the root; the effective ceiling over a cycle of two packages; a dependency's example whose `given` exceeds the root's ceiling, accepted; the report's `granted_by`, `effective_ceiling` and `authority`.
- **Trust:** a dependency with a trusted module, rejected without the trust line (U7012) and accepted with it; a release in that trusted module, reached from the root, accepted; a release in a sibling module of the same package, still U3024 (and U7012 for the unconsented module); a sibling's call into the trusted module without `with` (U3020); a trust line that does not cover the same name from another source or another major; a new transitive package matching an existing trust line by name, refused until `--accept-trust`; a trusted package of which only a sibling module changes (a `via` whose helper in `privacy.util` changes), refused by `una ita update` without `--accept-trust`; a call from the trusted module into a sibling without `with`, and beyond the trust line's `may` (U3020, U3021); an archive with a file outside rule 27's list, such as a `.c` or `.a` file, U7010.
- **Labels:** a dependency's read whose failure is raised by a root's `Files.write` under a secret program counter, rejected only in the program that writes it.
- **Editions:** a dependency of a later edition than its dependent (U7012); `una.toml` with edition `0.4` (U7002 or U7007 by the landing order) and its `una migrate`; a U7007 inside a dependency, reported at the root's line with "update the dependency".
- **Offline:** `una check`, `una build` and `una run` with the network disabled succeed on a fetched lock, and fail with U7011, never a network attempt, when a package is absent.
- **Registry,** against a test registry with its own root: a valid release; each failure of rule 39 (a bad signature, a non-canonical `S`, a small-order key, a key revoked before the release, a missing co-signature, a stale checkpoint for `update`, a checkpoint below the lock's resolution point, a fork between two checkpoints, an inclusion proof for another leaf, a statement whose authority understates the archive's, a second statement for one version, a statement that breaks rule 26); a map or chain that omits a yank, an advisory, a revocation, a recovery or a transfer, and a chain whose `previous` skips an entry, caught by the count (each U7010); one back-dating witness and one forward-dating witness, each with no effect on the cooldown, the recovery delay or freshness; a witness whose timestamps decrease (U7010); offline, a vendored package with its digest, key and checkpoint edited together, rejected for its co-signatures and its owner chain; a root rotation, an expired last root, and clock skew; a recovery cancelled by a current key, one not yet effective after six days of witness time, one effective after seven, and none for Itaúna; a revocation "compromised since" flagging later releases; a mirror serving a bad proof, skipped.
- **Publishing:** refusal without `publish to`, with an unproven obligation, with a need of another source or a `path` need, and with authority growth in a minor version (U7013), by the client and by the registry.
- **`una migrate`:** each row of rule 63; a name with `-`; comments carried; the clash list of a package raised from `0.3`.

## Appendix A. Amendments to accepted records, appended on acceptance

**ADR 0021, Amendment (2026-10-19): edition `0.4` (ADR 0122).** A package of edition `0.4` or later is described by `ita.una` and pinned by `ita.lock`, as ADR 0122 states; this record's rules stay those of editions `0.1` to `0.3`.

**ADR 0085, Amendment (2026-10-19): dependencies (ADR 0122).** Decision 5's condition holds: the call graph is that of the whole program, every package of the root's lock included, known when the label is computed (ADR 0122 rule 55). `pub` functions of any package keep the label their callers give them.

**ADR 0093, Amendment (2026-10-19): dependencies (ADR 0122).**
- **Rule 2.** Every module of a dependency is in a boundary, one extent per package. The ceiling at a call into it is the one the calling package writes with `may` lines in its `ita.una`, empty by default, and empty for a package the caller does not need. The table `[inherit]` is not created: no dependency receives authority by name (ADR 0122 rule 51, Question 2).
- **Rule 8 and the U7002 of `spec/13 §7`.** A module of a dependency that the root names in a trust line is not a boundary module of its package's extent: it is an extent of its own, called into across a boundary, and a release whose policy it declares is not U3024. Every other release reachable from a call into the package stays U3024, and every other module of a dependency stays never trusted (ADR 0122 rule 53.1). `spec/13 §7`'s "a module of `[trusted] modules` inside a boundary" is U7002 except for such a module.

**ADR 0109, Amendment (2026-10-19): profiles across dependencies (ADR 0122).** Each package is checked under its own profile (ADR 0122 rule 50). A gated construct that adds no trust needs nothing of the root; a dependency's trusted modules need the root's trust line whatever the profiles (ADR 0122 rule 53).

**ADR 0116, Amendment (2026-10-19).** The `[drive]` table's form in `ita.una` is the `drive` section of ADR 0122 rule 11, with the same keys, ranges and errors, reported as U7008.

**ADR 0117, Amendment (2026-10-19).** "The code of #893" for invalid limits in `ita.una` is U7008, and the limit lines sit under an `http` grant line (ADR 0122 rule 12).

## Appendix B. Notes for draft records

**ADR 0121, Note, appended on its acceptance, coordinated with Question 8.1.** In edition `0.4`, the edition switch of rule 2 and its opt-ins are read from `ita.una` (ADR 0122), where rule 2 says `una.toml`. The `0.3`-to-`0.4` pair of the edition test (rule 11) runs `una migrate` on each `0.3` package of the suite, then compares; a difference that the migration alone causes is not a clash. `una.toml` naming edition `0.4` is U7002 if no released `una` accepted that form when ADR 0122's files land, and a U7007 ledger entry otherwise.

## Amendment of 2026-10-25: the yank command

Rule 41 states that an owner yanks a release by a logged statement, and rule 57 lists the commands of `una ita`. This amendment names the command that writes that statement.

- **`una ita yank <version> [path]`** writes the yank statement of the release `<version>` of the package at `path`: a statement in the chain of the package's name (rule 36.1), whose `previous` is the head of that chain, of the form `spec/30 §11` gives, which the log then holds as a leaf (rule 41).
- **Who signs it.** The owner's keys, at least the owner's threshold of keys valid at the head of its chain (rule 38). A yank signed by fewer is not a yank: the registry does not log it, and no client reads it as one.
- **Where it goes.** To the source that the head's `publish to` names, as a submission, through the same transport as `una ita publish` (rule 43), with the registry's signed promise to log it (rule 36.2).
- **What it changes.** As rule 41 states: new resolutions skip the release, existing locks keep it, and nothing is deleted. A version that was never released, or is already yanked, is refused.

## Amendment of 2026-10-25: co-signer changes wait

- Status: Accepted, recorded under the project's delegation.

**Context.** Rule 38.1 gives an owner whose keys are lost or stolen two fallbacks: recovery co-signers designated in advance, and, for an owner with none, a registry dispute. A co-signer designation logged after a key was stolen defeats both, since rule 38 lets the owner's threshold change the co-signers at once. The minimal chain:
- Entry 0, `ita-owner/1`: the owner registers one key `A`, threshold one, no co-signers. A thief copies `A`.
- Entry 1, `ita-key/1`, signed by `A`: `cosigners 1 X <key>`, naming a co-signer key `X` that the thief holds. `A` is a current key, so the registry logs it.

From then on a recovery needs `X`'s signature, which only the thief gives, and a dispute is refused, since the owner has co-signers. The owner cannot revoke `A`, its only key. It can log `cosigners none` with its own copy of `A`, but the thief logs a new designation after each one. Every release the thief signs with `A` counts, for good.

**Decision.**
1. **Co-signer changes wait.** Every change to the recovery co-signers, a designation, a replacement or a removal (`cosigners <k> …` or `cosigners none`), is a pending key event. It takes effect only on a checkpoint whose witness time is at least seven days after the time it was logged, measured as a recovery's (rules 36.4 and 38.1), with no cancellation between, and governs the entries logged after that checkpoint.
2. **Cancellation.** Any one current key of the owner can cancel it while it is pending, by `cancel <index>`. Such a cancellation cannot be overruled.
3. **The co-signers in effect.** While a change is pending, the co-signers in effect are the old ones, for every rule that names them: recoveries, revocations, overrules and disputes. With none in effect, a dispute is available.
4. **Recoveries and disputes end waiting changes.** When a recovery or a dispute takes effect, every co-signer change logged before that and still pending never takes effect, since keys the recovery or the dispute replaces signed it. A recovery leaves in effect the co-signers that were in effect when it was logged; a dispute leaves none.
5. **Overrules.** A cancellation of a recovery is overruled by the threshold of the co-signers that were in effect when the recovery was logged, so a co-signer change that takes effect while the recovery waits cannot take the overrule away from them. An overrule counts whenever it is logged before the recovery took effect, after the recovery's seven days included: the recovery takes effect at the first checkpoint that is at least seven days of witness time after it was logged and after its last cancellation was overruled, and governs the entries logged after the overrule when its seven days had passed by then. Once the co-signers overruled a cancellation of a recovery, later cancellations of it by owner keys do not count: their decision binds. Without this, a thief holding the only owner key cancels each recovery just before its seven days end, the effect checkpoint follows, and no overrule fits before it, for good.
5a. **The co-signers' claim.** The threshold of the co-signers in effect may log a compromise claim, `revoke <key> since <p>`, on the owner's last key, as a claim only: the keys held do not change, and what that key signed after `p` no longer counts. This is rule 38.1's fallback of co-signers whose threshold can revoke a key, for an owner with a single key.
6. **Reporting.** `una ita lock` and `una ita update`, and `una ita audit` once it reads registries, report a pending co-signer change as U7014, as a pending recovery: it is consented to with `--accept-key`, and its taking effect is a key change of its own (rule 25).
7. **Competing requests.**
   - **Two attacker classes.** An attacker holding only a stolen key of the owner (key-only) can log whatever that key's signature allows, but cannot obtain a recovery or a dispute: the registry logs those only after it verifies the account, by rule 38's registration checks and rule 38.1. An attacker who also took the account over (key and account) can obtain them. Competing requests arise only with an account takeover.
   - **The rule.** A recovery or a dispute logged while a dispute of the same chain waits competes, and so does a dispute logged while a recovery waits that was logged with co-signers in effect. A request waits from its log position until it takes effect; such a recovery waits even when cancelled, since its co-signers may overrule the cancellation. A dispute logged while a recovery logged with no co-signers in effect waits does not compete: both take effect in turn, the later one last. So whether requests compete never depends on a cancellation, which a later claim of compromise can void. None of the competing requests takes effect, and the owner's chain is frozen from the log position of the later one.
   - **A frozen chain stays frozen.** No co-signers can settle it: a dispute is logged only for an owner with no co-signers in effect, so the co-signers in effect at the first dispute's position are always none, and an overrule by them cannot exist. No later entry changes the owner's keys, its co-signers or its requests, and a dispute or a recovery logged after the freeze changes nothing. A revocation with `since <p>` logged after the freeze, signed as rule 38 requires by the keys in effect at the freeze, still states a key's compromise.
   - **Resolution.** A name of a frozen owner is refused (U7010), and the refusal gives the freeze's position. The name's owner registers anew, under a new owner name.
   - **Releases.** A release logged before the freeze stands, unless it is signed by a key compromised before its position. A release logged from the freeze on never counts, whoever signs it.
   - **Why it is safe.** Nothing in the log tells the owner's request from the attacker's, so a rule that picked a winner would give the name to the attacker in some chain. A freeze gives it to no one: no request takes effect, so the attacker never ends up with the name. The freeze is the outcome only with an account takeover, and the owner's guarantee that a stolen key cannot keep it from its names is stated for the key-only attacker.
7a. **`--fresh` for `una ita lock --check`.** A frozen chain fails every package of its owner: `lock` and `update` refuse its names, and `lock --check`, which needs no fresh checkpoint, fails on a locked package whose owner's chain is frozen in the newest checkpoint a place serves, and says in a note how far it looked, the witness time of that checkpoint in UTC or that a later freeze was not checked. `una ita lock --check --fresh`, a new flag, requires a checkpoint less than 24 hours old by the witnesses' time, as a resolution's, read from a place or a mirror and extending the lock's resolution point, and fails without one (U7013) and on a frozen owner (U7010). CI runs it wherever a place or a mirror is reachable, since `una ita audit` is opt-in.
7b. **The root a checkpoint names.** Every checkpoint carries, inside its signed text, one extension line `ita-root <n> sha256:<hex>`: the version and digest of the root metadata it was signed under. A place that hides root version n+1 from a client that has not walked to it would otherwise make that client verify the latest checkpoint, and decide every name, under the older key set, which a rotation may have retired because it was compromised. With the line, a checkpoint names a root past the chain the place serves, and the client treats the result as unavailable (U7013) rather than trusting the older keys; a digest other than the walked version's is refused (U7010). Removing or altering the line invalidates the log's signature and every cosignature.
7c. **Witness rotations.** A checkpoint's cosignatures are counted with a threshold that is the larger of the root version it names and the last root's, so it never drops below the current one. A witness of the last root counts. A retired witness, one the last root no longer lists, counts only on a checkpoint that names a root version it belonged to, that a consistency proof shows to be a prefix of a checkpoint signed under the last root, and when no root lists its key in a new optional root metadata field, `witnesses-revoked <fingerprint> …`, whose keys count for nothing at any version, under any name. A rotation that retires a key because it is or may be compromised MUST list it there. When the prefix proof cannot be completed because a place withholds what it needs, every result that would count a retired witness is unavailable (U7013), never decided on fewer cosignatures. Every copy of one size's checkpoint names the same root version; copies that differ are two checkpoints of that size, refused (U7010). So a rotation that leaves fewer than the threshold of the old witnesses keeps the older history usable, the log binding it to the current checkpoint, while retired witnesses cannot vouch for a branch off the log, and a compromised witness is revoked outright.
7d. **The deadline.** This refines rule 38.1's "first kept checkpoint whose witness time is at least seven days after" the request, whose text stands. A registry cosigns every checkpoint whose size is a multiple of 64 by the witness threshold, and the deadline D is found from the first such multiple at or after the request whose witness time reaches the target, as the first size in the 64 sizes before it, after the previous multiple and after the request, whose witness time reaches the target, or that multiple itself if none does. A checkpoint's witness time, the k-th latest of its cosignatures, need not grow with the size when different witnesses cosign late, so the first such checkpoint over every size could differ between a client and a registry; this definition makes them agree, and a client reads O(W/64 + 64) checkpoints for W sizes to find it. A registry MUST NOT log anything that relies on the request before D. It MUST publish or serve on D only cosignature lines, by any witness, dated at or after the target, MUST NOT let any checkpoint after the request and before D reach the target, and MUST NOT publish or serve a cosignature line dated before the target on any checkpoint from D on, so that a place stripping cosignatures from D cannot lower its witness time below the target unseen with lines the registry published. A registry may therefore fix D by the cosignatures as they arrive rather than by when it signs D, and need not refuse growth while it waits for them; it then publishes no line dated before the target on a checkpoint that may still become D, holding such a line back rather than publishing it as it arrives. The guarantee covers the lines a registry publishes or serves: a witness that published its own early line on D outside the registry is outside it.
8. **The bound.** The bound on the key-and-account attacker rests on the registry's account verification, rule 38's registration and rule 38.1's disputes. A compromised registry is outside this model: the log's transparency, the witnesses and `una ita audit` cover it.

**Consequences.**
- A co-signer change takes seven days, as a recovery does; an owner designates its co-signers a week before it may need them.
- **The owner response window.** The guarantees against a stolen key hold for an owner that responds within seven days of the attacker's first pending change; where the attacker makes no pending change, there is no deadline. An owner that does not respond in time can lose its names to a thief holding only a key, and an owner that never acts cannot be protected against a valid key. With an account taken over too, a thief's dispute that the owner does not answer takes effect, and an answer forces at worst a freeze. So a registry notifies the owner's verified contact as soon as any pending key event, co-signer change, recovery or dispute is logged against the owner's chain, and clients report each such pending event as U7014 to everyone who locks, updates or audits a package of that owner.
- A single-key owner whose key is stolen keeps the dispute: the thief's designation waits, the dispute is logged meanwhile, and when it takes effect it ends the designation.
- `spec/30 §14` rule 5 states these rules as normative text.

## Amendment 9 (2026-10-25): stated thresholds as known at logging

- Status: Accepted, recorded under the project's delegation.

**Context.** Every `ita-key/1` entry states a `threshold`: the current one for a cancellation, an overruling, a co-signer change and a claim on a former or last key, and one within the keys after the event for an addition or a revocation. A mismatch refuses the chain. A claim of compromise, `revoke <key> since <p>`, is retroactive: it can void entries logged before it, and so change the state at every later position. The rules did not say against which state an entry's stated threshold is checked. Checked against the state a later claim rewrote, an honest entry can become a mismatch, which refuses the whole chain:
- Entry 0: the owner registers keys `A` and `B`, threshold two, and the co-signer `X` at threshold one. A thief copies `A`.
- Entry 1: the owner, below its threshold without `A`, asks a recovery naming `R`, signed by the log's key and `X`.
- Entry 2, six and a half days later: the thief cancels the recovery with `A`.
- Entry 3, a day later, after the recovery's seven days: `X` overrules the cancellation, stating `threshold 2`, the owner's threshold then.
- Entry 4, a week later: `R` claims `revoke A since 0`.

The claim voids the thief's cancellation, so the recovery takes effect at its deadline, the overruling's own position, with threshold one. Checked against that state, the overruling's `threshold 2` is a mismatch, and the chain is refused: the registry must refuse the owner's claim, and the owner loses its names to the thief. A claim from a later position instead leaves the thief's releases counting. The replay also refused a chain for a mismatch found in a pass that assumes every claim, even one that the fixpoint later drops.

**Decision.**
1. **Stated thresholds, as known at logging.** An entry's stated `threshold`, the current one or one within the keys after the event, is checked against the state known when the entry was logged: the replay of the chain up to and including it, with the claims that settle to, its own claim among them, as a registry replays a chain to log an entry. Where that replay is itself refused or does not settle, which only entries that later claims make void can cause, the entry is checked against the chain's own settled claims. It is never checked against a state that a claim logged later rewrote. A mismatch at its logging still refuses the chain (U7010); a later claim never makes an earlier entry's stated threshold a refusal, and no pass of the fixpoint that assumes a claim refuses an entry for its stated threshold. The fixpoint's rules for which keys are valid are otherwise unchanged.
2. **Timings that differ, for keys and consent.** The comparison of a request's served and earliest timings (`spec/30 §14` rule 1) also covers rule 8's verdict: which requests took effect, from which position, and so whether the owner's keys changed since a lock's point. When the two timings give different histories, the owner's names are unavailable (U7013), and each request's missing deadline checkpoint is recorded, never decided on the served timing.

**Alternatives rejected.**
- **A mismatch voids the entry** instead of refusing the chain. A thief's claim could then rewrite the state so that a victim's honest entries mismatch, and drop them: an overruling, a cancellation of the thief's request, or a co-signer change would be lost to a later claim.
- **Refusals count only in the settled pass.** It removes the refusal of a pass that assumes a claim, but in the scenario above the settled pass is the one where the claim stands, and the overruling still mismatches there: the owner's claim is still refused.

**Consequences.**
- An owner's response by the rules can always be logged after honest entries whose stated thresholds were right when they were logged; a registry checks a new entry against the chain it holds, as a client does.
- The client settles one replay of a prefix of the chain per revocation from the first claim on, to know the claims at each logging: O(S · P · E · K) for S such revocations, P passes, E entries and K keys, and nothing more for a chain with no claim. Revocations are rare events of a chain.
- More names are unavailable when a place withholds cosignatures: whenever the two timings give different histories, not only when a release's verdict differs.
- `spec/30 §14` rules 1 and 5 and the `ita-key/1` table state these rules as normative text.

## Amendment 10 (2026-10-26): the cap of claims

- Status: Accepted, recorded under the project's delegation.

**Context.** Amendment 9 checks each entry's stated threshold against the state known at its logging. A claim of compromise is retroactive, so the client settles the chain up to each claim once more, and each settled replay is linear in the chain. A thief holding a stolen key can log claims cheaply, `revoke <former key> since <p>` while that key is current, so with no bound the work grows with the product of the claims and the chain's length: quadratic in what an attacker logs.

**Decision.**
1. **No refusal by count.** A registry never refuses a claim because of how many claims a chain holds: the owner's claim is always loggable.
2. **Reset points.** The client counts the claims, `revoke <name> since <p>` entries applied or void, in one pass over the chain before any replay, by segment between reset points. A reset point is the effect position of a dispute, which cannot be cancelled, or of a recovery that no `cancel` entry names, whose effect point the witness time reached. Reset points are read from the entries and the effect points only, never from the claims, so the count does not depend on the fixpoint. An effect point that cannot be read makes the names unavailable (U7013), a reset neither assumed nor denied.
3. **Sealed segments.** A segment of more than 64 claims that a reset point follows is sealed. Every entry in it is void, except the dispute or recovery that made the reset: no key change in it applies, its claims are dropped, and no release logged in it counts, the owner's own included, which the owner republishes after the reset. The key state resumes at the reset point from the key the reset names. Claims outside sealed segments still reach back with their `since`, so the owner's claim after the reset voids the thief's releases before it. The history exposes each sealed segment, its first and last log index and the reset that sealed it, for `una ita audit` to name.
4. **The last segment.** A client does not replay a chain whose last segment, with no reset after it, holds more than 64 claims. Every name of that owner is refused (U7010), giving the count and the log index of the first claim past it, and no release of the owner counts, the thief's included. It is not the registry's misbehavior: an honest registry logs it.
5. **Why 64.** It is twice the most keys an owner holds at once (32): an owner can claim every key it holds, and claim each again from an earlier position once it learns more. An honest owner claims a stolen key once, rarely more than a few keys in a chain's life.
6. **The work.** The claims replayed are at most 64 per segment that is not sealed, 64 times one more than the resets, and a sealed segment costs O(E). A reset needs the co-signers or the registry's dispute, which a key-only thief cannot obtain, so a thief cannot cheaply raise this bound. The client settles the chain once, then once more up to each claim, and makes one forward pass per group of entries logged with the same claims, linear in the chain's length for a bounded number of resets. Each signature of an entry by a key is verified once across all of them, and the count of checks covers all of them.
7. **Evidence and omissions.** A replay up to an entry that does not settle is recorded as the registry's misbehavior. A request's effect that cannot be read during any replay makes the name unavailable (U7013), never a verdict.

**Rationale: integrity over availability.** A cap that refuses claims at logging lets a key-only thief spend the claims and block the owner's own, so the thief's releases signed with the stolen key would still count. A budget of claims per signing key does not hold: the thief adds keys and spends each one's budget. Freezing the chain at the cap does not either: releases logged before the freeze would still count. Sealing is safe: only a reset seals, which a key-only thief cannot make; sealing voids releases and never makes one count; the owner's honest entries in a sealed segment are an availability cost only.

**Consequences.**
- A key-only thief can make an owner's names unavailable by logging claims past the cap, and never make its own releases count.
- The owner restores its names by a dispute, or a recovery with no cancellation against it, that takes effect, followed by its claim; the owner republishes its releases from the sealed segment. A thief that cancels the owner's recovery keeps it from resetting the count, and the owner then uses a dispute. The owner may also register under a new owner name.
- `spec/30 §14` rule 5 states these rules as normative text.

## Amendment 10, correction (2026-10-26): reset points

- Status: Accepted, recorded under the project's delegation.

**Context.** Amendment 10 read a reset point syntactically: the effect point of any dispute, or of a recovery that no `cancel` entry names. A thief holding a key and the account could log claims past the cap, then a dispute naming its own key. The owner's competing answer within the window would then fall in the sealed segment, so nothing competed and the thief's dispute took effect. That breaks rule 5: an answer within the window forces at worst a freeze. A `cancel` naming a future index also kept a recovery from resetting.

**Decision.** A recovery or a dispute is a reset point only when all of these hold, read from positions and witness time only, never from the claims:
1. the witness time reached its effect point;
2. no other recovery or dispute is logged from it to its effect point;
3. no freeze can be in effect at its effect point: no two recoveries or disputes waited at once before it;
4. for a recovery, no `cancel` naming it is logged after it and before its effect point.

A recovery, a dispute or a `cancel` logged while a resetting request waits is never sealed.

**Correction.** Where Amendment 10 says the owner "then uses a dispute", an owner with co-signers in effect cannot dispute. A thief that cancels the owner's recovery while it waits keeps it from resetting. Once that recovery takes effect after the co-signers' overruling, the thief holds no current key, so the owner's next recovery cannot be cancelled and resets the count.

## Amendment 11 (2026-10-26): the own-word floor from the first assumption

- Status: Accepted, recorded under the project's delegation.

**Context.** A revocation that meets its requirement only with the signature of the key it names is that key's own word, and its `p` is taken as no earlier than its own index. The replay applied that floor only from its second pass: the first assumed every claim at its stated `p`. Scenario: `A` registered; `A` and `B` add `B`; `A` is stolen; the thief logs `revoke A since 0` signed by `A` alone; the owner answers `revoke A since <the addition>` signed by `B`. Assumed at `since 0`, the thief's claim voided `B`'s addition, so the owner's claim was void and dropped, and a dropped claim never returns: `A` was compromised only from the thief's own claim, and what `A` signed between the addition and that claim counted.

**Decision.** The floor holds from the first assumption. A claim whose signatures include the key it names, read from its signatures alone, is first assumed from its own index, whatever `p` it states. A later pass gives it back its stated `p` only when its other sound signers, or the cosigners in effect, meet its requirement without the named key. The fixpoint keeps its shape: claims are only dropped, never added back, and a chain whose claims do not settle within the pass cap is refused as before.

**Consequences.**
- A thief's self-claim, signed with the stolen key alone or with keys that key added after the theft, never reaches back before its own index, so it never voids what the owner's keys signed before it, and the owner's claim stands.
- `spec/30 §14` rule 5 states this as normative text.

## Amendment 12 (2026-10-26): `publish to` names a workspace source

- Status: Accepted, recorded under the project's delegation.

**Context.** `spec/30 §2` rule 4 required `publish to` to name `itaty` or a source of the package's own `sources` section. Rule 43 refuses to publish a package with a `sources` section, and `spec/30 §15` rule 1 takes a source other than `itaty` from the `sources` of the nearest workspace whose `itaty.una` lists the package as a member. The two rules contradict each other: a package published to a private source failed its own manifest check (U7008), so it could not be published, and the archive of one already logged failed the check of its code at fetch, which allows U7008 only for registry needs, and at `una check`.

**Decision.**
1. `publish to` names `itaty` or a source name that `sources` could declare, never `path`, `git` or `workspace`; any other name is U7008 at the line, as before. Reading `ita.una` does not look the source up.
2. `una ita publish` and `una ita yank` look it up when they run, before anything is built or sent: `itaty`, or a source that the `sources` of the nearest workspace listing the package as a member declares. A source no such workspace declares, a workspace that does not declare it, and a workspace file that cannot be read are refused (U7013), each by its kind, never by a message; nothing is sent.
3. The `ita.una` of a published archive, read at fetch or by `una check`, therefore accepts its `publish to` line. No other diagnostic is relaxed: rule 43's refusals and the archive check's allowances are unchanged.

**Checks.** The manifest tests accept `publish to acme` without `sources` and refuse `path`, `git` and `workspace`. The client's lookup is tested for each kind of refusal. `una ita publish` and `una ita yank` refuse a source the workspace does not declare. A release published to a private source is locked, fetched, yanked and audited end to end.

**Consequences.**
- A package names its private source in its own `ita.una`; the declaration of that source, its URL and its root's pin, lives in the workspace, where the publisher's other packages find it too.
- `spec/30 §2` rule 4 states this as normative text.

## Amendment 12, correction (2026-10-26): when the source is looked up

- Status: Accepted, recorded under the project's delegation.

Amendment 12, decision 2, says the source is looked up "before anything is built or sent". `spec/30 §15` rule 3 orders publishing: the refusals of `spec/30 §11` rule 12, then a package in a git repository, then `una check` at tier 1, and only then the archive and the source. The lookup keeps that order: `una ita publish` looks the source up after the package's own refusals and its check, and before its archive is built or anything is sent. `una ita yank`, which checks no code, looks it up before it reads the registry. A package that does not check is refused for that, whatever source it names.

## Amendment (2026-10-19): `serve` grants (ADR 0123)

Rule 12's grants gain `serve "<scheme>://<address>:<port>"`, the `ita.una` form of the `serve:` grant of ADR 0123 rules 6 to 9, with its items as lines below it, one item per line: `certificate "F"`, `key "F"`, `tls12`, `at most N at once`, `at most N per second|minute|hour`, `waiting|connections|requests|headers at most N`, `peer at most N` or `peer none`, `handshakes at most N per second`, `line|header|body at most N KiB`, `buffers|replies|delivered at most N MiB`, `peer_buffers|peer_replies at most N MiB` or `none`, `idle|read|write|answer within N second|seconds`, and `hosts "a", "b"`. The grammar, the mapping of each line to the item of the text form, and the errors are those of `spec/31 §2` rules 3 and 4; every error is U7008 at its line (rule 13). The production `grant` of rule 5 gains the alternative `serve_grant` of `spec/31 §2` rule 3, and the limit lines of ADR 0117 stay under `http` grants only. As for every grant, `--grant` keeps the text form, and a dependency's `run` section is not read.

## Amendment of 2026-10-31: `una migrate` converts `quality.toml` (ADR 0131)

Proposed together with ADR 0131 (issue #1302). It takes effect when ADR 0131 is accepted and its migration slice lands; until then rule 64 reads as written.

1. **Rule 64, a package that already has `ita.una`.** Rule 64 makes `una migrate` on such a package a usage error. With this amendment, when the package root holds `quality.toml`, `una migrate` converts it to `quality.una` (ADR 0131 rule 12) and does nothing else; it stays a usage error only when there is nothing to convert.
2. **Rule 64, a package of edition `0.1` to `0.3`.** `una migrate` also converts `quality.toml` when the package root holds one, in the same run as the edition move. `una migrate --only quality` converts the quality file alone and leaves the manifest, the lock and the edition unchanged.
3. **Question 8.1.** "One manifest format and one parser" in edition `0.4` is kept and extended: every configuration file of an edition `0.4` package is in the lexical syntax of rule 4, read by the syntax crate (ADR 0131 rules 3 and 4). The TOML reader that `quality.toml` uses until the slice lands is accepted for that interim (ADR 0131 rule 17).

No other rule of this record changes.
