//! `ita.una`, `ita.lock` and `itaty.una` read into the settings of a package (ADR 0122,
//! `spec/30`): the checks that need no other file.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_diag::Code;
use una_project::ita::{
    check_lock_consistency, parse_ita_lock, parse_ita_manifest, parse_workspace,
};
use una_project::{Edition, Format, Profile, ProjectError};
use una_syntax::ita::Text;

const HEAD: &str = "ita case\n  version 0.1.0\n  edition 0.4\n";
const LOCK: &str = "lock 1\n\nsolver\n  encoding \"una-smt-1\"\n  name \"z3\"\n  version \"5.1.0\"\n  logic \"QF_UFLIA\"\n  steps 5000000\n  seed 0\n";
const DIGEST: &str = "sha256:3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";

/// The errors of `ita.una` text as "code message".
fn errors(text: &str) -> Vec<String> {
    render(&parse_ita_manifest(Text::Written(text)).1)
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
        &format!("{HEAD}  profile kernel\n"),
        Code::U7008,
        "unknown profile `kernel`",
    );
    has(
        &format!("{HEAD}  home \"http://x\"\n"),
        Code::U7008,
        "`https://` URL",
    );
    // A name reserved for an origin is no source; any other source name is checked against
    // the workspace's `itaty.una` when the package is published (`spec/30 §2` rule 4).
    for reserved in ["path", "git", "workspace"] {
        has(
            &format!("{HEAD}  publish to {reserved}\n"),
            Code::U7008,
            "names no source",
        );
    }
    assert_eq!(
        errors(&format!("{HEAD}  profile app\n  publish to itaty\n")),
        Vec::<String>::new()
    );
    // A published package has no `sources`: its own manifest accepts the line.
    assert_eq!(
        errors(&format!("{HEAD}  publish to acme\n")),
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
fn limits_on_http_grants_are_read_and_refused_elsewhere() {
    // ADR 0117 rule 8 (slice S5): a limit line under an `http` grant is read into the grant's
    // text form, never dropped; under another grant it is refused, since the grant would
    // otherwise allow more than written.
    let (manifest, errors) = parse_ita_manifest(&format!(
        "{HEAD}\nrun\n  grants\n    http \"https://a.example\" get\n      at most 1 per minute\n"
    ));
    assert_eq!(render(&errors), Vec::<String>::new());
    assert_eq!(manifest.grants.len(), 1);
    assert!(
        manifest.grants[0].value.ends_with(";1/minute"),
        "{:?}",
        manifest.grants[0].value
    );
    has(
        &format!("{HEAD}\nrun\n  grants\n    console\n      at most 1 per minute\n"),
        Code::U7008,
        "limits apply to `http` grants only",
    );
}

#[test]
fn a_drive_section_maps_into_the_manifest_with_its_ranges() {
    // ADR 0116 rule 1, as amended for `ita.una`: the host-driven step.
    let manifest = parse_ita_manifest(&format!(
        "{HEAD}\ndrive\n  module main\n  ticks 16\n  budget 8\n"
    ))
    .0;
    let drive = manifest.drive.unwrap();
    assert_eq!(drive.module.value, "main");
    assert_eq!((drive.ticks, drive.budget), (16, Some(8)));
    has(
        &format!("{HEAD}\ndrive\n  module main\n  ticks 0\n"),
        Code::U7008,
        "`ticks` is a number of milliseconds",
    );
    has(
        &format!("{HEAD}\ndrive\n  ticks 16\n"),
        Code::U7008,
        "has no `module`",
    );
    has(
        &format!("{HEAD}\nrun\n  entry main\n\ndrive\n  module main\n  ticks 16\n"),
        Code::U7008,
        "not both",
    );
}

#[test]
fn a_serve_section_maps_into_the_manifest_with_its_ranges() {
    // ADR 0123 rule 16, `spec/31 §7` rule 1: the host-driven form of a server.
    let manifest = parse_ita_manifest(&format!(
        "{HEAD}\nserve\n  within 10000\n  module shop.api\n"
    ))
    .0;
    let serve = manifest.serve.unwrap();
    assert_eq!(serve.module.value, "shop.api");
    assert_eq!(serve.within, Some(10_000));
    let manifest = parse_ita_manifest(&format!("{HEAD}\nserve\n  module shop.api\n")).0;
    assert_eq!(manifest.serve.unwrap().within, None);
    for within in ["0", "86400001"] {
        let text = format!("{HEAD}\nserve\n  module main\n  within {within}\n");
        has(&text, Code::U7008, "`within` is a number of milliseconds");
        assert!(parse_ita_manifest(&text).0.serve.is_none(), "{within}");
    }
    assert_eq!(
        parse_ita_manifest(&format!(
            "{HEAD}\nserve\n  module main\n  within 86400000\n"
        ))
        .0
        .serve
        .unwrap()
        .within,
        Some(86_400_000)
    );
    has(
        &format!("{HEAD}\nserve\n  within 10\n"),
        Code::U7008,
        "has no `module`",
    );
    has(
        &format!("{HEAD}\nrun\n  entry main\n\nserve\n  module main\n"),
        Code::U7008,
        "a `serve` section or a `run entry`, not both",
    );
    has(
        &format!("{HEAD}\ndrive\n  module main\n  ticks 16\n\nserve\n  module main\n"),
        Code::U7008,
        "a `serve` section or a `drive` section, not both",
    );
    has(
        &format!("{HEAD}\nserve\n  module main\n  module other\n"),
        Code::U7008,
        "`module` appears twice",
    );
    has(
        &format!("{HEAD}\nserve\n  module main\n  within 5\n  within 6\n"),
        Code::U7008,
        "`within` appears twice",
    );
    has(
        &format!("{HEAD}\nserve\n  module main\n  workers 4\n"),
        Code::U7008,
        "the lines of `serve` are `module` and `within`",
    );
    has(
        &format!("{HEAD}\nserve\n  module main\n\ndrive\n  module main\n  ticks 16\n"),
        Code::U7008,
        "out of order",
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
    // A registry section's `last root` (ADR 0122, amendment of 2026-11-03): a version from 1,
    // and a digest.
    check(
        &format!(
            "\nregistry acme\n  root \"{DIGEST}\"\n  checkpoint 2 \"{DIGEST}\"\n  last root 0 \"{DIGEST}\"\n"
        ),
        "root versions start at 1",
    );
    check(
        &format!(
            "\nregistry acme\n  root \"{DIGEST}\"\n  checkpoint 2 \"{DIGEST}\"\n  last root 1 \"sha256:x\"\n"
        ),
        "64 lowercase",
    );
    check(
        &format!("\nregistry acme\n  root \"{DIGEST}\"\n  checkpoint 2 \"{DIGEST}\"\n"),
        "a `last root` line",
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
    let twins: [(&str, &str, &str, Code); 21] = [
        (
            "an unknown profile",
            "profile = \"kernel\"\n",
            "  profile kernel\n",
            Code::U7008,
        ),
        (
            "a budget of 0",
            "[run]\nmax_memory = 0\n",
            "\nrun\n  max_memory 0\n",
            Code::U7008,
        ),
        (
            "a budget above the cap",
            "[run]\nmax_memory = 4097\n",
            "\nrun\n  max_memory 4097\n",
            Code::U7008,
        ),
        (
            "a systems package with an entry and no budget",
            "profile = \"systems\"\n\n[run]\nentry = \"main\"\n",
            "  profile systems\n\nrun\n  entry main\n",
            Code::U7008,
        ),
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

/// `serve` grants (`spec/31 §2`): read into their text spec, every problem U7008 at its item
/// line or at the grant, and the later grant of an overlap refused.
#[test]
fn serve_grants_are_checked_item_by_item() {
    let text = format!(
        "{HEAD}\nrun\n  grants\n    serve \"https://[::1]:8443\"\n      certificate \"c.pem\"\n      key \"k.pem\"\n      body at most 64 KiB\n    serve \"http://127.0.0.1:8080\"\n"
    );
    let (manifest, errors) = parse_ita_manifest(&text);
    assert_eq!(render(&errors), Vec::<String>::new());
    let grants: Vec<&str> = manifest.grants.iter().map(|g| g.value.as_str()).collect();
    assert_eq!(
        grants,
        [
            "serve:https://[::1]:8443;certificate=c.pem;key=k.pem;body=64",
            "serve:http://127.0.0.1:8080"
        ]
    );
    let bad = format!(
        "{HEAD}\nrun\n  grants\n    serve \"http://127.0.0.1:8080\"\n      body at most 2000000 KiB\n      tls12\n    serve \"https://127.0.0.1:8081\"\n      key \"k.pem\"\n    serve \"http://0.0.0.0:8080\"\n    serve \"http://127.0.0.1:8082\"\n    serve \"http://127.0.0.1:8082\"\n"
    );
    let (manifest, errors) = parse_ita_manifest(&bad);
    let located: Vec<(String, usize)> = errors
        .iter()
        .map(|e| {
            (
                e.message.clone(),
                bad[..e.range.start].matches('\n').count() + 1,
            )
        })
        .collect();
    assert_eq!(
        located,
        [
            (
                "`body` is a number of KiB from 0 to 1048576, not `2000000`".to_owned(),
                8
            ),
            (
                "`tls12` is for https grants only; this grant is http".to_owned(),
                9
            ),
            ("an https grant names its `certificate` file".to_owned(), 10),
            (
                "`127.0.0.1:8082` overlaps an earlier serve grant: each address and port is granted once, and a wildcard shares its port with no grant of its family".to_owned(),
                14
            ),
        ]
    );
    assert!(errors.iter().all(|e| e.code == Code::U7008));
    // `0.0.0.0:8080` would overlap the first grant, which is refused, so it is kept.
    let kept: Vec<&str> = manifest.grants.iter().map(|g| g.value.as_str()).collect();
    assert_eq!(
        kept,
        [
            "serve:http://0.0.0.0:8080",
            "serve:http://127.0.0.1:8082",
            "serve:http://127.0.0.1:8082"
        ]
    );
}

#[test]
fn a_systems_package_declares_its_budget_in_ita_una() {
    // ADR 0109 rules 1 and 4 under ADR 0122: `profile systems` is accepted, `max_memory` is
    // from 1 to 4096, and a `systems` package with a `run entry` declares it, at the entry.
    let text = format!("{HEAD}  profile systems\n\nrun\n  entry main\n  max_memory 64\n");
    let (manifest, found) = parse_ita_manifest(&text);
    assert_eq!(render(&found), Vec::<String>::new());
    assert_eq!(manifest.profile, Profile::Systems);
    assert_eq!(manifest.max_memory, Some(64));
    let (manifest, found) = parse_ita_manifest(&format!("{HEAD}  profile app\n"));
    assert_eq!(render(&found), Vec::<String>::new());
    assert_eq!(manifest.profile, Profile::App);
    assert_eq!(manifest.max_memory, None);
    // Without an entry, or as `app`, no budget is needed.
    for text in [
        format!("{HEAD}  profile systems\n"),
        format!("{HEAD}\nrun\n  entry main\n"),
    ] {
        assert_eq!(errors(&text), Vec::<String>::new(), "{text}");
    }
    let text = format!("{HEAD}  profile systems\n\nrun\n  entry main\n");
    let (_, found) = parse_ita_manifest(&text);
    assert_eq!(
        render(&found),
        [
            "U7008 a `systems` package with a `run entry` declares `run max_memory`, its memory budget in MiB"
        ]
    );
    assert_eq!(&text[found[0].range.clone()], "main");
    for bad in ["0", "4097"] {
        let text = format!("{HEAD}  profile systems\n\nrun\n  entry main\n  max_memory {bad}\n");
        let (manifest, found) = parse_ita_manifest(&text);
        assert_eq!(
            render(&found),
            [format!(
                "U7008 `run max_memory` is a number of MiB from 1 to 4096, not {bad}"
            )]
        );
        assert_eq!(&text[found[0].range.clone()], bad);
        assert_eq!(manifest.max_memory, None);
    }
}

#[test]
fn edition_0_5_is_named_in_ita_una() {
    // ADR 0139 rule 6: edition 0.5, a preview after 0.4, is named in `ita.una` as 0.4 is.
    let (manifest, errors) = parse_ita_manifest("ita case\n  version 0.1.0\n  edition 0.5\n");
    assert_eq!(render(&errors), Vec::<String>::new());
    assert_eq!(manifest.edition, Edition::V0_5);
    assert!(Edition::V0_5.has_collections_and_numerics());
    assert!(!Edition::V0_4.has_collections_and_numerics());
    has(
        "ita case\n  version 0.1.0\n  edition 0.6\n",
        Code::U7008,
        "edition `0.6` is not supported",
    );
}

/// What the `ita_lock` fuzz target checks (#997), and that a package's name and version name
/// the bytes they are read from: every range lies within the text as given (#2249).
fn ranges_lie_within(text: &str) {
    let (lock, errors) = parse_ita_lock(Text::Written(text));
    for error in &errors {
        assert!(
            error.range.start <= error.range.end && error.range.end <= text.len(),
            "{text:?}: {error:?}"
        );
    }
    for package in &lock.read.packages {
        assert!(text.get(package.range.clone()).is_some(), "{text:?}");
        assert_eq!(
            text.get(package.name.range.clone()),
            Some(package.name.value.to_string().as_str()),
            "{text:?}"
        );
        assert_eq!(
            text.get(package.version.range.clone()),
            Some(package.version.value.as_str()),
            "{text:?}"
        );
    }
}

/// The input the `ita_lock` fuzz target found (#2249): a `\r\n` line end, then an invalid UTF-8
/// byte before a `package` line. The ranges indexed the decoded text, one byte shorter per
/// `\r\n`, so the package's range began inside the U+FFFD. Its variants: `\r\n` line ends
/// throughout, invalid bytes before and inside a package line.
#[test]
fn the_lock_ranges_index_the_text_as_given() {
    let found = b"lock\nsolver\r\n\xffpackage r 0.3";
    assert_eq!(found.len(), 27);
    let text = String::from_utf8_lossy(found);
    ranges_lie_within(&text);
    let (lock, _) = parse_ita_lock(&text);
    assert_eq!(lock.read.packages.len(), 1);
    assert_eq!(&text[lock.read.packages[0].range.clone()], "package r 0.3");
    let package = format!("\npackage json 2.1.0\n  from itaty\n  digest \"{DIGEST}\"\n");
    let variants: Vec<Vec<u8>> = vec![
        format!("{LOCK}{package}")
            .replace('\n', "\r\n")
            .into_bytes(),
        format!("{LOCK}\r\n\u{FFFD}{package}").into_bytes(),
        [
            LOCK.replace('\n', "\r\n").as_bytes(),
            b"\xff\xfe",
            package.as_bytes(),
        ]
        .concat(),
        [
            LOCK.as_bytes(),
            b"\r\npackage js\xffon 2.1.0\r\n  from itaty\r\n",
        ]
        .concat(),
        [LOCK.as_bytes(), b"\r\npackage json 2.\xe2\x821.0\r\n"].concat(),
        b"lock 1\r\n\r\nsolver\r\n\r\n\xc3package a 1.0.0\r\npackage b 1.0.0".to_vec(),
        b"\r\n\r\nlock\r\nsolver\r\npackage\r\n\xff".to_vec(),
    ];
    for bytes in variants {
        ranges_lie_within(&String::from_utf8_lossy(&bytes));
    }
    // With `\r\n` line ends throughout, the lock reads as with `\n` alone.
    let lf = format!("{LOCK}{package}");
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(
        render(&parse_ita_lock(&crlf).1),
        render(&parse_ita_lock(&lf).1)
    );
    assert_eq!(parse_ita_lock(&crlf).0.solver, parse_ita_lock(&lf).0.solver);
}
