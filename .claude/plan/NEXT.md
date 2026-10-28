# Next (ordered)

1. Decide #1277 (scope of the no-libm and load-command checks with the solver inside `una`) and fix #1287/#1288 so a fresh clone builds and tests.
2. ADR 0124 step 2 (#1207): the runtime inside `una`; then step 3 (#1208, macOS linker) and step 4 (#1209, Linux static executables); then step 5 (#1210): the one-file install.
3. ADR 0123 S4-S7 (#1006-#1009): the HTTP server.
4. No-foreign-code epic #1097: #1099 (withdraw the C backend), #1100 (own TLS crypto), #1093 (own big integer).
5. ADR 0120 S8 (#991, simd required) now that profiles exist; #736 raw cells then #1089.
6. Release candidate evidence (skill `release`): sanitize, Miri, hunt, timing oracles (#1254), Linux probes P1/P2/P6 (#1203-#1205), amd64 on real hardware (#1246); v0.3.18 / v0.1.14 for #923.
7. Docs: guide and Learn for edition 0.4, one install page, #1245 LSP; ADR 0126 (module constants) awaits a decision.
