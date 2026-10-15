"""Tests of the hunt's generators and drivers that need no compiler.

    cd scripts/security-hunt && python3 -m unittest discover -s tests
"""

import io
import json
import os
import random
import re
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from hunt import drivers, gate, native, oracle, sandbox  # noqa: E402
from hunt.families import OPERATORS, editions_of  # noqa: E402
from hunt.program import CLASSES, MUST_REJECT, OPEN, RESIDUAL, derive_seed, rng_for  # noqa: E402
from hunt.refactor import _blocks, r6_rename, r9_reorder  # noqa: E402
from hunt.sandbox import Run  # noqa: E402

def every_program(n=40):
    for op in sorted(OPERATORS):
        for i in range(n):
            for ed in editions_of(op):
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
            for ed in editions_of(op):
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
            if p.edition == "0.1" and "U0103" not in p.expect:
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
        # From the review of #547: a public ending is must-reject for every carrier but the
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
        # #548: a public ending of a secret is must-reject; a secret ending, and the structural
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
        # #548: only an acceptance, or a rejection by an obligation, with the solver.
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
        # From the review of #539: a predicate's line is expected too, so dropping it is `unknown`.
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
        # From the review of #540: a predicate's line saying anything else is `unknown`.
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


class Concurrency(unittest.TestCase):
    """C6, the oracle's programs of family C (#807)."""

    def test_every_form_builds_a_runnable_varied_program_in_0_2_only(self):
        forms = set()
        for i in range(60):
            p02 = OPERATORS["C6"](derive_seed(1, "C6", i), "0.2")
            p01 = OPERATORS["C6"](derive_seed(1, "C6", i), "0.1")
            forms.add(p02.variant)
            self.assertIn("together", p02.files["src/app/main.una"])
            self.assertNotIn("⟦", p02.files["src/app/main.una"])
            if p02.variant == "attempt-sibling":
                # The checks reject a secret-decided failure even inside `attempt` (measured).
                self.assertEqual((p02.cls, p02.expect, p02.vary), (MUST_REJECT, ["U4002"], []))
                continue
            self.assertEqual(p02.cls, OPEN)
            self.assertEqual(len(p02.vary), 1, p02.name)
            self.assertEqual(p02.vary[0]["label"], "secret")
            self.assertIn('entry = "app.main"', p02.files["una.toml"])
            # Edition 0.1 reserves `together`: must-reject, with nothing to vary or run.
            # (A form with a `fail` block also expects U0101, 0.1's own refusal of that form.)
            fail_form = "fail Bad" in p01.files["src/app/main.una"]
            self.assertEqual((p01.cls, p01.expect, p01.vary),
                             (MUST_REJECT, ["U0103", "U0101"] if fail_form else ["U0103"], []))
        self.assertEqual(forms, {"for-elements", "branch-value", "attempt-sibling", "secret-sink",
                                 "under-public-pc", "secret-fan-out", "shape-length",
                                 "public-fan-out", "element-fail"})


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


def done(views, live=None):
    """A complete end with these observer views."""
    return ("complete", {"views": views, "live": live or {}})


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

    TABLE = {k: {v} for k, v in oracle.HOST_ACCEPTS.items()}

    def core(self, *caps):
        """A Core export with user capabilities: (name, op, accepts label object or None)."""
        decls = [{"kind": "cap", "name": n, "ops": [{"name": o, "accepts": a}]} for n, o, a in caps]
        return {"modules": [{"path": "app.main", "decls": decls}]}

    def views(self, trace, observer="public", table=None, output=("done",)):
        doc = json.loads(run_doc("ok", list(output), trace=trace))
        return oracle.project(doc, observer, table or self.TABLE)

    def test_the_projection_drops_seq_and_outcome_and_keeps_order(self):
        view = self.views([EVENT, dict(EVENT, seq=1, args={"message": "b"})])
        self.assertEqual(view["trace"], [{k: v for k, v in EVENT.items()
                                          if k not in ("seq", "outcome")},
                                         {k: v for k, v in EVENT.items()
                                          if k not in ("seq", "outcome", "args")}
                                         | {"args": {"message": "b"}}])
        other = dict(EVENT, seq=7, outcome={"fail": {"variant": "X", "value": None}})
        self.assertEqual(self.views([EVENT]), self.views([other]))
        moved = dict(EVENT, args={"message": "<redacted:secret>"})
        self.assertNotEqual(self.views([EVENT]), self.views([moved]))

    def h3(self, accepts):
        """The trace shapes of H3: a user operation called only when the secret is true."""
        table = oracle.accepts_table(self.core(("Vault", "note", accepts)))
        note = {"seq": 0, "instance": "with:quiet#1", "capability": "Vault", "op": "note",
                "args": {"x": "1"}, "outcome": {"kind": "ok", "value": "unit"}}
        printed = dict(EVENT, seq=1)
        with_note, without = [note, printed], [dict(printed, seq=0)]
        return table, with_note, without

    def test_an_event_whose_operation_accepts_secret_is_hidden_below_it(self):
        table, a, b = self.h3({"level": "secret", "categories": [], "symbols": []})
        for observer in ("public", "internal"):
            self.assertEqual(self.views(a, observer, table), self.views(b, observer, table))

    def test_an_event_whose_operation_accepts_internal_is_seen_at_internal(self):
        table, a, b = self.h3({"level": "internal", "categories": [], "symbols": []})
        self.assertEqual(self.views(a, "public", table), self.views(b, "public", table))
        self.assertNotEqual(self.views(a, "internal", table), self.views(b, "internal", table))
        base = done({o: self.views(a, o, table) for o in ("public", "internal")})
        other = done({o: self.views(b, o, table) for o in ("public", "internal")})
        self.assertEqual(oracle.judge(base, other), "leak")
        self.assertEqual(oracle.leaking_observers(base, other), ["internal"])

    def test_a_public_print_under_the_secret_stays_a_leak(self):
        table, _, _ = self.h3({"level": "secret", "categories": [], "symbols": []})
        a = self.views([EVENT], "public", table, output=["yes"])
        b = self.views([], "public", table, output=[])
        self.assertEqual(oracle.judge(done({"public": a}), done({"public": b})), "leak")

    def test_an_unclassified_accepts_stays_visible(self):
        # No clause, an unknown level, a public label with categories, a missing level: each is
        # seen by every observer, so a miss gives a false finding, never a hidden leak.
        for acc in (None, {"level": "weird"}, {"level": "public", "categories": ["PII"]}, {},
                    {"level": ""}, {"level": "Secret"}):
            table, a, b = self.h3(acc)
            self.assertNotEqual(self.views(a, "public", table), self.views(b, "public", table),
                                acc)
        # Two declarations of one operation: seen when either is.
        table = oracle.accepts_table(self.core(("Vault", "note", {"level": "secret"}),
                                               ("Vault", "note", None)))
        self.assertTrue(oracle.sees("public", table[("Vault", "note")]))

    def test_an_event_of_an_unknown_operation_is_an_error(self):
        doc = run_doc("ok", ["a"], trace=[dict(EVENT, capability="Ghost", op="boo")])
        end = oracle.observe(Run(0, doc, "", False, 0.1), ["public"], self.TABLE, "secret")
        self.assertEqual(end[0], "error")
        self.assertIn("Ghost.boo", end[1])

    def test_keys_hmac_shows_only_its_key_name(self):
        ev = {"seq": 0, "instance": "host:Keys", "capability": "Keys", "op": "hmac",
              "args": {"key_name": "\"pepper\"", "data": "<redacted:secret>"},
              "outcome": {"kind": "ok", "value": "hashed:00"}}
        view = self.views([ev])
        self.assertEqual(view["trace"][0]["args"], {"key_name": "\"pepper\""})
        self.assertEqual(view, self.views([dict(ev, args={"key_name": "\"pepper\"",
                                                          "data": "\"x\""})]))

    def test_the_host_table_is_the_preludes(self):
        # Pinned to the declarations of crates/una-check/src/std: every host capability's
        # operations, with their `accepts` (none written: `public`, spec/07 §7).
        std = os.path.join(os.path.dirname(__file__), "..", "..", "..", "crates", "una-check",
                           "src", "std")
        found = {}
        for name in ("prelude.una", "prelude_0_2.una"):
            cap = op = None
            with open(os.path.join(std, name), encoding="utf-8") as f:
                lines = f.read().splitlines(keepends=True)
            for line in lines:
                m = re.match(r"pub capability (\w+)", line)
                if m:
                    cap, op = m.group(1), None
                    continue
                if cap and re.match(r"\S", line):
                    cap = op = None
                    continue
                m = re.match(r"  op (\w+)\(", line)
                if cap and m:
                    op = m.group(1)
                    found[(cap, op)] = "public"
                    continue
                m = re.match(r"    accepts (\w+)", line)
                if cap and op and m:
                    found[(cap, op)] = m.group(1)
        self.assertEqual(found, oracle.HOST_ACCEPTS)

    def test_observers_are_those_below_the_varied_label(self):
        self.assertEqual(oracle.observers_for([{"label": "secret[PII]"}]), ["public", "internal"])
        self.assertEqual(oracle.observers_for([{"label": "internal"}]), ["public"])
        self.assertEqual(oracle.observers_for([{"label": "odd"}]), ["public", "internal"])
        self.assertEqual(oracle.observers_for([{"label": "secret"}, {"label": "internal"}]),
                         ["public"])

    def test_observe_fails_closed(self):
        def end(code, text, timed_out=False):
            return oracle.observe(Run(code, text, "", timed_out, 0.1), ["public"], self.TABLE,
                                  "secret")[0]
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
        # A failed run with a diagnostic other than an error is an error too.
        self.assertEqual(end(1, run_doc("failed", failure={"variant": "Halt", "fields": {}},
                                        diagnostics=[diag("U6001"), diag("U0001", "warning")])),
                         "error")
        self.assertEqual(end(0, "not json"), "error")
        self.assertEqual(end(-6, ""), "error")
        self.assertEqual(end(0, json.dumps({"schema": "una.check/1"})), "error")

    def test_judge(self):
        a, b = done({"public": {"output": ["1"]}}), done({"public": {"output": ["2"]}})
        abort = ("abort", ["U6003"])
        self.assertEqual(oracle.judge(a, a), "same")
        self.assertEqual(oracle.judge(a, b), "leak")
        self.assertEqual(oracle.judge(a, abort), "termination")
        self.assertEqual(oracle.judge(None, a), "first")
        self.assertEqual(oracle.judge(None, abort), "abort")
        self.assertEqual(oracle.judge(a, ("error", "x")), "error")
        self.assertEqual(oracle.judge(a, ("rejected", [])), "error")
        # ok against failed is a difference in what is public, so a leak.
        ok = self.views([], output=[])
        failed = oracle.project(json.loads(run_doc("failed", failure={"variant": "H",
                                                                       "fields": {}})),
                                "public", self.TABLE)
        self.assertEqual(oracle.judge(done({"public": ok}), done({"public": failed})), "leak")

    def test_a_redacted_argument_at_a_visible_sink_is_a_finding_by_itself(self):
        table = oracle.accepts_table(self.core(("Vault", "note", None)))
        ev = {"seq": 0, "instance": "with:quiet#1", "capability": "Vault", "op": "note",
              "args": {"x": "<redacted:secret>"}, "outcome": {"kind": "ok", "value": "unit"}}
        self.assertEqual(len(self.views([ev], "public", table)["violations"]), 1)
        # Inside a rendered value, too.
        nested = dict(ev, args={"x": "Card(n: <redacted:confidential>)"})
        self.assertEqual(len(self.views([nested], "internal", table)["violations"]), 1)
        # An `internal` argument at an operation accepting `internal`, seen at `internal`: fine.
        table = oracle.accepts_table(self.core(("Vault", "note", {"level": "internal"})))
        fine = dict(ev, args={"x": "<redacted:internal>"})
        self.assertEqual(self.views([fine], "internal", table)["violations"], [])
        self.assertEqual(self.views([fine], "public", table)["trace"], [])
        # `Keys.hmac`'s data is not a sink, and is dropped first.
        hmac = {"seq": 0, "instance": "host:Keys", "capability": "Keys", "op": "hmac",
                "args": {"key_name": "\"k\"", "data": "<redacted:secret>"},
                "outcome": {"kind": "ok", "value": "hashed:00"}}
        self.assertEqual(self.views([hmac])["violations"], [])
        self.assertEqual(len(self.views([dict(hmac, args={"key_name": "<redacted:secret>"})])
                             ["violations"]), 1)

    def drive(self, ends, live=None):
        """Runs `oracle.oracle` on one program with one Int literal, whose base and variants end as
        `ends` says (in order), with no compiler: run_twice and export_core are replaced."""
        prog = gate.canary_oracle()
        calls = iter(ends)
        saved = oracle.run_twice_kept, oracle.export_core
        oracle.export_core = lambda *a: ({"complete": True, "modules": []}, None)
        # No run to build natively: the native sample is not exercised here (class Native is).
        oracle.run_twice_kept = lambda *a: (next(calls), None)
        try:
            buf = io.StringIO()
            with tempfile.TemporaryDirectory() as tmp:
                oracle.oracle([dict(prog.manifest(), digest=prog.digest())], "una", tmp, 1, buf,
                              lambda row: prog)
        finally:
            oracle.run_twice_kept, oracle.export_core = saved
        return json.loads(buf.getvalue())

    def test_two_complete_variants_are_compared_whatever_the_base_did(self):
        abort = ("abort", ["U6003"])
        ends = [abort, done({"public": {"output": ["A"]}}), done({"public": {"output": ["B"]}}),
                done({"public": {"output": ["A"]}}), abort, abort]
        row = self.drive(ends)
        self.assertTrue(row["finding"])
        self.assertEqual([v["verdict"] for v in row["variants"]],
                         ["termination", "leak", "same", "termination", "termination"])
        # Only complete-against-complete comparisons count.
        self.assertEqual(row["compared"], 2)
        self.assertEqual(len(row["termination"]), 3)

    def test_aborts_alone_compare_nothing(self):
        abort = ("abort", ["U6003"])
        row = self.drive([abort] * 6)
        self.assertEqual(row["compared"], 0)
        self.assertFalse(row["finding"])
        self.assertEqual({v["verdict"] for v in row["variants"]}, {"abort"})

    def test_liveness_is_a_difference_at_the_literals_level(self):
        same = {"public": {"output": ["x"]}}
        ends = [done(same, live={"o": 1})] + [done(same, live={"o": 2})] * 5
        row = self.drive(ends)
        self.assertFalse(row["finding"])
        self.assertTrue(row["live"])
        row = self.drive([done(same, live={"o": 1})] * 6)
        self.assertFalse(row["live"])

    def test_a_redacted_sink_in_any_complete_end_is_a_finding(self):
        bad = {"public": {"output": ["x"], "violations": ["Vault.note(x) is <redacted:secret>"]}}
        row = self.drive([done(bad)] * 6)
        self.assertTrue(row["finding"])
        self.assertEqual(row["violations"], ["Vault.note(x) is <redacted:secret>"])

    def test_two_runs_of_a_variant_must_match_byte_for_byte(self):
        doc = run_doc("ok", ["a"])
        runs = iter([Run(0, doc, "", False, 0.1), Run(0, doc.replace('"a"', '"b"'), "", False, 0.2)])
        dirs = []
        saved = oracle.sandbox.run, oracle.sandbox.fresh_dir
        oracle.sandbox.run = lambda argv, cwd, writable, timeout, env_extra=None: next(runs)

        def fresh(root, prefix):
            dirs.append(prefix)
            return tempfile.mkdtemp(prefix=prefix, dir=tmp)
        oracle.sandbox.fresh_dir = fresh
        try:
            with tempfile.TemporaryDirectory() as tmp:
                end = oracle.run_twice("una", {"una.toml": "x\n"}, "p", tmp, 1, ["public"],
                                       self.TABLE, "secret")
        finally:
            oracle.sandbox.run, oracle.sandbox.fresh_dir = saved
        self.assertEqual(end, ("error", "nondeterministic: two runs of one variant differ"))
        self.assertEqual(len(dirs), 2)
        self.assertNotEqual(dirs[0], dirs[1])
        # Elapsed time is not part of the document, so it does not count.
        self.assertEqual(oracle.raw(Run(0, doc, "", False, 0.1)), oracle.raw(Run(0, doc, "", False, 9)))
        self.assertEqual(oracle.RUN_VARYING_FIELDS, ())


class Gate(unittest.TestCase):
    def test_the_fixed_run_is_fixed_and_not_empty(self):
        a, b = gate.programs(), gate.programs()
        self.assertEqual([p.digest() for p in a], [p.digest() for p in b])
        self.assertEqual(len({p.name for p in a}), len(a))
        self.assertTrue(any(p.vary for p in a))
        self.assertTrue(any(p.cls == MUST_REJECT for p in a))
        # The pinned sizes are the generators' (a change of either fails here first).
        self.assertEqual(gate.PINNED, {
            "programs": len(a),
            "reject": sum(1 for p in a if p.cls != "must-keep"),
            "keep": gate.expected_keep(a),
            "oracle": sum(1 for p in a if p.vary),
            "differ": len(a),
        })
        # The pinned unknowns are programs of the fixed run, and the planted Core document is in
        # the repository.
        names = {p.name for p in a}
        self.assertTrue(set(gate.PINNED_UNKNOWN) <= names, set(gate.PINNED_UNKNOWN) - names)
        repo = os.path.join(os.path.dirname(__file__), "..", "..", "..")
        for name in [*gate.DIFFER_CANARIES, gate.DIFFER_CONTROL]:
            self.assertTrue(os.path.isfile(os.path.join(repo, gate.REPLAY, name)), name)

    def test_reading_counts_a_program_compared_only_when_its_own_function_is_accepted(self):
        def line(decl, outcome, note=""):
            return {"decl": decl, "outcome": outcome, "note": note}
        fallback = {"recheck": [line("app.main.main", "not re-checked", "a type"),
                                line("prelude.p", "not re-checked", "a type")]}
        self.assertEqual(gate.reading(fallback), ("fallback", "a type"))
        compared = {"recheck": [line("app.main.main", "accepted"),
                                line("app.main.f", "not re-checked", "a handler")]}
        self.assertEqual(gate.reading(compared), ("compared", None))
        conditional = {"recheck": [line("app.main.main", "accepted, conditional on A3")]}
        self.assertEqual(gate.reading(conditional), ("compared", None))
        # Only the prelude's declarations accepted: nothing of the program was compared.
        unread = {"recheck": [line("prelude.p", "accepted"),
                              line("app.main.main", "not re-checked", "a handler"),
                              line("app.main.g", "not re-checked", "a via")]}
        self.assertEqual(gate.reading(unread), ("unread", None))
        self.assertEqual(gate.reading({"recheck": []}), ("unread", None))

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

    def test_caught_needs_exactly_the_planted_fault(self):
        def row(observers=(), violations=(), **kw):
            return dict({"finding": bool(observers or violations), "error": None,
                         "variants": [{"observers": list(observers)}],
                         "violations": list(violations)}, **kw)
        self.assertTrue(gate.caught("oracle", row(["public"])))
        self.assertTrue(gate.caught("internal", row(["internal"])))
        self.assertFalse(gate.caught("internal", row(["internal", "public"])))
        self.assertFalse(gate.caught("internal", row()))
        self.assertTrue(gate.caught("redacted", row(violations=["v"])))
        self.assertFalse(gate.caught("redacted", row(["public"], ["v"])))
        self.assertFalse(gate.caught("oracle", row(["public"], error="boom")))
        self.assertFalse(gate.caught("oracle", row(["public"], set_aside="x")))

    def test_the_canary_programs_carry_their_planted_shapes(self):
        internal, redacted = gate.canary_internal(), gate.canary_redacted()
        for p in (internal, redacted):
            text = p.files["src/app/main.una"]
            self.assertIn("accepts secret", text)
            self.assertEqual([v["label"] for v in p.vary], ["secret"])
        self.assertIn("if flag", internal.files["src/app/main.una"])
        self.assertIn("@vault.note(n)", redacted.files["src/app/main.una"])
        self.assertEqual(gate.planted("internal")({})[("Vault", "note")], {"internal"})

    def test_the_gate_refuses_a_directory_that_is_not_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            open(os.path.join(tmp, "old"), "w").close()
            with self.assertRaises(SystemExit):
                gate.run("una", tmp, "0" * 40, "una-core-check", "z3")

    def test_shards_keep_row_order(self):
        rows = [{"n": i} for i in range(10)]

        def fn(part, buf, k):
            for r in part:
                buf.write(json.dumps(r) + "\n")
            return {("n",): len(part)}
        tally, lines = gate._sharded(fn, rows, 4)
        self.assertEqual(lines, rows)
        self.assertEqual(tally[("n",)], 10)


class GeneratorErrors(unittest.TestCase):
    """A lexical or syntax code a generated program does not expect is the generator's error, and
    a must-reject program counts only when rejected by one of its expected codes (#807)."""

    def row(self, codes, expect=(), cls=MUST_REJECT, verdict="rejected"):
        got = {c for c in codes}
        return {"name": "p", "class": cls, "verdict": verdict, "expect": list(expect),
                "codes": [[c, "src/app/main.una", 1, 1] for c in codes],
                "expect_seen": bool(got & set(expect)) if expect else None}

    def test_an_unexpected_syntax_code_is_an_error_whatever_the_class(self):
        self.assertTrue(gate.generator_errors([self.row(["U0001"], ["U4001"])]))
        self.assertTrue(gate.generator_errors([self.row(["U0001"], cls=OPEN)]))
        # An expected one (the edition gate of `together`) is not.
        self.assertEqual(gate.generator_errors([self.row(["U0103"], ["U0103"])]), [])

    def test_a_must_reject_program_needs_its_expected_code(self):
        self.assertEqual(gate.generator_errors([self.row(["U4002"], ["U4002"])]), [])
        self.assertTrue(gate.generator_errors([self.row(["U4003"], ["U4002"])]))
        # A must-reject program with no expected code would count any rejection.
        self.assertTrue(gate.generator_errors([self.row(["U4002"], [])]))

    def test_a_refactor_that_breaks_parsing_is_an_error_of_the_pair(self):
        def pair(base, twin):
            return {"name": "p", "refactor": "R6", "expect": [],
                    "base": {"codes": [[c, "x", 1, 1] for c in base]},
                    "twin": {"codes": [[c, "x", 1, 1] for c in twin]}}
        self.assertEqual(gate.keep_generator_errors([pair(["U4001"], ["U4001"])]), [])
        self.assertTrue(gate.keep_generator_errors([pair(["U4001"], ["U0001"])]))


class Edition03(unittest.TestCase):
    """The operators of edition 0.3's rules (#813): boundaries (ADR 0093) and conditions on
    operations (ADR 0092)."""

    NEW = ("B1", "B2", "Q1", "Q2", "Q3")

    def test_the_new_operators_run_0_2_as_the_probe_and_0_3(self):
        for op in self.NEW:
            self.assertEqual(editions_of(op), ("0.2", "0.3"), op)
        self.assertEqual(editions_of("T1"), ("0.1", "0.2"))
        progs = gate.programs()
        self.assertFalse(any(p.edition == "0.3" for p in progs if p.op not in self.NEW))
        self.assertFalse(any(p.edition == "0.1" for p in progs if p.op in self.NEW))
        for op in self.NEW:
            self.assertEqual(sum(1 for p in progs if p.op == op), 2 * gate.COUNT, op)

    def test_the_probe_is_a_check_with_its_gate_code(self):
        for op in self.NEW:
            for i in range(40):
                p = OPERATORS[op](derive_seed(1, op, i), "0.2")
                self.assertEqual(p.cls, MUST_REJECT, p.name)
                self.assertTrue(set(p.expect) <= {"U0101", "U7002"} and p.expect, p.name)
                self.assertEqual(p.vary, [], p.name)
                b = OPERATORS[op](derive_seed(1, op, i), "0.3")
                self.assertIn('edition = "0.3"', b.files["una.toml"])

    def test_the_forms_and_their_classes(self):
        seen = {}
        for i in range(200):
            for op in self.NEW:
                p = OPERATORS[op](derive_seed(5, op, i), "0.3")
                seen.setdefault(op, {})[p.variant.rsplit("-", 1)[0] if op in ("B1", "Q1", "Q2")
                                       else p.variant] = (p.cls, tuple(p.expect))
        self.assertEqual(len(seen["B1"]), 7)
        self.assertEqual(len(seen["Q1"]), 8)
        self.assertEqual(seen["Q1"]["ceiling"], (MUST_REJECT, ("U7002",)))
        self.assertTrue(all(cls == MUST_REJECT and e for cls, e in seen["B1"].values()))
        self.assertEqual(seen["B1"]["release"][1], ("U3024",))
        # Q2's rows are residual: the refuted ones expect their code, the proven ones none.
        self.assertEqual(seen["Q2"]["unguarded"], (RESIDUAL, ("U5001",)))
        self.assertEqual(seen["Q2"]["lock-weakened"], (RESIDUAL, ("U3023",)))
        self.assertEqual(seen["Q2"]["guarded"], (RESIDUAL, ()))
        self.assertEqual(set(seen["B2"]), {"grant-handler", "narrowed", "function-value",
                                           "under-secret-pc"})
        self.assertEqual(set(seen["Q3"]), {"guarded-live", "guarded-dry", "secret-condition"})
        for op in ("B2", "Q3"):
            self.assertTrue(all(cls == OPEN for cls, _ in seen[op].values()))

    def test_boundaries_and_contracts_tables(self):
        p = next(OPERATORS["B1"](derive_seed(1, "B1", i), "0.3") for i in range(200)
                 if OPERATORS["B1"](derive_seed(1, "B1", i), "0.3").variant.startswith("ceiling"))
        self.assertIn('[boundaries]\n"app.edge" = ["', p.files["una.toml"])
        q = next(OPERATORS["Q1"](derive_seed(1, "Q1", i), "0.3") for i in range(200)
                 if OPERATORS["Q1"](derive_seed(1, "Q1", i), "0.3").variant.startswith("lock"))
        self.assertIn('[contracts]\n"app.main.update" = ["@log: Log[write where not ',
                      q.files["una.toml"])
        # No table, no change: the files of the older operators are as before.
        self.assertNotIn("[boundaries]", OPERATORS["T1"](1, "0.2").files["una.toml"])

    def test_residual_rows_with_an_expected_code(self):
        V = drivers.Verdict
        u5001 = V("rejected", [["U5001", "a", 1, 1]], 0.1)
        r = drivers.judge_residual("accepted", u5001, "unguarded-dry", ["U5001"])
        self.assertEqual((r["full_kind"], r["invalid"], r["missed"]), ("rejected", False, False))
        r = drivers.judge_residual("accepted", V("accepted", [], 0.1), "unguarded-dry", ["U5001"])
        self.assertEqual((r["full_kind"], r["invalid"], r["missed"]), ("accepted", False, True))
        # The expected code may be outside the obligations' (U3023 through `contract_lock`).
        u3023 = V("rejected", [["U3023", "a", 1, 1]], 0.1)
        r = drivers.judge_residual("accepted", u3023, "lock-weakened-dry", ["U3023"])
        self.assertEqual((r["full_kind"], r["invalid"]), ("rejected", False))
        # A rejection by another code, even an obligation's, is not the expected one.
        u5002 = V("rejected", [["U5002", "a", 1, 1]], 0.1)
        r = drivers.judge_residual("accepted", u5002, "unguarded-dry", ["U5001"])
        self.assertTrue(r["invalid"])
        self.assertFalse(r["missed"])
        r = drivers.judge_residual("accepted", u3023, "unguarded-dry", ["U5001"])
        self.assertTrue(r["invalid"])
        # No solver is invalid, never missed or clean.
        r = drivers.judge_residual("accepted", None, "unguarded-dry", ["U5001"])
        self.assertEqual((r["invalid"], r["missed"]), (True, False))
        # Without `expect` the row keeps its old, descriptive meaning.
        r = drivers.judge_residual("accepted", V("accepted", [], 0.1), "guarded-dry")
        self.assertEqual((r["invalid"], r["missed"]), (False, False))

    def test_the_edition_0_3_canaries(self):
        rej = gate.canary_reject_boundary()
        self.assertEqual((rej.cls, rej.expect, rej.edition), (MUST_REJECT, ["U3021"], "0.3"))
        self.assertIn('"app.edge" = ["Store[read]"]', rej.files["una.toml"])
        self.assertIn("with @store = @store[read]", rej.files["src/app/main.una"])
        orc = gate.canary_oracle_condition()
        self.assertEqual([v["label"] for v in orc.vary], ["secret"])
        pos = orc.vary[0]
        line = orc.files[pos["file"]].split("\n")[pos["line"] - 1]
        self.assertEqual(line.strip(), "let x: Int = 5")
        self.assertIn("work(1, dry: x > 3)", orc.files[pos["file"]])
        self.assertIn("journal where not dry", orc.files[pos["file"]])
        self.assertTrue(gate.caught("oracle-condition", {
            "finding": True, "error": None, "variants": [{"observers": ["public"]}],
            "violations": []}))


class Native(unittest.TestCase):
    """The native sample's comparison (`native.compare`, #792), on run documents and traces in
    the format the engines write: printed lines quoted by `una_core::quote`, the root's `task`
    null, outcomes `{kind, value}`."""

    @staticmethod
    def print_event(seq, line, task=None):
        return {"seq": seq, "task": task, "instance": "host:Console", "capability": "Console",
                "op": "print", "args": {"line": native.quote(line)},
                "outcome": {"kind": "ok", "value": "unit"}}

    def doc(self, status="ok", output=("hi",), trace=None, diagnostics=()):
        return {"schema": "una.run/1", "status": status, "output": list(output),
                "trace": trace if trace is not None else [self.print_event(1, "hi")],
                "diagnostics": list(diagnostics)}

    def runs(self, doc, code=0, stdout="hi\n", stderr="", ntrace="same", ncode=None):
        interp = Run(code, json.dumps(doc), "", False, 0.1)
        nrun = Run(code if ncode is None else ncode, stdout, stderr, False, 0.1)
        return interp, nrun, doc["trace"] if ntrace == "same" else ntrace

    def compare(self, doc, **kw):
        interp, nrun, trace = self.runs(doc, **kw)
        return native.compare(interp, doc, nrun, trace)

    def test_quote_is_una_cores(self):
        self.assertEqual(native.quote('a"b\\c{d}\ne\tf\x01g\x7fh\u202ai'),
                         '"a\\"b\\\\c\\{d\\}\\ne\\tf\\u{1}g\\u{7f}h\\u{202a}i"')

    def test_the_same_run_agrees(self):
        self.assertEqual(self.compare(self.doc()), [])

    def test_each_difference_is_reported(self):
        doc = self.doc()
        self.assertIn("exit code", self.compare(doc, ncode=1)[0])
        self.assertEqual(self.compare(doc, stdout="ho\n"), ["standard output differs"])
        self.assertEqual(self.compare(doc, ntrace=[self.print_event(1, "ho")]),
                         ["the trace differs"])

    def test_a_redacted_label_is_a_divergence(self):
        # A trace that differs only in the label of a redacted argument is a divergence, never
        # normalized away.
        ev = {"seq": 1, "task": None, "instance": "with:quiet#1", "capability": "Vault",
              "op": "note", "args": {"x": "<redacted:secret>"},
              "outcome": {"kind": "ok", "value": "unit"}}
        doc = self.doc(output=(), trace=[ev])
        other = [dict(ev, args={"x": "<redacted:internal>"})]
        self.assertEqual(self.compare(doc, stdout="", ntrace=other), ["the trace differs"])

    def test_the_clock_is_the_only_field_set_aside(self):
        ev = {"seq": 1, "task": None, "instance": "host:Clock", "capability": "Clock", "op": "now",
              "args": {}, "outcome": {"kind": "ok", "value": "1700000000000"}}
        doc = self.doc(output=(), trace=[ev])
        later = [dict(ev, outcome={"kind": "ok", "value": "1700000000999"})]
        self.assertEqual(self.compare(doc, stdout="", ntrace=later), [])
        # Only its value: another kind is a difference.
        failed = [dict(ev, outcome={"kind": "failed", "value": "1700000000999"})]
        self.assertEqual(self.compare(doc, stdout="", ntrace=failed), ["the trace differs"])

    def test_standard_error(self):
        # Empty on ok.
        self.assertEqual(self.compare(self.doc(), stderr="warning\n"),
                         ["standard error is not empty on a run that ends ok"])
        diag = {"severity": "error", "code": "U6001", "message": "main failed: Boom"}
        doc = self.doc(status="failed", output=(), trace=[], diagnostics=[diag])
        ok = ("[U6001] Error: main failed: Boom\n  at src/app/main.una:9:5\n"
              "  note: run `una explain U6001` for an explanation\n")
        self.assertEqual(self.compare(doc, code=1, stdout="", stderr=ok), [])
        # The error must be the first line, and only places may follow it.
        self.assertIn("the error", self.compare(doc, code=1, stdout="",
                                                stderr="noise\n" + ok)[0])
        self.assertIn("more than the error", self.compare(doc, code=1, stdout="",
                                                         stderr=ok + "extra\n")[0])
        # The note of another code is not the error's.
        self.assertIn("more than the error", self.compare(
            doc, code=1, stdout="",
            stderr=ok + "  note: run `una explain U6003` for an explanation\n")[0])
        self.assertIn("the error", self.compare(
            doc, code=1, stdout="", stderr="[U6001] Error: main failed: Bang\n")[0])

    def test_a_resource_abort_follows_the_native_test_and_its_trace_prefix(self):
        # ADR 0061: where each engine runs out is not exact; as the native test checks, the same
        # exit code, error and output; and one trace, without `seq`, a prefix of the other.
        diag = {"severity": "error", "code": "U6004",
                "message": "this call exceeds the maximum depth of 2000"}
        trace = [self.print_event(1, "a")]
        doc = self.doc(status="aborted", output=("a",), trace=trace, diagnostics=[diag])
        err = "[U6004] Error: this call exceeds the maximum depth of 2000\n  at src/a.una:3:1\n"
        self.assertEqual(self.compare(doc, code=1, stdout="a\n", stderr=err,
                                      ntrace=[self.print_event(7, "a")]), [])
        extra = [self.print_event(1, "a"), {"seq": 2, "task": None, "capability": "Files",
                                            "op": "write", "args": {}, "outcome": {"kind": "ok"}}]
        self.assertEqual(self.compare(doc, code=1, stdout="a\n", stderr=err, ntrace=extra), [])
        other = [{"seq": 1, "task": None, "capability": "Files", "op": "write", "args": {},
                  "outcome": {"kind": "ok"}}]
        self.assertEqual(self.compare(doc, code=1, stdout="a\n", stderr=err, ntrace=other),
                         ["neither trace is a prefix of the other"])
        self.assertIn("the error", self.compare(
            doc, code=1, stdout="a\n",
            stderr="[U6003] Error: this call exceeds the memory limit of 256 MiB\n")[0])
        self.assertEqual(self.compare(doc, code=1, stdout="", stderr=err),
                         ["standard output differs"])

    def test_tasks_with_quoted_lines_and_a_null_root(self):
        trace = [self.print_event(1, 'left {1} "q"', "0"), self.print_event(2, "right", "1"),
                 self.print_event(3, "2")]
        doc = self.doc(output=('left {1} "q"', "right", "2"), trace=trace)
        # The tasks' lines in another order: agreed.
        self.assertEqual(self.compare(doc, stdout='right\nleft {1} "q"\n2\n'), [])
        # Another line: not what the trace printed, nor the document's.
        self.assertTrue(self.compare(doc, stdout='left {1} "q"\nother\n2\n'))
        # On an ordered run (no failure, no cancellation), an extra task event is a divergence.
        extra = trace + [self.print_event(4, "late", "2")]
        self.assertIn("the trace differs on an ordered run",
                      self.compare(doc, stdout='left {1} "q"\nright\n2\nlate\n', ntrace=extra))
        # Whether a run is ordered is the interpreter's run's to say: a cancellation only the
        # native run has is a divergence, not a reason to compare more loosely.
        native_cancel = trace + [{"seq": 4, "task": "1", "cancelled": True}]
        self.assertIn("the trace differs on an ordered run",
                      self.compare(doc, stdout='left {1} "q"\nright\n2\n', ntrace=native_cancel))
        # On a run that is not ordered (a task cancelled), the extra events of tasks that started
        # after it are allowed, and only the lines they printed.
        cancel = {"seq": 5, "task": "1", "cancelled": True}
        unordered = trace + [cancel]
        doc2 = self.doc(output=('left {1} "q"', "right", "2"), trace=unordered)
        self.assertEqual(self.compare(doc2, stdout='left {1} "q"\nright\n2\nlate\n',
                                      ntrace=unordered + [self.print_event(6, "late", "2")]), [])
        self.assertTrue(self.compare(doc2, stdout='left {1} "q"\nright\n2\nlate\n',
                                     ntrace=unordered))

    def test_the_sample_is_a_fixed_function_of_the_name(self):
        names = [f"p{i}" for i in range(2000)]
        picked = [n for n in names if native.sampled(n)]
        self.assertEqual(picked, [n for n in names if native.sampled(n)])
        self.assertTrue(0.06 < len(picked) / len(names) < 0.10, len(picked))

    def test_the_pinned_list_is_read_by_name(self):
        self.assertIsInstance(native.unsupported(), dict)

    def test_every_compared_family_has_its_lowest_hash_program_in_the_sample(self):
        def row(name, op, compared, sampled=False, error=None):
            return {"name": name, "op": op, "compared": compared, "error": error,
                    "native": {"sampled": sampled}}
        rows = [row(f"a{i}", "C2", 1) for i in range(5)] + [row("b", "H3", 0),
                                                            row("e", "K4", 2, error="x")]
        picked = gate.native_strata(rows)
        self.assertEqual([r["op"] for r in picked], ["C2"])
        lowest = min((r for r in rows[:5]), key=lambda r: native.key(r["name"]))
        self.assertIs(picked[0], lowest)
        # A family whose lowest program is already in the hashed sample adds nothing.
        lowest["native"]["sampled"] = True
        self.assertEqual(gate.native_strata(rows), [])


if __name__ == "__main__":
    unittest.main()
