//! The hunt's oracle runner never reaches a release (ADR 0115): the build that
//! `scripts/release.sh` makes, `-p una-cli -p una-rt`, resolves no `oracle` feature anywhere,
//! no manifest of the workspace but this crate's turns it on, for any target, and this crate's
//! own build does.
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::Path;
use std::process::Command;

/// `cargo tree` of `packages`, one line per package with its enabled features.
fn features(packages: &[&str]) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(&root)
        .args([
            "tree",
            "--locked",
            "--offline",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .args(["--format", "{p} [{f}]"]);
    for package in packages {
        command.args(["--package", package]);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_release_build_resolves_no_oracle_feature() {
    let tree = features(&["una-cli", "una-rt"]);
    assert!(tree.contains("una-cli v"), "{tree}");
    assert!(tree.contains("una-interp v"), "{tree}");
    for line in tree.lines() {
        assert!(
            !line.contains("oracle"),
            "the release build has an oracle feature: {line}"
        );
    }
}

/// The workspace's packages, as `cargo metadata --no-deps` reads their manifests: each with its
/// dependencies, for every target, and its features.
fn workspace_packages() -> Vec<serde_json::Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args([
            "metadata",
            "--no-deps",
            "--locked",
            "--offline",
            "--format-version",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    metadata["packages"].as_array().unwrap().clone()
}

#[test]
fn only_this_crate_turns_the_oracle_on() {
    // Every manifest of the workspace, for every target: only workspace crates can depend on
    // `una-interp` or `una-cli`, so this is every place the feature could be turned on.
    let packages = workspace_packages();
    assert!(packages.len() > 5, "{}", packages.len());
    let mut enablers = Vec::new();
    for package in &packages {
        let name = package["name"].as_str().unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            let target = dependency["name"].as_str().unwrap();
            let features = dependency["features"].as_array().unwrap();
            if (target == "una-interp" || target == "una-cli")
                && features.iter().any(|f| f.as_str() == Some("oracle"))
            {
                enablers.push(format!(
                    "{name} -> {target} ({})",
                    dependency["target"].as_str().unwrap_or("every target")
                ));
            }
        }
        // A feature of another crate that turns the oracle on through its dependencies: only
        // `una-cli`'s own `oracle` feature may, for `una-interp`'s.
        for (feature, enables) in package["features"].as_object().unwrap() {
            for item in enables.as_array().unwrap() {
                let item = item.as_str().unwrap();
                if item.ends_with("/oracle") && !(name == "una-cli" && feature == "oracle") {
                    enablers.push(format!("{name} feature {feature} -> {item}"));
                }
            }
            if feature == "default" {
                let defaults: Vec<&str> = enables
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect();
                assert!(
                    !defaults.iter().any(|d| d.contains("oracle")),
                    "{name}'s default features turn the oracle on: {defaults:?}"
                );
            }
        }
    }
    assert_eq!(
        enablers,
        ["una-oracle -> una-cli (every target)"],
        "only una-oracle may turn the oracle feature on"
    );
    // And it does, for both crates, in its own build.
    let tree = features(&["una-oracle"]);
    for crate_name in ["una-cli ", "una-interp "] {
        assert!(
            tree.lines()
                .any(|line| line.starts_with(crate_name) && line.contains("oracle")),
            "{tree}"
        );
    }
}
