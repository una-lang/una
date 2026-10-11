"""Writes formal/UnaFormal/F1/WithImportTests.lean (#538) from `una --json core --with-rejected`.

The package is `formal/gen/with_import_tests.una` as the module `m` of an edition-0.2 package.
`una check` rejects `run` (C5(b), U4002) and `chime_secret` (ruling E's case (a), U4002); the
export lists rejected declarations under `rejected`, which are put back into the module, in source
order, and the document is marked complete. Nothing else of the export changes. A second document,
`unbound`, is the same with `reach`'s `with` removed: a call of `branch` where no scope installs
`Vault`'s handler, which `una check` would reject before export (ruling E's case (c)); and a third,
`forged`, binds a user name starting with `@`.

Usage: una --json core --with-rejected <pkg> > export.json
       python3 formal/gen/with_import_tests.py export.json formal/UnaFormal/F1/WithImportTests.lean
"""
import copy, json, sys

def name(d):
    return d["value"]["name"] if d["kind"] == "fun" else d.get("name", d.get("value", {}).get("name"))

def main(src, out):
    doc = json.load(open(src, encoding="utf-8"))
    (mod,) = [m for m in doc["modules"] if m["path"] == "m"]
    order = [name(d) for d in mod["decls"]]
    rejected = [r["decl"] for r in doc.pop("rejected", []) if r["module"] == mod["path"]]
    src_order = ["Vault", "quiet", "tally", "Shut", "Door", "failing", "fine", "work", "guarded",
                 "branch", "reach", "pick", "open_door", "run", "Bell", "noisy", "chime",
                 "chime_secret", "chime_public", "Gong", "soft", "hard", "strike_it", "pick_gong"]
    decls = mod["decls"] + rejected
    assert sorted(map(name, decls)) == sorted(src_order), sorted(map(name, decls))
    mod["decls"] = sorted(decls, key=lambda d: src_order.index(name(d)))
    doc["complete"] = True
    # `Gong` in a document of its own: F1 has one event operation (G3), and `Gong`'s handlers,
    # which disagree, leave `Gong.strike` the event, beside `Console.print`.
    gong = ["Gong", "soft", "hard", "strike_it", "pick_gong"]
    mixed = copy.deepcopy(doc)
    (xm,) = [m for m in mixed["modules"] if m["path"] == "m"]
    xm["decls"] = [d for d in xm["decls"] if name(d) in gong]
    mixed_text = json.dumps(mixed, separators=(",", ":"), ensure_ascii=False)
    mod["decls"] = [d for d in mod["decls"] if name(d) not in gong]
    text = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)
    # (c): `reach` calls `branch` with no `with` around it.
    unbound = copy.deepcopy(doc)
    for d in unbound["modules"][0]["decls"] if unbound["modules"][0]["path"] == "m" else []:
        pass
    (um,) = [m for m in unbound["modules"] if m["path"] == "m"]
    for d in um["decls"]:
        if d["kind"] == "fun" and d["value"]["name"] == "reach":
            assert d["value"]["body"]["kind"] == "with"
            d["value"]["body"] = d["value"]["body"]["body"]
    unbound_text = json.dumps(unbound, separators=(",", ":"), ensure_ascii=False)
    # A forged marker: `work`'s parameter renamed `@Vault.note`.
    forged = copy.deepcopy(doc)
    (fm,) = [m for m in forged["modules"] if m["path"] == "m"]
    for d in fm["decls"]:
        if d["kind"] == "fun" and d["value"]["name"] == "work":
            d["value"]["params"][0][0] = "@Vault.note"
    forged_text = json.dumps(forged, separators=(",", ":"), ensure_ascii=False)
    # A document capability named as one of the host's, `Clock`: the document is not read.
    hostnamed = copy.deepcopy(doc)
    (hm,) = [m for m in hostnamed["modules"] if m["path"] == "m"]
    vault = next(d for d in hm["decls"] if d["kind"] == "cap" and d.get("name") == "Vault")
    clock = copy.deepcopy(vault)
    clock["name"] = "Clock"
    hm["decls"].append(clock)
    hostnamed_text = json.dumps(hostnamed, separators=(",", ":"), ensure_ascii=False)
    for t in (text, unbound_text, forged_text, mixed_text, hostnamed_text):
        assert '"#' not in t
    with open(out, "w", encoding="utf-8") as f:
        f.write(f'''/-
Tests of the reading of `with` scopes (#538), checked when the library builds. The documents are
written by `formal/gen/with_import_tests.py` from `una`'s own export of
`formal/gen/with_import_tests.una`; each verdict is what `una check` says of the declaration, or
not re-checked where F1 does not read it.
-/
import UnaFormal.F1.ImportTests

namespace Una.F1.WithImportTests

open Import ImportTests

/-- The verdict of `decl` in `doc` read with its `with` scopes only, not combined with #524's
reading (`Import.checkDocumentRead`). -/
def runScoped (doc : String) (decl : String) : String :=
  match Lean.Json.parse doc with
  | .error _ => "the test document does not parse"
  | .ok j => match checkDocumentRead (fun rd σ t => t.lsubst rd σ) j {{}} true with
    | .error e => s!"error: {{e}}"
    | .ok outs => match outs.lookup decl with
      | some o => verdict o
      | none => "missing"

/-- `Vault.note` (accepts `secret`) with two quiet handlers, `quiet` and `tally`; `Door.open`
(fails `Shut`, accepts `secret`) with `failing` and `fine`; `Bell.ring` (accepts `public`) with
`noisy`, which prints. -/
def withs : String := r#"{text}"#

/-- The same, with `reach`'s `with` removed (ruling E, case (c)). -/
def unbound : String := r#"{unbound_text}"#

/-- The same, with a user parameter named as a marker. -/
def forged : String := r#"{forged_text}"#

/-- `Gong.strike` with `soft`, quiet, and `hard`, which prints, chosen by two scopes. -/
def mixed : String := r#"{mixed_text}"#

/-- The same as `withs`, with a capability of its own named `Clock`, one of the host's. -/
def hostNamed : String := r#"{hostnamed_text}"#

-- H3: a handler installed under a secret program counter, and one installed publicly and reached
-- from a secret branch: accepted, as `una check` says. So is what they install and invoke.
#guard run withs "m.guarded" == "accepted"
#guard run withs "m.reach" == "accepted"
#guard run withs "m.branch" == "accepted"
#guard run withs "m.work" == "accepted"
#guard run withs "m.quiet.note" == "accepted"
-- Two handlers of one operation, chosen by two scopes: accepted.
#guard run withs "m.pick" == "accepted"
#guard run withs "m.tally.note" == "accepted"
-- C5(b): a failing handler chosen under a secret program counter, its failure passed out of the
-- branch: rejected, as `una check` rejects it (U4002).
#guard run withs "m.run" == "rejected"
#guard run withs "m.open_door" == "accepted"
-- Ruling E: a printing handler installed under a secret program counter is rejected (a), and at a
-- public one certified (b); the code that invokes it emits.
#guard run withs "m.chime_secret" == "rejected"
#guard run withs "m.chime_public" == "accepted"
#guard run withs "m.chime" == "accepted"
#guard run withs "m.noisy.ring" == "accepted"
-- A quiet and a printing handler of one operation, chosen by two scopes: not read by scopes, so
-- the `with`s are not re-checked (fail closed).
#guard run mixed "m.pick_gong" == "not re-checked"
-- Its invoker, outside the scopes: `Gong` has two handlers and no reading fixes which one runs, so
-- neither reads `Gong.strike` as F1's event (fail closed).
#guard run mixed "m.strike_it" == "not re-checked"
-- (c): a call of a function that invokes an operation no scope installs there: not re-checked by
-- the scoped reading, nor by #524's, which does not read an operation of a capability `with`
-- scopes install.
#guard runScoped unbound "m.reach" == "not re-checked"
#guard run unbound "m.reach" == "not re-checked"
-- A document capability named as one of the host's (`std.host`, here `Clock`): the document is not
-- read, since `uses` names a capability without its module.
#guard ["m.work", "m.guarded", "m.chime"].all (run hostNamed · == "not re-checked")
-- A user name starting with `@`, as a marker's: the document is not read.
#guard ["m.work", "m.guarded", "m.reach"].all (run forged · == "not re-checked")

end Una.F1.WithImportTests
''')

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
