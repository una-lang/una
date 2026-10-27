# ADR 0123 S6: the `[serve]` host-driven form (design note)

- Issue: #1008, part of #897. Implementation plan step 6 of ADR 0123: the `serve` section (rule 16), its checks (U7008 in `ita.una`, U3027 and U3029 on `handle` and `on_stop`, U3012 on their `uses`, the usage errors) and its driver; the report's server entries (rule 22). It builds on the `diverges` and `on stop` of #942 (ADR 0116 S5, merged) and on the `HttpServer` capability of S5 (#1007, `docs/plan/0123-s5-design.md`).
- Base: ADR 0123 S5 (#1007) on S4 (#1006, the serve threads, `docs/plan/0123-s4-design.md`), itself on ADR 0117 S7 (#972) and ADR 0123 S3 (#1005), as those branches stand; the code is written on that tower once S5 builds. S4's amendment draft is taken as given: dedicated serve threads, so the `[serve]` driver is not "one accept loop per worker" (rule 19 as written) but the engine's delivery to tasks on the workers, two hand-offs per request (amendment B.6).
- Normative text: ADR 0123 rules 16, 17 (item 5), 18 (item 5: `on_stop` after the drain), 22; ADR 0122 rule 11 (the sections of `ita.una`) with the `serve` section the ADR 0123 rule 16 proposes; ADR 0116 rules 1 and 2 (the `drive` section and its usage errors, the template); `spec/31` gains §4 "The host-driven form" and `spec/30` the `serve` section; `spec/13 §6` for the report.
- Not in this slice: `test_request` (S7, #1009), the example servers and `--serve-script` (S7); the TLS server (#1059).

This note fixes how S6 is built, and lists the decisions it takes where the records leave a choice. Each decision takes the most secure option the records allow.

## 1. What this slice delivers, and what it leaves

| Part | This slice | Later |
| --- | --- | --- |
| The `serve` section of `ita.una`: `module`, optional `within` | `una-syntax` (`ita.rs`), `una-project` (`Serve`), with U7008 at each problem (§2) | |
| `una.toml` `[serve]` | none: edition `0.4` is written in `ita.una` only (ADR 0122), so `[serve]` in `una.toml` stays U7002 as any unknown table (decision S6-D1) | |
| The checks on the module: `handle`, `on_stop`, their shapes, `diverges`, `fails`, `uses` | `una-check` and `una-cli` (§3) | |
| The driver in both engines, with `within` when set | `una-interp` and `una-rt`, over S5's operations (§4) | |
| The host stop: drain, then `on_stop`, exit code 0; the hard stop | both engines (§4) | |
| The report's server entries, `out_of_model` additions and warnings (rule 22) | `una-cli` `report.rs`, within `una.report/2` (§5) | |
| Usage errors | both engines (§6) | |
| `test_request` for the tests of `handle` | none; the tests of this slice build a `Request` by its constructor, whose fields are `internal` and public to construct (rule 1) | S7 |

## 2. The `serve` section (rule 16, ADR 0122 rule 11)

- **Grammar**, in `spec/30`'s `section` production: `serve = "serve" NEWLINE INDENT "module" module_path NEWLINE [ "within" int_lit NEWLINE ] DEDENT ;`, as the `drive` section is written. `una-syntax`'s `ita.rs` reads it into `Serve { module: Located, within: Option<u64>, at: Range }`, beside `Drive`; `una-project`'s `Manifest` gains `pub serve: Option<Serve>`.
- **Checks, each U7008 at its line** (ADR 0122 rule 13 maps `una.toml`'s U7002 to U7008 in `ita.una`): `serve` beside a `run` section with `entry`, or beside a `drive` section (one program per package); a missing `module`; a `module` that is not a module of the package (checked where `drive`'s is, in the names pass, U7008 at the line); `within` outside 1 to 86,400,000 or not an integer; a line of another form; a section given twice. `serve` in a package of an earlier edition cannot occur: `ita.una` exists from edition `0.4` only.
- **`run grants` keeps its meaning.** A `serve` section with no `serve "..."` grant in `run grants` and none on the command line is a usage error at the run, exit code 2 (rule 16): the manifest alone cannot know the flags.
- **A dependency's `serve` section is not read** (ADR 0122 rule 12, as its `run` and `drive`): the reader of a dependency's `ita.una` ignores it, with the rule's existing path.

## 3. The module's shape (rule 16)

Checked in `una-cli`'s analysis after the names and types passes, where `driven_init` checks a `drive` module today (decision S6-D2: the same place, the same error kind):

- `pub handle(request: internal Request) -> internal Reply`: public, no type parameters, exactly one parameter whose type is `internal Request` of `std.host` (the labeled form; `Request` alone is U4001 at the declaration since the field types are `internal`, so the check asks for the exact header), result `internal Reply`. It MAY declare `uses` and `fails`. It MUST NOT be `diverges`: U3027 at the header (the existing code, "a function that diverges where it may not", reused with the message "`handle` of the `serve` module cannot diverge: every request ends after finitely many steps").
- `pub on_stop() -> Unit`, optional: public, no parameters, no type parameters, MAY declare `uses`, MUST NOT declare `fails` (U3029 at the header, the existing code of `[drive]`'s `on_stop`, with its message) nor `diverges` (U3027).
- **`uses` against the grants** (U3012): the `uses` of `handle` and `on_stop` are checked against the final grants as `main`'s are, by the existing path for the entry function, applied to both. `HttpServer` in `handle`'s `uses` is allowed (a handler may call `next`? no: decision S6-D3 below).
- **Decision S6-D3: `handle` MUST NOT use `HttpServer`.** Rule 16's synthesized program gives `answer` "`uses @server: HttpServer[reply]` and the `uses` of handle"; a `handle` that itself lists `HttpServer` could call `next` inside a request, taking exchanges the driver never answers, or `close` the server from a request. The ADR's rule 16 does not list it among `handle`'s rights, and every other capability of `handle` flows through the grants; so `uses @x: HttpServer[...]` on `handle` or `on_stop` is U3012 with the message "`handle` of the `serve` module cannot use `HttpServer`: the host answers each request with the reply `handle` returns". This is the most secure reading of rule 16 and is stated for the reviewer; if the project wants `handle` to hold `reply`, the check drops to `next` and `close` only.
- **A module without `handle` of this shape** is a usage error, exit code 2, as for `drive` (ADR 0116 rule 2): "the module `shop.api` of the `serve` section has no `pub handle(request: internal Request) -> internal Reply`", naming the first difference found (missing, not public, generic, parameters, result, `diverges`).
- **`handle` is an ordinary function** otherwise: tests call it directly (rule 16, "Testing"), it may be used as a value, and nothing else of the checker changes.

## 4. The driver (rule 16, "Meaning"; rules 17 and 18)

The meaning is the Una program of rule 16; the engines run an equivalent driver in Rust, as the `drive` driver runs `init`/`step`/`on_stop` (decision S6-D4: no synthesized Una or Core IR, so `una core` of the package does not change and the formal track sees `handle` as a function). In both engines:

1. **Start.** The serve threads start before `main` (S4 P8: a `serve` section is a static condition); the driver is the run's main task. It performs the run's first server operation at once (accepting begins, S4 §4).
2. **The loop.** A `for ever`-shaped loop: `next` (S5 §3), then `start` a task that runs `answer(exchange)`:
   - `answer` calls `handle(exchange.request)`; a failure of `handle`, when it declares `fails`, becomes `Reply(status: 500, headers: [], body: "")`, never the failure's payload (rule 16); then `reply(to: exchange.right, reply)` with its failure ignored (`if error use unit`: `NotHeld` cannot happen, the task holds the right; `BadReply` for a reply `handle` built with a forbidden header or a `204` with a body answers nothing, and the task's end then auto-answers `500`, rule 4.5, which is the right outcome for a program bug).
   - With `within w` set in the section, the task is started `within w` milliseconds: a task cut by its deadline is cancelled, and its reply right auto-answers `503` with `Retry-After: 1` (rule 4.5); without it, the host's `answer` deadline bounds what the peer waits (rule 16, "Why no `within` by default").
   - The started task takes its number from the run's task counter, so its path is `main/k` as for any started task; its placement is by ADR 0116 P6 (S4's amendment B.5).
   - `tasks` has no `at most` here: the grant's `/once` bounds the exchanges open, which bounds the started tasks (rule 15, "either is a limit on work in flight").
3. **Failures of `next`.** `Interrupted` (a host stop) ends the loop; `TimedOut` cannot happen (the driver has no deadline); `Closed` cannot happen (nothing calls `close`: the driver's program never does, and `handle` cannot, S6-D3).
4. **The stop** (rule 18 item 5): after `next` fails `Interrupted`, the driver waits until every started task has ended (the `tasks` statement's end), then calls `on_stop()` when declared, then the run ends with exit code 0. `on_stop` runs on the main task with the grants of the run. At the hard stop (ADR 0116 rule 16), every task is cancelled, the auto-answers of their rights are sent (`503`), `on_stop` does not run, and the run ends with U6007: the existing hard-stop path.
5. **Traces.** Each request's events (`next` in the driver's task, `handle`'s effects and `reply` in the started task) are the ordinary events of the two tasks, so a trace of a `serve` run reads as the trace of rule 16's program would; the reduced form of S5 §5 applies.
6. **Virtual time** (`--virtual-time`, S7's scripts): the driver is the same program; with one worker in native code (ADR 0117 rule 12) the started tasks run in delivery order.
7. **The executable.** `una build` of a package with a `serve` section writes the driver into the executable as the `drive` driver is written: the description carries `"serve": {"module": ..., "within": ...}` beside `"serve": true` of S3 (renamed `serve_grants` for the latter, a change within the description's own format, which is not a public schema).
8. **`una run` without a `serve` grant** is the usage error of §2.

## 5. The report (rule 22)

Within `una.report/2`, additive:
- `host_capabilities` lists `HttpServer` when a `serve:` grant exists, and gains, per grant, an object `{"address": "...", "scheme": "http" | "https", "limits": {...each item with its value...}, "checks_host": true | false}` under a new key `serve_grants` (decision S6-D5: a new key beside `host_capabilities`, which stays a list of names).
- `tcb` lists `una-http`'s server code as an entry `"server": "una-http/serve"` and, for `https` grants, `"tls": "rustls server"` (the `tls` entry waits for #1059 and is `null` until then).
- `out_of_model` gains, at its end, "what a server's peers send in reaction to its timing, and which requests its full queues refuse" and "what a peer receives when the host's deadlines cut an exchange", only for packages with a `serve:` grant or a `serve` section; `out_of_model` becomes a `Vec` built per package instead of the fixed array of eight.
- `warnings` (a new key, a list of strings, empty when none): a plain `serve:http://` grant on an address that is not loopback; `hosts=*`, `peer=none` or `peer_buffers=none`; `tls12`.
- The human output prints a "Servers" section with the same facts.

## 6. Diagnostics and usage errors

- **No new code.** U7008 for the `serve` section's problems in `ita.una`; U3027 and U3029 on `handle` and `on_stop`; U3012 for their `uses` and for `HttpServer` on `handle` (S6-D3); U4001 where `handle` returns a `secret` value in a field (the existing field rule).
- **Usage errors, exit code 2:** a `serve` module without `handle` of the shape of §3; a run with a `serve` section and no `serve:` grant; both messages name the module and the section.
- `spec/13 §4` does not change; `spec/13 §5` gains the two usage errors; `spec/30` the section; `spec/31 §4` the host-driven form, its checks and its meaning; `spec/13 §6` the report keys.

## 7. Tests

- **Conformance, project** (`ita.una` packages): `project/serve_section_ok` (a package with `serve` and a `serve` grant, `-- expect: ok`), `project/serve_with_entry` (U7008), `project/serve_with_drive` (U7008), `project/serve_without_module` (U7008), `project/serve_unknown_module` (U7008), `project/serve_within_out_of_range` (U7008 at the line), `project/serve_twice` (U7008).
- **Conformance, checks:** `authority/serve_handle_diverges` (U3027), `authority/serve_on_stop_fails` (U3029), `authority/serve_on_stop_diverges` (U3027), `authority/serve_handle_uses_ungranted` (U3012), `authority/serve_handle_uses_server` (U3012, S6-D3), `security/serve_handle_secret_reply` (U4001).
- **Usage errors**, in `crates/una-cli/tests/serve_section.rs`: a module without `handle` (each shape difference), `handle` not public, a run without a `serve:` grant; each with exit code 2 and its message.
- **Runs, both engines, under `--virtual-time`** through S4's scripted harness (S7 brings the flag): the health endpoint of the ADR's first example (`handle` returning `200` for `/health`, `404` otherwise) over keep-alive and `Connection: close`; a `handle` that fails (`500`, empty body, the failure's payload absent from the bytes); `within` cutting a slow `handle` (`503`, `Retry-After: 1`); a `handle` that returns a `204` with a body (`BadReply`, then the task's `500`); a host stop with one request in flight: it is answered, `on_stop` runs after it, exit code 0; the hard stop (U6007, `503`s); traces compared byte for byte between the engines, with the request's events in `main` and `main/k`.
- **Report:** a snapshot of `una report` for a package with a `serve` grant and a section: `serve_grants`, `tcb.server`, the two `out_of_model` lines, a warning for `hosts=*`; and for a package without, none of them.
- **Scaling** (native): 10,000 requests through the driver at saturation with a trivial `handle`, CPU time per request constant from 1,000 to 10,000 (the two hand-offs are O(1)).
- **The guide:** `docs/guide/configure/ita/serve.md`, the section and its checks, with the health endpoint.

## 8. Decisions

- S6-D1 `[serve]` lives in `ita.una` only (edition `0.4` is written there, ADR 0122); `una.toml` keeps refusing it as an unknown table.
- S6-D2 The module's shape is checked where `drive`'s is, with the same usage-error kind and the existing U3027/U3029 codes.
- S6-D3 `handle` and `on_stop` cannot use `HttpServer` (U3012 with a message of their own): the host answers each request with the reply `handle` returns, and a request cannot take exchanges or close the server. Stated for the project; the alternative (allow `reply` only) is one line.
- S6-D4 The driver is engine code equivalent to rule 16's program, not synthesized Una or Core IR; `handle` stays an ordinary function for the checker, the verifier and the formal track.
- S6-D5 The report adds `serve_grants` and `warnings` as new keys, and turns `out_of_model` into a per-package list; all within `una.report/2`.
- S6-D6 `tasks` of the driver has no `at most`: the grant's `/once` bounds the work in flight, as rule 15 states for either bound.
