# Adversarial check of PR #336 (#333): tuple sorts named by a digest

Reviewer: Una-3. Date: 2026-10-07. PR head: `169b880`, based on `81cf9a8`; the baseline is `81cf9a8`. Platform: macOS arm64, Z3 5.1.0.

**Verdict: Clean** for all four items, on macOS arm64 and Linux amd64.

## Method

`dumpq.rs` is a scratch example binary, added only to the reviewer's copies of the two trees and never to una. Through the public API (`una_cli::check`, `una_verify::obligations`, `una_verify::queries`, with the repository's lock), it writes every SMT query of each program. It ran over:
- every conformance case and example (1,035 programs);
- 7 adversarial packages (`mkcases.py`).

That gives 1,042 programs. 295 of them produce obligations, for 917 queries; the case added last (`two-edit-call-in-loop`) adds 7 more.

## 1. Soundness: no conflation, no split

`compare2.py` recomputes, independently from each old (`81cf9a8`) query, the name the PR should give each tuple sort. The old projection declarations `i!t!<k>!<old> (<old>) <sort_k>` give the component sorts. The new name is `i!Tuple<n>!` followed by the first 32 hex digits of SHA-256 over each component's sort and a newline, applied recursively. The script then substitutes the names into the old query and compares it with the new one.

| Queries | Result |
| --- | --- |
| 847 without tuple sorts | byte-identical |
| 70 with tuple sorts (plus 3 from `two-edit-call-in-loop`) | 66 identical after renaming; 7 identical up to the order of `declare-` lines |

- **Why the 7 reorder:** declarations are emitted sorted by name, and the new names sort differently. No assertion changed.
- **No split, no conflation:** there are 17 distinct tuple sorts in the old queries and 17 in the new ones, related one to one. No sort gets two names, and no two sorts share one.
- **Name shape:** every new name has the form `i!Tuple<n>!<32 hex>`.

### Adversarial packages (`mkcases.py`)

| Case | What it exercises | Obligations |
| --- | --- | --- |
| `order-and-arity` | (Nat, Text) against (Text, Nat); 2 against 3 `Nat` components; an unprovable postcondition | 18 proven, 1 unknown |
| `nested-loops` | an inner loop's state inside an outer loop | 7 proven |
| `refined-labeled` | a refined `Pos` component, a plain one, and a `secret Int` one | 10 proven |
| `kind-names` | user kinds `Tuple2` and `Int8` in modules `a_b` and `a.b`, whose sorts must stay distinct | 15 proven |
| `generic-twice` | a generic `repeat[T]` with a loop, called at `Int` and at `Text`; `Box[Int]`, `Box[Text]`, `List[Int]` and `Option[Text]` components | 6 proven |
| `wide-60` | 60 components: old query 61,091 bytes, new 23,417 | 121 proven |
| `two-edit-call-in-loop` | a call with two `edit` parameters inside a loop | 3 tuple queries, identical after renaming |

### Analysis

- **The digest's input is injective.** Each component sort is followed by a newline, and no sort can contain one, while the arity is in the name. Component sorts are unambiguous in their own right:
  - prelude and user-kind sorts spell their arguments in prefix form with a fixed arity;
  - `kind_sort` doubles `_` inside module segments before joining them with `_`;
  - kind names have no `_` (U1005);
  - Una names cannot contain `!` (U1005);
  - user sorts carry the `u!` prefix, encoder sorts `i!`.
- **Nested tuples are not reachable from source.** A loop's state components are bindings, and no binding has tuple type. No corpus query projects a tuple out of a tuple. The PR's unit test and the recursive recomputation above cover the case anyway.
- **A collision would fail, not mislead.** If two distinct sorts ever shared a name, Z3 would reject the duplicate `declare-sort` or `declare-fun` (no check passes), or the projections' result sorts would clash. A shared name cannot silently prove a false goal.
- **Erased distinctions predate this PR.** Sort erasure (refinements, labels and fixed widths become `Int`) already gives some different types the same tuple sort in the old naming. The new naming keeps exactly the same distinctions.

## 2. Verdicts: identical, with cold caches

`verdicts2.py` ran `una --json obligations` on all 1,042 programs, per binary, with a fresh HOME and therefore a fresh proof-cache key. It first deleted every `.una` cache directory under the corpus.

| Run | Result |
| --- | --- |
| 81cf9a8 | 923 obligations: 653 proven, 259 refuted, 11 unknown |
| 169b880 | the same |
| Differences in id, status or exit code | 0 programs |

Each binary got 120 cache hits within its own run. They come from single-file conformance cases that share one directory's cache, and the count is the same for both binaries.

## 3. una-bench: lock pins and proof cache

- **Bench packages:** `una --json check` and `una --json test` on all 150 una-bench packages (48 tasks as reference, stub and scaffold, P1, P4 and P7, and the 3 Suite D bases), compared with the Gate 1 pin `v0.1.3`:
  - byte-identical;
  - no U7004, since `encoding_version una-smt-1` is accepted;
  - the bench oracle is identical on 48/48 tasks (571/571 hidden cases, same traces and predicates).
- **No stale cache hit across the upgrade (`stale.py`).** The cache was warmed with 81cf9a8, and 169b880 then ran with the same key. Of the 274 obligations of the 18 programs whose queries contain tuple sorts:
  - the 73 with a tuple sort got **0 hits**, so they were re-proved;
  - the 201 without one all hit, since their queries did not change.

  That matches the PR's statement that the cache key hashes the whole query. The verdicts were equal to the cold run, and a third run hit all 923 obligations.
- **Warm cache on a bench task (A12, edition 0.1, 122 obligations):** the first run is all proven with 0 hits, the second all proven with 122 hits, and the `check` output is identical.

## 4. Determinism

- **Two runs on macOS:** all 917 queries are byte-identical.
- **macOS against Linux:** `linux-dump.sh` built the same tree in the local-CI image (`linux/amd64`, emulated) with its own target volume, and dumped the same corpus. All 924 queries of the 296 programs are **byte-identical** to the macOS dump. The queries hold no host path, so this is a byte-for-byte comparison.

## Notes, not findings

- The fixed-length names make queries linear, as the PR states. The largest query, `wide-60`, shrank from 61 KB to 23 KB.
- Not bumping `encoding_version` is consistent with what was observed: no U7004, no stale hit, and identical verdicts.

## Files

| File | Contents |
| --- | --- |
| `dumpq.rs` | the query dump (added to the reviewer's scratch copies only) |
| `mkcases.py` | the adversarial packages |
| `compare2.py` | the renaming comparison |
| `verdicts.py`, `verdicts2.py` | the verdict runs |
| `stale.py` | the cache test |
| `linux-dump.sh` | the Linux dump |

They need `Z3_DIR`, and the two built trees next to them.
