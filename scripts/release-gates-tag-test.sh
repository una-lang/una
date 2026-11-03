#!/bin/bash
# Tests of scripts/release-gates-tag.sh (#1017): the release tag that the release and
# reproducible gates make in the gates' own clone is undone when the gate ends, whether it
# passes, fails or is stopped, and put back when the clone had it from its source; a run killed
# before undoing it is undone later; and nothing but the gates' own clone is ever changed: not a
# repository without the clone gate's marker, nor one that a link, a worktree's .git file or an
# exported GIT_DIR points at, nor the clone's source. Every repository is made here, in a fresh
# temporary directory; no other repository is read or written.
#
# Usage: scripts/release-gates-tag-test.sh
set -u

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
. "$root/scripts/release-gates-tag.sh"

work=$(mktemp -d "${TMPDIR:-/tmp}/release-gates-tag-test.XXXXXX") || exit 1
work=$(CDPATH='' cd -P -- "$work" && pwd -P)
trap 'rm -rf "${work:?}"' EXIT

# Git with no configuration of the machine's and a fixed identity, so that the test sees only
# its own repositories.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE GIT_OBJECT_DIRECTORY \
    GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_NAMESPACE

status=0 count=0
check() { # name, then a command that must succeed
    local name=$1
    shift
    count=$((count + 1))
    if ! "$@"; then
        echo "error: $name" >&2
        status=1
    fi
}
refs() { git -C "$1" for-each-ref --format='%(refname) %(objectname)'; }
tag_of() { git -C "$1" rev-parse --quiet --verify "refs/tags/$2"; }
no_tag() { ! tag_of "$1" "$2" > /dev/null; }
same() { [ "$1" = "$2" ]; }

# The source: commit A with the annotated tag v0.0.1 (a past release), then commit B, gated.
src=$work/src
git init --quiet "$src" || exit 1
git -C "$src" commit --quiet --allow-empty -m A || exit 1
git -C "$src" tag -a -m v0.0.1 v0.0.1 || exit 1
git -C "$src" commit --quiet --allow-empty -m B || exit 1
sha=$(git -C "$src" rev-parse HEAD)
released=$(tag_of "$src" v0.0.1)
before=$(refs "$src")

# The gates' clone of the source under OUT, as the clone gate makes it.
out=$work/out
clone=$out/clone
mkdir -p "$out"
git clone --quiet --no-checkout "$src" "$clone" || exit 1
git -C "$clone" checkout --quiet --detach "$sha" || exit 1
check "the clone gate's marker is written" clone_mark "$clone" "$out"
check "the gates' own clone is owned" clone_owned "$clone" "$out"
pending=$clone/.git/$RELEASE_GATES_PENDING

# A gate that passes sees the tag on the gated commit, and the tag is gone at its end.
seen=$work/seen
(cd "$clone" && with_clone_tag "$clone" "$out" v9.9.9 "$sha" sh -c 'git rev-parse "refs/tags/v9.9.9^{commit}" > "$1"' sh "$seen") > /dev/null
check "a passing gate exits 0" same "$?" 0
check "the command sees the tag on the gated commit" same "$(cat "$seen")" "$sha"
check "a passing gate's new tag is removed" no_tag "$clone" v9.9.9
check "the list of tags to undo is dropped" test ! -e "$pending"
check "the clone stays exact" same "$(git -C "$clone" status --porcelain --untracked-files=all)" ""

# A gate that fails keeps its exit code, and its tag is removed.
(cd "$clone" && with_clone_tag "$clone" "$out" v9.9.9 "$sha" sh -c 'exit 3') > /dev/null 2>&1
code=$?
check "a failing gate keeps its exit code" same "$code" 3
check "a failing gate's tag is removed" no_tag "$clone" v9.9.9

# A gate stopped by a signal (TERM, as an interrupted run's) has its tag removed too.
(cd "$clone" && with_clone_tag "$clone" "$out" v9.9.9 "$sha" sh -c 'kill -TERM $PPID; sleep 1') > /dev/null 2>&1
code=$?
check "a stopped gate exits as stopped" same "$code" 143
check "a stopped gate's tag is removed" no_tag "$clone" v9.9.9

# A tag the clone had from its source (a past release's commit gated again) is put back on the
# object it named, the annotated tag itself.
rm -f "$seen"
(cd "$clone" && with_clone_tag "$clone" "$out" v0.0.1 "$sha" sh -c 'git rev-parse "refs/tags/v0.0.1^{commit}" > "$1"' sh "$seen") > /dev/null
check "the command sees the source's tag moved to the gated commit" same "$(cat "$seen")" "$sha"
check "the source's tag is put back in the clone" same "$(tag_of "$clone" v0.0.1)" "$released"

# A run killed before it undid its tags (no trap runs on SIGKILL): the list stays, a second tag
# of the same name keeps what the clone had first, and a later clone_untag undoes them all.
clone_tag "$clone" "$out" v9.9.9 "$sha" && clone_tag "$clone" "$out" v0.0.1 "$sha" \
    && clone_tag "$clone" "$out" v0.0.1 "$sha"
check "the killed run's tags are listed" test -f "$pending"
check "a later run undoes them" eval 'clone_untag "$clone" "$out" > /dev/null'
check "the killed run's new tag is removed" no_tag "$clone" v9.9.9
check "the killed run's moved tag is put back" same "$(tag_of "$clone" v0.0.1)" "$released"
check "nothing is left to undo" test ! -e "$pending"
check "undoing nothing succeeds" clone_untag "$clone" "$out"

# Names that are not a release tag's are refused, and make no ref.
for name in main 'v1/../heads/main' 'v1 2' 'v1;true' v '-v1' 'refs/tags/v1'; do
    check "the name '$name' is refused" eval '! clone_tag "$clone" "$out" "$name" "$sha" 2> /dev/null'
done
check "a refused name makes no ref" same "$(refs "$clone" | grep -c refs/tags/)" 1
# A list that holds a line that is no tag to undo fails, and changes nothing.
printf 'main none\n' > "$pending"
check "a line that is no tag to undo fails" eval '! clone_untag "$clone" "$out" 2> /dev/null'
check "a branch is never removed" git -C "$clone" rev-parse --quiet --verify HEAD > /dev/null
rm -f "$pending"

# A clone whose marker names another OUT, or a clone that is not OUT/clone, is not the gates'.
printf '%s\n' "$work/elsewhere" > "$clone/.git/$RELEASE_GATES_MARKER"
check "a marker of another OUT is refused" eval '! clone_tag "$clone" "$out" v9.9.9 "$sha" 2> /dev/null'
check "a refused clone gets no tag" no_tag "$clone" v9.9.9
clone_mark "$clone" "$out"
check "a clone that is not OUT/clone is refused" eval '! clone_owned "$clone" "$work" 2> /dev/null'

# A real repository, with no marker, is never changed: neither tagged nor untagged, even with a
# list of tags to undo in its .git.
real=$work/real/clone
git init --quiet "$real" && git -C "$real" commit --quiet --allow-empty -m R && git -C "$real" tag v1.0.0
printf 'v1.0.0 none\n' > "$real/.git/$RELEASE_GATES_PENDING"
real_before=$(refs "$real")
check "a repository without the marker is not tagged" eval '! clone_tag "$real" "$work/real" v2.0.0 HEAD 2> /dev/null'
check "a repository without the marker is not untagged" eval '! clone_untag "$real" "$work/real" 2> /dev/null'
check "the real repository's refs are unchanged" same "$(refs "$real")" "$real_before"

# A clone whose .git is a link to the source's, or a worktree of the source (a .git file), is
# refused before anything is written, even with a marker naming its OUT.
mkdir -p "$work/link/clone"
ln -s "$src/.git" "$work/link/clone/.git"
check "a .git that is a link is refused" eval '! clone_tag "$work/link/clone" "$work/link" v9.9.9 "$sha" 2> /dev/null'
git -C "$src" worktree add --quiet --detach "$work/wt/clone" "$sha" || exit 1
check "a worktree's .git file is refused" eval '! clone_tag "$work/wt/clone" "$work/wt" v9.9.9 "$sha" 2> /dev/null'
git -C "$src" worktree remove --force "$work/wt/clone"
mkdir -p "$work/clone-link"
ln -s "$clone" "$work/clone-link/clone"
printf '%s\n' "$work/clone-link" > "$clone/.git/$RELEASE_GATES_MARKER"
check "a clone that is a link is refused" eval '! clone_tag "$work/clone-link/clone" "$work/clone-link" v9.9.9 "$sha" 2> /dev/null'
clone_mark "$clone" "$out"

# An exported GIT_DIR naming the source does not redirect the tag: it is made and undone in the
# clone only.
rm -f "$seen"
(cd "$clone" && GIT_DIR=$src/.git GIT_WORK_TREE=$src with_clone_tag "$clone" "$out" v9.9.8 "$sha" \
    sh -c 'env -u GIT_DIR -u GIT_WORK_TREE git rev-parse "refs/tags/v9.9.8^{commit}" > "$1"' sh "$seen") > /dev/null
check "with GIT_DIR exported, the tag is the clone's" same "$(cat "$seen")" "$sha"
check "with GIT_DIR exported, the clone's tag is removed" no_tag "$clone" v9.9.8

# The source, the clone's remote, is never changed, and nothing is pushed to it.
check "the source's refs are unchanged" same "$(refs "$src")" "$before"
check "the source has no gate's tag" no_tag "$src" v9.9.9

if [ $status -ne 0 ]; then
    echo "release-gates-tag-test: FAIL" >&2
    exit 1
fi
echo "release-gates-tag-test: PASS ($count checks)"
