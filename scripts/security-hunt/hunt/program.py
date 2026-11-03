"""A generated program: a small package, its operator and its expected verdict class."""

import hashlib
import json
import os
import random
from dataclasses import dataclass, field

# The expected verdict classes of docs/plan/security-hunt-generators.md §1.
MUST_REJECT = "must-reject"
MUST_KEEP = "must-keep"
# A program `una check` must accept: a rejection is over-strictness, for triage (#513).
MUST_ACCEPT = "must-accept"
OPEN = "open"
# A program whose abort a secret decides, a termination channel T6 does not claim: its
# verdicts are recorded, never judged clean (#548).
RESIDUAL = "residual"
CLASSES = (MUST_REJECT, MUST_KEEP, MUST_ACCEPT, OPEN, RESIDUAL)

LOCK = """encoding_version = "una-smt-1"
solver           = "z3"
solver_version   = "5.1.0"
logic            = "QF_UFLIA"
step_budget      = 5000000
random_seed      = 0
"""


def derive_seed(campaign_seed, op, index):
    """The seed of one program, from the campaign's seed, its operator and its index.

    It depends on nothing else, so `repro` rebuilds any program from these three values.
    """
    digest = hashlib.sha256(f"{campaign_seed}:{op}:{index}".encode()).digest()
    return int.from_bytes(digest[:8], "big")


def rng_for(seed):
    """A private random generator, so that generators never share state."""
    return random.Random(seed)


@dataclass
class Program:
    """One generated package.

    `files` maps paths relative to the package root to their text. `expect` lists the
    diagnostic codes the operator expects when the class is must-reject, as a hint for
    triage: the verdict alone decides the finding.
    """

    op: str
    rule: str
    edition: str
    seed: int
    cls: str
    files: dict
    expect: list = field(default_factory=list)
    note: str = ""
    variant: str = ""
    # Where the oracle's varied literals are (una.Varied.place); empty when none.
    vary: list = field(default_factory=list)
    # What the re-check must say of the program's own declaration (F8): "read", "not-rechecked"
    # or "outside"; empty for the other operators, which `differ` judges as before. The manifest
    # writes it as `expect_recheck`: `differ`'s rows use `recheck` for the re-check's lines.
    recheck: str = ""
    # A residual program the full check must prove (family Q's twins, #813): any rejection of it
    # is an error (`drivers.judge_residual`).
    prove: bool = False
    # The command whose verdict judges the program (`drivers.COMMANDS`): `check`, `una check
    # --tier 0`, for every family but the registry's resolution forms, judged by `lock-check`,
    # `una ita lock --check` (family Y, ADR 0122). The manifest writes it only when it is not
    # `check`.
    command: str = "check"
    # The registries a `lock-check` program reads, {source: its directory in the program}: the
    # driver lists each in the run's own Itaoca (`mirrors.una`), since a mirror is named by an
    # absolute `file://` URL (`spec/30 §14` rule 1).
    mirrors: dict = field(default_factory=dict)

    def __post_init__(self):
        if self.cls not in CLASSES:
            raise ValueError(f"unknown class {self.cls}")

    @property
    def has_dependencies(self):
        """Whether the program is a package of edition 0.4 or later with needs (family D and Y):
        its Core export is `una.core/2`, which this release does not write (`spec/30 §12` rule
        7), so the oracle and the re-check comparison treat it apart."""
        text = self.files.get("ita.una")
        return isinstance(text, str) and "\nneeds\n" in text

    @property
    def name(self):
        suffix = f"-{self.variant}" if self.variant else ""
        return f"{self.op}-{self.edition}-{self.seed:016x}{suffix}"

    def write(self, root):
        """Writes the package under `root/<name>` and returns that directory."""
        base = os.path.join(root, self.name)
        for rel, text in self.files.items():
            path = os.path.join(base, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            if isinstance(text, bytes):
                # A registry's tiles and archives (family Y) are bytes.
                with open(path, "wb") as f:
                    f.write(text)
            else:
                with open(path, "w", encoding="utf-8") as f:
                    f.write(text)
        return base

    def manifest(self):
        """The manifest row: everything needed to judge and to reproduce the program."""
        return {
            "name": self.name,
            "op": self.op,
            "rule": self.rule,
            "edition": self.edition,
            "seed": self.seed,
            "class": self.cls,
            "expect": self.expect,
            "note": self.note,
            "variant": self.variant,
            "vary": self.vary,
            **({"expect_recheck": self.recheck} if self.recheck else {}),
            **({"prove": True} if self.prove else {}),
            **({"command": self.command} if self.command != "check" else {}),
        }

    def digest(self):
        """A digest of the files, to check that a reproduction is exact."""
        h = hashlib.sha256()
        for rel in sorted(self.files):
            data = self.files[rel]
            h.update(rel.encode() + b"\0" + (data if isinstance(data, bytes) else data.encode())
                     + b"\0")
        return h.hexdigest()


def write_manifest(path, programs):
    with open(path, "w", encoding="utf-8") as f:
        for p in programs:
            row = p.manifest()
            row["digest"] = p.digest()
            f.write(json.dumps(row, sort_keys=True) + "\n")
