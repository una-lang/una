"""Byte diff of v0.1.3 against the v0.1.4 candidate over the v0.1 review corpora: every
package and single file under the adversarial and case directories of docs/reviews, plus this
review's cases. For each target, `una --json check` (tier 1) and, when the target has a `main`,
`una --json run` with its `-- grant:` directives, compared byte for byte with the exit status.
Usage: compare.py <repo> <v0.1.3 una> <candidate una>."""
import os, re, subprocess, sys
repo, old, new = sys.argv[1:4]
roots = [os.path.join(repo, "docs/reviews", d) for d in sorted(os.listdir(os.path.join(repo, "docs/reviews")))
         if os.path.isdir(os.path.join(repo, "docs/reviews", d))]
targets = []
for root in roots:
    for dirpath, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in ("target", "results")]
        if "una.toml" in files:
            targets.append(dirpath); dirs[:] = []; continue
        targets += [os.path.join(dirpath, f) for f in files if f.endswith(".una")]
targets.sort()
def runs(una, target):
    cwd = target if os.path.isdir(target) else os.path.dirname(target)
    out = []
    p = subprocess.run([una, "--json", "check", target], capture_output=True, timeout=900, cwd=cwd)
    out.append(("check", p.returncode, p.stdout))
    src = target if os.path.isfile(target) else None
    text = ""
    if src:
        text = open(src, errors="replace").read()
    else:
        for dp, _, fs in os.walk(os.path.join(target, "src")):
            for f in fs:
                if f.endswith(".una"):
                    text += open(os.path.join(dp, f), errors="replace").read()
    if "pub main()" in text:
        args = []
        for g in re.findall(r"^-- grant: (.+)$", text, re.M):
            for one in g.split(","):
                args += ["--grant", one.strip()]
        p = subprocess.run([una, "--json", "run", target] + args, capture_output=True, timeout=900, cwd=cwd)
        out.append(("run", p.returncode, p.stdout))
    return out
same = 0
differ = []
for t in targets:
    a, b = runs(old, t), runs(new, t)
    if a == b:
        same += 1
    else:
        differ.append(t)
        for (ka, ca, sa), (kb, cb, sb) in zip(a, b):
            if (ca, sa) != (cb, sb):
                print(f"DIFFERS {ka}: {os.path.relpath(t, repo)} exit {ca} -> {cb}")
print(f"{same} targets identical, {len(differ)} differ, of {len(targets)}")
