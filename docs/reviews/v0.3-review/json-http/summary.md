# Cross-review of JSON and HTTP for v0.3.0: summary

- **Decision:** N-37. Reviewer: Claude (Una-1). Authors under review: Una-2, JSON (#238, ADR 0073, `spec/23`) and HTTP (#239, ADR 0074, `spec/24`), and where they meet Map and Text.
- **Candidate:** `5495b61d65508fe1d26785f335d861799a58757f`, `main` after #298. Rechecks: H-1 at `651062b` (#317), H-2 at `1f461b5` (#318).
- **Method:** `plan.md`. Findings: `findings.md`. Cases: `cases/`, each a package or a script that can become a regression test.

## Verdict

**Ready**, once #317 and #318 merge. Two findings, both fixed and rechecked:

| Finding | Severity | Fix | Recheck |
| --- | --- | --- | --- |
| H-1: whether an `Http` request fails revealed its `internal` arguments as `public` | Critical | #310, PR #317, N-39 and N-40 | Una-1, at `651062b`: Clean |
| H-2: one large `parse_json` aborted the process instead of ending with U6003 | Minor | #314, PR #318, N-41 | Una-2, the JSON author, at `1f461b5`: approved |

No other information-flow leak, no credential sent where `spec/24 §3` forbids it, and no divergence between the engines was found.

| Area | Result | Evidence |
| --- | --- | --- |
| 1. HTTP responses parsed as JSON | Clean | A two-origin logging server: credentials dropped after a redirect leaves the origin and kept dropped on the way back; every failure kind; `TimedOut` at `--http-timeout` |
| 2. JSON built from Map and Text | Clean | 10,119 inputs against an independent reference of `spec/23`: 0 differences in value, failure or position, every round trip a fixed point; 66 texts built into JSON from maps and text functions: 0 differences |
| 3. Labels across the four | H-1, fixed | 23 cases, then 48 in the recheck: every deciding argument, every method, loops, `together`, generics, `fails of`, `Files` and `Keys` in both editions, and no over-labeling |
| 4. Parity of the engines | Clean | Identical requests, results and traces, `host:Http` events redacted and the key in none; U6004 traces identical |
| 5. Performance | H-2, fixed | Doubling from 1 to 21 MB, best of three: ×1.8 to ×2.3 in both engines, before and after the fix |
| 6. Failure labels (N-27) | H-1, fixed | `parse_json`'s failure carries its text's label; `Http`'s now carries its deciding arguments' |
| 7. Edition 0.1 and names | Clean | 13 cases in both editions |
| 8. Documentation | Clean | The guide's JSON and HTTP sections are tested and agree with the specification; the CHANGELOG lines match |
