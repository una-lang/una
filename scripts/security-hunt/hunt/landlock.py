"""Confines a run of the hunt on Linux with Landlock, then executes it (#793).

    python3 -I landlock.py --write DIR --identity DEV:INO [--read DIR]... [--port PORT] -- ARGV...

The process restricts itself, then `execve`s ARGV, which inherits the restriction and can never
lift it. With the kernel's Landlock ABI from `MIN_ABI` to `KNOWN_ABI`:

- every file right of the ABI is handled: ARGV may write, create, remove, rename and link only
  under `--write`; read and execute only there, in the `--read` directories, its own
  executable's directory, its working directory and the system's (`SYSTEM_READ`); and among
  devices write only `/dev/null` and read only it and the random and zero devices;
- TCP is handled: ARGV may connect only to `--port` (the mock server's) and bind nowhere;
- abstract Unix sockets and signals are scoped: ARGV reaches no process outside its own domain
  through them.

`--write` is the directory the parent made for this run: it is opened without following a
symbolic link, and its device and inode must be `--identity`, the ones the parent recorded, so
a run cannot swap another run's directory for a link to somewhere else.

It never asks for less than that ("best effort"): an ABI below `MIN_ABI`, an ABI above
`KNOWN_ABI` (whose new rights this file does not handle yet), or any call that fails, stops it
before ARGV runs, with exit code `SETUP_FAILED` and a line on stderr that starts with
`ERROR_PREFIX`, which `sandbox.run` turns into a refusal of the whole run.

The file is standalone (the standard library only) and imports nothing from the hunt, so that it
runs with `python3 -I` from any directory. The syscall numbers are those of the generic table,
which aarch64 and x86_64 share for every call from 424 on (`include/uapi/asm-generic/unistd.h`
and `arch/x86/entry/syscalls/syscall_64.tbl`: 444, 445 and 446).
"""

import ctypes
import os
import struct
import sys

SYS_CREATE_RULESET = 444
SYS_ADD_RULE = 445
SYS_RESTRICT_SELF = 446
PR_SET_NO_NEW_PRIVS = 38
CREATE_RULESET_VERSION = 1
RULE_PATH_BENEATH = 1
RULE_NET_PORT = 2

# File rights (include/uapi/linux/landlock.h), by the ABI that added them.
EXECUTE, WRITE_FILE, READ_FILE, READ_DIR = 1 << 0, 1 << 1, 1 << 2, 1 << 3
REMOVE_DIR, REMOVE_FILE, MAKE_CHAR, MAKE_DIR = 1 << 4, 1 << 5, 1 << 6, 1 << 7
MAKE_REG, MAKE_SOCK, MAKE_FIFO, MAKE_BLOCK, MAKE_SYM = 1 << 8, 1 << 9, 1 << 10, 1 << 11, 1 << 12
REFER = 1 << 13  # ABI 2
TRUNCATE = 1 << 14  # ABI 3
IOCTL_DEV = 1 << 15  # ABI 5
BIND_TCP, CONNECT_TCP = 1 << 0, 1 << 1  # ABI 4
SCOPE_ABSTRACT_UNIX_SOCKET, SCOPE_SIGNAL = 1 << 0, 1 << 1  # ABI 6

FS_BY_ABI = {1: (1 << 13) - 1, 2: REFER, 3: TRUNCATE, 5: IOCTL_DEV}
# The rights a rule beneath a file, not a directory, may carry.
FILE_RIGHTS = EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV
READ_RIGHTS = EXECUTE | READ_FILE | READ_DIR

# Below ABI 6 signals and abstract Unix sockets cannot be scoped, so the hunt does not run there.
MIN_ABI = 6
# The newest ABI whose rights this file knows. In the upstream `include/uapi/linux/landlock.h`
# (read 2026-10-17), ABI 7 adds only flags of the audit log and ABI 8 only the synchronization of
# threads, no access right; a later ABI adds rights this file does not handle (the header lists a
# file right of ABI 9 over Unix sockets and rights of ABI 10 over UDP), so a kernel above 8 is
# refused until they are handled.
KNOWN_ABI = 8

SYSTEM_READ = ("/usr", "/lib", "/lib64", "/bin", "/sbin", "/etc", "/proc")
DEVICES = (("/dev/null", READ_FILE | WRITE_FILE | TRUNCATE | IOCTL_DEV),
           ("/dev/urandom", READ_FILE), ("/dev/random", READ_FILE), ("/dev/zero", READ_FILE))

SETUP_FAILED = 125
ERROR_PREFIX = "hunt-sandbox-error:"


class SetupError(RuntimeError):
    pass


def _libc():
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    libc.prctl.restype = ctypes.c_int
    return libc


def _check(what, result):
    if result < 0:
        err = ctypes.get_errno()
        raise SetupError(f"{what} failed: {os.strerror(err)} (errno {err})")
    return result


def abi(libc=None):
    """The kernel's Landlock ABI version, or 0 when Landlock is missing or off, or the system is
    not Linux, whose syscall numbers these are."""
    if not sys.platform.startswith("linux"):
        return 0
    libc = libc or _libc()
    version = libc.syscall(SYS_CREATE_RULESET, None, ctypes.c_size_t(0),
                           ctypes.c_uint32(CREATE_RULESET_VERSION))
    return max(int(version), 0)


def handled(version):
    """The rights a ruleset handles at ABI `version`: (file, network, scope)."""
    if version < MIN_ABI:
        raise SetupError(f"Landlock ABI {version} is below {MIN_ABI}: the hunt does not run")
    if version > KNOWN_ABI:
        raise SetupError(f"Landlock ABI {version} is above {KNOWN_ABI}, whose rights the hunt "
                         "handles: its new rights must be handled before the hunt runs there")
    fs = 0
    for since, rights in FS_BY_ABI.items():
        if version >= since:
            fs |= rights
    return fs, BIND_TCP | CONNECT_TCP, SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL


def ruleset_attr(fs, net, scoped):
    """`struct landlock_ruleset_attr`, its three fields of ABI 6."""
    return struct.pack("=QQQ", fs, net, scoped)


def rules(read, cwd, executable, fs):
    """The read rules: (path, rights), each path existing; none of them writes, but
    `/dev/null`. The run's own directory has its rule apart (`confine`)."""
    out = []
    for path in (*SYSTEM_READ, *read, cwd, os.path.dirname(executable)):
        if path and os.path.isdir(path):
            out.append((path, READ_RIGHTS & fs))
    for path, rights in DEVICES:
        if os.path.exists(path):
            out.append((path, rights & fs))
    return out


def open_write(write, identity):
    """The run's own directory, opened without following a link, if it is the one the parent
    made: its device and inode are `identity`."""
    fd = os.open(write, os.O_PATH | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    st = os.fstat(fd)
    if (st.st_dev, st.st_ino) != identity:
        os.close(fd)
        raise SetupError(f"{write} is not the directory the parent made for this run")
    return fd


def _add_path_rule(libc, ruleset, fd, rights, what):
    rule = struct.pack("=Qi", rights, fd)
    rb = ctypes.create_string_buffer(rule, len(rule))
    _check(f"landlock_add_rule {what}",
           libc.syscall(SYS_ADD_RULE, ctypes.c_int(ruleset), ctypes.c_int(RULE_PATH_BENEATH),
                        rb, ctypes.c_uint32(0)))


def confine(write, identity, read=(), port=None, cwd=None, executable=""):
    """Restricts this process, and what it executes later, as the module says."""
    for path in read:
        if os.path.realpath(path) == "/":
            raise SetupError("the root is not a directory a run may read")
    libc = _libc()
    version = abi(libc)
    fs, net, scoped = handled(version)
    attr = ruleset_attr(fs, net, scoped)
    buf = ctypes.create_string_buffer(attr, len(attr))
    ruleset = _check("landlock_create_ruleset",
                     libc.syscall(SYS_CREATE_RULESET, buf, ctypes.c_size_t(len(attr)),
                                  ctypes.c_uint32(0)))
    try:
        own = open_write(write, identity)
        try:
            _add_path_rule(libc, ruleset, own, fs, write)
        finally:
            os.close(own)
        for path, rights in rules([os.path.realpath(p) for p in read],
                                  os.path.realpath(cwd or os.getcwd()), executable, fs):
            if not os.path.isdir(path):
                rights &= FILE_RIGHTS
            fd = os.open(path, os.O_PATH | os.O_CLOEXEC)
            try:
                _add_path_rule(libc, ruleset, fd, rights, path)
            finally:
                os.close(fd)
        if port is not None:
            rule = struct.pack("=QQ", CONNECT_TCP, int(port))
            rb = ctypes.create_string_buffer(rule, len(rule))
            _check(f"landlock_add_rule port {port}",
                   libc.syscall(SYS_ADD_RULE, ctypes.c_int(ruleset),
                                ctypes.c_int(RULE_NET_PORT), rb, ctypes.c_uint32(0)))
        _check("prctl(PR_SET_NO_NEW_PRIVS)", libc.prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))
        _check("landlock_restrict_self",
               libc.syscall(SYS_RESTRICT_SELF, ctypes.c_int(ruleset), ctypes.c_uint32(0)))
    finally:
        os.close(ruleset)
    return version


def parse(argv):
    """`--write DIR --identity DEV:INO [--read DIR]... [--port PORT] -- ARGV...`."""
    write, identity, read, port = None, None, [], None
    i = 0
    while i < len(argv) and argv[i] != "--":
        flag = argv[i]
        if i + 1 >= len(argv):
            raise SetupError(f"{flag} needs a value")
        value = argv[i + 1]
        if flag == "--write":
            write = value
        elif flag == "--identity":
            dev, _, ino = value.partition(":")
            identity = (int(dev), int(ino))
        elif flag == "--read":
            read.append(value)
        elif flag == "--port":
            port = int(value)
        else:
            raise SetupError(f"unknown option {flag}")
        i += 2
    command = argv[i + 1:]
    if write is None or identity is None or not command:
        raise SetupError("usage: landlock.py --write DIR --identity DEV:INO [--read DIR]... "
                         "[--port PORT] -- ARGV...")
    return write, identity, read, port, command


def main(argv):
    try:
        write, identity, read, port, command = parse(argv)
        confine(write, identity, read, port, executable=os.path.realpath(command[0]))
    except (SetupError, OSError, ValueError) as e:
        sys.stderr.write(f"{ERROR_PREFIX} {e}\n")
        return SETUP_FAILED
    os.execv(command[0], command)
    return SETUP_FAILED


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
