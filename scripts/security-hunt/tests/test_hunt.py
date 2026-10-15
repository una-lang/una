"""Tests of the hunt's generators and drivers that need no compiler.

    cd scripts/security-hunt && python3 -m unittest discover -s tests
"""

import io
import json
import os
import random
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from hunt import drivers, gate, oracle, sandbox  # noqa: E402
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
            # A program that uses `together` in 0.1 on purpose is a must-reject check of the
            # gate (U0103): families C, CS, CP and TC.
            if p.edition == "0.1" and p.expect != ["U0103"]:
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

    def test_f_family_covers_every_carrier(self):
        from hunt.families.fragment import CARRIERS, ENDINGS
        seen = set()
        for p in every_program(120):
            if p.op != "F1":
                continue
            seen.add(p.variant)
        carriers = {v.split("-", 1)[0] for v in seen}
        self.assertTrue({c.split("-")[0] for c in CARRIERS} <= carriers | {"first"})
        self.assertTrue(any(v.endswith("write-internal") for v in seen))

    def test_f7_covers_every_prelude_carrier(self):
        from hunt.families.fragment import PRELUDE_CARRIERS
        seen = set()
        for p in every_program(200):
            if p.op != "F7":
                continue
            seen.add(p.variant)
        endings = ("secret-result", "secret-branch", "public-result", "print", "print-under",
                   "write-internal")
        carriers = {next(v[:-len(e) - 1] for e in endings if v.endswith("-" + e)) for v in seen}
        self.assertEqual(carriers, set(PRELUDE_CARRIERS))

    def test_f_public_endings_are_classified_without_changing_programs(self):
        # Una-2's review of #547: a public ending is must-reject for every carrier but the
        # structural `is-ok`, which is must-accept; the programs' text is unchanged, so earlier
        # seeds rebuild to their digests (seed 11's first F1 program, below).
        from hunt.families.fragment import PUBLIC_ENDINGS, LEAK_CODES, f1
        from hunt.program import MUST_ACCEPT, MUST_REJECT
        for p in every_program(120):
            if p.op not in ("F1", "F7"):
                continue
            ending = next(e for e in ("secret-result", "secret-branch", "public-result",
                                      "print-under", "print", "write-internal")
                          if p.variant.endswith("-" + e))
            carrier = p.variant[:-len(ending) - 1]
            if ending not in PUBLIC_ENDINGS:
                self.assertEqual((p.cls, p.expect), (OPEN, []), p.variant)
            elif p.op == "F7" and carrier == "is-ok":
                self.assertEqual((p.cls, p.expect), (MUST_ACCEPT, []), p.variant)
            else:
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, LEAK_CODES), p.variant)
        p = f1(derive_seed(11, "F1", 0), "0.1")
        self.assertEqual(p.variant, "fold-print-under")
        self.assertTrue(p.digest().startswith("d53a6d3c2efa7957"))

    def test_f8_classes_and_recheck_expectations(self):
        # N-138: a public ending of a secret is must-reject; a secret ending, and the structural
        # `public` carrier at any ending, must-accept; the residual is its own class, never clean.
        from hunt.families.fragment import (FIXED_TABLES, LEAK_CODES, PUBLIC_ENDINGS,
                                            RESIDUAL_ENDINGS)
        from hunt.program import MUST_ACCEPT, MUST_REJECT, RESIDUAL
        kinds = {c: k for k, table in FIXED_TABLES for c in table}
        seen = set()
        for p in every_program(300):
            if p.op != "F8":
                continue
            carrier = next(c for c in sorted(kinds, key=len, reverse=True)
                           if p.variant.startswith(c + "-"))
            ending = p.variant[len(carrier) + 1:]
            if p.edition == "0.1":
                self.assertEqual((p.cls, p.expect, p.recheck), (MUST_REJECT, ["U1001"], ""))
                continue
            seen.add(carrier)
            self.assertEqual(p.recheck, kinds[carrier])
            if kinds[carrier] == "residual":
                self.assertEqual(p.cls, RESIDUAL)
                self.assertIn(ending, RESIDUAL_ENDINGS)
                self.assertNotIn("requires", p.files["src/main.una"] if "src/main.una" in p.files
                                 else "".join(p.files.values()))
            elif ending in PUBLIC_ENDINGS and carrier != "public":
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, LEAK_CODES), p.variant)
            else:
                self.assertEqual(p.cls, MUST_ACCEPT, p.variant)
        self.assertEqual(seen, set(kinds))

    def test_f8_lets_over_the_secret_are_annotated_secret(self):
        # Seed 13's run: `let t: U8 = s + 1` is U4001 at the `let` itself, so a carrier that
        # annotates a `let` over `s` with a public type tests nothing of fixed-width arithmetic.
        import re
        from hunt.families.fragment import FIXED_TABLES
        for _, table in FIXED_TABLES:
            for name, (_, _, body, _) in table.items():
                for m in re.finditer(r"let (\w+): ([^=]+?) = ([^\n]*)", body):
                    rhs = m.group(3)
                    over_secret = re.search(r"\bs\b", rhs) or rhs.startswith("when ")
                    if over_secret:
                        # `Result[secret U8, U8]` labels the value it holds: that is enough.
                        self.assertIn("secret", m.group(2), (name, m.group(0)))

    def test_f8_leaves_earlier_programs_unchanged(self):
        # F8 has tables of its own: F1's and F7's programs rebuild to the digests of seed 11.
        from hunt.families.fragment import f1
        p = f1(derive_seed(11, "F1", 0), "0.1")
        self.assertTrue(p.digest().startswith("d53a6d3c2efa7957"))
        self.assertNotIn("expect_recheck", p.manifest())

    FIXED_DOC = json.dumps({"schema": "una.core/1", "complete": True, "modules": [
        {"path": "app.main", "decls": [{"kind": "fun", "value": {"name": "run"}}]}]})

    def judge_fixed(self, line, expect_main):
        return drivers.judge_differ(Run(0, self.FIXED_DOC, "", False, 0.1), self.FIXED_DOC,
                                    Run(0, line, "", False, 0.1), main="app.main.run",
                                    expect_main=expect_main)

    def test_differ_f8_not_rechecked_is_clean_only_for_its_reason(self):
        why = "F1 restriction (`div` on operands of mixed or unknown integer types)"
        j = self.judge_fixed(f"c.json\tapp.main.run\tnot re-checked\t{why}\n", "not-rechecked")
        self.assertEqual(j["kind"], "clean")
        j = self.judge_fixed("c.json\tapp.main.run\tnot re-checked\tthe form `x`\n", "not-rechecked")
        self.assertEqual(j["kind"], "unknown")
        j = self.judge_fixed("c.json\tapp.main.run\taccepted\t\n", "not-rechecked")
        self.assertEqual(j["kind"], "finding")

    def test_residual_is_invalid_unless_the_full_check_decides(self):
        # N-138, N-146: only an acceptance, or a rejection by an obligation, with the solver.
        V = drivers.Verdict
        after = "overflow-print-after"
        cases = [
            (None, "invalid (no solver)"),
            (V("rejected", [["U5003", "a", 1, 1]], 0.1), "invalid (U5003)"),
            (V("timeout", [], 60.0), "invalid (timeout)"),
            (V("ice", [], 0.1), "invalid (ice)"),
            (V("inconsistent", [], 0.1), "invalid (inconsistent)"),
            (V("rejected", [["U4001", "a", 1, 1]], 0.1), "invalid (rejected by U4001)"),
        ]
        for full, kind in cases:
            r = drivers.judge_residual("accepted", full, after)
            self.assertEqual((r["full_kind"], r["invalid"], r["triage"]), (kind, True, False), kind)
        r = drivers.judge_residual("accepted", V("rejected", [["U5001", "a", 1, 1]], 0.1), after)
        self.assertEqual((r["full_kind"], r["invalid"], r["triage"], r["triage_tier0"]),
                         ("rejected", False, False, True))

    def test_residual_triage_only_on_an_accepted_print_after(self):
        V = drivers.Verdict
        ok = V("accepted", [], 0.1)
        self.assertTrue(drivers.judge_residual("accepted", ok, "zero-div-print-after")["triage"])
        r = drivers.judge_residual("accepted", ok, "zero-div-secret-result")
        self.assertEqual((r["triage"], r["triage_tier0"]), (False, False))
        r = drivers.judge_residual("rejected", ok, "zero-div-print-after")
        self.assertEqual((r["triage"], r["triage_tier0"]), (True, False))

    def test_differ_residual_rows_never_say_clean(self):
        # The residual path of `differ`, with the compiler and the re-check stubbed: the rows file
        # carries `residual`, never `clean`, and no solver makes the row invalid.
        from hunt.families.fragment import f8
        from hunt.program import RESIDUAL
        prog = next(p for p in (f8(derive_seed(13, "F8", i), "0.2") for i in range(200))
                    if p.cls == RESIDUAL and p.variant.endswith("-print-after"))
        row = dict(prog.manifest(), digest=prog.digest())
        core = json.dumps({"schema": "una.core/1", "complete": True, "modules": [
            {"path": "app.main", "decls": [{"kind": "fun", "value": {"name": "run"}}]}]})
        ok = json.dumps({"schema": "una.diagnostics/1", "diagnostics": []})

        def fake(argv, cwd, writable, timeout, env_extra=None, mock_port=None):
            if "core" in argv:
                return Run(0, core, "", False, 0.1)
            if argv[0] == "/cc":
                return Run(0, "c.json\tapp.main.run\taccepted\t\n", "", False, 0.1)
            return Run(0, ok, "", False, 0.1)
        real = sandbox.run
        sandbox.run = fake
        try:
            with tempfile.TemporaryDirectory() as root:
                out = io.StringIO()
                drivers.differ([row], "/una", "/cc", root, 10, out)
                r = json.loads(out.getvalue())
        finally:
            sandbox.run = real
        self.assertEqual((r["differ"], r["differ_raw"]), ("residual", "clean"))
        self.assertEqual((r["invalid"], r["full_kind"], r["triage"], r["triage_tier0"]),
                         (True, "invalid (no solver)", False, True))

    def test_differ_f8_outside_is_expected_unknown(self):
        j = self.judge_fixed("c.json\tapp.main.run\tnot re-checked\tthe primitive `fixed`\n",
                             "outside")
        self.assertEqual((j["kind"], j.get("expected")), ("unknown", True))
        j = self.judge_fixed("c.json\tapp.main.run\taccepted\t\n", "outside")
        self.assertEqual(j["kind"], "finding")
        # A missing line fails closed, as for predicates.
        j = self.judge_fixed("c.json\tapp.main.other\taccepted\t\n", "outside")
        self.assertEqual(j["kind"], "unknown")
        self.assertIn("no line", j["detail"])

    def test_differ_main_not_rechecked_is_unknown(self):
        # Family F stays in the re-check's vocabulary: its own declaration not re-checked is
        # `unknown`, never clean.
        lines = "c.json\tapp.main.run\tnot re-checked\twhy\nc.json\tapp.main.same\taccepted\t\n"
        j = drivers.judge_differ(Run(0, self.DOC, "", False, 0.1), self.DOC,
                                 Run(0, lines, "", False, 0.1), main="app.main.run")
        self.assertEqual(j["kind"], "unknown")
        self.assertIn("app.main.run", j["detail"])
        j = drivers.judge_differ(Run(0, self.DOC, "", False, 0.1), self.DOC,
                                 Run(0, lines, "", False, 0.1))
        self.assertEqual(j["kind"], "clean")

    def test_parse_recheck(self):
        out = "doc.json\tmain.run\taccepted\tany host\ndoc.json\tmain.g\tnot re-checked\tthe type `X`\n"
        rows = drivers.parse_recheck(out)
        self.assertEqual([r["outcome"] for r in rows], ["accepted", "not re-checked"])
        self.assertEqual(rows[1]["note"], "the type `X`")

    DOC = json.dumps({"schema": "una.core/1", "complete": True, "modules": [
        {"path": "app.main", "decls": [{"kind": "fun", "value": {"name": "run"}},
                                       {"kind": "fun", "value": {"name": "same"}}]}]})
    LINES = "c.json\tapp.main.run\taccepted\t\nc.json\tapp.main.same\tnot re-checked\twhy\n"

    def judge(self, exp=None, doc=None, rec=None):
        exp = exp or Run(0, doc if doc is not None else self.DOC, "", False, 0.1)
        return drivers.judge_differ(exp, exp.stdout, rec)

    def test_differ_clean_needs_every_condition(self):
        self.assertEqual(self.judge(rec=Run(0, self.LINES, "", False, 0.1))["kind"], "clean")

    def test_differ_predicate_outside_the_flow_model_is_clean(self):
        # #532: a refinement predicate has its own category, counted apart and never dropped.
        lines = self.LINES + "c.json\tprelude.is_even\toutside the flow model\trefinement predicate\n"
        self.assertEqual(self.judge(rec=Run(0, lines, "", False, 0.1))["kind"], "clean")
        rows = drivers.parse_recheck(lines)
        self.assertEqual([r["outcome"] for r in rows],
                         ["accepted", "not re-checked", "outside the flow model"])

    def test_differ_needs_a_line_for_each_predicate(self):
        # Una-2's review of #539: a predicate's line is expected too, so dropping it is `unknown`.
        doc = json.loads(self.DOC)
        doc["modules"].append({"path": "prelude", "decls": [{"kind": "pred", "name": "is_even"}]})
        doc = json.dumps(doc)
        line = "c.json\tprelude.is_even\toutside the flow model\trefinement predicate\n"
        self.assertEqual(self.judge(doc=doc, rec=Run(0, self.LINES + line, "", False, 0.1))["kind"],
                         "clean")
        j = self.judge(doc=doc, rec=Run(0, self.LINES, "", False, 0.1))
        self.assertEqual(j["kind"], "unknown")
        self.assertIn("prelude.is_even", j["detail"])

    def test_differ_predicate_line_must_be_outside_the_flow_model(self):
        # Una-2's review of #540: a predicate's line saying anything else is `unknown`.
        doc = json.loads(self.DOC)
        doc["modules"].append({"path": "prelude", "decls": [{"kind": "pred", "name": "is_even"}]})
        doc = json.dumps(doc)
        for outcome in ("not re-checked", "accepted"):
            line = f"c.json\tprelude.is_even\t{outcome}\twhy\n"
            j = self.judge(doc=doc, rec=Run(0, self.LINES + line, "", False, 0.1))
            self.assertEqual(j["kind"], "unknown", outcome)
            self.assertIn("prelude.is_even", j["detail"])
        # A whole-document fallback reports every line not re-checked for one reason.
        lines = "".join(f"c.json\t{d}\tnot re-checked\ta policy F1 cannot read\n"
                        for d in ("app.main.run", "app.main.same", "prelude.is_even"))
        self.assertEqual(self.judge(doc=doc, rec=Run(0, lines, "", False, 0.1))["kind"], "clean")

    def test_differ_rejected_is_a_finding(self):
        lines = self.LINES.replace("not re-checked\twhy", "rejected\ta flow")
        self.assertEqual(self.judge(rec=Run(1, lines, "", False, 0.1))["kind"], "finding")

    def test_differ_rejection_before_a_timeout_or_crash_is_a_finding(self):
        lines = self.LINES.replace("not re-checked\twhy", "rejected\ta flow")
        for rec in (Run(None, lines, "", True, 9), Run(3, lines, "panic", False, 0.1)):
            j = self.judge(rec=rec)
            self.assertEqual(j["kind"], "finding")
            self.assertTrue(j["detail"].startswith("a rejected declaration, then"), j["detail"])

    def test_differ_unknown_cases(self):
        rec_ok = Run(0, self.LINES, "", False, 0.1)
        incomplete = self.DOC.replace('"complete": true', '"complete": false')
        cases = {
            "export timeout": dict(exp=Run(None, "", "", True, 9), rec=None),
            "export exit": dict(exp=Run(1, "", "boom", False, 0.1), rec=None),
            "export not JSON": dict(doc="not json", rec=rec_ok),
            "incomplete": dict(doc=incomplete, rec=rec_ok),
            "re-check did not run": dict(rec=None),
            "re-check timeout": dict(rec=Run(None, "", "", True, 9)),
            "re-check crash": dict(rec=Run(3, self.LINES, "panic", False, 0.1)),
            "error outcome": dict(rec=Run(1, "c.json\t-\terror\tbad\n", "", False, 0.1)),
            "exit 1 without rejected": dict(rec=Run(1, self.LINES, "", False, 0.1)),
            "missing declaration": dict(rec=Run(0, self.LINES.split("\n")[0] + "\n", "", False, 0.1)),
            "no output": dict(rec=Run(0, "", "", False, 0.1)),
            "odd outcome": dict(rec=Run(0, self.LINES.replace("accepted", "maybe"), "", False, 0.1)),
        }
        for name, kw in cases.items():
            j = self.judge(**kw)
            self.assertEqual(j["kind"], "unknown", name)
            self.assertTrue(j["detail"], name)

    def test_composed_families_classes(self):
        seen = set()
        for p in every_program(60):
            if p.op not in ("CS1", "CS2", "PG1"):
                continue
            seen.add((p.op, p.variant.replace("-twin", "")))
            if p.op.startswith("CS") and p.edition == "0.1":
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, ["U0103"]), p.name)
            elif p.variant.endswith("-twin"):
                self.assertEqual(p.cls, OPEN, p.variant)
                self.assertNotIn("secret", p.files["src/app/main.una"], p.name)
            else:
                self.assertEqual(p.cls, MUST_REJECT, p.variant)
                self.assertTrue(p.expect, p.variant)
        self.assertEqual({v for op, v in seen if op == "PG1"},
                         {"box-return", "box-print", "option-box-return", "sink-as-secret"})

    def test_composed2_classes(self):
        seen = set()
        for p in every_program(60):
            if p.op not in ("TC1", "TC2", "TC3", "CP1", "GS1"):
                continue
            seen.add(p.op)
            if p.op != "GS1" and p.edition == "0.1":
                self.assertEqual((p.cls, p.expect), (MUST_REJECT, ["U0103"]), p.name)
            elif p.op == "TC3" or p.variant.endswith("-twin") or p.variant.startswith("to-public"):
                self.assertEqual(p.cls, OPEN, p.variant)
            else:
                self.assertEqual(p.cls, MUST_REJECT, p.variant)
                self.assertTrue(p.expect, p.variant)
        self.assertEqual(seen, {"TC1", "TC2", "TC3", "CP1", "GS1"})
        for p in every_program(60):
            if p.op == "TC2" and p.variant.startswith("to-public"):
                self.assertEqual(p.variant, "to-public-after-block")

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


def run_doc(status, output=(), failure=None, diagnostics=(), trace=()):
    return json.dumps({"schema": "una.run/1", "tier": 0, "status": status, "output": list(output),
                       "failure": failure, "diagnostics": list(diagnostics),
                       "trace": list(trace)})


def diag(code, severity="error"):
    return {"severity": severity, "code": code, "message": "m",
            "primary": {"file": "src/a.una", "start": {"line": 1, "col": 1}}}


EVENT = {"seq": 0, "instance": "console", "capability": "Console", "op": "print",
         "args": {"message": "hi"}, "outcome": {"ok": {"value": "unit"}}}


class Oracle(unittest.TestCase):
    def test_values_are_fixed_capped_and_never_the_original(self):
        files = {"src/a.una": 'let s: secret Int = 4242\nlet t = "abc"\n'}
        ints = oracle.values("Int", "4242", files)
        self.assertEqual(ints, oracle.values("Int", "4242", files))
        self.assertEqual(len(ints), oracle.VALUES_PER_POSITION)
        self.assertNotIn("4242", ints)
        self.assertEqual(oracle.values("Bool", "true", files), ["false"])
        texts = oracle.values("Text", '"abc"', files)
        self.assertIn('""', texts)
        self.assertNotIn('"abc"', texts)
        for t in texts:
            self.assertNotIn("{", t)
            self.assertNotIn("\\", t)
        with self.assertRaises(ValueError):
            oracle.values("Float", "1.0", files)

    def test_substitute_replaces_only_the_literal_at_its_place(self):
        files = {"src/a.una": "x\n  let s: secret Int = 5 + 5\n"}
        pos = {"file": "src/a.una", "line": 2, "col": 23, "literal": "5"}
        out = oracle.substitute(files, pos, "-1")
        self.assertEqual(out["src/a.una"], "x\n  let s: secret Int = -1 + 5\n")
        self.assertEqual(files["src/a.una"], "x\n  let s: secret Int = 5 + 5\n")
        with self.assertRaises(ValueError):
            oracle.substitute(files, dict(pos, col=3), "0")

    def test_a_policy_to_public_sets_the_program_aside(self):
        to_public = {"src/k.una": "pub policy p\n  from secret Int\n  to Int\n  via v\n"}
        to_internal = {"src/k.una": "pub policy p\n  from secret Int\n  to internal Int\n"}
        self.assertTrue(oracle.releases_to_public(to_public))
        self.assertFalse(oracle.releases_to_public(to_internal))
        self.assertFalse(oracle.releases_to_public({"src/a.una": "module a\n"}))

    def test_the_projection_drops_only_the_outcome(self):
        doc = json.loads(run_doc("ok", ["a"], trace=[EVENT]))
        view = oracle.project(doc)
        self.assertEqual(view["trace"], [{k: v for k, v in EVENT.items() if k != "outcome"}])
        other = dict(EVENT, outcome={"fail": {"variant": "X", "value": None}})
        self.assertEqual(view, oracle.project(json.loads(run_doc("ok", ["a"], trace=[other]))))
        moved = dict(EVENT, args={"message": "<redacted:secret>"})
        self.assertNotEqual(view, oracle.project(json.loads(run_doc("ok", ["a"], trace=[moved]))))

    def test_observe_fails_closed(self):
        def end(code, text, timed_out=False):
            return oracle.observe(Run(code, text, "", timed_out, 0.1))[0]
        self.assertEqual(end(0, run_doc("ok", ["a"])), "complete")
        self.assertEqual(end(1, run_doc("failed", failure={"variant": "Halt", "fields": {}},
                                        diagnostics=[diag("U6001")])), "complete")
        self.assertEqual(end(1, run_doc("aborted", diagnostics=[diag("U6003")])), "abort")
        self.assertEqual(end(None, "", timed_out=True), "abort")
        self.assertEqual(end(1, run_doc("rejected", diagnostics=[diag("U4001")])), "rejected")
        # Anything else is an error: an internal error, another abort, an ok run with
        # diagnostics, an exit code that disagrees, unreadable output, a signal.
        self.assertEqual(end(3, run_doc("aborted", diagnostics=[diag("U8001")])), "error")
        self.assertEqual(end(1, run_doc("aborted", diagnostics=[diag("U6003"), diag("U8001")])),
                         "error")
        self.assertEqual(end(0, run_doc("ok", diagnostics=[diag("U0001", "warning")])), "error")
        self.assertEqual(end(1, run_doc("ok")), "error")
        self.assertEqual(end(1, run_doc("failed")), "error")
        self.assertEqual(end(0, "not json"), "error")
        self.assertEqual(end(-6, ""), "error")
        self.assertEqual(end(0, json.dumps({"schema": "una.check/1"})), "error")

    def test_judge(self):
        a = ("complete", {"output": ["1"]})
        b = ("complete", {"output": ["2"]})
        abort = ("abort", ["U6003"])
        self.assertEqual(oracle.judge(a, a), "same")
        self.assertEqual(oracle.judge(a, b), "leak")
        self.assertEqual(oracle.judge(a, abort), "termination")
        self.assertEqual(oracle.judge(abort, a), "termination")
        self.assertEqual(oracle.judge(abort, ("abort", ["timeout"])), "same")
        self.assertEqual(oracle.judge(a, ("error", "x")), "error")
        self.assertEqual(oracle.judge(a, ("rejected", [])), "error")
        # ok against failed is a difference in what is public, so a leak.
        ok = oracle.project(json.loads(run_doc("ok")))
        failed = oracle.project(json.loads(run_doc("failed", failure={"variant": "H",
                                                                       "fields": {}})))
        self.assertEqual(oracle.judge(("complete", ok), ("complete", failed)), "leak")


class Gate(unittest.TestCase):
    def test_the_fixed_run_is_fixed_and_not_empty(self):
        a, b = gate.programs(), gate.programs()
        self.assertEqual([p.digest() for p in a], [p.digest() for p in b])
        self.assertEqual(len({p.name for p in a}), len(a))
        self.assertTrue(any(p.vary for p in a))
        self.assertTrue(any(p.cls == MUST_REJECT for p in a))
        self.assertGreater(gate.expected_keep(a), 0)

    def test_the_canaries_are_what_they_claim(self):
        rej = gate.canary_reject()
        self.assertEqual(rej.cls, MUST_REJECT)
        self.assertNotIn("secret", "".join(rej.files.values()))
        orc = gate.canary_oracle()
        self.assertEqual(len(orc.vary), 1)
        pos = orc.vary[0]
        self.assertEqual(pos["label"], "secret")
        line = orc.files[pos["file"]].split("\n")[pos["line"] - 1]
        # The literal is public in Una: no label on its binding.
        self.assertEqual(line.strip(), "let x: Int = 5")
        self.assertTrue(oracle.values(pos["type"], pos["literal"], orc.files))

    def test_shards_keep_row_order(self):
        rows = [{"n": i} for i in range(10)]

        def fn(part, buf, k):
            for r in part:
                buf.write(json.dumps(r) + "\n")
            return {("n",): len(part)}
        tally, lines = gate._sharded(fn, rows, 4)
        self.assertEqual(lines, rows)
        self.assertEqual(tally[("n",)], 10)


if __name__ == "__main__":
    unittest.main()
