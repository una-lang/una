//! The bundle's reviewed feature claims stay connected to executable conformance
//! packages. The existing conformance runner owns their runtime expectations.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_catalog_and_its_negative_controls_pass() {
    for script in ["scripts/site-features-test.py", "scripts/site-features.py"] {
        let mut command = Command::new("python3");
        command.arg("-B").arg(root().join(script));
        if script.ends_with("site-features.py") {
            let version = Command::new(env!("CARGO_BIN_EXE_una"))
                .arg("--version")
                .output()
                .unwrap();
            assert!(version.status.success());
            let version = String::from_utf8(version.stdout).unwrap();
            let editions = version
                .lines()
                .find_map(|line| line.strip_prefix("editions: "))
                .unwrap();
            command
                .arg("--editions")
                .arg(editions)
                .arg("--una")
                .arg(env!("CARGO_BIN_EXE_una"));
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{script}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
