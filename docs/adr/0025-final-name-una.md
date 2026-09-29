# 0025. Final language name: Una

- Status: Accepted
- Date: 2026-09-29
- Deciders: Leonardo
- Supersedes: 0016

## Context

The language was developed under the codename Vera, then named Troth (ADR 0016). This decision fixes its final name before any public release and before M9 freezes the Agent ABI schema names.

**Name.** *Una* comes from Tupi *una*, "black, dark", as in the Una river and in Itaúna ("black stone"). It was chosen for a firm, solid base rooted in Brazilian geography.

**Collisions.**

- An existing functional language is also called Una (github.com/una-language/una-language). It compiles to JavaScript through a Babel plugin, and most of its repositories have been inactive since 2021 (`reviews/triage.md §5`).
- The package name `una` is taken on crates.io, PyPI and npm.
- "una" is a common word in Spanish and Italian, which hurts searchability.

## Decision

The language is named **Una**. Every use of the previous name is renamed:

| From | To |
| --- | --- |
| language name Troth (codename Vera) | Una |
| crates `troth-*` | `una-*` |
| binary `troth` | `una` |
| extension `.troth` and code fences `troth` | `.una` and `una` |
| `troth.toml` | `una.toml` |
| `.troth/cache` | `.una/cache` |
| Agent ABI schemas `troth.diagnostic/1`, `troth.diagnostics/1`, `troth.obligations/1`, `troth.report/1` | `una.diagnostic/1`, `una.diagnostics/1`, `una.obligations/1`, `una.report/1` |
| lock `encoding_version = "troth-smt-1"` | `una-smt-1` |
| diagnostic codes `T####` | `U####` (the number is unchanged), explain files included |

Names derived from the language name follow it: the Troth Abstract Machine (TAM) becomes the Una Abstract Machine (UAM), and U0103 messages say "not a keyword in Una", for example "`match` is not a keyword in Una; use `when`".

Prose never puts an article directly before the name: it says "not a keyword in Una", never "a Una" or "an Una". Diagnostic codes never contain `+`, and Unicode code points stay written as `U+0000`, so the two cannot be confused.

Published package names, if any, use `una-lang` or `unalang`. Both were free on crates.io, PyPI and npm on 2026-09-29.

"Vera" and "Troth" remain only as history: in `reviews/triage.md §5`, in ADR 0016, in this ADR, and in the process files of the autonomous run (`OVERNIGHT.md`, `reviews/morning-report.md`, `reviews/open-decisions.md`, `plan/runs/`). The rename is mechanical and changes no semantics. Git history is not rewritten.

## Consequences

- The CLI, the file extension and the Agent ABI schemas carry the final name before M9 freezes the ABI.
- Every document, test and snapshot uses the new names; older commits keep the previous ones.
- Diagnostic codes keep their numbers, so decisions and cases that cite a code carry over by changing one letter.
- The prefix `U` sits next to the notation `U+0000` for code points. Codes always have four digits and no `+`, so the two stay distinct.

## Alternatives considered

- **Keeping Troth:** the owner chose a name with a Brazilian root for the language's identity.
- **Publishing under the bare name `una`:** taken on the three registries; `una-lang` and `unalang` are free.
- **Keeping the `T` prefix for codes:** fewer edits, but codes would keep pointing at the previous name in every diagnostic an agent reads.
