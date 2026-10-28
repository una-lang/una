# ADR 0109 slice S1: the profile and the declared budget (design note)

- Issue: #31 (ADR 0109, profiles `app` and `systems`; Implementation steps 1, 2, 4 and 7, with ADR 0122's manifest of edition 0.4)
- Base: `main`. The earlier branch `origin/dev/31-profiles-manifest` (42cecf18, edition 0.3 only, 231 commits behind) is read, not merged.
- Scope: no construct is added. A `systems` package becomes declarable in both manifest formats and carries its budget; the `overflow` obligation of ADR 0109 rule 3 is slice S2 (its own PR); the first gated construct, `simd required`, is ADR 0120 S8 (#991), on top of both.

This note fixes how S1 is built and lists its decisions where the records leave a choice. Each takes the most secure option.

## 1. What the tree has

- `una.toml` (editions 0.1 to 0.3, `crates/una-project/src/manifest.rs`) knows no `profile` and no `max_memory`: both are unknown keys, U7002.
- `ita.una` (edition 0.4, `spec/30 §2`, `crates/una-syntax/src/ita.rs` and `crates/una-project/src/ita.rs`) parses `profile app | systems` in the head and refuses `systems` with U7008 "not supported by this release, which does not implement profiles" (`ita.rs:124-140`); the `run` section has `entry` and `grants` only.
- The checker receives the manifest (`PackageInput.settings`) but reads no profile; `una run`, `una test` and the executable take `--max-memory` with 4096 MiB by default (`spec/11 §7`); the executable reads an embedded description (`Meta`, `crates/una-rt/src/runtime.rs`) with its grants.
- The build report's `package` section has `name` and `version`; the Core export header has `schema` and `edition`.
- U2039 and U3032 (ADR 0120 rules 58 and Diagnostics) exist in ADR text only.

## 2. Decisions

**D1: both manifest formats, each under its own edition rule.**
- `una.toml`, edition `0.3` only (ADR 0109 rule 1): `[package] profile = "app" | "systems"` (default `app`; another value U7002), `[run] max_memory = <int>` from 1 to 4096 (U7002 outside); in editions `0.1` and `0.2` both stay unknown keys (U7002). A `systems` package with a `[run] entry` and no `[run] max_memory` is U7002 at the entry (rule 4).
- `ita.una`, edition `0.4` (`spec/30 §2`, under ADR 0122's rule that each `una.toml` key has one `ita.una` line): `profile systems` is accepted; the `run` section gains the line `"max_memory" int_lit NEWLINE`, with the same range and the same rule for a `systems` package with `entry`, each U7008 at the line, as `spec/30 §2` rule 3 makes every manifest problem. The grammar and the key table of `spec/30` grow by that line; the sentence "`systems` is U7008 in this release" goes, and the case `tests/conformance/project/ita_profile_systems_unsupported` is replaced by the accepted case.
- One type for both: `una_project::Profile { App, Systems }` and `Manifest { profile, max_memory }`, so every reader sees one value whatever the format.

**D2: the budget is a ceiling the flag may lower, never raise** (rule 4). With a declared budget, `una run`, `una test` and the executable use it when `--max-memory` is absent; a `--max-memory` above it is a usage error, exit code 2, before anything runs; a lower one lowers it. Without the key, the flag keeps `spec/11 §7`'s meaning. The executable learns the budget from its embedded description (`"max_memory"` in the JSON `Meta`, written by `una build`), and `una_rt::runtime::with_budget` applies the rule once at start; a description that holds a budget outside 1 to 4096 is treated as no description (the executable refuses to start, as for any malformed description). `--max-memory` of `una run` and `una test` becomes `Option<usize>` so "absent" is known. The budget check itself is unchanged: the same `held() + reserved <= ceiling` as today, so U6003 keeps its meaning.

**D3: the profile reaches the checker now, and does nothing yet.** `Env.profile: Profile` from `PackageInput.settings`, beside `Env.edition`, so S2 adds the obligation without touching the plumbing. S1 adds no check that depends on it (rule 2.1: `systems` adds only obligations, which S2 brings). Any construct that S1 could gate does not exist, so U3032 is registered and unused.

**D4: report and Core export.** The report's `package` section gains `profile` (rule 7), and `max_memory` when declared, both read from the manifest (the human form prints `profile: systems` and `max memory: 256 MiB`). The Core export header (`una.core/1`, `spec/13 §8`) gains `"profile": "app" | "systems"` (rule 6), an additive field under ADR 0076. The snapshot of `report_events_json` and the Core header tests change accordingly.

**D5: U2039 and U3032 are registered** in `crates/una-diag/src/codes.rs` (enum, `ALL`, entry), `docs/spec/13 §4` and `crates/una-explain/explain/`, with ADR 0120's meanings: U2039 (types) "a `map`, `zip_map` or `update` in a function with `simd required` whose lambda cannot be vectorized for a listed target"; U3032 (authority) "a `simd required` clause outside a `systems` package of edition `0.4`". Their explain texts say that the clause is ADR 0120 rule 58 and lands with its slice; the `texts.rs` test requires one file per code.

**D6: what does not change.** The call depth, the escape budget, the grants, every check and every run-time check (rule 2.3). `profile` and `simd` keep their lexical status (`simd` is un-reserved by S8 in edition 0.4, not here). No `unsafe`, no dependency added.

## 3. Conformance and tests

- `tests/conformance/project/` as `ita.una` packages of edition 0.4: `profile_systems_budget` (`profile systems`, `run entry`, `max_memory 64`: ok, runs), `profile_systems_without_budget` (U7008 at the `entry` line), `max_memory_above_cap` (4097: U7008 at the line), `max_memory_zero` (U7008), `profile_unknown` (U7008), `profile_app_explicit` (ok); and as `una.toml` of edition 0.3: `profile_systems_budget_0_3` (ok), `profile_systems_without_budget_0_3` (U7002 at `entry`), `profile_edition_0_2` (U7002, unknown key). The existing `ita_profile_systems_unsupported` case is removed (its expectation is reversed by this slice; `editions.rs` is updated if it lists it).
- CLI tests (`crates/una-cli/tests`): `una run` of a package with `max_memory 64` and `--max-memory 65` exits 2 with the usage message; `--max-memory 32` runs with 32 (a program that allocates between the two gives U6003 under 32 and runs under 64); the same for `una test`; `una build` then the executable: `--max-memory 65` exits 2, `--max-memory 32` applies (the shape of the old branch's `native.rs` test).
- Report and Core: `profile` in both documents, `app` by default.
- Scaling test: parsing an `ita.una` with n `needs` lines (n = 2,000 and 4,000) and a `una.toml` with n `[labels]` entries is linear (growth bound 3 of the shortest of three runs); the profile and budget checks add O(1).

## 4. Complexity

Reading the two new keys is O(1) in the manifest's size; `with_budget` is O(1) at start. Nothing on a hot path changes.

## 5. Spec and docs changed in this slice

- `spec/13 §7`: the `[package] profile` and `[run] max_memory` keys of edition 0.3 (text of ADR 0109 rules 1 and 4); `§4`: the rows U2039 and U3032; `§6`: `package.profile` and `package.max_memory` in the report; `§8`: `"profile"` in the Core header.
- `spec/30 §2`: the `max_memory` line in `run`, the key table row, and the profile sentence of rule 4.
- `spec/11 §7`: `--max-memory` against a declared budget.
- Guide pages `docs/guide/configure/package/profile.md` and `docs/guide/configure/run/max_memory.md` (the old branch's text, adapted to both formats), linked from `edition.md`.
- Changelog fragment `changelog.d/31-profiles-budget.md`.
