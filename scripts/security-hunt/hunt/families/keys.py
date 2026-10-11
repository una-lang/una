"""Family K: `Keys.hmac` (ADR 0039, `spec/07 §7` rule 4, `spec/14 §3`).

`key_name` accepts only `public`, and it alone decides the failure; `data` accepts every
label and is not a sink; the result is `internal ⊔ ℓ*(data)`. The model has no `Keys`
(docs/reviews/security-hunt/coverage.md), so these rules rest on the checker alone.
"""

from ..program import MUST_REJECT, OPEN, Program, rng_for
from ..una import Varied, header, join_decls, package

MOD = "app.main"
USES = "  uses @keys: Keys[hmac]\n  uses @console: Console[print]\n  fails KeyError\n"


def _make(op, edition, seed, cls, body_fn, expect=(), variant="", note="", varied=None,
          entry=False, categories=()):
    text = join_decls(header(MOD), body_fn)
    files = package("hunt", edition, {MOD: text}, categories=categories,
                    entry=MOD if entry else None,
                    grants=["console", "keys:keys.toml"] if entry else ())
    vary = []
    if varied is not None:
        files, vary = varied.place(files)
    return Program(op=op, rule="spec/07 §7 rule 4 (ADR 0039)", edition=edition, seed=seed,
                   cls=cls, files=files, expect=list(expect), note=note, variant=variant,
                   vary=vary)


# K1. A key name that is not public, or a call under a raised program counter.
def k1(seed, edition):
    rng = rng_for(seed)
    level = rng.choice(["internal", "secret"])
    form = rng.choice(["labeled-name", "under-pc", "interpolated-name"])
    if form == "labeled-name":
        fn = (f"pub tag(k: {level} Text) -> internal Hashed\n" + USES +
              '=\n  @keys.hmac(key_name: k, data: "x")\n    if error pass')
    elif form == "interpolated-name":
        fn = (f"pub tag(k: {level} Text) -> internal Hashed\n" + USES +
              '=\n  @keys.hmac(key_name: "pepper-{k}", data: "x")\n    if error pass')
    else:
        fn = (f"pub tag(flag: {level} Bool) -> internal Option[internal Hashed]\n" + USES +
              '=\n  if flag\n    let h = @keys.hmac(key_name: "pepper", data: "x")\n'
              "      if error pass\n    Some(h)\n  else\n    None")
    return _make("K1", edition, seed, MUST_REJECT, fn, ["U4003"], variant=f"{level}-{form}")


# K2. The result lowered without a policy.
def k2(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(["print", "equality", "return-public", "in-list", "secret-data-internal"])
    data = "d" if form == "secret-data-internal" else '"x"'
    param = "d: secret Text" if form == "secret-data-internal" else ""
    head = f"pub show({param}) -> Unit\n" + USES + "=\n"
    let = f'  let h = @keys.hmac(key_name: "pepper", data: {data})\n    if error pass\n'
    if form == "print":
        fn, expect = head + let + '  @console.print("{h}")', ["U4003"]
    elif form == "equality":
        fn = head + let + ('  let g = @keys.hmac(key_name: "pepper", data: "y")\n'
                           '    if error pass\n  @console.print("{h == g}")')
        expect = ["U4003"]
    elif form == "return-public":
        fn = ("pub show() -> Hashed\n  uses @keys: Keys[hmac]\n  fails KeyError\n=\n"
              '  @keys.hmac(key_name: "pepper", data: "x")\n    if error pass')
        expect = ["U4001"]
    elif form == "in-list":
        fn = ("pub show() -> List[Hashed]\n  uses @keys: Keys[hmac]\n  fails KeyError\n=\n"
              '  let h = @keys.hmac(key_name: "pepper", data: "x")\n    if error pass\n  [h]')
        expect = ["U4001"]
    else:
        # `secret` data makes the result `secret`, so an `internal` binder is too low.
        fn = (f"pub show({param}) -> internal Hashed\n  uses @keys: Keys[hmac]\n"
              "  fails KeyError\n=\n" + '  @keys.hmac(key_name: "pepper", data: d)\n    if error pass')
        expect = ["U4001"]
    return _make("K2", edition, seed, MUST_REJECT, fn, expect, variant=form)


# K3. A key name read from the host, which is `internal`, decides `UnknownKey`.
def k3(seed, edition):
    rng = rng_for(seed)
    source = rng.choice(["env", "files"])
    if source == "env":
        fn = ("pub tag() -> internal Hashed\n  uses @env: Env[get]\n" + USES +
              '=\n  let k = @env.get("KEY_NAME")\n    if none use "pepper"\n'
              '  @keys.hmac(key_name: k, data: "x")\n    if error pass')
    else:
        fn = ("pub tag() -> internal Hashed\n  uses @files: Files[read]\n" + USES +
              '=\n  let k = @files.read("key-name.txt")\n    if error use "pepper"\n'
              '  @keys.hmac(key_name: k, data: "x")\n    if error pass')
    return _make("K3", edition, seed, MUST_REJECT, fn, ["U4003"], variant=source)


# K4. Deep labels in `data`, run by the oracle with the key material varied (V-host).
def k4(seed, edition):
    rng = rng_for(seed)
    # Interpolating a record is edition 0.2 only (campaign 2 met U2001 in 0.1).
    form = rng.choice(["record", "list", "text"] if edition == "0.2" else ["list", "text"])
    v = Varied()
    lit = v.lit("Text", "secret[PII]", '"0612345678"')
    kinds = "kind Card\n  holder: Text\n  number: secret[PII] Text"
    if form == "record":
        bind, data = '  let c = Card(holder: "Ana", number: s)\n', '"{c}"'
    elif form == "list":
        bind, data = "  let l = [s, s]\n", '"{l}"'
    else:
        bind, data = "", "s"
    main = ("pub main() -> Unit\n  uses @keys: Keys[hmac]\n  uses @console: Console[print]\n"
            "  fails KeyError\n=\n"
            f"  let s: secret[PII] Text = {lit}\n" + bind +
            f'  let h = @keys.hmac(key_name: "pepper", data: {data})\n    if error pass\n'
            '  @console.print("tagged")')
    p = _make("K4", edition, seed, OPEN, kinds + "\n\n" + main, variant=form,
              note="expected accepted; the output must not depend on the secret or the "
                   "key material", varied=v, entry=True, categories=["PII"])
    p.files["keys.toml"] = 'pepper = "cGVwcGVyLWtleS1tYXRlcmlhbA=="\n'
    return p


OPERATORS = {"K1": k1, "K2": k2, "K3": k3, "K4": k4}
