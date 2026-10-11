"""Tests of the hunt's generators and drivers that need no compiler.

    cd scripts/security-hunt && python3 -m unittest discover -s tests
"""

import json
import os
import random
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from hunt import drivers, sandbox  # noqa: E402
from hunt.families import OPERATORS  # noqa: E402
from hunt.program import CLASSES, MUST_REJECT, OPEN, derive_seed, rng_for  # noqa: E402
from hunt.refactor import _blocks, r6_rename, r9_reorder  # noqa: E402
from hunt.sandbox import Run  # noqa: E402

EDITIONS = ("0.1", "0.2")


def every_program(n=40):
    for op in sorted(OPERATORS):
        for i in range(n):
            for ed in EDITIONS:
                yield OPERATORS[op](derive_seed(1, op, i), ed)


class Generators(unittest.TestCase):
    def test_every_operator_builds_a_package(self):
        for p in every_program():
            self.assertIn(p.cls, CLASSES)
            self.assertIn("una.toml", p.files)
            self.assertIn("verification.lock", p.files)
            self.assertTrue(any(k.endswith(".una") for k in p.files), p.name)
            for rel, text in p.files.items():
                self.assertNotIn("⟦", text, f"{p.name}: a varied marker left in {rel}")
                self.assertTrue(text.endswith("\n"), f"{p.name}: {rel}")
            if p.cls == MUST_REJECT:
                self.assertTrue(p.expect, p.name)

    def test_a_seed_rebuilds_the_same_program(self):
        for op in OPERATORS:
            for ed in EDITIONS:
                seed = derive_seed(99, op, 3)
                a, b = OPERATORS[op](seed, ed), OPERATORS[op](seed, ed)
                self.assertEqual(a.digest(), b.digest())
                self.assertEqual(a.name, b.name)

    def test_seeds_depend_on_campaign_operator_and_index(self):
        self.assertEqual(derive_seed(1, "T1", 0), derive_seed(1, "T1", 0))
        self.assertNotEqual(derive_seed(1, "T1", 0), derive_seed(2, "T1", 0))
        self.assertNotEqual(derive_seed(1, "T1", 0), derive_seed(1, "T2", 0))
        self.assertNotEqual(derive_seed(1, "T1", 0), derive_seed(1, "T1", 1))

    def test_operators_reach_their_variants(self):
        seen = {}
        for p in every_program(120):
            seen.setdefault(p.op, set()).add(p.variant)
        self.assertGreaterEqual(len(seen["T3"]), 12)
        self.assertGreaterEqual(len(seen["T4"]), 12)
        self.assertTrue(any("together" in v for v in seen["T4"]))
        self.assertEqual(len(seen["T8"]), 6)
        self.assertEqual(len(seen["H1"]), 8)
        self.assertEqual(len(seen["H4"]), 3)
        self.assertEqual(len(seen["K2"]), 5)
        self.assertEqual(len(seen["K4"]), 3)
        self.assertFalse(any(p.edition == "0.1" and p.op == "K4" and p.variant != "text"
                             for p in every_program()))

    def test_keys_programs_that_run_carry_their_key_file(self):
        for p in every_program():
            if p.op == "K4":
                self.assertIn("keys.toml", p.files)
                self.assertIn('"keys:keys.toml"', p.files["una.toml"])
                self.assertIn('categories = ["PII"]', p.files["una.toml"])

    def test_together_only_in_edition_0_2(self):
        for p in every_program():
            # Family C uses `together` in 0.1 on purpose, as a must-reject check of the gate.
            if p.edition == "0.1" and not p.op.startswith("C"):
                self.assertFalse(any("together" in t for t in p.files.values()), p.name)

    def test_listing_of_t1_t2_t3(self):
        for p in every_program():
            toml = p.files["una.toml"]
            keys = p.files.get("src/app/keys.una", "")
            if p.op == "T1" and p.variant == "header-only":
                self.assertTrue(keys.startswith("trusted module app.keys"))
                self.assertNotIn('"app.keys"', toml)
            if p.op == "T1" and p.variant == "listed-only":
                self.assertTrue(keys.startswith("module app.keys"))
                self.assertIn('modules = ["app.keys"]', toml)
            if p.op == "T2":
                self.assertTrue(keys.startswith("module app.keys"))
            if p.op == "T3":
                self.assertTrue(keys.startswith("trusted module app.keys"))
                self.assertNotIn('modules = ["app.keys"]', toml)

    def test_open_programs_with_main_mark_their_varied_literal(self):
        found = 0
        for p in every_program():
            for pos in p.vary:
                found += 1
                lines = p.files[pos["file"]].split("\n")
                line = lines[pos["line"] - 1]
                self.assertEqual(line[pos["col"] - 1:pos["col"] - 1 + len(pos["literal"])],
                                 pos["literal"])
                self.assertIn("[run]", p.files["una.toml"])
        self.assertGreater(found, 0)

    def test_h5_rejects_only_a_failure_passed_under_a_secret_pc(self):
        # Campaign 2's triage: a failure handled inside the secret branch never leaves it.
        for p in every_program(80):
            if p.op != "H5":
                continue
            self.assertEqual(p.cls, MUST_REJECT if p.variant == "pc-secret-pass" else OPEN,
                             p.variant)

    def test_no_generator_writes_a_shape_campaign_2_found_ill_formed(self):
        for p in every_program():
            text = "\n".join(v for k, v in p.files.items() if k.endswith(".una"))
            self.assertNotIn("to_text()", text, p.name)
            self.assertNotIn('\\"', text, p.name)
            for line in text.split("\n"):
                if "@keys.hmac(" in line or "@inner.hmac(" in line:
                    i = text.split("\n").index(line)
                    self.assertIn("if error pass", text.split("\n")[i + 1], p.name)

    def test_h6_probes_are_must_reject(self):
        seen = set()
        for p in every_program(60):
            if p.op == "H6":
                self.assertEqual(p.cls, MUST_REJECT)
                seen.add(p.variant)
        self.assertEqual(len(seen), 6)

    def test_t11_internal_pc_expects_the_call_under_it(self):
        for p in every_program(80):
            if p.op == "T11" and "-pc-internal-" in p.variant:
                self.assertEqual(p.expect, ["U4002"], p.variant)

    def test_p_family_classes(self):
        seen = {}
        for p in every_program(80):
            if p.op not in ("P1", "P2", "P3"):
                continue
            seen.setdefault(p.op, set()).add(p.variant)
            if p.op == "P1" or p.variant.endswith("-twin") or p.variant.endswith("len-const"):
                self.assertEqual(p.cls, OPEN, p.variant)
            else:
                self.assertEqual(p.cls, MUST_REJECT, p.variant)
                self.assertTrue(p.expect, p.variant)
            if p.op == "P2" and "len-dep" in p.variant:
                self.assertIn("if pin > 0 then [pin] else []", p.files["src/app/main.una"])
        self.assertTrue({"return-len-dep", "return-len-dep-top", "print-len-const"} <= seen["P2"])
        self.assertTrue(any(v.startswith("which-variant") for v in seen["P3"]))

    def test_g_family_classes(self):
        seen = {}
        for p in every_program(60):
            if p.op not in ("G1", "G2", "G3"):
                continue
            seen.setdefault(p.op, set()).add(p.variant)
            text = p.files["src/app/main.una"]
            if p.op == "G1" or (p.op == "G3" and not p.variant.endswith("-twin")):
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, ["U4001"]), p.variant)
                self.assertIn("secret Int]", text)
            else:
                self.assertEqual(p.cls, OPEN, p.variant)
            self.assertNotIn("use(", text.replace("feed_sink(", ""))
        self.assertEqual(len(seen["G1"]), 16)
        self.assertEqual(seen["G3"], {"chain", "pair", "chain-twin", "pair-twin"})

    def test_c_family_is_edition_0_2_and_gated_in_0_1(self):
        for p in every_program(40):
            if p.op not in ("C1", "C2", "C3"):
                continue
            self.assertIn("together", p.files["src/app/main.una"])
            if p.edition == "0.1":
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, ["U0103"]), p.name)
            elif p.variant.endswith("twin") or p.variant == "twin-public-pc":
                self.assertEqual(p.cls, OPEN, p.variant)
            else:
                self.assertEqual(p.cls, MUST_REJECT, p.variant)

    def test_s_family_classes(self):
        seen = set()
        for p in every_program(60):
            if p.op not in ("S1", "S2", "S3"):
                continue
            seen.add((p.op, p.variant))
            text = p.files["src/app/main.una"]
            self.assertNotIn(" in [", text, p.name)
            if p.variant.endswith("-twin"):
                self.assertEqual(p.cls, OPEN, p.variant)
                self.assertNotIn("secret", text, p.name)
            else:
                self.assertEqual(p.cls, MUST_REJECT, p.variant)
                self.assertTrue(p.expect, p.variant)
        self.assertTrue(any(op == "S2" and v.startswith("is-") for op, v in seen))

    def test_o_family_classes(self):
        for p in every_program(60):
            if p.op not in ("O1", "O2", "O3"):
                continue
            if p.edition == "0.1" and p.op in ("O1", "O3"):
                self.assertEqual(p.cls, MUST_REJECT, p.name)
                self.assertEqual(p.expect, ["U1001"] if p.op == "O1" else ["U0101"])
            elif p.variant.endswith("-twin"):
                self.assertEqual(p.cls, OPEN, p.variant)
            else:
                self.assertEqual(p.cls, MUST_REJECT, p.variant)
            if p.op == "O3" and p.variant.startswith("to-int"):
                self.assertIn("T: Integer", p.files["src/app/main.una"])

    def test_f_family_is_open_and_covers_every_carrier(self):
        from hunt.families.fragment import CARRIERS, ENDINGS
        seen = set()
        for p in every_program(120):
            if p.op != "F1":
                continue
            self.assertEqual((p.cls, p.expect), (OPEN, []))
            seen.add(p.variant)
        carriers = {v.split("-", 1)[0] for v in seen}
        self.assertTrue({c.split("-")[0] for c in CARRIERS} <= carriers | {"first"})
        self.assertTrue(any(v.endswith("write-internal") for v in seen))

    def test_parse_recheck(self):
        out = "doc.json\tmain.run\taccepted\tany host\ndoc.json\tmain.g\tnot re-checked\tthe type `X`\n"
        rows = drivers.parse_recheck(out)
        self.assertEqual([r["outcome"] for r in rows], ["accepted", "not re-checked"])
        self.assertEqual(rows[1]["note"], "the type `X`")

    def test_t11_class_follows_the_failure_label(self):
        for p in every_program(80):
            if p.op != "T11":
                continue
            public = p.variant.startswith("to-public-pc-public")
            self.assertEqual(p.cls, OPEN if public else MUST_REJECT, p.variant)


class Refactors(unittest.TestCase):
    TEXT = (
        "-- test: x\nmodule m\n\nimport a (b)\n\n"
        "pub f(x: Int) -> Int = g(x)\n\n"
        "g(x: Int) -> Int\n=\n  let s = \"g(x) f\"\n  x -- g\n\n"
        "--- doc of k\npub kind K = | A\n"
    )

    def test_r6_renames_outside_strings_and_comments(self):
        out = r6_rename({"src/m.una": self.TEXT, "una.toml": "x"}, random.Random(1))
        t = out["src/m.una"]
        self.assertIn('"g(x) f"', t)
        self.assertIn("-- g", t)
        self.assertNotIn("pub f(", t)
        self.assertNotIn("= g(x)", t)
        self.assertEqual(out["una.toml"], "x")

    def test_r6_renames_inside_interpolations(self):
        text = 'module m\n\nmask(t: Int) -> Int = t\n\npub show(t: Int) -> Text = "{mask(t)} mask"\n'
        out = r6_rename({"src/m.una": text}, random.Random(2))["src/m.una"]
        self.assertNotIn("{mask(t)}", out)
        self.assertIn('} mask"', out)

    def test_r9_keeps_every_declaration(self):
        head, decls = _blocks(self.TEXT)
        self.assertTrue(head.endswith("import a (b)"))
        self.assertEqual(len(decls), 3)
        self.assertTrue(decls[2].startswith("--- doc of k"))
        out = r9_reorder({"src/m.una": self.TEXT}, random.Random(3))
        _, after = _blocks(out["src/m.una"])
        self.assertEqual(sorted(after), sorted(decls))
        self.assertNotEqual(after, decls)

    def test_refactors_apply_to_generated_programs(self):
        applied = 0
        for p in every_program(5):
            for fn in (r6_rename, r9_reorder):
                out = fn(p.files, rng_for(p.seed))
                if out is not None:
                    applied += 1
                    self.assertEqual(set(out), set(p.files))
        self.assertGreater(applied, 0)


class Checks(unittest.TestCase):
    def diag(self, code):
        return {"schema": "una.diagnostic/1", "code": code, "severity": "error",
                "primary": {"file": "src/a.una", "start": {"line": 3, "col": 5}}}

    def test_parse_check(self):
        ok = drivers.parse_check(Run(0, '{"schema":"una.diagnostics/1","diagnostics":[]}', "",
                                     False, 0.1))
        self.assertEqual(ok.kind, "accepted")
        doc = json.dumps({"schema": "una.diagnostics/1", "diagnostics": [self.diag("U4005")]})
        bad = drivers.parse_check(Run(1, doc, "", False, 0.1))
        self.assertEqual(bad.kind, "rejected")
        self.assertEqual(bad.codes, [["U4005", "src/a.una", 3, 5]])
        self.assertEqual(drivers.parse_check(Run(3, "", "boom", False, 0.1)).kind, "ice")
        self.assertEqual(drivers.parse_check(Run(None, "", "", True, 9)).kind, "timeout")
        self.assertEqual(drivers.parse_check(Run(1, doc.replace("U4005", "U4005"), "", False,
                                                 0)).kind, "rejected")
        self.assertEqual(drivers.parse_check(Run(0, doc, "", False, 0)).kind, "inconsistent")

    def test_sandbox_profile_denies_network_and_writes(self):
        prof = sandbox.profile("/tmp/x")
        self.assertIn("(deny network*)", prof)
        self.assertIn("(deny file-write*)", prof)
        self.assertNotIn("network-outbound", prof)
        self.assertIn('localhost:8080', sandbox.profile("/tmp/x", mock_port=8080))

    def test_remove_dir_stays_under_root(self):
        with tempfile.TemporaryDirectory() as root:
            inner = sandbox.fresh_dir(root, "r-")
            sandbox.remove_dir(inner, root)
            self.assertFalse(os.path.exists(inner))
            with self.assertRaises(ValueError):
                sandbox.remove_dir(root, root)
            with self.assertRaises(ValueError):
                sandbox.remove_dir("/tmp", root)

    def test_rebuild_detects_a_changed_generator(self):
        p = OPERATORS["T4"](derive_seed(5, "T4", 0), "0.2")
        row = dict(p.manifest(), digest=p.digest())
        self.assertEqual(drivers.rebuild(row).digest(), p.digest())
        row["digest"] = "0" * 64
        with self.assertRaises(ValueError):
            drivers.rebuild(row)


if __name__ == "__main__":
    unittest.main()
