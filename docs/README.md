# Documentation map

## Reading order

1. `00-charter.md`: thesis, principles, proof boundary, and v0.1 scope. Read first, then identify the package edition and its terms in `spec/13 §7` before selecting normative rules.
2. `spec/`: the normative language definition, in numeric order. Read the chapters that apply to the package edition; `spec/13 §7` lists the edition terms and preview ledger.
3. `compiler/`: how the reference compiler is built.
4. `adr/`: why key decisions were made.
5. `plan/`: roadmap, v0.1 milestones, deferred backlog.
6. `reviews/`: triage of external design reviews, and the independent review of v0.1 (`reviews/v0.1-review/`).
7. `guide/syntax-reference.md`: the language guide; `guide/una-for-agents.md`: a compact reference for agents writing Una. `guide/language/` and `guide/configure/` contain construct and `una.toml` pages with examples, edge cases, editions and normative sources; the guide tests their code blocks (#585).
8. `fixes/`: fix requests and their progress.
9. `history/v0.1/`: the record of how v0.1 was built: the run instructions, the milestone and closure plans, the decision log and the status report.

## Normative status

| Folder | Status |
| --- | --- |
| `spec/` | Normative. Uses RFC 2119 keywords. |
| `adr/` | Normative once `Accepted`. |
| `compiler/` | Normative for the reference implementation, not for the language. |
| `plan/`, `reviews/`, `guide/`, `fixes/`, `history/` | Informative. |

## Specification files

| File | Defines |
| --- | --- |
| `spec/01-lexical-structure.md` | Characters, tokens, indentation, keywords, literals, operators |
| `spec/02-grammar.md` | Surface syntax in EBNF |
| `spec/03-types.md` | Kinds, generics, refinements, predicates, labels, subtyping |
| `spec/04-functions-and-contracts.md` | Declarations, clauses, calls, visibility, `main` |
| `spec/05-capabilities-and-effects.md` | Capabilities, `uses`, `with`, handlers, host capabilities |
| `spec/06-errors.md` | Error kinds, `fail`, handling clauses, `attempt`, `checked` |
| `spec/07-information-flow.md` | Labels, program-counter label, sinks, policies, trusted modules |
| `spec/08-requirements-and-examples.md` | Requirements, provenance, examples |
| `spec/09-core-ir.md` | Core IR and desugaring from surface syntax |
| `spec/10-static-semantics.md` | Formal typing rules |
| `spec/11-dynamic-semantics.md` | Una Abstract Machine, evaluation rules, traces |
| `spec/12-verification.md` | Obligations, decidable fragment, SMT encoding, statuses, lock |
| `spec/13-diagnostics-and-abi.md` | Diagnostic codes, JSON schemas, CLI, build report |
| `spec/14-standard-library.md` | Prelude and host capabilities available in v0.1 |
| `spec/15-canonical-form.md` | The canonical form: lines, spacing, blank lines, comments, U7001 |
| `spec/16-native-backend.md` | Native executable generation and runtime contract |
| `spec/17-memory-model.md` | Editable values, ownership, `edit` and `take` |
| `spec/18-fixed-width-integers.md` | Fixed-width integer types and overflow obligations |
| `spec/19-structured-concurrency.md` | `together` tasks, cancellation and traces |
| `spec/20-constraints-on-generics.md` | Built-in constraints on type parameters |
| `spec/21-maps.md` | Ordered maps and their operations |
| `spec/22-text.md` | Text operations and their contracts |
| `spec/23-json.md` | JSON values, parsing and serialization |
| `spec/24-http.md` | The HTTP capability |
| `spec/25-units-of-measure.md` | Units and measured integer types in edition 0.3 |
| `spec/26-long-running-programs.md` | Host-driven steps and long-running programs |
| `spec/28-collections.md` | Standard collections in edition 0.4 |
| `spec/29-numerics.md` | F64 in 0.3; numeric foundation additions in 0.4 |
| `spec/30-packages.md` | Package and dependency rules in edition 0.4 |
| `spec/31-http-servers.md` | HTTP servers in edition 0.4 |

## Starting a contribution

Use the assigned issue as the scope. Read the charter, identify the package edition, then read its normative specification and accepted ADRs before changing code. Check `docs/compiler/` for implementation conventions, write the relevant conformance or regression test first, and use `scripts/local-ci.sh` for local validation. The site is a separate repository pinned by `site/website.lock`; `scripts/site-bundle.sh` and `scripts/site-website.sh` build it from this checkout.

## Conventions

- Code blocks tagged `una` in the spec are illustrative fragments and may omit module headers or surrounding declarations. Complete, tested programs live in `examples/`, `tests/conformance/` and `guide/una-for-agents.md`.
- Formal rules use LaTeX in `$$` blocks.
- Section references use the form `spec/03 §4`.
