# allow log

## check run 1
- U3007 "`@keys.hmac` can fail with `KeyError`; handle the failure with an `if error` clause, or wrap the call in `attempt`"
- U4003 "`@keys.hmac` accepts `public` data, and this argument is `secret`; declassify it through a policy first"
- U2006 "parameter `key_name` of operation `Keys.hmac` is given twice"
- U4001 "`found` is `Bool`, and this value is `internal Bool`: a flow from `internal` to `public`"
- consulted: message only (U2006 shows the first positional parameter of hmac is `key_name`, so the U4003 was my address landing in the key-name position)
- enough: partly (the parameter order and the label of the data parameter of `Keys.hmac` are not given anywhere)
- changed: `@keys.hmac("pepper", value: address)` with `if error pass` under a `fails KeyError`... (first try: handle via label guess); `let edit found: internal Bool`
- DOC: una-compact.md lists `Keys[hmac]` as a host capability but never gives the signature of `Keys.hmac` (parameters, result type, failure kind KeyError) nor how to write a stand-in handler for it.

## check run 2
- U2005 "operation `Keys.hmac` has no parameter `value`" (repair: "the parameters that no other argument gives are `data`")
- consulted: message only (the repair text)
- enough: yes
- changed: `@keys.hmac("pepper", data: address)`

## check run 3
- U4001 "`found` is `internal Bool`, and this value is `secret Bool`: a flow from `secret` to `internal`"
- consulted: message only
- enough: yes (the HMAC of a secret stays secret, so the comparison is secret)
- changed: result and `found` are `secret Bool`; no declassification, the caller decides whether to lower the decision

## check run 4
- U2001 x4 "argument `seed` of `fake_hash` must be `Int`, found `Text`"
- U4001 x2 "parameter `allowlist` of `allowed` is `List[Hashed]`, and this value is `List[internal Hashed]`: a flow from `internal` to `public`"
- consulted: message only (+ una explain U3015 for `fake_hash` and stand-in handlers)
- enough: yes
- changed: stand-in maps known addresses to Int seeds via a helper `seed_of`; allowlist parameter is `List[internal Hashed]`
- DOC: una-compact.md does not mention `std.testing` or `fake_hash`; only `una explain U3015` shows them, and nothing says that hashes are `internal`.

## check run 5
- U4001 "parameter `seed` of `fake_hash` is `Int`, and this value is `secret Int`: a flow from `secret` to `public`"
- U4001 "parameter `address` of `seed_of` is `secret Text`, and this value is `restricted Text`: a flow from `restricted` to `secret`"
- consulted: message only; probed `import std.testing (fake_hmac)` in a scratch package (U1001 "module `std.testing` declares no `fake_hmac`")
- enough: partly (the `data` of a Keys handler is `restricted`, and `fake_hash` takes a public Int)
- changed: dropped `seed_of`; the stand-in picks among public literal seeds by comparing `data` inline, so only the choice is labeled
- DOC: no document lists the functions of `std.testing`, nor the labels of the parameters and result of `Keys.hmac`.

## check run 6
- no diagnostics, exit 0 (PASS)

## test run 1
- 2 examples passed (on the list / not on the list), exit 0
