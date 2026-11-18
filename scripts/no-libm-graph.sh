# The release dependency graph check of scripts/no-libm.sh (#1093, #1949), sourced by it and by
# its canaries (scripts/no-libm-test.sh); it is not run on its own. It defines two functions,
# reads no environment variable and sets only variables named graph_*. The crates the check
# asserts are written in scripts/no-libm.sh, not here.
#
# no_libm_graph_verdict CRATE STATUS TREE ERR: the verdict on one run of `cargo tree -i CRATE`
# that exited with STATUS, its standard output in the file TREE and its standard error in ERR.
# It prints the verdict and returns 0 when the graph does not hold CRATE, 1 on a finding:
#
# - STATUS 0 and a TREE that is not empty: the graph holds CRATE, a finding;
# - STATUS 0, an empty TREE, and an ERR with the line `warning: nothing to print.` and no line
#   that starts with `error`: CRATE is in Cargo.lock but not in the graph, not a finding (the
#   pinned cargo, 1.98.1, reports such a crate so);
# - STATUS non-zero and an ERR that says the package ID specification CRATE did not match any
#   packages: CRATE is not in Cargo.lock, not a finding;
# - anything else (STATUS 0 with an empty TREE and no such warning, or any other failure): a
#   finding, so the check fails closed.
no_libm_graph_verdict() {
    graph_crate=$1 graph_status=$2 graph_tree=$3 graph_err=$4
    if [ "$graph_status" -eq 0 ]; then
        if [ -s "$graph_tree" ]; then
            cat "$graph_tree" "$graph_err" >&2
            echo "no-libm: the release graph holds $graph_crate"
            return 1
        fi
        if grep -qxF 'warning: nothing to print.' "$graph_err" && ! grep -q '^error' "$graph_err"; then
            echo "no-libm: $graph_crate is not in the release graph"
            return 0
        fi
        cat "$graph_err" >&2
        echo "no-libm: cargo tree of $graph_crate printed nothing and did not say it had nothing to print"
        return 1
    fi
    if grep -qF "package ID specification \`$graph_crate\` did not match any packages" "$graph_err"; then
        echo "no-libm: $graph_crate is not in the release graph"
        return 0
    fi
    cat "$graph_err" >&2
    echo "no-libm: cargo tree of $graph_crate failed (exit status $graph_status)"
    return 1
}

# no_libm_graph_check ROOT TMP CRATE: runs `cargo tree` on the release graph of the repository
# at ROOT for CRATE, its normal dependencies for the host, with its output in the directory TMP,
# and returns the verdict of no_libm_graph_verdict.
no_libm_graph_check() {
    (cd "$1" && cargo tree --locked -e normal -p una-cli -p una-rt -i "$3") \
        > "$2/tree" 2> "$2/err"
    no_libm_graph_verdict "$3" "$?" "$2/tree" "$2/err"
}
