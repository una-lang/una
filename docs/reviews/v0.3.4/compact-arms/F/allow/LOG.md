# allow LOG

Consulted before writing: docs: syntax-reference.md §7.7 (Keys.hmac: any label for `data`, result carries its label), §8.5 (policy `attendee_key` that HMACs a secret phone in a trusted module), §10 and §12 (`std.testing.fake_hash`, usable only in examples and handlers installed by `given`), §17 complete example (`fake_keys` handler), una-for-agents.md §13 (`contains[T: Eq]`).

## Check run 1
- Diagnostics: none (exit 0).
- Consulted: n/a
- Enough: n/a
- Change: first version: trusted module `keyed` with policy `address_key` (secret Text -> internal Hashed via `@keys.hmac(key_name: "pepper", ...)`); `is_allowed(address: secret Text, allowlist: List[internal Hashed]) -> internal Bool uses @keys: Keys[hmac] fails KeyError` compares only hashes (`allowlist.contains(item: hashed)`); stand-in handler `fake_pepper` maps one address to `fake_hash(1)` under `pepper`; two examples.

## Test run 1
- `una test`: 2 examples, 2 passed, exit 0. The trace shows `key_name: "pepper"` and `data: <redacted:secret>`.

DOC: the variants of `KeyError` (and `FileError`) are not listed in either guide (syntax-reference.md §7.7 and §12 only name the kinds), so a stand-in key service cannot fail with a realistic variant; mine returns a non-matching hash for any key other than `pepper` instead.
DOC: syntax-reference.md §7.7 says the result of `Keys.hmac` "carries the label of `data`", but its signature says `-> internal Hashed`; in the test trace the hash of a secret address is `<redacted:secret>`. The two statements read as contradictory until one notices the label is raised by the argument.
