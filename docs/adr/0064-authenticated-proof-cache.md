# 0064. The proof cache is authenticated by a per-user key

- Status: Accepted
- Date: 2026-10-02
- Deciders: Owner (accepted on 2026-10-02), on a proposal by Owner (Una-2)
- Resolves: issue #44 (v0.1 review R-018)
- Supersedes: ADR 0041 rule 1; rules 2 and 3 stay
- Amends: `00-charter.md §3`, `spec/12 §9`, `spec/13 §6`

## Context

ADR 0041 made the proof cache (`.una/cache/proofs/`) part of the Trusted zone, because nothing authenticated its entries. Anyone who could write the directory could turn a `refuted` or `unknown` answer into `proven` (R-018).

That covers more than a careless edit:
- a generating agent;
- a shared checkout;
- a cache restored from an artifact;
- a `.una/` committed by mistake.

Only a forged `proven` breaks soundness. A forged `refuted` or `unknown` makes the check fail, which is visible.

Re-validating a cached `proven` would mean solving the query again, or checking a solver proof certificate with a checker in the Trusted zone. So re-validation cannot make a warm cache both sound and fast.

## Decision

1. **Tags.** Each cache entry carries an HMAC-SHA-256 tag, under the user's secret key, over the entry's key and its answer.
   - An entry whose tag is missing or does not match is not an answer: the query is solved again, and the entry is rewritten.
   - This is never an error, and it is not reported as tampering. A cache from before this ADR re-verifies once.
2. **The key.**
   - It lives outside the package, in the file that `UNA_PROOF_KEY` names, or else in `$HOME/.una/proof-cache.key`.
   - When the file does not exist, `una` makes it from 32 bytes of the system's random source, readable and writable by the user only, in a directory only the user can enter. It creates the file only when it is absent, so two processes that make it at once agree.
   - A key shorter than 16 bytes is not used.
   - The library never reads the environment: `una-cli` resolves the key from the variables the binary passes it.
3. **No key, no cache.** When no key can be read or made, there is no cache for that run: every obligation is solved, and nothing is written. This is never an error.
4. **Trusted zone.** The proof cache leaves the Trusted zone, and the key file joins it. A team that wants a shared cache shares the key file, so that sharing is explicit.
5. **Report.** `una.report/2` keeps its schema.
   - `tcb.proof_cache` still names the cache directory.
   - A new field, `tcb.proof_cache_key`, names the key file: `~/.una/proof-cache.key` for the default, or the value of `UNA_PROOF_KEY` as given. It is `null` when the run used no cache.
   - `obligations.cached` keeps counting the results the cache answered.
6. **Determinism.** Statuses, counterexamples, diagnostics and exit codes do not depend on the cache, since it only skips solving an identical query. Only `obligations.cached` and timing depend on whether the key matches, as they already depended on whether `.una/` existed.

## Consequences

- An edit of `.una/cache/proofs/` can no longer forge a proof. The adversarial program of R-018 is refuted after the forgery too.
- Caches no longer transfer between users or machines unless the key file does.
- The first run after this change verifies everything once.
- In-process callers of `una_cli::check` pass no key, and run without a cache.

## Alternatives considered

- **Re-validating entries.** Rejected: see the Context.
- **Caching only `refuted` and `unknown`.** This is sound with no key, but it gives up the cache for the answers that are worth caching. It would be the fallback if keys were unwanted.
- **A key inside the package.** Rejected: whoever can write the cache can usually read the package too.
- **`una.report/3` with a structured `tcb.proof_cache`.** Rejected to keep the schema that Gate 1's tooling reads, by adding a field rather than changing one.

## Amendment of 2026-10-25: the key file's checks

Accepted under the project's delegation, with advisory UNA-SA-018 (#1160) and the hardening of #1164. The rules above put the key file in the Trusted zone, but left how it is read to the implementation. A key file that someone else can write or swap is no secret. And a cache entry written through a link is a write the package chose. So:

- **The key file.** `una` uses an existing key file only when it is a regular file, not a symbolic link, with exactly one link, owned by the process's effective user, whose mode grants nothing to group or others, and of 16 to 4096 bytes. It checks this on the name, and again on the file it opened (same device and inode). Otherwise the cache is off for the run, and `una` prints a note naming the key file and the reason, except under `--json`, where stderr stays empty (`spec/13 §5`) and the report's `tcb.proof_cache_key` is `null`. A key being installed briefly has two links, so a reader that finds a second link retries for a short time before turning the cache off.
- **The cache entries.** An entry is read only when it is a regular file of at most 64 KiB, and anything else is a miss. An entry is written under the package's root through a fresh, exclusively created temporary file, renamed onto the entry's own name. A link at that name is replaced, never followed, so no write of the cache lands outside the entry it names.
- **Library callers.** A proof cache can only be built with a key of at least 16 bytes.
