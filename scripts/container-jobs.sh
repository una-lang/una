#!/bin/sh
# The job count of the build inside a Linux container of scripts/local-ci.sh (#701,
# docs/release-gates.md): one job per 2 GiB of the Docker VM's memory, at most one per CPU
# and at least one. A CARGO_BUILD_JOBS set on the host can lower it and never raise it, so a
# setting meant for the host does not run the VM out of memory (fail closed).
#
# Usage: scripts/container-jobs.sh MEMORY_BYTES CPUS [HOST_JOBS]
# Prints the job count on the first line and why on the second; a bad argument exits 2.
set -eu

memory=${1:-}
cpus=${2:-}
host=${3:-}
case "$memory:$cpus" in
*[!0-9:]* | :* | *:)
    echo "error: the VM's memory ($memory) and CPU count ($cpus) must be numbers" >&2
    exit 2
    ;;
esac
cap=$((memory / 2147483648))
[ "$cap" -gt "$cpus" ] && cap=$cpus
[ "$cap" -lt 1 ] && cap=1
if [ -z "$host" ]; then
    echo "$cap"
    echo "one job per 2 GiB of the VM's memory, at most one per CPU"
    exit 0
fi
case "$host" in
*[!0-9]*)
    echo "error: CARGO_BUILD_JOBS is $host, not a positive number" >&2
    exit 2
    ;;
esac
# Digits only, so a number; "0" and "00" are not positive.
if [ "$host" -lt 1 ]; then
    echo "error: CARGO_BUILD_JOBS is $host, not a positive number" >&2
    exit 2
fi
if [ "$host" -gt "$cap" ]; then
    echo "$cap"
    echo "CARGO_BUILD_JOBS=$host on the host, capped to the VM's memory"
else
    echo "$host"
    echo "CARGO_BUILD_JOBS=$host from the host"
fi
