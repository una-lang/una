"""Tests of family N and the server run (#1011) that need no compiler.

    cd scripts/security-hunt && python3 -m unittest discover -s tests
"""

import base64
import json
import os
import sys
import tempfile
import unittest
import urllib.parse

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from hunt import drivers, families, gate, oracle, serving  # noqa: E402
from hunt.families import OPERATORS, SERVER_OPERATORS, servers  # noqa: E402
from hunt.program import OPEN, derive_seed  # noqa: E402
from hunt.sandbox import Run  # noqa: E402

DATE = b"Date: Tue, 14 Nov 2023 22:13:20 GMT\r\n"


def reply(status, body=b"", close=False, reason=b"X"):
    head = b"HTTP/1.1 %d %s\r\n" % (status, reason) + DATE
    if status != 100:
        head += b"Content-Length: %d\r\n" % len(body)
    if close:
        head += b"Connection: close\r\n"
    return head + b"\r\n" + body


def wire_text(conns):
    """A `--serve-out` text from {k: (bytes, ends)}."""
    lines = []
    for k, (data, ends) in sorted(conns.items()):
        lines.append({"at": k, "bytes": base64.b64encode(data).decode(), "connection": k})
        lines += [{"at": k, "connection": k, end: True} for end in sorted(ends)]
    return "".join(json.dumps(x, sort_keys=True) + "\n" for x in lines)


CONTROL = reply(200, b"seen /control", close=True)


class FamilyN(unittest.TestCase):
    def test_family_n_is_apart_from_the_fixed_run(self):
        self.assertEqual(sorted(SERVER_OPERATORS), ["N1", "N2", "N3", "N4", "N5", "N6"])
        self.assertFalse(set(SERVER_OPERATORS) & set(OPERATORS))
        self.assertFalse(set(SERVER_OPERATORS) & set(families.TIMING_OPERATORS))
        self.assertFalse(any(families.family_of(op) == "N" for op in OPERATORS))

    def test_the_fixed_run_writes_no_serve_field(self):
        # The manifest rows of every other family are unchanged: `serve` only when not empty.
        for op in sorted(OPERATORS)[:6]:
            row = OPERATORS[op](derive_seed(767, op, 0), families.editions_of(op)[0]).manifest()
            self.assertNotIn("serve", row)
        self.assertNotIn("N1", {p.op for p in gate.programs()[:200]})

    def test_every_form_runs_at_any_count(self):
        forms = {(op, f) for op, fs in servers.FORMS.items() for f in fs}
        for count in (1, 5, serving.COUNT):
            progs = serving.programs(count=count)
            self.assertEqual({(p.op, p.variant) for p in progs}, forms, count)
            self.assertEqual(len({p.name for p in progs}), len(progs))

    def test_a_seed_rebuilds_the_same_program(self):
        for p in serving.programs(count=2):
            again = drivers.rebuild(dict(p.manifest(), digest=p.digest()))
            self.assertEqual(again.digest(), p.digest())
            self.assertEqual(again.serve, p.serve)

    def test_only_edition_0_4(self):
        for op, gen in SERVER_OPERATORS.items():
            with self.assertRaises(ValueError):
                gen(1, "0.3")

    def test_a_server_package_and_its_script(self):
        for p in serving.programs(count=3):
            if p.op in ("N5", "N6"):
                continue
            self.assertEqual(p.cls, OPEN)
            self.assertIn("edition 0.4", p.files["ita.una"])
            self.assertIn(servers.GRANT, p.files["ita.una"])
            self.assertIn("ita.lock", p.files)
            self.assertEqual(p.serve["kind"], "request")
            self.assertIn(p.serve["status"], (200, 400, 413, 414, 415, 417, 421, 431, 501, 505))
            self.assertEqual("answers" in p.serve, p.serve["status"] == 200, p.name)
            lines = [json.loads(x) for x in p.files["script.jsonl"].splitlines()]
            ats = [x["at"] for x in lines]
            self.assertEqual(ats, sorted(ats))
            sent = {}
            for x in lines:
                self.assertIn(set(x) - {"at", "connection"},
                              ({"grant", "peer", "reads"}, {"bytes"}), p.name)
                if "bytes" in x:
                    sent[x["connection"]] = sent.get(x["connection"], b"") + base64.b64decode(
                        x["bytes"], validate=True)
                else:
                    self.assertEqual(x["grant"], servers.LISTENER)
            # Connection 2 sends connection 1's bytes, split; 3 the control request.
            self.assertEqual(sent[1], sent[2])
            self.assertTrue(sent[1].endswith(servers.BEHIND))
            self.assertEqual(sent[3], servers.CONTROL)
            self.assertGreater(sum(1 for x in lines if x["connection"] == 2 and "bytes" in x), 1)

    def test_split_keeps_every_byte(self):
        import random
        rng = random.Random(4)
        for n in (2, 3, 10, 200):
            data = bytes(range(n))
            pieces = servers.split(rng, data, 5)
            self.assertEqual(b"".join(d for _t, d in pieces), data)
            self.assertEqual([t for t, _d in pieces], list(range(5, 5 + len(pieces))))
            self.assertTrue(all(d for _t, d in pieces))

    def test_refusals_follow_the_spec_rules(self):
        # A few pinned forms: what spec/31 §3 and §4 say of them.
        def first(op, form):
            for p in serving.programs(count=1):
                if p.op == op and p.variant == form:
                    return p
            self.fail(form)
        self.assertEqual(first("N3", "length-and-chunked").serve["status"], 400)
        self.assertEqual(first("N3", "coding-not-implemented").serve["status"], 501)
        self.assertEqual(first("N4", "host-not-granted").serve["status"], 421)
        self.assertEqual(first("N1", "version").serve["status"], 505)
        self.assertEqual(first("N2", "encoding").serve["status"], 415)
        self.assertEqual(first("N2", "expectation").serve["status"], 417)
        self.assertTrue(first("N2", "continue").serve["continue"])
        # `HTTP/1.0` closes the connection after its reply: the request behind it is never read.
        self.assertEqual(len(first("N1", "http10").serve["answers"]), 1)
        self.assertEqual(len(first("N1", "plain").serve["answers"]), 2)


class FamilyN5(unittest.TestCase):
    """N5, replies with injected headers (`spec/31 §5` rule 1)."""

    def programs(self, count=3):
        return [p for p in serving.programs(count=count) if p.op == "N5"]

    def test_every_form_in_both_modes(self):
        self.assertEqual({p.variant for p in self.programs()}, set(servers.FORMS["N5"]))
        self.assertEqual(len(servers.FORMS["N5"]), 2 * len(servers.N5_FORMS))

    def test_one_connection_and_a_value_the_request_line_can_carry(self):
        for p in self.programs():
            self.assertEqual(p.serve["kind"], "reply")
            lines = [json.loads(x) for x in p.files["script.jsonl"].splitlines()]
            self.assertEqual({x["connection"] for x in lines}, {1})
            data = base64.b64decode(lines[1]["bytes"])
            line = data.split(b"\r\n", 1)[0]
            # The request itself is well-formed: every byte of its target is printable ASCII.
            self.assertTrue(all(0x21 <= b <= 0x7E for b in line.split(b" ")[1]), p.name)
            self.assertIn(b"\r\nConnection: close\r\n", data)
            text = p.files["src/srv.una"]
            self.assertNotIn("NAME", text)
            self.assertNotIn("STATUS", text)
            # A refused value carries the marker, so an injection that reaches the wire shows.
            if p.serve["status"] == "refused" and p.variant.startswith("value-"):
                target = line.split(b" ")[1].decode("ascii")
                self.assertIn(servers.MARK, urllib.parse.unquote(target), p.name)

    def test_una_text_writes_every_control_by_an_escape(self):
        lit = servers.una_text('a"b\\c{d}\n\t\r\x00\x7fç')
        self.assertEqual(lit, '"a\\"b\\\\c\\{d\\}\\n\\t\\u{D}\\u{0}\\u{7F}ç"')
        self.assertFalse(any(ord(c) < 0x20 or ord(c) == 0x7F for c in lit))

    def test_query_encoding_is_ascii(self):
        self.assertEqual(servers._q("ç a\r\n"), "%C3%A7%20a%0D%0A")
        self.assertEqual(servers._q("A-z._~9"), "A-z._~9")

    def judge(self, expect, data, ends=("closed",)):
        text = wire_text({1: (data, set(ends))})
        return serving.judge_reply(dict(expect, marker=servers.MARK),
                                   serving.read_wire(text), 0)

    def test_a_refused_reply_with_a_fallback_or_the_host_answer(self):
        fallback = {"status": "refused", "mode": "fallback"}
        auto = {"status": "refused", "mode": "auto"}
        self.assertEqual(self.judge(fallback, reply(502, b"bad reply", close=True)), [])
        self.assertEqual(self.judge(auto, reply(500, close=True)), [])
        self.assertTrue(self.judge(fallback, reply(500, close=True)))
        self.assertTrue(self.judge(auto, reply(200, b"ok", close=True)))

    def test_an_injection_on_the_wire_is_a_finding(self):
        expect = {"status": "refused", "mode": "auto"}
        split = (b"HTTP/1.1 200 OK\r\n" + DATE + b"X-Echo: a\r\nSet-Cookie: pwn=1\r\n"
                 b"Content-Length: 2\r\n\r\nok")
        found = self.judge(expect, split)
        self.assertTrue(any("injected" in f for f in found), found)
        found = self.judge(expect, reply(500, close=True) + reply(200, b"PWN"))
        self.assertTrue(any("injected" in f for f in found), found)

    def test_an_echoed_header_sent_once_as_given(self):
        expect = {"status": 200, "mode": "fallback", "header": ["X-Echo", "a b"]}
        good = (b"HTTP/1.1 200 OK\r\n" + DATE + b"Content-Length: 2\r\nX-Echo: a b\r\n"
                b"Connection: close\r\n\r\nok")
        self.assertEqual(self.judge(expect, good), [])
        self.assertTrue(self.judge(expect, good.replace(b"X-Echo: a b", b"X-Echo: ab")))
        self.assertTrue(self.judge(expect, good, ends=()))


class FamilyN6(unittest.TestCase):
    """N6, handlers that read labeled data, for the oracle's server mode."""

    def programs(self, count=5):
        return [p for p in serving.programs(count=count) if p.op == "N6"]

    def test_every_form_with_its_expected_code(self):
        seen = {}
        for p in self.programs():
            seen[p.variant] = p.expect
            self.assertEqual(p.expect, [servers.N6_CLOSED[p.variant]]
                             if p.variant in servers.N6_CLOSED else [], p.name)
        self.assertEqual(set(seen), set(servers.N6_FORMS))

    def test_one_varied_secret_and_the_oracle_route(self):
        for p in self.programs():
            self.assertEqual(p.serve, {"kind": "oracle"})
            self.assertFalse(serving.judged(p.manifest()))
            self.assertEqual(len(p.vary), 1, p.name)
            self.assertEqual((p.vary[0]["type"], p.vary[0]["label"]), ("Int", "secret"))
            text = p.files[p.vary[0]["file"]]
            line = text.splitlines()[p.vary[0]["line"] - 1]
            col = p.vary[0]["col"] - 1
            self.assertEqual(line[col:col + len(p.vary[0]["literal"])], p.vary[0]["literal"])
            self.assertNotIn("⟦", text)
            self.assertNotIn("SECRET", text)
            self.assertNotIn("LOCAL", text)
            self.assertIn("script.jsonl", p.files)

    def test_a_released_form_is_compared_at_public_only(self):
        released = [p for p in self.programs() if p.variant == "released"]
        self.assertTrue(released)
        for p in released:
            self.assertTrue(oracle.releases_to(p.files, "internal"))
            self.assertFalse(oracle.releases_to_public(p.files))
            self.assertIn("trusted\n  release", p.files["ita.una"])
        others = [p for p in self.programs() if p.variant != "released"]
        self.assertFalse(any(oracle.releases_to(p.files, "internal") for p in others))


class OracleServeMode(unittest.TestCase):
    """The oracle's server mode: the wire in the `internal` view, nothing else changed."""

    TABLE = {**{k: {v} for k, v in oracle.HOST_ACCEPTS.items()},
             **{k: {v} for k, v in oracle.SERVER_ACCEPTS.items()}}

    def doc(self, wire=None):
        d = {"schema": "una.run/1", "status": "ok", "output": ["stopped"], "failure": None,
             "diagnostics": [], "trace": []}
        if wire is not None:
            d["serve_out"] = wire
        return d

    def test_internal_sees_the_wire_and_public_does_not(self):
        a = self.doc([{"at": 1, "bytes": "QQ==", "connection": 1}])
        b = self.doc([{"at": 1, "bytes": "Qg==", "connection": 1}])
        self.assertEqual(oracle.project(a, "public", self.TABLE),
                         oracle.project(b, "public", self.TABLE))
        self.assertNotEqual(oracle.project(a, "internal", self.TABLE),
                            oracle.project(b, "internal", self.TABLE))

    def test_without_the_mode_no_view_has_a_wire(self):
        for observer in ("public", "internal"):
            self.assertNotIn("wire", oracle.project(self.doc(), observer, self.TABLE))

    def test_the_server_operations_are_not_in_the_host_table(self):
        # The fixed run's table is unchanged: the server's operations join it in the mode alone.
        self.assertFalse(set(oracle.SERVER_ACCEPTS) & set(oracle.HOST_ACCEPTS))
        self.assertEqual(oracle.SERVER_ACCEPTS[("HttpServer", "close")], "public")

    def test_the_mode_runs_the_script_and_reads_the_wire(self):
        argvs = []
        saved = oracle.sandbox.run, oracle.sandbox.fresh_dir

        def run(argv, cwd, writable, timeout, env_extra=None):
            argvs.append((argv, env_extra))
            if "--serve-out" in argv:
                with open(argv[argv.index("--serve-out") + 1], "w", encoding="utf-8") as f:
                    f.write('{"at":1,"bytes":"QQ==","connection":1}\n')
            return Run(0, json.dumps(self.doc()), "", False, 0.1)
        oracle.sandbox.run = run
        try:
            with tempfile.TemporaryDirectory() as tmp:
                oracle.sandbox.fresh_dir = lambda root, prefix: tempfile.mkdtemp(dir=tmp)
                end = oracle.run_twice("una", {"ita.una": "x\n"}, "p", tmp, 1,
                                       ["public", "internal"], self.TABLE, "secret", serve=True)
                plain = oracle.run_twice("una", {"ita.una": "x\n"}, "p", tmp, 1,
                                         ["public", "internal"], self.TABLE, "secret")
        finally:
            oracle.sandbox.run, oracle.sandbox.fresh_dir = saved
        self.assertEqual(end[0], "complete")
        self.assertEqual(end[1]["views"]["internal"]["wire"],
                         [{"at": 1, "bytes": "QQ==", "connection": 1}])
        self.assertNotIn("wire", end[1]["views"]["public"])
        self.assertNotIn("wire", plain[1]["views"]["internal"])
        served = [a for a, env in argvs if "--serve-script" in a]
        self.assertEqual(len(served), 2)
        for a, env in argvs:
            if "--serve-script" in a:
                self.assertIn("--virtual-time", a)
                self.assertEqual(env.get("UNA_TEST_RUN_SERVER"), "1")
            else:
                self.assertNotIn("UNA_TEST_RUN_SERVER", env or {})


class Judge(unittest.TestCase):
    REFUSED = {"kind": "request", "status": 400}
    ANSWERED = {"kind": "request", **servers.answered("/a")}

    def judge(self, expect, conns, replies_sent):
        return serving.judge_request(expect, serving.read_wire(wire_text(conns)), replies_sent)

    def refusal(self, status=400):
        return reply(status, close=True)

    def test_a_clean_refusal(self):
        r = self.refusal()
        conns = {1: (r, {"shutdown"}), 2: (r, {"shutdown", "closed"}), 3: (CONTROL, {"closed"})}
        self.assertEqual(self.judge(self.REFUSED, conns, 1), [])

    def test_a_smuggled_request_is_a_finding(self):
        r = self.refusal() + reply(200, b"seen /behind", close=True)
        conns = {1: (r, {"closed"}), 2: (r, {"closed"}), 3: (CONTROL, {"closed"})}
        found = self.judge(self.REFUSED, conns, 3)
        self.assertTrue(any("behind a refusal" in f for f in found), found)
        self.assertTrue(any("replied 3 times" in f for f in found), found)

    def test_another_status_is_a_finding(self):
        r = self.refusal(501)
        conns = {1: (r, {"shutdown"}), 2: (r, {"shutdown"}), 3: (CONTROL, {"closed"})}
        self.assertTrue(self.judge(self.REFUSED, conns, 1))

    def test_a_refusal_that_keeps_the_connection_is_a_finding(self):
        r = reply(400)
        conns = {1: (r, set()), 2: (r, set()), 3: (CONTROL, {"closed"})}
        found = self.judge(self.REFUSED, conns, 1)
        self.assertTrue(any("Connection: close" in f for f in found), found)
        self.assertTrue(any("stays open" in f for f in found), found)

    def test_a_split_read_otherwise_is_a_finding(self):
        r = self.refusal()
        conns = {1: (r, {"shutdown"}), 2: (self.refusal(431), {"shutdown"}),
                 3: (CONTROL, {"closed"})}
        found = self.judge(self.REFUSED, conns, 1)
        self.assertTrue(any("split" in f for f in found), found)

    def test_the_date_alone_may_differ_between_connections(self):
        r = self.refusal()
        later = r.replace(b"22:13:20", b"22:13:21")
        conns = {1: (r, {"shutdown"}), 2: (later, {"shutdown"}), 3: (CONTROL, {"closed"})}
        self.assertEqual(self.judge(self.REFUSED, conns, 1), [])

    def test_a_clean_twin_and_a_refused_twin(self):
        ok = reply(200, b"seen /a") + reply(200, b"seen /behind", close=True)
        conns = {1: (ok, {"closed"}), 2: (ok, {"closed"}), 3: (CONTROL, {"closed"})}
        self.assertEqual(self.judge(self.ANSWERED, conns, 5), [])
        r = self.refusal()
        conns = {1: (r, {"shutdown"}), 2: (r, {"shutdown"}), 3: (CONTROL, {"closed"})}
        self.assertTrue(self.judge(self.ANSWERED, conns, 1))

    def test_continue_comes_first(self):
        expect = {"kind": "request", **servers.answered("/a", continued=True)}
        ok = reply(100) + reply(200, b"seen /a") + reply(200, b"seen /behind", close=True)
        conns = {1: (ok, {"closed"}), 2: (ok, {"closed"}), 3: (CONTROL, {"closed"})}
        self.assertEqual(self.judge(expect, conns, 5), [])

    def test_a_server_that_died_is_a_finding(self):
        r = self.refusal()
        conns = {1: (r, {"shutdown"}), 2: (r, {"shutdown"}), 3: (b"", {"closed"})}
        found = self.judge(self.REFUSED, conns, 0)
        self.assertTrue(any("connection 3" in f for f in found), found)

    def test_unframed_bytes_are_a_finding(self):
        bad = b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nseen"
        conns = {1: (bad, {"closed"}), 2: (bad, {"closed"}), 3: (CONTROL, {"closed"})}
        self.assertTrue(self.judge(self.ANSWERED, conns, 5))
        with self.assertRaises(serving.WireError):
            serving.replies(b"HTTP/1.1 200 OK\r\nX: a\r\nInjected\r\n\r\n")
        with self.assertRaises(serving.WireError):
            serving.replies(b"HTTP/1.1 200 OK\r\n\r\nbody")

    def test_a_wire_line_of_no_form_is_refused(self):
        for text in ('{"at": 1}\n', '{"at": 1, "connection": 1, "x": true}\n', "nope\n",
                     '{"at": 1, "connection": 1, "bytes": "!!"}\n'):
            with self.assertRaises(serving.WireError, msg=text):
                serving.read_wire(text)

    def test_judge_needs_a_known_expectation(self):
        p = serving.programs(count=1)[0]
        bad = type(p)(**{**p.__dict__, "serve": {"kind": "other"}})
        with self.assertRaises(ValueError):
            serving.judge(bad, {}, 0)


class Runs(unittest.TestCase):
    def test_a_una_that_does_not_serve_scripts_stops_the_run(self):
        run = Run(2, "", "error: the scripted connections of `--serve-script` " + serving.PENDING,
                  False, 0.1)
        with self.assertRaises(serving.NotATestBuild):
            serving.end_of(run, None)

    def test_a_run_must_end_ok_with_stopped(self):
        good = json.dumps({"schema": "una.run/1", "status": "ok", "output": ["stopped"],
                           "diagnostics": [], "trace": []})
        self.assertEqual(serving.end_of(Run(0, good, "", False, 0.1), "")[0], "ok")
        self.assertEqual(serving.end_of(Run(0, good, "", False, 0.1), None)[0], "error")
        self.assertEqual(serving.end_of(Run(None, "", "", True, 30), "")[0], "error")
        aborted = good.replace('"ok"', '"aborted"')
        self.assertEqual(serving.end_of(Run(1, aborted, "", False, 0.1), "")[0], "error")

    def run_with(self, results):
        """`serve_program` on a program with `sandbox.run` replaced by a stub that returns each
        of `results`, (run, serve-out text), in turn."""
        prog = serving.programs(count=1)[0]
        saved = serving.sandbox.run, serving.sandbox.fresh_dir
        queue = list(results)

        def run(argv, cwd, writable, timeout, env_extra=None):
            self.assertEqual(env_extra.get("UNA_TEST_RUN_SERVER"), "1")
            self.assertIn("--virtual-time", argv)
            self.assertIn("--serve-script", argv)
            r, text = queue.pop(0)
            if text is not None:
                with open(argv[argv.index("--serve-out") + 1], "w", encoding="utf-8") as f:
                    f.write(text)
            return r
        serving.sandbox.run = run
        try:
            with tempfile.TemporaryDirectory() as tmp:
                serving.sandbox.fresh_dir = lambda root, prefix: tempfile.mkdtemp(dir=tmp)
                return prog, serving.serve_program(prog, "una", tmp)
        finally:
            serving.sandbox.run, serving.sandbox.fresh_dir = saved

    def test_two_runs_that_differ_are_an_error(self):
        doc = json.dumps({"schema": "una.run/1", "status": "ok", "output": ["stopped"],
                          "diagnostics": [], "trace": []})
        _p, row = self.run_with([(Run(0, doc, "", False, 0.1), "a\n"),
                                 (Run(0, doc, "", False, 0.1), "b\n")])
        self.assertIn("nondeterministic", row["error"])

    def test_an_engine_not_at_rest_is_a_finding(self):
        doc = json.dumps({"schema": "una.run/1", "status": "ok", "output": ["stopped"],
                          "diagnostics": [], "trace": []})
        run = Run(0, doc, "warning: a reserve is left\n", False, 0.1)
        _p, row = self.run_with([(run, ""), (run, "")])
        self.assertTrue(row["finding"])
        self.assertTrue(any("not at rest" in f for f in row["findings"]), row)


if __name__ == "__main__":
    unittest.main()
