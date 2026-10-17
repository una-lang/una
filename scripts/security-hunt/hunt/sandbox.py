"""Runs a command in the hunt's sandbox (docs/plan/security-hunt-oracle.md §6).

- A fresh directory per run, under the campaign's root, as the only writable place.
- No network, except loopback to one port when a mock server is given.
- A clean environment: `HOME` and `TMPDIR` point inside the run's directory.
- A wall-clock timeout.

There are two backends, and no unsandboxed fallback:

- macOS: `sandbox-exec`, with a profile that denies the network, signals to processes outside
  the run's own, and every write outside the run's writable directory.
- Linux (#793): the gate runs inside a hardened container (`scripts/hunt-linux.sh`), and each
  run is confined by Landlock (`landlock.py`): writes only under its writable directory, reads
  only under it, the directories it names, its executable's and the system's, and TCP only to
  the mock's port. The backend refuses to run unless the container is verified first
  (`container_problems`).
"""

import errno
import hashlib
import os
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time

from . import landlock

SANDBOX_EXEC = "/usr/bin/sandbox-exec"
LANDLOCK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "landlock.py")
# The marker that `scripts/hunt-linux.sh` sets in the gate's container, and its value.
MARKER = "UNA_HUNT_SANDBOX"
MARKER_VALUE = "container"
# Directories every run on Linux may read besides its own: the tools' (the gate adds `una`'s,
# the re-check's and the solver's) and those the container's script names in `EXTRA_READ_ENV`.
READABLE = []
EXTRA_READ_ENV = "UNA_HUNT_READ"
# The gate's output directory, which no run may read beyond its own directory (`landlock_argv`).
OUT_ROOT = []
# How long the output of a run's killed process group may take to close.
DRAIN = 5
# The proof-cache key every run reads (`UNA_PROOF_KEY`), made once per gate outside every run's
# directory, which no run may write: without it, `una` makes a key in each run's fresh `HOME`,
# about 6 ms a run (#793).
PROOF_KEY = []
# What the key was when made: its file's device, inode, size and modification time, and a digest
# of its bytes. A run starts only while the file is unchanged (`check_proof_key`), so a key that
# went missing or changed fails the gate rather than leaving `una` without one.
_KEY_STATE = []


class SandboxUnavailable(RuntimeError):
    pass


def backend():
    """`"sandbox-exec"` on macOS, `"landlock"` on Linux, else None."""
    if sys.platform == "darwin":
        return "sandbox-exec"
    if sys.platform.startswith("linux"):
        return "landlock"
    return None


def available():
    if backend() == "sandbox-exec":
        return os.path.exists(SANDBOX_EXEC)
    if backend() == "landlock":
        return not container_problems()
    return False


def _status(path="/proc/self/status"):
    fields = {}
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                key, _, value = line.partition(":")
                fields[key.strip()] = value.strip()
    except OSError:
        pass
    return fields


def _root_mount(path="/proc/self/mounts"):
    """The options of the mount at `/`, the last one listed."""
    options = None
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                parts = line.split()
                if len(parts) >= 4 and parts[1] == "/":
                    options = parts[3].split(",")
    except OSError:
        pass
    return options


def _up_interfaces(net):
    """The network interfaces that are up (`IFF_UP` in their flags). A container without a
    network still lists the kernel's own fallback devices (`gre0`, `sit0`, `tunl0` and the like),
    down, and files such as `bonding_masters`, which are no interface."""
    up = []
    for name in sorted(os.listdir(net)):
        flags = os.path.join(net, name, "flags")
        if not os.path.isfile(flags):
            continue
        with open(flags, encoding="utf-8") as f:
            if int(f.read().strip(), 16) & 0x1:
                up.append(name)
    return up


def _routes_off_loopback(proc_net):
    """The routes, IPv4 and IPv6, through an interface other than loopback."""
    out = []
    for name, device in (("route", 0), ("ipv6_route", -1)):
        with open(os.path.join(proc_net, name), encoding="utf-8") as f:
            lines = f.read().splitlines()
        if name == "route":
            lines = lines[1:]
        for line in lines:
            fields = line.split()
            if fields and fields[device] != "lo":
                out.append(f"{name}: {line.strip()}")
    return out


# Public addresses a connection to which must fail with ENETUNREACH in the gate's container.
EXTERNAL = ((socket.AF_INET, ("1.1.1.1", 53)), (socket.AF_INET6, ("2606:4700:4700::1111", 53)))


def _external_connections():
    """What a TCP connection to each of `EXTERNAL` gives: the errno's name, or "connected"."""
    out = []
    for family, address in EXTERNAL:
        try:
            s = socket.socket(family, socket.SOCK_STREAM)
        except OSError as e:
            out.append(errno.errorcode.get(e.errno, type(e).__name__))
            continue
        s.settimeout(2)
        try:
            s.connect(address)
            out.append("connected")
        except OSError as e:
            out.append(errno.errorcode.get(e.errno, type(e).__name__))
        finally:
            s.close()
    return out


def container_problems(env=None, status=None, root="/", net="/sys/class/net", landlock_abi=None,
                       root_options=None, proc_net="/proc/net", external=None):
    """Why this process is not in the hunt's verified container: an empty list when it is.

    Each condition is checked from inside, never taken from the marker alone: the marker that
    `scripts/hunt-linux.sh` sets, the container runtime's own file, no capabilities, the
    no-new-privileges flag, a user other than root, a read-only root, no network interface up
    but loopback, no route but through it (so no default route) and a connection to a public
    address that fails with ENETUNREACH, and Landlock at the ABI `landlock.py` needs."""
    env = os.environ if env is None else env
    status = _status() if status is None else status
    root_options = _root_mount() if root_options is None else root_options
    problems = []
    if env.get(MARKER) != MARKER_VALUE:
        problems.append(f"{MARKER} is not {MARKER_VALUE!r}: not started by scripts/hunt-linux.sh")
    if not os.path.exists(os.path.join(root, ".dockerenv")):
        problems.append("no /.dockerenv: not in a container")
    for field in ("CapEff", "CapPrm", "CapInh", "CapAmb", "CapBnd"):
        if field not in status or status[field].strip("0") != "":
            problems.append(f"capabilities {field} {status.get(field)!r}, not none")
    if status.get("NoNewPrivs") != "1":
        problems.append("no-new-privileges is not set")
    uids = status.get("Uid", "0").split()
    if not uids or "0" in uids:
        problems.append("running as root")
    gids = status.get("Gid", "0").split()
    if not gids or "0" in gids:
        problems.append("running in group root")
    if not root_options or "ro" not in root_options:
        problems.append(f"the root is not mounted read-only ({root_options})")
    try:
        up = _up_interfaces(net)
        routes = _routes_off_loopback(proc_net)
    except (OSError, ValueError) as e:
        up, routes = None, [f"unreadable: {e}"]
    if up is None or [name for name in up if name != "lo"]:
        problems.append(f"network interfaces up {up}, not loopback alone")
    if routes:
        problems.append(f"routes off loopback: {routes[:3]}")
    reached = _external_connections() if external is None else external
    # No route (ENETUNREACH), or for IPv6 a kernel without it (EAFNOSUPPORT).
    if any(r not in ("ENETUNREACH", "EAFNOSUPPORT") for r in reached):
        problems.append(f"connections to public addresses gave {reached}, not ENETUNREACH")
    version = landlock.abi() if landlock_abi is None else landlock_abi
    if version < landlock.MIN_ABI:
        problems.append(f"Landlock ABI {version}, below {landlock.MIN_ABI}")
    elif version > landlock.KNOWN_ABI:
        problems.append(f"Landlock ABI {version}, above {landlock.KNOWN_ABI}, whose rights the "
                        "hunt handles")
    return problems


def _quote(path):
    return '"' + path.replace("\\", "\\\\").replace('"', '\\"') + '"'


def profile(writable, mock_port=None):
    """The sandbox profile: everything is allowed but the network and writes, which are
    denied except under `writable` and to the null and terminal devices."""
    rules = [
        "(version 1)",
        "(allow default)",
        "(deny network*)",
        "(deny signal (target others))",
        "(deny file-write*)",
        f"(allow file-write* (subpath {_quote(os.path.realpath(writable))}))",
        '(allow file-write* (literal "/dev/null") (literal "/dev/tty") (literal "/dev/dtracehelper"))',
    ]
    if mock_port is not None:
        rules.append(f'(allow network-outbound (remote ip "localhost:{int(mock_port)}"))')
        rules.append('(allow network-bind (local ip "localhost:*"))')
    return "\n".join(rules) + "\n"


def landlock_argv(argv, writable, identity, env, mock_port=None):
    """The command that confines `argv` with Landlock, then executes it: the run may read its
    tools' directories, those on its `PATH`, and those the container names, none of which may be
    the root or hold the gate's output, where the other runs' directories are."""
    read = list(READABLE)
    read += [os.path.dirname(p) for p in PROOF_KEY]
    read += [p for p in env.get("PATH", "").split(":") if p]
    read += [p for p in os.environ.get(EXTRA_READ_ENV, "").split(":") if p]
    for path in read:
        real = os.path.realpath(path)
        for out in OUT_ROOT:
            out = os.path.realpath(out)
            if real == "/" or os.path.commonpath([real, out]) in (real, out):
                raise SandboxUnavailable(f"a run may not read {path}: it is the root or holds, "
                                         f"or lies in, the gate's output {out}")
    # `-S`: the launcher imports nothing of `site`, which saves its start on every run.
    cmd = [sys.executable, "-I", "-S", LANDLOCK, "--write", writable,
           "--identity", f"{identity[0]}:{identity[1]}"]
    for path in dict.fromkeys(read):
        cmd += ["--read", path]
    if mock_port is not None:
        cmd += ["--port", str(int(mock_port))]
    return cmd + ["--"] + list(argv)


class Run:
    """A finished run: its exit code, output, and whether it timed out."""

    def __init__(self, code, stdout, stderr, timed_out, seconds):
        self.code = code
        self.stdout = stdout
        self.stderr = stderr
        self.timed_out = timed_out
        self.seconds = seconds


# The run directories `fresh_dir` made, by real path, with the device and inode each had: a run
# may write only in one of these, its own (#793).
_FRESH = {}
_FRESH_LOCK = threading.Lock()


def fresh_dir(root, prefix):
    """A new directory for one run, under `root`."""
    os.makedirs(root, exist_ok=True)
    path = os.path.realpath(tempfile.mkdtemp(prefix=prefix, dir=root))
    st = os.lstat(path)
    with _FRESH_LOCK:
        _FRESH[path] = (st.st_dev, st.st_ino)
    return path


def identity(writable):
    """The device and inode `fresh_dir` recorded for `writable`, checked again now: a run writes
    only in a directory made for it, never through a link put in its place."""
    real = os.path.realpath(writable)
    with _FRESH_LOCK:
        recorded = _FRESH.get(real)
    if recorded is None or os.path.normpath(os.path.abspath(writable)) != real:
        raise ValueError(f"{writable} is not a run's own directory: a run writes only in one "
                         "that sandbox.fresh_dir made")
    st = os.lstat(real)
    if (st.st_dev, st.st_ino) != recorded:
        raise ValueError(f"{writable} is no longer the directory sandbox.fresh_dir made")
    return recorded


def remove_dir(path, root):
    """Removes a run's directory, only if it lies under `root`."""
    real, base = os.path.realpath(path), os.path.realpath(root)
    if not base or base == "/" or os.path.commonpath([real, base]) != base or real == base:
        raise ValueError(f"refusing to remove {path} outside {root}")
    shutil.rmtree(real)
    with _FRESH_LOCK:
        _FRESH.pop(real, None)


_VERIFIED = []


def make_proof_key():
    """Makes the gate's proof-cache key, in a directory of its own outside the gate's output, and
    names it in `PROOF_KEY`; returns the directory, which `drop_proof_key` removes. Its mode lets
    the gate's user write it: only the sandbox keeps a run from doing so, which its canary
    checks."""
    home = tempfile.mkdtemp(prefix="una-hunt-key-")
    path = os.path.join(home, "proof-cache.key")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    secret = os.urandom(32)
    with os.fdopen(fd, "wb") as f:
        f.write(secret)
    PROOF_KEY[:] = [os.path.realpath(path)]
    _KEY_STATE[:] = [(_key_stat(path), hashlib.sha256(secret).hexdigest())]
    return home


def _key_stat(path):
    st = os.stat(path, follow_symlinks=False)
    return (st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns, stat.S_ISREG(st.st_mode))


def check_proof_key(full=False):
    """Fails unless the gate's key is the file it made, unchanged; `full` reads it again and
    compares its digest too."""
    if not PROOF_KEY:
        return
    path, (made, digest) = PROOF_KEY[0], _KEY_STATE[0]
    try:
        same = _key_stat(path) == made
        if same and full:
            with open(path, "rb") as f:
                same = hashlib.sha256(f.read()).hexdigest() == digest
    except OSError:
        same = False
    if not same:
        raise SandboxUnavailable(f"the gate's proof-cache key {path} is missing or changed")


def drop_proof_key(home):
    PROOF_KEY[:] = []
    _KEY_STATE[:] = []
    shutil.rmtree(home, ignore_errors=True)


def require():
    """Refuses to run unless this platform's sandbox is there; the container is verified once."""
    kind = backend()
    if kind == "sandbox-exec":
        if not os.path.exists(SANDBOX_EXEC):
            raise SandboxUnavailable("sandbox-exec is missing: the hunt does not run unsandboxed")
        return kind
    if kind == "landlock":
        if not _VERIFIED:
            problems = container_problems()
            if problems:
                raise SandboxUnavailable("the hunt does not run unsandboxed on Linux: "
                                         + "; ".join(problems))
            _VERIFIED.append(True)
        return kind
    raise SandboxUnavailable(f"no sandbox for {sys.platform}: the hunt does not run unsandboxed")


def run(argv, cwd, writable, timeout, env_extra=None, mock_port=None):
    """Runs `argv` in `cwd`, sandboxed, with `writable`, a directory `fresh_dir` made for it, the
    only writable one. The run starts a process group of its own, with no input, and the group
    is killed when the run ends or times out. A process that left the group, `setsid` or
    `setpgid`, is not killed with it: if it still holds the run's output open once the group is
    killed, the run is a `SandboxUnavailable`, which fails the gate."""
    kind = require()
    check_proof_key()
    ident = identity(writable)
    writable = os.path.realpath(writable)
    home = os.path.join(writable, ".home")
    tmp = os.path.join(writable, ".tmp")
    os.makedirs(home, exist_ok=True)
    os.makedirs(tmp, exist_ok=True)
    # `UNA_HUNT_RUN`: the marker without which the oracle runner refuses to run (ADR 0115).
    env = {"PATH": "/usr/bin:/bin", "HOME": home, "TMPDIR": tmp, "LANG": "C.UTF-8",
           "UNA_HUNT_RUN": "1"}
    if PROOF_KEY:
        env["UNA_PROOF_KEY"] = PROOF_KEY[0]
    env.update(env_extra or {})
    if kind == "sandbox-exec":
        cmd = [SANDBOX_EXEC, "-p", profile(writable, mock_port)] + list(argv)
    else:
        cmd = landlock_argv(argv, writable, ident, env, mock_port)
    start = time.monotonic()
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        timed_out = False
    except subprocess.TimeoutExpired:
        timed_out = True
    finally:
        _kill_group(p.pid)
    if timed_out:
        try:
            out, err = p.communicate(timeout=DRAIN)
        except subprocess.TimeoutExpired:
            for pipe in (p.stdout, p.stderr):
                pipe.close()
            p.kill()
            p.wait()
            raise SandboxUnavailable(f"a process of the run {argv[0]} outlived its process "
                                     "group: its output was still open after the group was "
                                     "killed") from None
    stdout, stderr = out.decode("utf-8", "replace"), err.decode("utf-8", "replace")
    if timed_out:
        return Run(None, stdout, stderr, True, time.monotonic() - start)
    if kind == "landlock" and p.returncode == landlock.SETUP_FAILED and \
            landlock.ERROR_PREFIX in stderr:
        raise SandboxUnavailable("the run's Landlock confinement failed: " + stderr.strip())
    return Run(p.returncode, stdout, stderr, False, time.monotonic() - start)


def _kill_group(pgid):
    """Kills the process group of a run, what is left of it."""
    try:
        os.killpg(pgid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def run_fresh(argv, cwd, root, timeout, env_extra=None, mock_port=None):
    """`run` with a directory of its own under `root`, removed afterwards."""
    writable = fresh_dir(root, "run-")
    extra = {} if mock_port is None else {"mock_port": mock_port}
    try:
        return run(argv, cwd, writable, timeout, env_extra=env_extra, **extra)
    finally:
        remove_dir(writable, root)

