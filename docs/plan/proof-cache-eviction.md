# Proof cache eviction (design note)

- Issue: #1479 (CV-16 of the performance scan of 2026-11-02, area `una-verify`).
- Decision of 2026-11-03 on the card: the proof cache stays, and old entries are evicted so that `.una/cache/proofs` does not grow without limit, bounded by size and by age, deterministic and safe, keeping the keyed HMAC tags and the properties of ADR 0064, safe under concurrent `una` processes, and measured.
- Records read: ADR 0041 and its amendment, ADR 0064 and its amendment of 2026-10-25, ADR 0063 rule L5, ADR 0122 rules 30 and 48, ADR 0124 rule 10, `spec/12 §9` and `§10`, `spec/13 §5`, `§6` and `§7`, `spec/30 §2` and `§7`, `docs/security/tcb.md §3.10`.
- Code read: `crates/una-verify/src/cache.rs`, `crates/una-verify/src/run.rs` (the three call sites), `crates/una-cli/src/proof_key.rs`, `crates/una-cli/src/verification.rs`, `crates/una-diag/src/safe_write.rs`, `crates/una-project/src/tree.rs` (the `O_NOFOLLOW` table and the advisory lock of Itaoca).
- Scope: the toolchain only. No construct of the language, no query text and no cache key changes. This note is the design; the implementation is a later card, after the record changes of section 10 are accepted.

## 1. The cache today

### 1.1 Files

- **Directory.** `<root>/.una/cache/proofs/`, flat, built in `verification.rs:164-170` only for a package root and only when the user's key resolved (`proof_key.rs`). In-process callers and single files run without a cache. The shared store `~/.una/itaoca/proofs/` of ADR 0122 rule 48 is not implemented in this checkout.
- **Entries.** One file per query, `<key>.json`, where `<key>` is 64 lowercase hex digits. The JSON is `{"answer": <Answer>, "tag": "<hex HMAC>"}` (`cache.rs:55-59`). A `proven` answer is a few hundred bytes; a `refuted` one carries its model values and can be larger. A reader refuses anything over 64 KiB (`MAXIMUM_ENTRY`, `cache.rs:21`).
- **Temporaries.** A write creates `.<key>.json.<pid>.<serial>.una-tmp` in the same directory (`safe_write::unique_name`) and renames it onto `<key>.json`. A process that dies between the two leaves the temporary behind. Nothing removes it: `safe_write::sweep_stale_private_dirs` (#1359) removes only private directories, never files.

### 1.2 Keys and tags

- **Key** (`cache.rs:76-99`): SHA-256, in hex, over the whole `verification.lock` text, NUL, the solver build digest (ADR 0124 rule 10), NUL, and the exact script for editions `0.1` to `0.3`; for edition `0.4` (ADR 0122 rule 48, `spec/30 §7`), the domain tag `una-proof-cache/2` followed by the lock's `solver` section, the build digest and the script, each length-prefixed. The script includes the preamble, so the key covers the seed, the step budget, the logic and every assertion.
- **Tag** (`cache.rs:177-185`): HMAC-SHA-256 under the user's secret key over the key, a NUL byte and the answer serialized again. `get` recomputes the tag from the answer it parsed and compares it in constant time (`cache.rs:103-117`). An entry whose tag fails is a miss: the query is solved and the entry is rewritten.
- **Key file** (`proof_key.rs`, ADR 0064 amendment of 2026-10-25): `UNA_PROOF_KEY` or `~/.una/proof-cache.key`, used only when it is a private regular file of the user with one link and 16 to 4096 bytes. The key file, not the cache, is in the Trusted zone.

### 1.3 Reads and writes

- **Reads** (`get`, `read_entry`, `cache.rs:103-135`): `symlink_metadata` on the name must say regular file; the file is opened, its metadata must again say regular file; at most 64 KiB + 1 bytes are read; UTF-8, JSON and the tag must pass. Any failure is a miss.
- **Writes** (`put`, `cache.rs:144-173`): never for `unknown` with reason `wallclock`, `cancelled`, `cpu`, `memout` or `crash`. The directory chain is created by `safe_write::create_dir_inside`, which refuses a level that leads outside the package root (#1160). The entry is written by `safe_write::write_new_name`: a fresh temporary created with `O_CREAT|O_EXCL`, filled, renamed onto the entry's name, which replaces a link at that name rather than following it. No `fsync`: a torn entry fails its tag.
- **Call sites** (`run.rs`): every query sent is looked up first and stored after a miss: the first query (`run.rs:717-750`), the second query of ADR 0053/0054 (`run.rs:767-808`), and the single queries of the retry and pinned paths (`run.rs:860-875`).

### 1.4 Atomicity and concurrency

- A name always holds a complete entry or nothing: the only mutation is `rename(2)` onto it. A reader that opened the old file keeps reading the old bytes.
- Processes share the directory without any lock: the CLI, several CLIs, and the language server, which runs tier 1 through the cache on every save (ADR 0063 rule L5). Two writers of the same key race on the rename; the last one wins, and both wrote the answer of the same query.
- The cache is a performance device by specification: statuses, counterexamples, diagnostics and exit codes do not depend on it (ADR 0064 rule 6, `spec/12 §10`). Only `obligations.cached`, the `(cached)` marks of `una obligations` and timing do.

### 1.5 Existing limits

| Limit | Value | Where |
| --- | --- | --- |
| Size of one entry read | 64 KiB | `cache.rs:21` |
| Size of a key | at least 16 bytes | `cache.rs:18`, `proof_key.rs` |
| Size of the key file | 16 to 4096 bytes | `proof_key.rs` |
| Number of entries | none | |
| Total bytes | none | |
| Age of an entry | none | |
| Crash temporaries | never removed | |

Every edit makes new keys for the obligations it touches, every save in the editor stores them, and every solver change (ADR 0124 rule 10) and lock change makes a fresh set. Nothing removes the old ones, so the directory grows with the history of the package, one small file per query ever solved (CV-16).

## 2. Requirements

1. **Bounded.** The bytes `una` stores under `.una/cache/proofs/` stay under a maximum, and an entry unused for longer than a maximum age is removed by a later run.
2. **No scan on the hot path.** A lookup and a store cost O(1) file-system operations amortized. No run lists the entries of the whole cache.
3. **Never a wrong answer.** Eviction only removes names. The only effect it can have on a run is a miss, and a miss is a solve.
4. **Concurrent processes.** No lock is needed for correctness, and none is held across a solve.
5. **ADR 0064 intact.** Keys, tags, key-file checks, the regular-file and size checks of reads, the link-safe writes.
6. **Output intact.** Only what ADR 0064 rule 6 already lets depend on the cache may change.

## 3. Options

Notation: n entries in the cache, w queries in one run (its working set), B the byte budget, G the number of generation directories (option C), P concurrent processes.

### Option A: end-of-run mark and sweep (the CV-16 proposal)

At the end of a complete tier-1 run, remove every entry whose key the run did not compute.

- **Time.** O(1) per lookup and store, plus O(n) per run: a full listing and a removal per stale entry. The listing runs on every run, warm or cold, including every editor save.
- **Space.** O(w) keys in memory; the cache holds the last run's working set.
- **Failure modes.** Two processes on different branches, or the editor and the CLI on different edits, remove each other's entries on every run, so neither stays warm. A partial run (cancelled by the editor, ADR 0063 rule 4) cannot sweep. The shared store of ADR 0122 rule 48 serves many roots, and a sweep by one root would remove the others' entries, so the option does not extend to it.
- **Security.** Removal by listing needs the same link care as option C. No new file is parsed.
- **Bound.** Neither size nor age: the bound is "what the last run used", which a large program can still make large, and a short run makes small.
- **Verdict.** Rejected: an O(n) scan on every run, and concurrent users defeat it.

### Option B: last-use times and a batched scan

On a hit, set the entry's modification time to now (`File::set_modified`). When an estimate of the size passes the budget, list the cache, sort by time and remove the oldest until a low-water mark is reached.

- **Time.** A hit costs an extra metadata write. The sweep is O(n) listing and stat calls plus O(n log n) to sort (O(n) with selection). Triggered only after Θ(n) new writes, it is O(1) amortized per store, but one run pays it all at once: a pause of O(n) system calls in the middle of a command, an editor save included.
- **Space.** O(n) in memory during a sweep. On disk, within the budget between sweeps plus what is written before the next trigger.
- **Failure modes.** The trigger needs a size estimate without a scan, so this option needs the ledger of option C anyway. Two processes can sweep at once; without an advisory lock they double the work, and with one they need lock files and a timeout. A file system mounted read-only or with restricted timestamps makes every hit a failing write. A planted entry with a far-future time is never evicted first.
- **Security.** A metadata write on the read path widens what a read does to the package directory. Removal needs the same link care as option C.
- **Bound.** Exact least-recently-used order, size and age.
- **Verdict.** Rejected: the cost lands as a pause on one unlucky command, the read path writes, and it needs the ledger and a lock on top.

### Option C: generation directories with an append-only byte ledger

Entries live in numbered generation directories. New entries go to the newest. When the newest has received its share of the budget, or has lived its share of the maximum age, a new generation starts, and the oldest is removed whole. A hit in an older generation moves the entry into the newest by one rename.

- **Time.** A store: the entry write as today, plus one `O_APPEND` write of at most 16 bytes to the generation's ledger and a check of its length. A lookup that hits the newest generation: as today. A lookup that hits an older one: up to G name lookups plus one rename. A miss: G name lookups, then the solve it already costs. Starting a run: one listing of the generations directory (at most a handful of names) and one `lstat` of the newest generation's `born` marker. Removing a generation costs O(entries in it), and each entry is removed once, so it is O(1) amortized per store. No operation lists the entries of more than the one generation being removed.
- **Space.** On disk, at most B plus a bounded overshoot (section 5.2). In memory, O(1).
- **Failure modes.** A working set larger than about (G - 1)/G of the budget misses cyclically, as any bounded cache does; that is a solve, never a wrong answer. A process that holds a stale idea of the newest generation writes at most one entry into an older one before it notices. A crash at any point leaves a state the next run cleans up or ignores (section 6.3). On a network file system, `O_APPEND` may not be atomic, so the ledger can undercount and the bound becomes approximate.
- **Security.** Nothing new is parsed but generation names, which are decimal numbers: the ledger is used only through its length, the `born` marker only through its modification time. Removal is name by name, restricted to the names the cache itself makes, never a recursive removal and never through a link (section 6.4).
- **Bound.** Size and age, both approximate by one generation: an entry is kept as long as it is used at least once per generation, and removed between one and about 1 + 1/G maximum ages after its last use (section 5.3).
- **Verdict.** Chosen.

### Option D: a compact index file

One file, `index`, holding a record per entry (key, size, last use), updated on every store and every hit as an append-only log, compacted when it grows past twice its live size; eviction picks the least recently used from the index.

- **Time.** O(1) amortized per access for the append, O(live records) per compaction, amortized O(1). Eviction in O(log n) per entry with an in-memory heap built at load, O(n) to load the index once per run.
- **Space.** O(n) on disk for the index and O(n) in memory per run to load it.
- **Failure modes.** Every hit writes. Compaction needs an exclusive lock across processes (the advisory lock of Itaoca's floor, `itaoca.rs:239-284`, shows the pattern and its five-second timeout). A torn tail after a crash needs recovery rules. A file that is in the directory but not in the index (a crash between the entry and its record, or a file put there by hand) is never evicted unless a reconciliation scan runs, which brings the O(n) scan back.
- **Security.** A new parsed file in a directory that anyone who can write the package can edit. A forged index can only cause misses or keep entries alive, but it is one more parser and one more format to fuzz.
- **Bound.** Exact least-recently-used order, size and age.
- **Verdict.** Rejected: an O(n) load per run, writes on the read path, a lock for compaction and a new parser, for an exact order that a performance device does not need.

### Comparison

| | A: mark and sweep | B: times and scan | C: generations | D: index file |
| --- | --- | --- | --- | --- |
| Lookup, hit | O(1) | O(1) + a metadata write | O(1); O(G) + 1 rename in an older generation | O(1) + an append |
| Store | O(1) | O(1) + ledger | O(1) + an append | O(1) + an append |
| Per run | O(n) scan | O(1); O(n log n) when triggered | O(G) listing | O(n) index load |
| Eviction, amortized per store | O(1) | O(1) (O(log n) with a full sort) | O(1) | O(1) |
| Largest single pause | O(n) every run | O(n) at a trigger | O(entries of one generation) | O(n) at compaction |
| Memory | O(w) | O(n) at a sweep | O(1) | O(n) |
| Needs a lock | no | yes, or doubled work | no | yes |
| New parsed file | no | no | no | yes |
| Size bound | no | yes | yes, approximate | yes |
| Age bound | no | yes | yes, approximate | yes |
| Order | last run only | exact LRU | LRU by generation | exact LRU |
| Concurrent users | defeat each other | correct | correct | correct |
| Extends to ADR 0122's shared store | no | yes | yes | yes |

## 4. Decision

**Option C: generation directories with an append-only byte ledger,** with G = 4 generations, a byte budget and a maximum age as the knobs of section 7. It is the only option with O(1) amortized work per access and per run, no lock, no new parser, and no pause longer than the removal of one generation. Its order is approximate, which costs at most an occasional re-solve, and the cache's correctness does not depend on its order at all.

## 5. The design

### 5.1 Layout

```text
.una/cache/proofs/
  gen/
    <n>/                      a generation: n in decimal, no leading zero, 0 to 2^32 - 1
      born                    empty; its modification time is when the generation started
      ledger                  one byte per started 4 KiB of each entry stored here
      <key>.json              entries, as today
      .<key>.json.<pid>.<serial>.una-tmp   temporaries of writes in flight
  migrated                    empty; present once the flat entries of before are removed
```

- The cache directory stays `.una/cache/proofs/`, as `spec/12 §9` names it, and `tcb.proof_cache` keeps naming `.una/cache/`.
- An entry's file, its JSON, its key and its tag are unchanged: only the directory it sits in changes.
- The generations sit one level down, in `gen/`, so that listing them never lists the flat entries of an older `una` (section 9).

### 5.2 Operations

Let B be the byte budget, A the maximum age, G = 4, and the share of a generation B/G bytes and A/G of time. The unit of accounting is 4 KiB, the allocation block of APFS and of ext4 by default, so that the ledger tracks disk use rather than JSON length.

1. **Opening, once per run** (each `una check`, and each check the language server runs).
   - Resolve `.una/cache/proofs/gen` with `create_dir_inside` (#1160): a level that leads outside the package root turns the cache off for the run, as a write does today.
   - List `gen/`. A name is a generation only when it parses as decimal of the form above and `symlink_metadata` says directory. Every other name is ignored and never followed. Let `cur` be the largest generation; when there is none, create `gen/0`.
   - Read the modification time of `cur/born` with `symlink_metadata`. When it is missing, create it (`create_new`) and take now. When now minus it is at least A/G, rotate (step 4).
   - Remove every generation n with n <= cur - G, and every generation n < cur whose successor's `born` is older than A (step 5). The successor's birth is when n stopped receiving new entries.
   - When `migrated` is absent, run one batch of the migration (section 9).
2. **Lookup** of a key: try `cur/<key>.json`, then `cur - 1`, down to `cur - G + 1`, with `read_entry` and the tag check of today. The first entry whose tag passes is the answer. When it came from an older generation, rename it to `cur/<key>.json` and append its units to `cur/ledger` (a promotion). A failed promotion is ignored: the answer was already read and checked, so the run uses it.
3. **Store** of an answer: as today, into `cur`, with `write_new_name`. Then open `cur/ledger` with `O_WRONLY|O_APPEND|O_CREAT|O_NOFOLLOW|O_NONBLOCK`, mode 0600, check through the open descriptor that it is a regular file, append ceil(length / 4096) bytes (1 to 16), and read its length L from the descriptor. When L x 4096 >= B/G, rotate. When the store fails because `cur` is gone (another process removed it), open again (step 1) and keep going; the answer is stored at the next miss.
4. **Rotation** from `cur` to `cur + 1`: `mkdir gen/<cur + 1>`. The process whose `mkdir` succeeds creates `born` in it and runs step 5's removal of `cur + 1 - G`; a process whose `mkdir` fails with "already exists" adopts `cur + 1` and removes nothing. Within one process, the newest generation is held in one shared atomic value, so the parallel stores of a run rotate once. A process that still writes into an old generation sees that generation's ledger past its share at its next store, finds `cur + 1` with one `symlink_metadata`, and adopts it, so it overshoots by at most one entry per store in flight.
5. **Removal** of a generation: rename it to `gen/.removing-<n>.<pid>.<serial>` (a rename within the resolved `gen/`, so two removers never work on one name), then list it and remove, with `remove_file` (which unlinks a link without following it), only the names the cache makes: `<64 hex>.json`, the temporaries of `unique_name`, `ledger` and `born`. Then `remove_dir`. Anything else in it, which `una` did not write, makes `remove_dir` fail; the directory is left and ignored (it is not a generation name). A directory `.removing-*` left by a crash is removed by the next run's opening, under the same rules.

### 5.3 Bounds

- **Size.** At most G generations exist after an opening or a rotation, each accounted at most B/G plus the overshoot. The overshoot is at most one entry (64 KiB) per store in flight per process, and a run admits at most `min(cores, max(1, floor(M / (2 x L))))` solver children at once (ADR 0124 rule 6), so with P processes it is at most P x children x 64 KiB. The ledgers add 1 byte per 4 KiB accounted, and each generation one directory and the `born` file. The bound covers what `una` writes: files that someone puts into the cache by hand are not counted, and a generation holding one is not removed.
- **Age.** An entry used at time t is in the newest generation at t (stored there, or promoted there). That generation stops receiving entries no later than t + A/G at the next run, and is removed once its successor's birth is older than A. So an entry unused since t is removed by the first run after about t + A + A/G, and kept at least until t + A unless the size bound removes it first. An entry used at least once per generation is never removed by age.
- **Working set.** A run whose obligations fit in (G - 1)/G x B keeps them across rotations by promotion: each rotation moves the working set once, one rename per entry, which is O(1) amortized per store since a rotation follows B/G bytes of new entries or A/G of time.

### 5.4 Cost per run, warm and cold

- Warm run, every answer in `cur`: one listing of `gen/` (at most G + a few names), one `symlink_metadata` of `born`, and the reads of today. No write.
- Warm run just after a rotation: each answer is found in `cur - 1` after one failed name lookup and moved with one rename and one append.
- Cold run: G failed lookups per query (each a `symlink_metadata` that fails with "not found", microseconds), the solve, the store and one append. The solve dominates by orders of magnitude.

## 6. Safety invariants

### 6.1 Eviction can only cause a re-solve

- Eviction removes or renames names; it never writes into an entry. An entry is an answer only when its tag, computed from the bytes read under the user's key, matches (ADR 0064 rule 1). So eviction can turn a hit into a miss and nothing else, and a miss is solved by the same query with the same lock.
- A promotion moves a file whose bytes were already read and checked. The tag binds the key and the answer, not the directory, so the moved entry is the same answer. If someone swaps the file for something else between the check and the rename, the moved name is checked again at its next read, like any entry.
- An entry whose tag fails is never promoted. It stays where it is and leaves with its generation, or, in `cur`, is replaced by the next store as today.

### 6.2 An entry being read is never lost

- A reader holds an open descriptor from `File::open` on. Removing or renaming the name does not affect the bytes read through it (POSIX `unlink` and `rename` semantics, on macOS and Linux, the platforms `una` serves). A reader that has not opened the file yet finds no name and has a miss.
- So a reader observes the whole entry or a miss, never part of one. No lock is needed, and none is held across a solve.

### 6.3 Crash safety

Every step leaves a state that is correct to read and that the next opening cleans up:

| Crash between | State left | Effect |
| --- | --- | --- |
| temporary created and renamed | a temporary in a generation | removed with its generation |
| entry renamed and ledger appended | an entry not counted | at most one per crash; removed with its generation |
| `mkdir` of a generation and its `born` | a generation without `born` | the next opening creates `born` |
| rename to `.removing-*` and the end of the removal | a partly removed directory | the next opening removes it |
| two migration batches | flat entries left, no `migrated` | the next opening runs the next batch |

The ledger is never parsed, only measured, so a torn or garbage ledger changes only when a rotation happens. No `fsync` is added: the cache stays best-effort, as `put` documents.

### 6.4 No link is followed (#1160, #1164)

- `gen/` and its parents are resolved by `create_dir_inside`; a level that leads outside the package root turns the cache off for the run, and nothing is listed, renamed or removed outside it.
- A name in `gen/` is a generation only when `symlink_metadata` says directory: a link with a generation's name is ignored. A link inside a generation is removed as a name (`unlink`), never followed. Removal never uses `remove_dir_all`.
- The ledger is opened with `O_NOFOLLOW` and `O_NONBLOCK`, so a link or a FIFO swapped in at its name is refused, not written through or waited on, and then checked to be a regular file through its descriptor. `una-project/src/tree.rs:56-131` already holds the flag values for macOS and Linux; the implementation moves that table to `una-diag::safe_write`, which `una-verify` already depends on, rather than copying it. On a platform where the table has no values, no ledger is written and the cache stores nothing, as `tree.rs` refuses every write there.
- `born` and `migrated` are created with `create_new` (`O_EXCL`, which never follows a link) and read only with `symlink_metadata`.
- Entries are still written by `write_new_name` and read by `read_entry`, unchanged.
- A package that plants files in `.una/` (a cloned repository, ADR 0064's Context) can, at most: plant a generation with a large number, which makes the real ones look old and removes them (misses); plant a large ledger, which forces rotations (misses); plant entries, which fail their tags (misses); plant files a removal does not recognize, which keep their directory (not counted by the bound). A generation number at 2^32 - 1 has no successor: the cache stops rotating and is bounded by the age rule only, until the directory is removed. None of these can make an answer wrong or make `una` write outside the package.

### 6.5 ADR 0064 and its amendment

Unchanged: the key and its domain tag, the tag under the user's key, the constant-time compare, the key-file checks of `proof_key.rs`, the 64 KiB regular-file reads, the writes through a fresh temporary renamed onto the entry's own name, and the answers that are never stored. Without a key there is no cache, so no generation is opened, rotated or removed (ADR 0064 rule 3: nothing is written).

### 6.6 Output

- Eviction prints nothing, never on stderr under `--json` (`spec/13 §5`), and never changes an exit code: every failure of opening, rotation, promotion or removal is treated as "no change" and the run continues.
- Statuses, counterexamples, diagnostics and exit codes do not depend on the cache (ADR 0064 rule 6, `spec/12 §10`). What may change is what already depends on it: `obligations.cached`, the `(cached)` marks of `una obligations`, the language server's proof counts that come from those, and timing.
- The policy is deterministic for one sequence of runs and one clock: the same accesses at the same times remove the same generations. Concurrent processes interleave, which, as today, changes only which run finds an entry first.
- The clock is read once per opening. A clock that moves back makes nothing older (no rotation, no removal by age); one that jumps forward removes generations early (misses).

## 7. Knobs

### 7.1 Defaults

| Knob | Default | Range | Meaning |
| --- | --- | --- | --- |
| Maximum bytes | 512 MiB | 16 MiB to 65,536 MiB | B, the accounted bytes of `.una/cache/proofs/` |
| Maximum age | 30 days | 1 to 3,650 days | A, the age after which an unused entry is removed |
| Generations | 4 | fixed | G, not a knob |

- **Why 512 MiB.** At one 4 KiB block per entry, it holds about 131,000 entries, so a program with 20,000 obligations keeps several edits' worth of history and every branch in use. The example packages use well under 1 MiB.
- **Why 30 days.** Every warm run promotes what it uses, so active work never ages out. A branch untouched for a month verifies once again on its first check, as after a solver change (ADR 0124 rule 10).
- **Why bounded ranges.** A cloned repository chooses its project file: the lower bound keeps a hostile value from turning the cache into a re-solve on every run of a large program, and the upper bound keeps it from asking for an unbounded store.

### 7.2 Where they are set

The project files are closed: an unknown table of `una.toml` is U7002 (`spec/13 §7`) and an unknown section of `ita.una` is U7008 (`spec/30 §2` rule 3). So any knob in a project file changes the specification and needs the record change of section 10.

- **Edition `0.4`, `ita.una`.** A new last section in the production `section` of `spec/30 §2`:

  ```text
  cache
    proofs at most 512 mib
    proofs unused for at most 30 days
  ```

  as `cache = "cache" NEWLINE INDENT [ "proofs" "at" "most" int_lit "mib" NEWLINE ] [ "proofs" "unused" "for" "at" "most" int_lit "days" NEWLINE ] DEDENT ;`, each line at most once and in that order, a value outside its range U7008 at the line, and the canonical form of `spec/30 §2` rule 6.
- **Editions `0.1` and `0.2`.** No knob: their stability promise (`spec/13 §7`) says a later release must not change what they accept, and a new `una.toml` table would be accepted where it is U7002 today. They use the defaults.
- **Edition `0.3`.** No knob either, by the same reasoning applied to its preview terms, which only let it get stricter. It uses the defaults.
- **A single file** has no cache today and gets none.
- The knobs change no query and no key, so they need no edition gate of their own: a package of edition `0.4` checked by two releases with different defaults is bounded by whichever ran last.

### 7.3 Pruning now

`spec/13 §5` defines no cache command, and the CLAUDE.md scope guard forbids adding one without the specification. The specification already gives the way to empty the cache: "Removing `.una/` verifies every obligation again" (`spec/12 §9`), so `rm -r .una/cache/proofs` prunes everything now, and eviction makes the routine case automatic.

A command that applies the budget immediately, without emptying the cache, would fit the conventions of `spec/13 §5` as `una cache prune [path]`, printing one `una.cache/1` document under `--json` (`schema`, `removed`, `kept_bytes`) and exiting 2 on a usage error. It **needs an amendment** of `spec/13 §5`, and this design does not propose it for the implementation card: nothing requires it, and section 11 asks whether it is wanted.

## 8. How to measure

The cache gains a counter record, `CacheStats`, returned to its caller and asserted by tests, not printed: lookups, hits by generation age (0 to G - 1), misses, stores, appends, promotions, rotations, generations removed, entries removed, names listed. The counters make the complexity claims testable without wall-clock timing.

1. **Counter-based complexity test (scaling).** Fill a cache to 1,000, 10,000 and 100,000 entries, then run 1,000 lookups and 1,000 stores at each size. Assert that names listed per run is at most G + 4 plus the entries of the generations removed in that run, that the total entries removed never exceed the total stored, and that lookups, appends and promotions per access are the same constants at every size. A quadratic or scanning implementation fails at 100,000.
2. **Bound test.** With a test-only constructor that takes B = 1 MiB and A, store entries of mixed sizes (300 bytes to 64 KiB) until 10 x B has been written. After every store, walk the directory in the test (the test may scan) and assert: at most G generations; the sum of the entries' lengths rounded up to 4 KiB at most B plus the overshoot bound for one process; no file outside the layout of section 5.1.
3. **Age test.** With an injected clock (the opening takes `now`) and `born` times set with `File::set_modified`: an entry stored at t and never used is present at t + A - 1 day and gone at the first opening after t + A + A/G; an entry used once per A/G survives 10 x A.
4. **Concurrency test.** Spawn 8 processes (the test binary re-executed, or `una check` on generated packages) that share one cache with B = 1 MiB, each doing 2,000 random lookups and stores over 4,000 keys, with answers derived from the key so a wrong answer is detectable. Assert: every hit returns the answer of its own key; no process fails or prints; the bound of test 2 holds within the overshoot for 8 processes at the end and at sampled points; no `.removing-*` and no temporaries remain after a final opening.
5. **Crash tests.** For each row of section 6.3, build the state by hand and assert that the next opening leaves a valid layout and that lookups of untouched keys still hit.
6. **Link and planted-file tests,** next to `no_entry_is_stored_through_a_link_leading_outside_the_package_1160` and `an_entry_is_read_only_from_a_small_regular_file_and_never_written_through_a_link`: `gen` a link outside the root (cache off, nothing outside touched); a generation name that is a link (ignored, target untouched); a link inside a generation pointing outside (unlinked, target intact); `ledger` a link or a FIFO (refused, no hang); a planted generation 2^32 - 1; a planted file the removal does not know (directory kept, everything else removed).
7. **Output test.** `una check --json` and `una obligations` on the example packages, twice, with and without a forced rotation and removal between the runs: identical documents except `obligations.cached` and the `(cached)` marks, and stderr empty under `--json`.
8. **Existing tests kept.** `a_second_run_reports_every_obligation_as_cached`, `a_forged_or_foreign_cache_entry_is_solved_again`, `a_damaged_cache_entry_is_solved_again_and_never_reused_11`, `an_unknown_obligation_is_an_error_every_run_and_only_a_wallclock_unknown_is_not_cached`, the `cache.rs` unit tests and `lsp.rs`'s cache case, with paths updated to the generation layout.
9. **Measurement for the PR.** Wall time and `obligations.cached` of `una check --json`, cold (after `rm -r .una`) and warm, on the example packages and on a package with dependencies, before and after the change, five runs each, in a release build. The warm figure must not regress beyond noise; the expected cost is one small listing and one `lstat` per run.

## 9. Migration

- A cache of an earlier `una` is flat: `<key>.json` and temporaries directly in `.una/cache/proofs/`. The new layout never reads them; its first run starts with an empty `gen/0` and verifies everything once, as after ADR 0064 and after a solver change under ADR 0124 rule 10.
- The flat files are removed in batches: while `migrated` is absent, each opening lists `.una/cache/proofs/` and removes, under the rules of section 5.2 step 5, at most 4,096 names that are a flat entry or a temporary, regular files or links only. When a listing finds none, it creates `migrated`. Each run's share of the migration is O(4,096), and the whole migration is O(n) once per package.
- Moving the flat entries into `gen/0` instead would keep them warm, but a large old cache (the case this card exists for) would fill `gen/0` past its share at once and be removed by rotation anyway. Removing them is simpler and the same in effect.
- An older `una` run on a migrated package writes flat entries again, which the newer one never reads; it does not see `migrated` and does not mind `gen/`. Running old and new releases alternately therefore costs re-solves, never a wrong answer, and the flat entries of the old release are unbounded as before. The next run of a newer release removes nothing more, since `migrated` exists: that leftover is a stated limitation, removable with `rm -r .una`.

## 10. Records and documents to amend

No ADR is edited by this note. Before the implementation, these need a dated amendment or text change, accepted as the project decides:

1. **ADR 0064**, dated amendment: the cache is bounded by size and age through generations; eviction only removes names; the layout; the knobs and defaults; the safety rules of section 6. It amends `spec/12 §9`.
2. **`spec/12 §9`**: the sentences on the layout ("Each entry is one JSON file named by the key in hex" stays, in a generation directory), eviction with its bounds and the defaults, and that an evicted entry is solved again.
3. **`spec/30 §2`** (and ADR 0122, dated amendment, since `spec/30` carries its rules): the `cache` section of `ita.una`, its grammar, its order, its checks (U7008) and its canonical form.
4. **ADR 0122 rule 30 and rule 48**, the same dated amendment: when the shared store `~/.una/itaoca/proofs/` lands, it uses the same generations; its knobs, a per-user store's, cannot come from a package's `ita.una` (section 11, question 3).
5. **`spec/13 §5`**, only if the prune command of section 7.3 is wanted.
6. **`docs/security/tcb.md §3.10`** (not a record): the guards of eviction and the planted-file cases of section 6.4.
7. Not amended: ADR 0041 (rule 2's `cached` count is unchanged), ADR 0063 rule L5 (the editor still shares the cache), ADR 0124 rule 10 (the key is unchanged), `spec/13 §6` (`tcb.proof_cache` still names `.una/cache/`; no report field is added), `spec/12 §10`.

The implementation card then carries the changelog fragment; this note, like the earlier design notes in `docs/plan/`, adds none.

## 11. Open questions

1. **Defaults.** 512 MiB and 30 days with 4 generations, or other values?
2. **Editions `0.1` to `0.3` without a knob.** This note reads their stability terms as forbidding a new `una.toml` table, so they get the defaults only. Is that the intended reading, or should edition `0.3`'s preview admit `[cache]`?
3. **Project file or per user.** The cache is on the user's disk, while `ita.una` is chosen by whoever wrote the package. Should the knobs also, or instead, be settable per user (for example, variables beside `UNA_PROOF_KEY`), which would also serve the shared store of ADR 0122, and which wins when both are set?
4. **A prune command.** Is `una cache prune [path]` (section 7.3) wanted, given `rm -r .una/cache/proofs` and automatic eviction?
5. **Migration.** Is one re-verification of every package after the upgrade acceptable, as after ADR 0064 and ADR 0124 rule 10, rather than moving the flat entries into the first generation?
