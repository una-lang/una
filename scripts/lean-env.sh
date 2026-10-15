# The Lean release that formal/lean-toolchain pins, for the scripts that build the formal track
# (scripts/formal.sh, the hunt gate of scripts/release-gates.sh). Sourced, with `root` set to the
# repository's root: installs the release when it is missing, in UNA_FORMAL_LEAN when it is set,
# else in the user's cache, one directory per version; puts it first on PATH; and checks that
# `lean --version` is that version, failing otherwise.
toolchain=$(cat "$root/formal/lean-toolchain")
version=${toolchain#leanprover/lean4:}
lean_home=${UNA_FORMAL_LEAN:-${XDG_CACHE_HOME:-$HOME/.cache}/una/lean/$version}
if [ ! -x "$lean_home/bin/lake" ]; then
    echo "== installing the Lean release pinned in formal/lean-toolchain in $lean_home"
    "$root/scripts/install-lean.sh" "$lean_home" || return 1
fi
PATH="$lean_home/bin:$PATH"
export PATH
case "$(lean --version)" in
    "Lean (version ${version#v},"*) ;;
    *)
        echo "error: lean in $lean_home is $(lean --version), not $version (formal/lean-toolchain)" >&2
        return 1
        ;;
esac
