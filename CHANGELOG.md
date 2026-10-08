# Changelog

All notable changes to Una are recorded here. Versions follow the milestones and gates of `docs/plan/roadmap.md`.

## Unreleased

### Security

- **Executables and releases carry no path of the machine that built them** (#382, S-13). On macOS, `una build` kept the absolute path of `libuna_rt.a` in the executable's debug map, so every executable named its builder's install directory, usually with the home directory and user name, and two installs gave different bytes. Released archives also named the path of the machine that built them: the cargo registry and the toolchain under its home directory, and its source tree. `una build` now links without debug information (`-Wl,-S`, on macOS and Linux), and `scripts/release.sh` remaps the source root, the cargo home and the toolchain to `/una`, `/cargo` and `/rust`, for Rust and for C. What a program does is unchanged. `scripts/check-release.sh`, which local-ci and the release gate run, checks that the same program built by the archive unpacked in two places is the same executable, and that neither the archive nor that executable names a home directory or the source tree.

### Diagnostics

- **A U0103 rename never makes a call reach another function** (#125, R-1 and R-2 of the deferred review). The rename edit for a word reserved for a later version chose a new name one file at a time, so a declaration and its uses in another module could get different names, and the use's name could be another function that module already had: renaming the import line alike, as the rest of the file was, made `table(1)` call `a.the_table` and print 101 instead of 1. The name is now chosen once for the whole package, none of its modules' names, the private ones included; and a name an `import` line lists gets the same edit, where it got advice to rename the module. Applying every edit gives a program that means what the source meant.
- **The helper repair of a failing function value writes its header as the value is used** (#126, P-1 of the deferred review). When a function that can fail and has a `requires` is passed as a value, U5001 proposes a named helper that calls it only where its precondition holds. Its header dropped the labels and printed a generic result at its type parameter, so written as described it was U4001: `risky_where_defined(a: Int, b: Int) -> Int` for `risky(a: secret Int, b: Int) -> secret Int`. It now has the labels, refinements and type arguments of the use, and checks once written. When the precondition reads labeled data, no such helper can check, since its guard around a call that can fail would reveal that data (U4002): the repair now says so.
- **Edition 0.1 names a decision on `T` values in its messages** (#362). Since #354, edition 0.1 tells `label(T*)` from `label(T)` in its flows, as edition 0.2 does, but its messages still wrote `label(T*)` as `label(T)`, so a rejected flow could read "a flow from `label(T)` to `label(T)`". They now name `label(T*)` as edition 0.2's do. Only the text changes: over 2164 targets (conformance, examples and the review corpora under `docs/`), every exit code and every edition 0.2 message is unchanged, and every edition 0.1 message is unchanged once `label(T*)` is read as `label(T)`.

### Documentation

- **Decisions on type-parameter values given to function values** (#364): the guide's section on labels shows why `[if a == b then p else q].map(each: k)` is U4001 at `each: k`, in both editions, and how to pass values of `T` through or return the decision; `una explain U4001` points to it.
- **Agent ABI versioning** (ADR 0076, #370): adding a key to a JSON document, at any position, keeps its `una.*/N` version, and consumers ignore unknown keys; removing, renaming or retyping a key, changing a meaning, or adding a value to a closed set of strings bumps it. The diagnostic `code` and the obligation `kind` are open sets. No schema changes version.
- **Deferred reviews of ADR 0054, the U0103 rename and the precondition repair** (#124, #125, #126), in `docs/reviews/v0.3.3/repairs-and-0054.md`: the re-solve of ADR 0054 never gives `proven`; four findings, none unsound by itself, are triaged there.
- **Constructor argument order and inline lambdas at operations** (ADR 0081, #8, #85). `spec/04 §3` rule 5 says a constructor's arguments are evaluated in the order its kind declares its fields, which canonical form gives them, so `una fmt` cannot change what a program does (R-023). `spec/07 §7` rule 4 says an inline lambda given as an operation argument is typed against the declared parameter type, so `@p.grab(fn() -> pin)` is U4001 where the same lambda bound with `let` is accepted; two conformance cases record both forms (R3-N1). Nothing in the implementation changes. ADR 0081 also decides #14, the report's handlers, which follows.

### Performance

- **Checking many generic functions is linear in their number** (#375). The least fixpoint of result summaries built each component's callers from every edge and found each function it checked by scanning every declaration, and the language-server index found each type's declaration by a linear search: each was quadratic in the number of functions. They are now one pass over the edges, a map from function to declaration, and a binary search, so a check is O((n + E) log n): 16,000 generic functions take 14 G instructions, from 37 G, and doubling them doubles the work. Outputs are byte-identical.

### Command line

- **The report lists handlers that hold authority** (ADR 0081, #14). `una report` listed under `authority` only the functions with `uses`, so a handler with `uses` clauses, which performs operations of its own, did not appear, and a host capability that only a handler used was missing from `tcb.host_capabilities`. The new key `handler_authority`, after `authority`, lists each such handler with the capability it implements, `public` and its `uses`, and `tcb.host_capabilities` counts handlers. Both are additive under ADR 0076: the schema stays `una.report/2`, and `authority` still lists functions only.
- **A crash of the compiler is an internal compiler error, as `spec/13 §5` says** (#376). A panic in any command, on the main thread or a worker thread (the checker, `una run`, `una test`, `una build`), exited with code 101 and, with `--json`, printed no document. It is now U8001 with exit code 3: one line on stderr in human mode, and one `una.diagnostics/1` document on stdout with `--json`. Rust's own panic message stays on stderr.

### Checks

- **A `via` body is exempt from label checks in edition 0.2 too** (#384). Two edition 0.2 rules, the program counter of an update of an editable binding and a map key above the map's top label, reported U4001 inside the `via` function of a policy, which `spec/07 §6` rule 6 exempts from label checks; a `via` written with `let edit`, a `for` loop or a map was rejected. Both now follow the exemption as the other rules do. Every other function of a trusted module, including one a `via` calls, is checked as before.
- **A kind cannot take the name of a built-in type in edition 0.2** (ADR 0079, #130). A kind, a variant or a capability named `Int`, `Nat`, `Bool`, `Text`, `Unit`, `Hashed`, `List`, `Option` or `Result` silently replaced the built-in type in its module; in edition 0.2 it is U1002, as the fixed-width names and `Map` already were. Edition 0.1 is unchanged.

## v0.3.2

A security patch of the Beta, completing v0.3.1's edition 0.1 fixes at their root: edition 0.1 now tells a decision on `T` values apart in every flow, as edition 0.2 always has. Edition 0.2 is unchanged. In edition 0.1, binding a value decided by comparing `T` values to a written annotation that mentions `T` (`let y: T = ...`, `List[T]`, a kind of `T`) is now U4001, as in edition 0.2, even where the program would not leak: leave the annotation out, since inference keeps the decision (`docs/guide/una-for-agents.md` §13, #345).

### Security

- **An annotation no longer drops a decision on `T` values, in edition 0.1** (#354, Critical). In a generic body, `let y: T = if a == b then p else q`, or a record field `Hold[T]`, dropped the label of the comparison, since edition 0.1 read the decision label `ℓ*_T` as `ℓ_T` in every flow; a later `k(y)`, a generic callee or `map` then revealed it past every rule of #253, #346 and #350. Edition 0.1 now tells `ℓ*_T` from `ℓ_T` in its flows, as edition 0.2 does: a decided value flowing into an annotation, a field or a function value's parameter of type `T` is U4001 at the flow. No accepted program of the repository changes; already rejected edition 0.1 programs of the security suites gain that U4001 beside their U4002 or U4003. Found by Una-2 (H-6); her matrix's section 15 is the conformance cases `security/hof_launder__*`.

## v0.3.1

A security patch of the Beta. Edition 0.2, the edition of the Beta, is unchanged in what it accepts. Edition 0.1 packages compiled by v0.3.0 could reveal a decision on `T` values through function values: through the prelude's higher-order functions and through generic functions of the program. That is fixed below, with the same rule as v0.1.4 and v0.1.5. Every finding came from adversarial reviews before any report from outside: a matrix of 194 cells, every path by which a function value can be called in edition 0.1 crossed with the result, effect and failure channels. Further edition 0.1 hardening follows in the next patch (#354). The release also carries three performance fixes.

### Security

- **A decision on `T` values given to function values, in edition 0.1** (#346, #350, Critical). In a generic body, a call that gave function values data decided by comparing values of `T` was accepted in edition 0.1, and could reveal the decision: through the prelude, `[if a == b then p else q].map(each: k)` written to a `public` sink, and so `filter`, `first_where` and `fold`; through a generic function of the program, its result, its own effects and failures, and the function values' (`logger(decided, f: k)` writing `f(x)`, or failing on it), with named function values, lambdas whose parameter is annotated `T`, and functions in a record field. Such a call now raises its result at every position, runs under the decision so that its effects are bounded by it and it may not fail (U4002), and labels its failure, as #253 does for a call written in the body. Lambdas whose parameter types are inferred, and values of `T` passed through, are unchanged. Edition 0.2 already rejected these programs (U4001 at the function argument). Found by Una-2's reviews of the v0.1.4 backport (#343) and of #349; her matrix and cases are the conformance cases `security/hof_*`, `security/tapp_*_0_1`, `security/decided_argument_to_prelude_callback_0_1` and `security/n39_*_0_1`.

### Performance

- **Checking a deeply nested type is no longer cubic in its depth** (#331, N-47). Resolving a type, the occurs check and taking a type's shape copied the whole value of each solved variable they met, at every node, so a value nested d deep checked in O(d³). They now follow variables by reference: a check is quadratic in the depth, since each expression keeps its own type (#335), and about 18 times faster at depth 160. Diagnostics and the queries sent to the solver are byte-identical on every conformance case and example.
- **The interpreter reads a binding in constant time** (#329). Its run-time environment indexes each name's bindings, so reading one no longer scans every binding in scope: a body of n bindings that each read an early one runs in O(n), not O(n²) (16,000 bindings: 2.6 s to 0.03 s). A closure keeps the bindings its body reads rather than a copy of the whole environment, so making and calling one costs what it reads, not what is in scope. Outputs and traces are unchanged.
- **Tier-1 queries of loops stay linear in the loop's state** (#333). A tuple sort of the loop state is named `i!Tuple<n>!<id>`, a 128-bit digest of its components, instead of spelling every component, so each projection's name stays short: a loop of n editable bindings gives queries linear in n (400 bindings: 2.0 MB of query text to 115 KB). The query text changes, so every proof-cache key of such a query changes and the first check after upgrading re-proves it; verdicts are the same. `encoding_version` stays `una-smt-1`: the encoding is renamed, not changed in meaning, and cache keys already hash the whole query.

## v0.3.0

The Beta of Una, on edition 0.2. Edition 0.2 gains a standard library for real programs: maps, text operations, JSON and an HTTP client, with labels on every value they produce. Lists and texts built in loops and through calls are updated in place. The release was cross-reviewed before its tag, each area by someone other than its author (N-37): JSON and HTTP (#308), maps and text (#309), and the in-place work (#311). Upgrading from v0.1.x: read v0.2.0 below as well, including its Critical fix #253, which makes edition 0.1 reject a shape that v0.1.0 to v0.1.3 accept. Edition 0.2 of v0.2.0 had an information-flow leak: an un-annotated `let edit` whose initial value left a label open, as `[]`, `None` or `empty_map()`, could print an `internal` value read in a loop before its assignment (#323, see Security). It is fixed in this release. v0.2.0 was validated but not published because of this leak, so v0.3.0 is the first published release of edition 0.2, and v0.1.x users go from v0.1.3 to v0.3.0.

### Security

- **Soundness: no label of an editable binding is decided by the order its uses are checked** (#323, ADR 0075, N-43 to N-45; found by the review #330). In v0.2.0, the labels that an un-annotated `let edit` leaves open, as the elements of `[]`, the value of `None` or the values of `empty_map()`, were fixed by whichever use the checker saw first. A read before the `internal` assignment, in a loop, then printed the `internal` value with no diagnostic: `let edit xs = []`, then a loop printing `{xs}` and appending `s.len`, printed `[]` and then `[11]`. The same held for options, results, maps and their nested positions. Now every open label position, at any depth, is a label variable solved over every assignment, `edit` argument and join of the binding, and every read sees the solution: those programs are U4003 at the read that reaches a sink. Two intentional tightenings: a read printed before the only `internal` assignment, with no loop, is U4003 too, and an un-annotated binding of functions, as `let edit fs = []` that later holds lambdas, is U4001 asking for an annotation, where v0.2.0 accepted both. A position that inference cannot solve, as in `[None]`, keeps its U2001. 30 shapes from the review are conformance cases.
- **Whether a host operation fails is labeled by the arguments that decide it** (#310, N-39, N-40; finding H-1 of the review #308, Critical). An `Http` request to an `internal` URL could reveal, through whether it failed, one bit of that URL to a `public` sink. The URL, the key name and the header list's labels now label the failure: with an `internal` one, `attempt` gives an `internal` `Result`, and `if error pass` and `if error fail` are U4002. Programs whose arguments are `public` are unchanged. `Files.write` with an `internal` path changes the same way, in both editions. A header value with a control character aborts the run with the new runtime code U6004, before anything is sent.
- **HTTP credentials stay with their origin.** `key_name`'s bearer token, and the program's `Authorization`, `Cookie` and `Proxy-Authorization` headers, are dropped once a redirect leaves the request's origin, and stay dropped for the rest of the chain. The review #308 checked this against a two-origin server in both engines.

### Edition 0.2: the standard library

- **Maps** (#236, ADR 0069, `spec/21`). Edition 0.2 gains `Map[K, V]`, keyed by an `Ord` type, in ascending key order, with `empty_map()`, `m.len`, `has`, `get`, `keys`, `values`, `entries`, `with`, `without`, and `edit m.set` and `edit m.remove`.
  - `len` is O(1); `get`, `has`, `set` and `remove` are O(log n), in place when nothing else holds the map; `with` and `without` on a map another binding holds copy it, O(n). A text key is copied into the map at insert, O(key length).
  - Which keys a map holds is part of its shape: reads carry the key position's labels with the top label, and a key above an editable map's top label is U4001 at the key.
  - `m.len` and `m.has(key: k)` are in the refinement language, and each operation's contract is a hypothesis after it.
  - In edition 0.2, a user kind named `Map` or `Entry` is U1002, and `with` after `.` is a method name.
  - The native debug runtime's note now reads "lists, maps and records copied".
- **Text functions** (#237, ADR 0070, `spec/22`). Edition 0.2 gains `split(on:)`, `join(separator:)`, `ends_with(suffix:)`, `contains_text(part:)`, `find(text:)`, `replace(pattern:, by:)`, `slice(start:, end:)`, `to_upper`, `to_lower`, `chars`, `parse_int` with the kind `ParseError`, `trim_start`, `trim_end` and `bytes`.
  - Positions count scalar values. `find`, `split` and `replace` are linear, over Rust's Two-Way search, which `find` and `contains_text` share.
  - Both engines keep a text with its scalar count and whether it is ASCII, and an index of every 64th scalar value built at the first position on a non-ASCII text: `.len` is O(1), and `slice` and `find` positions O(1) after it. An append in place updates them in O(appended part) (ADR 0070 X2, X10).
  - Every result carries every label of every argument, and so does a failure of a prelude function that declares one, as `parse_int` does: `attempt`'s `Ok` or `Err`, the payload, and the code that handles it run under those labels (N-27). The contracts are hypotheses, `slice`'s precondition is an obligation, `find` is `Some` exactly when `contains_text` holds (N-19), and its position bounds the match within the text (N-21).
  - The parameters `separator`, `start` and `end`, and `pattern` and `by` replace the ADR's `with`, `from` and `to`, and `old` and `new`, since `with`, `from` and `old` are keywords (N-20).
- **JSON** (ADR 0073, `spec/23`, #238). In edition 0.2 the prelude reads and writes JSON: the kind `Json` (`Null`, `Boolean`, `Number` for exact integers, `Decimal` for other numbers as written, `Str`, `Array`, `Object` as a map in key order), `parse_json` failing with `JsonError` (`Syntax`, `TooDeep` past 512 levels, `DuplicateKey`, at a position in scalar values), a canonical `to_json`, and `quote_json`. Parsing is O(n + Σ k log k), writing O(n), with no recursion on the native stack; one implementation serves both engines. Parsed values keep the label of their text, and a secret cannot enter a `Json` constructor.
- **A JSON value over the memory budget is U6003** (#314, N-41, review finding H-2 of #308). `parse_json` and `to_json` check the budget every 4096 values and every 4096 scalar values of a string, in both engines, so a large input ends with U6003 at the call, where an allocation past twice the budget aborted the process. The parser now reads the text in place instead of copying it into a vector of characters, four bytes each. No measurable slowdown: by doubling from 1 to 21 MB, every step stays ×1.8 to ×2.3 in both engines.
- **HTTP** (ADR 0074, `spec/24`, #239). In edition 0.2 the host capability `Http` sends HTTP/1.1 requests:
  - `get`, `post`, `put` and `delete`, one operation per method, return an `internal Response` or fail with `HttpError`;
  - origins and methods are granted with `http:<origin>[:<methods>]`, deny by default, https unless a grant says http, and redirects stay within the grants;
  - `key_name` sends a key of the `keys:` grant as a bearer token, to that origin only, never seen by the program or shown in a trace; a redirect to another origin also drops the program's `Authorization`, `Cookie` and `Proxy-Authorization` headers for the rest of the chain, and traces redact `Header.value` in both engines;
  - each request has one total deadline (`--http-timeout`, default 30 s, at most a day) and a body limit (`--max-response`, default 16 MiB), in `una run` and in executables;
  - whether a request fails is labeled by its URL, headers and key name (#310, N-39): with an `internal` one, `attempt` gives an `internal` `Result` and `if error pass` is U4002, and so for `Files.write` with an `internal` path; a header value with a control character aborts the run with the new runtime code U6004 (N-40), since the value is `internal`;
  - one client serves both engines, over rustls with ring and the Mozilla roots compiled in, the new trusted dependencies of ADR 0074.
- **`std.host` can be imported** (#16, N-33, `spec/10 §11` rule 10). `import std.host (Files)` names the host capabilities and their failure kinds (`Console`, `Clock`, `Env`, `Files`, `Keys`, `FileError`, `KeyError`), the same declarations the prelude puts in scope, in both editions. Any other `std` path is U1001 with a message that names the two standard modules that can be imported, `std.host` and `std.testing`, where it said `std.host` was not a module. In edition 0.2 it also exports `Http`, `Header`, `Response` and `HttpError` (#307).

### Checks and verification

- **Generic builders of maps and lists check, and so do nested ones** (#323, ADR 0075, N-43; finding M-1 of the review #309). In a `let edit` without an annotation, the positions its initial value leaves open, such as the key and value of `empty_map()` or the element of `[]`, have label variables, solved over every assignment, `edit` argument and join of the binding, and then fixed. So `tally[T: Ord](xs: List[T]) -> Map[T, Int]` and `group_by` built with `let edit m = empty_map()` check, and their results carry the labels of their arguments: a `tally` of `internal` words has `internal` counts. Reads before an assignment see its label too. The binding's top label, annotated bindings and determined positions stay fixed. Solving is linear in the flows; a function with such a binding is checked once more. Lists and maps nested inside them, as `Map[Text, List[Int]]`, check the same way.
- **`attempt` over a `fails of` parameter no longer implies effects** (#270). A private relay whose body is `let r = attempt f(x)` infers `uses of f`, as with a plain call, so another relay can pass its own parameter to it without a false U3003.
- **Every handling clause written on its expression's line is reported in one check** (#268). The parser goes on past the first such line of a body, so each one gets its U0101 and its edit, and applying them all converges in one round.
- **The counterexample of an assignment shows the value before it** (#277). For `n = n - 1` on `n: Pos`, it shows `n = 1, n (assigned) = 0`, where it showed `n = 0`.
- **A path that fails no longer reaches the return obligation** (#274). A rebind on a path that ends in `fail`, such as `ys = ys.append(...)` before `fail Bad`, no longer refutes the function's `ensures`.
- **Loops keep the length of a list they update with `set`** (#260, ADR 0071). In edition 0.2, a loop whose every change to a list is `set`, or an `edit` call whose callee ensures `xs.len == old(xs).len`, keeps the list's length in its body and after it. So `for i in 0..<xs.len` with `set(edit xs, index: i, …)` verifies with no guard, which fixes the v0.2.0 known limit S-1.
- **Refutations that only a loop's abstraction allows are unknown** (#260, ADR 0071 D3). A refutation inside a loop body that sets values the loop forgot is checked once more at the loop's first iteration. It stays U5001 with that iteration's counterexample when the first iteration fails, and is U5002 with the reason "the loop forgets every fact about `x` ..." otherwise. After a loop, such a refutation is U5002. Both are errors, as before.
- **Two more counterexamples that were no run** (#278). The body of a loop over a list now assumes the list has an element, so `for y in ys` with `set(edit xs, index: 0, …)` and `xs.len == ys.len` verifies. A loop whose state starts from an earlier loop's final value is U5002, not refuted at a first iteration it cannot pin.

### Performance

- **Checking a long body takes linear time** (#326). The checker resolved a local by scanning every local in scope, and saved the taken state of every local around each `if`, `when` and loop, so a body of n bindings checked in O(n²). Locals are now found by an index from their name, in the type checker, name resolution and elaboration, and the saved states cover the taken locals only. The verifier copied its whole path context, every hypothesis, at each step of a body; the hypotheses are now a shared list and the binder types and loop-entry facts are copied on write, so a step costs O(1). Diagnostics, the queries sent to the solver and their verdicts are unchanged: on every conformance case and example, the output and the proof-cache keys are byte-identical. A body of 16,000 bindings checks about 19 times faster than before at tier 0, and a loop over 16,000 state bindings about 4 times faster at tier 1.
- **Building a list through calls takes linear time** (ADR 0072, #262). An `edit` argument, or a binding at its last use rebound from a call's result (`acc = push(acc, x)`), now moves into the callee, which appends in place through any number of call levels, in the interpreter and in native code: amortized O(1) per append instead of a copy of the whole list at each call. A binding that the code after a failure reads keeps its value, as before. Native functions own the parameters they consume, found by borrow inference. A read-only parameter is still borrowed at no cost.
- **Appending to a text in a loop takes linear time** (ADR 0070 X10, #261). `t = "{t}..."` appends in place when nothing else holds the text, in the interpreter and in native code: amortized O(length of the appended part) per append, O(total length) for a loop, instead of a copy of the whole text each time. A text that something else holds is copied as before, so a copy taken earlier keeps its value.
- **Building a list or a text through function values takes linear time** (ADR 0072 amendment N-42, #312). `acc = f(acc, x)` with `f` a function passed as a value or a lambda, through any number of higher-order layers, and `fold` with a list or text state, now append in place, in the interpreter and in native code: amortized O(1) per append instead of a copy at each call. A lambda owns its parameters; a call whose target only reads an argument costs one retain and release. Native `fold` appending to a list of 80,000 items went from 10 s to 0.018 s, with no copies (IP-1 and IP-2 of the review #311).
- **Appending in a task of `together` takes linear time in the interpreter** (#263). A list or text that a task, or a loop inside it, appends to is updated in place, as native code already did: the bindings of the block's own tasks no longer count as read by the tasks after them. A list a task captures keeps its value for the other tasks.
- **Keeping the old value on failure stays quadratic, by design** (IP-3, N-42): see Known limits.

### Command line, runtime and tools

- **`una --version`** (#305, N-38, `spec/13 §5`). It prints the version, the editions it serves and the pinned Z3 (`una 0.2.0`, `editions: 0.1, 0.2`, `z3: 5.1.0`), and `una.version/1` with `--json`, with no commit hash. `install.sh` prints it after installing, and the bug and soundness templates, `CONTRIBUTING.md` and `SECURITY.md`, which ask for it, are now right.
- **A closed stdout or stderr discards the output** (#259). `spec/13 §5` and `spec/16 §3` now say what both engines already did: writing to a closed stream is not a failure to write, as for `/dev/null`, so `una run`, `una test` and native executables exit as they would with the stream open. A broken pipe (EPIPE), a reader that went away, still exits with code 2. Tests now pin both cases in the interpreter and in native code.
- **Executables keep only the runtime they call** (#239). `una build` links with dead-stripping, so an executable carries none of the runtime it never calls, the TLS stack of `Http` included: `examples/hello` built natively shrinks to about 40% of its size (debug build, macOS).
- **Runs that start together on a new home share one proof-cache key** (#282). The key file `~/.una/proof-cache.key` is now written to a file of its own and linked into place. Before, a run that lost the race to make it could read it half written, and ran with the cache off.
- **A solver that does not read its query is stopped at its wallclock** (#286). The deadline now runs from before the solver starts, and the query is written on a thread of its own: a solver that left the query unread used to keep the run waiting until it exited, past `--solver-wallclock`.
- **The amd64 `linux` gate runs its tests one at a time** (#286). Under emulation on an arm64 Mac, processes spawned from many threads at once could hang the test run. The native `linux-arm64` gate keeps full parallelism and is the primary evidence of the tests on Linux.
- **Three release archives** (#241): macOS on arm64, Linux on x86_64, and now Linux on arm64 (`aarch64-unknown-linux-gnu`), each built and checked by the `release` gate.

### Documentation

- **A getting-started tutorial** (#245, `docs/guide/getting-started.md`): from install to a verified, natively built program, `examples/wordfreq`, through a contract, a refuted proof and its counterexample, examples, grants, `una run`, `una build` and the editor. The guide test checks every block, and the tutorial's last package is the example.
- **The website** (#244, N-34). `scripts/site.sh` builds it with mdBook 0.5.4 into `target/site`, from the repository's own documents: a landing page from the README, the tutorial and the guides, the specification, the ADRs with their status, the validation records, the changelog, and a downloads page whose archive digests come from the validation records that `install.sh` trusts. The theme uses the Graúna palette in a light and a dark variant, with Una highlighting. Every page must build and every link between pages must resolve. `scripts/local-ci.sh` runs it when mdBook 0.5.4 is installed and prints that it skipped it otherwise, and the new `site` gate of `scripts/release-gates.sh` requires it (N-36). Hosting and the domain are not decided yet.
- **Project files for the public launch** (#246). A README for newcomers, `CONTRIBUTING.md`, `SECURITY.md` with what is in scope (the checker's soundness, the runtime, the trusted base), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), and issue templates for bug reports, soundness reports and questions.
- **Keeping the old value on failure is a known limit** (#311, N-42, finding IP-3 of the in-place review). The guide for agents explains why `acc = step(acc, item: x)` with `if error use acc` copies `acc` at every iteration, and shows the two linear forms: check first, or an update that cannot fail and returns a status with the new value. Measured with the debug copy counter: n copies for the first form, none for the other two.
- **The downloads page links no vsix** (#305, N-38). Its editor section says the VS Code extension ships on the Marketplace and Open VSX with the Beta, and links #243, where it linked `images/*.vsix`, which the repository does not hold. `scripts/site.sh` now fails on a link into the repository that names no tracked file, and leaves inline code alone, where it rewrote `sum[T: Integer](xs)` as a link.
- `docs/guide/syntax-reference.md`: the U0103 message format no longer shows as an HTML tag.

### Known limits

- **Keeping the old value on failure is quadratic** (IP-3, N-42). `acc = step(acc, item: x)` with `if error use acc` reads `acc` after the call, so every iteration copies it. A failed call changes nothing for its caller (`spec/17`), so the copy is the price of that guarantee. The guide for agents (§11) shows the two linear forms: check first, or an update that cannot fail and returns a status with the new value.
- **About 39 KB of `ring` in every macOS executable** (ADR 0074, H7). An executable that never uses `Http` carries no `rustls` or `webpki` code, and on Linux no `ring` code either. On macOS, `ld64` cannot dead-strip `ring`'s assembly kernels, which no call reaches. A test bounds the residual at 64 KiB on macOS and at zero on Linux.
- **Label polymorphism** (#40). A list whose element labels depend on a type parameter cannot be passed to a parameter of the program's own generic function (U4001: the parameter's top label is `public`), and a `fold` whose step returns such a list is U4001. The guide for agents (§13) shows the shapes and their workarounds, such as `sort`.
- **A closed stdout or stderr discards the output** (#259). A program run with a closed stream loses what it writes there and exits as it would with the stream open, as with `/dev/null`. A broken pipe, a reader that went away, exits with code 2.
- Native code is generated with Cranelift, and each target is built on its own platform. LLVM release builds are later (`plan/backlog.md`).
- Constraints have no laws: no axiom states that an order is total or transitive (`spec/20 §5`). There are no user traits or implementations.
- The point where a run exhausts its memory differs between engines and builds (ADR 0061 rule 5).
- A redacting view of a value, `redact(x)`, is deferred (ADR 0068).
- This release was built and checked locally (`scripts/release-gates.sh`), on macOS natively and Linux in containers.

## v0.2.0 (validated, not published; see v0.3.0)

This version passed its release gates (docs/releases/0.2.0-validation.md) but was not published, because edition 0.2 had the information-flow leak that v0.3.0 fixes (#323). Every change below ships in v0.3.0.

The first stable release of v0.2. The native backend compiles the whole language of edition 0.2 into executables that match the interpreter. Edition 0.2 completes its scope with structured concurrency, constraints on generics and printing of structured values, and every Critical and Major finding of the two independent reviews of v0.2 (#176, #177) is fixed. Gate 1's benchmark stays pinned to v0.1.x.

### Security

- **Labels of function values passed for `of` parameters** (#210, P0, Critical). In edition 0.2, a function value given for a `uses of` or `fails of` parameter, as `map`, `filter`, `first_where` and `fold` take, lost its labels. That was both a false rejection and an information-flow leak, present since v0.2.0-alpha.3. v0.1.x is not affected.
- **Defense in depth near `of` parameters** (#216). Three sites keep or join the labels the rules require, although none leaked: the effects of a function value given for an `of` parameter run under its own label too.
- **Edit arguments flow back with their final labels** (#186). An `edit` argument's new value is checked after every argument is typed. Before, a `secret` could reach a public place through a generic callee.
- **A decision on `T` values given to an effectful `uses of` parameter** (#234, Critical). In edition 0.2, a value chosen by comparing values of `T` inside a generic function, then passed to a `uses of` parameter whose effect reveals it, was accepted. Fixed with ADR 0067: a decision that reaches a call of an `of` parameter is recorded, and each caller runs its argument under the labels of the type argument. A pure callback stays accepted, while an effectful one over secrets is U4002.
- **A decision on `T` values through a call of a function-typed parameter, in edition 0.1** (#253, Critical, fixed by #255). A value chosen by comparing values of `T`, passed to a function value whose result reaches a sink, was accepted in edition 0.1. It now raises the call's result, as edition 0.2 already did. Edition 0.1 now rejects that shape, which v0.1.0 to v0.1.3 accept.
- **v0.2.0-alpha.3 is superseded.** Its security note named #210. alpha.3 is also affected by #234, so use v0.2.0.

### Edition 0.2

- **Structured concurrency** (ADR 0065, `spec/19`, #32, #167): `together` blocks and `together for`, with cancellation of the tasks after a failed one and a deterministic trace. `together for` proves the length of its list.
- **Constraints on generics** (ADR 0066, `spec/20`, #37): `Eq`, `Ord` and `Integer` on function type parameters, written or inferred for private functions, with the codes U2025 to U2027. The prelude gains a generic `min`, `max`, `contains`, `sum` and a stable `sort`.
- **Printing structured values** (ADR 0068, #204): interpolation writes any value that holds no function in its Una form, as `una test` shows it, keeping its labels.
  - Records, variants, lists, `Option` and `Result` are written in full, and `Text` inside them is quoted.
  - A value of a type parameter needs `T: Eq`, else U2026 with a repair. A function position is U2001.
  - A record with a `secret` field cannot reach `Console` (U4003), and the diagnostic names the field.
  - Every `Hashed` is written `<hashed>`, also through a type parameter, while traces keep `hashed:<hex>` (ADR 0068 amendment).
  - Native code writes values of a type parameter inside a structure through composite type descriptors, byte-identical to the interpreter.
- **Position-precise result labels of user generic functions** (ADR 0067, #92):
  - A call of a generic function raises each position of its result only by the labels of the type arguments that reach it (a summary per function, computed to a least fixpoint over recursion), not by every type argument (D-137).
  - Two symbolic labels: `ℓ_T` for a value of `T` as passed in, and `ℓ*_T` for every label of the type argument, which comparisons and conditions on `T` values carry.
  - `filter` and `first_where` take predicates over labeled elements, raising the result.
  - `una contract` shows `-- result labels: …` and `una report` lists `generic_results`.
  - Newly rejected, by the ratified rules: a decision on `T` values that reaches an effectful `of` parameter (U4002 at the caller, #234), and a function value called with an argument that carries such a decision.
  - Edition 0.1 keeps D-137.
- **Exact integers in `reflect` bodies** (ADR 0057 amendment, #180): the body of a `reflect` predicate makes no `overflow` obligation and no U2022, and every engine computes its integers exactly, as the verifier reads it.
- **Literals in constructors** (#187): `Some(3)`, `Ok(3)` and `Box(value: 3)` take the fixed-width type they are checked against.
- **Negative literals** in `*`, `/` and `mod` stay linear for the verifier (#188).

### Edition 0.1

Edition 0.1 keeps its rules, apart from the security fix #253 above. Some of its messages and repairs change:
- U2009 at a variant compared with `==` or `!=` offers `e is V` (#205).
- A reserved word used as a binding name says what it is reserved for. `let result` gets a rename of the binding and its uses (#206).
- A handling clause on its expression's line says where it goes, with an edit that moves it (#211).
- The U3004 message no longer gives advice that misleads in edition 0.1 (#256).
- The repairs of one check apply together without editing the same text (#257).

### Native backend

- **Fix: a failure through a `fails of` parameter was swallowed** (#267, Critical, found by the pre-tag review). In native code, a failure of a function value given for a `fails of` parameter was dropped. `if error pass` passed nothing on, `if error use` used a wrong value, and `attempt` gave `Ok`, where `una run` failed. Every native call now checks its callee's failure flag. A conformance case of relays through `if error pass`, `if error use`, `attempt`, two levels and `together` tasks is part of the native equivalence test.
- **`together` and `together for`** (#162): a thread per task with its own heap, values copied at the fork and the join, non-atomic reference counts.
  - An operation in progress at cancellation completes (#200).
- **Constrained generics** (#171): one hidden type descriptor per constrained type parameter (ADR 0066 G4, `spec/20 §7`), also captured by closures.
- **Tuples, `let (...)`, `for` loops and `set`**, and effectful and failing functions in the prelude's higher-order functions (#116).
- **In-place updates** (#30): a uniquely held list or record is updated in place at its last use.
  - #181 limits this to words with no live alias.
  - #182 keeps a range loop's high bound for the whole loop.
- **Grants per operation** (#183): `main` is checked against the modes of each `files` grant, as `una run` checks it.
- **Pipes and writes**:
  - executables ignore `SIGPIPE`, and report a closed output with exit code 2 (#184);
  - a failed write to stderr exits 2 (#198).
- **Native review minors** (#185):
  - key files load after U3012;
  - `--max-memory 0` is a usage error in `una run`, `una test` and executables;
  - `una build` works through a symlinked `una`;
  - the trusted base names `una-rt` and its allocator `cap` (ADR 0061 amendment, charter §3).
- **The runtime build** (#203): the README builds `una-rt` with `una-cli`, and a missing or stale `libuna_rt.a` names the command that builds it.

### Interpreter and tools

- **Lists are updated in place** at their last use in the interpreter too (#201). A loop of 100,000 appends takes a fraction of a second instead of about two minutes.
  - Loops run in constant memory in both engines.
  - The debug runtime's quarantine of freed slots is bounded.
- **`una run` with a closed stdout** reports its diagnostics on stderr, as the executable does (#199).
- **The solver's wallclock** is checked before its answer, so `--solver-wallclock 0` makes every query unknown (#189).
- **Diagnostics agents meet:**
  - U2009 at a compared variant offers `e is V` (#205);
  - a reserved word used as a binding name says what it is reserved for (#206);
  - a handling clause on its expression's line says where it goes (#211).
- **The agent guide and the syntax reference** cover edition 0.2, and every example is checked (#178).
- **The repairs of one check apply together** (#257, PR #265, from the pre-tag review). No two repairs edit the same text, applying them all converges, and none changes a program's meaning:
  - one `[T: C]` edit per type parameter, with the strongest constraint;
  - distinct names for each renamed `let result`, also past a `together` block;
  - each handling clause on a line of its own.
- **An installer** (#240): `install.sh` installs `una`, its runtime and the pinned Z3 into `~/.una`. It does so only when the release's validation record is a PASS, and only with the archive digest the record lists. `--uninstall` removes exactly what it installed.
- **Release gates** (#232): GitHub Actions is not used. `scripts/release-gates.sh SHA OUT` runs seven gates on a fresh clone of one commit:
  - the cold build and tests;
  - local CI on macOS and in a Linux container;
  - both release archives;
  - determinism;
  - the interpreter against native code.

  It records the results as `docs/releases/<version>-validation.md`, which a tag needs (`docs/releases/validation.md`).

### Architecture decisions

The decisions of v0.2, all accepted:
- ADR 0055, native backend;
- ADR 0056, memory model;
- ADR 0057, fixed-width integers;
- ADR 0058, `unsafe` at the runtime's C ABI boundary;
- ADR 0059, release archives ship the native runtime;
- ADR 0060, recursive kinds;
- ADR 0061, a memory limit;
- ADR 0062, effect polymorphism;
- ADR 0063, the language server;
- ADR 0064, the authenticated proof cache;
- ADR 0065, structured concurrency;
- ADR 0066, constraints on generics;
- ADR 0067, position-precise result labels of user generic functions, with its amendment of 2026-10-06;
- ADR 0068, printing structured values;
- ADR 0069, a map type, accepted and not yet implemented;
- ADR 0070, text operations, accepted and not yet implemented.

### Known issues

- **A closed stdout or stderr counts as success** (3c-1, #259). When a write fails with `EBADF` because the stream is closed rather than a broken pipe, both engines exit 0. It is fixed after this release.
- **One same-line handling clause reported per body** (#268). When a function body has several lines with a handling clause on its expression's line, only the first is reported, so applying the repairs takes a check per line. Separate functions each report theirs.
- **A false U3003 for `attempt` over a `fails of` parameter in a relay** (#270). A relay whose only clause is `fails of f` and whose body is `let r = attempt f(2)` is rejected with U3003 when another relay calls it, although no effect is performed. `if error use` instead of `attempt`, or adding `uses of`, avoids it. It is not a soundness issue.

### Known limits

- Native code is generated with Cranelift, and each target is built on its own platform. LLVM release builds are later (`plan/backlog.md`).
- The archives are for macOS on arm64 and Linux on x86_64. Linux on arm64 follows right after this release (#241).
- Constraints have no laws: no axiom states that an order is total or transitive (`spec/20 §5`).
- There are no user traits or implementations, and no map type.
- The point where a run exhausts its memory differs between engines and builds (ADR 0061 rule 5).
- A redacting view of a value, `redact(x)`, is deferred (ADR 0068).
- **Loops that update a list with `set` lose its length** (S-1, #260, ADR 0071). In edition 0.2, `for i in 0..<xs.len` with `set(edit xs, index: i, …)` is U5001, with a counterexample where `xs.len` is 0. Workaround: guard the update with `if i < xs.len`. The fix follows this release.
- **Appending to text in a loop is quadratic** (S-2, #261). `t = "{t}x"` copies the text each time, about n^1.7 in both engines (320,000 appends: 4.4 s interpreted, 2.3 s native). Appending to a list stays linear. The in-place update (ADR 0070 X10) follows this release.
- **A list passed by `edit`, or rebound from a call, in a loop is quadratic** (#262). In `grow(edit acc)` or `acc = push(acc, …)`, the caller still holds the list during the call, so each append copies it, in both engines.
- **Nested loops that append, in the interpreter** (#263). A loop that appends to a list it binds is quadratic in the interpreter when it is inside another loop or a `together` task. Native code stays linear, and the output is the same.
- This release was built and checked locally (`scripts/local-release.sh`), on macOS natively and Linux in a container, while GitHub Actions was unavailable.

## v0.2.0-alpha.3

The third pre-release of v0.2. The native backend now compiles every runnable program of the suite. Edition 0.2 gains the memory model, `for` loops, recursive kinds and effect polymorphism, and the release also brings a language server, a memory limit and an authenticated proof cache. Gate 1's benchmark stays pinned to v0.1.x (v0.1.3).

### Native backend (slices 3 to 6, #113 to #116)

- **Kinds, lists, `Option` and `Result`, the prelude, lambdas and closures** compile natively. Generic functions compile once.
- **Reference counting.** Heap values are reference counted (NB-2), in a heap that `una-rt` keeps in safe Rust. Equality, release and rendering never recurse on the native stack, so a value can nest deeper than any call (ADR 0060).
  - A debug build of the runtime reports, as an internal error, a value left referenced after `main` and any use of a released value. The tests link that build.
- **Failures** are returned values, and nothing unwinds (`spec/16 §4` rule 3). Handling clauses, `attempt`, `checked` and policies compile natively.
- **U6001 natively.** A failure of `main` shows the message of `una run`, with the redaction of labeled fields. The build describes the failure from the error kind that the checker resolved, and Core now carries that resolved kind for functions and capability operations.
- **Capabilities.** Host capabilities, with the interpreter's grants and path rules, handlers and `with` compile as environment passing.
- **Traces.** Native traces equal the interpreter's, with redaction and outcomes.
- **Fixed-width integers** compile natively.
- **Machine widths.** Arithmetic and comparisons whose operands and result are proven to fit a machine word compile to one instruction with no check (NB-3, FW-1).
  - The build report's `native.arbitrary_precision` lists only the `Int` parameters and results with no proven range.
- **The whole suite.** Every runnable program of the examples, the conformance suite and the reviews runs natively with the interpreter's output, exit status and trace, and `tests/native/unsupported.txt` is empty. The release check builds every example natively.
- **Fix: empty text literals.** On x86_64 Linux, an executable could print an empty literal in place of the next literal of the program, because the runtime cached literals by address. Literals are now cached by contents. v0.2.0-alpha.2's Linux archive has this bug.

### Edition 0.2

- **The memory model** (ADR 0056, `spec/17`, #29):
  - `edit` and `take` parameters;
  - `let edit` bindings and assignment;
  - `old(...)` in `ensures`;
  - the codes U2017 to U2021.
- **`for` loops** over lists and ranges (#81). The Core form is `Loop`, and the refinements of the bindings a loop updates are its invariant.
- **Recursive kinds** (ADR 0060, #38): a kind may hold itself through any field, as long as it is grounded (U2002).
- **Effect polymorphism** (ADR 0062, #36): `uses of p` and `fails of p` let a function have the effects and the failures of a function parameter, with the codes U3017 to U3019.

### Specified, not yet implemented

- **Structured concurrency** (ADR 0065, `spec/19`): `together` blocks and `together for`, in edition 0.2.

### Tools

- **`una lsp`** (ADR 0063, #43), a language server over stdio. It offers:
  - diagnostics, and quick fixes ordered by escape cost;
  - formatting;
  - hover, go-to-definition and document symbols;
  - proof counts.

  Tier 1 runs on save and is cancelled by a newer change.
- **A memory limit** (ADR 0061, #45): `--max-memory <MiB>`, 4096 by default, on `una run`, `una test` and native executables.
  - Over the budget, a run aborts with U6003, "this call exceeds the memory limit of N MiB", instead of being killed by the OS.
  - The stack is capped at 1 GiB, whatever `--max-depth` says.
- **An authenticated proof cache** (ADR 0064, #44). Cache entries carry an HMAC tag under a per-user key, `~/.una/proof-cache.key` or the file `UNA_PROOF_KEY` names. A forged entry is solved again.
  - The key, not the cache, is in the Trusted zone.
  - The build report adds `tcb.proof_cache_key`.
- **Local CI and releases** (#156). While GitHub Actions is unavailable, `scripts/local-ci.sh`, with `--linux` for a `linux/amd64` container, runs CI's steps, and `scripts/local-release.sh` builds both release archives.

### Fixes from v0.1.3

- `let b = a` keeps `a` in scope (#139).

## v0.1.3

A patch release of Una Core for the Gate 1 pin, cut from v0.1.2 with one fix. Edition 0.1 only; report schema versions are unchanged.

### Fixes

- **`let b = a` keeps `a` in scope** (issue #139). A `let` whose value is another binding no longer merges into that binding's `let`. Before, `una check` passed such a program, but `una run` and `una test` stopped with U8001 ("`a` is not bound") when both names were used, as in `let b = a` followed by `a + b`. The merge is meant only for the compiler's temporaries. The conformance case `runtime/let_of_a_binding` covers it.

## v0.2.0-alpha.2

The second pre-release of v0.2. It adds the second slice of the native backend and fixed-width integers in edition 0.2. Gate 1's benchmark stays pinned to v0.1.x.

### Native backend (slice 2)

- **What compiles now** (#112): `Int`, `Bool`, `Text` and `Unit` values, arithmetic, comparisons, `if`, `let`, text concatenation and interpolation, and calls between functions that do not fail.
- **`Int` stays mathematical.** A small integer is one machine word, with inline fast paths for `+`, `-` and `*` that fall back to arbitrary precision on overflow. Division and remainder stay Euclidean.
- **Call depth.** Native calls count toward `--max-depth`, as in the interpreter, and exceeding it is U6003 at the call site.
- **The differential test** covers more of the suite. The programs not compiled yet, such as those with records and variants, lists, failures, `with` or fixed-width integers, stay listed in `tests/native/unsupported.txt` with the construct that is missing.

### Fixed-width integers (edition 0.2)

- **Edition 0.2** (ADR 0056): `edition = "0.2"` in `una.toml` enables the constructs of v0.2. Edition 0.1 packages are unchanged.
- **`I8` to `I64` and `U8` to `U64`** (ADR 0057, `spec/18`, #28) are built-in kinds, distinct from `Int`, with explicit conversions (`x.to_u8()`, `x.to_int()`).
  - A literal takes its type from its context. One that does not fit is U2022, a new code.
  - Operands of different integer types are U2001.
  - In clauses, every integer type reads as its mathematical `Int`.
- **Overflow is a proof obligation.** Arithmetic on a fixed-width type and every narrowing conversion must be proven to fit, or the program is rejected with a counterexample. The first repair proposed is a guard.
- **Wrapping, saturating and bit operations** are built-in methods with their argument labeled: `by` for arithmetic and shifts, `mask` for bit operations (ADR 0057, amendment of 2026-10-02). Wrapping and saturating methods have exact contracts; bit operations have partial range contracts.
- The native backend does not compile fixed-width integers yet.

### Specification

- **The memory model** (ADR 0056, `spec/17`), mutable value semantics with `edit` and `take` parameters, is specified but not implemented yet.

## v0.2.0-alpha.1

The first pre-release of v0.2, with the first slice of the native backend (ADR 0055, `spec/16`, #26). It runs in parallel with Gate 1, whose benchmark stays pinned to v0.1.x.

### Native backend (slice 1)

- **`una build [path]`** checks a package at tier 1, compiles `main` of the `[run] entry` module through Cranelift, and links it with the runtime library through the system `cc`. It writes `target/una/<name>`, or the file that `--out` names, and with `--json` it prints `una.build/1`.
- **The executables** follow the contract of `una run`. They take `--grant`, `--trace <file>` and `--max-depth`, check `main`'s capabilities against their grants (U3012) before running, and write the trace in the interpreter's JSON form.
- **What compiles in this slice:** `main` printing text through `Console`, as `examples/hello` does. A construct the backend does not compile yet is a usage error (exit 2) that names it. The next slices add integers and control flow, kinds, failures and capabilities.
- **The differential test** runs every runnable program of the examples, the conformance suite and the reviews in the interpreter and natively, and compares their output, exit status and trace. The programs not compiled yet are listed explicitly (`tests/native/unsupported.txt`).
- **The runtime library** `una-rt` is written in Rust. Its C ABI boundary, `ffi.rs`, is the only code of the repository that uses `unsafe` (ADR 0058).

### Build report

- The report is `una.report/2` (ADR 0055):
  - a section `native` gives the host target, and the `Int` parameters and results that stay at arbitrary precision;
  - the `out_of_model` entry "interpreter stack exhaustion" is now "stack exhaustion".

### Release archives

- Each archive also ships the runtime library, as `lib/libuna_rt.a` beside `una` (ADR 0059). Building natively needs the system `cc`.

## v0.1.2

A patch release of Una Core with agent-facing fixes, shipped before the Gate 1 full run. Report schema versions are unchanged (D-148).

### Information flow

- **An operation argument's sink bounds only its covariant positions** (ADR 0052, issue #84). Covariant positions are the data the caller sends to the handler. An operation whose parameter is a function type with a labeled parameter can now be called with the exact type. ADR 0050's flow still rejects every leaking form of F-001. D-164 is superseded.

### Verification

- **Refutations through nonlinear arithmetic are checked against real arithmetic** (ADRs 0053 and 0054, issues #72 and #104). A product, quotient or remainder of two values is uninterpreted in the encoding. When the model of a refutation disagrees with real arithmetic, the query is solved once more with the arithmetic interpreted.
  - A real violation is `refuted` (U5001), with a counterexample that is always an execution, such as `a * a` as `Pos` at `a = 0`.
  - A model that no execution has gives `unknown` (U5002), with no counterexample. Both statuses reject the program.

### Diagnostics

- **Literal patterns** (issue #73): `is Some(0)` says "literal patterns are not supported in v0.1". The repairs are a second `when` with a literal arm and `else`, or a comparison in the arm. Nested patterns already had their message in v0.1.0.
- **Hints for three common mistakes** (issue #75):
  - `fail` with a stored value;
  - `when` right after `=` in the short form;
  - `if ... then` with no `else`.
- **Reserved words** (issues #75 and #105): a word reserved for a later version gets a rename edit, the same in the whole file. It is capitalized for a kind or capability, and numbered when the file already has the name. In a module path, the repair is advice to move the file.
- **A function with a `requires` passed as a value** (issues #76 and #106): the message names the function value. The only repair is a lambda, or a named helper when the function can fail, that guards every `requires`.
- Every proposed edit checks once applied.

### Review

- A limited review of these fixes (issue #97) gave the verdict Ready for v0.1.2. Its three Minor findings are fixed in this release: V12-001 (#104), V12-002 (#105) and V12-003 (#106).

### Known limitations

- Nested user-generic calls inside a `fold` lambda stay U4001 (D-137, issue #92, v0.2).
- A `pub` declaration renamed by the U0103 edit is renamed per file. A module that uses it may pick another name only when that module already has a name that clashes.

## v0.1.1

A patch release of Una Core. It accepts sound programs that v0.1.0 rejected, found while the Gate 1 tasks were revalidated at the v0.1.0 pin. Report schema versions are unchanged (D-148).

### Information flow

- **Position-precise result labels of the structural prelude functions** (ADR 0051, issue #89). `append`, `first`, `is_empty`, `at`, `is_some`, `is_ok`, `map`, `fold`, `filter` and `first_where` label each position of their result with what it holds. The top label comes only from what decides the result's shape. The other functions of `spec/14 §1` "Result labels" keep the ADR 0038 D3.4 rule, since they read element values. `spec/14 §1` and `spec/07 §7` rule 8 state the rule.
- It unblocks two forms of the same cause:
  - a generic or labeled `fold` that appends to its accumulator, such as `back.fold(initial: front, step: fn(acc, x) -> acc.append(item: x))` inside `concat[T]`, or over `List[secret Int]`;
  - structural operations on records with a labeled field that the code never reads: `is_some`, `append`, `at` and `first_where` no longer label their results with that field's label.
- 67 structural probes of the conformance suite that v0.1.0 rejected are now accepted. They come from the label sweeps and the prelude result cases, and none of them reads an element value. Every leak control stays rejected: a sum or a count by a secret test returned as `Int`, secret elements collected into a `List[Int]`, a predicate or mapping over a secret field, a secret index, and `contains`.

### Review

- A limited review of the change (issue #90, `docs/reviews/`) found nothing above Note, with the verdict Ready for v0.1.1. Its two notes are applied: the wording of `fold`'s top label (the accumulator type's label, joined with the top labels of `xs` and `step`) and a count in the pull request.

### Known limitations

- Nested user-generic calls: a user generic function called inside a `fold` lambda, such as `concat([x], back: acc)` in another generic function, is still U4001. D-137 raises every position of a user generic result, the top included, by the labels of its type arguments (issue #92, v0.2).

## v0.1.0 (Una Core)

The first release of Una Core, the reference compiler of the language, written in Rust. It exists to test the thesis of `docs/plan/roadmap.md` §1 at Gate 1, and it is not ready for production use. The specification (`docs/spec/`) and the accepted ADRs (`docs/adr/`) define the language; this list summarizes them.

### Language and front end

- Lexer with significant indentation, parser and canonical formatter. Non-canonical source is an error (U7001), with mechanical edits that reproduce `una fmt`.
- Modules, imports and `pub` visibility; packages with `una.toml` and `verification.lock`. The module paths `std` and `prelude` are reserved (U1009).
- Kinds: aliases, records, sums with payloads, generics without constraints, and refinements with `where` and range sugar. Refined types are not type arguments in v0.1 (ADR 0040).
- Predicates: `reflect`, which the solver sees, and `opaque`, which is established only inside `checked` (ADR 0042).
- Functions with `requires`, `ensures`, `uses`, `fails` and `derived from`; labeled arguments; method-call syntax; lambdas. Function types carry their effects, failures, refinements and labels everywhere (ADRs 0037, 0038).
- Types: mathematical `Int`, `Bool`, `Text`, `Unit`, `List`, `Option`, `Result` and `Hashed`.

### Authority, errors and information flow

- Capabilities: declarations, `uses`, implicit passing by name, `with` rebinding, handlers and host capabilities (`Console`, `Clock`, `Env`, `Files`, `Keys`). Capabilities and effectful function values are second-class, and never type arguments (U3016).
- Errors: one error kind per function, `fail`, handling clauses, `attempt` and `checked`, with a per-module escape budget.
- Information flow: labels with levels and categories, the program-counter label, sink labels, and declassification policies in `trusted` modules. Generic functions carry the labels of their type arguments, handler operations are checked against their worst caller, and `Keys.hmac` keeps the label of its data (ADRs 0038, 0039).
- `Files` grants never follow a symbolic link in the final component of a path (ADR 0038 D9).

### Verification

- Refinements and contracts are verified in QF_UFLIA with Z3, at the version pinned in `verification.lock`.
- Statuses `proven`, `refuted` and `unknown`, with counterexamples in source names and repairs ranked by where they apply.
- A proof cache keyed by the query text. It is part of the trusted base in v0.1, and the build report counts its results (ADR 0041).
- User names and encoder symbols live in separate SMT namespaces (ADR 0038 D8).

### Running and testing

- Core IR and the Una Abstract Machine, the reference interpreter.
- `una run` with explicit grants, and `una test` for the examples of requirements (`given`, `do`, `expect`, `expect performed`).
- Effect traces with labeled values redacted.

### Agent ABI (schema version 1)

- JSON output for every command, and diagnostics with registered codes and ranked repairs.
- `una explain`, `una contract`, `una obligations`, `una trace` and `una report`, the build report with its four zones.
- 68 diagnostic codes, each with an explanation. Every code except the internal error has a failing and a fixed example.

### Examples and guides

- Seven example packages, each verified with every obligation proven: `events` (the reference example), `greenhouse`, `hello`, `journal`, `pantry`, `shortlinks` and `toolshelf`.
- `docs/guide/syntax-reference.md`, the language guide, and `docs/guide/una-for-agents.md`, a tested reference for agents.
- Release archives for Linux (x86_64) and macOS (arm64), built and checked in CI.

### Proof boundary

Every guarantee has the form *the implementation satisfies its specification, assuming the trusted base behaves as specified and the stated assumptions hold* (charter §3). The build report lists all four zones for every build.

| Zone | Contents in v0.1 |
| --- | --- |
| Proven | Type safety, effect and capability confinement, information-flow noninterference (termination-insensitive), refinement and contract obligations |
| Trusted | The compiler, the reference interpreter, the SMT solver, the standard library, `trusted` modules, host capability implementations, and the proof cache of the package |
| Assumed | Behavior of external systems as declared in `capability` signatures (return types, failures, `ensures`) |
| Human | Adequacy of requirements and contracts: that the specification captures intent |

These channels are outside the model, and the build report says so:
- timing;
- termination and divergence;
- resource consumption and allocation;
- interpreter stack exhaustion;
- external systems beyond their declared signatures;
- races between the check and the use of a file path;
- hard links: a hard link inside a `Files` grant to a file outside it is reachable through the grant (ADR 0048).

### Known limitations

These are deferred, with a target version and a reason for each in [`docs/plan/backlog.md`](docs/plan/backlog.md):
- Effect polymorphism: higher-order prelude functions take only pure, non-failing lambdas.
- Refined elements of collections.
- Label polymorphism beyond symbolic labels on type parameters.
- Authentication of the proof cache.
- A bound on the interpreter's heap: `--max-depth` scales memory.
- Recursive kinds with a base variant.
- Nested patterns, which are reported with a two-step repair.

The memory model, concurrency, laws, cost contracts, native code generation, FFI and the platform layer belong to later versions (charter §5).

### Independent review

- An independent review of v0.1 (`docs/reviews/v0.1-review/`) found 11 Critical, 11 Major and 17 Minor findings and 6 notes. Every Critical and Major finding is fixed with a regression test, and the rest are fixed or recorded in the backlog.
- The review's 143 adversarial programs are part of the conformance suite or the CLI tests. The only exceptions are the programs whose findings are deferred.
- A re-review of the fixes (`docs/reviews/v0.1-rereview/`) found 5 Critical, 2 Major and 11 Minor findings and 6 notes, with 89 new adversarial programs. Every Critical and Major finding is fixed:
  - labels of kind fields at every consumer;
  - per-parameter variance of user generic kinds (ADR 0046);
  - labels of generic function values;
  - a policy's `via` function requires nothing beyond `from` (ADR 0047);
  - refinements at every annotated `let`;
  - refined kinds over function types, rejected in every build (ADR 0045);
  - `expect fails` after a `do` line that cannot fail (ADR 0044).
  Hard links are declared out of model (ADR 0048). Class-level sweeps of label computation, variance and flow sites added a conformance case per covered cell.
- Four issues found by the Gate 1 task builders were handled:
  - an unlabeled first argument binds the first parameter (ADR 0049);
  - the U3002 repair no longer widens an attenuated binding at rank 1;
  - annotated lambda parameters assume their refinement;
  - a refutation through uninterpreted arithmetic says so.
- A limited re-review (`docs/reviews/v0.1-final-review/`) found 1 Critical and 3 Minor findings, with 18 new adversarial programs. The Critical is fixed: an operation argument flows into the operation's parameter type at its contravariant positions and invariant kind arguments (ADR 0050). The three Minor findings are fixed too.
- A last limited review of that round (`docs/reviews/v0.1-final-review/round3/`) gave the verdict Ready for v0.1.0. Its two non-blocking findings are tracked for v0.1.x.
