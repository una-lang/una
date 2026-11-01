---
name: review
description: Run the adversarial review of a PR with the reviewer agent and fold its findings before the merge. Use for every PR that touches crates/ (not docs-only).
---
Spawn the `reviewer` agent with the PR number (or branch) and the files it touches; it returns ranked findings. Fix every High and Medium on the branch (a `builder` or yourself), note Low items in the PR body as follow-ups or issues on the board, then rerun the build gate of the merge skill. At most two review rounds; leftovers become board issues and do not block the merge unless they are High.

Before spawning the reviewer, run a `scout` for the lookups it will need (callers of changed functions, the spec text the PR cites, the tests it should have touched) and pass the answers in the reviewer brief. Record the review with the harness (`harness.py record`) when the PR is run through it, naming the reviewer as distinct from the author; soundness or runtime changes get a second reviewer.
