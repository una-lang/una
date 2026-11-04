# Draft: ADR 0117 amendment for issue #1265 (per-name and per-zone bounds on the pools)

Status: draft for the project to append to `docs/adr/0117-concurrency-for-servers.md` as a dated amendment. The accepted text is not edited.

## Amendment (date of acceptance): the pools' bounds per name and per zone (#1265)

Point 1 of the amendment of 2026-10-27 (slice S7 merged with issue #1148) left the bounds per name and per zone of the withdrawn S7 resolver (review R7-10 and R7-13) to a later issue. Issue #1265 puts them on the bounded lookup and connect pools of `spec/24 §4` rule 2, which keep their sizes and their start before `main`:

1. **One lookup per name at a time,** names compared without case. A request for a name whose lookup is queued or running joins it and waits until its own deadline; a lookup whose requests all gave up never runs if it has not started, and else runs to its end, which a later request of the name joins. A name whose lookups stall holds one thread of the lookup pool. A request that joins a lookup earlier code started takes a time earlier requests shaped, as a wait in the pool's queue always could; it reaches a value only through a deadline (`spec/24 §4` rule 2, Out of model).
2. **A quarter of a pool per zone,** and at least 1, on both pools. The zone is a name's last two labels (not the name without its first label, as S7 designed: a wildcard zone answers at any depth, and `x.y.example.net` must not escape `example.net`'s share by its `y`), the `/24` of an IPv4 address, the `/48` of an IPv6 address; a connection's zone is that of the request's host. A step of a zone at its share waits in its zone's own line and joins the end of the pool's line when a step of its zone ends. The names under a public suffix of two labels, such as `co.uk`, share one zone: a cost in fairness, not in the bound.
3. **Space.** Lines are pruned of steps whose requests gave up once they double, so a pool holds O(waiting requests + threads) entries, at an amortized O(1) per step.
4. **Not taken from S7:** the 5-second memory of a failed name. It is a cache across requests, whose answer would depend on what earlier code looked up (the history that P8 removes), and the bounds above already hold without it.
5. **What stays open.** Many distinct zones that all stall (an attacker's own domains, or many `/24`s) can still hold every thread of a pool, each request ending at its own deadline: the residue `L_res` covers what those threads do, and the reactor's non-blocking resolution, when it exists, removes the pools.
