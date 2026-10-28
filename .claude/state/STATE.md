# State (2026-10-28, after the consolidation into main)

- main: ce178871, workspace tests 1914 passed / 0 failed / 63 ignored, clippy and fmt clean. Tests need the built Z3 library: `UNA_Z3_LIB=~/.cache/una/z3/<key>/lib/libz3shell.a` (#1288).
- Branches: only `main`, `release/v0.1` (v0.1.13), `release/v0.3` (v0.3.17, tag 810837af) and the two `legacy/*` refs. No worktrees. Parked work lives as patch series under /Users/leocardoso/Projects/una-lang/ops/parked/ with a "Parked:" card each (#1274, #1275, #1276, #1279, #1281, #1282, #1285) plus a bundle of the old local-only branches.
- Releases: v0.3.17 published 2026-10-27 (three security fixes); v0.3.18 and v0.1.14 pending for #923 (UNA-SA-020 draft in ops/coordination/releases).
- Edition 0.4 on main: ADR 0116 S1-S8, 0117 S1-S7, 0119 S1-S9, 0120 S1-S7, 0109 S1-S2, 0122 S1-S8, 0123 S1-S3, 0124 step 1 (Z3 inside una). Open: 0123 S4-S7, 0124 steps 2-5, 0120 S8.
- Open P1s: #1254 (timing warm-up channel; fix #1255 merged, oracle to confirm on a quiet machine), #1265 (pool bounds), #1266 (reactor/helper deadline outcome), #1277 (una loads libc++/libm with Z3: release build gate), #1278 (solver handshake miss), #1287 (cold-cache report test), #1288 (Z3 library for tests).
- Machine: Docker Desktop (desktop-linux), amd64 under Rosetta (the million-task test self-skips there, #1246); cmake 4.4.4; Z3 cache under ~/.cache/una/z3; gate outputs under /Users/leocardoso/Projects/una-lang/ops/gates; GitHub Actions blocked on billing (local gates are the evidence).
