# The generators and mutators of the security hunt

- Issue: #513, part (a) of `docs/plan/security-hunt.md`
- Date: 2026-10-10
- Status: design (L2), no code yet

## 1. What they produce

Every program the generators write is a small package:
- a `una.toml`, with `[labels]`, `[trusted]` and `[run]` as the shape needs;
- `src/` modules;
- for a source-level variation of the oracle, the positions of its varied literals.

Each program carries an operator id from §4, the `spec/07` rule it targets, a seed, and an **expected verdict class**. The class decides which oracle judges it:

| Class | Meaning | Judged by | A finding when |
|---|---|---|---|
| **must-reject** | the operator injects a flow that `spec/07` forbids | `una check` alone | `una check` accepts it |
| **must-keep** | the operator rewrites an accepted or rejected program without changing any label, flow or effect | `una check` alone | the verdict changes: rejected to accepted is a candidate leak; accepted to rejected is an over-strict checker |
| **open** | the operator writes a program the spec allows or forbids by rules the generator does not decide | the re-check (b), the reverse (c), the two-run oracle (e) | as those parts say |

The first two classes are two more cheap oracles. They need only `una check`, about one run per program, so they are the first campaigns. A must-reject program that is accepted also goes to the two-run oracle, which turns it into a concrete witness, or shows that the flow is not observable.

## 2. How programs are written

- **Generators, not text surgery.** Most operators are templates with holes: a hole is a type, a label, an expression of a given type and label, a statement, a sink, a branch, a wrapper. A small Python model of the surface syntax (`spec/02`) fills them and prints canonical text. So a program parses by construction, and `una fmt --check` is a test of the printer.
- **Mutators over the corpus.** For part (b) and the must-keep class, mutators start from the conformance programs and the examples that `una check` accepts:
  1. They first run `una fmt` on them, so that the canonical form (`spec/15`) makes line-level edits reliable.
  2. A mutator edits declarations, statements and expressions by their canonical layout.
  3. A mutant that does not parse is dropped and counted against its operator, never judged.
- **Inside the importer's vocabulary**, for part (b). A mutator that targets the re-check uses only what the importer reads: `if`, `case` on `Option` and `Result`, `attempt`, `handle`, loops, `Files`, `Http`, `Env.get`, one other operation, generics, `map`, `filter`, `fold`, `first_where`, B1 policies, and the primitives the model has. Otherwise its programs come back "not re-checked", and they belong to the oracle instead.
- **Both editions.** Each operator states its editions. A program is written once per edition it applies to, since the rules differ (edition 0.1's defense in depth, ADR 0067's summaries).
- **Code.** Python 3, the standard library only, in `scripts/security-hunt/`. Programs and evidence go to a directory outside the repository, under the oracle's temporary root. Nothing generated is committed except minimized repros of findings, which follow the S-xx process.

## 3. Values

Literals that the two-run oracle varies are written behind an annotated binding, `let s: secret T = <literal>`, at the start of the function that uses them. That keeps the variation a single edit. The values are those of `docs/plan/security-hunt-oracle.md` §2.

## 4. The operators

Each family targets one row of `docs/reviews/security-hunt/coverage.md`. The families come in the order of its §2: trusted modules and policies first, since the model assumes every policy is lawful.

### T. Trusted modules and policies (`spec/07 §5`, §6, §7 rule 5)

| Id | Program | Class |
|---|---|---|
| T1 | A policy in a module whose header says `trusted` and that `una.toml` does not list; and the reverse | must-reject (U7005, U4004) |
| T2 | A policy in a module that is neither | must-reject (U4004) |
| T3 | The listing in `una.toml` names the module by another spelling: a trailing space, another case, a Unicode lookalike, a file path instead of a module path, a duplicate entry, a module that does not exist | must-reject (U7005), or a usage diagnostic |
| T4 | The `via` function reached other than through its policy: called directly; named as a value; given to `map`, `fold` or `first_where`; stored in a record field; returned by a function; wrapped in a lambda; passed through a generic identity function; called from a handler body; from a `together` task; from another module through an import | must-reject (U4005) |
| T5 | The `via` function declared `pub`, or exported through an alias or a re-export | must-reject (U4005) |
| T6 | The `via` function in another module than its policy | must-reject (U4005) |
| T7 | Types that differ beyond labels: a refinement on one side only, two refined kinds, a record, a generic type, a function type, a `requires` clause | must-reject (U4005) |
| T8 | Policies the spec allows but B1 does not: a non-scalar `to` (a record, a list, `Option`), a `via` function with effects (`Console`, `Files`, `Keys`), a `via` that calls another function, one that calls another policy | open: the two-run oracle. A difference the policy's `to` explains is not a finding; one visible below `to` is |
| T9 | Two policies with one `via` function; two `via` functions with one name in two modules | open |
| T10 | A policy call under a raised pc, in a lambda, in a handler body, under a symbolic pc in a generic function, in an `of` argument, in a `together` task | must-reject when the pc is above `to` (U4002), else open |
| T11 | A policy whose `via` function fails (S-18): `attempt`, `handle`, `if error pass`, `use` and `fail` around the call, with `to` at `public`, `internal` and `secret`, at a public and a raised pc | must-reject or open, by ADR 0090's rule |
| T12 | Two policies composed: the result of one given to the other, `to` lower than the second's `from`, and the reverse | open |

### H. Handlers and the effect floor (§7 rule 4)

| Id | Program | Class |
|---|---|---|
| H1 | An operation body that branches on its parameter, raised by `accepts`, then performs an effect, calls a function with effects, or fails | must-reject when the floor forbids it (U4003, U4002) |
| H2 | An operation body whose result carries its parameter into a result type labeled lower | must-reject (U4001) |
| H3 | `with` installing a handler under a raised pc, and a handler installed public whose operation a secret branch calls | open |
| H4 | A user handler of a host capability (`Keys`, `Files`, `Console`) that breaks the worst caller's rule | must-reject |
| H5 | A handler whose operation fails, with `attempt` and `if error` at the caller under various pcs | open |

### K. `Keys.hmac` (ADR 0039)

| Id | Program | Class |
|---|---|---|
| K1 | A key name that is `secret` or `internal`, or computed under a raised pc | must-reject (U4003) |
| K2 | The result lowered without a policy: through a field, `.len`, an equality, an interpolation (`<hashed>`), `to_text`, a list, `Option` | must-reject where the label falls (U4001, U4003) |
| K3 | `UnknownKey` decided by a key name taken from `Env.get` or `Files.read` (`internal`) | must-reject |
| K4 | `data` with deep labels: a record with a `secret` field, a list of `secret[PII]` | open: the two-run oracle, with V-host varying the key material |

### G. Kinds with type parameters, and invariance (§7 rule 2, ADR 0046)

| Id | Program | Class |
|---|---|---|
| G1 | A kind `Box[T]` whose field is a function of `T`: put a `secret` in and take it out as `public`, through each flow site of rule 2 | must-reject (U4001) |
| G2 | The same with a covariant kind, where the flow is allowed | open: it should be accepted; the two-run oracle checks that it does not leak |
| G3 | Recursive and mutually recursive kinds, with the parameter reaching a function type at the second level | must-reject or open, by rule 2's least solution |
| G4 | A kind parameter instantiated at a function type with effects | open |

### P. Failure payloads (§1, §3 rule 3)

| Id | Program | Class |
|---|---|---|
| P1 | A failure whose payload carries a `secret` field, passed with `if error pass` to `main` | open: U6001 prints only `public` fields, and the oracle compares them |
| P2 | `if error use e` that reads the payload, under a public pc, into a sink | must-reject when the field is above the sink |
| P3 | A payload chosen by a secret branch | must-reject (U4002: a failure under a raised pc) |

### C. `together` (`spec/19`)

As built (`hunt/families/concurrency.py`); in edition 0.1 every program is must-reject U0103, which checks the edition gate.

| Id | Program | Class |
|---|---|---|
| C1 | The number of tasks decided by a `secret` value (a `secret` list, a range with a `secret` bound), with a sink, a failure, or the result's length in a public position | must-reject (U4003, U4002, U4001); twins over a public list of `secret` elements are open |
| C2 | A task's own `secret` element decides a failure or a sink; a branch's binding keeps its label after the block | must-reject |
| C3 | A block under a `secret` program counter whose branch calls a function with effects, or nests a `together for` with a sink | must-reject (U4002, U4003); a public-pc twin is open |
| C4 | Cancellation seen by a sibling: one task's failure is decided by an `internal` value (`Http.get` on an `internal` URL) and passed out of the task, while a sibling task, or code after the block, has a public effect | must-reject (U4002); an `attempt` twin, where the block cannot fail, is open |
| C5 | Probes of C4's premise that a user operation's failure cannot depend on a `secret` argument: a handler that reads its argument through `Http.get` or `Files.read` and fails on the result, and a handler that always or never fails chosen by `with` under a `secret` condition | must-reject |

Not generated: the relative order of the effects of different tasks, and how far a cancelled task got, depend on timing, which is outside the model (`spec/07 §1`, `spec/19 §3` rule 3). The two-run oracle compares each task's events alone, never their interleaving.

### B. Authority boundaries (ADR 0093, `spec/05 §4`, `spec/07 §6`)

As built (`hunt/families/boundaries.py`, #813). `[boundaries]` is a table of edition 0.3, so the fixed run generates B in editions 0.2 and 0.3 (`OPERATOR_EDITIONS`): in 0.2 every program is a must-reject probe of the edition gate, an unknown table (U7002), with a parse error (U0101) where it narrows a binding (`@console[print]`), and nothing of it runs.

| Id | Program | Class |
|---|---|---|
| B1 | A call into a boundary module that takes a capability from the caller by name (U3020), grants a binding, a functional argument or a handler beyond the ceiling (U3021), uses a function of it as a value without a grant (U3020), installs a handler of it whose `uses` the installer's header would give by name (U3020), or reaches a release through its private helpers (U3024) | must-reject |
| B2 | Calls into a boundary granted within its ceiling (a handler, a narrowed rebinding `@console[print]`, a function of it as a value), while the caller holds a secret that reaches only an operation that accepts it | open, for the oracle; a closed call grants a public operation into the boundary from a function called under a secret branch (U4002), which the oracle sets aside |

### Q. Conditions on allowed effects and contract locks (ADR 0092, `spec/05 §3`)

As built (`hunt/families/conditions.py`, #813), in editions 0.2 and 0.3 like B: in 0.2 every program is a must-reject probe of the edition gate, `where` in a `uses` (U0101) or an unknown table (U7002).

| Id | Program | Class |
|---|---|---|
| Q1 | What tier 0 rejects: a condition in a function type (U0101), with `result` (U2012), `it` (U2013), an operation (U2014) or an opaque predicate (U2016); a conditioned function used as a value (U3006); a header that grants an operation its `[contracts]` lock does not list, with a condition or without one (U3023); a condition in a `[boundaries]` ceiling (U7002), in a package where everything else resolves (the gate's `ceiling-control` canary) | must-reject; in edition 0.2 the plain lock is the probe of `[contracts]` alone (U7002) |
| Q2 | What only the full check decides: a conditioned operation used where its condition may be false, directly, in a lambda, through an unconditioned helper, through a callee that promises less, or after reassigning an `edit` parameter (U5001); a header condition that does not imply its lock's (U3023, through the solver) | residual, with an expected code: tier 0 accepts them, and the full check, with the pinned solver, must reject each with its code and no other (`judge_residual`); an acceptance is a finding, a rejection by another code an error. Twins (guarded, under `requires`, a callee that promises the condition) are residual rows the full check must prove (`prove`): a rejection of one is an error |
| Q3 | A conditioned public operation whose condition is on a public parameter, while a secret goes to an operation that accepts it | open, for the oracle; a closed call puts the condition on a secret, so the public operation runs under a secret branch (U4003), which the oracle sets aside: a checker that took the condition for a guard would let the number of public events show the secret |

Not generated yet (#813's second part): units (ADR 0104), refined elements (ADR 0103), U3025 and U3026 (ADR 0100), and an edition 0.3 sample of every older family.

### D. Authority, information flow and trust across packages (ADR 0122 rules 49 to 53, `spec/30 §12` and §13)

As built (`hunt/families/dependencies.py`, #1001). Each program is a root package of edition 0.4, described by `ita.una` and pinned by `ita.lock`, with one dependency: a path package under `deps/`, or, for the forms this release mounts from a registry, a release of the registry `acme` vendored under `vendor/ita/` with its provenance (`spec/30 §9` rule 8, §14 rule 10). The hunt writes every digest, signature, proof and lock line itself (`hunt/itaty.py`, `hunt/packages.py`). `ita.una` is a file of edition 0.4, so the fixed run generates D in editions 0.3 and 0.4: in 0.3 every program is the edition gate's probe, a root whose `ita.una` names 0.3 (U7008), and nothing of it is built but the root and a path dependency.

| Id | Program | Class |
|---|---|---|
| D1 | A call into the dependency that takes a capability by name (U3020), grants a binding or a functional argument beyond the root's ceiling (U3021), a ceiling wider than the lock records or one it does not record (U7009), from a path or from the registry; and what a root may not reach or write: a dependency importing the root (U1001), a root module under the dependency's local name (U1010), a function that is not `pub` (U1004), a root `trusted` line naming a dependency's module (U7008, U7005), a dependency with needs, labels or boundaries of its own, a ceiling naming a dependency's own capability, a path package named otherwise than its need, a reserved or look-alike local name (U7008), and a local package without `accept name` (U7012) | must-reject |
| D2 | The root holds a secret and passes it to the dependency's functions, plain, generic or with a functional argument, and gets it back still secret, while only public text reaches the dependency's `Console.print`, granted within its ceiling | open, for the oracle, which compares these programs on the host's `accepts` alone, since a program of several packages has no Core export in this release (`una.core/2`, `spec/30 §12` rule 7); closed calls: the dependency printing the secret (U4003), returning it as public (U4001), and a call that prints granted under a branch on the secret (U4002), which the oracle sets aside |
| D3 | A path dependency whose trusted module declares a policy the root uses: trusted by a trust line and the lock's consent to the package's whole content (`honest`), and refused without the trust line, without the consent, with a consent to other content (the trusted module changed, or another file), with a trust line that leaves a trusted module out, names another major line or a registry source, with the lock's trusted modules not the package's (U7012), and with a grant into the trusted extent beyond the intersection of the root's two ceilings (U3021) | must-reject; `honest` is a must-accept control |

### Y. Registry packages: digests, provenance, vendoring, yanks and resolution (ADR 0122 rules 16 to 41, `spec/30 §9` to §11 and §14)

As built (`hunt/families/registry.py`, #1001), in editions 0.3, the probe (U7008, with no registry built), and 0.4. Each program has a registry `acme` that the hunt builds itself (`hunt/itaty.py`): root metadata, the owner `acme`, releases, yanks and checkpoints cosigned by three witnesses at threshold two, with a pure-Python Ed25519 (`hunt/ed25519.py`, checked against RFC 8032's vectors). It is an implementation of the formats of its own, so an honest program that the client in `una` refuses, or a forged one it accepts, is a finding either way. The must-accept programs of D and Y are the controls of their forgeries: the gate makes a control that the checks reject an error, not a note.

| Id | Program | Judged by | Class |
|---|---|---|---|
| Y1 | Releases of `acme/ledger` on one line, and of another line and another name, some yanked or avoided, some logged inside the cooldown, a pre-release; the hunt resolves the root's need by minimal version selection with the raises of `spec/30 §10` rule 4 and writes the lock. The check accepts that lock (`honest`, `honest-raised`) and must refuse one that pins a yanked, avoided, newer, pre-release or cooling release, or gives another digest or log index (U7009), and a need with no resolution (U7013) | `una ita lock --check`, at the lock's resolution point, the registry served as a mirror that the run's own Itaoca lists (`Program.command`, `drivers.lock_check`) | must-reject; the two honest forms are must-accept controls |
| Y2 | The release vendored with its provenance, the root importing its module. The check accepts the honest copy, and must refuse (U7010): a file changed, added (a foreign source, a build script, `.DS_Store`), or removed; the provenance missing or not in its one encoding; a key other than the lock's; a statement re-signed by another key; a point cosigned by one witness, by a forged witness key, or by one witness under two names; a checkpoint signed by a forged log key; root metadata other than the pin, or a next version signed below its threshold; the lock's point or log index moved; an inclusion proof changed; files and digest changed together; a statement whose `manifest` is not the archive's; a root expired at the point's witness time; a point cosigned by a witness a later root version revokes; and a point signed under an older root version with fewer cosignatures than the last version's threshold | `una check` | must-reject; `honest` is a must-accept control |
| Y3 | Y2's vendored release with a provenance whose later root versions rotate witnesses or change the witness threshold (`spec/30 §9` rule 7, §11 rules 8 and 9). The check accepts the controls: a witness retired without revocation, with another cosigning (`honest-retired`), two retired over two versions (`honest-two-retired`), one revoked with the two others cosigning (`honest-revoked-enough`), a threshold raised from one to two over two cosignatures (`honest-raised-enough`), raised to three then lowered to two (`honest-raised-lowered`), and a witness of the last root cosigning the first version's point (`honest-new-witness`); and must refuse (U7010) a witness revoked then another retired, a witness retired then revoked by a later version, and a threshold raised to three over two cosignatures. Every form's provenance carries the latest checkpoint signed under the last root version and the consistency proof that the point is its prefix (ADR 0122, amendment of 2026-11-03). The forgeries of the proof and the chain: the retired witness's point in a provenance of the format `ita-provenance/1` (`retired-format-1`) or of the format 2 with no proof (`retired-no-proof`), unavailable (U7013); an edited consistency hash (`consistency-tampered`); the later root version and the proof dropped, with the lock's `last root` kept (`truncated-lock-kept`) or edited too, below the root-version floor the run's own Itaoca stores (`truncated-below-floor`, the program's `floors`), refused (U7010). A unit test counts each provenance's witnesses by the spec's rule apart from the builder and the client, a retired witness's line counting only with the proof | `una check` | must-reject; the `honest-` forms are must-accept controls, the honest twins of Y2's and Y3's rotated forgeries |

### S. User sum types, and `is` / `in` (§3 rule 2, §7 rule 3)

| Id | Program | Class |
|---|---|---|
| S1 | `when` on a `secret` sum value, with an effect or a failure in an arm | must-reject (U4003, U4002) |
| S2 | `is` and `in` on `secret` values into a public sink | must-reject |
| S3 | A variant with a `secret` field, matched and printed through each arm | must-reject where the field reaches a sink |

### O. `Ord`, `sort`, `min`, `max` (ADR 0066)

| Id | Program | Class |
|---|---|---|
| O1 | `sort` of a list mixing labels, then the length, the first element, or an equality on the result, into a sink | must-reject at the join |
| O2 | `min` and `max` of a `public` and a `secret` value | must-reject |
| O3 | A generic function constrained by `Ord`, whose result position a comparison chooses | must-reject at a public sink, and open for (b) |

### F. The covered fragment, for part (b)

These operators stay inside the importer's vocabulary, so their programs are re-checked:

- F1. Raise a literal behind `let s: secret T` and let it flow on unchanged. The program is then either rejected by both `una check` and the re-check, or accepted by both.
- F2. Wrap a statement in `if s` on a `secret` condition.
- F3. Wrap a call in `attempt`, `handle`, `if error pass`, `use` or `fail`.
- F4. Route a value through `map`, `filter`, `fold`, `first_where`, a lambda, a generic function, a record field, `Option`, `Result`, or a tuple.
- F5. Put a `Files.write` or `Files.read` under a raised pc, in a lambda, in recursion, or as a value. S-14's shapes.
- F6. Vary an `Http` call's URL, headers and body labels. S-15's shapes.
- F7. Route a secret through the prelude functions the re-check reads since #532, #542 and #545: `at`, at a public and at a secret index, `or_else`, with a secret value and with a secret default, `min`, `max`, `abs`, `slice`, and `is_ok`, of a public result of a secret (structural, so public) and of a result chosen under a secret condition (so secret). A table of its own, so that F1's programs for earlier seeds rebuild unchanged.

The class is open, but for F1's and F7's endings that reveal the secret at `public` (a public result, a print, a print under the secret): those are must-reject, so `una check` alone decides them whatever the re-check says, and F7's structural `is_ok`, whose result is public, is must-accept there (the review of #547). The judge of the rest is the comparison: a declaration that `una check` accepts and the re-check rejects is a candidate, and the program's own declaration not re-checked is `unknown`.

### N. Servers (ADR 0123, `spec/31`, #1011)

Edition 0.4. Each program is a server in the language form (`spec/31 §1` rule 6) and a script of connections (`spec/31 §8` rule 6) that stands for its peers under virtual time, so no run binds a listener or reaches a network. They are not judged by `una check`: `hunt servers` (`scripts/security-hunt/hunt/serving.py`) runs each one twice with `--serve-script` and `--serve-out`, and judges what the peers received against the expectation the generator wrote (`Program.serve`). The family is kept apart from the operators of the release gate's fixed run (`SERVER_OPERATORS`), with a seed of its own, 1011.

**Malformed requests** (N1 to N4). The server answers each request it is given `200` with `seen <target>`. Each program sends one request, malformed by one rule or a well-formed twin of it, then `GET /behind` on the same connection, which a peer that smuggles wants read. A refusal must be the host's own answer alone (its status, `Content-Length: 0`, `Connection: close`, the side shut), and the request behind it never answered, so the program sees neither; a twin is answered `200`, the request behind it too unless the first closes the connection. The same bytes go again on a second connection, split at one to four random byte boundaries over several instants, and must get the same bytes back but for `Date` (`spec/31 §3` rule 10); a third connection then sends a valid request, which must be answered, so no malformed request ends the server. The program's `reply` events must count exactly the requests delivered.

| Operator | Rules | Forms |
|---|---|---|
| N1 | `spec/31 §3` rules 1 and 2: the request line and line ends | twins `plain`, `HTTP/1.0`; a double space, a leading empty line, a tab for a space, a fourth empty part (400), a method not implemented, `CONNECT`, `TRACE` and lower case included (501), a method that is not a token (400), a version other than `HTTP/1.1` and `HTTP/1.0`, `http/1.1` and a tab after it included (505), a target byte outside 0x21 to 0x7E (400), a line over `line` (414), a bare LF ending the request line or a header line, a CR not followed by LF (400) |
| N2 | rules 3 and 6: header lines, encodings, expectations | twins with trimmed and tabbed values, `Connection` listing connection headers, `Content-Encoding: identity` in any case, `Expect: 100-continue` (`100` first); obsolete folding, whitespace before the colon, a name that is not a token, a control byte or a value that is not UTF-8, a `Connection` naming another header or holding an empty element, `_` for `-` in a framing or connection name (400), another coding (415), another expectation (417), more lines than `headers` or bytes than `header` (431) |
| N3 | rule 4, rule 8: framing, the defense against smuggling | twins framed by `Content-Length` and by `chunked` in any case, with upper-case sizes and trailers; both headers, `chunked` in `HTTP/1.0`, two `Transfer-Encoding`, a malformed coding list, two `Content-Length` even equal, a length with a sign, a list, hex, 20 digits or a fraction, a chunk size with an extension, whitespace, a sign, `0x` or 17 digits, chunk data not followed by an empty line, a body that is not UTF-8 (400), a coding not implemented (501), a length or a chunk over `body` (413) |
| N4 | rule 5, `spec/31 §4`: `Host` and the target, path confusion | twins with segments other servers read as special (`..;`, `%252e%252e`, `//`), queries holding an encoded CRLF, the absolute-form on the grant's own authority, `Host` in other cases, with a trailing dot, `[::1]` and a port; `Host` missing or twice, malformed (`a/b@c`, `127.1`, `0x7f`, an empty or large port, a zone, an empty label) (400), a DNS name the grant does not list (421), the asterisk, authority and foreign absolute forms, a character RFC 3986 does not allow, a dot segment raw or encoded, a segment that decodes to `/`, `\`, NUL or a control byte, a bad or non-UTF-8 percent-encoding (400) |

**Replies with injected headers** (N5, `spec/31 §5` rule 1; ADR 0123 Security, "Header injection and response splitting"). The handler echoes the decoded value of a request's first query parameter into a header of its reply, as a server that echoes a client's value does: decoded, it can hold what the request line cannot, a CR, an LF, a NUL or another control byte. A header's name is `public` (`Header`, `spec/24 §1`), so a request cannot choose it without a policy (the check says U4001), and the forms on names write the name in the program, as a server's own mistake. A reply the host must refuse fails with `BadReply`, sends nothing and keeps the reply right: the handler then answers `502` with the failure's name (`fallback`), or leaves the right to the host's `500` (`auto`). A finding: any other reply, more than one reply on the connection (response splitting), or the injection's marker anywhere on the wire. Twins echo a value the host must send as given (spaces, a tab, `;`, `=`, UTF-8, `Set-Cookie` and `Content-Type` as names), which must reach the wire once, byte for byte.

| Operator | Rules | Forms, each in both modes |
|---|---|---|
| N5 | `spec/31 §5` rule 1 | the twin `echoed`; a value with CRLF, with a bare LF or CR, with NUL, another control byte or 0x7F; a name the host writes itself (`Content-Length`, `Transfer-Encoding`, `Connection`, `Date`, `Keep-Alive`, `Proxy-Connection`, `TE`, `Trailer`, `Upgrade`, in any case); a name that is not a token (a space, a colon, a trailing space, a CRLF, non-ASCII, empty); a body on a `204` or a `304` |

The `reply` events of N5 are not counted: the fallback replies under a program counter its failure raises, quiet code whose operations append no event (`spec/11 §5`, `spec/26 §6` rule 4); the wire is the judge.

The run takes the first 40 seeds of each operator and then, for each form those did not reach, the first later seed that has it, so every form runs (203 programs). Measured on an Apple M1 Max with a debug test build of `una` at a load average of 7 to 46 from other work (2026-11-09): 203 programs in 10 seconds, and 750 with seed 3, 150 per operator, in 35 seconds, all clean.

### R. Refactors that keep every label (must-keep)

Each is applied to accepted and to rejected programs, and must not change the verdict. They rewrite the root package's own modules, under `src/`, only: a dependency's files are pinned by a consent's digest, a vendored copy's digest and a registry's statements, so a rewrite of them would change a verdict by design (#1001).

- R1. Inline a `let` binding with a declared type, or introduce one, with the same label.
- R2. Extract an expression into a private function with the same declared types and labels, or inline such a function.
- R3. Pass a value through `id[T](x: T) -> T = x`.
- R4. Eta-expand a function value into a lambda, or eta-reduce one.
- R5. Swap the branches of an `if` and negate its condition.
- R6. Rename bindings and functions consistently.
- R7. Move a declaration to another module of the package, and import it.
- R8. Add a binding that nothing uses, with a `public` value.
- R9. Reorder independent declarations.

An exception is known in advance. Rule 8's edition 0.1 defense makes R3 and R4 change verdicts on purpose: an annotated or `let`-bound lambda accepts no decision. Those pairs are excluded by construction, and the exclusion is listed with the operator.

## 5. Order of the campaigns

1. **must-reject and must-keep** of the T family, then H, K, G, P, C, S, O. They need `una check` only, at about one run per program.
2. **Part (b)**, with the F family and the R family's accepted programs.
3. **The two-run oracle** on everything accepted: first the open T8 to T12 programs, then the other families, then the examples under V-host.

Each campaign is heavy work and takes a machine slot between benchmark runs (condition 3 of its approval). The first slot measures the cost on 500 programs before a full round is planned.

## 6. What is kept

Each campaign writes a table of its results. A row holds:
- the operator, the rule, the edition and the seed;
- the class;
- `una check`'s verdict and diagnostics;
- the re-check's outcome;
- the oracle's outcome.

A row that is a finding also gets its program, its minimized form and the runs' evidence. Counts per operator give the coverage that the campaign reached, and the share of mutants that did not parse.
