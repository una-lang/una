# Recheck of #312 (PR #320, 8700345)

Raw results of the recheck. Una-3, 2026-10-07. Verdict: Clean.

- **Builds.** The tip was built with `cargo build --release -p una-cli -p una-rt` and `cargo build -p una-rt`. The runner is `../run.py`, with `UNA_TREE` and `Z3_DIR` set.
- **Cases.** 38 of 38 pass in all four configurations:
  - `sem.py`, 18 cases (`sem.log`, `results-sem.json`);
  - `edge.py`, 10 cases (`edge.log`, `results-edge.json`);
  - `recheck312.py`, 10 new cases (`recheck312.log`, `results-recheck312.json`).
- **Scaling.** `../scale.py` produced:
  - `scale-20000_40000_80000.json`: fold-append, fold-text, fn-value and lambda-local, linear with 0 copies (text: 1);
  - `scale-2000_4000_8000.json` and `scale-2000_8000.json` (`regress.log`): the regression sweep over every shape.
- **Unchanged.** keep-on-error stays quadratic, the IP-3 known limit.
