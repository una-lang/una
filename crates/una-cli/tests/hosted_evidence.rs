//! `scripts/hosted-evidence.sh` (#2106, ADR 0124 rule 31 as its amendment of 2026-11-27 reads
//! it, `docs/release-gates.md` §2 "The hosted x86-64 evidence"): the gate machine takes the
//! `linux` and `limits-linux` gates of `x86_64-unknown-linux-gnu` from the hosted runner's
//! artifact of the gated commit, and passes only on a complete, untampered artifact of that
//! commit from a native x86-64 machine. A stand-in `gh` serves the run and the artifact, so the
//! test reads nothing from GitHub.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "fedcba9876543210fedcba9876543210fedcba98";

/// Writes an artifact of `leg` on `commit` into `dir`, as scripts/hosted-x86-64.sh writes one,
/// with the manifest lines `extra` appended or put in place of their key, then `tamper` run in
/// `dir` after the digests are written.
fn artifact(dir: &Path, leg: &str, commit: &str, manifest: &[(&str, &str)], tamper: &str) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let mut keys = vec![
        ("leg", leg.to_owned()),
        ("commit", commit.to_owned()),
        ("result", "PASS".to_owned()),
        ("image", "ubuntu24 20261117.1".to_owned()),
        (
            "kernel",
            "Linux 6.11.0-1018-azure #18~24.04.1-Ubuntu SMP x86_64".to_owned(),
        ),
        ("cpu", "AMD EPYC 7763 64-Core Processor".to_owned()),
        ("cpus", "4".to_owned()),
        ("emulation", "native".to_owned()),
        ("docker_architecture", "x86_64".to_owned()),
        (
            "run",
            "https://github.com/una-lang/una/actions/runs/7 attempt 1".to_owned(),
        ),
    ];
    for (key, value) in manifest {
        match keys.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = (*value).to_owned(),
            None => keys.push((key, (*value).to_owned())),
        }
    }
    let lines: String = keys
        .iter()
        .filter(|(_, value)| value != "<none>")
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect();
    let script = r#"
        set -e
        cd "$1"
        leg=$2 commit=$3
        sum() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
        printf '== the leg\nhosted x86-64: PASS (%s on %s)\n' "$leg" "$commit" > "$leg.log"
        printf '%s' "$4" > "$leg.manifest"
        if [ "$leg" = archive ]; then
            name=una-9.9.9-x86_64-unknown-linux-gnu.tar.gz
            printf 'archive' > "$name"
            sum "$name" > "$name.sha256"
            printf 'archive=%s\narchive_sha256=%s\n' "$name" "$(sum "$name" | cut -d ' ' -f 1)" >> "$leg.manifest"
        fi
        grep -q '^log_sha256=' "$leg.manifest" \
            || echo "log_sha256=$(sum "$leg.log" | cut -d ' ' -f 1)" >> "$leg.manifest"
        sum "$leg.log" "$leg.manifest" > SHA256SUMS
        eval "$5"
    "#;
    let status = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(dir)
        .arg(leg)
        .arg(commit)
        .arg(lines)
        .arg(tamper)
        .status()
        .unwrap();
    assert!(status.success());
}

/// One run of the API's listing of a workflow's runs.
fn run_json(id: u64, event: &str, sha: &str, conclusion: &str, attempt: u32) -> String {
    format!(
        r#"{{"id":{id},"event":"{event}","head_sha":"{sha}","status":"completed","conclusion":"{conclusion}","run_attempt":{attempt},"html_url":"https://github.com/una-lang/una/actions/runs/{id}"}}"#
    )
}

/// The API's listing of the runs `runs`, newest first.
fn listing(runs: &[String]) -> String {
    format!(
        r#"{{"total_count":{},"workflow_runs":[{}]}}"#,
        runs.len(),
        runs.join(",")
    )
}

/// The API's listing of a run's jobs: the four legs, `leg` with `conclusion`, `group` and
/// `labels`, the others passed on a hosted runner.
fn jobs_json(leg: &str, conclusion: &str, group: &str, labels: &str) -> String {
    let jobs: Vec<String> = ["z3-library (one)", "linux", "limits-linux", "archive"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let (conclusion, group, labels) = if *name == leg {
                (conclusion, group, labels)
            } else {
                ("success", "GitHub Actions", r#"["ubuntu-24.04"]"#)
            };
            format!(
                r#"{{"id":{i},"name":"{name}","conclusion":"{conclusion}","runner_group_name":"{group}","labels":{labels},"runner_name":"GitHub Actions {i}"}}"#
            )
        })
        .collect();
    format!(r#"{{"total_count":4,"jobs":[{}]}}"#, jobs.join(","))
}

/// The jobs of a run whose every leg passed on a GitHub-hosted runner.
fn hosted_jobs() -> String {
    jobs_json("linux", "success", "GitHub Actions", r#"["ubuntu-24.04"]"#)
}

/// The run that the stand-in serves an artifact of.
const RUN_ID: u64 = 7;

/// The listing of one successful dispatched run of SHA, [`RUN_ID`], attempt 1.
fn one_run() -> String {
    listing(&[run_json(RUN_ID, "workflow_dispatch", SHA, "success", 1)])
}

/// Runs scripts/hosted-evidence.sh for `leg` of SHA with a stand-in `gh`, which serves
/// `runs` for the listing of the workflow's runs of SHA (and fails any other listing), `jobs`
/// for the jobs of run [`RUN_ID`], attempt 1, and `served` as the artifact of run [`RUN_ID`]
/// only: a download of any other run fails, so the run chosen shows (#2328).
fn evidence(tmp: &Path, leg: &str, runs: &str, jobs: &str, served: &Path) -> Output {
    let gh = tmp.join("gh");
    std::fs::write(tmp.join("runs.json"), runs).unwrap();
    std::fs::write(tmp.join("jobs.json"), jobs).unwrap();
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\ncase $1:$2 in\n  api:repos/una-lang/una/actions/workflows/linux-x86-64.yml/runs\\?head_sha={SHA}\\&*) cat '{runs}' ;;\n  api:repos/una-lang/una/actions/runs/{RUN_ID}/attempts/1/jobs*) cat '{jobs}' ;;\n  run:download) [ \"$3\" = {RUN_ID} ] || {{ echo \"no artifact of run $3\" >&2; exit 1; }}\n    while [ $# -gt 0 ]; do [ \"$1\" = --dir ] && dir=$2; shift; done; cp -R '{served}'/. \"$dir\"/ ;;\n  *) echo \"unexpected: $*\" >&2; exit 9 ;;\nesac\n",
            runs = tmp.join("runs.json").display(),
            jobs = tmp.join("jobs.json").display(),
            served = served.display()
        ),
    )
    .unwrap();
    let status = Command::new("chmod").arg("755").arg(&gh).status().unwrap();
    assert!(status.success());
    Command::new(root().join("scripts/hosted-evidence.sh"))
        .arg(SHA)
        .arg(leg)
        .arg(tmp.join("out"))
        .env("UNA_HOSTED_GH", &gh)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A refused artifact: what it is, its commit, its manifest's changes, the tampering after
/// its digests, and what the refusal says.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)], &'a str, &'a str);

#[test]
fn a_complete_native_artifact_of_the_gated_commit_passes_and_is_kept() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hosted-evidence-pass");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    for leg in ["linux", "limits-linux", "archive"] {
        let served = tmp.join(format!("served-{leg}"));
        artifact(&served, leg, SHA, &[], "");
        let output = evidence(&tmp, leg, &one_run(), &hosted_jobs(), &served);
        let out = stdout(&output);
        assert_eq!(output.status.code(), Some(0), "{leg}: {out}");
        assert!(
            out.contains(&format!("hosted evidence: PASS ({leg} of {SHA}, run https://github.com/una-lang/una/actions/runs/7 attempt 1, image ubuntu24 20261117.1")),
            "{leg}: {out}"
        );
        // The runner's whole log is in the gate's output, and the files stay in OUT.
        assert!(
            out.contains(&format!("\n| hosted x86-64: PASS ({leg} on {SHA})\n")),
            "{out}"
        );
        assert!(tmp.join(format!("out/{leg}/{leg}.manifest")).is_file());
        if leg == "archive" {
            assert!(
                out.lines().any(|l| l.starts_with(
                    "hosted: archive una-9.9.9-x86_64-unknown-linux-gnu.tar.gz sha256 "
                )),
                "{out}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn any_other_commit_machine_result_or_digest_fails() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hosted-evidence-fail");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let served = tmp.join("served");
    let cases: &[Case] = &[
        (
            "another commit",
            OTHER,
            &[],
            "",
            "the manifest is of the commit",
        ),
        (
            "a failed leg",
            SHA,
            &[("result", "FAIL")],
            "",
            "the leg's result is FAIL",
        ),
        (
            "emulated",
            SHA,
            &[("emulation", "emulated")],
            "",
            "the runner was not native",
        ),
        (
            "an arm64 engine",
            SHA,
            &[("docker_architecture", "aarch64")],
            "",
            "the Docker engine was aarch64",
        ),
        (
            "an aarch64 kernel",
            SHA,
            &[("kernel", "Linux 6.8.0 #1 SMP aarch64")],
            "",
            "the kernel was",
        ),
        (
            "another image",
            SHA,
            &[("image", "self-hosted 1")],
            "",
            "not GitHub's ubuntu-24.04",
        ),
        (
            "an image with no version",
            SHA,
            &[("image", "ubuntu24 unknown")],
            "",
            "not GitHub's ubuntu-24.04 with its version",
        ),
        (
            "another run",
            SHA,
            &[(
                "run",
                "https://github.com/una-lang/una/actions/runs/8 attempt 1",
            )],
            "",
            "the manifest is of the run",
        ),
        (
            "another attempt",
            SHA,
            &[(
                "run",
                "https://github.com/una-lang/una/actions/runs/7 attempt 2",
            )],
            "",
            "the manifest is of the run",
        ),
        (
            "a planted file whose name matches a listed one as a pattern",
            SHA,
            &[],
            "echo x > archive.lo.",
            "archive.lo. is not in SHA256SUMS",
        ),
        (
            "no image",
            SHA,
            &[("image", "<none>")],
            "",
            "not GitHub's ubuntu-24.04",
        ),
        (
            "another leg",
            SHA,
            &[("leg", "limits-linux")],
            "",
            "the manifest is of the leg",
        ),
        (
            "a changed log",
            SHA,
            &[],
            "echo more >> archive.log",
            "archive.log does not match SHA256SUMS",
        ),
        (
            "a log the manifest does not vouch for",
            SHA,
            &[("log_sha256", "0000")],
            "",
            "the log does not match its manifest",
        ),
        (
            "an unlisted file",
            SHA,
            &[],
            "echo x > planted",
            "planted is not in SHA256SUMS",
        ),
        (
            "a changed archive",
            SHA,
            &[],
            "echo x >> una-9.9.9-x86_64-unknown-linux-gnu.tar.gz",
            "does not match its manifest",
        ),
        (
            "no archive",
            SHA,
            &[],
            "rm una-9.9.9-x86_64-unknown-linux-gnu.tar.gz",
            "lacks una-9.9.9",
        ),
        (
            "no sums",
            SHA,
            &[],
            "rm SHA256SUMS",
            "the artifact has no SHA256SUMS",
        ),
    ];
    for (case, commit, manifest, tamper, why) in cases {
        // The archive leg, which also carries the archive's checks. A log of another commit's
        // leg would end with that commit; the check of the manifest comes first.
        artifact(&served, "archive", commit, manifest, tamper);
        let output = evidence(&tmp, "archive", &one_run(), &hosted_jobs(), &served);
        let out = stdout(&output);
        assert_eq!(output.status.code(), Some(1), "{case}: {out}");
        assert!(
            out.contains("hosted evidence: FAIL (archive of"),
            "{case}: {out}"
        );
        assert!(out.contains(why), "{case}: {out}");
    }
    // No successful run on the commit: nothing to download, FAIL with how to make one.
    artifact(&served, "linux", SHA, &[], "");
    let output = evidence(&tmp, "linux", &listing(&[]), &hosted_jobs(), &served);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("no successful dispatched run of linux-x86-64.yml on this commit")
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_usage_error_is_refused() {
    for args in [
        &["abc", "linux", "out"][..],
        &[SHA, "z3-library", "out"][..],
        &[SHA, "linux"][..],
    ] {
        let output = Command::new(root().join("scripts/hosted-evidence.sh"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn the_gates_take_the_x86_64_legs_from_the_hosted_evidence_off_an_x86_64_engine() {
    // scripts/release-gates.sh, scripts/local-release.sh and the toolchain-free gate read the
    // hosted evidence where the Docker engine is not x86-64, and never fall back to the
    // emulated leg (#2106).
    let gates = std::fs::read_to_string(root().join("scripts/release-gates.sh")).unwrap();
    assert!(
        gates.contains("gate_linux() { hosted_or_local linux linux scripts/local-ci.sh --linux; }")
    );
    assert!(gates.contains(
        "gate_limits-linux() { hosted_or_local limits-linux limits-linux scripts/local-ci.sh --linux --limits; }"
    ));
    assert!(
        gates
            .contains("run \"$gate\" scripts/hosted-evidence.sh \"$SHA\" \"$leg\" \"$OUT/hosted\"")
    );
    assert!(gates.contains("not the archive the hosted runner checked"));
    let release = std::fs::read_to_string(root().join("scripts/local-release.sh")).unwrap();
    assert!(
        release
            .contains("\"$root/scripts/hosted-evidence.sh\" \"$commit\" archive \"$hosted_dir\"")
    );
    // The evidence outlives the run: the gates keep it under OUT/hosted, and print it whole
    // (#2327).
    assert!(release.contains("hosted_dir=${UNA_HOSTED_DIR:-"));
    assert!(release.contains("printf '%s\\n' \"$evidence\"\n"));
    assert!(gates.contains("UNA_HOSTED_DIR='$OUT/hosted/release' scripts/local-release.sh"));
    assert!(
        gates.contains("UNA_HOSTED_DIR='\"'$OUT/hosted/reproducible'\"' scripts/local-release.sh")
    );
    assert!(gates.contains("\"$SHA\" \"$OUT/hosted/toolchain-free\" $TARGETS"));
    assert!(!gates.contains("sed -n \"/^hosted: /p; /^hosted evidence: /p\""));
    assert!(release.contains("not the archive the hosted runner checked natively"));
    // The workflow runs every leg, on the exact head commit, with pinned actions and no secret.
    let workflow =
        std::fs::read_to_string(root().join(".github/workflows/linux-x86-64.yml")).unwrap();
    for needed in [
        "runs-on: ubuntu-24.04",
        "- leg: z3-library",
        "- leg: linux",
        "- leg: limits-linux",
        "- leg: archive",
        "ref: ${{ github.event.pull_request.head.sha || github.sha }}",
        "permissions:\n  contents: read",
        "scripts/hosted-x86-64.sh \"$LEG\"",
    ] {
        assert!(workflow.contains(needed), "the workflow lacks {needed}");
    }
    assert!(
        !workflow.contains("secrets."),
        "the workflow uses no secret"
    );
    for line in workflow
        .lines()
        .filter(|l| l.trim_start().starts_with("uses:"))
    {
        let pin = line.split('@').nth(1).unwrap_or("");
        let sha = pin.split_whitespace().next().unwrap_or("");
        assert!(
            sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "an action pinned by its full commit hash: {line}"
        );
    }
}

#[test]
fn only_the_newest_successful_dispatched_run_of_the_commit_counts() {
    // #2328: the listing holds, newer than the one good run (7), a failed dispatched run of
    // SHA (8), a successful dispatched run of another commit (9), an unfinished dispatched run
    // of SHA (10) and a successful pull request run of SHA (11). The stand-in serves an artifact
    // of run 7 only, so a filter removed from the script makes it choose another run and fail.
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hosted-evidence-runs");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let served = tmp.join("served");
    artifact(&served, "linux", SHA, &[], "");
    let unfinished = run_json(10, "workflow_dispatch", SHA, "success", 1)
        .replace(r#""status":"completed""#, r#""status":"in_progress""#);
    let runs = listing(&[
        run_json(11, "pull_request", SHA, "success", 1),
        unfinished,
        run_json(9, "workflow_dispatch", OTHER, "success", 1),
        run_json(8, "workflow_dispatch", SHA, "failure", 1),
        run_json(RUN_ID, "workflow_dispatch", SHA, "success", 1),
        run_json(6, "workflow_dispatch", SHA, "success", 1),
    ]);
    let output = evidence(&tmp, "linux", &runs, &hosted_jobs(), &served);
    let out = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{out}");
    assert!(
        out.contains(
            "hosted: run https://github.com/una-lang/una/actions/runs/7 (id 7, attempt 1)"
        ),
        "{out}"
    );
    // Without run 7, none counts: the newer ones are refused, and the older 6 is chosen, whose
    // artifact the stand-in does not serve.
    let runs = listing(&[
        run_json(11, "pull_request", SHA, "success", 1),
        run_json(9, "workflow_dispatch", OTHER, "success", 1),
        run_json(8, "workflow_dispatch", SHA, "failure", 1),
    ]);
    let output = evidence(&tmp, "linux", &runs, &hosted_jobs(), &served);
    let out = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{out}");
    assert!(
        out.contains("no successful dispatched run of linux-x86-64.yml on this commit"),
        "{out}"
    );
    // A listing that is not the API's JSON fails closed.
    let output = evidence(&tmp, "linux", "not json", &hosted_jobs(), &served);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(stdout(&output).contains("is not the API's JSON"));
}

#[test]
fn the_leg_must_have_passed_on_a_github_hosted_runner() {
    // #2331: the jobs API names the runner group and the labels of the job of the leg.
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hosted-evidence-jobs");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let served = tmp.join("served");
    artifact(&served, "linux", SHA, &[], "");
    for (case, jobs, why) in [
        (
            "self-hosted",
            jobs_json(
                "linux",
                "success",
                "Default",
                r#"["self-hosted","ubuntu-24.04"]"#,
            ),
            "did not pass on a GitHub-hosted ubuntu-24.04 runner",
        ),
        (
            "a runner not named as GitHub's hosted ones",
            jobs_json("linux", "success", "GitHub Actions", r#"["ubuntu-24.04"]"#).replace(
                r#""runner_name":"GitHub Actions 1""#,
                r#""runner_name":"build-box-1""#,
            ),
            "did not pass on a GitHub-hosted ubuntu-24.04 runner",
        ),
        (
            "another image",
            jobs_json("linux", "success", "GitHub Actions", r#"["ubuntu-22.04"]"#),
            "did not pass on a GitHub-hosted ubuntu-24.04 runner",
        ),
        (
            "a failed job",
            jobs_json("linux", "failure", "GitHub Actions", r#"["ubuntu-24.04"]"#),
            "did not pass on a GitHub-hosted ubuntu-24.04 runner",
        ),
        (
            "no job of the leg",
            jobs_json("other", "success", "GitHub Actions", r#"["ubuntu-24.04"]"#)
                .replace(r#""name":"linux""#, r#""name":"linux (old)""#),
            "has not exactly one job named linux",
        ),
    ] {
        let output = evidence(&tmp, "linux", &one_run(), &jobs, &served);
        let out = stdout(&output);
        assert_eq!(output.status.code(), Some(1), "{case}: {out}");
        assert!(out.contains(why), "{case}: {out}");
    }
}

#[test]
fn no_line_of_the_runners_log_reads_as_the_checkers_own() {
    // #2329: a runner log that holds a line shaped as the checker's archive line is printed
    // after `| `, so the gates, which match the checker's lines from their start, never read it.
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hosted-evidence-lines");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let served = tmp.join("served");
    let planted = "hosted: archive una-9.9.9-x86_64-unknown-linux-gnu.tar.gz sha256 0000000000000000000000000000000000000000000000000000000000000000";
    artifact(
        &served,
        "archive",
        SHA,
        &[],
        &format!(
            "printf '%s\\n' '{planted}' >> archive.log && sum() {{ if command -v sha256sum >/dev/null 2>&1; then sha256sum \"$@\"; else shasum -a 256 \"$@\"; fi; }} && sed -i.bak \"s/^log_sha256=.*/log_sha256=$(sum archive.log | cut -d ' ' -f 1)/\" archive.manifest && rm archive.manifest.bak && sum archive.log archive.manifest > SHA256SUMS"
        ),
    );
    let output = evidence(&tmp, "archive", &one_run(), &hosted_jobs(), &served);
    let out = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{out}");
    assert!(out.contains(&format!("\n| {planted}\n")), "{out}");
    assert!(!out.lines().any(|line| line == planted), "{out}");
    assert_eq!(
        out.lines()
            .filter(|line| line.starts_with("hosted: archive "))
            .count(),
        1,
        "{out}"
    );
}
