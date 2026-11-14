"""Builders for programs of several packages (ADR 0122, `spec/30`): a root package of edition
0.4 described by `ita.una`, its path dependencies under `deps/`, its registry dependencies
vendored under `vendor/ita/` with their provenance, and the lock that pins them.

The generators of families D and Y assemble programs from these pieces, so that each manifest
and lock parses by construction; every digest, signature and proof is computed here
(`itaty.py`), never copied from the compiler.
"""

from . import itaty
from .una import module_file

ROOT = "hunt"
SOURCE = "acme"
OWNER = "acme"
SOURCE_URL = "https://ita.acme.example"


def need_block(need):
    """One need of `needs`: `need` is a dict with `name`, and optionally `version`, `local`
    (`as`), `origin` (the text after `from`), `may` (a list of ceilings, one line), `avoid`
    (versions) and `accept` (`accept name`)."""
    head = need["name"]
    if need.get("version"):
        head += " " + need["version"]
    if need.get("local"):
        head += " as " + need["local"]
    if need.get("origin"):
        head += " from " + need["origin"]
    lines = ["  " + head]
    if need.get("may"):
        lines.append("    may " + ", ".join(need["may"]))
    lines += ["    avoid " + v for v in need.get("avoid", ())]
    if need.get("accept"):
        lines.append("    accept name")
    return "\n".join(lines)


def manifest(name, edition, version="0.1.0", sources=(), needs=(), trust=(), trusted=(),
             labels=(), boundaries=(), entry=None, grants=(), raw=()):
    """`ita.una`, its sections in the canonical order (`spec/30 §2` rule 2). `sources` are
    (name, url, pin, owners); `trust` are lines after the section's indentation, as
    `util modules keys`; `raw` are extra (section, body) pairs placed after the others, for a
    form that writes a section this builder does not."""
    blocks = [f"ita {name}\n  version {version}\n  edition {edition}"]
    if sources:
        lines = ["sources"]
        for src, url, pin, owners in sources:
            lines.append(f'  {src} "{url}"')
            lines.append(f'    root "{pin}"')
            if owners:
                lines.append("    owners " + ", ".join(owners))
        blocks.append("\n".join(lines))
    if needs:
        blocks.append("needs\n" + "\n".join(need_block(n) for n in needs))
    if trust:
        blocks.append("trust\n" + "\n".join("  " + t for t in trust))
    if labels:
        blocks.append("labels\n" + "\n".join("  " + c for c in labels))
    if trusted:
        blocks.append("trusted\n" + "\n".join("  " + m for m in trusted))
    if boundaries:
        lines = ["boundaries"]
        for module, ceiling in boundaries:
            lines.append("  " + module)
            if ceiling:
                lines.append("    may " + ", ".join(ceiling))
        blocks.append("\n".join(lines))
    if entry is not None:
        lines = ["run", f"  entry {entry}"]
        if grants:
            lines.append("  grants")
            lines += ["    " + g for g in grants]
        blocks.append("\n".join(lines))
    for section, body in raw:
        blocks.append(section + ("\n" + body if body else ""))
    return "\n\n".join(blocks) + "\n"


def package_files(name, edition, modules, version="0.1.0", **sections):
    """A package's own files: `ita.una` and each module of `modules`, {module path: text}."""
    files = {"ita.una": manifest(name, edition, version=version, **sections)}
    for module, text in modules.items():
        files[module_file(module)] = text
    return files


def under(prefix, files):
    return {f"{prefix}/{rel}": text for rel, text in files.items()}


# The owner's keys: three at threshold 2, the least a registry logs and a client replays (the key
# floor, `spec/30 §16` rule 1 and §14 rule 5). The owner signs with the first two.
OWNER_KEYS = ("laptop", "desktop", "backup")
OWNER_THRESHOLD = 2


def owner_keys(label):
    """The owner's keys from `label`, and the two it signs with: (keys, signers)."""
    keys = [itaty.key_from(f"{label}:owner" + ("" if at == 0 else f":{at}"), f"{OWNER}.{name}")
            for at, name in enumerate(OWNER_KEYS)]
    return keys, keys[:OWNER_THRESHOLD]


def register_owner(reg, label):
    """Registers the owner `acme` in `reg` at the key floor; returns the keys it signs with."""
    keys, signers = owner_keys(label)
    reg.register(OWNER, keys, threshold=OWNER_THRESHOLD)
    return signers


def registry_for(seed, label="acme"):
    """A registry `acme` with the owner `acme` registered with three keys at threshold 2. Its
    keys come from `label`, so the programs of a family share them; its entries, from what the
    generator logs. Returns (registry, the owner's keys that sign its releases and yanks)."""
    reg = itaty.Registry(SOURCE, f"hunt-{label}")
    return reg, register_owner(reg, f"hunt-{label}")


def vendored(reg, name, version, files, size, provenance=None, prefix="vendor/ita"):
    """The vendored copy of a logged release at the resolution point of `size` and its
    provenance (`spec/30 §9` rule 8), as files of the root package."""
    rel = itaty.release_dir(name, version)
    out = under(f"{prefix}/{reg.source}/{rel}", files)
    out[f"{prefix}/{reg.source}/{rel}.provenance"] = (
        provenance if provenance is not None else reg.provenance(name, version, size))
    return out


def registry_lock_entry(reg, name, version, granted=()):
    """The lock's entry of a logged release (`spec/30 §14` rule 8)."""
    index, _, signer, content = reg.releases[(name, version)]
    return {"name": name, "version": version, "from": reg.source, "digest": content,
            "signed": reg.signed_name(name, signer), "logged": index,
            "granted": list(granted)}


def path_lock_entry(name, version, directory, files, granted=(), consented=None, trusted=()):
    """The lock's entry of a path dependency (`spec/30 §12` rule 6): its content digest, the
    ceiling granted by the root, and a consent to its trusted modules."""
    entry = {"name": name, "version": version, "from": f'path "{directory}"',
             "digest": itaty.content_digest(files), "granted": list(granted)}
    if consented is not None:
        entry["consented"] = consented
        entry["trusted"] = list(trusted)
    return entry


def mirror_files(reg, prefix="mirror"):
    """The registry's store as a directory a mirror serves (`spec/30 §14` rule 2)."""
    return under(prefix, reg.files)
