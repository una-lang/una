#!/bin/sh
# The hosted x86-64 evidence of a commit, on the gate machine (#2106, ADR 0124 rule 31 as its
# amendment of 2026-11-27 reads it, docs/release-gates.md §2 "The hosted x86-64 evidence"). It
# reads, never writes: it finds the successful dispatched run (workflow_dispatch) of
# .github/workflows/linux-x86-64.yml whose head commit is SHA, downloads the artifact of LEG (linux-x86-64-LEG-SHA) into DIR/LEG, and
# passes only when
#   - every file's SHA-256 matches the artifact's SHA256SUMS, and the log's its manifest;
#   - the manifest names LEG and SHA, with result PASS;
#   - the machine was native x86-64 Linux: kernel x86_64, emulation native, Docker engine
#     x86_64, and the runner image GitHub's ubuntu-24.04 (ImageOS ubuntu24);
#   - for `archive`, the archive the runner packed, checked and ran toolchain-free is there, and
#     matches its .sha256 file and the manifest.
# It prints the run, the manifest and then the runner's whole log, so that the gate's log holds
# it; the files stay in DIR/LEG, which the release record keeps. Anything missing, another
# commit, another digest or a failed check is FAIL (exit 1); a usage error exits 2.
#
# For `archive`, the line `hosted: archive <name> sha256 <digest>` gives the archive that a build
# on the gate machine must equal (scripts/local-release.sh, the toolchain-free gate).
#
# With --native-engine alone it exits 0 when the Docker engine here is x86-64 and this shell is
# not translated by Rosetta (the native-host mode of scripts/local-ci.sh --linux, #1939), where
# the legs run here instead, and 1 otherwise.
#
# UNA_HOSTED_REPOSITORY names the repository (default una-lang/una); UNA_HOSTED_GH the gh
# command (default gh), for the tests.
#
# Usage: scripts/hosted-evidence.sh SHA LEG DIR | --native-engine
set -u

if [ "$*" = --native-engine ]; then
    engine=$(docker info --format '{{.Architecture}}' 2>/dev/null) || engine=""
    translated=$( (sysctl -n sysctl.proc_translated 2>/dev/null) || echo 0)
    [ "$engine" = x86_64 ] && [ "$translated" != 1 ]
    exit
fi
usage() {
    echo "usage: scripts/hosted-evidence.sh SHA linux|limits-linux|archive DIR | --native-engine" >&2
    exit 2
}
[ "$#" -eq 3 ] || usage
sha=$1 leg=$2 dir=$3
printf '%s' "$sha" | grep -Eqx '[0-9a-f]{40}' || usage
case $leg in linux | limits-linux | archive) ;; *) usage ;; esac
repo=${UNA_HOSTED_REPOSITORY:-una-lang/una}
gh=${UNA_HOSTED_GH:-gh}
workflow=linux-x86-64.yml
artifact=linux-x86-64-$leg-$sha

fail() {
    echo "hosted evidence: FAIL ($leg of $sha: $1)"
    exit 1
}
digest() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
    else shasum -a 256 "$1" | cut -d ' ' -f 1; fi
}

# The newest successful dispatched run of the workflow on exactly that commit, chosen here from
# the listing of the commit's runs (#2328), never by the query alone: a pull request's run takes
# its workflow file from the pull request's merge commit, which is not the gated commit's, and a
# failed or unfinished run, or another commit's, is never evidence. Prints `id attempt url`.
listing=$("$gh" api "repos/$repo/actions/workflows/$workflow/runs?head_sha=$sha&per_page=100" 2>&1) \
    || fail "listing the runs of $workflow: $listing"
chosen=$(printf '%s' "$listing" | python3 -I -c '
import json, sys
sha = sys.argv[1]
try:
    runs = json.load(sys.stdin)["workflow_runs"]
except (ValueError, KeyError, TypeError):
    sys.exit(3)
good = [r for r in runs
        if r.get("event") == "workflow_dispatch"
        and r.get("head_sha") == sha
        and r.get("status") == "completed"
        and r.get("conclusion") == "success"
        and isinstance(r.get("id"), int)
        and isinstance(r.get("run_attempt"), int)]
if good:
    run = max(good, key=lambda r: r["id"])
    print(run["id"], run["run_attempt"], run.get("html_url", ""))
' "$sha") || fail "the listing of the runs of $workflow is not the API's JSON"
run=${chosen%% *}
rest=${chosen#* }
attempt=${rest%% *}
url=${rest#* }
printf '%s' "$run" | grep -Eqx '[0-9]+' \
    || fail "no successful dispatched run of $workflow on this commit; dispatch it on the branch whose head is $sha (gh workflow run $workflow --ref main) and wait for it"
[ "$url" = "https://github.com/$repo/actions/runs/$run" ] || fail "the run $run has the URL $url"
echo "hosted: run $url (id $run, attempt $attempt), workflow $workflow, head $sha"

# The job of the leg ran on a GitHub-hosted runner (#2331, #2336). What carries that is the
# runner group, `GitHub Actions` for GitHub's hosted runners, and the runner's name,
# `GitHub Actions <number>` for them; the labels are only what the job asked for in `runs-on`
# (the gated commit's own workflow), so their check is one of consistency, not of the machine.
# None of this is authentication: it binds the evidence to what GitHub's API says ran it.
jobs=$("$gh" api "repos/$repo/actions/runs/$run/attempts/$attempt/jobs?per_page=100" 2>&1) \
    || fail "listing the jobs of run $run: $jobs"
hosted=$(printf '%s' "$jobs" | python3 -I -c '
import json, re, sys
leg = sys.argv[1]
try:
    jobs = json.load(sys.stdin)["jobs"]
except (ValueError, KeyError, TypeError):
    sys.exit(3)
mine = [j for j in jobs if j.get("name") == leg]
if len(mine) != 1:
    sys.exit(4)
job = mine[0]
if job.get("conclusion") != "success" or job.get("runner_group_name") != "GitHub Actions" \
        or job.get("labels") != ["ubuntu-24.04"] \
        or not re.fullmatch(r"GitHub Actions [0-9]+", str(job.get("runner_name"))):
    print("job", job.get("id"), "conclusion", job.get("conclusion"), "runner group",
          job.get("runner_group_name"), "runner", job.get("runner_name"), "labels", job.get("labels"))
    sys.exit(5)
print(job.get("id"), job.get("runner_name", ""))
' "$leg")
case $? in
    0) echo "hosted: job $hosted, a GitHub-hosted ubuntu-24.04 runner" ;;
    4) fail "run $run has not exactly one job named $leg" ;;
    5) fail "the job of $leg did not pass on a GitHub-hosted ubuntu-24.04 runner: $hosted" ;;
    *) fail "the jobs of run $run are not the API's JSON" ;;
esac

target=$dir/$leg
rm -rf "${target:?}"
mkdir -p "$target" || fail "making $target"
"$gh" run download "$run" --repo "$repo" --name "$artifact" --dir "$target" \
    || fail "downloading the artifact $artifact of run $run"

[ -f "$target/SHA256SUMS" ] || fail "the artifact has no SHA256SUMS"
[ -f "$target/$leg.manifest" ] && [ -f "$target/$leg.log" ] || fail "the artifact has no $leg.manifest or $leg.log"
# Every file the sums list, each matching; and no file they do not list, but the archive's.
while read -r want name; do
    case $name in */* | '' | .*) fail "SHA256SUMS lists $name" ;; esac
    [ -f "$target/$name" ] || fail "SHA256SUMS lists $name, which the artifact lacks"
    [ "$(digest "$target/$name")" = "$want" ] || fail "$name does not match SHA256SUMS"
done < "$target/SHA256SUMS"
for file in "$target"/*; do
    name=${file##*/}
    case $name in SHA256SUMS | *.tar.gz | *.tar.gz.sha256) continue ;; esac
    awk -v n="$name" '$2 == n { found = 1 } END { exit !found }' "$target/SHA256SUMS" \
        || fail "$name is not in SHA256SUMS"
done

value() { sed -n "s/^$1=//p" "$target/$leg.manifest" | head -n 1; }
echo "== the manifest"
cat "$target/$leg.manifest"
[ "$(value leg)" = "$leg" ] || fail "the manifest is of the leg $(value leg)"
[ "$(value commit)" = "$sha" ] || fail "the manifest is of the commit $(value commit)"
[ "$(value result)" = PASS ] || fail "the leg's result is $(value result)"
[ "$(value emulation)" = native ] || fail "the runner was not native: emulation $(value emulation)"
[ "$(value docker_architecture)" = x86_64 ] || fail "the Docker engine was $(value docker_architecture)"
case " $(value kernel) " in
    " Linux "*" x86_64 ") ;;
    *) fail "the kernel was $(value kernel)" ;;
esac
# GitHub's ubuntu-24.04 image and its version, such as `ubuntu24 20261122.23.1` (#2331).
printf '%s' "$(value image)" | grep -Eqx 'ubuntu24 [0-9]{8}(\.[0-9]+)+' \
    || fail "the runner image was $(value image), not GitHub's ubuntu-24.04 with its version"
[ "$(value run)" = "$url attempt $attempt" ] \
    || fail "the manifest is of the run $(value run), not $url attempt $attempt"
[ "$(value log_sha256)" = "$(digest "$target/$leg.log")" ] || fail "the log does not match its manifest"
grep -qx "hosted x86-64: PASS ($leg on $sha)" "$target/$leg.log" || fail "the log does not end the leg with PASS"

if [ "$leg" = archive ]; then
    # The trace of the link of the archive's `una`, as SHA256SUMS lists it (#2363).
    awk '$2 == "una.link-trace" { found = 1 } END { exit !found }' "$target/SHA256SUMS" \
        || fail "the artifact holds no una.link-trace that SHA256SUMS lists"
    name=$(value archive)
    case $name in
        una-*-x86_64-unknown-linux-gnu.tar.gz) ;;
        *) fail "the manifest names no archive of x86_64-unknown-linux-gnu" ;;
    esac
    [ -f "$target/$name" ] && [ -f "$target/$name.sha256" ] || fail "the artifact lacks $name or its .sha256"
    archive=$(digest "$target/$name")
    [ "$archive" = "$(value archive_sha256)" ] || fail "$name does not match its manifest"
    [ "$(cut -d ' ' -f 1 "$target/$name.sha256")" = "$archive" ] || fail "$name does not match its .sha256 file"
    echo "hosted: archive $name sha256 $archive"
fi
# The runner's log, every line after `| `, so that no line of it can read as one of this
# checker's own `hosted: ` lines (#2329).
echo "== the runner's log ($leg.log, sha256 $(value log_sha256))"
sed 's/^/| /' "$target/$leg.log"
echo "hosted evidence: PASS ($leg of $sha, run $url attempt $attempt, image $(value image), kernel $(value kernel))"
