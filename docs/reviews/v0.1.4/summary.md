# Adversarial check of the v0.1.4 backport (PR #343, A′): summary

- **Candidates:** `a520f08`, then `d38d2ec` after the fix of H-1 (978852e), on release/v0.1. **Base:** v0.1.3 (`e7a7f48`). **Reviewer:** Claude (Una-2). **Author:** Una-1.
- **Also checked:** edition 0.2 on the released v0.3.0 binary and on main `1eed241`, for H-1's class.

## Verdict at `d38d2ec`: Ready

- **H-1, Critical:** in edition 0.1, a decision on `T` values reached a public sink through a callback applied by `map`, `filter`, `fold` or `first_where`. It was runnable, and it leaked on v0.1.3 and on `a520f08`.
- **Fixed at `d38d2ec`:** the four repros, eight variants and the effectful and failing callbacks are rejected, and the controls are accepted.
- **Edition 0.2,** and so v0.3.0, rejected every variant throughout.
- At `a520f08` the verdict was Not ready.

| Area | Verdict | Evidence |
| --- | --- | --- |
| #253, calls of function values | Ready | 13 cases (`cases/tapp`): every shape the candidate's rule covers is U4003, the controls are accepted |
| #253, prelude higher-order functions | Ready at `d38d2ec` (H-1 fixed; Not ready at `a520f08`) | 4 runnable leaks on v0.1.3 and `a520f08` (`cases/run`), all U4003 at `d38d2ec`; 10 variants (`cases/recheck`) |
| N-39 in edition 0.1 | Ready | 8 cases (`cases/n39`), capped at what each operation accepts |
| No other change | Ready | 310 programs of the v0.1 review corpora: every difference is an intended change in this review's cases, and the 277 earlier programs are byte-identical |
| Edition 0.2 (v0.3.0, main) | Not affected | 15 cases (`cases/hof-0.2`), each rejected at the `T*` decision |
