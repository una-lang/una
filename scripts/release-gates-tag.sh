# The release tag of the release gates' own clone (#1017, docs/release-gates.md §2), sourced by
# scripts/release-gates.sh and scripts/release-gates-tag-test.sh. scripts/local-release.sh needs
# the tag v<version> on the gated commit, so the `release` and `reproducible` gates make it in
# OUT/clone; a gate that reads tags (the released sections of CHANGELOG.md, the security
# advisories' releases) would then see that version as released. So every tag made here is
# undone when its gate ends, whether the gate passes, fails or is interrupted: a tag the clone
# did not have is deleted, and one it had (a past release's commit gated again, whose tag came
# from the source) is put back on the object it named. What was undone is listed first, in the
# clone's .git, so a run killed before it could undo it is undone at the start of the next run.
#
# Nothing here touches a repository but the gates' own clone: each function refuses unless the
# clone is OUT/clone, its .git a directory there (not a link nor a worktree's file) with no
# commondir that would put its refs in another repository, git reads that .git as both the
# repository and the home of its refs, and the marker that the clone gate writes into it names
# OUT, compared as physical paths, so that a spelling of OUT through a link (/tmp and
# /private/tmp) is the same OUT. Git runs with that .git named explicitly and with no variable
# that would point it at another repository or add configuration. No ref outside refs/tags/ of
# the clone changes, and nothing is pushed or fetched: the source and any remote are untouched.
#
# Two runs on one OUT at once would see each other's tag, and the second's start would undo the
# first's live tag, so a run holds OUT's lock (gates_lock) from its start to its exit.

# The marker's name in the clone's .git, holding OUT, and the list of the tags to undo.
RELEASE_GATES_MARKER=una-release-gates-clone
RELEASE_GATES_PENDING=una-release-gates-tags

# The variables that would make git read another repository, or configuration that is not the
# machine's own files, unset by scripts/release-gates.sh for every gate and by clone_git for each
# step here. GIT_CONFIG_KEY_<n> and GIT_CONFIG_VALUE_<n> are read only with GIT_CONFIG_COUNT;
# scripts/release-gates.sh unsets them too (git_environment_unset).
RELEASE_GATES_GIT_VARIABLES=(GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE GIT_OBJECT_DIRECTORY
    GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_NAMESPACE GIT_CEILING_DIRECTORIES
    GIT_DISCOVERY_ACROSS_FILESYSTEM GIT_CONFIG GIT_CONFIG_PARAMETERS GIT_CONFIG_COUNT
    GIT_SHALLOW_FILE GIT_GRAFT_FILE GIT_REPLACE_REF_BASE GIT_NO_REPLACE_OBJECTS GIT_QUARANTINE_PATH
    GIT_PREFIX)

# Unsets every variable above, and every exported GIT_CONFIG_KEY_<n> and GIT_CONFIG_VALUE_<n>.
git_environment_unset() {
    local name
    unset "${RELEASE_GATES_GIT_VARIABLES[@]}"
    for name in $(compgen -e); do
        case $name in GIT_CONFIG_KEY_* | GIT_CONFIG_VALUE_*) unset "$name" ;; esac
    done
}

# The physical path of the directory $1 (every link resolved), or a failure.
physical_dir() {
    (CDPATH='' cd -P -- "$1" 2> /dev/null && pwd -P)
}

# git in the clone $1 only, whatever the environment says.
clone_git() {
    local clone=$1
    shift
    (
        git_environment_unset
        git --git-dir="$clone/.git" --work-tree="$clone" "$@"
    )
}

# clone_mark CLONE OUT: the clone gate's marker, written once its fresh clone exists: OUT's
# physical path.
clone_mark() {
    local out
    [ -d "$1/.git" ] && [ ! -L "$1/.git" ] || return 1
    out=$(physical_dir "$2") || return 1
    printf '%s\n' "$out" > "$1/.git/$RELEASE_GATES_MARKER"
}

# clone_owned CLONE OUT: whether CLONE is the gates' own clone under OUT; says why not on stderr.
# Each refusal has a message of its own, which scripts/release-gates-tag-test.sh checks.
clone_owned() {
    local clone=$1 out=$2 physical marked gitdir common
    if [ "$clone" != "$out/clone" ]; then
        echo "release gates: $clone is not $out/clone" >&2
        return 1
    fi
    if [ -L "$clone" ] || [ -L "$clone/.git" ] || [ ! -d "$clone/.git" ]; then
        echo "release gates: $clone/.git is not a directory of the clone's own" >&2
        return 1
    fi
    if [ -e "$clone/.git/commondir" ] || [ -L "$clone/.git/commondir" ]; then
        echo "release gates: $clone/.git has a commondir, which would put its refs in another repository" >&2
        return 1
    fi
    marked=""
    if [ -f "$clone/.git/$RELEASE_GATES_MARKER" ] && [ ! -L "$clone/.git/$RELEASE_GATES_MARKER" ]; then
        marked=$(cat "$clone/.git/$RELEASE_GATES_MARKER")
        marked=$(physical_dir "$marked") || marked=""
    fi
    if [ -z "$marked" ] || [ "$marked" != "$(physical_dir "$out")" ]; then
        echo "release gates: $clone is not the clone that the clone gate made for $out; run the clone gate first" >&2
        return 1
    fi
    physical=$(physical_dir "$clone/.git") || return 1
    gitdir=$(clone_git "$clone" rev-parse --absolute-git-dir 2> /dev/null) \
        && common=$(clone_git "$clone" rev-parse --git-common-dir 2> /dev/null) || {
        echo "release gates: git cannot read $clone" >&2
        return 1
    }
    gitdir=$(physical_dir "$gitdir") || gitdir=""
    if [ "$gitdir" != "$physical" ]; then
        echo "release gates: the repository of $clone is not its own .git" >&2
        return 1
    fi
    # A relative common dir is relative to the directory git ran in, which is this shell's.
    common=$(physical_dir "$common") || common=""
    if [ "$common" != "$physical" ]; then
        echo "release gates: the refs of $clone are not in its own .git" >&2
        return 1
    fi
}

# Whether $1 is a release tag's name: v, a digit, then letters, digits, `.`, `+` and `-` only,
# so that it names a ref under refs/tags/ and nothing else.
release_tag_name() {
    [[ "$1" =~ ^v[0-9][0-9A-Za-z.+-]*$ ]] && git check-ref-format "refs/tags/$1"
}

# Whether the list $1 is whole: empty, or ending in a newline. A list cut by a crash or a full
# disk is not, and no line is appended to it, which would fuse two lines.
pending_whole() {
    [ ! -s "$1" ] || [ -z "$(tail -c 1 "$1")" ]
}

# clone_tag CLONE OUT TAG SHA: TAG on SHA in the clone, once it is listed to be undone. A tag
# already listed keeps what it named first, so that undoing it restores the clone's own.
clone_tag() {
    local clone=$1 out=$2 tag=$3 sha=$4 pending old
    clone_owned "$clone" "$out" || return 1
    release_tag_name "$tag" || { echo "release gates: $tag is not a release tag" >&2; return 1; }
    pending=$clone/.git/$RELEASE_GATES_PENDING
    [ -L "$pending" ] && { echo "release gates: $pending is a link" >&2; return 1; }
    if [ -f "$pending" ] && ! pending_whole "$pending"; then
        echo "release gates: $pending does not end in a newline; run the clone gate first" >&2
        return 1
    fi
    if ! { [ -f "$pending" ] && awk -v t="$tag" '$1 == t { found = 1 } END { exit !found }' "$pending"; }; then
        old=$(clone_git "$clone" rev-parse --quiet --verify "refs/tags/$tag") || old=none
        printf '%s %s\n' "$tag" "$old" >> "$pending" || return 1
    fi
    clone_git "$clone" tag -f "$tag" "$sha" > /dev/null
}

# clone_untag CLONE OUT: undoes every tag listed, in the clone only, then drops the list. A list
# that does not end in a newline has its last line undone too, and fails, and is kept.
clone_untag() {
    local clone=$1 out=$2 pending tag old status=0
    pending=$clone/.git/$RELEASE_GATES_PENDING
    [ -e "$pending" ] || [ -L "$pending" ] || return 0
    clone_owned "$clone" "$out" || return 1
    [ -L "$pending" ] && { echo "release gates: $pending is a link" >&2; return 1; }
    if ! pending_whole "$pending"; then
        echo "release gates: $pending does not end in a newline" >&2
        status=1
    fi
    while read -r tag old || [ -n "$tag" ]; do
        if ! release_tag_name "$tag" || ! { [ "$old" = none ] || [[ "$old" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]]; }; then
            echo "release gates: $pending holds a line that is not a tag to undo" >&2
            status=1
        elif [ "$old" = none ]; then
            if ! clone_git "$clone" rev-parse --quiet --verify "refs/tags/$tag" > /dev/null; then
                echo "release gates: the clone has no tag $tag left to remove"
            elif clone_git "$clone" update-ref -d "refs/tags/$tag"; then
                echo "release gates: removed the clone's tag $tag"
            else
                status=1
            fi
        elif clone_git "$clone" update-ref "refs/tags/$tag" "$old"; then
            echo "release gates: put the clone's tag $tag back on $old"
        else
            status=1
        fi
    done < "$pending"
    if [ $status -eq 0 ]; then
        rm -f "$pending"
    else
        echo "release gates: the clone's tags listed in $pending could not all be undone" >&2
    fi
    return $status
}

# with_clone_tag CLONE OUT TAG SHA COMMAND...: COMMAND with TAG on SHA in the clone, in a subshell
# whose exit, however it comes, undoes the tag; a tag that cannot be undone fails it.
with_clone_tag() {
    (
        clone=$1 out=$2 tag=$3 sha=$4
        shift 4
        trap 'status=$?; clone_untag "$clone" "$out" || status=1; exit $status' EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        trap 'exit 129' HUP
        clone_tag "$clone" "$out" "$tag" "$sha" || exit 1
        "$@"
    )
}

# OUT's lock: a directory made atomically by mkdir, holding the pid of the run that made it.
RELEASE_GATES_LOCK=release-gates.lock

# gates_lock OUT: takes OUT's lock for the process $$, or says on stderr which run holds it and
# fails. A lock whose process is gone (a run killed with SIGKILL) is stale and taken over; a lock
# with no pid yet is a run that is starting, and is not.
gates_lock() {
    local out=$1 lock=$1/$RELEASE_GATES_LOCK pid stale
    if mkdir "$lock" 2> /dev/null; then
        printf '%s\n' "$$" > "$lock/pid" && return 0
        rm -rf "${lock:?}"
        echo "release gates: cannot write $lock/pid" >&2
        return 1
    fi
    if [ -L "$lock" ] || [ ! -d "$lock" ]; then
        echo "release gates: $lock is not a lock directory; remove it" >&2
        return 1
    fi
    pid=$(cat "$lock/pid" 2> /dev/null) || pid=""
    if ! [[ $pid =~ ^[1-9][0-9]*$ ]]; then
        echo "release gates: $lock names no run yet, so another run on $out may be starting; runs on one OUT must not overlap (remove $lock if no run is left)" >&2
        return 1
    fi
    if kill -0 "$pid" 2> /dev/null || ps -p "$pid" > /dev/null 2>&1; then
        echo "release gates: another run (pid $pid) holds $lock; runs on one OUT must not overlap" >&2
        return 1
    fi
    # Stale: moved aside by one rename, so that of two runs taking it over only one does, and
    # checked to be the lock that was read, not one a third run has just made.
    stale=$lock.stale.$$
    if ! mv "$lock" "$stale" 2> /dev/null; then
        echo "release gates: another run took $lock" >&2
        return 1
    fi
    if [ "$(cat "$stale/pid" 2> /dev/null)" != "$pid" ]; then
        [ -e "$lock" ] || mv "$stale" "$lock" 2> /dev/null
        echo "release gates: another run took $lock" >&2
        return 1
    fi
    rm -rf "${stale:?}"
    echo "release gates: took over the lock of a run that ended without releasing it (pid $pid)"
    if mkdir "$lock" 2> /dev/null && printf '%s\n' "$$" > "$lock/pid"; then
        return 0
    fi
    echo "release gates: another run took $lock" >&2
    return 1
}

# gates_unlock OUT: releases OUT's lock if the process $$ holds it.
gates_unlock() {
    local lock=$1/$RELEASE_GATES_LOCK
    if [ "$(cat "$lock/pid" 2> /dev/null)" = "$$" ]; then
        rm -rf "${lock:?}"
    fi
}
