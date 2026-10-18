#!/usr/bin/env python3
"""Compares two `una --json run` documents of one program, run by two `una` binaries (ADR 0115).

Every field must be equal, but the outcome value of each `Clock.now` event: the host's time,
which differs between any two runs. That value is masked only after it is checked to be an
integer in both documents, so a malformed or labeled value there still fails. The event's place,
operation, argument names and outcome kind are compared like everything else, and the output byte
for byte.

Usage: scripts/compare-run-docs.py A.json B.json
       scripts/compare-run-docs.py --self-test
Prints the count of masked fields; exits 0 when the documents agree, 1 when they do not.
"""
import copy
import json
import re
import sys

INTEGER = re.compile(r"-?[0-9]+")


def masked(doc):
    """`doc` with the outcome value of every `Clock.now` event masked, and how many: or a reason
    why it cannot be, a value there that is not an integer."""
    doc = copy.deepcopy(doc)
    count = 0
    for event in doc.get("trace", []) if isinstance(doc, dict) else []:
        if event.get("capability") == "Clock" and event.get("op") == "now":
            outcome = event.get("outcome")
            if not isinstance(outcome, dict) or "value" not in outcome:
                continue
            value = outcome["value"]
            if not isinstance(value, str) or not INTEGER.fullmatch(value):
                return None, f"a Clock.now outcome is not an integer: {json.dumps(value)[:80]}"
            outcome["value"] = "<clock>"
            count += 1
    return (doc, count), None


def compare(a, b):
    """(agree, masked count, reason)."""
    ma, why = masked(a)
    if why:
        return False, 0, why
    mb, why = masked(b)
    if why:
        return False, 0, why
    (da, ca), (db, cb) = ma, mb
    if ca != cb:
        return False, 0, f"{ca} and {cb} Clock.now events"
    if da != db:
        return False, ca, "the documents differ outside the Clock.now outcome values"
    return True, ca, None


def self_test():
    """Each change the comparison must catch is caught, and the clock alone is not."""
    base = {"schema": "una.run/1", "status": "ok", "output": ["day 20379"], "trace": [
        {"seq": 1, "capability": "Clock", "op": "now", "args": {},
         "outcome": {"kind": "ok", "value": "1792339452111"}},
        {"seq": 2, "capability": "Console", "op": "print", "args": {"line": "day 20379"},
         "outcome": {"kind": "ok", "value": "unit"}}]}

    def changed(edit):
        doc = copy.deepcopy(base)
        edit(doc)
        return doc

    later = changed(lambda d: d["trace"][0]["outcome"].update(value="1792339453141"))
    agree = [("the same document", base), ("another time from the clock", later)]
    differ = [
        ("another output line", changed(lambda d: d["output"].__setitem__(0, "day 20380"))),
        ("another printed line", changed(
            lambda d: d["trace"][1]["args"].update(line="day 20380"))),
        ("another outcome value outside Clock.now", changed(
            lambda d: d["trace"][1]["outcome"].update(value="other"))),
        ("another outcome kind of the clock", changed(
            lambda d: d["trace"][0]["outcome"].update(kind="fail"))),
        ("a clock value that is not an integer", changed(
            lambda d: d["trace"][0]["outcome"].update(value="<redacted:secret>"))),
        ("another argument name", changed(lambda d: d["trace"][0].update(args={"x": "1"}))),
        ("another status", changed(lambda d: d.update(status="failed"))),
        ("one Clock.now event fewer", changed(lambda d: d["trace"].pop(0))),
    ]
    failures = []
    for name, doc in agree:
        ok, _, why = compare(base, doc)
        if not ok:
            failures.append(f"{name}: refused ({why})")
    for name, doc in differ:
        ok, _, _ = compare(base, doc)
        if ok:
            failures.append(f"{name}: accepted")
    if failures:
        for line in failures:
            print(f"compare-run-docs: self-test FAIL: {line}", file=sys.stderr)
        return 1
    print(f"compare-run-docs: self-test PASS ({len(agree)} agreeing, {len(differ)} differing)")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    docs = []
    for path in argv:
        try:
            with open(path, encoding="utf-8") as f:
                docs.append(json.load(f))
        except (OSError, ValueError) as e:
            print(f"compare-run-docs: {path}: {e}", file=sys.stderr)
            return 1
    ok, count, why = compare(*docs)
    if not ok:
        print(f"compare-run-docs: {why}", file=sys.stderr)
        return 1
    print(f"{count} Clock.now outcome values masked")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
