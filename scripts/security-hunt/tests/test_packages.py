"""Tests of families D and Y and of the registry builder they use (#1001), that need no
compiler.

    cd scripts/security-hunt && python3 -m unittest discover -s tests
"""

import base64
import hashlib
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from hunt import drivers, ed25519, gate, itaty, oracle, sandbox  # noqa: E402
from hunt.families import OPERATORS, editions_of  # noqa: E402
from hunt.families import dependencies, registry  # noqa: E402
from hunt.program import MUST_ACCEPT, MUST_REJECT, OPEN, Program, derive_seed  # noqa: E402
from hunt.refactor import REFACTORS  # noqa: E402

REPO = os.path.realpath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
FAMILY_OPS = ("D1", "D2", "D3", "Y1", "Y2", "Y3")


def programs(op, n=60, edition="0.4"):
    return [OPERATORS[op](derive_seed(11, op, i), edition) for i in range(n)]


def verify_inclusion(index, size, leaf, proof, root):
    """RFC 9162 §2.1.3.2, written apart from the builder's own proof."""
    if index >= size:
        return False
    fn, sn, r = index, size - 1, leaf
    for p in proof:
        if sn == 0:
            return False
        if fn & 1 or fn == sn:
            r = itaty.node_hash(p, r)
            if not fn & 1:
                while fn & 1 == 0 and fn != 0:
                    fn >>= 1
                    sn >>= 1
        else:
            r = itaty.node_hash(r, p)
        fn >>= 1
        sn >>= 1
    return sn == 0 and r == root


class Ed25519(unittest.TestCase):
    def test_rfc_8032_vectors(self):
        cases = [
            ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
             "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
             "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bac"
             "c61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
            ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
             "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
             "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e"
             "458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
        ]
        for seed, public, message, signature in cases:
            seed = bytes.fromhex(seed)
            self.assertEqual(ed25519.public_key(seed).hex(), public)
            self.assertEqual(ed25519.sign(seed, bytes.fromhex(message)).hex(), signature)


class Builder(unittest.TestCase):
    def test_the_content_digest_is_the_one_a_conformance_lock_records(self):
        base = os.path.join(REPO, "tests/conformance/project/ita_trust_dependency/deps/util")
        files = {}
        for rel in ("ita.una", "src/keys.una"):
            with open(os.path.join(base, rel), encoding="utf-8") as f:
                files[rel] = f.read()
        self.assertEqual(itaty.content_digest(files), "sha256:d62ff8433f0714c356006087d9de55"
                                                      "fa2a35aa8636a3cf424861b8b6813e04ea")

    def test_the_archive_is_its_listing_then_the_files_in_order(self):
        files = {"src/b.una": "b\n", "ita.una": "i\n"}
        listing = (b"ita.una\x002\x00" + hashlib.sha256(b"i\n").hexdigest().encode() + b"\n"
                   b"src/b.una\x002\x00" + hashlib.sha256(b"b\n").hexdigest().encode() + b"\n")
        self.assertEqual(itaty.archive(files), b"ita-archive/1\n" + str(len(listing)).encode()
                         + b"\n" + listing + b"i\nb\n")

    def test_every_inclusion_proof_verifies_and_a_changed_one_does_not(self):
        for size in range(1, 20):
            leaves = [itaty.leaf_hash(f"entry {i}") for i in range(size)]
            root = itaty.tree_root(leaves)
            for i in range(size):
                proof = itaty.inclusion_proof(i, leaves)
                self.assertTrue(verify_inclusion(i, size, leaves[i], proof, root), (size, i))
                if proof:
                    bad = [bytes([proof[0][0] ^ 1]) + proof[0][1:]] + proof[1:]
                    self.assertFalse(verify_inclusion(i, size, leaves[i], bad, root))

    def test_tile_paths(self):
        self.assertEqual(itaty.index_path(5), "005")
        self.assertEqual(itaty.index_path(1234567), "x001/x234/567")
        files = itaty.log_tiles([b"a", b"b", b"c"])
        self.assertEqual(sorted(files), ["tile/0/000.p/3", "tile/entries/000.p/3"])
        self.assertEqual(files["tile/entries/000.p/3"], b"\x00\x01a\x00\x01b\x00\x01c")

    def test_a_map_of_one_name_hashes_its_leaf_up_through_empty_siblings(self):
        head = (3, bytes(range(32)), 2)
        root, tiles = itaty.map_tiles({"acme/ledger": head})
        key = itaty.map_key("acme/ledger")
        h = itaty.map_leaf(key, head)
        for depth in range(255, -1, -1):
            empty = itaty._EMPTY[255 - depth]
            bit = key[depth // 8] >> (7 - depth % 8) & 1
            h = itaty.node_hash(empty, h) if bit else itaty.node_hash(h, empty)
        self.assertEqual(root, h)
        self.assertEqual(len(tiles), 32)
        self.assertEqual(itaty.map_tiles({})[0], itaty._EMPTY[256])

    def test_a_note_signs_its_text_and_a_cosignature_its_time(self):
        k = itaty.key_from("test", "witness.0")
        doc = itaty.signed("hello", [k])
        text, sigs = doc.split("\n\n", 1)
        self.assertEqual(text, "hello")
        self.assertTrue(sigs.startswith("— witness.0 "))
        line = itaty.cosign_line(k, "hello\n", 7)
        self.assertIn(" witness.0 ", line)
        import base64
        raw = base64.b64decode(line.split(" ")[2])
        self.assertEqual(raw[:4], k.cosignature_id())
        self.assertEqual(int.from_bytes(raw[4:12], "big"), 7)

    def test_the_lock_is_written_as_una_ita_writes_it(self):
        text = itaty.lock_text([("acme", "sha256:" + "a" * 64, 3, "sha256:" + "b" * 64)],
                               [{"name": "acme/ledger", "version": "0.3.1", "from": "acme",
                                 "digest": "sha256:" + "c" * 64,
                                 "signed": "ed25519:acme:" + "d" * 64, "logged": 1,
                                 "granted": [("hunt", "Console[print]")]}])
        self.assertTrue(text.startswith(itaty.LOCK_HEADER + "lock 1\n\nsolver\n"))
        self.assertIn('\nregistry acme\n  root "sha256:', text)
        self.assertIn('\npackage acme/ledger 0.3.1\n  from acme\n  digest "sha256:', text)
        self.assertIn('  logged 1\n  granted by hunt may Console[print]\n', text)


class Resolution(unittest.TestCase):
    old, new = itaty.T0 - 30 * itaty.DAY, itaty.T0 - 3600

    def test_semver_order(self):
        order = ["0.3.0", "0.3.1-alpha", "0.3.1-alpha.1", "0.3.1-alpha.beta", "0.3.1-beta.2",
                 "0.3.1-beta.11", "0.3.1-rc.1", "0.3.1", "0.3.10"]
        self.assertEqual(sorted(reversed(order), key=registry.key), order)

    def test_minimal_selection_and_its_raises(self):
        r = {"0.3.0": (self.old, False), "0.3.1": (self.old, False), "0.3.2": (self.old, False)}
        self.assertEqual(registry.resolve("0.3", [], r, itaty.T0), "0.3.0")
        self.assertEqual(registry.resolve("0.3.1", [], r, itaty.T0), "0.3.1")
        self.assertIsNone(registry.resolve("0.3.3", [], r, itaty.T0))
        self.assertEqual(registry.resolve("0.3", ["0.3.0"], r, itaty.T0), "0.3.1")
        yanked = dict(r, **{"0.3.0": (self.old, True)})
        self.assertEqual(registry.resolve("0.3", [], yanked, itaty.T0), "0.3.1")
        # A raise skips a release logged inside the cooldown, and a pre-release.
        cooling = dict(yanked, **{"0.3.1": (self.new, False), "0.3.1-rc.1": (self.old, False)})
        self.assertEqual(registry.resolve("0.3", [], cooling, itaty.T0), "0.3.2")
        # A selection is not held back by the cooldown: only a raise is.
        self.assertEqual(registry.resolve("0.3.1", [], cooling, itaty.T0), "0.3.1")
        # Another line is another identity.
        self.assertIsNone(registry.resolve("0.4", [], r, itaty.T0))

    def test_every_y1_form_pins_what_it_claims(self):
        for p in programs("Y1"):
            note = dict(part.split(" ", 1) for part in p.note.split(", "))
            if p.variant.startswith("honest"):
                self.assertEqual(p.cls, MUST_ACCEPT)
                self.assertEqual(note["chosen"], note["pinned"], p.name)
            elif p.variant == "no-resolution":
                self.assertEqual((p.cls, p.expect, note["chosen"]), (MUST_REJECT, ["U7013"], "None"))
            elif p.variant in ("wrong-digest", "wrong-logged"):
                self.assertEqual(note["chosen"], note["pinned"], p.name)
            else:
                self.assertNotEqual(note["chosen"], note["pinned"], p.name)
                self.assertEqual(p.expect, ["U7009"])
            self.assertEqual(p.command, "lock-check")
            self.assertEqual(p.mirrors, {"acme": "mirror"})
            self.assertIn("mirror/root/1", p.files)


class Families(unittest.TestCase):
    def test_editions_are_the_probe_and_0_4(self):
        for op in FAMILY_OPS:
            self.assertEqual(editions_of(op), ("0.3", "0.4"))
            for p in programs(op, 20, "0.3"):
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, ["U7008"]), p.name)
                self.assertIn("  edition 0.3\n", p.files["ita.una"])
                self.assertFalse(any(k.startswith(("mirror/", "vendor/")) for k in p.files))

    def test_classes_and_expected_codes(self):
        seen = {}
        for op in FAMILY_OPS:
            for p in programs(op, 120):
                seen.setdefault(op, set()).add(p.variant)
                if p.cls == MUST_ACCEPT:
                    self.assertTrue(p.variant.startswith("honest"), p.name)
                    self.assertEqual(p.expect, [])
                elif p.cls == OPEN:
                    self.assertEqual(op, "D2")
                    self.assertTrue(p.vary, p.name)
                    closed = any(p.variant.startswith(f) for f in dependencies.D2_CLOSED)
                    self.assertEqual(bool(p.expect), closed, p.name)
                else:
                    self.assertEqual(p.cls, MUST_REJECT)
                    self.assertTrue(p.expect, p.name)
                if op in ("Y2", "Y3") and p.cls == MUST_REJECT:
                    self.assertEqual(p.expect, ["U7010"])
        self.assertEqual({v.split("-path")[0].split("-registry")[0] for v in seen["D1"]},
                         set(dependencies.D1_FORMS))
        self.assertEqual(seen["D3"], set(dependencies.D3_FORMS))
        self.assertEqual(seen["Y1"], set(registry.Y1_FORMS))
        self.assertEqual(seen["Y2"], set(registry.Y2_FORMS))
        self.assertEqual(seen["Y3"], set(registry.Y3_FORMS))

    def test_a_registry_dependency_is_vendored_with_its_provenance(self):
        for p in programs("D2", 40):
            if p.variant.endswith("-registry"):
                rel = "vendor/ita/acme/acme+util-0.1.0"
                self.assertIn(rel + "/ita.una", p.files)
                self.assertTrue(p.files[rel + ".provenance"].startswith("ita-provenance/1\n"))
                self.assertIn('\n  acme/util 0.1 from acme\n', p.files["ita.una"])
                break
        else:
            self.fail("no D2 program with a registry dependency")

    def test_the_honest_consent_is_the_digest_of_the_files_loaded(self):
        for p in programs("D3", 80):
            files = {k[len("deps/util/"):]: v for k, v in p.files.items()
                     if k.startswith("deps/util/")}
            consented = f'consented "{itaty.content_digest(files)}"'
            self.assertEqual(consented in p.files["ita.lock"],
                             p.variant in ("honest", "no-trust-line", "module-unnamed",
                                           "lock-modules", "other-major", "from-source",
                                           "narrow-trust"), p.name)

    def test_refactors_leave_every_dependency_file_as_it_is(self):
        for op in FAMILY_OPS:
            for p in programs(op, 20):
                for rid, refactor in REFACTORS.items():
                    out = refactor(p.files, __import__("random").Random(1))
                    if out is None:
                        continue
                    for rel in p.files:
                        if not rel.startswith("src/"):
                            self.assertEqual(out[rel], p.files[rel], (p.name, rid, rel))

    def test_a_program_writes_its_bytes_and_hashes_them(self):
        p = programs("Y1", 1)[0]
        with tempfile.TemporaryDirectory() as d:
            base = p.write(d)
            with open(os.path.join(base, "mirror/root/1"), "rb") as f:
                self.assertEqual(f.read(), p.files["mirror/root/1"])
        self.assertEqual(p.manifest()["command"], "lock-check")
        self.assertNotIn("command", programs("Y2", 1)[0].manifest())
        self.assertTrue(p.has_dependencies)
        self.assertFalse(OPERATORS["B1"](1, "0.3").has_dependencies)


class Drivers(unittest.TestCase):
    def test_lock_check_lists_the_mirror_by_its_absolute_url_in_the_runs_itaoca(self):
        p = programs("Y1", 1)[0]
        seen = {}

        def fake_run(argv, cwd, writable, timeout, env_extra=None, **_):
            itaoca = env_extra["UNA_ITAOCA"]
            with open(os.path.join(itaoca, "mirrors.una"), encoding="utf-8") as f:
                seen["mirrors"] = f.read()
            seen.update(argv=argv, itaoca=itaoca, writable=writable)
            return sandbox.Run(0, '{"diagnostics": []}', "", False, 0.1)

        with tempfile.TemporaryDirectory() as d, mock.patch.object(sandbox, "run", fake_run):
            root = os.path.realpath(d)
            pkg = p.write(root)
            v = drivers.check("una", pkg, root, 10, p)
        self.assertEqual(v.kind, "accepted")
        self.assertEqual(seen["argv"], ["una", "--json", "ita", "lock", "--check", "."])
        self.assertEqual(seen["mirrors"], f'mirrors\n  acme\n    "file://{pkg}/mirror"\n')
        self.assertTrue(seen["itaoca"].startswith(seen["writable"]))

    def test_an_unknown_command_is_an_error(self):
        p = programs("Y2", 1)[0]
        p = Program(op=p.op, rule="", edition="0.4", seed=1, cls=MUST_REJECT, files=p.files,
                    expect=["U7010"], command="publish")
        with self.assertRaises(ValueError):
            drivers.check("una", "/nonexistent", "/nonexistent", 1, p)

    def test_the_export_of_a_program_with_dependencies_must_be_refused_as_una_core_2(self):
        def run_fresh(code, err):
            return lambda *a, **k: sandbox.Run(code, "", err, False, 0.1)
        refusal = ("error: the package has dependencies, whose Core export (`una.core/2`) this "
                   "release does not write yet\n")
        with mock.patch.object(sandbox, "run_fresh", run_fresh(2, refusal)):
            self.assertIsNone(oracle.dependency_export("una", "/p", "/r", 1))
        with mock.patch.object(sandbox, "run_fresh", run_fresh(0, "")):
            self.assertIn("the oracle must read it", oracle.dependency_export("una", "/p", "/r", 1))
        with mock.patch.object(sandbox, "run_fresh", run_fresh(2, "another usage error")):
            self.assertIsNotNone(oracle.dependency_export("una", "/p", "/r", 1))


def parse_provenance(text):
    """(root texts, checkpoint text, its cosignature lines) of a provenance, read apart from the
    builder: each document is length-prefixed (`spec/30 §9` rule 7)."""
    data = text.encode()
    pos = data.index(b"\n") + 1
    roots, checkpoint = [], None
    while True:
        end = data.index(b"\n", pos)
        kind, length = data[pos:end].decode().split(" ")
        doc = data[end + 1:end + 1 + int(length)].decode()
        pos = end + 1 + int(length)
        if kind == "root":
            roots.append(doc)
        elif kind == "checkpoint":
            checkpoint = doc
            break
    body, sigs = checkpoint.split("\n\n", 1)
    return roots, body + "\n", [line for line in sigs.splitlines() if line]


def root_fields(doc):
    """(version, witness threshold, {public key: name} of its witnesses, revoked fingerprints)."""
    lines = doc.split("\n\n", 1)[0].splitlines()
    version = int(lines[2].split()[1])
    at = next(i for i, l in enumerate(lines) if l.startswith("witnesses "))
    k = int(lines[at].split()[1])
    witnesses = {}
    for line in lines[at + 1:at + 1 + 3]:
        _, name, b = line.split()
        witnesses[base64.b64decode(b)] = name
    revoked = set()
    for line in lines:
        if line.startswith("witnesses-revoked "):
            revoked |= set(line.split()[1:])
    return version, k, witnesses, revoked


def counted_cosigners(text):
    """The witnesses that count on a provenance's point and the threshold k, by `spec/30 §9` rule
    7 as the spec states it, written apart from the builder and the client: a witness of the last
    stored root, or one of the named root that the last one retired, never one whose fingerprint
    any stored root revokes; k the larger of the named and the last root's threshold."""
    roots, body, sigs = parse_provenance(text)
    fields = [root_fields(r) for r in roots]
    named = int(next(l for l in body.splitlines() if l.startswith("ita-root ")).split()[1])
    _, named_k, named_w, _ = next(f for f in fields if f[0] == named)
    _, last_k, last_w, _ = fields[-1]
    revoked = set().union(*(f[3] for f in fields))
    by_id = {}
    for pub, name in {**named_w, **last_w}.items():
        cid = hashlib.sha256(name.encode() + b"\n" + bytes([4]) + pub).digest()[:4]
        by_id[(name, cid)] = pub
    counted = set()
    for line in sigs[1:]:
        _, name, blob = line.split()
        pub = by_id.get((name, base64.b64decode(blob)[:4]))
        if pub is not None and hashlib.sha256(pub).hexdigest() not in revoked:
            counted.add(pub)
    return len(counted), max(named_k, last_k), len(roots)


class Rotations(unittest.TestCase):
    # How many root versions each form of Y3 stores, the first included.
    ROOTS = {"honest-retired": 2, "honest-two-retired": 3, "honest-revoked-enough": 2,
             "honest-raised-enough": 2, "honest-raised-lowered": 3, "honest-new-witness": 2,
             "revoked-then-retired": 3, "retired-revoked-later": 3, "raised-to-three": 2}

    def test_every_y3_form_counts_as_its_class_claims(self):
        # The hunt's rotation code, checked by an independent count of the provenance it writes:
        # a control counts at least k witnesses and a forgery fewer, so a forgery's rejection
        # is for the rule it breaks and not for a defect of `rotate_witness` or `add_root`.
        seen = set()
        for p in programs("Y3", 120):
            prov = next(v for k, v in p.files.items() if k.endswith(".provenance"))
            count, k, roots = counted_cosigners(prov)
            self.assertEqual(roots, self.ROOTS[p.variant], p.name)
            if p.cls == MUST_ACCEPT:
                self.assertGreaterEqual(count, k, p.name)
            else:
                self.assertEqual(p.expect, ["U7010"])
                self.assertLess(count, k, p.name)
            seen.add(p.variant)
        self.assertEqual(seen, set(registry.Y3_FORMS))

    def test_the_y2_rotation_forgeries_count_below_their_threshold(self):
        for p in programs("Y2", 200):
            if p.variant not in ("revoked-witness", "threshold-raised"):
                continue
            prov = next(v for k, v in p.files.items() if k.endswith(".provenance"))
            count, k, roots = counted_cosigners(prov)
            self.assertEqual((roots, count, k), (2, 1, 2), p.name)

    def test_the_rotated_controls_are_in_the_fixed_run(self):
        # Every form of Y3, and so a rotated control beside each rotated forgery, is judged by
        # the gate's fixed run, not only by a larger sample of seeds.
        forms = {p.variant for p in gate.programs() if p.op == "Y3" and p.edition == "0.4"}
        self.assertEqual(forms, set(registry.Y3_FORMS))

    def test_a_rotated_control_differs_from_its_forgery_only_in_what_counts(self):
        # The same seed's control and forgery share the release and the vendored files: only
        # the provenance (its later roots and the point's cosignatures) and the lock's point
        # change, and the pin too where the first root's threshold differs (one, raised).
        seed = derive_seed(11, "Y3", 0)
        for control, forgery, pin in (("honest-retired", "revoked-then-retired", False),
                                      ("honest-revoked-enough", "retired-revoked-later", False),
                                      ("honest-raised-lowered", "raised-to-three", False),
                                      ("honest-raised-enough", "raised-to-three", True)):
            a = registry.y3(seed, "0.4", form=control).files
            b = registry.y3(seed, "0.4", form=forgery).files
            self.assertEqual(sorted(a), sorted(b))
            changed = {k for k in a if a[k] != b[k]}
            allowed = (".provenance", "ita.lock") + (("ita.una",) if pin else ())
            self.assertTrue(changed and all(k.endswith(allowed) for k in changed), changed)
            self.assertEqual("ita.una" in changed, pin, (control, forgery))


class Gate(unittest.TestCase):
    def test_the_new_canaries_are_what_they_claim(self):
        dep = gate.canary_reject_dependency()
        self.assertEqual((dep.cls, dep.command), (MUST_REJECT, "lock-check"))
        self.assertEqual(registry.y1(dep.seed, "0.4", form="honest").files, dep.files)
        ven = gate.canary_reject_vendored()
        self.assertEqual(registry.y2(ven.seed, "0.4", form="honest").files, ven.files)
        orc = gate.canary_oracle_dependency()
        self.assertTrue(orc.has_dependencies)
        pos = orc.vary[0]
        line = orc.files[pos["file"]].split("\n")[pos["line"] - 1]
        self.assertEqual(line.strip(), "let x: Int = 5")

    def test_the_unexported_pins_are_the_generators_accepted_programs(self):
        # The fixed run's programs of several packages that a correct checker accepts: the
        # controls and D2's open forms are exported as `una.core/2`, which is refused; Y1's
        # honest forms are judged by `lock --check` and not loaded.
        progs = [p for p in gate.programs() if p.op in FAMILY_OPS and p.edition == "0.4"]
        accepted = [p for p in progs if p.cls == MUST_ACCEPT or (p.cls == OPEN and not p.expect)]
        got = {"dependencies": sum(1 for p in accepted if p.command == "check"),
               "not loaded": sum(1 for p in accepted if p.command != "check")}
        self.assertEqual(got, gate.PINNED_UNEXPORTED)


if __name__ == "__main__":
    unittest.main()
