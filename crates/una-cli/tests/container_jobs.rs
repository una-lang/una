//! `scripts/container-jobs.sh` (#701, `docs/release-gates.md`): the job count of the build
//! inside a Linux container of `scripts/local-ci.sh`. It is one job per 2 GiB of the Docker VM's
//! memory, at most one per CPU and at least one, and a host value can lower it but never raise it.
//! Also the user those containers run as (#1394): never root, since the solver child refuses root
//! (ADR 0124 rule 7), with `scripts/container-volumes.sh` handing the volumes to it first.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

/// The VM of the measurement in `docs/release-gates.md`: 7834 MiB, 10 CPUs.
const MEMORY: &str = "8214851584";
const GIB: u64 = 1 << 30;

/// The script's exit code, the job count and the reason it prints.
fn jobs(args: &[&str]) -> (Option<i32>, String, String) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/container-jobs.sh");
    let output = Command::new("sh").arg(script).args(args).output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut lines = stdout.lines();
    let count = lines.next().unwrap_or_default().to_owned();
    let why = lines.next().unwrap_or_default().to_owned();
    (output.status.code(), count, why)
}

#[test]
fn unset_takes_one_job_per_2_gib_at_most_one_per_cpu() {
    assert_eq!(jobs(&[MEMORY, "10"]).1, "3");
    assert_eq!(jobs(&[&(64 * GIB).to_string(), "10"]).1, "10");
    assert_eq!(jobs(&[&GIB.to_string(), "10"]).1, "1");
    let (code, _, why) = jobs(&[MEMORY, "10"]);
    assert_eq!(code, Some(0));
    assert!(why.contains("one job per 2 GiB"), "{why}");
}

#[test]
fn a_lower_host_value_lowers_the_count() {
    let (code, count, why) = jobs(&[MEMORY, "10", "2"]);
    assert_eq!((code, count.as_str()), (Some(0), "2"));
    assert!(why.contains("CARGO_BUILD_JOBS=2 from the host"), "{why}");
}

#[test]
fn a_higher_host_value_is_capped_to_the_memory() {
    let (code, count, why) = jobs(&[MEMORY, "10", "6"]);
    assert_eq!((code, count.as_str()), (Some(0), "3"));
    assert!(
        why.contains("CARGO_BUILD_JOBS=6 on the host, capped"),
        "{why}"
    );
}

#[test]
fn a_host_value_or_a_vm_size_that_is_not_a_positive_number_is_an_error() {
    for args in [
        vec![MEMORY, "10", "0"],
        vec![MEMORY, "10", "00"],
        vec![MEMORY, "10", "x"],
        vec!["", "10"],
        vec![MEMORY, ""],
    ] {
        assert_eq!(jobs(&args).0, Some(2), "{args:?}");
    }
}

/// A file of the repository, by its path from the root.
fn repository_file(path: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .join(path),
    )
    .unwrap()
}

#[test]
fn the_linux_image_runs_as_an_unprivileged_user() {
    // The image of the tests and the release image that builds the Linux archive (#2105).
    for file in [
        "scripts/local-ci/Dockerfile",
        "scripts/local-ci/Dockerfile.release",
    ] {
        let dockerfile = repository_file(file);
        let users: Vec<&str> = dockerfile
            .lines()
            .filter_map(|line| line.trim().strip_prefix("USER "))
            .collect();
        assert_eq!(
            users.last().copied(),
            Some("1000:1000"),
            "{file}: {users:?}"
        );
    }
    // The unprivileged run of scripts/local-ci.sh names the user itself too.
    let local_ci = repository_file("scripts/local-ci.sh");
    let run = local_ci.find("\"$image\" scripts/local-ci.sh").unwrap();
    let start = local_ci[..run].rfind("docker run").unwrap();
    assert!(local_ci[start..run].contains("--user 1000:1000"));
    let pack = local_ci
        .find("\"$release_image\" scripts/local-ci.sh --release-pack")
        .unwrap();
    let start = local_ci[..pack].rfind("docker run").unwrap();
    assert!(local_ci[start..pack].contains("--user 1000:1000"));
    for script in [
        "scripts/local-ci.sh",
        "scripts/hunt-linux.sh",
        "scripts/local-release.sh",
    ] {
        assert!(
            repository_file(script).contains("container-volumes.sh"),
            "{script}"
        );
    }
}

#[test]
fn the_volume_handover_refuses_a_pair_without_a_mount_point() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/container-volumes.sh");
    for args in [
        &["linux/arm64", "image"][..],
        &["linux/arm64", "image", "volume"],
        &["p", "i", "v:"],
    ] {
        let output = Command::new("sh").arg(&script).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
