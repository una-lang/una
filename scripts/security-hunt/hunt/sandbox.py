"""Runs a command in the hunt's sandbox (docs/plan/security-hunt-oracle.md §6).

- A fresh directory per run, under the campaign's root, as the only writable place.
- No network, except loopback to one port when a mock server is given.
- A clean environment: `HOME` and `TMPDIR` point inside the run's directory.
- A wall-clock timeout.

It refuses to run without `sandbox-exec`: the coordinator's condition 1 has no fallback.
"""

import os
import shutil
import subprocess
import tempfile
import time

SANDBOX_EXEC = "/usr/bin/sandbox-exec"


class SandboxUnavailable(RuntimeError):
    pass


def available():
    return os.path.exists(SANDBOX_EXEC)


def _quote(path):
    return '"' + path.replace("\\", "\\\\").replace('"', '\\"') + '"'


def profile(writable, mock_port=None):
    """The sandbox profile: everything is allowed but the network and writes, which are
    denied except under `writable` and to the null and terminal devices."""
    rules = [
        "(version 1)",
        "(allow default)",
        "(deny network*)",
        "(deny file-write*)",
        f"(allow file-write* (subpath {_quote(os.path.realpath(writable))}))",
        '(allow file-write* (literal "/dev/null") (literal "/dev/tty") (literal "/dev/dtracehelper"))',
    ]
    if mock_port is not None:
        rules.append(f'(allow network-outbound (remote ip "localhost:{int(mock_port)}"))')
        rules.append('(allow network-bind (local ip "localhost:*"))')
    return "\n".join(rules) + "\n"


class Run:
    """A finished run: its exit code, output, and whether it timed out."""

    def __init__(self, code, stdout, stderr, timed_out, seconds):
        self.code = code
        self.stdout = stdout
        self.stderr = stderr
        self.timed_out = timed_out
        self.seconds = seconds


def fresh_dir(root, prefix):
    os.makedirs(root, exist_ok=True)
    return tempfile.mkdtemp(prefix=prefix, dir=root)


def remove_dir(path, root):
    """Removes a run's directory, only if it lies under `root`."""
    real, base = os.path.realpath(path), os.path.realpath(root)
    if not base or base == "/" or os.path.commonpath([real, base]) != base or real == base:
        raise ValueError(f"refusing to remove {path} outside {root}")
    shutil.rmtree(real)


def run(argv, cwd, writable, timeout, env_extra=None, mock_port=None):
    """Runs `argv` in `cwd`, sandboxed, with `writable` the only writable directory."""
    if not available():
        raise SandboxUnavailable("sandbox-exec is missing: the hunt does not run unsandboxed")
    home = os.path.join(writable, ".home")
    tmp = os.path.join(writable, ".tmp")
    os.makedirs(home, exist_ok=True)
    os.makedirs(tmp, exist_ok=True)
    env = {"PATH": "/usr/bin:/bin", "HOME": home, "TMPDIR": tmp, "LANG": "C.UTF-8"}
    env.update(env_extra or {})
    cmd = [SANDBOX_EXEC, "-p", profile(writable, mock_port)] + list(argv)
    start = time.monotonic()
    try:
        p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, timeout=timeout)
        return Run(p.returncode, p.stdout.decode("utf-8", "replace"),
                   p.stderr.decode("utf-8", "replace"), False, time.monotonic() - start)
    except subprocess.TimeoutExpired as e:
        return Run(None, (e.stdout or b"").decode("utf-8", "replace"),
                   (e.stderr or b"").decode("utf-8", "replace"), True, time.monotonic() - start)
