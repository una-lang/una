# Summary

Toolchain: una 0.3.3, every package edition 0.2. Each package ran `una fmt` before every `una --json check`.

| Task | Check runs | Passing state | Other runs | Notes |
| --- | --- | --- | --- | --- |
| 1. greet | 1 | yes (check exit 0) | test 1 (1/1 pass), run 1 (printed `Hello, <user>! Your name has 10 letters.`) | Printing `Env` text needs a policy in a trusted module (`reveal.login_name`, internal -> public). |
| 2. orders | 1 | yes | test 1 (2/2 pass) | Exact output `{"count":2,"names":["a","b"],"total":5}` checked by an example. |
| 3. stock | 3 | yes | test 2 (1/1, then 3/3 pass) | Run 1: U0005 (`\r` is not an escape) and U1002 (handler parameter `body` clashes with op parameter). Secret token reaches the header through policy `token.api_token_header` (secret -> internal). |
| 4. tally | 1 | yes | test 1 (3/3 pass) | 3 obligations proven: `add_vote` ensures `counts.has(key: vote)`, `with_vote` ensures `result.has(key: vote)`, `votes_of` requires `counts.has(key: name)`. |
| 5. sensors | 1 | yes | test 1 (4/4 pass) | `read_all` fails with the first failure in list order; `count_answered` counts answers with `attempt` per task. |

Total: 7 check runs, 5 of 5 tasks passing, none gave up. DOC notes are in each task's LOG.md (orders, stock, tally, sensors).
