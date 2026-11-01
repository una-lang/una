# 0133. Own cryptography for TLS: a pure-Rust provider in place of `ring`

- Status: Proposed
- Date: 2026-11-01
- Deciders:
- Resolves: issue #1100 (part of epic #1097); slice 1 is implemented with this record
- Related:
  - ADR 0074 (the HTTP client), which chose `rustls` with the `ring` provider and records that `ring` "holds assembly and C" (its Decision and Consequences), and its residual on macOS (item 7);
  - ADR 0124 (one binary), whose embedded Z3 is the one foreign-code exception of epic #1097;
  - ADR 0122 rule 38 and `docs/security/tcb.md` (Ed25519 for the registry client is already pure Rust);
  - ADR 0119 (`una-hash`, `getrandom` as the operating system's random source);
  - ADR 0058 (`unsafe` at the runtime boundary);
  - `docs/compiler/architecture.md §7` (the allowlist of external crates);
  - issue #1093 (own big integer), issue #1092 (symbol check on shipped artifacts).
- Target: the toolchain and the runtime (`una-http`, so `una`, `una-rt` and every program that uses `Http`). No construct of the language changes, so no edition changes (`spec/13 §7`).
- Amends, on acceptance and with the last slice: ADR 0074 (the TLS stack: the provider), `docs/compiler/architecture.md §7`, `docs/security/tcb.md` (the trusted base and the reference to `ring`).

> **In plain words.**
> - **Why.** `ring` compiles C and assembly (from BoringSSL). It is linked into every program that uses HTTPS. Una admits no foreign code (epic #1097).
> - **What.** A new crate, `una-crypto`, of pure Rust with `unsafe` forbidden, written from the specifications and checked against the RFC vectors and Google's Wycheproof vectors. A small adapter in `una-http` plugs it into `rustls` as its `CryptoProvider`.
> - **`rustls` stays.** It carries no foreign code (rule 3, verified).
> - **In slices.** This record covers all of it; slice 1 (SHA-2, HMAC, HKDF, ChaCha20-Poly1305, X25519) ships with it. `ring` stays the provider until the last slice switches.

## Context

### What `ring` is in the tree

`rustls` 0.23.46 is built with `default-features = false, features = ["ring", "std", "tls12"]` (`Cargo.toml`). The `ring` feature enables `ring` 0.17.14 as the provider and as the signature backend of `rustls-webpki`. `ring` has a build script that compiles C and assembly (`cc`). `rcgen` also uses `ring`, in `una-http`'s dev-dependencies only (it makes the tests' certificates).

### What the provider has to give

`rustls::crypto::CryptoProvider` has five parts: `cipher_suites`, `kx_groups`, `signature_verification_algorithms` (used by `rustls-webpki` for the certificate chain and by the handshake for `CertificateVerify`), `secure_random` and `key_provider`. A cipher suite needs a hash, an HMAC, an HKDF (TLS 1.3) or a PRF over the HMAC (TLS 1.2), and an AEAD. The client needs no signing: `Http` is a client, and `una-http` serves nothing over TLS (`serve.rs` reads PEM only to hold the types).

### What the card lists

SHA-256/384/512, HMAC, HKDF; X25519; ChaCha20-Poly1305; AES-128/256-GCM; Ed25519; ECDSA P-256/P-384 verification; RSA PKCS#1 v1.5 and PSS verification. That is the set `rustls-webpki`'s verification algorithms and the TLS 1.3 and 1.2 suites of `ring`'s provider need, except ECDH on P-256 and P-384 (Question 2).

## Decision

### Crate and boundaries

1. **`una-crypto`** is a new workspace crate (`crates/una-crypto`): primitives only, with a byte-slice interface, no allocation in the hot paths, no `rustls` dependency. `#![forbid(unsafe_code)]` holds for the whole crate (it is also the workspace lint). Its one normal dependency is `zeroize` (rule 7); it has no build script, no `-sys` crate, no C, no assembly, no FFI.
2. **The adapter** (`CryptoProvider`, the suites, the key exchange group, the verification algorithms, `SecureRandom`) is a module of `una-http` (`crates/una-http/src/provider`), written in the last slice. `una-crypto` stays free of `rustls`, so it can be tested and reviewed alone and reused (the registry client may use its SHA-2 and Ed25519 later).
3. **`rustls` carries no foreign code, and stays.** Verified on 2026-11-01 for `rustls` 0.23.46, `rustls-webpki` 0.103.16, `rustls-pki-types` 1.15.1, `subtle` 2.6.1, `zeroize` 1.9.1 and `untrusted` 0.9.0 (`once_cell` and `webpki-roots` are Rust and data): none has a `.c`, `.cc`, `.h`, `.s` or `.S` file; the only build script is `rustls`'s, which does nothing unless the nightly-only `read_buf` feature is on (it is off here); the only `asm!` is the empty compiler barrier of `zeroize` (Rust inline assembly, no instructions emitted). With the `ring` feature removed, `cargo tree -p una-http -e normal` contains neither `ring` nor `cc` (checked). `rustls-webpki` then has no signature backend of its own, which is why the verification algorithms are ours (slices 3 to 6). `getrandom` remains for the random source: its operating-system calls are the same trusted code `una-hash` already uses (ADR 0119).
4. **`ring` leaves the shipped graph at the switch and stays only as a development reference.** In `una-crypto` it is a dev-dependency for differential tests (rule 12), and in `una-http` it stays a dev-dependency (`rustls` with its `ring` feature, and `rcgen`) because the test servers need signing and certificate generation, which a client provider does not provide. A dev-dependency never reaches a shipped build; the release gate checks the shipped artifacts' symbols (#1092). Open question 1 asks whether even that should be removed.

### Discipline

5. **Constant time.** For every secret (keys, nonces' derived state, plaintext, scalars, MAC keys, tag comparison operands): no branch, no memory index, no loop count, no shift amount, no division or remainder and no early exit depends on it. Comparison is `ct::eq`; selection is by masks (`mask64`, bitwise select); `core::hint::black_box` stops the optimizer from re-introducing a branch in comparisons and masks. Lengths, counters and the round number are public. AEAD open verifies the tag over the whole ciphertext before it decrypts a byte and leaves the buffer untouched on failure.
6. **Indices.** The workspace denies `clippy::indexing_slicing`. A module whose arrays are indexed by a loop counter bounded by a constant, or by a slice bound computed from public lengths, allows it for the module, with a comment naming this rule. A review of those modules checks that no index derives from a secret. Table lookups indexed by secret data are forbidden (this rules out the classic T-table AES, rule 14).
7. **Wipe.** What the code does: every type that holds secret state (hash state that absorbed a key, HMAC and Poly1305 keys, ChaCha20 state and key stream block, X25519 scalars and the ladder's and the inversion's named intermediates) is wiped when it is dropped or when its function ends, with `Zeroize` from the `zeroize` crate, which writes volatile; the compression functions read the hash buffer in place, with no copy of it. What it does not do, stated plainly: copies in registers and on the stack that the compiler makes of by-value temporaries inside the field and block functions cannot be wiped from safe Rust; arrays the crate returns (a tag, a shared secret, a derived key) are the caller's to wipe, since a function that hands an array over cannot. `zeroize` is pure Rust, already in the tree through `rustls`, and is allowed for `una-crypto` by this record (`architecture.md §7`); a hand-written wipe without a volatile write is removable by the optimizer. Its inline `asm!` (an empty compiler barrier) and its `unsafe` volatile write join the trusted base, which Open question 6 asks the project to confirm.
8. **Target limit.** The field arithmetic of X25519 (and later P-256/P-384) multiplies 64-bit words into 128 bits. That is constant time on x86-64 and AArch64, the targets Una ships; it is not guaranteed on 32-bit targets or on cores with data-dependent multipliers, which Una does not ship. Variable-time work is allowed only on public data and is labelled where it occurs (signature verification: the message, signature and public key are public).
9. **How constant time is tested.** Rust and LLVM give no constant-time guarantee, so three layers, none a proof: (a) the structural rules 5 to 7, enforced by lints and by review; (b) a statistical test per primitive in the dudect style (`tests/timing.rs`: two input classes that differ in a secret, interleaved at random, the slow tail cropped, Welch's t below 10), `#[ignore]` in the ordinary run and part of the release evidence, run on the release candidate on a quiet machine with `--release` (heavy evidence runs on the release candidate, not on every pull request); (c) the independent review of the last slice reads the disassembly of the compare, select and ladder routines for secret-dependent branches. The statistical test passed for `ct::eq`, the AEAD tag check and X25519 with the first slice.
10. **Complexity and speed** (the project's rule: best average-case complexity, stated, with scaling tests). All primitives are linear in the input (`O(n)` hashing, MAC, AEAD, `O(L)` HKDF), X25519 is a fixed 255-step ladder (`O(1)`). `tests/perf.rs` (release builds) checks that 8 times the data costs between 4 and 16 times the time, and a throughput floor. Measured on the development Mac (Apple silicon, release, no hardware instructions) with slice 1: SHA-256 about 180 MiB/s, SHA-512 about 290 MiB/s, ChaCha20-Poly1305 about 420 MiB/s, X25519 about 33 microseconds. These are enough for an HTTP client whose bodies are `Text` (ADR 0074); hardware acceleration (SHA extensions, AES instructions, NEON) needs `core::arch` intrinsics, which are `unsafe` in Rust, so it is out of scope and would need its own record (Question 3).
11. **Test vectors.** Each primitive is checked against (a) the vectors of its specification (FIPS 180-4, RFC 4231, RFC 5869, RFC 7748 including the iterated vectors, RFC 8439), and (b) the Wycheproof vectors of C2SP/wycheproof, licensed Apache-2.0 (the licence file is vendored beside them, the same licence as this repository). They are vendored under `crates/una-crypto/tests/vectors/wycheproof/` at commit `d17cc89fa5c8f4f4d378b5385ebf75da5d0926c9` (2026-10-23), one file per algorithm, and `tests/vectors/DIGESTS.sha256` pins the SHA-256 of each; `tests/vectors_pinned.rs` fails when a file differs from its pin or has none. Updating means fetching the files at a named commit, regenerating the digests in the same change and re-running every test; a vector that a primitive rejects is a bug in the primitive, never a reason to drop the vector. Later slices vendor `aes_gcm_test.json`, `ed25519_test.json`, `ecdsa_secp256r1_*` and `ecdsa_secp384r1_*`, `rsa_signature_*` and `rsa_pss_*` the same way. A test counts the vectors it ran and compares the count with the file's `numberOfTests`, so a silently skipped group fails. Vectors that the typed interface cannot express (a nonce that is not 12 bytes) are skipped by name and asserted invalid in the file.
12. **Differential tests.** Against `ring` (SHA-2, HMAC, HKDF, ChaCha20-Poly1305, later AES-GCM, Ed25519, ECDSA, RSA) and `curve25519-dalek` (X25519; `ring` cannot take a chosen private key), with deterministic pseudo-random inputs around every block, length and limb boundary, as dev-dependencies only. Both are already in `Cargo.lock`; no crate is added to the tree.

### Algorithms

13. **Hashes and MACs.** FIPS 180-4 SHA-256, SHA-384, SHA-512 with data-independent compression; HMAC (RFC 2104) over a `Hash` trait; HKDF (RFC 5869) with the 255-block limit and an error, not a panic, beyond it. Lengths are checked, not assumed: `finalize_into`, `mac_into` and `hkdf::extract` refuse an output slice of the wrong length, and `hkdf::expand` refuses a pseudorandom key that is not exactly the hash length. `hmac::verify` accepts a full-length tag only; `hmac::verify_truncated` is the explicit opt-in for protocols that truncate (RFC 2104 §5), with a tag of at least half the hash length. The bit length of a message is kept modulo `2^64` (`2^128` for SHA-384/512), the wrap of FIPS 180-4's length field, unreachable in memory.
14. **AES-GCM (slice 2).** No table lookups indexed by secrets: AES by a bitsliced implementation of the S-box circuit (in the manner of public constant-time designs), GHASH by 64-bit integer carry-less multiplication emulated with masks. It is slower than hardware AES by an order of magnitude and is accepted for that reason (rule 10). Because of that cost, the provider orders ChaCha20-Poly1305 first when both are offered.
15. **Ed25519 verification (slice 3)** shares the field of X25519, adds SHA-512, the group in extended coordinates and scalar arithmetic modulo the group order, and is strict: canonical encodings, small-order keys and `R` refused, `S` below the order, as ADR 0122 rule 38 requires of the registry client (RFC 8032 and the Wycheproof EdDSA vectors).
16. **ECDSA P-256 and P-384 verification (slice 5)** and **RSA PKCS#1 v1.5 and PSS verification (slice 6)** need modular arithmetic on 256-, 384- and 2048- to 8192-bit integers. They use the own big integer of #1093 when it exists; until then slice 5 may hold a fixed-size field implementation of its own. Verification handles public data only (rule 8). RSA follows the policy of `rustls-webpki`'s algorithms (moduli 2048 to 8192 bits, a public exponent of at least 65537 and at most 33 bits, no SHA-1), PSS with the MGF1 and salt rules of RFC 8017.
17. **The provider (slice 7).** TLS 1.3: `TLS_CHACHA20_POLY1305_SHA256`, `TLS_AES_256_GCM_SHA384`, `TLS_AES_128_GCM_SHA256`; TLS 1.2 (kept, the `tls12` feature is on today): the ECDHE-ECDSA and ECDHE-RSA suites with the same AEADs. Key exchange: X25519. `SecureRandom` is `getrandom`; `KeyProvider` refuses every key (a client provider; the test servers use `ring`, rule 4). The verification algorithms are the list `rustls-webpki` documents for its default provider. Then `rustls` loses its `ring` feature, `crate::client::tls_config` builds its provider, and the size test of ADR 0074 item 7 (the residual of 39 KB of assembly on macOS) must fall to zero.

### Slices

| Slice | Content | Status |
| --- | --- | --- |
| 1 | `ct`, SHA-256/384/512, HMAC, HKDF, ChaCha20-Poly1305, X25519 | with this record |
| 2 | AES-128/256-GCM (rule 14) | |
| 3 | Ed25519 verification (rule 15) | |
| 4 | the big integer (#1093), if it is not there | |
| 5 | ECDSA P-256 and P-384 verification | |
| 6 | RSA PKCS#1 v1.5 and PSS verification | |
| 7 | the provider in `una-http`, the switch, interop tests against `ring` servers and the pinned public roots, the size and symbol checks (#1092) | |
| 8 | the independent security review, fuzzing and the timing evidence on the release candidate, the amendments listed above | |

Each slice is one pull request that refers to #1100, with its vectors and differential tests, its adversarial review (at most two rounds) and the build gate. **The change that switches the provider (7) needs the independent security review of slice 8 before it merges**, as the card requires.

## Consequences

- The shipped graph loses `ring`, `cc` and every assembly object of `ring`; the trusted base gains `una-crypto` (about 1,350 lines for slice 1) and the adapter, both safe Rust with the review burden of cryptography. This is the point of the card, and also its risk: an own implementation has to earn trust by vectors, differential tests, timing evidence and review, which is why slice 8 is not optional.
- Speed on bulk data drops against `ring` (rule 10 and 14). HTTP bodies are small text; the handshake cost is dominated by X25519 (33 microseconds) and signature verification.
- `unsafe_code` stays forbidden in the new crate; the `zeroize` dependency is an existing one.
- Nothing is switched by slice 1: `ring` remains the provider, so no behaviour changes yet.

## Alternatives considered

- **A. Keep `ring`.** Rejected: it is the foreign code the epic removes.
- **B. `rustls` with RustCrypto crates (`rustls-rustcrypto`).** Pure Rust and a smaller job, but it adds many crates to the supply chain (`sha2`, `hmac`, `hkdf`, `aes-gcm`, `chacha20poly1305`, `x25519-dalek`, `p256`, `p384`, `rsa`, `ed25519-dalek` and their trees), some of which with their own `unsafe` and their own optimizations, and several of which (`rsa`) carry known timing advisories. The epic's rule is an implementation of our own, written from the specification and validated against public vectors. It stays the fallback if an own implementation of a verification algorithm (RSA first) cannot meet the review (Question 4).
- **C. `aws-lc-rs`.** A larger C code base with a CMake build (ADR 0074 H). Rejected.
- **D. Provider in `una-crypto`.** Rejected: it would tie the primitives' review and reuse to `rustls`' trait surface and version.
- **E. T-table AES with a cache-timing mitigation.** Rejected by rule 6: it leaks by cache timing, and a mitigation needs hardware instructions.

## Open questions

1. **Dev-only `ring`.** May `ring` stay a dev-dependency (differential tests; `rcgen` and the test servers), outside every shipped build? Removing it entirely means a test-certificate generator and a signing server of our own.
2. **ECDH on P-256 and P-384.** `ring`'s default provider offers them, the card does not list them. A client with X25519 only fails against a server that supports neither X25519 nor TLS 1.2 fallback to it, which is rare. Add them (constant-time field and scalar multiplication, a new slice) or accept X25519 only?
3. **Hardware instructions.** Accept the slower software primitives (rule 10), or open a record for `core::arch` intrinsics in one audited module (which needs `unsafe`)?
4. **Fallback for RSA.** If the own RSA verification cannot meet the review, may slice 6 use a vetted pure-Rust crate as a documented exception, or does the card wait?
5. **Name of the crate** (`una-crypto`) and the placement of the adapter in `una-http`.
6. **`zeroize` in the trusted base.** `zeroize` contains `unsafe` code and one `asm!` (an empty compiler barrier, Rust inline assembly, no foreign code). It is already linked through `rustls` today. May `una-crypto` depend on it directly, or should the crate own a wipe, which in safe Rust cannot be guaranteed against the optimizer?
