# ADR 0123 S3: `serve:` grants and TLS (design note)

Issue #1005, part of #897. Implementation plan step 3 of ADR 0123: the `serve:` grant spec, its limits and their defaults, the `Host` rule of the grant, overlap checks, binding before `main`, the TLS files checked on the open descriptor, and the pinned TLS configuration (rules 6 to 9). Base: `origin/main` with ADR 0123 S1 (#1028) and S2 (#1029) merged.

## What this slice does, and what it leaves

| Part | This slice | Later |
| --- | --- | --- |
| The `serve:` spec, items, defaults, ranges, errors | `una-core` (`http::serve`), pure | |
| `ita.una` grant lines for `serve` | `una-syntax` reads them into the text spec, `una-project` checks them (U7008) | |
| `una.toml` | a `serve:` grant is U7002 in every edition (see Decision 1) | |
| `--grant serve:...` | usage errors, exit code 2, in `una run` and executables | |
| Overlap checks | U7008 at the second grant in `ita.una`; a usage error across manifest and flags | |
| The `Host` rule of a grant (`hosts=`, the default, `421`) | `una-core`, pure, over the `Host` that `una-http` reads (S2) | used by the serve threads (step 4) |
| Binding before `main` | a plain listening socket per grant, from `una-sys`, held for the run | the reactor's slice (ADR 0117 step 6, ADR 0123 step 4) takes the sockets over and accepts on them |
| The descriptor limit | raised to the sum of `connections` plus 64 before `main`, else a usage error | |
| TLS files | opened once, checked on the open descriptor (`fstat`), read from it, parsed as PEM | |
| The key matching the certificate, and the TLS server configuration | described in Rust (`TlsPolicy`), not built | pending #1059 (see "TLS provider") |
| `HttpServer`, `[serve]`, U3033, the report, traces | none | steps 5 to 8 |

## Layout

- `crates/una-core/src/http/serve.rs`: `ServeGrant::parse(spec) -> Result<ServeGrant, ServeError>`, the `Limits` with their defaults, `Hosts` and `Hosts::allows(name, ip)`, `overlap(&[ServeGrant])`, and `TlsPolicy::pinned(tls12)`. Pure and shared by the manifest, the checker and both engines, as the `http:` grant is. `http.rs` becomes `http/mod.rs` (`una_core::http::serve`).
- `crates/una-syntax/src/ita.rs`: `serve "<origin>"` grant lines and their item lines, written as the text spec of `spec/13 §5`, with the range of each item line.
- `crates/una-project`: `is_grant` knows `serve:`; `una.toml` refuses it (U7002); `ita.una` checks it item by item, and checks overlaps (U7008).
- `crates/una-interp/src/host.rs`: `Grant::Serve`, whose `certificate` and `key` resolve against the grant's base as other paths do.
- `crates/una-http/src/serve.rs`: `prepare(&[ServeGrantOf]) -> Result<Prepared, String>`, the work before `main`: overlap, the descriptor limit, TLS files, binding. Both engines call it after U3012 and before `main`, where `keys:` files are read.
- `crates/una-sys/src/listen.rs` and `identity.rs`: the system calls (socket options, effective user and groups, the descriptor limit), each `unsafe` block with a `// SAFETY:` comment.
- `crates/una-check`: `granted` maps a `serve:` grant to `HttpServer[next, reply, close]` (rule 6, "a grant allows all three operations"), which has no effect until the capability lands (step 5).

## The text spec

`serve:<scheme>://<address>:<port>` then `;<item>` suffixes (rule 6). The items, with their defaults (rule 7):

| Item | Default | Range |
| --- | --- | --- |
| `N/once` | 256 | 1 to 2^31 − 1 |
| `N/second`, `N/minute`, `N/hour` | none | 1 to 2^31 − 1; at most one rate per grant (Decision 4) |
| `waiting=N` | 1024 | 0 to 2^31 − 1 |
| `connections=N` | 1024 | 1 to 1,048,576 |
| `peer=N` or `peer=none` | `max(16, connections / 16)` | 1 to 1,048,576 |
| `handshakes=N` | 512 | 1 to 2^31 − 1; per peer `max(4, N / 16)` |
| `requests=N` | 1000 | 1 to 2^31 − 1 |
| `line=K` | 8 | 1 to 64 (KiB) |
| `headers=N` | 100 | 1 to 1000 |
| `header=K` | 16 | 1 to 256 (KiB) |
| `body=K` | 1024 | 0 to 1,048,576 (KiB) |
| `buffers=M` | 64 | 1 to 65,536 (MiB) |
| `peer_buffers=M` or `none` | `max(1, buffers / 64)` | 1 to 65,536 |
| `replies=M` | 64 | 1 to 65,536 (MiB) |
| `peer_replies=M` or `none` | `max(1, replies / 64)` | 1 to 65,536 |
| `delivered=M` | 256 | 1 to 65,536 (MiB) |
| `idle=S`, `read=S`, `write=S` | 5, 20, 30 | 1 to 3600 |
| `answer=S` | 30 | 1 to 86,400 |
| `hosts=a,b` or `hosts=*` | IP literals and `localhost` | registered names, a leftmost `*.` |
| `certificate=F`, `key=F` | none | `https` only, and required there; no `;` |
| `tls12` | off | `https` only |

Every derived default is computed from the values the grant gives, after all items are read, so `connections=4096` makes the default `peer` 256. Sizes and counts are held as `u64` and compared with checked arithmetic (rule 7, "Arithmetic").

## `ita.una` form

ADR 0123 rule 7 proposes the lines of `serve`; ADR 0122, accepted without them, leaves their grammar open. This slice fixes it as the closest reading of rule 7's example, one item per line, each mapped to one text item (Decision 2):

```una
run
  grants
    serve "https://0.0.0.0:8443"
      certificate "tls/chain.pem"          -- certificate=tls/chain.pem
      key "tls/key.pem"                    -- key=tls/key.pem
      tls12                                -- tls12
      at most 256 at once                  -- 256/once
      at most 6000 per minute              -- 6000/minute
      waiting at most 1000                 -- waiting=1000
      connections at most 4096             -- connections=4096
      peer at most 64                      -- peer=64
      peer none                            -- peer=none
      handshakes at most 512 per second    -- handshakes=512
      requests at most 1000                -- requests=1000
      headers at most 100                  -- headers=100
      line at most 8 KiB                   -- line=8
      header at most 16 KiB                -- header=16
      body at most 1024 KiB                -- body=1024
      buffers at most 64 MiB               -- buffers=64 (also peer_buffers, replies, peer_replies, delivered)
      idle within 5 seconds                -- idle=5 (also read, write, answer)
      hosts "shop.example", "*.shop.example"   -- hosts=shop.example,*.shop.example
```

A unit word is required where the table has a unit, and only that unit (`KiB` for `line`, `header`, `body`; `MiB` for the byte budgets; `second` or `seconds` for the times), so no line is read in a unit it does not name. Every problem is U7008 at its line: a line of another form, an item given twice, a value out of range, `certificate`, `key` or `tls12` on an `http` grant (at the item), and a missing `certificate` or `key` on an `https` one, or an overlap (at the grant).

## Errors

- **`una.toml`:** a `serve:` grant is U7002 in every edition (Decision 1).
- **`ita.una`:** U7008 (ADR 0122 rule 13 maps U7002 of `una.toml` keys to U7008 in `ita.una`).
- **`--grant`, usage errors, exit code 2:** a malformed spec or item, an item twice, a value out of range, TLS items on `http`, a missing `certificate` or `key` on `https`; a `serve:` grant for a package of an edition before `0.4`, or a single file; an overlap among all the run's grants; an address that cannot be bound; a descriptor limit below the sum of `connections` plus 64 that the run cannot raise; the TLS files of rule 9 (unreadable, not a regular file, too large, malformed, the key file's owner, group or mode); an `https` grant on a platform other than macOS and Linux.

No new diagnostic code: U3033 belongs to step 5.

## Overlap (rule 6)

Each address and port is granted once; a wildcard (`0.0.0.0`, `[::]`) may not share its port with another grant of its family. IPv6 sockets are bound with `IPV6_V6ONLY`, so `[::]:p` and `0.0.0.0:p` do not overlap. The check sorts the grants by family and port and scans each run, O(g log g) for g grants. An address is compared as parsed, and must be written in its canonical form (Decision 3), so one address has one spelling.

## The `Host` rule of a grant (rule 8)

`Hosts::allows(name, ip)` takes the `Host` that `una_http::request::host` read (S2: its syntax is checked on every grant, the name is in ASCII lower case without its port and without one trailing `.`):

- without `hosts=`: `localhost` or an IP literal;
- `hosts=*`: any name of valid syntax;
- `hosts=a,b`: the name equals one of the names, or, for `*.suffix`, is one label followed by `.suffix` (`spec/24 §2`). An IP literal matches no name (Decision 5).

Otherwise `421`. O(n) in the names of the grant.

## Binding before `main`

After U3012 and before `main`, as `keys:` files are read, `una_http::serve::prepare`:

1. checks the overlaps of the final grants (manifest and flags together);
2. raises the soft descriptor limit to the sum of `connections` over the grants plus 64, capped at the hard limit; below it, a usage error;
3. for each `https` grant: opens the key and then the certificate, once each, checks the open descriptor, reads from it at most 1 MiB, parses the PEM (below);
4. binds one listening socket per grant (`una_sys::listen`): `SO_REUSEADDR` set, `SO_REUSEPORT` never set, `IPV6_V6ONLY` set on IPv6, close-on-exec, backlog `min(connections, SOMAXCONN)`; an address that cannot be bound is a usage error that names it.

The sockets are held in `Prepared` until the run ends, and nothing accepts on them yet: the reactor's slice (ADR 0117 step 6, ADR 0123 step 4) takes them over, sets them non-blocking and registers them, and the serve threads accept. Until then a peer's connection waits in the kernel's backlog. No program of this slice can name `HttpServer` (step 5), so no program's behavior depends on it.

## The TLS files (rule 9)

- **The key file.** Opened once with `O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOCTTY`, following symbolic links (secret volumes are links); `O_NONBLOCK` keeps a FIFO from blocking the open. Then `fstat` on the descriptor, never the path: a regular file; owned by the effective user or by root; `st_mode & 0o027 == 0`; the group read bit only when the file's group is the effective group or a supplementary group (`getgroups`). The key is read from that descriptor, at most 1 MiB, and the buffer is cleared after parsing.
- **The certificate.** Opened and read the same way from its descriptor, which must be a regular file of at most 1 MiB (Decision 6); its owner and mode are not checked, since it is public.
- **PEM.** The chain is the `CERTIFICATE` blocks, leaf first, at least one; the key is the first private key block, PKCS#8, SEC1 or PKCS#1, through the PEM reader of `rustls-pki-types` (re-exported by `rustls`, no new crate, no provider). A file with no such block is malformed.
- **The key stays in the host.** `Prepared` holds the chain and the key; its `Debug` prints neither, and no message names the key's bytes.

### TLS provider: pending #1059

The project's use of `ring`, the provider that `rustls` uses for the client, is an open question (#1059). Until it is decided this slice binds no TLS provider and builds no `rustls::ServerConfig`. What waits for #1059:

- checking that the key matches the certificate (a usage error);
- building the server configuration from `TlsPolicy`;
- every live TLS test of the Conformance section of ADR 0123.

`TlsPolicy::pinned(tls12)` describes the configuration that step builds: TLS 1.3 only, TLS 1.2 added only by `tls12` and then with the extended master secret required; no early data; no stateless tickets, resumption through a bounded in-memory session cache only; a key log that writes nothing; ALPN `http/1.1` only; no client certificates. A unit test pins every field.

## Decisions to confirm

1. **`una.toml` refuses `serve:` in every edition (U7002).** In editions `0.1` to `0.3` it is an edition error (rule 0). In edition `0.4`, `una.toml` is itself U7002 (ADR 0122 Question 8.1), so a `serve:` grant in it is never read; it is reported as well, with the same message. `ita.una` is the manifest form, with U7008 (ADR 0122 rule 13), not U7002.
2. **The `ita.una` grammar of `serve` lines** (above), extrapolated from rule 7's example to every item. It needs a dated amendment of ADR 0123 or ADR 0122 to become normative beyond `spec/31 §2`, which this slice writes.
3. **Canonical addresses only.** An address must be written as Rust's `Ipv4Addr` and `Ipv6Addr` print it (`[::1]`, not `[0:0:0:0:0:0:0:1]`); IPv4-mapped IPv6 addresses (`[::ffff:127.0.0.1]`), multicast addresses and zones are refused, since a socket with `IPV6_V6ONLY` cannot receive IPv4 on them and a multicast address cannot listen. Ports have no leading zero.
4. **One rate per grant.** ADR 0117 rule 8 refuses "a kind of limit given twice", and rule 19 keeps "the GCRA timestamp of the rate", one per grant, so `N/second` and `N/minute` together are refused rather than one being silently dropped.
5. **`hosts=` holds registered names only**, by the syntax that S2 checks in `Host` (letters, digits, `-`, `_`, `.`, no empty label, a last label that is not numeric), with an optional leftmost `*.`, or `*` alone; IP literals, ports and duplicates are refused. Matching is in ASCII lower case; one trailing `.` of a listed name is removed.
6. **The certificate file is a regular file of at most 1 MiB,** and so is the key: a FIFO or a device cannot stall or flood the start of a run.
7. **Values have no leading zeros or signs**, in the text spec and in `ita.una`, so one value has one spelling.
8. **`SO_REUSEADDR` is set** on the listening socket, as the standard library does, so a server restarts while old connections are in `TIME_WAIT`; it never lets another process share an active listener on Linux, and `SO_REUSEPORT` is never set (rule 19).

## Tests

- Conformance (`tests/conformance/project/serve_*`, edition `0.4`, `ita.una`): every item's form and range, the defaults (a grant with no items is accepted), each TLS rule of the grant, overlaps, `una.toml` refusals in editions `0.3` and `0.4`.
- Unit tests: `una-core` (parse, defaults, derived defaults, ranges, errors with their item index, overlap, `Hosts::allows`, `TlsPolicy`), `una-syntax` (lines to text spec), `una-project`, `una-http::serve` (key file modes, owners, links, FIFOs, sizes; binding, overlap, the descriptor limit; ports from the kernel, no fixed port), `una-sys`.
- `una-cli` integration (`crates/una-cli/tests/serve_grants.rs`): usage errors for `--grant`, edition refusals, binding failures, TLS file refusals, in `una run` and in an executable.
