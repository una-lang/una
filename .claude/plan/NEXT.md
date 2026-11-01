# Next (ordered)

1. Decide #1277 (scope of the no-libm and load-command checks with the solver inside `una`) and fix #1287/#1288 so a fresh clone builds and tests.
2. ADR 0124 step 2 (#1207): the runtime inside `una`; then step 3 (#1208, macOS linker) and step 4 (#1209, Linux static executables); then step 5 (#1210): the one-file install.
3. ADR 0123 S4-S7 (#1006-#1009): the HTTP server.
4. No-foreign-code epic #1097: #1099 (withdraw the C backend), #1100 (own TLS crypto), #1093 (own big integer).
5. ADR 0120 S8 (#991, simd required) now that profiles exist; #736 raw cells then #1089.
6. Release candidate evidence (skill `release`): sanitize, Miri, hunt, timing oracles (#1254), Linux probes P1/P2/P6 (#1203-#1205), amd64 on real hardware (#1246); v0.3.18 / v0.1.14 for #923.
7. Docs: guide and Learn for edition 0.4, one install page, #1245 LSP; ADR 0126 (module constants) awaits a decision.

## Added 2026-10-31
8. Una Quality epic (#1292), all in Needs spec: foundation ADR #1296 first (identifiers #1303 `UQ`, configuration #1304, file format #1302), then phases #1297-#1301; guarantee diff #1291; specification mutation #1293; requirement coverage #1289; typestate ADR #1290; Language Pack #1295 (Ready). Metric definitions: ops/coordination/UNA-QUALITY-METRICS-DEFINITIONS.md (comment on #1296).
9. #1099 needs the dated amendment of ADR 0106 that ADR 0124 question 1 already ruled (the C backend is withdrawn; the release path becomes Una's own code generation).

## Added 2026-11-01
10. Order now: #1341 (in flight), #1342, #1343 (complexity findings), #1368 (bigint follow-ups), #1376 (S4 follow-ups), then #1209 slices 4a+4b (plan ready), #1007 (S5) only after the L_srv labels and timing oracles exist, #1100 slice 7b/8 after the owner accepts ADR 0133, #1340 (Z3 source address), #991 (simd).
11. Pacing: one card in flight, about three code cards a day; small cards when the day's budget is spent.
