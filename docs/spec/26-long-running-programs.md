# 26. Long-running programs

Status: Normative, edition 0.4 preview (ADR 0116).

A program that runs until it is told to stop, such as a game loop, a server or a worker, hands its loop to the host, or runs a `for ever` loop in a function whose header says `diverges`. The rules of ADR 0116 join this chapter as they land, each with its conformance cases; until a rule has landed, the construct it defines is a syntax error (U0101) or its word is reserved (U0103).

**Editions.** The rules of this chapter apply only to packages of edition `0.4` (`spec/13 §7`). Earlier editions do not change.

## 1. Edition and words

1. **Edition.** A package of edition `0.4` names it in `ita.una` (`edition 0.4`, `spec/30`); `una.toml` naming it is U7002, with `una migrate` as its repair (ADR 0122 Question 8.1). Edition `0.4` is cumulative over edition `0.3`, as `0.3` is over `0.2`: it has every construct, keyword and rule of edition `0.3`. It is a preview under the terms of edition `0.3` (`spec/13 §7`) until the rules that target it have landed.
2. **Reserved words.** In edition `0.4`, `diverges` and `within` are reserved: used as an identifier, each is U0103, `` `<word>` is reserved for edition 0.4 (long-running programs) ``, until the construct of this chapter that uses it lands. In earlier editions they are identifiers.
