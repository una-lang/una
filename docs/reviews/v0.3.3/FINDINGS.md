# Cross-feature review of edition 0.2 (#177), Una-1

This is an adversarial review of the information flow of edition 0.2 across its features, and of the agreement between the native backend and the interpreter. It ran on main 7f03d9c, which includes ADR 0077 stages 2a and 2b.

**How to run it:** `python3 -I gen.py`, then `UNA=<una> ./run-matrix.sh`. The output of the run is in `results-7f03d9c.txt`.

## Method

Each cell is a package `cases/<section>__<name>`.

**Verdicts:**
- A `reject` cell sends a `secret` to the public `Log` sink through a combination of features. It must be rejected with a security diagnostic (U4xxx). If it checks clean, it is a LEAK. If it is rejected only by other codes, the cell is BROKEN, which means the case itself was wrong (every BROKEN cell was fixed during the review).
- Each `reject` cell has a `__control` twin, with public data where the secret was. The control must check clean (`accept`), unless it is marked `rule`.
- `rule` means rejected by a rule that does not depend on the secret.

**Native agreement:** every accepted cell that runs, which is every cell except the HTTP ones (checked only), is run by the interpreter (`una run`, and `una --json run` for the trace) and as a native executable (`una build`, run with `--trace`). The output, the exit code and the trace must be identical.

## Coverage (224 cells)

| Section | What it attacks | Cells |
| --- | --- | --- |
| `edit_loop`, `edit_field`, `edit_list`, `edit_param` | rebinds, field and list updates, ranges, `edit` and `take` parameters, receivers, generic `edit` parameters, all under secret conditions | 18 |
| `labelvar` | ADR 0075 label variables (map value and key, list, option) | 8 |
| `together` | branch results, `together for`, cancellation as a channel, which task failed first, `attempt` in a branch | 12 |
| `uses_of`, `fails_of`, `of_raise` | effect counts and callback failures over secret-filtered lists, user `of` functions, fold steps, failure payloads; decisions on `T` given to functions named by `uses of` (direct, `map`, `together`, `let`, map value, option, `for`, fold) | 26 |
| `map`, `effects`, `print` | keys and iteration order as channels; a labeled key type with a public top; `has`, `get`, `without`, `with`, `len`; a generic tally; effects whose number depends on a secret shape (`for` over secret lists and bounds, `together for`, map keys); interpolation of labeled maps, lists and kinds; filters and loops over secret map values | 46 |
| `json` | parse and branch, whether a text parses, `quote_json`, item counts, `to_json`, a secret object key, key order | 14 |
| `text` | `find`, `split`, `replace`, `chars`, `bytes`, `ends_with`, `to_upper`, `trim_start`, `parse_int`, `find` then `slice`, `join` | 22 |
| `http` (check only) | a secret URL, header value, body or key name; the response status to a public sink; whether an `internal` URL request fails | 6 |
| `fixed` | wrapping, saturating, guarded and narrowing arithmetic on secrets | 8 |
| `generic` | `max` and fold, `sort` position and length, `index_of`, a generic over a map with a secret key, `together` inside a generic decision | 12 |
| `recursive` | tree size (a public shape), total, and a search tree whose shape secret comparisons decide | 5 |
| `handler` | handler arguments captured from secrets: effects, results, failures, and `with` arguments | 8 |
| `policy`, `via_02` | a `via` that calls a module that is not trusted, a trusted function outside a policy, and edition 0.2 constructs inside `via` bodies | 9 |
| `checked` | whether `checked … else fail` fails on a secret | 2 |
| `traces` | the redaction of labeled arguments of secret-accepting operations, interpreter against native (8 shapes, `together` and loops included) | 8 |
| `native` | edge cases, interpreter against native: wrapping `U64`, `I8`, saturating, shifts, masks, negative `/` and `mod`, big `Int`, Unicode lengths and bytes, `split` with empty parts, non-ASCII `to_upper`, `sort` of texts, map key order, a JSON round trip, `parse_int` | 20 |

## Results on 7f03d9c

| Outcome | Count |
| --- | --- |
| Leak cells rejected with U4xxx | 96 of 96 |
| Accepted cells checking clean | 122 of 126; the other 4 are F-1 |
| Accepted cells run on both engines with identical output, exit and trace | 122 of 122 |
| `rule` cells | 2 (the known #40 limit) |
| **LEAK** | **0** |
| **NATIVE** divergences | **0** |

## Findings

No leak was found, so there is no S-number.

### F-1: edition 0.2 label rules are applied inside `via` bodies, which spec/07 §6 rule 6 exempts (a false positive, not a leak)

**What happens.** `via` bodies are not checked for labels (spec/07 §5 rule 3, §6 rule 6), and edition 0.1 rules respect that: a plain `if` on the secret checks clean (`via_02__plain_if_on_labeled_value`). Two edition 0.2 rules do not:
- **Rebinding an editable binding under a labeled condition** (`memory.rs`, `rebind`). Its U4001 "`n` is labeled `public`, and this update runs under a `secret[Pin]` condition" is reported inside `via` bodies: `via_02__edit_under_labeled_condition`, `via_02__loop_bound_from_labeled_value`, `policy__via_with_loop_computes_more_than_its_purpose`.
- **A map key above the map's top label** (`map.rs`, the key check of `set`/`remove`). It is reported inside a `via` body: `via_02__map_key_from_labeled_value`.

**Cause.** Neither check tests `self.labels()`, the guard that carries the exemption (`flow.rs`: `self.pass == Pass::Check && self.ctx.labels`), and both compare with `flows_to` rather than `below`. `flows_to` is never more permissive than `below`, so the effect is extra rejections only: no flow is accepted that should not be.

**Impact.** A trusted `via` written with edition 0.2 constructs (`let edit`, `for`, maps) is rejected although the spec allows it. The workaround is to write the `via` without them.

**Suggested fix (for triage).**
- Guard both checks with `self.labels()`.
- Consider routing them through `below`, so that symbolic labels and `⊤` are treated as in every other flow.

### Design note: a `via` may return a pure function (no finding)

A policy's `to` may be a function type. In `to public fn(Int) -> Int via offset_by` with `offset_by(s) = fn(x) -> x + s`, the lambda's body is part of the `via` body, so it is exempt from label checks, and it runs later, each time the caller calls the function. This declassifies no more than `to public Int` returning `s`. The policy is listed in the build report, its code is the TCB, and a function with `uses` or `fails` cannot be returned (U3006), so only pure computation of the captured value escapes. The coordinator ruled it a design note, with no ADR. It is pinned by the conformance case `security/via_exemption_body_only` (#384), which also shows that the exemption reaches only `via` bodies.

### Observations: redaction differs between outputs (outside the model, reported as asked)

- **O-1 (known, #9).** `una test` prints the result of a `do` line whose *type* is labeled, for example `secret Int` from `reveal(21)`, in clear, in `result.value` and in the failure `reason`. Yet inside the same output, a kind's field declared `secret` is `<redacted:secret>`. Runtime traces redact by static argument labels (spec/11 §5).
- **O-2.** A counterexample of U5001 prints the solver's values of `secret`-labeled bindings in clear, as in `counterexample: pin = -1, result = 0`, in human output, in `check --json` and in `test --json`.
  - These values are chosen by the solver and are not runtime data, so they reveal no secret of a run. That is outside the model.
  - They are, however, printed under a name whose label is `secret`, while traces and test results redact labeled positions.
  - A decision on whether counterexamples redact labeled bindings, as `<redacted:secret>` or by value, belongs with #9.

## Code read (spec/07 paths of edition 0.2)

- **`flow.rs`:** every direct `flows_to` outside `below` (pc against a capability bound, pc against `accepts`, declassification under pc) runs after the symbolic-pc returns, on concrete labels, or is stricter than `below`.
- **`call.rs`:** the edition 0.2 branches are sound.
  - `call_value` strips `T*` from the argument's flow, raises the result and the pc by the decision, and calls `raise_of` for `of` parameters (ADR 0067).
  - `generic_result` uses ADR 0067 summaries in edition 0.2 and D-137's coarse raise in edition 0.1.
  - `decided_callbacks` is edition 0.1 only, and #349 kept it as defense in depth.
- **`memory.rs` and `map.rs`:** see F-1, the only path found that applies a label rule where an exemption holds.
