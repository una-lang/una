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
10. Order now (owner directive 2026-11-02: language first, Una Quality afterwards; up to 3 cards in flight in parallel; ADRs 0123 S4, 0127-0131, 0133 accepted, PR #1427): epic by epic, security cards first when they appear. Done 2026-11-02: #1023, #1209 slice 4b (4c: resolution, layout, GOT; needs the P2 probe #1204 before shipping, and #1100 7b before the C compiler is dropped), #1430 fuzz gate, #1389. In flight: #1436 (security S-34), #1432. Then #888 closes with #946; #895 closes with #983 (#1089 goes with #736); #1097 without the release path: #1100 slice 8 then 7b, #1059, #1102/#1276, #962; #891 (4c/4d need probe P2 #1204; P1 #1203); #897 (S5 #1007 needs L_srv, oracles, #1059, 7b; leftovers #1390 #1391 #1392 #1172 #1009); #896; #893; #894 (#991 simd first); ADR 0130 (#1325...); #852 after ADR 0132 text fixes and PR #1351 rebase. Also: #1394 (non-root Linux container), #1340, #1386, fuzzing minimal gate. v0.3 closing work: #1395 #1396 #1397 #1017 (see ops/coordination/owner-review-2026-11-02/). Una Quality (#1292, tree in quality-epic-tree.md) after the language epics; ready heads: #1316, #1325, #1333.
11. Pacing: one card in flight, about three code cards a day; small cards when the day's budget is spent.
