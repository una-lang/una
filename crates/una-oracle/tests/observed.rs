//! The observed run of `una-oracle` (ADR 0115): the same run as `una --json run`, every value
//! shown and each declassification recorded; `una --json run` itself unchanged.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A package whose `main` sends a secret to an operation accepting `secret`, then releases it
/// to `public` through a policy and prints it.
fn package(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/app")).unwrap();
    std::fs::copy(
        root().join("verification.lock"),
        dir.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(
        dir.join("una.toml"),
        "[package]\nname = \"observed\"\nversion = \"0.1.0\"\nedition = \"0.2\"\n\n\
         [trusted]\nmodules = [\"app.release\"]\n\n\
         [run]\nentry = \"app.main\"\ngrants = [\"console\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/app/release.una"),
        "trusted module app.release\n\n\
         pub policy lower\n  from secret Int\n  to public Int\n  via reveal\n\
         \x20 purpose \"oracle test\"\n  owner hunt\n\n\
         reveal(n: secret Int) -> public Int = n\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/app/main.una"),
        "module app.main\n\nimport app.release (lower)\n\n\
         capability Vault\n  op note(x: Int) -> Unit\n    accepts secret\n\n\
         handler quiet() for Vault\n  op note(x) = unit\n\n\
         send(n: secret Int) -> Unit\n  uses @vault: Vault[note]\n=\n  @vault.note(n)\n\n\
         pub main() -> Unit\n  uses @console: Console[print]\n=\n\
         \x20 let s: secret Int = 5\n  send(s) with @vault = quiet()\n\
         \x20 let p: public Int = lower(s)\n  @console.print(\"{p}\")\n",
    )
    .unwrap();
    dir
}

/// `una-oracle --json run .` in `dir`, with `marker` as `UNA_HUNT_RUN` when given.
fn oracle(dir: &Path, marker: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_una-oracle"));
    command.args(["--json", "run", "."]).current_dir(dir);
    command.env_remove("UNA_HUNT_RUN");
    if let Some(value) = marker {
        command.env("UNA_HUNT_RUN", value);
    }
    command.output().unwrap()
}

/// `una --json run .` in `dir`, through the library, as the `una` binary runs it.
fn una_run(dir: &Path) -> serde_json::Value {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let path = dir.to_string_lossy().into_owned();
    let status = una_cli::run(
        ["una", "--json", "run", path.as_str()],
        &mut stdout,
        &mut stderr,
        false,
        &|_| Ok(None),
        una_cli::Heap {
            held: &|| 0,
            ceiling: &|_| {},
        },
    );
    assert_eq!(status.code(), 0, "{}", String::from_utf8_lossy(&stderr));
    serde_json::from_slice(&stdout).unwrap()
}

#[test]
fn it_refuses_to_run_without_the_hunt_s_marker() {
    let dir = package("observed-refused");
    for marker in [None, Some("0"), Some("")] {
        let output = oracle(&dir, marker);
        assert_eq!(output.status.code(), Some(2), "{marker:?}");
        assert!(output.stdout.is_empty(), "{marker:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("prints labeled values"), "{stderr}");
    }
}

#[test]
fn it_takes_only_the_arguments_of_una_json_run() {
    let dir = package("observed-usage");
    for args in [
        &["run", "."][..],
        &["--json", "check", "."],
        &["--json", "test", "."],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_una-oracle"))
            .args(args)
            .current_dir(&dir)
            .env("UNA_HUNT_RUN", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn an_observed_run_shows_every_value_and_records_each_release() {
    let dir = package("observed-run");
    let output = oracle(&dir, Some("1"));
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let observed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let plain = una_run(&dir);

    // `una --json run` redacts the secret argument, as before.
    assert_eq!(plain["schema"], "una.run/1");
    assert_eq!(plain["trace"][0]["args"]["x"], "<redacted:secret>");
    assert!(plain.get("releases").is_none());

    // The observed run shows it, and records the release, after the one event.
    assert_eq!(observed["schema"], "una.oracle-run/1");
    assert_eq!(observed["trace"][0]["args"]["x"], "5");
    assert_eq!(
        observed["releases"],
        serde_json::json!([{
            "after": 1,
            "policy": "app.release.lower",
            "from": "secret",
            "to": "public",
            "value": "5",
        }])
    );

    // Everything else is the run's own: status, output, diagnostics, and the trace but for its
    // values.
    for key in ["tier", "status", "output", "failure", "diagnostics"] {
        assert_eq!(observed[key], plain[key], "{key}");
    }
    let events = |doc: &serde_json::Value| -> Vec<serde_json::Value> {
        doc["trace"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                let mut event = event.clone();
                event.as_object_mut().unwrap().remove("args");
                event
            })
            .collect()
    };
    assert_eq!(events(&observed), events(&plain));
    // Its keys come in `una.run/1`'s order, then `releases`.
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.starts_with(r#"{"schema":"una.oracle-run/1","tier":"#),
        "{text}"
    );
    let trace = text.find(r#""trace":"#).unwrap();
    let releases = text.find(r#""releases":"#).unwrap();
    assert!(trace < releases, "{text}");
}
