# State (2026-10-27, consolidation day)

- main: moving; the ADR 0117 tower and the remaining lanes are being merged in dependency order (see /Users/leocardoso/Projects/una-lang/ops/coordination/consolidation/PLAN.md until the move completes).
- Release lines: release/v0.3 at 807dca5e (v0.3.17 candidate, gate run f); release/v0.1 at v0.1.13.
- Open P1 on main: #1254 (timing warm-up channel) - fix #1255 merged, oracle to confirm at release time.
- Machine: Docker Desktop (desktop-linux), amd64 under Rosetta; Rosetta cannot run the million-task test (#1246); cmake installed for the Z3 build.
