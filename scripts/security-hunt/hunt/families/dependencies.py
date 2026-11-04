"""Family D: authority, information flow and trust across packages (ADR 0122 rules 49 to 53,
`spec/30 §12` and §13).

Every program is a root package of edition 0.4 with one dependency, read from a path
(`deps/<dir>`) or, for the forms that this release mounts from a registry, vendored from the
registry `acme` with its provenance (`spec/30 §9` rule 8, §14 rule 10). The dependency is a
boundary of ADR 0093 whose ceiling the root writes with `may` lines under its need and the lock
records: a call into it takes no capability by name (U3020), each grant is within the ceiling
(U3021), and a ceiling wider than the lock's is U7009. Its trusted modules are trusted only by
a trust line of the root and the lock's consent to its whole content, pinned by digest (U7012).
Its labels are the program's: a dependency is checked with the root as one program.

`ita.una` is a file of edition 0.4: in edition 0.3, every program of the family is the edition
gate's probe, a root whose `ita.una` names an earlier edition (U7008), with nothing else built.
"""

from .. import itaty
from ..packages import (OWNER, ROOT, SOURCE, SOURCE_URL, package_files, path_lock_entry,
                        registry_for, registry_lock_entry, under, vendored)
from ..program import MUST_ACCEPT, MUST_REJECT, OPEN, Program, rng_for
from ..una import Varied

RULE = "ADR 0122 rules 49 to 53, spec/30 §12"
GATE = "`ita.una` is a file of edition 0.4; an earlier edition is U7008"
EDITIONS = ("0.3", "0.4")
MAIN = "main"
DEP_DIR = "deps/util"
VERSION = "0.1.0"

CONSOLE = "Console[print]"
CLOCK = "Clock[now]"

SAY = ("pub say(line: Text) -> Unit\n  uses @console: Console[print]\n=\n"
       "  @console.print(line)")
STAMP = "pub stamp() -> Int\n  uses @clock: Clock[now]\n=\n  @clock.now()"
NOTIFY = ("pub notify(key: Text, done: fn(Text) -> Unit) -> Unit\n  uses of done\n=\n"
          "  done(key)")
KEEP = "pub keep(n: secret Int) -> secret Int = n + 1"
PEEK = "pub peek(n: secret Int) -> Int = n"
SHOW = ("pub show(n: secret Int) -> Unit\n  uses @console: Console[print]\n=\n"
        '  @console.print("{n}")')
TWICE = "pub twice[T](x: T, f: fn(T) -> T) -> T = f(f(x))"
SAME = "pub same[T](x: T) -> T = x"
HIDDEN = "hidden(line: Text) -> Text = line"


def module(name, *decls, imports=()):
    lines = [f"module {name}"]
    if imports:
        lines.append("")
        lines += [f"import {m} ({', '.join(sorted(names))})" for m, names in sorted(imports)]
    return "\n".join(lines) + "\n\n" + "\n\n".join(d.rstrip("\n") for d in decls) + "\n"


def trusted_module(name, *decls):
    return f"trusted module {name}\n\n" + "\n\n".join(d.rstrip("\n") for d in decls) + "\n"


def root_main(imports, *decls):
    return module(MAIN, *decls, imports=imports)


class Dep:
    """One dependency of the root, and what the root's files need for it."""

    def __init__(self, origin, modules, seed, local="util", may=(), granted=None,
                 manifest_sections=None, accept=True, trusted=(), trust_line=None,
                 consent="honest", files_after=None, name=None, manifest_name=None):
        self.origin = origin
        self.local = local
        self.may = list(may)
        self.granted = list(may) if granted is None else list(granted)
        self.accept = accept
        self.trust_line = trust_line
        if origin == "path":
            self.name = name or local
        else:
            self.name = name or f"{OWNER}/{local}"
        sections = dict(manifest_sections or {})
        if trusted:
            sections["trusted"] = list(trusted)
        # The name the dependency's own `ita.una` gives, which a path need's name must be.
        self.files = package_files(manifest_name or self.name, "0.4", modules, version=VERSION,
                                   **sections)
        # The files the dependency holds when the program runs, which may differ from those
        # its consent was recorded for (form `stale`).
        self.loaded = dict(self.files, **(files_after or {}))
        self.trusted = list(trusted)
        self.consent = consent
        self.seed = seed

    def need(self):
        need = {"name": self.name, "may": self.may}
        if self.origin == "path":
            need["origin"] = f'path "{DEP_DIR}"'
            need["accept"] = self.accept
        else:
            need["version"] = VERSION.rsplit(".", 1)[0]
            need["origin"] = SOURCE
        return need

    def build(self):
        """The dependency's files in the root, its lock entries and the lock's registries."""
        granted = [(ROOT, ", ".join(self.granted))] if self.granted else []
        if self.origin == "path":
            consented = None
            if self.trusted and self.consent != "missing":
                consented = itaty.content_digest(self.files)
            entry = path_lock_entry(self.name, VERSION, DEP_DIR, self.loaded, granted=granted,
                                    consented=consented,
                                    trusted=self.trusted if consented else ())
            return under(DEP_DIR, self.loaded), [entry], []
        reg, owner = registry_for(self.seed, "d")
        reg.release(self.name, VERSION, self.files, [owner], authority=_authority(self.files))
        size = reg.checkpoint(itaty.T0)
        files = vendored(reg, self.name, VERSION, self.files, size)
        if self.loaded != self.files:
            # A vendored copy edited after the lock: its digest is not the lock's (U7010).
            files.update(under(f"vendor/ita/{SOURCE}/{itaty.release_dir(self.name, VERSION)}",
                               self.loaded))
        entry = registry_lock_entry(reg, self.name, VERSION, granted=granted)
        return files, [entry], [reg.lock_point(size)], reg.pin()


def _authority(files):
    """A release statement's `authority`: the capabilities and operations its public functions
    use (`spec/30 §11` rule 6), from the `uses` lines of the modules this family writes."""
    ops = {}
    for rel, text in files.items():
        if not rel.endswith(".una"):
            continue
        for line in text.splitlines():
            line = line.strip()
            if line.startswith("uses @") and ":" in line:
                cap = line.split(":", 1)[1].strip()
                name, _, rest = cap.partition("[")
                ops.setdefault(name, set()).update(o.strip() for o in rest.rstrip("]").split(","))
    if not ops:
        return "none"
    return ", ".join(f"{c}[{', '.join(sorted(o))}]" for c, o in sorted(ops.items()))


def assemble(edition, dep, main_text, entry=False, extra_modules=None, trust=(),
             root_trusted=(), lock_override=None):
    """The root package with `dep`: its manifest, its modules, the dependency's files and the
    lock. In edition 0.3 (the probe) the manifest names 0.3 and only the root's files and a
    path dependency are written: the manifest is refused before anything else is read."""
    need = dep.need()
    built = dep.build()
    dep_files, entries, registries = built[0], built[1], built[2]
    sources = []
    if dep.origin == "registry":
        sources = [(SOURCE, SOURCE_URL, built[3], [OWNER])]
    modules = {MAIN: main_text, **(extra_modules or {})}
    trust_lines = list(trust)
    if dep.trust_line:
        trust_lines.append(dep.trust_line)
    files = package_files(ROOT, edition, modules, sources=sources, needs=[need],
                          trust=trust_lines, trusted=root_trusted,
                          entry=MAIN if entry else None,
                          grants=["console"] if entry else ())
    files.update(dep_files)
    files["ita.lock"] = (lock_override(entries, registries) if lock_override
                         else itaty.lock_text(registries, entries))
    return files


def _probe(op, seed, edition, variant, rule):
    """The edition gate's probe of `op`: the root names edition 0.3 in `ita.una` (U7008)."""
    dep = Dep("path", {"say": module("say", SAY)}, seed, may=[CONSOLE])
    main = root_main([("util.say", ["say"])],
                     "pub greet() -> Unit\n  uses @console: Console[print]\n=\n"
                     '  say("hi") with @console = @console[print]')
    files = assemble("0.3", dep, main)
    return Program(op=op, rule=rule, edition=edition, seed=seed, cls=MUST_REJECT, files=files,
                   expect=["U7008"], note=GATE, variant=variant)


# D1. Authority across a package boundary, from a path or from the registry: a capability by
# name (U3020), a grant beyond the ceiling the root writes, by a binding or a functional
# argument (U3021), a ceiling wider than the lock records or one the lock does not record
# (U7009), and what the root may not reach or write: a dependency importing the root (U1001),
# a module of the root under the dependency's local name (U1010), a function that is not `pub`
# (U1004), a root `trusted` line naming a dependency's module (U7008, U7005), a dependency with
# needs, labels or boundaries of its own and a ceiling naming a dependency's capability (U7008),
# a path package named otherwise than its need (U7008), a reserved or look-alike local name
# (U7008), and a local package without `accept name` (U7012).
D1_FORMS = ("by-name", "ceiling", "functional-argument", "lock-widened", "lock-ungranted",
            "imports-root", "local-clash", "private", "root-trusted", "dep-needs", "dep-labels",
            "dep-boundaries", "dep-capability-ceiling", "wrong-name", "reserved-local",
            "skeleton-local", "not-accepted")
# The forms this release mounts from the registry as well as from a path.
D1_REGISTRY = ("by-name", "ceiling", "functional-argument", "lock-widened", "lock-ungranted",
               "private")


def d1(seed, edition):
    rng = rng_for(seed)
    form = rng.choice(D1_FORMS)
    origin = rng.choice(("path", "registry")) if form in D1_REGISTRY else "path"
    variant = f"{form}-{origin}"
    if edition != "0.4":
        return _probe("D1", seed, edition, variant, RULE)
    say_mod = module("say", SAY, STAMP, NOTIFY, HIDDEN)
    modules = {"say": say_mod}
    may, granted, sections, extra, trusted_root = [CONSOLE], None, {}, {}, ()
    local, name, accept = "util", None, True
    greet = ("pub greet() -> Unit\n  uses @console: Console[print]\n=\n"
             '  say("hi") with @console = @console[print]')
    imports = [("util.say", ["say"])]
    if form == "by-name":
        decls = ["pub greet() -> Unit\n  uses @console: Console[print]\n=\n  say(\"hi\")"]
        expect = ["U3020"]
    elif form == "ceiling":
        imports = [("util.say", ["stamp"])]
        decls = ["pub stamped() -> Int\n  uses @clock: Clock[now]\n=\n"
                 "  stamp() with @clock = @clock"]
        expect = ["U3021"]
    elif form == "functional-argument":
        may = []
        imports = [("util.say", ["notify"])]
        decls = ["pub announce(key: Text) -> Unit\n  uses @console: Console[print]\n=\n"
                 "  notify(key, done: fn(p) -> @console.print(p))"]
        expect = ["U3021"]
    elif form == "lock-widened":
        may, granted = [CONSOLE, CLOCK], [CONSOLE]
        decls = [greet]
        expect = ["U7009"]
    elif form == "lock-ungranted":
        granted = []
        decls = [greet]
        expect = ["U7009"]
    elif form == "imports-root":
        modules = {"say": module("say", SAY, "pub echo() -> Int = base()",
                                 imports=[("main", ["base"])])}
        decls = [greet, "pub base() -> Int = 1"]
        expect = ["U1001"]
    elif form == "local-clash":
        extra = {"util.extra": module("util.extra", "pub one() -> Int = 1")}
        decls = [greet]
        expect = ["U1010"]
    elif form == "private":
        imports = [("util.say", ["hidden", "say"])]
        decls = [greet, "pub mirror(line: Text) -> Text = hidden(line)"]
        expect = ["U1004"]
    elif form == "root-trusted":
        trusted_root = ["util.say"]
        decls = [greet]
        expect = ["U7008", "U7005"]
    elif form == "dep-needs":
        sections = {"needs": [{"name": "other", "origin": 'path "../other"', "accept": True}]}
        decls = [greet]
        expect = ["U7008"]
    elif form == "dep-labels":
        sections = {"labels": ["PII"]}
        decls = [greet]
        expect = ["U7008"]
    elif form == "dep-boundaries":
        sections = {"boundaries": [("say", [CONSOLE])]}
        decls = [greet]
        expect = ["U7008"]
    elif form == "dep-capability-ceiling":
        modules = {"say": say_mod,
                   "store": module("store", "pub capability Store\n  op read(key: Text) -> Text")}
        may = [CONSOLE, "util.store.Store[read]"]
        decls = [greet]
        expect = ["U7008"]
    elif form == "wrong-name":
        name = rng.choice(["utils", "util_core", "acme/util"])  # the dependency's own name
        decls = [greet]
        expect = ["U7008"]
    elif form in ("reserved-local", "skeleton-local"):
        local = rng.choice(["std", "prelude", "console", "files"]) if form == "reserved-local" \
            else rng.choice(["s7d", "std_", "c0nsole", "fi1es"])
        imports = [(f"{local}.say", ["say"])]
        decls = [greet]
        expect = ["U7008"]
    else:
        accept = False
        decls = [greet]
        expect = ["U7012"]
    dep = Dep(origin, modules, seed, local=local, may=may, granted=granted,
              manifest_sections=sections, accept=accept, manifest_name=name)
    if form in ("reserved-local", "skeleton-local"):
        dep.name = "util"
        dep.files = package_files("util", "0.4", modules, version=VERSION, **sections)
        dep.loaded = dict(dep.files)
    files = assemble(edition, dep, root_main(imports, *decls), extra_modules=extra,
                     root_trusted=trusted_root)
    if form in ("reserved-local", "skeleton-local"):
        files["ita.una"] = files["ita.una"].replace(
            f'  util from path "{DEP_DIR}"', f'  util as {local} from path "{DEP_DIR}"')
    return Program(op="D1", rule=RULE, edition=edition, seed=seed, cls=MUST_REJECT, files=files,
                   expect=expect, variant=variant)


# D2. Information flow across a package boundary, while the root holds a secret: the secret
# reaches a dependency's function and comes back still secret, through a generic or a
# functional argument too, and only public text reaches the dependency's `Console.print`
# granted within its ceiling; no observer below the secret sees a difference (the oracle).
# Its closed calls are rejected by a correct checker, so the oracle sets them aside: the
# dependency printing the secret (U4003 at its `print`), returning it as public (U4001), and a
# call that prints, granted under a branch on the secret (U4002).
D2_OPEN = ("keep", "generic", "higher-order", "grant-after")
D2_CLOSED = ("dep-prints-secret", "dep-declassifies", "under-secret-pc")
D2_EXPECT = {"dep-prints-secret": ["U4003"], "dep-declassifies": ["U4001"],
             "under-secret-pc": ["U4002"]}


def d2(seed, edition, form=None):
    rng = rng_for(seed)
    chosen = rng.choice(D2_OPEN + D2_CLOSED)
    # `form`, for the gate's canaries only, builds that form from the seed's random choices.
    form = form or chosen
    origin = rng.choice(("path", "registry"))
    variant = f"{form}-{origin}"
    if edition != "0.4":
        return _probe("D2", seed, edition, variant, RULE + ", spec/07 §3")
    v = Varied()
    lit = v.lit("Int", "secret", rng.choice(["5", "7", "12"]))
    k = rng.choice([2, 3, 4])
    names, body, decls = ["say"], "", []
    # The dependency's module holds only what the form calls, so that a closed call's error is
    # the only one the program has.
    own = {"keep": KEEP, "same": SAME, "twice": TWICE, "show": SHOW, "peek": PEEK}
    if form == "keep":
        names.append("keep")
        body = "  let t = keep(s)\n"
    elif form == "generic":
        names.append("same")
        body = "  let t: secret Int = same(s)\n"
    elif form == "higher-order":
        names.append("twice")
        body = f"  let t: secret Int = twice(s, f: fn(x) -> x + {k})\n"
    elif form == "grant-after":
        names.append("keep")
        body = (f"  let t = keep(s + {k})\n"
                f"  say(\"step {k}\") with @console = @console[print]\n")
    elif form == "dep-prints-secret":
        names.append("show")
        body = "  show(s) with @console = @console[print]\n"
    elif form == "dep-declassifies":
        names.append("peek")
        body = "  let t = peek(s)\n"
    else:
        decls.append("loud() -> Unit\n  uses @console: Console[print]\n=\n"
                     '  say("loud") with @console = @console[print]')
        body = f"  if s > {k}\n    loud()\n"
    say_mod = module("say", SAY, *[own[n] for n in names if n in own])
    main = root_main([("util.say", names)], *decls,
                     "pub main() -> Unit\n  uses @console: Console[print]\n=\n"
                     f"  let s: secret Int = {lit}\n" + body
                     + '  say("done") with @console = @console[print]')
    dep = Dep(origin, {"say": say_mod}, seed, may=[CONSOLE])
    files = assemble(edition, dep, main, entry=True)
    files, vary = v.place(files)
    closed = form in D2_CLOSED
    note = ("a closed call: a correct checker rejects it, and the oracle sets it aside" if closed
            else "expected accepted; the oracle checks that no observer below the secret sees it")
    return Program(op="D2", rule=RULE + ", spec/07 §3", edition=edition, seed=seed, cls=OPEN,
                   files=files, expect=D2_EXPECT.get(form, []), note=note, variant=variant,
                   vary=vary)


# D3. Trust a dependency adds (`spec/30 §13`): a path dependency whose trusted module declares a
# policy. Trusted by a trust line of the root and the lock's consent to the package's whole
# content (`honest`, which the checks must accept), it is refused (U7012) without a trust line,
# without the lock's consent, with a consent to other content (any file changed, the trusted
# module or not), with a trust line that leaves a trusted module out, names another major line
# or a registry source, or with the lock's trusted modules not the package's; and a grant into
# its trusted extent beyond the intersection of the root's two ceilings is U3021.
D3_FORMS = ("honest", "no-trust-line", "unconsented", "stale-trusted", "stale-other",
            "module-unnamed", "lock-modules", "other-major", "from-source", "narrow-trust")
KEYS = trusted_module("keys", "pub policy reveal\n  from secret Text\n  to Text\n  via shown\n"
                      '  purpose "show a token"\n  owner team',
                      "shown(t: secret Text) -> Text = t")
VAULT = trusted_module("vault", "pub policy release\n  from secret Int\n  to Int\n  via opened\n"
                       '  purpose "count"\n  owner team', "opened(n: secret Int) -> Int = n")
NOTE_SAY = ("pub note(line: Text) -> Unit\n  uses @console: Console[print]\n=\n"
            "  @console.print(line)")


def d3(seed, edition, form=None):
    rng = rng_for(seed)
    chosen = rng.choice(D3_FORMS)
    # `form`, for the gate's canaries only, builds that form from the seed's random choices.
    form = form or chosen
    variant = form
    if edition != "0.4":
        return _probe("D3", seed, edition, variant, "ADR 0122 rule 53, spec/30 §13")
    modules = {"keys": KEYS, "say": module("say", NOTE_SAY, "pub one() -> Int = 1")}
    trusted = ["keys"]
    trust_line = "util modules keys"
    consent, after, lock_modules = "honest", None, None
    may = []
    decls = ["pub show(t: secret Text) -> Text = reveal(t)"]
    imports = [("util.keys", ["reveal"])]
    cls, expect = MUST_REJECT, ["U7012"]
    if form == "honest":
        cls, expect = MUST_ACCEPT, []
    elif form == "no-trust-line":
        trust_line = None
    elif form == "unconsented":
        consent = "missing"
    elif form == "stale-trusted":
        after = {"src/keys.una": KEYS.replace('"show a token"', '"show any token"')}
    elif form == "stale-other":
        after = {"src/say.una": modules["say"].replace("= 1", "= 2")}
    elif form == "module-unnamed":
        modules["vault"] = VAULT
        trusted = ["keys", "vault"]
    elif form == "lock-modules":
        modules["vault"] = VAULT
        trusted = ["keys", "vault"]
        trust_line = "util modules keys, vault"
        lock_modules = ["keys"]
    elif form == "other-major":
        trust_line = "util " + rng.choice(["1", "0.2", "2"]) + " modules keys"
    elif form == "from-source":
        trust_line = "util from acme modules keys"
    else:
        # The trust line's ceiling is narrower than the need's: a grant into the trusted
        # extent beyond their intersection.
        modules["keys"] = trusted_module(
            "keys", "pub policy reveal\n  from secret Text\n  to Text\n  via shown\n"
            '  purpose "show a token"\n  owner team',
            "shown(t: secret Text) -> Text = t",
            "pub tell(line: Text) -> Unit\n  uses @console: Console[print]\n=\n"
            "  @console.print(line)")
        may = [CONSOLE]
        trust_line = "util modules keys"
        imports = [("util.keys", ["reveal", "tell"])]
        decls.append("pub speak() -> Unit\n  uses @console: Console[print]\n=\n"
                     '  tell("hi") with @console = @console[print]')
        expect = ["U3021"]
    dep = Dep("path", modules, seed, may=may, trusted=trusted, trust_line=trust_line,
              consent=consent, files_after=after)
    if form == "narrow-trust":
        dep.trust_line = "util modules keys\n    may Clock[now]"
    lock_override = None
    if lock_modules is not None:
        def lock_override(entries, registries):
            for e in entries:
                e["trusted"] = lock_modules
            return itaty.lock_text(registries, entries)
    files = assemble(edition, dep, root_main(imports, *decls), lock_override=lock_override)
    return Program(op="D3", rule="ADR 0122 rule 53, spec/30 §13", edition=edition, seed=seed,
                   cls=cls, files=files, expect=expect, variant=variant)


OPERATORS = {"D1": d1, "D2": d2, "D3": d3}
EDITIONS_BY_OP = {op: EDITIONS for op in OPERATORS}
