---
name: merge
description: Merge a pull request into main safely - base check, build, tests, merge commit, cleanup of branch and worktree. Use for every merge.
---
1. `gh pr view <N> --json headRefOid,baseRefName,mergeable,files`; the base must be `main` (`gh pr edit <N> --base main` for a stacked PR whose base just merged - retarget BEFORE deleting the old base branch).
2. The head must contain origin/main: in the PR's worktree `git fetch origin && git merge origin/main`, resolve (git rerere is on), push.
3. Build gate per merge: `cargo build --workspace && cargo test --workspace --locked --no-fail-fast` (plus `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`); a PR touching una-rt, una-sys or una-codegen also runs `scripts/sanitize.sh` only when the change is in unsafe code or memory handling - otherwise it waits for the release gates.
4. `gh pr merge <N> --merge`; read the output, then `gh pr view <N> --json state,mergeCommit` before announcing anything.
5. Cleanup: `git worktree remove --force .claude/worktrees/<n>`, `git branch -D <branch>`, `git push origin --delete <branch>` (only if no open PR uses it as base: `gh pr list --base <branch>`). Only `main`, `release/v0.1` and `release/v0.3` are long-lived.
6. Board: the issue goes to Done; the changelog fragment lives in changelog.d/.
