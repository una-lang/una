# State (2026-11-01, process = workspace Engineering Harness + v4)

- main: ce178871, workspace tests 1914 passed / 0 failed / 63 ignored, clippy and fmt clean. Tests need the built Z3 library: `UNA_Z3_LIB=~/.cache/una/z3/<key>/lib/libz3shell.a` (#1288).
- Branches: only `main`, `release/v0.1` (v0.1.13), `release/v0.3` (v0.3.17, tag 810837af) and the two `legacy/*` refs. No worktrees. Parked work lives as patch series under /Users/leocardoso/Projects/una-lang/ops/parked/ with a "Parked:" card each (#1274, #1275, #1276, #1279, #1281, #1282, #1285) plus a bundle of the old local-only branches.
- Releases: v0.3.17 published 2026-10-27 (three security fixes); v0.3.18 and v0.1.14 pending for #923 (UNA-SA-020 draft in ops/coordination/releases).
- Edition 0.4 on main: ADR 0116 S1-S8, 0117 S1-S7, 0119 S1-S9, 0120 S1-S7, 0109 S1-S2, 0122 S1-S8, 0123 S1-S3, 0124 step 1 (Z3 inside una). Open: 0123 S4-S7, 0124 steps 2-5, 0120 S8.
- Open P1s: #1254 (timing warm-up channel; fix #1255 merged, oracle to confirm on a quiet machine), #1265 (pool bounds), #1266 (reactor/helper deadline outcome), #1277 (una loads libc++/libm with Z3: release build gate), #1278 (solver handshake miss), #1287 (cold-cache report test), #1288 (Z3 library for tests).
- Machine: Docker Desktop (desktop-linux), amd64 under Rosetta (the million-task test self-skips there, #1246); cmake 4.4.4; Z3 cache under ~/.cache/una/z3; gate outputs under /Users/leocardoso/Projects/una-lang/ops/gates; GitHub Actions blocked on billing (local gates are the evidence).

## Board sweep 2026-10-31
The board was audited: 34 closed issues set to Done, 6 open issues added, the Agent field of open items set to "coordinator" (process v4 has no Una-N sessions), stale In progress / In review items without a PR or branch moved to Ready or Backlog (epics kept In progress). Decisions are logged in ops/coordination/OWNER-DECISIONS-OVERNIGHT.md (N-473 to N-475). The owner is away; the Coordenador works the open cards autonomously.

## 2026-11-01 (harness era)
- Work now runs through the workspace Engineering Harness (`../AGENTS.md`, `../.agents/`, adapter `../.claude/CLAUDE.md`); v4 rules win on conflict. Per card: board In progress -> worktree + `harness.py start --worktree` -> builder -> `ops/run-gates.sh <run> build tests verification` (only way to run gates) -> independent review recorded -> `gate` -> merge -> Done.
- Merged today: ADR 0124 step 3 (#1208 internal macOS linker, PR #1369; card stays open for fuzz/oracle/macos-oldest), #1100 own TLS crypto slices 1-7 (PRs #1370-#1374: una-crypto SHA-2/HMAC/HKDF/ChaCha20-Poly1305/X25519/AES-GCM/Ed25519/ECDSA/RSA verify + own rustls provider behind features; ring still default; ADR 0133 Proposed; slice 7b switch and slice 8 review wait for the owner), ADR 0123 S4 serve engine (#1006, PR #1375; follow-ups #1376). #522 closed (C backend withdrawn).
- In flight: #1341 (native text reads in place), worktree .claude/worktrees/1341. Planned: #1209 (Linux linker; plan in ops/coordination/builder-notes/1209-plan.md; worktree .claude/worktrees/1209 and run 20261101T115937Z-1209-3418c4 prepared).
- Main at b3188f335. Owner decisions pending: ADRs 0127-0133 acceptance, ADR 0133 open questions, S4 amendment of ADR 0123, N-480/481/489 assumptions, N-499..N-504 in ops/coordination/OWNER-DECISIONS-OVERNIGHT.md.
