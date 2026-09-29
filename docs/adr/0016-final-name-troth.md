# 0016. Final language name: Troth

- Status: Superseded by 0025
- Date: 2026-09-29
- Deciders: Leonardo

## Context

The project was developed under the codename Vera. "Vera" collides with github.com/aallan/vera, an unrelated public language for machine-written code that uses the same CLI name (`vera`) and the same file extension (`.vera`) (`reviews/triage.md §5`). A shared CLI name and extension would confuse agents, package tooling and search, and the Agent ABI schema names (`vera.diagnostic/1` and others) would freeze the codename into every consumer once M9 stabilizes them.

## Decision

The language is named **Troth**. Every use of the codename is renamed:

| From | To |
| --- | --- |
| language name Vera | Troth |
| crates `vera-*` | `troth-*` |
| binary `vera` | `troth` |
| extension `.vera` and code fences `vera` | `.troth` and `troth` |
| `vera.toml` | `troth.toml` |
| `.vera/cache` | `.troth/cache` |
| Agent ABI schemas `vera.diagnostic/1`, `vera.diagnostics/1`, `vera.obligations/1`, `vera.report/1` | `troth.diagnostic/1`, `troth.diagnostics/1`, `troth.obligations/1`, `troth.report/1` |
| lock `encoding_version = "vera-smt-1"` | `troth-smt-1` |
| diagnostic codes `V####` | `T####` (the number is unchanged) |

Names derived from the language name follow it: the Vera Abstract Machine (VAM) becomes the Troth Abstract Machine (TAM), and T0103 messages say "not a Troth keyword".

"Vera" remains only as history: in the prior-art section of `reviews/triage.md` and in this ADR. The rename is mechanical and changes no semantics. Git history is not rewritten, and the owner renames the GitHub repository.

## Consequences

- The CLI, the file extension and the Agent ABI schemas no longer collide with aallan/vera, before any public release and before M9 freezes the ABI.
- Every document, test and snapshot uses the new names; old commits keep the codename.
- Diagnostic codes keep their numbers, so decisions and cases that cite a code carry over by changing one letter.

## Alternatives considered

- **Keeping Vera:** the collision would grow with every release and every agent trained on either project.
- **Keeping the `V` prefix for codes:** fewer edits, but codes would keep pointing at the old name in every diagnostic an agent reads.
- **Aver:** ruled out by jasisz/aver, a language with the same goal (`reviews/triage.md §5`).
