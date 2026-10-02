# 0063. Language server

- Status: Accepted
- Date: 2026-10-02
- Deciders: Leonardo (accepted on 2026-10-02), on a proposal by Claude (Una-1)
- Resolves: issue #43 (backlog row "Language server (LSP)", design D)
- Amends: `spec/13 §5` (a command), `compiler/architecture.md §2` and `§7`

## Context

Una is written by agents and reviewed by people. Agents use the Agent ABI:
- `una --json check` (`una.diagnostics/1`, with repairs and their edits);
- `una obligations`, `una contract` (`una.contract/1`), `una report` and `una explain`.

People read and review in an editor, and today an editor shows them nothing until they run the CLI. A language server puts the same information where they read: the diagnostics, the proofs, the contracts, the labels and the repairs.

What exists:

- **Tiers.** Tier 0 (`una check --tier 0`) needs no solver. Its target is under one second for 5,000 lines (`compiler/architecture.md §8`). Tier 1 adds the obligations, solved by Z3 through `verification.lock`, with the proof cache in `.una/cache/proofs/` (`spec/12 §9`). Results are deterministic (`spec/12 §10`).
- **Measured.** On the example packages, cold tier 1 takes under 0.1 s and a cached one about 0.01 s, in a debug build.
- **One value, two renderings.** Every diagnostic is one value (`spec/13 §1`). Repairs carry mechanical edits (`spec/13 §3`). The formatter is one function, `una_fmt::format`. The contract of a function is `una.contract/1` (ADR 0029).
- **Positions** are 1-based lines and columns counted in Unicode scalar values (`spec/13 §1`). LSP counts UTF-16 code units by default, and LSP 3.17 lets the client accept UTF-32.
- **Dependencies** beyond the allowlist need an ADR (`compiler/architecture.md §7`).
- **Editions.** A package has edition `0.1` or `0.2`. The Gate 1 benchmark stays on `0.1`.

The proposal asked six questions: the binary (L1), the protocol (L2), the features (L3), when the checks run (L4), the solver and the cache (L5) and the editions (L6). Leonardo answered them on 2026-10-02, each with option A. The other options are under "Alternatives considered".

## Decision

1. **The binary (L1).**
   - `una lsp` is a subcommand that speaks LSP on stdin and stdout.
   - A library crate, `una-lsp`, holds the protocol and the server's state.
   - `una-cli` calls it and passes it the function that checks a package, so no crate depends on `una-cli`.
   - `una-lsp` depends on `una-diag`, `una-syntax`, `una-fmt`, `una-project` and `una-check`, for the positions, the formatter, the package's files and the headers and resolutions of a check.
   - One binary keeps one version, one lock and one Z3 for the CLI and the editor.
2. **The protocol (L2).**
   - JSON-RPC 2.0 over stdio with `Content-Length` framing, written on `serde_json`.
   - Hand-written types cover the subset of LSP 3.17 that rule 3 needs.
   - No dependency is added.
3. **The features of v0.2 (L3).**
   1. **Diagnostics,** published per file:
      - from `una.diagnostic/1`, with `code`, `severity`, `related` as related information, and a link to `una explain`;
      - a refuted or unknown obligation is a diagnostic as in the CLI (U5001, U5002), with its counterexample in the message.
   2. **Code actions.** Each repair with an edit is a quick fix, in the order of its escape cost, then rank. A repair without an edit is shown as advice in the diagnostic.
   3. **Formatting:** the whole document, through `una_fmt::format`.
   4. **Hover:**
      - on a function's name or a call: its contract (`una.contract/1`), with the header declared or inferred, `uses`, `fails`, `requires`, `ensures`, doc comments and requirements;
      - on a value: its type with labels and refinements;
      - on an obligation's site: its status and goal.
   5. **Go to definition:** functions, kinds, variants, capabilities, handlers, policies, requirements and locals.
   6. **Document symbols:** the outline of a module.
   7. **Code lens on each function:** "n proven, m refuted, k unknown", from the obligations of tier 1.
4. **When the checks run (L4).**
   - **On every change:** the whole package is checked again at tier 0, debounced by about 200 ms, over the open buffers, so unsaved text is checked.
   - **On save:** tier 1 runs on the saved package, through the proof cache.
   - **Cancellation:** a newer change cancels a running check and kills its solver processes.
   - There is no incremental compilation in v0.2.
5. **The solver and the proof cache (L5).** The server uses `verification.lock` unchanged (step budget, seed, solver version) and the CLI's wall-clock guard, and it shares `.una/cache/proofs/`. A result in the editor is the result of `una check`, and `spec/12 §10` holds.
6. **Editions (L6).** Each package is served by its `una.toml`, a single file as edition `0.1`. The server ships with the v0.2 compiler and serves both editions.

Rules that follow:

7. **Positions.** The server offers `positionEncoding` UTF-32, which matches Una's columns in Unicode scalar values. When the client cannot use it, the server converts to and from UTF-16 at the boundary.
8. **The Agent ABI is the model.** Diagnostics, repairs, contracts and obligations reach the editor as the same values that `--json` prints, converted at the edge. No check exists only in the editor.
9. **What the server never does.** It never runs programs, examples or `una build`, and it grants no capability. It runs only the checks, the formatter and the solver of `verification.lock`.
10. **Several packages.** Each file belongs to the package of the nearest `una.toml` above it.
11. **Commands and schemas.** `una lsp` joins `spec/13 §5`. The `una.*` schemas are unchanged.
12. **Testing.** Scripted sessions run the server in process: requests in, responses and notifications out, compared as JSON. Its diagnostics on a scripted package equal `una --json check` on the same files.
13. **Architecture.** `compiler/architecture.md §2` lists `una-lsp` and its dependencies. `§7` notes that it needs no dependency beyond the allowlist.

## Consequences

- Reviewers see diagnostics, proofs, contracts and labels in their editor, and apply repairs as quick fixes.
- The server adds a crate and a command, but no dependency (L2 A) and no new check.
- Large packages may need incremental checking later. The tier 0 target is the trigger.

## Alternatives considered

- **A separate `una-lsp` binary** (L1 B). Not chosen: it can drift in version from the CLI, and the archives would ship two executables.
- **`lsp-server` and `lsp-types`** (L2 B). Not chosen: two dependencies and an amendment of the allowlist, for a subset that `serde_json` covers.
- **Completion, find references and rename in v0.2** (L3 B). Deferred: they roughly double the work, and agents, the main writers, do not need them.
- **Tier 1 on every change** (L4 B). Not chosen: the solver would run on half-written code. **Tier 0 only** (L4 C) would show no proofs.
- **A smaller solver budget in the editor** (L5 B). Not chosen: a proof could be `unknown` in the editor and `proven` in CI.
- **Edition `0.2` only** (L6 B). Not chosen: it leaves the stable edition, and the benchmark's packages, without the server.
- **An editor-specific extension without LSP.** Not chosen: LSP serves every editor with one server.
- **A daemon shared by the CLI and the editor.** Not chosen for v0.2: the shared proof cache already reuses work, and a daemon adds lifetime and locking problems.
