"""A registry builder for the hunt's programs of several packages (ADR 0122, `spec/30`).

It writes, in Python with the standard library only, what a registry and Itá write: root
metadata, an owner's registration, release statements and yanks as signed notes, the log as
C2SP tiles and entry bundles, the name map's tiles, checkpoints cosigned by witnesses, archives,
provenance documents (`spec/30 §9` rule 7), vendored copies and the lock's lines. It is an
implementation of its own, written from `spec/30` §9, §11 and §14, so a program built here is a
second opinion on the formats: an honest program the client in `una` refuses, or a forged one it
accepts, is a finding either way.

Everything is a pure function of the seed it is given: keys come from seeds, times are fixed
numbers, and no clock is read, so a program is rebuilt byte for byte from its operator and seed.

Costs: a registry of E entries and S checkpoints signs O(E + 4 S) notes, each a few
milliseconds (`ed25519.py`); the tree and its tiles are O(E log E) hashes; the name map is
O(N · 32 · 8) hashes for N names; a provenance is O(E) for its inclusion proof's tree.
"""

import base64
import functools
import hashlib

from . import ed25519

# The times of every registry the hunt writes, in seconds: fixed, so programs are reproducible.
# A resolution point is read at its own witness time, never against the local clock (`lock
# --check` and loading need no freshness, `spec/30 §14` rule 3), so the times need not be near
# the present.
T0 = 1_700_000_000
DAY = 24 * 3600
EXPIRES = 4_000_000_000


def b64(data):
    return base64.b64encode(data).decode("ascii")


def sha256(data):
    return hashlib.sha256(data).digest()


def sha256_digest(data):
    """`sha256:` and the lowercase hex of the SHA-256 of `data`."""
    return "sha256:" + hashlib.sha256(data).hexdigest()


def as_bytes(value):
    return value if isinstance(value, bytes) else value.encode("utf-8")


# ---------------------------------------------------------------------------------------------
# Keys and signed notes (`spec/30 §11` rules 2 to 4)
# ---------------------------------------------------------------------------------------------

class Key:
    """An Ed25519 key with its name: `seed` is 32 bytes."""

    def __init__(self, seed, name):
        self.seed = seed
        self.name = name
        self.public = ed25519.public_key(seed)

    def _id(self, kind):
        return sha256(self.name.encode() + b"\n" + bytes([kind]) + self.public)[:4]

    def id(self):
        return self._id(0x01)

    def cosignature_id(self):
        return self._id(0x04)

    def text(self):
        """`<name> <base64 key>`, as root metadata and owner entries write a key."""
        return f"{self.name} {b64(self.public)}"

    def fingerprint(self):
        return hashlib.sha256(self.public).hexdigest()


@functools.lru_cache(maxsize=None)
def key_from(label, name):
    """A key whose seed is derived from `label`: the same label, the same key. Cached, since its
    public key costs a scalar multiplication and the programs of a family share their keys."""
    return Key(sha256(b"una-hunt-key\0" + label.encode()), name)


def sign_line(key, text):
    signature = ed25519.sign(key.seed, text.encode())
    return f"— {key.name} {b64(key.id() + signature)}\n"


def cosign_line(key, text, time):
    message = f"cosignature/v1\ntime {time}\n{text}".encode()
    signature = ed25519.sign(key.seed, message)
    return f"— {key.name} {b64(key.cosignature_id() + time.to_bytes(8, 'big') + signature)}\n"


def signed(text, signers):
    """A signed note of `text` by each of `signers`: the text, a line feed if it has none, a blank
    line, then one signature line per signer over that exact text."""
    if not text.endswith("\n"):
        text += "\n"
    return text + "\n" + "".join(sign_line(k, text) for k in signers)


# ---------------------------------------------------------------------------------------------
# The log's tree (RFC 9162 §2.1) and its tiles (C2SP tlog-tiles)
# ---------------------------------------------------------------------------------------------

def leaf_hash(leaf):
    return sha256(b"\x00" + as_bytes(leaf))


def node_hash(left, right):
    return sha256(b"\x01" + left + right)


def _split(n):
    k = 1
    while k * 2 < n:
        k *= 2
    return k


def tree_root(leaves):
    if not leaves:
        return sha256(b"")
    if len(leaves) == 1:
        return leaves[0]
    k = _split(len(leaves))
    return node_hash(tree_root(leaves[:k]), tree_root(leaves[k:]))


def inclusion_proof(index, leaves):
    """The audit path of leaf `index` in the tree of `leaves`, from the leaf up (RFC 9162
    §2.1.3.1)."""
    if len(leaves) <= 1:
        return []
    k = _split(len(leaves))
    if index < k:
        return inclusion_proof(index, leaves[:k]) + [tree_root(leaves[k:])]
    return inclusion_proof(index - k, leaves[k:]) + [tree_root(leaves[:k])]


def _subproof(m, leaves, whole):
    n = len(leaves)
    if m == n:
        return [] if whole else [tree_root(leaves)]
    k = _split(n)
    if m <= k:
        return _subproof(m, leaves[:k], whole) + [tree_root(leaves[k:])]
    return _subproof(m - k, leaves[k:], False) + [tree_root(leaves[:k])]


def consistency_proof(m, leaves):
    """The consistency proof from the tree of the first `m` of `leaves` to the tree of all of
    them (RFC 9162 §2.1.4.1)."""
    if 0 < m < len(leaves):
        return _subproof(m, leaves, True)
    return []


WIDTH = 256


def index_path(n):
    groups = []
    while True:
        groups.append(n % 1000)
        n //= 1000
        if n == 0:
            break
    groups.reverse()
    return "/".join(("x" if i + 1 < len(groups) else "") + f"{g:03d}"
                    for i, g in enumerate(groups))


def _tile_path(prefix, index, width):
    path = f"{prefix}/{index_path(index)}"
    if width < WIDTH:
        path += f".p/{width}"
    return path


def log_tiles(entries):
    """The hash tiles and entry bundles of the log of `entries`, by path."""
    files = {}
    leaves = [leaf_hash(e) for e in entries]
    size = len(leaves)
    level = 0
    while True:
        span = 1 << (8 * level)
        nodes = size // span
        if nodes == 0:
            break
        index = 0
        while index * WIDTH < nodes:
            width = min(nodes - index * WIDTH, WIDTH)
            data = b"".join(tree_root(leaves[(index * WIDTH + at) * span:
                                             (index * WIDTH + at + 1) * span])
                            for at in range(width))
            files[_tile_path(f"tile/{level}", index, width)] = data
            index += 1
        level += 1
    index = 0
    while index * WIDTH < size:
        width = min(size - index * WIDTH, WIDTH)
        data = b""
        for at in range(width):
            entry = as_bytes(entries[index * WIDTH + at])
            data += len(entry).to_bytes(2, "big") + entry
        files[_tile_path("tile/entries", index, width)] = data
        index += 1
    return files


# ---------------------------------------------------------------------------------------------
# The name map (`spec/30 §11` rule 9) and its tiles
# ---------------------------------------------------------------------------------------------

_EMPTY = [bytes(32)]
for _ in range(256):
    _EMPTY.append(node_hash(_EMPTY[-1], _EMPTY[-1]))


def map_key(name):
    return sha256(name.encode())


def map_leaf(key, head):
    index, leaf, count = head
    return sha256(b"\x00" + key + index.to_bytes(8, "big") + leaf + count.to_bytes(8, "big"))


def _tile_root(bottom, t):
    """The root of a map tile of tile level `t` whose non-empty bottom nodes are `bottom`."""
    height = 256 - 8 * (t + 1)
    row = dict(bottom)
    for _ in range(8):
        parents = {}
        for i in row:
            p = i // 2
            if p in parents:
                continue
            parents[p] = node_hash(row.get(2 * p, _EMPTY[height]), row.get(2 * p + 1, _EMPTY[height]))
        row = parents
        height += 1
    return row.get(0, _EMPTY[height])


def map_tiles(heads):
    """The map's root and tiles for `heads`, {name: (index, leaf hash, count)}."""
    keyed = sorted((map_key(n), h) for n, h in heads.items())
    files = {}
    if not keyed:
        return _EMPTY[256], files

    def level(group, t):
        nodes, leaves = {}, {}
        i = 0
        while i < len(group):
            byte = group[i][0][t]
            j = i
            while j < len(group) and group[j][0][t] == byte:
                j += 1
            if t == 31:
                k, head = group[i]
                nodes[byte] = map_leaf(k, head)
                leaves[byte] = head
            else:
                nodes[byte] = level(group[i:j], t + 1)
            i = j
        root = _tile_root(nodes, t)
        bitmap = bytearray(32)
        for b in nodes:
            bitmap[b // 8] |= 1 << (b % 8)
        data = bytes(bitmap)
        for b in sorted(nodes):
            if t == 31:
                index, leaf, count = leaves[b]
                data += index.to_bytes(8, "big") + leaf + count.to_bytes(8, "big")
            else:
                data += nodes[b]
        files[f"map/{t}/{root.hex()}"] = data
        return root

    return level(keyed, 0), files


# ---------------------------------------------------------------------------------------------
# Package content (`spec/30 §9` rules 1 to 3)
# ---------------------------------------------------------------------------------------------

def listing(files):
    out = b""
    for path in sorted(files, key=lambda p: p.encode()):
        data = as_bytes(files[path])
        out += (path.encode() + b"\0" + str(len(data)).encode() + b"\0"
                + hashlib.sha256(data).hexdigest().encode() + b"\n")
    return out


def content_digest(files):
    """The content digest of a package's files, {path: text or bytes}."""
    return sha256_digest(listing(files))


def archive(files):
    lst = listing(files)
    out = b"ita-archive/1\n" + str(len(lst)).encode() + b"\n" + lst
    for path in sorted(files, key=lambda p: p.encode()):
        out += as_bytes(files[path])
    return out


def release_dir(name, version):
    """`<owner>+<name>-<version>`, or `<name>-<version>` for a name without an owner."""
    return name.replace("/", "+") + "-" + version


# ---------------------------------------------------------------------------------------------
# A registry
# ---------------------------------------------------------------------------------------------

class Registry:
    """A registry of `source`, built in memory: root metadata (three root keys at threshold 2, a
    log key, three witnesses at threshold `witness_k`, a security key and a key of Itaúna), its
    entries, the head of each name, and its files as a directory copy of its store keeps them."""

    def __init__(self, source, label, witness_k=2, expires=EXPIRES):
        self.source = source
        self.expires = expires
        self.label = label
        self.roots_keys = [key_from(f"{label}:root:{i}", f"{source}.root.{i}") for i in range(3)]
        self.log = key_from(f"{label}:log", f"{source}.example/log")
        self.witnesses = [key_from(f"{label}:witness:{i}", f"witness.{i}") for i in range(3)]
        self.security = key_from(f"{label}:security", f"{source}.security")
        self.itauna = key_from(f"{label}:itauna", f"{source}.itauna")
        self.witness_k = witness_k
        self.roots = []
        self.entries = []
        self.heads = {}
        self.files = {}
        # Each release logged: (name, version) -> (index, statement, signer, content digest).
        self.releases = {}
        self.checkpoints = {}
        # The root version each checkpoint names, by size.
        self.named = {}
        self.add_root()

    def root_text(self, version, revoked=()):
        lines = ["ita-root/1", f"source {self.source}", f"version {version}",
                 f"expires {self.expires}", "root 2"]
        lines += [f"key {k.text()}" for k in self.roots_keys]
        lines.append(f"log {self.log.text()}")
        lines.append(f"witnesses {self.witness_k}")
        lines += [f"key {k.text()}" for k in self.witnesses]
        if revoked:
            lines.append("witnesses-revoked " + " ".join(revoked))
        lines += ["security 1", f"key {self.security.text()}", "itauna 1",
                  f"key {self.itauna.text()}"]
        return "\n".join(lines)

    def rotate_witness(self, at, label, revoke=True, witness_k=None):
        """The next root version, with the witness at `at` replaced by a new key, listed in
        `witnesses-revoked` when `revoke`, and the witness threshold `witness_k` when given."""
        old = self.witnesses[at]
        self.witnesses = list(self.witnesses)
        self.witnesses[at] = key_from(f"{self.label}:{label}", f"witness.rotated.{at}")
        if witness_k is not None:
            self.witness_k = witness_k
        return self.add_root(revoked=[old.fingerprint()] if revoke else ())

    def add_root(self, revoked=(), signers=None):
        version = len(self.roots) + 1
        document = signed(self.root_text(version, revoked),
                          signers if signers is not None else self.roots_keys[:2])
        self.roots.append(document)
        self.files[f"root/{version}"] = document.encode()
        return document

    def pin(self):
        """The digest of the first root metadata: the pin of `sources`."""
        return sha256_digest(self.roots[0].encode())

    def root_line(self):
        return f"ita-root {len(self.roots)} {sha256_digest(self.roots[-1].encode())}"

    def previous(self, name):
        head = self.heads.get(name)
        if head is None:
            return "previous none"
        return f"previous {head[0]} sha256:{head[1].hex()}"

    def append(self, name, document):
        index = len(self.entries)
        self.entries.append(document)
        count = self.heads.get(name, (0, b"", 0))[2] + 1
        self.heads[name] = (index, leaf_hash(document), count)
        return index

    def register(self, owner, keys, threshold=1):
        chain = f"owner:{owner}"
        text = (f"ita-owner/1\nsource {self.source}\nowner {owner}\n{self.previous(chain)}\n"
                f"threshold {threshold}" + "".join(f"\nkey {k.text()}" for k in keys))
        return self.append(chain, signed(text, keys))

    def release(self, name, version, files, signers, needs=(), authority="none",
                trust="none", edition="0.4"):
        """Logs a release of `name` at `version` whose archive holds `files`, and stores the
        archive. Returns its log index."""
        content = content_digest(files)
        manifest = sha256_digest(as_bytes(files["ita.una"]))
        lines = ["ita-release/1", f"source {self.source}", f"name {name}", f"version {version}",
                 self.previous(name), f"digest {content}", f"manifest {manifest}",
                 f"edition {edition}"]
        lines += [f"needs {self.source} {n} {v}" for n, v in sorted(needs)]
        lines += [f"authority {authority}", f"trust {trust}"]
        document = signed("\n".join(lines), signers)
        index = self.append(name, document)
        self.files[f"blobs/sha256/{content[len('sha256:'):]}"] = archive(files)
        self.releases[(name, version)] = (index, document, signers[0], content)
        return index

    def yank(self, name, version, signers):
        text = (f"ita-yank/1\nsource {self.source}\nname {name}\nversion {version}\n"
                f"{self.previous(name)}")
        return self.append(name, signed(text, signers))

    def checkpoint_text(self, size, map_root):
        leaves = [leaf_hash(e) for e in self.entries[:size]]
        return (f"{self.log.name}\n{size}\n{b64(tree_root(leaves))}\nita-map {b64(map_root)}\n"
                f"{self.root_line()}\n")

    def checkpoint(self, time, witnesses=None):
        """Signs a checkpoint at the current size, cosigned at `time` by `witnesses` (every
        witness by default), and one for every smaller size that has none, cosigned the same
        way: a registry signs one after every entry. Writes the tiles and the map's tiles.
        Returns the size."""
        witnesses = self.witnesses if witnesses is None else witnesses
        size = len(self.entries)
        map_root, tiles = map_tiles(self.heads)
        for kept in range(1, size + 1):
            if kept in self.checkpoints and kept != size:
                continue
            # A kept checkpoint below the current size carries the map of its own time; only
            # its tree and root line are read from it, but its map line must be the one it
            # was signed with, so earlier sizes keep theirs once written.
            text = self.checkpoint_text(kept, map_root)
            document = text + "\n" + sign_line(self.log, text)
            document += "".join(cosign_line(w, text, time) for w in witnesses)
            self.checkpoints[kept] = document
            self.named[kept] = len(self.roots)
            self.files[f"checkpoint/{kept}"] = document.encode()
        self.files["checkpoint/latest"] = self.checkpoints[size].encode()
        self.files.update(log_tiles(self.entries))
        self.files.update(tiles)
        return size

    def resolution_point(self, size):
        """(size, `sha256:<hex root>`) of the checkpoint of `size`."""
        leaves = [leaf_hash(e) for e in self.entries[:size]]
        return size, "sha256:" + tree_root(leaves).hex()

    def latest_under_last(self, size):
        """(checkpoint, consistency proof) of the latest checkpoint signed under the last root
        version, larger than `size`, as a client proves the point of `size` its prefix (`spec/30
        §11` rule 9), or None when the registry signed none."""
        last = len(self.roots)
        sizes = [s for s, v in self.named.items() if v == last and s > size]
        if not sizes:
            return None
        to = max(sizes)
        leaves = [leaf_hash(e) for e in self.entries[:to]]
        return self.checkpoints[to], consistency_proof(size, leaves)

    def provenance(self, name, version, size, roots=None, checkpoint=None, statement=None,
                   index=None, proof=None, key=None, fmt=2, latest="honest"):
        """The provenance of a release at the resolution point of `size` (`spec/30 §9` rule 7);
        any part may be given in place of the honest one, for a forgery. In format 2, when the
        point names a root version older than the last, it carries the latest checkpoint under
        the last version and the consistency proof (ADR 0122, amendment of 2026-11-03), unless
        `latest` is None; `latest` may also be a (checkpoint, proof) pair of a forgery's."""
        rel_index, rel_statement, signer, _ = self.releases[(name, version)]
        roots = self.roots if roots is None else roots
        checkpoint = self.checkpoints[size] if checkpoint is None else checkpoint
        statement = rel_statement if statement is None else statement
        index = rel_index if index is None else index
        if proof is None:
            proof = inclusion_proof(rel_index, [leaf_hash(e) for e in self.entries[:size]])
        key = signer if key is None else key
        if latest == "honest":
            latest = (self.latest_under_last(size)
                      if self.named.get(size, len(self.roots)) < len(self.roots) else None)
        out = f"ita-provenance/{fmt}\n"
        for document in roots:
            out += f"root {len(document.encode())}\n{document}"
        out += f"checkpoint {len(checkpoint.encode())}\n{checkpoint}"
        if fmt == 2 and latest is not None:
            latest_checkpoint, consistency = latest
            out += f"latest {len(latest_checkpoint.encode())}\n{latest_checkpoint}"
            out += "".join(f"consistency {b64(h)}\n" for h in consistency)
        out += f"statement {len(statement.encode())}\n{statement}"
        out += f"index {index}\n"
        out += "".join(f"proof {b64(h)}\n" for h in proof)
        out += f"key {key.name} {b64(key.public)}\n"
        return out

    def lock_point(self, size, last=None):
        """The lock's `registry` section at the resolution point of `size`, as `una ita lock`
        writes it: (source, pin, size, root hash, last root version, its digest), the last root
        version the one the registry serves, or `last`."""
        _, root = self.resolution_point(size)
        version = len(self.roots) if last is None else last
        return (self.source, self.pin(), size, root, version,
                sha256_digest(self.roots[version - 1].encode()))

    def signed_name(self, name, key):
        owner = name.split("/", 1)[0] if "/" in name else "itauna"
        return f"ed25519:{owner}:{key.fingerprint()}"


# ---------------------------------------------------------------------------------------------
# The lock (`spec/30 §4`)
# ---------------------------------------------------------------------------------------------

# The comment `una ita` writes first: `lock --check` compares the lock byte for byte.
LOCK_HEADER = "-- Written by `una ita`; do not edit.\n"
SOLVER = ('solver\n  encoding "una-smt-1"\n  name "z3"\n  version "5.1.0"\n'
          '  logic "QF_UFLIA"\n  steps 5000000\n  seed 0\n')


def lock_text(registries=(), packages=()):
    """`ita.lock`: `registries` are (source, pin, size, root hash, last root version, its
    digest), as `Registry.lock_point` gives them, and `packages` dicts with
    `name`, `version`, `from` (a source, or `path "<dir>"`), `digest`, and optionally `signed`,
    `logged`, `needs` [(local, name, version)], `granted` [(by, ceiling)], `consented` and
    `trusted` [module paths]."""
    out = LOCK_HEADER + "lock 1\n\n" + SOLVER
    for source, pin, size, root, last, last_digest in registries:
        out += (f'\nregistry {source}\n  root "{pin}"\n  checkpoint {size} "{root}"\n'
                f'  last root {last} "{last_digest}"\n')
    for p in sorted(packages, key=lambda p: (p["name"], p["version"])):
        out += f"\npackage {p['name']} {p['version']}\n  from {p['from']}\n"
        out += f'  digest "{p["digest"]}"\n'
        if p.get("signed"):
            out += f'  signed "{p["signed"]}"\n  logged {p["logged"]}\n'
        for local, name, version in sorted(p.get("needs", ()), key=lambda n: n[1]):
            out += f"  needs {local} {name} {version}\n"
        for by, ceiling in p.get("granted", ()):
            out += f"  granted by {by} may {ceiling}\n"
        if p.get("consented"):
            out += f'  consented "{p["consented"]}"\n'
        if p.get("trusted"):
            out += "  trusted modules " + ", ".join(p["trusted"]) + "\n"
    return out
