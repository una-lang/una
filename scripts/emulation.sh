#!/bin/sh
# Whether this process runs natively (#1939, ADR 0124 rule 31): the limits gate and the
# confinement cases count only when run natively on each target, never under emulation, where no
# seccomp filter installs (ADR 0124 probe P1, #1442). scripts/limits.sh refuses to run unless this
# exits 0.
#
# Exit 0: native. Exit 1: emulated, and the reason on stderr. Exit 2: it cannot tell, and why;
# a caller treats that as emulated (fail closed). It detects:
#   - the marker of scripts/local-ci.sh, UNA_LOCAL_CI_SERIAL, set for a Linux container it does
#     not know to be native (an amd64 container on an engine that is not x86-64);
#   - on macOS, a process translated by Rosetta (sysctl.proc_translated is 1); on an arm64 Mac a
#     sysctl that does not answer 0 or 1 is "cannot tell";
#   - on Linux, an emulator that runs this shell as the interpreter of a binfmt_misc handler, such
#     as Rosetta for Linux (Docker Desktop's linux/amd64 on an arm64 Mac) or QEMU's user mode
#     (`qemu-x86_64`, `qemu-aarch64`): the kernel's executable of this shell, /proc/<pid>/exe read
#     from a child process (the emulator answers a process's reads of its own /proc/self/exe, not
#     another's), is then the emulator, an ELF of the host's machine, while uname -m names the
#     emulated one. The ELF's machine must be the one uname -m names (x86_64: EM_X86_64, 62;
#     aarch64: EM_AARCH64, 183), in a little-endian ELF; any other machine, a file that is not
#     such an ELF, or one it cannot read is "cannot tell".
# It does not detect full-system emulation (a whole virtual machine under QEMU's TCG, whose kernel
# is emulated too): there the kernel and the shell agree. The release gates run on an arm64 Mac
# and in Docker Desktop's virtual machine, which is hardware-virtualized, not emulated.
#
# Usage: scripts/emulation.sh [EXE]   (EXE, for the tests only: the file read in place of
#        /proc/<this shell's pid>/exe; the gates never pass it)
set -u

emulated() {
    echo "emulation: $1" >&2
    exit 1
}
unknown() {
    echo "emulation: cannot tell whether this runs natively: $1" >&2
    exit 2
}

[ "$#" -le 1 ] || unknown "usage: scripts/emulation.sh [EXE]"
if [ -n "${UNA_LOCAL_CI_SERIAL:-}" ]; then
    emulated "scripts/local-ci.sh marks this container as emulated (UNA_LOCAL_CI_SERIAL)"
fi
system=$(uname -s) || unknown "uname -s failed"
machine=$(uname -m) || unknown "uname -m failed"
case $system in
    Darwin)
        translated=$(sysctl -n sysctl.proc_translated 2>/dev/null) || translated=none
        case $translated:$machine in
            1:*) emulated "this process is translated by Rosetta ($system $machine)" ;;
            0:*) exit 0 ;;
            # Before Rosetta 2 an Intel Mac has no such sysctl, and nothing translates there.
            none:x86_64) exit 0 ;;
            *) unknown "sysctl.proc_translated is ${translated} on $system $machine" ;;
        esac
        ;;
    Linux)
        case $machine in
            x86_64) expected=62 ;;
            aarch64) expected=183 ;;
            *) unknown "no ELF machine is known here for $machine" ;;
        esac
        exe=${1:-/proc/$$/exe}
        # od is a child process, so the emulator, if any, does not answer this read for it.
        header=$(od -An -v -tu1 -N20 "$exe" 2>/dev/null) || unknown "cannot read $exe"
        set -- $header
        [ "$#" -eq 20 ] || unknown "$exe is shorter than an ELF header"
        [ "$1 $2 $3 $4" = "127 69 76 70" ] || unknown "$exe is not an ELF file"
        [ "$6" = 1 ] || unknown "$exe is not a little-endian ELF file"
        shift 18
        found=$(($1 + 256 * $2))
        if [ "$found" != "$expected" ]; then
            emulated "this shell's executable is an ELF for machine $found, not for $machine ($expected): an emulator runs it ($system $machine)"
        fi
        exit 0
        ;;
    *) unknown "no check is known for $system" ;;
esac
