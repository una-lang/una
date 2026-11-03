# Proof cache eviction (design note)

- Issue: #1479 (CV-16 of the performance scan of 2026-11-02, area `una-verify`).
- Decision of 2026-11-03 on the card: the proof cache stays, and old entries are evicted so that `.una/cache/proofs` does not grow without limit, bounded by size and by age, deterministic and safe, keeping the keyed HMAC tags and the properties of ADR 0064, safe under concurrent `una` processes, and measured.
- Records read: ADR 0041 and its amendment, ADR 0064 and its amendment of 2026-10-25, ADR 0063 rule L5, ADR 0121 rules 6, 9, 10 and 13, ADR 0122 rules 30 and 48, ADR 0124 rule 10, `spec/12 §9` and `§10`, `spec/13 §5`, `§6` and `§7`, `spec/30 §2` and `§7`, `docs/security/tcb.md §3.10`.
- Code read: `crates/una-verify/src/cache.rs`, `crates/una-verify/src/run.rs` (the three call sites), `crates/una-cli/src/proof_key.rs`, `crates/una-cli/src/verification.rs`, `crates/una-diag/src/safe_write.rs`, `crates/una-project/src/tree.rs` (the `O_NOFOLLOW` table and the advisory lock of Itaoca).
- Scope: the toolchain only. No construct of the language, no query text and no cache key changes. This note is the design; the implementation is a later card, after the record changes of section 10 are accepted. It depends on descriptor-relative file operations that are not on `main`: `una-sys` has no `dir` module today, the work is #1094, re-filed as #1275 and parked (+204 lines over the cap of ADR 0058), and the design needs a directory listing by descriptor that ADR 0058 does not admit, or a checked listing by path instead (section 6.4). **The implementation card is blocked until that dependency lands with the ADR 0058 amendment of section 10 item 9;** until then the cache behaves as today (section 6.4, "Until the dependency lands").

## 1. The cache today

### 1.1 Files

- **Directory.** `<root>/.una/cache/proofs/`, flat, built in `verification.rs:164-170` only for a package root and only when the user's key resolved (`proof_key.rs`). In-process callers and single files run without a cache. The shared store `~/.una/itaoca/proofs/` of ADR 0122 rule 48 is not implemented in this checkout.
- **Entries.** One file per query, `<key>.json`, where `<key>` is 64 lowercase hex digits. The JSON is `{"answer": <Answer>, "tag": "<hex HMAC>"}` (`cache.rs:55-59`). A `proven` answer is a few hundred bytes; a `refuted` one carries its model values and can be larger. A reader refuses anything over 64 KiB (`MAXIMUM_ENTRY`, `cache.rs:21`).
- **Temporaries.** A write creates `.<key>.json.<pid>.<serial>.una-tmp` in the same directory (`safe_write::unique_name`) and renames it onto `<key>.json`. `write_new_name` removes the temporary only on an error it sees (`safe_write.rs:86-90`); a process that dies between the two leaves it behind. Nothing removes it: `safe_write::sweep_stale_private_dirs` (#1359) removes only private directories, never files.

### 1.2 Keys and tags

- **Key** (`cache.rs:76-99`): SHA-256, in hex, over the whole `verification.lock` text, NUL, the solver build digest (ADR 0124 rule 10), NUL, and the exact script for editions `0.1` to `0.3`; for edition `0.4`, the domain tag `una-proof-cache/2` followed by the lock's `solver` section, the build digest and the script, each length-prefixed, as `spec/12 §9` states it. The script includes the preamble, so the key covers the seed, the step budget, the logic and every assertion.
- **A gap in the records.** `spec/30 §7` and ADR 0122 rule 48 define the edition `0.4` key over the domain tag, the `solver` section and the query only: they omit the build digest that `spec/12 §9` (ADR 0124 rule 10) and the code (`cache.rs:82-88`) put between the two. The code follows `spec/12 §9`; the text correction is section 10 item 7. It is independent of eviction.
- **Tag** (`cache.rs:177-185`): HMAC-SHA-256 under the user's secret key over the key, a NUL byte and the answer serialized again. `get` recomputes the tag from the answer it parsed and compares it in constant time (`cache.rs:103-117`). An entry whose tag fails is a miss: the query is solved and the entry is rewritten.
- **Key file** (`proof_key.rs`, ADR 0064 amendment of 2026-10-25): `UNA_PROOF_KEY` or `~/.una/proof-cache.key`, used only when it is a private regular file of the user with one link and 16 to 4096 bytes. The key file, not the cache, is in the Trusted zone.

### 1.3 Reads and writes

- **Reads** (`get`, `read_entry`, `cache.rs:103-135`): `symlink_metadata` on the name must say regular file; the file is opened, its metadata must again say regular file; at most 64 KiB + 1 bytes are read; UTF-8, JSON and the tag must pass. Any failure is a miss.
- **Writes** (`put`, `cache.rs:144-173`): never for `unknown` with reason `wallclock`, `cancelled`, `cpu`, `memout` or `crash`. No size check: an entry over 64 KiB is written although no read accepts it. The directory chain is created by `safe_write::create_dir_inside`, which refuses a level that leads outside the package root (#1160) but follows a link at any level that resolves inside it (`safe_write.rs:330-390`). The entry is written by `safe_write::write_new_name`: a fresh temporary created with `O_CREAT|O_EXCL`, filled, renamed onto the entry's name, which replaces a link at that name rather than following it. No `fsync`: a torn entry fails its tag.
- **Call sites** (`run.rs`): every query sent is looked up first and stored after a miss: the first query (`run.rs:717-750`), the second query of ADR 0053/0054 (`run.rs:767-808`), and the single queries of the retry and pinned paths (`run.rs:860-875`).

### 1.4 Atomicity and concurrency

- A name always holds a complete entry or nothing: the only mutation is `rename(2)` onto it. A reader that opened the old file keeps reading the old bytes.
- Processes share the directory without any lock: the CLI, several CLIs, and the language server, which runs tier 1 through the cache on every save (ADR 0063 rule L5). Two writers of the same key race on the rename; the last one wins, and both wrote the answer of the same query.
- The cache is a performance device by specification: statuses, counterexamples, diagnostics and exit codes do not depend on it (ADR 0064 rule 6, `spec/12 §10`). Only `obligations.cached`, the `(cached)` marks of `una obligations` and timing do.

### 1.5 Existing limits

| Limit | Value | Where |
| --- | --- | --- |
| Size of one entry read | 64 KiB | `cache.rs:21` |
| Size of one entry written | none | `cache.rs:144-173` |
| Size of a key | at least 16 bytes | `cache.rs:18`, `proof_key.rs` |
| Size of the key file | 16 to 4096 bytes | `proof_key.rs` |
| Number of entries | none | |
| Total bytes | none | |
| Age of an entry | none | |
| Crash temporaries | never removed | |

Three gaps follow.

1. **Growth.** Every edit makes new keys for the obligations it touches, every save in the editor stores them, and every solver change (ADR 0124 rule 10) and lock change makes a fresh set. Nothing removes the old ones, so the directory grows with the history of the package, one small file per query ever solved (CV-16).
2. **Crash temporaries** are never removed.
3. **Entries over 64 KiB.** A `refuted` answer whose model values serialize to more than 64 KiB is stored on every run and never read back: a miss and a rewrite each time, disk used for nothing. The design closes it: `put` refuses a serialized entry over `MAXIMUM_ENTRY`, which the read would refuse anyway, so every stored entry is at most 64 KiB and the accounting of section 5.2 counts 1 to 16 units per entry.

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

Entries live in numbered generation directories. New entries go to the newest. When the newest has received its share of the budget, counting the entries moved into it, or has lived its share of the maximum age, a new generation starts, and the oldest is set aside by one rename and removed whole at the end of the run. A hit in an older generation moves the entry into the newest by one rename.

- **Time.** A store: the entry write as today, plus two `fstatat` calls that the generation is still the newest (it is there, and its successor is not), one `O_APPEND` write of at most 16 bytes to the generation's ledger and a check of its length. A lookup that hits the newest generation: as today. A lookup that hits an older one: up to G name lookups, one rename and the same ledger append and check as a store. A miss: G name lookups, then the solve it already costs. Starting a run: one listing of the generations directory (at most 512 names read, normally G or fewer) and one `fstatat` of the newest generation's `born` marker, plus, until the migration is done, a listing of at most 4,096 flat names (section 9). Removing a generation costs Θ(entries in it), about n/G, once per B/G bytes stored or per A/G of time, and each entry is removed once, so it is O(1) amortized per store; it runs at the end of the run, after the run's answers are out (section 5.2 step 5). Apart from the migration, no operation lists the entries of a generation other than one being removed.
- **Space.** On disk, at most B + B/G plus a bounded overshoot when no run is in progress, and during a run at most one more generation per rotation the running processes have made (section 5.3). In memory, O(1): the descriptors of `gen/` and of the newest generation.
- **Failure modes.** A working set larger than about (G - 1)/G of the budget misses cyclically, as any bounded cache does; that is a solve, never a wrong answer. A process that holds a stale idea of the newest generation writes at most one entry into an older one before it notices. A crash at any point leaves a state the next run cleans up or ignores (section 6.3). On a network file system, `O_APPEND` may not be atomic, so the ledger can undercount and the bound becomes approximate.
- **Security.** Nothing new is parsed but generation names, which are decimal numbers: the ledger is used only through its length, the `born` marker only through its type and modification time. Every operation works through directory descriptors opened with `O_NOFOLLOW` from the package root, so a link at any level, inside the package root or not, turns the cache off for the run. Removal is name by name, relative to the descriptor of the directory being removed, restricted to the names the cache itself makes, never a recursive removal (section 6.4).
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
| Per run | O(n) scan | O(1); O(n log n) when triggered | O(G) listing (at most 512 names) | O(n) index load |
| Eviction, amortized per store | O(1) | O(1) (O(log n) with a full sort) | O(1) | O(1) |
| Largest single pause | Θ(n) every run | Θ(n) at a trigger | Θ(n/G) once per B/G stored, after the run's answers | Θ(n) at compaction |
| Memory | O(w) | O(n) at a sweep | O(1) | O(n) |
| Needs a lock | no | yes, or doubled work | no | yes |
| New parsed file | no | no | no | yes |
| Size bound | no | yes | yes, approximate | yes |
| Age bound | no | yes | yes, approximate | yes |
| Order | last run only | exact LRU | LRU by generation | exact LRU |
| Concurrent users | defeat each other | correct | correct | correct |
| Extends to ADR 0122's shared store | no | yes | yes | yes |

## 4. Decision

**Option C: generation directories with an append-only byte ledger,** with G = 4 generations, a byte budget and a maximum age as the knobs of section 7. It is the only option with O(1) amortized work per access and per run, no lock, no new parser, no write on the read path of an entry already in the newest generation, no sort and O(1) memory. Its largest pause, the removal of one generation, is Θ(n/G): the same order as option B's sweep, smaller by the constant G, and taken after the run's answers are out rather than inside a query. That, not an asymptotic gain on the pause, is its advantage over B, with the absence of a read-path write, a sort, a lock and O(n) memory. Its order is approximate, which costs at most an occasional re-solve, and the cache's correctness does not depend on its order at all.

Incremental removal (a bounded number of names per store) would make the worst pause O(1), but a generation that holds names the cache did not write would then need a cursor kept across processes, or be set aside before its own entries are gone, which leaves bytes `una` wrote outside the bound. Removing a generation in one pass, at the end of a run, keeps the bound exact; incremental removal stays a possible refinement if the measurement of section 8 shows the pause matters.

## 5. The design

### 5.1 Layout

```text
.una/cache/proofs/
  gen/
    <n>/                      a generation: n in decimal, no leading zero, 0 to 2^32 - 1
      born                    empty; its modification time is when the generation started
      ledger                  one byte per started 4 KiB of each entry stored or moved here
      <key>.json              entries, as today
      .<key>.json.<pid>.<serial>.una-tmp   temporaries of writes in flight
    .removing-<n>.<pid>.<serial>/          a generation set aside, removed at the end of a run
  kept/                       at most 16 set-aside directories that held names `una` does not write
    <n>.<pid>.<serial>/
  migrated                    empty; present once the flat entries of before are removed
```

- The cache directory stays `.una/cache/proofs/`, as `spec/12 §9` names it, and `tcb.proof_cache` keeps naming `.una/cache/`.
- An entry's file, its JSON, its key and its tag are unchanged: only the directory it sits in changes.
- The generations sit one level down, in `gen/`, so that listing them never lists the flat entries of an older `una` (section 9). `kept/` is never listed by any run except to count its names (at most 17 read) before a move into it (section 5.2 step 5).

### 5.2 Operations

Let B be the byte budget, A the maximum age, G = 4, and the share of a generation B/G bytes and A/G of time. The unit of accounting is 4 KiB of entry length, rounded up per entry, the allocation block of APFS and of ext4 by default, so that the ledger tracks disk use rather than JSON length. On a file system with a larger block, or with compression, the bound is in these units, not in blocks used.

Every step works through directory descriptors (section 6.4): the package root is opened by its canonical path, and each of `.una`, `cache`, `proofs`, `gen` and a generation is opened relative to its parent with `O_NOFOLLOW|O_DIRECTORY`. Entries, `ledger`, `born` and `migrated` are opened, created, renamed and removed relative to the descriptor of their directory. A level that is a link, inside the package root or not, or not a directory, turns the cache off for the run: no lookup, no store, no eviction. A run holds two descriptors, `gen/` and the newest generation, in one value shared by its parallel queries under a mutex. The mutex covers only the check of step 3, the swap of the held generation and the set-aside renames of a rotation, each O(1) calls; no removal of step 5 ever runs inside a lookup or a store, or while it is held.

Every rename that sets a directory aside (into `.removing-*` or into `kept/`) is **exclusive**: `renameat2` with `RENAME_NOREPLACE` on Linux, `renameatx_np` with `RENAME_EXCL` on macOS, the functions the `dir.rs` row of ADR 0058 already lists. It never replaces what is at the target. "Target exists" (`EEXIST`, or `ENOTEMPTY` where a system reports that) and "source gone" (`ENOENT`) are told apart: on "target exists" the rename is tried again with the next serial, at most 16 times, after which the directory stays where it is and the next opening tries again; on "source gone" another process moved it first. A file system without an exclusive rename turns the cache off for the run, as ADR 0058 requires (no weaker fallback). A promotion and an entry's write keep `renameat` onto the entry's own name, which replaces it on purpose, as today.

The clock is read at every opening and every reopening of step 1, **after** the `fstatat` of `cur/born`, never reused from an earlier opening. Let τ = 1 hour, the clock tolerance. On one machine a file's modification time comes from a clock that never leads the one read after it (Linux stamps files from the coarse clock, which lags), so a `born` that a concurrent rotation created is never later than now; τ absorbs the skew of a network file system whose server dates files. τ is below the smallest share of time, A/G = 6 hours at the lower bound of A (section 7.1).

1. **Opening, once per run** (each `una check`, and each check the language server runs).
   - Open or create the chain down to `gen/` as above.
   - Read `gen/`, at most 512 names. More than 512 names, or more than 256 names that are generations, means a planted directory: the cache is off for the run. The two limits leave room for a restart (step 4), which turns at most 256 generations into as many `.removing-*` names and adds `gen/0`, so a crash during a restart does not turn the cache off. A name is a generation when it parses as decimal of the form of section 5.1 and `fstatat` with `AT_SYMLINK_NOFOLLOW` says directory; a name `.removing-*` that says directory is pending removal. Every other name is ignored and never followed. Let `cur` be the largest generation; when there is none, create `gen/0` (step 4's creation).
   - `fstatat` `cur/born` without following, then read the clock: that is now. When `born` is missing, create it (`O_CREAT|O_EXCL|O_NOFOLLOW`), and its time is now; when the create fails because another process created it, `fstatat` it again and read the clock again. **A `born` that is not a regular file, or whose modification time is later than now + τ, is expired.** A `born` later than now by at most τ counts as dated now. When `born` is expired, or now minus it is at least A/G, rotate (step 4).
   - Set aside (step 5) every generation n with n <= cur - G, and every generation n < cur for which the next existing generation above n has a `born` that is expired or older than A (judged with the same now and the same τ). That `born` is when n stopped receiving new entries; a generation whose successor is missing (removed, or a gap among planted names) is judged by the next one above it.
   - Remember the pending directories read for the end of the run (step 5).
   - When `migrated` is absent, run one batch of the migration (section 9).
2. **Lookup** of a key: try `cur/<key>.json`, then `cur - 1`, down to `cur - G + 1`, with the read checks of today (section 6.5). The first entry whose tag passes is the answer. When it came from an older generation, move it (a promotion): check that `cur` is still the newest, exactly as step 3 does, `renameat` it to `cur/<key>.json`, then append its units, ceil(bytes read / 4096), to `cur/ledger` and check the length exactly as a store does, rotating when the share is reached. A failed promotion is ignored: the answer was already read and checked, so the run uses it.
3. **Store** of an answer: refuse a serialized entry over 64 KiB (section 1.5, gap 3). Check that `cur` is still the newest, with two O(1) calls: `fstatat(gen, "<cur>", AT_SYMLINK_NOFOLLOW)` must give the device and inode of the held descriptor, and `fstatat(gen, "<s>", AT_SYMLINK_NOFOLLOW)`, where s is the successor of step 4 (`cur + 1`, or `0` when `cur` is 2^32 - 1), must say "not found". Otherwise open again (step 1, with a fresh clock) first. The second call is what makes the check mean "newest" rather than "still there": a rotation by another process creates s before it sets anything aside. Write the entry into `cur` as today, through a fresh temporary created with `O_CREAT|O_EXCL|O_NOFOLLOW` and renamed onto its name, relative to the descriptor. Then open `cur/ledger` with `O_WRONLY|O_APPEND|O_CREAT|O_NOFOLLOW|O_NONBLOCK`, mode 0600, check through the open descriptor that it is a regular file, append ceil(length / 4096) bytes (1 to 16), and read its length L from the descriptor. When L x 4096 >= B/G, rotate. **A ledger that cannot be opened, is not a regular file, or refuses the append means the generation is full: rotate.** When the store fails because `cur` is gone (another process set it aside), open again (step 1) and keep going; the answer is stored at the next miss.
4. **Rotation** from `cur` to its successor.
   - The successor is `cur + 1`. When `cur` is 2^32 - 1 it has none: the rotation is a **restart**, which sets aside every generation read in step 1 (at most 256 exclusive renames) and then creates `gen/0`. The restart's set-asides are removed at the end of the run, like every other (step 5). A planted generation with a huge number never makes the cache stop rotating.
   - The process whose `mkdirat` of the successor succeeds creates `born` in it, then sets aside `cur + 1 - G` (step 5). It removes nothing during the rotation: the set-aside generation joins the pending directories this run removes at its end. A process whose `mkdirat` fails with "already exists" reads `gen/` again (step 1, without the migration, with a fresh clock read after the `fstatat` of the new `born`) and adopts its newest generation; it sets aside and removes nothing more. With the clock read after the `fstatat` and the tolerance τ, the adopted `born` is never expired, so one rotation by one process is never turned into a second one by another.
   - Within one process the shared value holds the newest generation, so the parallel queries of a run rotate once. A query that still writes into an older generation is one that passed the check of step 3 just before the rotation: at most one entry per query in flight (section 5.3).
5. **Removal** of a generation, in two parts.
   - **Set aside:** rename it exclusively (above), within `gen/`, to `.removing-<n>.<pid>.<serial>`. One rename; it is no longer a generation, so no lookup or store finds it. "Source gone" means another process set it aside first; "target exists" (a leftover of a crash whose pid and serial repeat) retries with the next serial.
   - **Remove, only at the end of the run** (for `una check`, after the last query and after the report is written; for the language server, after the diagnostics of the check are published), never inside a lookup, a store or a rotation, and never under the mutex. For each pending directory read at the opening or set aside by this run, the restart's included: open it with `O_NOFOLLOW|O_DIRECTORY` relative to `gen/`, read at most 4,096 of its names, and `unlinkat` relative to its descriptor only the names the cache makes: `<64 hex>.json`, the temporaries of `unique_name`, `ledger` and `born`, each after `fstatat` without following says regular file or link. Then `unlinkat` the directory itself with `AT_REMOVEDIR`. When that fails because names remain, list it **once more**, at most 4,096 names again, remove the names the cache makes and try `AT_REMOVEDIR` once more; this second pass exists for the at most one entry per query in flight that was written after the first listing. There is no third pass:
     - when the second listing found a name the cache does not make, the directory holds names `una` did not write: rename it exclusively into `kept/` (created relative to `proofs/`) as `<n>.<pid>.<serial>`, but only when `kept/` holds fewer than 16 names (counted by reading at most 17); otherwise it stays pending;
     - otherwise (a writer keeps adding cache-shaped names, or a directory holds more than 4,096 names) it stays pending, and the next run that reads it tries again.
     A pending directory that stays counts toward the 512 names of step 1, so a planted state that keeps refilling `kept/` or `gen/` ends in a cache that is off (section 6.4), never in unbounded growth or an unbounded loop. Each run's removal reads at most 2 x 4,096 names per pending directory it knows.
   - Two processes may remove the same pending directory at once (the ends of two runs that both read it, for example a crash leftover); every step tolerates "not found", so the work is at most doubled and the result is the same.

### 5.3 Bounds

Let k be the number of queries in flight across all processes that share the cache: a run admits at most `min(cores, max(1, floor(M / (2 x L))))` solver children at once (ADR 0124 rule 6), so with P processes k is at most P times that. Let o = k x 64 KiB, the overshoot of one generation.

- **Size, live generations.** Every write into a generation, a store or a promotion, appends its units to that generation's ledger, and every writer that finds the ledger at or past its share, or refused, rotates or adopts the successor. A writer writes at most one entry into a generation after it is full (it passed the check of step 3 before the rotation). So a generation holds at most B/G + o accounted bytes while it is the newest, and only shrinks once it is not (promotions move entries out, stale writes are already in o). At most G generations exist after an opening or a rotation, so the live generations hold at most B + G x o.
- **Size, pending removal.** A set-aside generation held at most B/G + o accounted bytes, and a restart's set-asides only move bytes that were live, so they keep the sum of live and pending. Let R_p be the number of generations that a running process p has set aside and not yet removed (its rotations so far, a restart counted once with what it moved), and C the bytes of pending directories that crashed runs left, which the next run that reads them removes. Since removal happens only at the end of a run, at any moment the total is at most **B + G x o + Σ_p R_p x (B/G + o) + C**. When every running process has rotated at most once, this is **B + G x o + P x (B/G + o) + C**. Each R_p is at most 1 + (the accounted bytes p wrote or promoted) / (B/G): what a run leaves pending beyond one generation is never more than it wrote itself. When no run is in progress and every run ended normally, C is 0 and no R_p remains, so the total is the live generations, at most B + G x o. A run that crashed leaves its set-asides as C until the next run that reads them ends; with one process and runs that rotate at most once, that gives the bound of the earlier version of this note, **B + B/G + (G + 1) x o**, which test 2 asserts after a run.
- **What is not counted.** The ledgers add 1 byte per 4 KiB accounted, each generation one directory and its `born`, and each crash at most one temporary (section 6.3). Files that someone puts into the cache by hand are not counted; a directory that holds one is set aside into `kept/` after the entries `una` wrote are removed from it, so the bytes `una` wrote stay within the bound.
- **Age.** An entry used at time t is in the newest generation at t (stored there, or promoted there). That generation stops receiving entries no later than t + A/G at the next run, and is set aside once the next existing generation above it has a `born` older than A, or an expired one. So an entry unused since t is removed by the first run after about t + A + A/G + τ (its removal completing at the end of that run), and kept at least until t + A - τ unless the size bound or an expired `born` removes it first. A concurrent rotation never makes a `born` expired (section 5.2, the clock), so neither claim depends on how runs interleave; only a clock that moves back by more than τ, or forward, or a `born` dated by a skewed server beyond τ, does. An entry used at least once per generation is never removed by age. A rotation is always possible (step 4's restart), so `cur` itself is never pinned: a planted `born` dated after now + τ is expired, and one dated within τ rotates by age by born + A/G <= now + τ + A/G; a planted ledger is full.
- **Working set.** Promotions count against the newest generation's share like stores, so a run whose obligations, together with its new stores, fit in about (G - 1)/G x B keeps them across rotations: each run moves the working set through at most G - 1 rotations, so an entry promoted in one run is still in one of the G generations a lookup tries at the next. Each rotation moves the working set once, one rename and one append per entry, which is O(1) amortized per store since a rotation follows B/G accounted bytes or A/G of time. A working set above that misses cyclically.

### 5.4 Cost per run, warm and cold

- Warm run, every answer in `cur`: the chain of opens to `gen/`, one read of `gen/` (at most G + a few names), one `fstatat` of `born`, and the reads of today. No write.
- Warm run just after a rotation: each answer is found in `cur - 1` after one failed name lookup and moved with one `fstatat`, one rename and one append; the generation the rotation set aside is removed at the end of the run that rotated, outside every query.
- Cold run: G failed lookups per query (each an `openat` that fails with "not found", microseconds), the solve, the check, the store and one append. The solve dominates by orders of magnitude.

## 6. Safety invariants

### 6.1 Eviction can only cause a re-solve

- Eviction removes or renames names; it never writes into an entry. An entry is an answer only when its tag, computed from the bytes read under the user's key, matches (ADR 0064 rule 1). So eviction can turn a hit into a miss and nothing else, and a miss is solved by the same query with the same lock.
- A promotion moves a file whose bytes were already read and checked. The tag binds the key and the answer, not the directory, so the moved entry is the same answer. If someone swaps the file for something else between the check and the rename, the moved name is checked again at its next read, like any entry.
- An entry whose tag fails is never promoted. It stays where it is and leaves with its generation, or, in `cur`, is replaced by the next store as today.

### 6.2 An entry being read is never lost

- A reader holds an open descriptor from its `openat` on. Removing or renaming the name does not affect the bytes read through it (POSIX `unlink` and `rename` semantics, on macOS and Linux, the platforms `una` serves). A reader that has not opened the file yet finds no name and has a miss.
- So a reader observes the whole entry or a miss, never part of one. No lock is needed, and none is held across a solve.

### 6.3 Crash safety

Every step leaves a state that is correct to read and that the next opening cleans up:

| Crash between | State left | Effect |
| --- | --- | --- |
| temporary created and renamed | a temporary in a generation | removed with its generation |
| entry renamed and ledger appended | an entry not counted | at most one per crash; removed with its generation |
| `mkdirat` of a generation and its `born` | a generation without `born` | the next opening creates `born` |
| set aside and the end of the run | a `.removing-*` directory | the next opening reads it; removed at the end of that run |
| two listings of a removal | a partly removed `.removing-*` | the same |
| first set aside and `mkdirat gen/0` of a restart | generations pending, no generation, at most 257 names in `gen/` | the next opening creates `gen/0` (within the 512 names of step 1) and removes the pending ones at its end |
| two migration batches | flat entries left, no `migrated` | the next opening runs the next batch |

The ledger is never parsed, only measured, so a torn or garbage ledger changes only when a rotation happens. No `fsync` is added: the cache stays best-effort, as `put` documents.

### 6.4 No link is followed (#1160, #1164, ADR 0122 rule 30)

- **Threat model.** Two kinds of writer of `.una/` besides `una` itself, both named in ADR 0064's Context: files planted in a repository the user clones, and processes that run with the user's permissions and change `.una/` while `una` runs (a generating agent, a shared checkout). The directories above the package root are the user's and outside the model.
- **Descriptors, not paths.** Every component below the package root is opened relative to its parent's descriptor with `O_NOFOLLOW|O_DIRECTORY`, and every file operation is relative to the descriptor of its directory (`openat`, `mkdirat`, `renameat`, `unlinkat`, `fstatat` with `AT_SYMLINK_NOFOLLOW`, and a listing of an opened descriptor), the pattern ADR 0122 rule 30 prescribes for Itaoca. A link at any level, even one that resolves inside the package root, is refused and turns the cache off for the run. This is stricter than `create_dir_inside`, which follows a link that stays inside the root (`safe_write.rs:330-390`): with it, a committed link `.una/cache/proofs/gen -> ../../../data` would make numbered directories of the package's own `data/` generations, and a store would append to, and a removal delete, files of the user's checkout named `ledger`, `born` or `<64 hex>.json`. With descriptors, a concurrent swap of a directory for a link after it was opened does not move the descriptor, so the claim also holds against the second kind of writer.
- **Where it comes from: not on `main`.** `crates/una-sys/src/` on `main` (01737b2c2) holds `attention`, `coroutine`, `io`, `poll`, `server`, `signals` and `solver/`; it has no `dir` module and no binding of `openat`, `mkdirat`, `renameat`, `renameat2`/`renameatx_np`, `unlinkat` or `fstatat`. `tree.rs:91-97` names only opens relative to a directory descriptor, for reads on Linux. The `dir` module is #1094 (open), re-filed as #1275, parked: it needs redoing on `main`'s provenance store and measured +204 lines over ADR 0058's cap. ADR 0058 (amendment of 2026-10-25) admits for `dir.rs` `openat`, `mkdirat`, `renameat2`/`renameatx_np`, `renameat`, `unlinkat` and `geteuid`. This design needs, beyond those:
  - **a "stat without following" relative to a descriptor.** No new binding: it is emulated by `openat(dir, name, O_RDONLY|O_NOFOLLOW|O_NONBLOCK)`, where `ELOOP` means a link and otherwise `fstat` through the standard library (`File::from(OwnedFd)`, then `metadata`) gives the type, the device, the inode and the modification time. `O_NONBLOCK` keeps a FIFO from blocking; a device node cannot be committed to a repository, and a process of the user cannot create one without privileges. Where this note says `fstatat`, it means this emulation unless the amendment binds `fstatat` itself;
  - **a listing of an opened directory.** The standard library lists by path only. Either `dir.rs` binds `fdopendir`, `readdir` and `closedir` (or `getdents64` on Linux and `getdirentries` on macOS), or the listing is taken by path with `read_dir` and accepted only when `symlink_metadata` of that path gives, before and after the listing, the device and inode of the held descriptor; either way every action on a listed name is relative to the held descriptor and checks the name again, so a listing of the wrong directory can only cause a miss or leave a directory pending, never an action outside the cache. The binding is the stricter choice and costs lines under the cap; the path listing costs none. Section 10 item 10 asks which.
- **Which crate.** ADR 0058 lets `una-rt`, `una-interp`, `una-cli`, `una-http` and `una-project` (and through it `una-check`) depend on `una-sys`; not `una-verify`, where the cache lives (`cache.rs`), nor `una-diag`. On `main` `una-verify` depends on `una-core`, `una-diag` and `una-syntax` only, and `ProofCache` is built in one place, `una-cli/src/verification.rs:164-170`. So the design keeps `una-verify` free of `una-sys`: `una-verify` defines a small trait for the cache's directory (open a child directory, list, stat without following, open or create a file, exclusive rename, rename onto a name, unlink, remove a directory, each relative to a held directory), and `una-cli`, which may depend on `una-sys`, supplies its implementation over `una_sys::dir` when it builds the cache. Unit tests of `una-verify` use an in-memory implementation of the trait; the link, planted-file and concurrency tests of section 8 run in `una-cli` against the real one. No change to the dependency list of ADR 0058 is needed for this route; adding `una-verify` to it is the alternative of section 10 item 9 (d).
- **On a platform `una_sys::dir` does not serve,** the cache is off, as `tree.rs` refuses every write there.
- **Until the dependency lands.** The implementation card waits: it is blocked on #1094/#1275 and on the ADR 0058 amendment of section 10 item 9. Until then the cache stays exactly as today (flat, unbounded, written through `safe_write`). No part of eviction is done by path in the meantime: a size or age sweep through the existing `safe_write` primitives, which follow a link that stays inside the package root (`create_dir_inside`, `safe_write.rs:330-390`), would bring back the case of a committed `.una/cache/proofs/gen -> ../../../data` removing files of the user's checkout. The one interim that needs no missing code, and that a separate card may take if wanted, is to turn the cache off for the run when `symlink_metadata` finds a link at `.una`, `.una/cache` or `.una/cache/proofs`: it removes nothing, closes the planted-repository case for today's stores, and leaves only a concurrent same-user swap between the check and a store, which today's code has as well.
- A name in `gen/` is a generation only when `fstatat` without following says directory: a link with a generation's name is ignored. A link inside a generation is removed as a name (`unlinkat`), never followed. Removal never uses `remove_dir_all`.
- The ledger is opened with `O_NOFOLLOW` and `O_NONBLOCK`, so a link or a FIFO swapped in at its name is refused, not written through or waited on, and then checked to be a regular file through its descriptor; a refused ledger is a full generation (section 5.2 step 3).
- `born` and `migrated` are created with `O_CREAT|O_EXCL|O_NOFOLLOW` and read only with `fstatat` without following; a `born` that is not a regular file, or is dated later than now + τ (section 5.2, the clock), is expired.
- **Planted state,** at most: a generation numbered `cur + G` or more makes the opening set aside every real generation at once (step 1, n <= cur - G), which **costs one full re-verification each time it is planted**, again whenever Git restores it; one at 2^32 - 1 makes the next rotation a restart, the same cost; a large, unopenable or non-regular ledger forces rotations (misses); a non-regular `born`, or one dated after now + τ, forces a rotation or a removal (misses); planted entries fail their tags (misses); more than 512 names, or more than 256 generations, in `gen/` turn the cache off (misses); a planted file in a generation or a `.removing-*` directory sends that directory to `kept/`, after the entries `una` wrote are removed from it, while `kept/` holds fewer than 16 names, and leaves it pending otherwise; a link anywhere on the path turns the cache off. Both bounds of section 5.3 hold in every case, `kept/` holds at most 16 directories, and none of these can make an answer wrong or make `una` write, rename or remove anything outside `.una/cache/proofs/`.
- **A planted state can turn the cache off for good, silently.** Unlike the other cases, "cache off" does not heal, because a run with the cache off runs no eviction or cleanup, and it prints nothing (section 6.6): the only symptom is `obligations.cached` staying 0. This is the case of a link on the path, of more than 512 names in `gen/`, and of a planted directory restored by every checkout once `kept/` is full and the pending directories reach the limit. It is not a new class: a committed regular file at `.una/cache` turns every write off today. `rm -r .una/cache/proofs` (section 7.3) restores the cache. The bytes in `kept/` are bytes the repository planted, not bytes `una` wrote; they are bounded by 16 directories, each of at most what one checkout restores.

### 6.5 ADR 0064 and its amendment

Unchanged: the key and its domain tag, the tag under the user's key, the constant-time compare, the key-file checks of `proof_key.rs`, the answers that are never stored. The reads keep their checks, now relative to the generation's descriptor: an `openat` with `O_NOFOLLOW|O_NONBLOCK` refuses a link at the entry's name, `fstat` must say regular file, and at most 64 KiB + 1 bytes are read. The writes keep theirs: a fresh temporary created exclusively and renamed onto the entry's own name, which replaces a link there rather than following it. One check is added: `put` refuses an entry over 64 KiB. Without a key there is no cache, so no generation is opened, rotated or removed (ADR 0064 rule 3: nothing is written).

### 6.6 Output

- Eviction prints nothing, never on stderr under `--json` (`spec/13 §5`), and never changes an exit code: every failure of opening, rotation, promotion or removal is treated as "no change" and the run continues.
- Statuses, counterexamples, diagnostics and exit codes do not depend on the cache (ADR 0064 rule 6, `spec/12 §10`). What may change is what already depends on it: `obligations.cached`, the `(cached)` marks of `una obligations`, the language server's proof counts that come from those, and timing. The removal at the end of a run adds to the run's wall time after its output, not to the output.
- The policy is deterministic for one sequence of runs and one clock: the same accesses at the same times remove the same generations. Concurrent processes interleave, which, as today, changes only which run finds an entry first.
- The clock is read at every opening and reopening, after the `fstatat` of `born` (section 5.2). A clock that moves back by at most τ changes nothing; by more than τ, it makes the newest `born` expired: one rotation (misses), after which the new `born` is dated by the clock as it is. One that jumps forward removes generations early (misses).

## 7. Knobs

### 7.1 Defaults

| Knob | Default | Range | Meaning |
| --- | --- | --- | --- |
| Maximum bytes | 512 MiB | 16 MiB to 65,536 MiB | B, the accounted bytes of `.una/cache/proofs/` |
| Maximum age | 30 days | 1 to 3,650 days | A, the age after which an unused entry is removed |
| Generations | 4 | fixed | G, not a knob |

- **Why 512 MiB.** At one 4 KiB block per entry, it holds about 131,000 entries, so a program with 20,000 obligations keeps several edits' worth of history and every branch in use. The example packages use well under 1 MiB.
- **Why 30 days.** Every warm run promotes what it uses, so active work never ages out. A branch untouched for a month verifies once again on its first check, as after a solver change (ADR 0124 rule 10).
- **Why bounded ranges.** A cloned repository chooses its project file. The upper bound keeps it from asking for an unbounded store. The lower bound only keeps the cache useful for small programs: 16 MiB holds about 4,000 entries at 4 KiB, so a hostile `ita.una` can still make the 20,000-obligation program above re-solve cyclically. The real answer to a hostile value is a per-user setting that wins over the package's (section 11, question 3).

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
- The knobs change no query and no key, so they need no edition gate of their own beyond the `0.4` section they live in: a package of edition `0.4` checked by two releases with different defaults is bounded by whichever ran last. The section is still a construct of edition `0.4`, so it carries the duties of ADR 0121 rules 9 and 10 (section 10 item 8).

### 7.3 Pruning now

`spec/13 §5` defines no cache command, and the CLAUDE.md scope guard forbids adding one without the specification. The specification already gives the way to empty the cache: "Removing `.una/` verifies every obligation again" (`spec/12 §9`), so `rm -r .una/cache/proofs` prunes everything now, and eviction makes the routine case automatic.

A command that applies the budget immediately, without emptying the cache, would fit the conventions of `spec/13 §5` as `una cache prune [path]`, printing one `una.cache/1` document under `--json` (`schema`, `removed`, `kept_bytes`) and exiting 2 on a usage error. It **needs an amendment** of `spec/13 §5`, and this design does not propose it for the implementation card: nothing requires it, and section 11 asks whether it is wanted.

## 8. How to measure

The cache gains a counter record, `CacheStats`, returned to its caller and asserted by tests, not printed: lookups, hits by generation age (0 to G - 1), misses, stores, appends, promotions, rotations, generations removed, entries removed, names listed. The counters make the complexity claims testable without wall-clock timing.

1. **Counter-based complexity test (scaling).** Fill a cache to 1,000, 10,000 and 100,000 entries, then run 1,000 lookups and 1,000 stores at each size. Assert that names listed per run is at most G + 4 plus the entries of the generations removed in that run, that the total entries removed never exceed the total stored, and that lookups, appends and promotions per access are the same constants at every size. A quadratic or scanning implementation fails at 100,000.
2. **Bound test.** With a test-only constructor that takes B = 1 MiB and A, store entries of mixed sizes (300 bytes to 64 KiB) until 10 x B has been written. After every store, walk the directory in the test (the test may scan) and assert: at most G generations, and at most R pending directories, R the rotations of the run so far from `CacheStats`; the sum of the entries' lengths rounded up to 4 KiB at most B + G x o + R x (B/G + o) during the run, and at most B + B/G + (G + 1) x o after the run ends (one process); no file outside the layout of section 5.1. Assert from `CacheStats` that no removal happened inside a lookup or a store (removals counted only by the end-of-run step). Run it twice more: with a **promotion-heavy** load, a working set of 0.6 x B reused on every run while new entries keep arriving, and with a warm-only load that never stores, asserting the same bound and that the working set is still hit after 20 rotations; and with an entry over 64 KiB, which is not stored.
3. **Age test.** With an injected clock (read by every opening after the `fstatat` of `born`) and `born` times set with `File::set_modified`: an entry stored at t and never used is present at t + A - 1 day and gone at the first opening after t + A + A/G; an entry used once per A/G survives 10 x A; a `born` dated now + τ - 1 second is not expired and one dated now + τ + 1 second is.
4. **Concurrency test.** Spawn 8 processes (the test binary re-executed, or `una check` on generated packages) that share one cache with B = 1 MiB, each doing 2,000 random lookups and stores over 4,000 keys, with answers derived from the key so a wrong answer is detectable. Assert: every hit returns the answer of its own key; no process fails or prints; at sampled points the total is at most B + G x o + Σ_p R_p x (B/G + o), with each process's R_p read from its `CacheStats` at the sample, and with P = 8 and every R_p <= 1 in a variant that sizes the stores so that each process rotates at most once; after a final run ends, the single-process bound B + B/G + (G + 1) x o holds, and no `.removing-*`, no `kept/` and no temporaries remain. A **forced rotation race**: with an injected hook, process Y rotates g to g + 1 between process X's clock read and X's `fstatat` of `born`, and again between X's `mkdirat` failure and its re-read; assert that the one share-triggered rotation sets aside exactly one generation (g + 1 - G), that g is still a generation after X's opening, and that X adopts g + 1 without rotating again. And a **stale holder**: process Z holds `cur` = g while Y rotates by age; assert that Z's next store finds `gen/<g + 1>` and lands in g + 1.
5. **Crash tests.** For each row of section 6.3, build the state by hand and assert that the next opening leaves a valid layout and that lookups of untouched keys still hit.
6. **Link and planted-file tests,** next to `no_entry_is_stored_through_a_link_leading_outside_the_package_1160` and `an_entry_is_read_only_from_a_small_regular_file_and_never_written_through_a_link`. Each asserts the bounds of section 5.3 over 10 x B stored and 10 x A of injected time, and that nothing outside `.una/cache/proofs/` changed:
   - `gen` a link outside the root; `gen`, `.una` and `.una/cache/proofs` each a link **inside** the root to a package directory holding `2024/ledger`, `7/born` and a `<64 hex>.json` (cache off, those files byte-identical);
   - a generation name that is a link (ignored, target untouched); a link inside a generation pointing outside (unlinked, target intact); a pending directory swapped for a link after the set-aside (nothing outside touched);
   - `ledger` a link, a FIFO or a directory (refused, no hang, the generation rotates as full);
   - `born` a directory, a link, or a regular file dated a year ahead (expired: rotation, or removal of the generation below);
   - planted generations 4294967295 and 4294967292 (a restart within G rotations; entries stored afterward are found), and a planted `gen/<cur + G>` (every real generation set aside: one full re-verification, then entries found again);
   - more than 512 names in `gen/`, and 257 generations (cache off); a crash after a restart's set-asides, leaving 256 `.removing-*` names and `gen/0` (cache on, the pending ones removed at the end of the next run);
   - a planted file in a generation and in a `.removing-*` directory (the entries `una` wrote removed, the directory moved to `kept/` and not read again), repeated 20 times as a checkout would restore it (at most 16 directories in `kept/`, the rest pending, then the cache off; `rm -r .una/cache/proofs` restores it);
   - a crash leftover at a set-aside's target name `.removing-<n>.<pid>.<serial>` and at a `kept/` target (the exclusive rename retries with the next serial; nothing replaced);
   - a writer that keeps creating `<64 hex>.json` in a `.removing-*` directory during the end-of-run removal (two listings, then the directory left pending; the run ends).
7. **Output test.** `una check --json` and `una obligations` on the example packages, twice, with and without a forced rotation and removal between the runs: identical documents except `obligations.cached` and the `(cached)` marks, and stderr empty under `--json`.
8. **Existing tests kept.** `a_second_run_reports_every_obligation_as_cached`, `a_forged_or_foreign_cache_entry_is_solved_again`, `a_damaged_cache_entry_is_solved_again_and_never_reused_11`, `an_unknown_obligation_is_an_error_every_run_and_only_a_wallclock_unknown_is_not_cached`, the `cache.rs` unit tests and `lsp.rs`'s cache case, with paths updated to the generation layout.
9. **Measurement for the PR.** The acceptance criterion is counter-based, from `CacheStats`: per warm run with every answer in `cur`, names listed at most G + 4, one `fstatat` of `born`, writes 0, promotions 0; per warm run just after a rotation, promotions equal to the hits in `cur - 1` and one removal at the end. Wall time and `obligations.cached` of `una check --json`, cold (after `rm -r .una`) and warm, on the example packages and on a package with dependencies, before and after the change, five runs each, in a release build, plus the wall time of one generation's removal at the default budget, are reported as information, not as a pass criterion.

## 9. Migration

- A cache of an earlier `una` is flat: `<key>.json` and temporaries directly in `.una/cache/proofs/`. The new layout never reads them; its first run starts with an empty `gen/0` and verifies everything once, as after ADR 0064 and after a solver change under ADR 0124 rule 10.
- The flat files are removed in batches: while `migrated` is absent, each opening reads at most 4,096 names of `.una/cache/proofs/`, through its descriptor, and stops there; it removes, relative to the descriptor and under the rules of section 5.2 step 5, the names among them that are a flat entry or a temporary, regular files or links only. When a batch removes no name, it creates `migrated`. Each run's share of the migration is O(4,096) names read, whatever else the directory holds, and the whole migration is O(n) once per package. A directory with more than 4,096 names that are not flat entries ahead of the flat ones in the listing (planted) ends the migration early and leaves those flat entries, which `una` no longer writes, as the leftover of the last bullet.
- Moving the flat entries into `gen/0` instead would keep them warm, but a large old cache (the case this card exists for) would fill `gen/0` past its share at once and be removed by rotation anyway. Removing them is simpler and the same in effect.
- An older `una` run on a migrated package writes flat entries again, which the newer one never reads; it does not see `migrated` and does not mind `gen/`. Running old and new releases alternately therefore costs re-solves, never a wrong answer, and the flat entries of the old release are unbounded as before. The next run of a newer release removes nothing more, since `migrated` exists: that leftover is a stated limitation, removable with `rm -r .una`.

## 10. Records and documents to amend

No ADR is edited by this note. Before the implementation, these need a dated amendment or text change, accepted as the project decides:

1. **ADR 0064**, dated amendment: the cache is bounded by size and age through generations; eviction only removes names; the layout; the knobs and defaults; the safety rules of section 6. It amends `spec/12 §9`.
2. **`spec/12 §9`**: the sentences on the layout ("Each entry is one JSON file named by the key in hex" stays, in a generation directory), eviction with its bounds and the defaults, that an evicted entry is solved again, and that an entry over 64 KiB is not stored.
3. **`spec/30 §2`** (and ADR 0122, dated amendment, since `spec/30` carries its rules): the `cache` section of `ita.una`, its grammar, its order, its checks (U7008) and its canonical form.
4. **ADR 0122 rule 30 and rule 48**, the same dated amendment: when the shared store `~/.una/itaoca/proofs/` lands, it uses the same generations and the same descriptor-relative operations; its knobs, a per-user store's, cannot come from a package's `ita.una` (section 11, question 3).
5. **`spec/13 §5`**, only if the prune command of section 7.3 is wanted.
6. **`docs/security/tcb.md §3.10`** (not a record): the threat model and guards of eviction, the descriptor-relative operations, and the planted-file cases of section 6.4.
7. **`spec/30 §7` and ADR 0122 rule 48**, a text correction of `spec/30 §7` and a dated note on ADR 0122 rule 48, independent of eviction: the edition `0.4` key also covers the solver build digest, as one more length-prefixed field after the lock's `solver` section, as `spec/12 §9` (ADR 0124 rule 10) and `cache.rs:82-88` already have it (section 1.2).
8. **Edition `0.4` duties of ADR 0121**, for the `cache` section of `ita.una`: the dated amendment of item 3 carries an "Edition `0.4`" section (rule 10): words `cache`, `proofs`, `unused`, `mib` and `days`, contextual in that section only; clashes none, since edition `0.3` has no `ita.una`; migration none; no unsettled point and no trust under rule 2; security, the rules of section 6. The implementation adds the construct's entry to the edition ledger (rule 9), and a release that later changes the section lists it in the changelog entry of rule 6.
9. **ADR 0058,** dated amendment, decisions for the project, before the implementation and together with #1094/#1275, which this card is blocked on. Measured on `main` (01737b2c2) with the commands of ADR 0058: the base code of `una-sys` is **1,519 lines** (`attention.rs` 115, `coroutine.rs` 666, `io.rs` 39, `lib.rs` 27, `poll.rs` 174, `server.rs` 165, `signals.rs` 333) against a cap of **1,594** (amendment of 2026-10-28), 75 lines of headroom; the `solver` files are 1,689 lines against their cap of 1,768. #1275 measured the `dir` module at +204 lines over the base cap, before anything this design adds. The decisions:
   - **(a) the cap:** a new base cap at the measured count of the merged `dir` module plus at most 5%, as the amendment of 2026-10-25 prescribes, with the count stated in the pull request;
   - **(b) the listing:** bind `fdopendir`, `readdir` and `closedir` (or `getdents64` on Linux and `getdirentries` on macOS) in the `dir.rs` row, or accept the path listing checked by device and inode of section 6.4, which needs no binding;
   - **(c) `fstatat`:** bind it in the `dir.rs` row, or accept the `openat` + `fstat` emulation of section 6.4, which needs no binding;
   - **(d) the dependent crate:** keep the list as it is, with `una-verify` reaching the descriptor operations only through the trait that `una-cli` implements (this design's choice), or add `una-verify` to the crates that may depend on `una-sys`;
   - **(e) the purpose:** the `dir.rs` row names "the package store"; it would also name the proof cache;
   - and, as for every change to `una-sys`, the independent security review before it merges.
10. Not amended: ADR 0041 (rule 2's `cached` count is unchanged), ADR 0063 rule L5 (the editor still shares the cache), ADR 0124 rule 10 (the key is unchanged), `spec/13 §6` (`tcb.proof_cache` still names `.una/cache/`; no report field is added), `spec/12 §10`.

The implementation card then carries the changelog fragment; this note, like the earlier design notes in `docs/plan/`, adds none.

## 11. Open questions

1. **Defaults.** 512 MiB and 30 days with 4 generations, or other values?
2. **Editions `0.1` to `0.3` without a knob.** This note reads their stability terms as forbidding a new `una.toml` table, so they get the defaults only. Is that the intended reading, or should edition `0.3`'s preview admit `[cache]`?
3. **Project file or per user.** The cache is on the user's disk, while `ita.una` is chosen by whoever wrote the package. Should the knobs also, or instead, be settable per user (for example, variables beside `UNA_PROOF_KEY`), which would also serve the shared store of ADR 0122, and which wins when both are set?
4. **A prune command.** Is `una cache prune [path]` (section 7.3) wanted, given `rm -r .una/cache/proofs` and automatic eviction?
5. **Migration.** Is one re-verification of every package after the upgrade acceptable, as after ADR 0064 and ADR 0124 rule 10, rather than moving the flat entries into the first generation?
