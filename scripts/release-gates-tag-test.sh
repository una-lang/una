#!/bin/bash
# Tests of scripts/release-gates-tag.sh (#1017): the release tag that the release and
# reproducible gates make in the gates' own clone is undone when the gate ends, whether it
# passes, fails or is stopped, and put back when the clone had it from its source; a run killed
# before undoing it is undone later; and nothing but the gates' own clone is ever changed: not a
# repository without the clone gate's marker, nor one that a link, a worktree's .git file or an
# exported GIT_DIR or GIT_COMMON_DIR points at, nor the clone's source; OUT spelled through a link
# is the same OUT; and two runs never hold one OUT at once. Every repository is made here, in a fresh
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
git_environment_unset

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
# refused WANT COMMAND...: COMMAND fails, and its stderr names WANT, the refusal under test.
refused() {
    local want=$1 err
    shift
    if err=$("$@" 2>&1 > /dev/null); then
        echo "  succeeded: $*" >&2
        return 1
    fi
    case $err in
        *"$want"*) return 0 ;;
    esac
    echo "  wanted: $want; got: $err" >&2
    return 1
}

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
# (The loop's variable is not `name`, which check's own local would hide inside its eval.)
for bad in main 'v1/../heads/main' 'v1 2' 'v1;true' v '-v1' 'refs/tags/v1'; do
    check "the name '$bad' is refused" refused "is not a release tag" clone_tag "$clone" "$out" "$bad" "$sha"
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
check "a worktree's .git file is refused as not a directory" \
    refused "is not a directory of the clone's own" clone_owned "$work/wt/clone" "$work/wt"
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

# An exported GIT_COMMON_DIR, which --git-dir does not override, does not move the tag into the
# source either.
rm -f "$seen"
(cd "$clone" && GIT_COMMON_DIR=$src/.git with_clone_tag "$clone" "$out" v9.9.7 "$sha" \
    sh -c 'env -u GIT_COMMON_DIR git rev-parse "refs/tags/v9.9.7^{commit}" > "$1"' sh "$seen") > /dev/null
check "with GIT_COMMON_DIR exported, the tag is the clone's" same "$(cat "$seen" 2> /dev/null)" "$sha"
check "with GIT_COMMON_DIR exported, the source gets no tag" no_tag "$src" v9.9.7
check "with GIT_COMMON_DIR exported, the clone's tag is removed" no_tag "$clone" v9.9.7
# Nor do GIT_CONFIG_COUNT and its keys, or GIT_CONFIG_PARAMETERS, reach git's steps here.
check "no configuration from the environment reaches the clone's git" same "$(
    GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=una.injected GIT_CONFIG_VALUE_0=yes \
        GIT_CONFIG_PARAMETERS="'una.parameter'='yes'" \
        clone_git "$clone" config --get-regexp '^una\.' 2> /dev/null
)" ""

# Each check of clone_owned refuses on its own, in a clone that passes every other one.
# Not OUT/clone: a repository with a marker naming its own directory.
other=$work/other
mkdir -p "$other"
git clone --quiet --no-checkout "$src" "$other/repo" || exit 1
printf '%s\n' "$other" > "$other/repo/.git/$RELEASE_GATES_MARKER"
check "a clone that is not OUT/clone is refused for its path" \
    refused "is not $other/clone" clone_owned "$other/repo" "$other"
# A commondir in the clone's .git, which would put its refs, and so the tag, in the source.
printf '%s\n' "$src/.git" > "$clone/.git/commondir"
check "a clone with a commondir is refused" refused "has a commondir" clone_tag "$clone" "$out" v9.9.6 "$sha"
check "a clone with a commondir puts no tag in the source" no_tag "$src" v9.9.6
rm -f "$clone/.git/commondir"
# A git that reads another repository than the clone's .git (here a shim that says so), for the
# repository and for its refs.
mkdir -p "$work/shim"
cat > "$work/shim/git" << 'SHIM'
#!/bin/bash
for argument in "$@"; do
    if [ "$argument" = "$SHIM_ARGUMENT" ]; then
        printf '%s\n' "$SHIM_ANSWER"
        exit 0
    fi
done
exec "$SHIM_GIT" "$@"
SHIM
chmod 755 "$work/shim/git"
shimmed() { # argument answer, then a command
    local argument=$1 answer=$2
    shift 2
    (
        export SHIM_ARGUMENT=$argument SHIM_ANSWER=$answer SHIM_GIT
        SHIM_GIT=$(command -v git)
        PATH=$work/shim:$PATH
        "$@"
    )
}
check "a clone whose git directory is another repository is refused" \
    shimmed --absolute-git-dir "$src/.git" refused "is not its own .git" clone_owned "$clone" "$out"
check "a clone whose refs are in another repository is refused" \
    shimmed --git-common-dir "$src/.git" refused "are not in its own .git" clone_owned "$clone" "$out"
check "the clone is still owned" clone_owned "$clone" "$out"

# A tag that cannot be put back (its object is gone) fails, and the list is kept.
printf 'v0.0.1 %s\n' 0123456789abcdef0123456789abcdef01234567 > "$pending"
check "a restore that fails fails the undo" eval '! clone_untag "$clone" "$out" > /dev/null 2>&1'
check "a list that could not be undone is kept" test -f "$pending"
check "the clone's own tag stays where it was" same "$(tag_of "$clone" v0.0.1)" "$released"
rm -f "$pending"

# A list whose last line has no newline (a write cut short) still has that line undone, fails,
# and is kept; nothing is appended to it.
git -C "$clone" tag v1.2.3 "$sha"
printf 'v1.2.3 none' > "$pending"
check "a list cut short fails the undo" eval '! clone_untag "$clone" "$out" > /dev/null 2>&1'
check "a list cut short has its last line undone" no_tag "$clone" v1.2.3
check "a list cut short is kept" test -f "$pending"
check "no tag is listed after a list cut short" refused "does not end in a newline" clone_tag "$clone" "$out" v9.9.9 "$sha"
check "a refused list gets no tag" no_tag "$clone" v9.9.9
rm -f "$pending"

# OUT spelled through a link is the same OUT: the marker holds its physical path, and a run
# with either spelling owns the clone (macOS's /tmp and /private/tmp, a gates directory linked
# into another).
ln -s "$out" "$work/out-link"
clone_mark "$work/out-link/clone" "$work/out-link"
check "the marker holds OUT's physical path" same "$(cat "$clone/.git/$RELEASE_GATES_MARKER")" "$out"
check "the clone is owned by OUT through a link" clone_owned "$work/out-link/clone" "$work/out-link"
printf '%s\n' "$work/out-link" > "$clone/.git/$RELEASE_GATES_MARKER"
check "a marker that spells OUT through a link is OUT's" clone_owned "$clone" "$out"
(cd "$clone" && with_clone_tag "$work/out-link/clone" "$work/out-link" v9.9.5 "$sha" true) > /dev/null
check "a gate run through the link passes" same "$?" 0
check "a gate run through the link removes its tag" no_tag "$clone" v9.9.5
clone_mark "$clone" "$out"

# OUT's lock: one run at a time. A live run's lock refuses a second run; a lock whose run is
# gone (killed with SIGKILL, no trap) is taken over; a lock with no pid yet is a run starting.
lockout=$work/lockout
mkdir -p "$lockout"
bash -c '. "$1/scripts/release-gates-tag.sh" && gates_lock "$2" && exec sleep 60' sh "$root" "$lockout" &
holder=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do
    [ -f "$lockout/$RELEASE_GATES_LOCK/pid" ] && break
    sleep 0.2
done
check "the lock holds its run's pid" same "$(cat "$lockout/$RELEASE_GATES_LOCK/pid" 2> /dev/null)" "$holder"
check "a second run on a held OUT is refused" refused "another run (pid $holder) holds" gates_lock "$lockout"
gates_unlock "$lockout"
check "a run that does not hold the lock does not release it" test -d "$lockout/$RELEASE_GATES_LOCK"
kill -KILL "$holder" 2> /dev/null
wait "$holder" 2> /dev/null
check "a lock whose run is gone is taken over" eval 'gates_lock "$lockout" > /dev/null'
check "the lock taken over holds this run's pid" same "$(cat "$lockout/$RELEASE_GATES_LOCK/pid")" "$$"
gates_unlock "$lockout"
check "the run's exit releases the lock" test ! -e "$lockout/$RELEASE_GATES_LOCK"
mkdir "$lockout/$RELEASE_GATES_LOCK"
check "a lock with no pid yet is refused" refused "names no run yet" gates_lock "$lockout"
rmdir "$lockout/$RELEASE_GATES_LOCK"

# The source, the clone's remote, is never changed, and nothing is pushed to it.
check "the source's refs are unchanged" same "$(refs "$src")" "$before"
check "the source has no gate's tag" no_tag "$src" v9.9.9

if [ $status -ne 0 ]; then
    echo "release-gates-tag-test: FAIL" >&2
    exit 1
fi
echo "release-gates-tag-test: PASS ($count checks)"
