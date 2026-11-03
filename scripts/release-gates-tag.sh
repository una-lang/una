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
# clone is OUT/clone, its .git a directory there (not a link nor a worktree's file), and the
# marker that the clone gate writes into it names OUT. Git runs with that .git named explicitly
# and with no variable that would point it at another repository. No ref outside refs/tags/ of
# the clone changes, and nothing is pushed or fetched: the source and any remote are untouched.

# The marker's name in the clone's .git, holding OUT, and the list of the tags to undo.
RELEASE_GATES_MARKER=una-release-gates-clone
RELEASE_GATES_PENDING=una-release-gates-tags

# git in the clone $1 only, whatever the environment says.
clone_git() {
    local clone=$1
    shift
    env -u GIT_DIR -u GIT_WORK_TREE -u GIT_COMMON_DIR -u GIT_INDEX_FILE -u GIT_OBJECT_DIRECTORY \
        -u GIT_ALTERNATE_OBJECT_DIRECTORIES -u GIT_NAMESPACE -u GIT_CEILING_DIRECTORIES \
        -u GIT_DISCOVERY_ACROSS_FILESYSTEM -u GIT_CONFIG_PARAMETERS -u GIT_CONFIG_COUNT \
        git --git-dir="$clone/.git" --work-tree="$clone" "$@"
}

# clone_mark CLONE OUT: the clone gate's marker, written once its fresh clone exists.
clone_mark() {
    [ -d "$1/.git" ] && [ ! -L "$1/.git" ] || return 1
    printf '%s\n' "$2" > "$1/.git/$RELEASE_GATES_MARKER"
}

# clone_owned CLONE OUT: whether CLONE is the gates' own clone under OUT; says why not on stderr.
clone_owned() {
    local clone=$1 out=$2 physical gitdir
    if [ "$clone" != "$out/clone" ]; then
        echo "release gates: $clone is not $out/clone" >&2
        return 1
    fi
    if [ -L "$clone" ] || [ -L "$clone/.git" ] || [ ! -d "$clone/.git" ]; then
        echo "release gates: $clone/.git is not a directory of the clone's own" >&2
        return 1
    fi
    if [ ! -f "$clone/.git/$RELEASE_GATES_MARKER" ] || [ -L "$clone/.git/$RELEASE_GATES_MARKER" ] \
        || [ "$(cat "$clone/.git/$RELEASE_GATES_MARKER")" != "$out" ]; then
        echo "release gates: $clone is not the clone that the clone gate made for $out; run the clone gate first" >&2
        return 1
    fi
    physical=$(CDPATH='' cd -P -- "$clone/.git" 2>/dev/null && pwd -P) || return 1
    gitdir=$(clone_git "$clone" rev-parse --absolute-git-dir 2>/dev/null) || {
        echo "release gates: git cannot read $clone" >&2
        return 1
    }
    gitdir=$(CDPATH='' cd -P -- "$gitdir" 2>/dev/null && pwd -P) || return 1
    if [ "$gitdir" != "$physical" ]; then
        echo "release gates: the repository of $clone is not its own .git" >&2
        return 1
    fi
}

# Whether $1 is a release tag's name: v, a digit, then letters, digits, `.`, `+` and `-` only,
# so that it names a ref under refs/tags/ and nothing else.
release_tag_name() {
    [[ "$1" =~ ^v[0-9][0-9A-Za-z.+-]*$ ]] && git check-ref-format "refs/tags/$1"
}

# clone_tag CLONE OUT TAG SHA: TAG on SHA in the clone, once it is listed to be undone. A tag
# already listed keeps what it named first, so that undoing it restores the clone's own.
clone_tag() {
    local clone=$1 out=$2 tag=$3 sha=$4 pending old
    clone_owned "$clone" "$out" || return 1
    release_tag_name "$tag" || { echo "release gates: $tag is not a release tag" >&2; return 1; }
    pending=$clone/.git/$RELEASE_GATES_PENDING
    [ -L "$pending" ] && { echo "release gates: $pending is a link" >&2; return 1; }
    if ! { [ -f "$pending" ] && awk -v t="$tag" '$1 == t { found = 1 } END { exit !found }' "$pending"; }; then
        old=$(clone_git "$clone" rev-parse --quiet --verify "refs/tags/$tag") || old=none
        printf '%s %s\n' "$tag" "$old" >> "$pending" || return 1
    fi
    clone_git "$clone" tag -f "$tag" "$sha" > /dev/null
}

# clone_untag CLONE OUT: undoes every tag listed, in the clone only, then drops the list.
clone_untag() {
    local clone=$1 out=$2 pending tag old status=0
    pending=$clone/.git/$RELEASE_GATES_PENDING
    [ -e "$pending" ] || [ -L "$pending" ] || return 0
    clone_owned "$clone" "$out" || return 1
    [ -L "$pending" ] && { echo "release gates: $pending is a link" >&2; return 1; }
    while read -r tag old; do
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
