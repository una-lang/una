# Adversarial check of the v0.1.4 backport (PR #343, A′): summary

- **Candidate:** `a520f08` on release/v0.1. **Base:** v0.1.3 (`e7a7f48`). **Reviewer:** Claude (Una-2). **Author:** Una-1.
- **Also checked:** edition 0.2 on the released v0.3.0 binary and on main `1eed241`, for H-1's class.

## Verdict: Not ready

H-1, Critical: in edition 0.1, a decision on `T` values still reaches a public sink through a callback applied by `map`, `filter`, `fold` or `first_where`. It is runnable, and it leaks on v0.1.3 and on the candidate. Edition 0.2, and so v0.3.0, rejects every variant.

| Area | Verdict | Evidence |
| --- | --- | --- |
| #253, calls of function values | Ready | 13 cases (`cases/tapp`): every shape the candidate's rule covers is U4003, the controls are accepted |
| #253, prelude higher-order functions | **Not ready** (H-1) | 4 runnable leaks (`cases/run`), on v0.1.3 and `a520f08` |
| N-39 in edition 0.1 | Ready | 8 cases (`cases/n39`), capped at what each operation accepts |
| No other change | Ready | 301 programs of the v0.1 review corpora: every difference is an intended change in this review's cases |
| Edition 0.2 (v0.3.0, main) | Not affected | 15 cases (`cases/hof-0.2`), each rejected at the `T*` decision |
