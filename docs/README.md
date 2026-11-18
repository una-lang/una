# Documentation map

## Reading order

1. `00-charter.md`: thesis, principles, proof boundary, and v0.1 scope. Read first, then identify the package edition and its terms in `spec/13 §7` before selecting normative rules.
2. `spec/`: the normative language definition, in numeric order. Read the chapters that apply to the package edition; `spec/13 §7` states the edition terms.
3. `compiler/`: how the reference compiler is built.
4. `adr/`: why key decisions were made. `plan/`: roadmap, design notes, deferred backlog. `reviews/`: triage of external design reviews and the review records of each release.
5. `guide/syntax-reference.md`: the language guide; `guide/una-for-agents.md`: a compact reference for agents writing Una. `guide/known-limits.md`: what this release does not do, or does only with limits, with the rule of each item. `guide/serving-http.md`: how a package of edition `0.4` serves HTTP, from its grants to its tests. `guide/language/control-flow/for-ever.md`: loops that run until stopped in edition `0.4`, `for ever`, `for until`, `stop`, `skip`, `diverges`, `on stop` and the host stops. `guide/driven-programs.md`: how a package of edition `0.4` hands its loop to the host, with a `drive` section and `init`, `step` and `on_stop`. `guide/tasks-and-time.md`: tasks and time limits, `together`, `tasks` and `start`, `within` and `first of`, cancellation, and the timing labels the checker enforces. `guide/language/` and `guide/configure/` contain construct and `una.toml` pages with examples, edge cases, editions and normative sources; the guide tests their code blocks (#585).

## Normative status

| Folder | Status |
| --- | --- |
| `spec/` | Normative. Uses RFC 2119 keywords. |
| `compiler/` | Normative for the reference implementation, not for the language. |
| `guide/` | Informative. |

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
| `spec/14-standard-library.md` | Prelude, host capabilities and `std.testing`, by edition |
| `spec/15-canonical-form.md` | The canonical form: lines, spacing, blank lines, comments, U7001 |
| `spec/16-native-backend.md` | The native backend: `una build` and native executables |
| `spec/17-memory-model.md` | Mutable value semantics: `let edit`, `edit` and `take` parameters, `for` loops |
| `spec/18-fixed-width-integers.md` | Fixed-width integer types and overflow obligations |
| `spec/19-structured-concurrency.md` | Structured concurrency: `together` blocks and `together for` |
| `spec/20-constraints-on-generics.md` | Built-in constraints on type parameters |
| `spec/21-maps.md` | `Map[K, V]`, ordered by its keys |
| `spec/22-text.md` | Text operations of the prelude |
| `spec/23-json.md` | JSON values, parsing and serialization |
| `spec/24-http.md` | The HTTP client capability `Http` |
| `spec/25-units-of-measure.md` | Units of measure, edition `0.3` |
| `spec/26-long-running-programs.md` | Long-running programs, edition `0.4` |
| `spec/27-tasks-and-time.md` | Tasks and time, edition `0.4` |
| `spec/28-collections.md` | Collections, edition `0.5` (moved from edition `0.4`, ADR 0139) |
| `spec/29-numerics.md` | Numerics: `F64` in edition `0.3`, the numeric foundation in edition `0.5` (moved from edition `0.4`, ADR 0139) |
| `spec/30-packages.md` | Packages and dependencies: `ita.una` and `ita.lock`, edition `0.4` |
| `spec/31-http-servers.md` | HTTP servers, edition `0.4` |
| `spec/32-quality.md` | Una Quality: metrics, configuration, gates and the `UQ` registry (Draft, ADR 0127 accepted; not implemented, no command runs it) |

## Starting a contribution

Use the assigned issue as the scope. Read the charter, identify the package edition, then read its normative specification before changing code. Check `docs/compiler/` for implementation conventions, and see `CONTRIBUTING.md` for what to build and check. The site is a separate repository pinned by `site/website.lock`; the maintainers build it from this checkout.

## Conventions

- Code blocks tagged `una` in the spec are illustrative fragments and may omit module headers or surrounding declarations. Complete, tested programs live in `examples/`, `tests/conformance/` and `guide/una-for-agents.md`.
- Formal rules use LaTeX in `$$` blocks.
- Section references use the form `spec/03 §4`.
