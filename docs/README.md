# Documentation map

## Reading order

1. `00-charter.md`: thesis, principles, proof boundary, v0.1 scope. Read first.
2. `spec/`: the normative language definition, in numeric order.
3. `compiler/`: how the reference compiler is built.
4. `adr/`: why key decisions were made.
5. `plan/`: roadmap, v0.1 milestones, deferred backlog.
6. `reviews/`: triage of external design reviews, and the independent review of v0.1 (`reviews/v0.1-review/`).
7. `guide/syntax-reference.md`: the language guide; `guide/una-for-agents.md`: a compact reference for agents writing Una, with every block tested.
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

## Conventions

- Code blocks tagged `una` in the spec are illustrative fragments and may omit module headers or surrounding declarations. Complete, tested programs live in `examples/`, `tests/conformance/` and `guide/una-for-agents.md`.
- Formal rules use LaTeX in `$$` blocks.
- Section references use the form `spec/03 §4`.
