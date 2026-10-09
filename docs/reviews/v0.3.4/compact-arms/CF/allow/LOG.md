# LOG: allow

## Check run 1
- Diagnostics: U4001 "parameter `seed` of `fake_hash` is `Int`, and this value is `restricted Int`: a flow from `restricted` to `pu"
- Consulted: message only (plus syntax-reference.md §7.7: `Keys.hmac` accepts any label for `data`, so inside a handler `data` is `restricted`).
- Enough: yes: a stand-in Keys handler cannot derive its hash from `data` at all; its result is `internal Hashed`.
- Change: the stand-in's hash now depends only on the key name (`fake_hash(1)` for `pepper`, `fake_hash(0)` otherwise); examples choose allowlists that contain or omit that hash; dropped the normalization example, which the stand-in cannot distinguish.
- DOC: syntax-reference.md §7.7/§10 and the events example (`fake_keys`) do not say that the `data` parameter of a `Keys` handler is `restricted`, so a stand-in key service cannot map different inputs to different hashes; this limits what an example can show.

## Check run 2
- Diagnostics: none (exit 0).
- Consulted: (nothing new)
- Enough: yes.
- Change: (stand-in rewritten as above). `is_allowed(email: secret Text, allowlist: List[internal Hashed]) -> secret Bool` trims and lowercases the address, hashes it with `@keys.hmac(key_name: "pepper", ...)` and compares only `Hashed` values with `contains`; `fails KeyError` with `if error pass` was accepted.

## Test run 1
- `una test`: 3 examples, 3 passed (exit 0). The trace shows `hmac` called with key_name "pepper" and data `<redacted:secret>`.

## Check run 3 (final verification sweep)
- Diagnostics: none (exit 0); `una test` also exit 0.
- Consulted: (nothing)
- Enough: yes
- Change: none
