"""Family Y: registry packages, their digests, provenance, vendoring, yanks and resolution
(ADR 0122 rules 16 to 41, `spec/30 §9` to §11 and §14).

Each program has a registry `acme` built by the hunt itself (`itaty.py`): the owner `acme`,
releases of `acme/ledger` and of other names, yanks, and checkpoints cosigned by three
witnesses at threshold two. Nothing of it is taken from the compiler, so an honest program the
client refuses, or a forged one it accepts, is a finding either way.

- **Y1, resolution and yanks**, judged by `una ita lock --check` at the lock's resolution point
  (`Program.command`): the registry is served as a mirror that the run's own Itaoca lists. The
  hunt resolves the root's needs itself, by minimal version selection with the raises of
  `spec/30 §10` rule 4 (yanked and avoided selections, pre-releases, the cooldown), and writes
  that lock: the check must accept it (`honest`), and refuse a lock that pins a yanked, avoided,
  newer, pre-release or cooling release, or another digest or log index (U7009), and a root
  whose needs have no resolution (U7013).
- **Y2, vendoring and provenance**, judged by `una check`: the release is vendored with its
  provenance (`spec/30 §9` rules 7 and 8), and the root imports its module, so the check mounts
  it only when it loads verified. Every forgery of the vendored copy, its provenance or the lock
  must be refused (U7010); the honest copy must be accepted.

`ita.una` is a file of edition 0.4: in edition 0.3, every program is the edition gate's probe
(U7008), with no registry built.
"""

from .. import itaty
from ..packages import (OWNER, ROOT, SOURCE, SOURCE_URL, manifest, mirror_files, registry_for,
                        registry_lock_entry, under, vendored)
from ..program import MUST_ACCEPT, MUST_REJECT, Program, rng_for
from .dependencies import module

EDITIONS = ("0.3", "0.4")
GATE = "`ita.una` is a file of edition 0.4; an earlier edition is U7008"
NAME = f"{OWNER}/ledger"
COOLDOWN = 72 * 3600


# ---------------------------------------------------------------------------------------------
# Versions and resolution (`spec/30 §10`), the hunt's own
# ---------------------------------------------------------------------------------------------

def parse(version):
    """(major, minor, patch, pre-release identifiers) of a full version."""
    core, _, pre = version.partition("-")
    major, minor, patch = (int(x) for x in core.split("."))
    return major, minor, patch, tuple(pre.split(".")) if pre else ()


def _pre_key(pre):
    # Semantic versioning 2.0.0 §11: a version without pre-release is above one with it;
    # numeric identifiers are below alphanumeric ones and compare as numbers.
    if not pre:
        return (1,)
    return (0,) + tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in pre) + ((-1,),)


def key(version):
    major, minor, patch, pre = parse(version)
    return (major, minor, patch, _pre_key(pre))


def line(version):
    major, minor, _, _ = parse(version)
    return (major,) if major >= 1 else (0, minor)


def requirement(req):
    """The least version a requirement of two or three parts admits."""
    parts = req.split("-")[0].split(".")
    full = ".".join(parts + ["0"] * (3 - len(parts)))
    return full + ("-" + req.split("-", 1)[1] if "-" in req else "")


def resolve(need, avoid, releases, point_time):
    """The release minimal version selection takes for one need of the root, or None: the least
    release of the need's line at or above it, not a pre-release; raised past a yanked or avoided
    one to the least release above it that is none of these, is not a pre-release, and was logged
    at least the cooldown before the resolution point (`spec/30 §10` rules 3 and 4).
    `releases` maps a version to (logged time, yanked)."""
    least = requirement(need)
    on_line = sorted((v for v in releases if line(v) == line(least)), key=key)
    candidates = [v for v in on_line if key(v) >= key(least) and not parse(v)[3]]
    if not candidates:
        return None
    chosen = candidates[0]
    if not (releases[chosen][1] or chosen in avoid):
        return chosen
    for v in candidates[1:]:
        logged, yanked = releases[v]
        if not yanked and v not in avoid and point_time - logged >= COOLDOWN:
            return v
    return None


# ---------------------------------------------------------------------------------------------
# Y1: resolution and yanks
# ---------------------------------------------------------------------------------------------

LEDGER = "module ledger\n\npub total() -> Int = {n}\n"
OTHER_NAMES = ("acme/clock", "acme/codec")


def release_files(name, version, n):
    short = name.split("/")[-1]
    return {"ita.una": (f"ita {name}\n  version {version}\n  edition 0.4\n  about \"Hunt\"\n"
                        f"  license \"Apache-2.0\"\n  publish to {SOURCE}\n"),
            f"src/{short}.una": f"module {short}\n\npub total() -> Int = {n}\n"}


Y1_FORMS = ("honest", "honest-raised", "pins-yanked", "pins-avoided", "pins-newer",
            "pins-prerelease", "pins-cooling", "wrong-digest", "wrong-logged", "no-resolution")


def _y1_scenario(rng, form):
    """The releases of `acme/ledger` on the line 0.3 for `form`, and the need and avoid lines:
    returns (versions in publishing order, yanked, avoided, recent, need)."""
    base = rng.choice([0, 1, 2])
    versions = [f"0.3.{base + i}" for i in range(4)]
    pre = f"0.3.{base + 1}-rc.1"
    need = rng.choice([f"0.3.{base}", "0.3"]) if base == 0 else f"0.3.{base}"
    yanked, avoided, recent = set(), set(), set()
    first = versions[0]
    if form in ("honest", "pins-newer", "wrong-digest", "wrong-logged"):
        if rng.random() < 0.5:
            yanked.add(versions[-1])
    elif form in ("honest-raised", "pins-yanked"):
        yanked.add(first)
        if rng.random() < 0.5:
            yanked.add(versions[1])
    elif form == "pins-avoided":
        avoided.add(first)
    elif form == "pins-prerelease":
        yanked.add(first)
        versions.append(pre)
    elif form == "pins-cooling":
        yanked.add(first)
        recent.add(versions[1])
    else:
        yanked.update(versions[:3])
        recent.add(versions[3])
    order = list(versions)
    rng.shuffle(order)
    return order, yanked, avoided, recent, need


def y1(seed, edition, form=None):
    rng = rng_for(seed)
    chosen = rng.choice(Y1_FORMS)
    # `form`, for the gate's canaries only, builds that form from the seed's random choices.
    form = form or chosen
    if edition != "0.4":
        return _probe("Y1", seed, edition, form, "lock-check")
    order, yanked, avoided, recent, need = _y1_scenario(rng, form)
    reg, owner = registry_for(seed, "y")
    old_time = itaty.T0 - 30 * itaty.DAY
    point_time = itaty.T0
    releases = {}
    # Noise: releases of another name and of another line, which resolution must not take.
    noise = rng.choice(OTHER_NAMES)
    reg.release(noise, "0.3.9", release_files(noise, "0.3.9", 9), [owner])
    for v in [v for v in order if v not in recent]:
        reg.release(NAME, v, release_files(NAME, v, len(releases) + 1), [owner])
        releases[v] = (old_time, False)
    reg.release(NAME, "0.4.0", release_files(NAME, "0.4.0", 40), [owner])
    for v in sorted(yanked & set(releases), key=key):
        reg.yank(NAME, v, [owner])
    reg.checkpoint(old_time)
    for v in sorted(recent, key=key):
        reg.release(NAME, v, release_files(NAME, v, len(releases) + 1), [owner])
        releases[v] = (point_time - 3600, False)
        reg.checkpoint(point_time - 3600)
    for v in yanked & recent:
        reg.yank(NAME, v, [owner])
    releases = {v: (t, v in yanked) for v, (t, _) in releases.items()}
    reg.append("other", "filler")
    size = reg.checkpoint(point_time)
    _, root = reg.resolution_point(size)
    chosen = resolve(need, avoided, releases, point_time)
    candidates = sorted((v for v in releases if line(v) == (0, 3)), key=key)
    pinned = chosen
    cls, expect = MUST_REJECT, ["U7009"]
    mutate = None
    if form in ("honest", "honest-raised"):
        cls, expect = MUST_ACCEPT, []
    elif form == "pins-yanked":
        pinned = sorted(yanked & set(releases), key=key)[0]
    elif form == "pins-avoided":
        pinned = sorted(avoided, key=key)[0]
    elif form == "pins-newer":
        later = [v for v in candidates if key(v) > key(chosen) and not releases[v][1]
                 and not parse(v)[3]]
        pinned = later[-1] if later else "0.4.0"
    elif form == "pins-prerelease":
        pinned = [v for v in candidates if parse(v)[3]][0]
    elif form == "pins-cooling":
        pinned = sorted(recent, key=key)[0]
    elif form == "wrong-digest":
        mutate = "digest"
    elif form == "wrong-logged":
        mutate = "logged"
    else:
        expect = ["U7013"]
        pinned = sorted(yanked, key=key)[0]
    if chosen is None and form != "no-resolution":
        raise ValueError(f"Y1 {form}: the scenario has no resolution")
    if form != "no-resolution" and cls == MUST_REJECT and pinned == chosen and mutate is None:
        raise ValueError(f"Y1 {form}: the mutated lock is the honest one")
    entry = registry_lock_entry(reg, NAME, pinned)
    if mutate == "digest":
        entry["digest"] = itaty.content_digest(release_files(NAME, pinned, 99))
    elif mutate == "logged":
        entry["logged"] = entry["logged"] + 1
    avoid_lines = sorted(avoided, key=key)
    files = _root(edition, reg, [{"name": NAME, "version": need, "origin": SOURCE,
                                  "avoid": avoid_lines}],
                  module("main", "pub one() -> Int = 1"))
    files["ita.lock"] = itaty.lock_text([(SOURCE, reg.pin(), size, root)], [entry])
    files.update(mirror_files(reg))
    return Program(op="Y1", rule="ADR 0122 rules 20 to 24, spec/30 §10", edition=edition,
                   seed=seed, cls=cls, files=files, expect=expect, variant=form,
                   command="lock-check", mirrors={SOURCE: "mirror"},
                   note=f"need {need}, chosen {chosen}, pinned {pinned}")


def _root(edition, reg, needs, main_text):
    files = {"ita.una": manifest(ROOT, edition,
                                 sources=[(SOURCE, SOURCE_URL, reg.pin() if reg else
                                           "sha256:" + "0" * 64, [OWNER])],
                                 needs=needs),
             "src/main.una": main_text}
    return files


def _probe(op, seed, edition, variant, command="check"):
    """The edition gate's probe of `op`: the root names edition 0.3 in `ita.una` (U7008). No
    registry is built: the manifest is refused before anything else is read."""
    files = _root(edition, None, [{"name": NAME, "version": "0.3", "origin": SOURCE}],
                  module("main", "pub one() -> Int = 1"))
    files["ita.lock"] = itaty.lock_text()
    return Program(op=op, rule="ADR 0122 rule 2", edition=edition, seed=seed, cls=MUST_REJECT,
                   files=files, expect=["U7008"], note=GATE, variant=variant,
                   command=command, mirrors={SOURCE: "mirror"} if command != "check" else {})


# ---------------------------------------------------------------------------------------------
# Y2: vendoring and provenance
# ---------------------------------------------------------------------------------------------

Y2_FORMS = ("honest", "file-changed", "file-added", "file-removed", "provenance-missing",
            "provenance-encoding", "key-mismatch", "statement-forged", "one-witness",
            "forged-witness", "renamed-witness", "log-forged", "root-unpinned", "root-next-weak",
            "point-moved", "logged-moved", "proof-tampered", "digest-swapped", "manifest-field",
            "expired-root", "revoked-witness", "threshold-raised")
FOREIGN = ("build.rs", "src/native.c", "lib/libledger.a", "src/.DS_Store", "src/helper.py")


def _checkpoint_with(reg, size, text_signer, cosigners):
    """The checkpoint of `size` as its text stands, signed by `text_signer` and cosigned at its
    time by `cosigners`, (key, name) pairs: a forgery's checkpoint."""
    honest = reg.checkpoints[size]
    text = honest.split("\n\n", 1)[0] + "\n"
    doc = text + "\n" + itaty.sign_line(text_signer, text)
    for k, name in cosigners:
        renamed = itaty.Key(k.seed, name) if name != k.name else k
        doc += itaty.cosign_line(renamed, text, itaty.T0)
    return doc


def y2(seed, edition, form=None):
    rng = rng_for(seed)
    chosen = rng.choice(Y2_FORMS)
    # `form`, for the gate's canaries only, builds that form from the seed's random choices.
    form = form or chosen
    if edition != "0.4":
        return _probe("Y2", seed, edition, form)
    n = rng.choice([3, 5, 8])
    version = rng.choice(["0.3.1", "0.3.2"])
    expires = itaty.T0 - itaty.DAY if form == "expired-root" else itaty.EXPIRES
    reg = itaty.Registry(SOURCE, "hunt-y", witness_k=1 if form == "threshold-raised" else 2,
                         expires=expires)
    owner = itaty.key_from("hunt-y:owner", f"{OWNER}.laptop")
    reg.register(OWNER, [owner])
    if rng.random() < 0.5:
        noise = rng.choice(OTHER_NAMES)
        reg.release(noise, "0.3.9", release_files(noise, "0.3.9", 9), [owner])
    files = release_files(NAME, version, n)
    attacker = itaty.key_from("hunt-y:attacker", f"{OWNER}.laptop")
    if form == "manifest-field":
        # A registry that logs a statement whose `manifest` is not the archive's `ita.una`.
        reg.release(NAME, version, files, [owner])
        idx, doc, signer, content = reg.releases[(NAME, version)]
        reg.entries.pop()
        reg.heads.pop(NAME)
        forged = doc.split("\n\n", 1)[0].replace(
            "manifest " + itaty.sha256_digest(files["ita.una"].encode()),
            "manifest " + itaty.sha256_digest(b"another manifest\n"))
        document = itaty.signed(forged, [owner])
        index = reg.append(NAME, document)
        reg.releases[(NAME, version)] = (index, document, owner, content)
    else:
        reg.release(NAME, version, files, [owner])
    witnesses = None
    if form == "revoked-witness":
        witnesses = reg.witnesses[:2]
    elif form == "threshold-raised":
        witnesses = reg.witnesses[:1]
    size = reg.checkpoint(itaty.T0, witnesses=witnesses)
    if form == "revoked-witness":
        # The next root revokes the witness at 0, one of the two that cosigned the point: its
        # cosignature counts for nothing at any version (`spec/30 §11` rule 8), so the point has
        # one that counts, below the threshold.
        reg.rotate_witness(0, "rotated", revoke=True)
    elif form == "threshold-raised":
        # The next root raises the witness threshold to two: the point, signed under the first
        # and cosigned by one witness, counts by the larger of the two (`spec/30 §11` rule 9).
        reg.witness_k = 2
        reg.add_root()
    _, root = reg.resolution_point(size)
    entry = registry_lock_entry(reg, NAME, version)
    prov = None
    vendor_files = dict(files)
    point = (SOURCE, reg.pin(), size, root)
    pin = reg.pin()
    if form == "file-changed":
        vendor_files["src/ledger.una"] = LEDGER.format(n=n + 1)
    elif form == "file-added":
        extra = rng.choice(FOREIGN + ("src/extra.una",))
        vendor_files[extra] = "module extra\n\npub two() -> Int = 2\n" \
            if extra.endswith(".una") else "fn main() {}\n"
    elif form == "file-removed":
        vendor_files.pop("ita.una")
    elif form == "key-mismatch":
        prov = reg.provenance(NAME, version, size, key=attacker)
    elif form == "statement-forged":
        idx, doc, _, content = reg.releases[(NAME, version)]
        forged = itaty.signed(doc.split("\n\n", 1)[0], [attacker])
        prov = reg.provenance(NAME, version, size, statement=forged, key=attacker)
        entry["signed"] = reg.signed_name(NAME, attacker)
    elif form == "one-witness":
        prov = reg.provenance(NAME, version, size, checkpoint=_checkpoint_with(
            reg, size, reg.log, [(reg.witnesses[1], reg.witnesses[1].name)]))
    elif form == "forged-witness":
        fake = itaty.key_from("hunt-y:fake-witness", reg.witnesses[2].name)
        prov = reg.provenance(NAME, version, size, checkpoint=_checkpoint_with(
            reg, size, reg.log, [(reg.witnesses[0], reg.witnesses[0].name),
                                 (fake, fake.name)]))
    elif form == "renamed-witness":
        w = reg.witnesses[0]
        prov = reg.provenance(NAME, version, size, checkpoint=_checkpoint_with(
            reg, size, reg.log, [(w, w.name), (w, reg.witnesses[1].name)]))
    elif form == "log-forged":
        fake_log = itaty.key_from("hunt-y:fake-log", reg.log.name)
        prov = reg.provenance(NAME, version, size, checkpoint=_checkpoint_with(
            reg, size, fake_log, [(w, w.name) for w in reg.witnesses]))
    elif form == "root-unpinned":
        other = itaty.Registry(SOURCE, "hunt-y-other")
        prov = reg.provenance(NAME, version, size, roots=other.roots + reg.roots[1:])
    elif form == "root-next-weak":
        weak = itaty.signed(reg.root_text(2), reg.roots_keys[:1])
        prov = reg.provenance(NAME, version, size, roots=reg.roots + [weak])
    elif form == "point-moved":
        moved = "sha256:" + itaty.sha256(root.encode()).hex()
        point = (SOURCE, pin, size, moved)
    elif form == "logged-moved":
        entry["logged"] += 1
    elif form == "proof-tampered":
        idx = reg.releases[(NAME, version)][0]
        proof = itaty.inclusion_proof(idx, [itaty.leaf_hash(e) for e in reg.entries[:size]])
        proof[0] = bytes([proof[0][0] ^ 1]) + proof[0][1:]
        prov = reg.provenance(NAME, version, size, proof=proof)
    elif form == "digest-swapped":
        vendor_files["src/ledger.una"] = LEDGER.format(n=n + 1)
        entry["digest"] = itaty.content_digest(vendor_files)
    if form == "provenance-encoding":
        prov = reg.provenance(NAME, version, size) + "\n"
    files_out = vendored(reg, NAME, version, vendor_files, size, provenance=prov)
    if form == "provenance-missing":
        files_out = {k: v for k, v in files_out.items() if not k.endswith(".provenance")}
    root_files = _root(edition, reg, [{"name": NAME, "version": "0.3", "origin": SOURCE}],
                       module("main", "pub one() -> Int = total()",
                              imports=[("ledger.ledger", ["total"])]))
    root_files["ita.lock"] = itaty.lock_text([point], [entry])
    root_files.update(files_out)
    cls, expect = (MUST_ACCEPT, []) if form == "honest" else (MUST_REJECT, ["U7010"])
    return Program(op="Y2", rule="ADR 0122 rules 16 to 32, spec/30 §9 rules 6 to 9",
                   edition=edition, seed=seed, cls=cls, files=root_files, expect=expect,
                   variant=form)


OPERATORS = {"Y1": y1, "Y2": y2}
EDITIONS_BY_OP = {op: EDITIONS for op in OPERATORS}
