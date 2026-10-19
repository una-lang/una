//! `ita.una`, `ita.lock` and `itaty.una` read into the settings of a package (ADR 0122,
//! `spec/30`): the checks that need no other file.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_diag::Code;
use una_project::ita::{
    check_lock_consistency, parse_ita_lock, parse_ita_manifest, parse_workspace,
};
use una_project::{Edition, Format, ProjectError};

const HEAD: &str = "ita case\n  version 0.1.0\n  edition 0.4\n";
const LOCK: &str = "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n";
const DIGEST: &str = "sha256:3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";

/// The errors of `ita.una` text as "code message".
fn errors(text: &str) -> Vec<String> {
    render(&parse_ita_manifest(text).1)
}

fn render(errors: &[ProjectError]) -> Vec<String> {
    errors
        .iter()
        .map(|e| format!("{} {}", e.code, e.message))
        .collect()
}

fn has(text: &str, code: Code, part: &str) {
    let found = errors(text);
    assert!(
        found
            .iter()
            .any(|e| e.starts_with(code.as_str()) && e.contains(part)),
        "{text:?}: expected {code} with {part:?}, got {found:?}"
    );
}

#[test]
fn settings_read_into_the_one_manifest() {
    let text = format!(
        "{HEAD}\nlabels\n  PII\n\ntrusted\n  events.keys\n\nescapes\n  checked per module 2\n\nboundaries\n  gallery\n    may Files[read]\n\ncontracts\n  pricing.update\n    @catalog: Catalog[price]\n\nrun\n  entry main\n  grants\n    console\n    keys \"k.toml\"\n"
    );
    let (manifest, errors) = parse_ita_manifest(&text);
    assert_eq!(render(&errors), Vec::<String>::new());
    assert_eq!(manifest.format, Format::Ita);
    assert_eq!(manifest.edition, Edition::V0_4);
    assert_eq!(manifest.name.unwrap().value, "case");
    assert_eq!(manifest.categories[0].value, "PII");
    assert_eq!(manifest.trusted[0].value, "events.keys");
    assert_eq!(manifest.checked_per_module, 2);
    assert_eq!(manifest.boundaries[0].ceiling[0].capability, "Files");
    assert_eq!(manifest.boundaries[0].ceiling[0].at.value, "Files[read]");
    assert_eq!(
        manifest.contracts[0].clauses[0].value,
        "@catalog: Catalog[price]"
    );
    let grants: Vec<&str> = manifest.grants.iter().map(|g| g.value.as_str()).collect();
    assert_eq!(grants, ["console", "keys:k.toml"]);
    assert_eq!(manifest.entry.unwrap().value, "main");
}

#[test]
fn the_head_is_checked() {
    has(
        "ita case\n  version 1.0\n  edition 0.4\n",
        Code::U7008,
        "full version",
    );
    has(
        "ita case\n  version 1.0.0\n  edition 0.3\n",
        Code::U7008,
        "keeps una.toml",
    );
    has(
        "ita case\n  version 1.0.0\n  edition 0.9\n",
        Code::U7008,
        "not supported",
    );
    has(
        &HEAD.replace("case", "Case_"),
        Code::U7008,
        "expected a package name",
    );
    has(
        &HEAD.replace("case", "a__b"),
        Code::U7008,
        "not a package name",
    );
    has(
        &HEAD.replace("case", "a/b"),
        Code::U7008,
        "not an owner name",
    );
    has(
        &format!("{HEAD}  profile systems\n"),
        Code::U7008,
        "not supported by this release",
    );
    has(
        &format!("{HEAD}  home \"http://x\"\n"),
        Code::U7008,
        "`https://` URL",
    );
    has(
        &format!("{HEAD}  publish to acme\n"),
        Code::U7008,
        "names no source",
    );
    assert_eq!(
        errors(&format!("{HEAD}  profile app\n  publish to itaty\n")),
        Vec::<String>::new()
    );
}

#[test]
fn needs_are_checked() {
    let source = format!(
        "\nsources\n  acme \"https://ita.acme.example\"\n    root \"{DIGEST}\"\n    owners acme\n"
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0 as std\n"),
        Code::U7008,
        "`std` is reserved",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0 as s7d\n"),
        Code::U7008,
        "skeleton of the reserved name `std`",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0 as std_\n"),
        Code::U7008,
        "reserved name `std`",
    );
    has(
        &format!("{HEAD}\nneeds\n  module 2.0\n"),
        Code::U7008,
        "`module` is reserved",
    );
    has(
        &format!("{HEAD}\nneeds\n  aa/json 2.0\n  bb/json 1.0\n"),
        Code::U7008,
        "local name `json` is already",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0\n  json 2.3\n"),
        Code::U7008,
        "needed twice on the major line 2",
    );
    assert_eq!(
        errors(&format!("{HEAD}\nneeds\n  json 1.0\n  json 2.0 as json2\n")),
        Vec::<String>::new()
    );
    has(
        &format!("{HEAD}\nneeds\n  json\n"),
        Code::U7008,
        "names no version",
    );
    has(
        &format!("{HEAD}\nneeds\n  util 1.0 from path \"../util\"\n"),
        Code::U7008,
        "takes its version",
    );
    has(
        &format!("{HEAD}\nneeds\n  util from path \"/abs\"\n"),
        Code::U7008,
        "relative",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0 from nowhere\n"),
        Code::U7008,
        "names no source",
    );
    has(
        &format!("{HEAD}{source}\nneeds\n  acme/ledger 0.3\n"),
        Code::U7008,
        "write `from acme`",
    );
    has(
        &format!("{HEAD}{source}\nneeds\n  acme/ledger 0.3 from itaty\n"),
        Code::U7008,
        "write `from acme`",
    );
    assert_eq!(
        errors(&format!(
            "{HEAD}{source}\nneeds\n  acme/ledger 0.3 from acme\n"
        )),
        Vec::<String>::new()
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0\n    may Http[get, get]\n"),
        Code::U7008,
        "operation `get` twice",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0\n    may Http[get]\n    may Http[post]\n"),
        Code::U7008,
        "names `Http` twice",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0\n    avoid 2.1\n"),
        Code::U7008,
        "not a full version",
    );
    has(
        &format!("{HEAD}\nneeds\n  json 2.0\n    accept name\n"),
        Code::U7008,
        "only under a path need",
    );
    assert_eq!(
        errors(&format!(
            "{HEAD}\nneeds\n  json from path \"../json\"\n    accept name\n"
        )),
        Vec::<String>::new()
    );
}

#[test]
fn sources_are_checked() {
    let one = |name: &str, owner: &str, root: &str| {
        format!("  {name} \"https://{name}.example\"\n    root \"{root}\"\n    owners {owner}\n")
    };
    has(
        &format!("{HEAD}\nsources\n{}", one("itaty", "acme", DIGEST)),
        Code::U7008,
        "reserved",
    );
    has(
        &format!("{HEAD}\nsources\n{}", one("git", "acme", DIGEST)),
        Code::U7008,
        "reserved",
    );
    has(
        &format!(
            "{HEAD}\nsources\n{}{}",
            one("a", "acme", DIGEST),
            one("b", "acme", DIGEST)
        ),
        Code::U7008,
        "served by the sources `a` and `b`",
    );
    has(
        &format!("{HEAD}\nsources\n{}", one("a", "acme", "sha256:AB")),
        Code::U7008,
        "64 lowercase",
    );
    has(
        &format!("{HEAD}\nsources\n  a \"https://a.example\"\n"),
        Code::U7008,
        "no `root` line",
    );
}

#[test]
fn trust_lines_are_checked() {
    has(
        &format!("{HEAD}\ntrust\n  acme/privacy modules privacy.keys\n"),
        Code::U7008,
        "names its major line and source",
    );
    has(
        &format!("{HEAD}\ntrust\n  acme/privacy 1.2 from itaty modules privacy.keys\n"),
        Code::U7008,
        "not a major line",
    );
    assert_eq!(
        errors(&format!(
            "{HEAD}\ntrust\n  acme/privacy 0.3 from itaty modules privacy.keys\n    may Keys[hmac]\n"
        )),
        Vec::<String>::new()
    );
    assert_eq!(
        errors(&format!(
            "{HEAD}\nneeds\n  acme/privacy 1.0\n\ntrust\n  acme/privacy modules privacy.keys\n"
        )),
        Vec::<String>::new()
    );
}

#[test]
fn unsupported_settings_are_refused_not_ignored() {
    has(
        &format!(
            "{HEAD}\nrun\n  grants\n    http \"https://a.example\" get\n      at most 1 per minute\n"
        ),
        Code::U7008,
        "limits on grants",
    );
    has(
        &format!("{HEAD}\ndrive\n  module main\n  ticks 1\n"),
        Code::U7008,
        "`drive` section",
    );
}

#[test]
fn boundaries_keep_their_rules_with_u7008() {
    let text = format!("{HEAD}\ntrusted\n  a.b\n\nboundaries\n  a\n  a.c\n\nrun\n  entry a.d\n");
    let found = errors(&text);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().all(|e| e.starts_with("U7008")), "{found:?}");
    assert!(
        found
            .iter()
            .any(|e| e.contains("`boundaries` lists `a.c` inside `a`"))
    );
    assert!(found.iter().any(|e| e.contains("`a.b` is in `trusted`")));
    assert!(found.iter().any(|e| e.contains("the `run entry` `a.d`")));
}

#[test]
fn the_lock_reads_and_meets_the_manifest() {
    let manifest = parse_ita_manifest(&format!("{HEAD}\nneeds\n  json 2.0\n"))
        .0
        .ita
        .unwrap();
    let lock_text = format!(
        "{LOCK}\npackage http 1.4.2\n  from itaty\n  digest \"{DIGEST}\"\n\npackage json 2.1.0\n  from itaty\n  digest \"{DIGEST}\"\n  needs itaty text 1.0.0\n\npackage text 1.0.3\n  from itaty\n  digest \"{DIGEST}\"\n"
    );
    let (lock, errors) = parse_ita_lock(&lock_text);
    assert_eq!(render(&errors), Vec::<String>::new());
    assert_eq!(lock.solver.step_budget, 5_000_000);
    let consistency = check_lock_consistency(&manifest, &lock, &std::collections::BTreeMap::new());
    assert_eq!(render(&consistency.in_manifest), Vec::<String>::new());
    // `http` is locked and reached by nothing.
    assert_eq!(
        render(&consistency.in_lock),
        ["U7009 `http 1.4.2` is locked but no need reaches it; run `una ita lock`"]
    );
}

#[test]
fn the_lock_is_checked() {
    let check = |extra: &str, part: &str| {
        let (_, errors) = parse_ita_lock(&format!("{LOCK}{extra}"));
        let found = render(&errors);
        assert!(
            found
                .iter()
                .any(|e| e.starts_with("U7009") && e.contains(part)),
            "{extra:?}: expected {part:?}, got {found:?}"
        );
    };
    check(
        &format!(
            "\npackage b 1.0.0\n  from itaty\n  digest \"{DIGEST}\"\n\npackage a 1.0.0\n  from itaty\n  digest \"{DIGEST}\"\n"
        ),
        "sorted by name",
    );
    check(
        "\npackage a 1.0.0\n  from itaty\n  digest \"sha256:x\"\n",
        "64 lowercase",
    );
    check(
        &format!("\npackage a 1.0.0\n  from acme\n  digest \"{DIGEST}\"\n"),
        "no registry of the lock",
    );
    check(
        &format!("\npackage a 1.0\n  from itaty\n  digest \"{DIGEST}\"\n"),
        "full version",
    );
    let (_, errors) = parse_ita_lock(&LOCK.replace("una-smt-1", "una-smt-0"));
    assert!(render(&errors)[0].contains("`encoding` must be \"una-smt-1\""));
    let (_, errors) = parse_ita_lock(&LOCK.replace("steps 5000000", "steps 0"));
    assert!(render(&errors)[0].contains("`steps` must be 1 or more"));
}

#[test]
fn workspaces_are_checked() {
    let (_, errors) =
        parse_workspace("itaty\n  members\n    \"a\"\n    \"a/\"\n    \"../b\"\n    \"/c\"\n");
    let found = render(&errors);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found[0].contains("listed twice"));
    let (workspace, errors) =
        parse_workspace("itaty\n  members\n    \"store\"\n    \"tools/report\"\n");
    assert_eq!(render(&errors), Vec::<String>::new());
    assert_eq!(workspace.members.len(), 2);
}

/// Every check `una.toml` makes has its twin in `ita.una` (ADR 0122 rules 11 and 13): the same
/// mistake written in each format is refused by each, with U7002 in `una.toml` and U7008 in
/// `ita.una`, or with the same code of another domain. The checks the checker makes against the
/// modules (a boundary, a contract or an entry naming no module) have their twins in the
/// conformance cases `project/ita_*` beside `project/boundaries_*` and `project/unknown_entry`.
#[test]
fn every_una_toml_check_has_its_ita_una_twin() {
    use una_project::parse_manifest;
    const TOML: &str = "[package]\nname = \"case\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n";
    let twins: [(&str, &str, &str, Code); 17] = [
        (
            "an unknown table",
            "[extra]\nkey = 1\n",
            "\nextra\n  key\n",
            Code::U7008,
        ),
        (
            "an unknown key of the head",
            "",
            "  color \"red\"\n",
            Code::U7008,
        ),
        (
            "a syntax error",
            "[labels\n",
            "\nlabels\n  [\n",
            Code::U7008,
        ),
        (
            "a category not UpperCamelCase",
            "[labels]\ncategories = [\"Bad_name\"]\n",
            "\nlabels\n  Bad_name\n",
            Code::U1005,
        ),
        (
            "a category twice",
            "[labels]\ncategories = [\"PII\", \"PII\"]\n",
            "\nlabels\n  PII\n  PII\n",
            Code::U1002,
        ),
        (
            "a trusted module that is not a path",
            "[trusted]\nmodules = [\"A\"]\n",
            "\ntrusted\n  A\n",
            Code::U7008,
        ),
        (
            "a negative escape budget",
            "[escape_budget]\nchecked_per_module = -1\n",
            "\nescapes\n  checked per module -1\n",
            Code::U7008,
        ),
        (
            "an entry that is not a path",
            "[run]\nentry = \"A\"\n",
            "\nrun\n  entry A\n",
            Code::U7008,
        ),
        (
            "an unknown grant",
            "[run]\ngrants = [\"teleport\"]\n",
            "\nrun\n  grants\n    teleport\n",
            Code::U7008,
        ),
        (
            "a malformed file grant",
            "[run]\ngrants = [\"files:data:exec\"]\n",
            "\nrun\n  grants\n    files \"data\" exec\n",
            Code::U7008,
        ),
        (
            "a malformed http grant",
            "[run]\ngrants = [\"http:ftp://x\"]\n",
            "\nrun\n  grants\n    http \"ftp://x\"\n",
            Code::U7008,
        ),
        (
            "a ceiling without operations",
            "[boundaries]\n\"gallery\" = [\"Files\"]\n",
            "\nboundaries\n  gallery\n    may Files\n",
            Code::U7008,
        ),
        (
            "a ceiling naming a capability twice",
            "[boundaries]\n\"gallery\" = [\"Files[read]\", \"Files[write]\"]\n",
            "\nboundaries\n  gallery\n    may Files[read]\n    may Files[write]\n",
            Code::U7008,
        ),
        (
            "a boundary inside another",
            "[boundaries]\n\"a\" = []\n\"a.b\" = []\n",
            "\nboundaries\n  a\n  a.b\n",
            Code::U7008,
        ),
        (
            "a trusted module inside a boundary",
            "[trusted]\nmodules = [\"a.b\"]\n\n[boundaries]\n\"a\" = []\n",
            "\ntrusted\n  a.b\n\nboundaries\n  a\n",
            Code::U7008,
        ),
        (
            "the entry inside a boundary",
            "[boundaries]\n\"a\" = []\n\n[run]\nentry = \"a.main\"\n",
            "\nboundaries\n  a\n\nrun\n  entry a.main\n",
            Code::U7008,
        ),
        (
            "an operation named twice",
            "[boundaries]\n\"gallery\" = [\"Files[read, read]\"]\n",
            "\nboundaries\n  gallery\n    may Files[read, read]\n",
            Code::U7008,
        ),
    ];
    for (what, toml, ita, code) in twins {
        let toml_code = if code == Code::U7008 {
            Code::U7002
        } else {
            code
        };
        let toml_text = if toml.is_empty() {
            format!("{}color = \"red\"\n", TOML)
        } else {
            format!("{TOML}{toml}")
        };
        let (_, toml_errors) = parse_manifest(&toml_text);
        assert!(
            toml_errors.iter().any(|e| e.code == toml_code),
            "una.toml, {what}: {:?}",
            render(&toml_errors)
        );
        let (_, ita_errors) = parse_ita_manifest(&format!("{HEAD}{ita}"));
        assert!(
            ita_errors.iter().any(|e| e.code == code),
            "ita.una, {what}: {:?}",
            render(&ita_errors)
        );
    }
}
