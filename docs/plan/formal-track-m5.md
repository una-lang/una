# Formal track: M5, a stateful host

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7 (M5, renumbered with the owner's approval of 2026-10-09)
- Previous step: `docs/plan/formal-track-m4-completion.md`
- Date: 2026-10-09
- Status: done. Parts 1 to 3 are proven, with no `sorry` (§10). The re-check of `Files` is M5b (§12, #467); `Http`'s is M5c (#470)

## 1. Why

T6 (`t6_release`) assumes M3's assumption 3: the host is a function of the request, it is the same in both runs, and no operation of the program changes it. M3 found that the real host violates it twice:

- **S-14:** the file system is state. A write to an `internal` path decides whether a later read of a `public` path fails.
- **S-15:** a server decides some failures, `TooLarge`, `Protocol` and `TimedOut`, by the body and header values of the request, which did not label them.

ADR 0085 fixed both in the checker, in v0.3.4 and v0.1.6. M5 drops the assumption: it models the host as state, states T6 over it, and proves that the ADR's rules are what the theorem needs. S-14 and S-15 become theorems twice over:
- the old rules leak in the model (a counterexample);
- the new rules reject the probe programs, and T6 covers every program they accept.

## 2. The host model

The world of a run is a state the run threads, and one run's final world is the next run's first world.

```lean
structure World where
  fs  : FS          -- the file system
  srv : Server      -- the servers Http talks to
```

**Files** is abstract: a type of file systems with two operations, and the properties the ADR relies on as the model's hypotheses about it, not as definitions.

```lean
structure FileSystem where
  FS : Type
  shape   : FS → Shape                        -- which files exist, and the structure that decides failures
  read    : Path → FS → Option Text           -- none: the read fails
  write   : Path → Text → FS → Option FS      -- none: the write fails
  -- (H1) whether a read fails depends on the path and the shape only
  read_fails : shape a = shape b → ((read p a).isSome ↔ (read p b).isSome)
  -- (H2) whether a write fails, and the shape it leaves, depend on the path and the shape only,
  --      not on the content (ADR 0085 decision 3)
  write_shape : shape a = shape b → (write p c a).map shape = (write p c' b).map shape
```

A write that creates `a`, so that a later write to `a/b` fails, is a change of shape that (H2) allows. Reads return content, which the model labels `internal`, as `spec/14 §3` does.

**Http** is a server with its own state, which may record every request and decide later failures by earlier bodies:

```lean
structure Server where
  St : Type
  st : St
  request : Req → St → Option Resp × St      -- none: the request fails; Req holds the URL, headers, body
```

Nothing constrains it. ADR 0085 decision 7 makes every request's failure, and its response, `internal`, so a public observer cannot see what the server decides.

**Events.** `out` (the console line), `req` (the request sent, a sink at `Http`'s `accepts`), `write` (a sink at `Files.write`'s `accepts`, path only), `read` (a sink at `Files.read`'s `accepts`, `public`: the path goes to the operating system, as the checker already requires), and `release`.

**Every operation of the stateful capabilities is covered.** `spec/14 §3` gives `Files` two operations, `read` and `write`, and both are modeled. `spec/24 §1` gives `Http` four, `get`, `post`, `put` and `delete`. They differ only in their method and in whether they send a body, so one form covers them: `req m url headers body key`, with `m` the method and `body` absent for `get` and `delete`. All four accept `internal`, fail with `HttpError` and return an `internal` `Response`, so one rule types them. The abort of a header value with a control character (U6004, N-40) is outside the model, as aborts are (`spec/07 §1`).

| Operation (`spec/24 §1`, `spec/14 §3`) | Model form | Sinks, at `accepts` | Failure label | Result |
| --- | --- | --- | --- | --- |
| `Files.read(path)` | `read path` | path, `public` | `pc ⊔ ℓ(path) ⊔ fileState` | content, `internal` |
| `Files.write(path, content)` | `write path content` | path and content, `internal`; also `pc ⊔ ℓ(path) ⊑ fileState` | `pc ⊔ ℓ(path) ⊔ fileState` | `Unit` |
| `Http.get(url, headers, key_name)` | `req "get" args` | every argument, `internal` | `pc ⊔ ℓ(args) ⊔ internal` (headers) | `Response`, `internal` |
| `Http.post(url, headers, body, key_name)` | `req "post" args` | every argument, `internal` | `pc ⊔ ℓ(args) ⊔ internal` | `Response`, `internal` |
| `Http.put(url, headers, body, key_name)` | `req "put" args` | every argument, `internal` | `pc ⊔ ℓ(args) ⊔ internal` | `Response`, `internal` |
| `Http.delete(url, headers, key_name)` | `req "delete" args` | every argument, `internal` | `pc ⊔ ℓ(args) ⊔ internal` | `Response`, `internal` |

In the model the request's arguments are texts, and its response is the body's text: the structure of `Header` and `Response` adds nothing the labels above do not cover.

**Evaluation** threads the world: `eval : Nat → Program → World → Env → Expr → Trace × Res × World`.

## 3. The typing: how ADR 0085 enters

- **The file-state label** is a field of the program, `p.fileState : Label`, capped at `internal`. The model states it declaratively, by the condition it must meet, not by the call-graph fixpoint the checker computes:
  - **T-Write:** `pc ⊔ ℓ(path) ⊑ p.fileState`, and `pc ⊔ ℓ(path) ⊑ accepts(Files.write)`. Its failure is labeled `pc ⊔ ℓ(path) ⊔ p.fileState`.
  - **T-Read:** its failure is labeled `pc ⊔ ℓ(path) ⊔ p.fileState`, and its result `internal` (the content).
  - **Calls and function values:** a function that writes carries a new effect flag, `w`. `Bound` asks `pc ⊑ p.fileState` for it, as it asks `pc ⊑ accepts` for `op` today.
  - That is the declarative form of decision 1's "the highest program counter its code can be entered under". A call of a writing function under `pc` is the entry of its write under `pc`. A function value or a `via` function is entered wherever it is called, which `Bound` at each `apply` covers.
- **T-Request** (`Http`): its arguments are sinks at `accepts`, and its failure is labeled by every argument (`pc ⊔ ℓ⁺(url, headers, body)`), as decision 7 states. `Header` declares its value `internal`, so in the model the headers argument always contributes `internal`: every request's failure is `internal`, and its response `internal` (`hget`).
- **Effects:** `Eff` gains `w`, and the host operation `get` becomes `req`. `Files.read` is an effect of its own (`r`), since its failure is decided by state.

## 4. The theorem

**Low-equivalence of worlds** for an observer at `L`:

```lean
def WorldRel (p : Program) (L : Label) (w₁ w₂ : World) : Prop :=
  (Vis p L (.ofLabel p.fileState) → shape w₁.fs = shape w₂.fs) ∧   -- the shape, when the observer may see it
  (Vis p L (.ofLabel internal) → w₁ = w₂)                           -- everything, for an observer who sees internal
```

**A premise of well-formedness, not an accident of the host table:** every sink of a stateful host capability accepts at most `internal`. For an observer who sees `internal`, the theorem keeps the two worlds equal, and that holds only if no host state can receive data above `internal`. The sinks are:
- `Files.write`'s path and content, which accept `internal` (`spec/14 §3`);
- `Files.read`'s path, which accepts `public`;
- every argument of the four `Http` operations, which accept `internal` (`spec/24 §1`).

`Keys.hmac`'s data accepts every label. That is sound only because `Keys` is stateless, which is assumption 6 of §5.

- For a `public` observer, in a package that writes only at `public` (`fileState = public`), two worlds must have the same shape, but their contents and servers may differ.
- When some write is `internal` (`fileState = internal`), the shapes may differ too. Every `Files` failure is then `internal`.
- An observer at `internal` or above sees everything the host holds.

**T6 with a stateful host** (`t6_host`): for two runs of a body the labeled typing accepts, under the hypotheses of `t6_release`, from related environments and `L`-related first worlds `w₁ ~ w₂`, when both runs end:
- the observer sees the same events with related outcomes, or the runs diverge at a release (`BRelR`, as today);
- and **the final worlds are `L`-related**.

The second conclusion is new. By induction it gives **across runs** (ADR 0085 decision 4): a sequence of runs of the package from related first worlds keeps them related, so one run's internal write cannot be observed by a later run's public read. The file-state label is static and joins every write of the package, which is what the induction needs.

**S-14 and S-15 as theorems:**

| Theorem | Statement |
| --- | --- |
| `s14_old_leaks` | In the model with the old rule (a read's failure labeled by its path only), S-14's probe is typed, and its two runs, with the secret path `a.txt` and `b.txt`, from the same world, print different public lines. Assumption 3 was needed, and its violation is a leak. |
| `s14_untyped` | Under ADR 0085's rules, the probe has no typing: its read's failure is `internal`, so the branch that prints runs under `internal`, and `Console` accepts `public`. The ADR's conformance shapes (write then read, write then count, a write under a condition, read before write) get the same theorem. |
| `s15_old_leaks`, `s15_untyped` | The same for a request whose failure a stateful server decides by an `internal` body. |
| `t6_host` | Every program the new rules accept is noninterferent, with a stateful host. |

Non-vacuity, two examples:
- **public writes:** a package whose writes are all `public` (`fileState = public`). Its read failures stay `public`, and two runs from worlds of the same shape agree on them;
- **an internal write, the S-14 fix proven positively:** a package with an `internal` write (`fileState = internal`). Two runs, from worlds whose shapes differ, agree for a `public` observer, because every `Files` failure is then `internal`.

## 5. What stays assumed

Each assumption is a hypothesis of the theorem or part of the model, stated where it is used:

1. **(H1) and (H2)**, content-independence of failures and shapes; and, for the re-checker's erasure, **(H3)**: a response does not depend on annotations, which a host given text meets (§11). Una writes only UTF-8, so a read of invalid UTF-8 cannot follow a Una write (ADR 0085 decision 3). A file system where content decides a failure, such as a full disk, is resource consumption, outside the model (`spec/07 §1`). **The real host does not meet H1 today (S-16):** `Files.read` fails on a file that is not valid UTF-8, which a file written by another program can be. The fix (Una-1, ADR in progress) makes invalid text abort the run, outside the model as aborts are. After it ships, H1 holds of the real host.
2. **No other writer.** During a run, and between runs, only the package's own writes change the file system. Other programs and people are outside the model (`spec/07 §1`, external systems; decision 4). That includes a server the package talks to: the model's servers do not write the file system (§10).
3. **One package.** The file-state label joins the writes of the whole program. That holds while a package imports no other package (decision 5); the ADR records what must change if that ever does.
4. **The checker computes a sound file-state label.** The model states it declaratively, at each write and each call of a writing function. That the checker's fixpoint over the call graph, its probe pass and its caps (lambdas at `internal`, values at their bound) meet it is the re-check's to check, not the theorem's. See §6.
5. **Timing, and the termination of the server**, are outside the model, as they are today. T6 is termination-insensitive.
6. **The other host capabilities** (`Env`, `Clock`, `Console` as a sink, `Keys`) keep M3's assumption 3, since they are stateless. `Keys.hmac`'s result rule (ADR 0039) is not part of M5.

## 6. Scope and order

1. **The model and the counterexamples first:** `World`, the threaded `eval`, the new rules, `s14_old_leaks` and `s15_old_leaks`. They show the model captures both findings before anything is proven about the fix.
2. **The untypable probes:** `s14_untyped`, `s15_untyped`, and the ADR's conformance shapes.
3. **`t6_host`**, with the final-world conclusion, then the across-runs corollary. This is the bulk: `eval`'s new result touches the unary and binary halves of every proof, so `t6_release` is rebuilt over it. Its statement for a world that no write changes is today's.
4. **Re-check (optional, after):** `una core --json` would export the file-state label and the `Files` and `Http` operations, and `Import.lean` would read them and check the declarative conditions, as A1 to A3 did for the rest. This needs export keys, #443-style. I propose to leave it for after the theorem, as its own item.

**Effort.** Parts 1 and 2 are small. Part 3 rebuilds every proof over the threaded world. On M3 and M4's evidence, the cost is in where the model and the checker disagree, which parts 1 and 2 bring out first.

## 7. What the 638 accepted declarations rely on today

"No accept without a theorem" applies to the host assumption too. The re-check's 638 accepted declarations (after A3) use these operations, counted over their bodies and signatures in the corpus:
- `Log.write` (182 signatures, 84 bodies), `Vault.keep` and `Ledger.record` (1 each): capabilities of the packages themselves, whose handler bodies are not in the reach set (`formal-track-m4-completion.md` §3);
- `Console.print` (7): a stateless sink, assumption 6.

None uses `Files`, `Http`, `Env` or `Keys`. F1's host operation `get` is never read from Core: the import reads only an operation from a scalar to `Unit` that does not fail (`Import.lean`'s header), which no `Http` or `Files` operation is. Every function is read with `get := false`. So no accepted declaration rests on the stateless-host assumption that M5 removes, and none moves.

A handler of a package's capability could itself call `Files` or `Http`. Its body is outside the reach set, as §3 of the completion plan states, and T4 and F2 (M7) model handlers.

## 8. The coordinator's answers (2026-10-09)

1. **`WorldRel`:** the reading above, with the premise that every stateful sink accepts at most `internal` (§4).
2. **A read's path is a sink:** yes, an event `read` at `Files.read`'s `accepts` (§2).
3. **The re-check:** after the theorem, as its own item, M5b. §7 answers which accepted declarations rely on the host today.
4. **Every operation is covered:** `Files.read`, `Files.write`, and `Http`'s `get`, `post`, `put` and `delete` (§2).
5. **Two non-vacuity examples:** public writes, and an internal write seen by a public observer (§4).

Parts 1 and 2 go first. The counterexamples, `s14_old_leaks` and `s15_old_leaks`, go to the coordinator before `t6_host` starts.

## 9. Questions asked (answered in §8)

1. **Shape for the public observer.** `WorldRel` asks equal shapes only when the file-state label is visible. Is that the reading you want of "low-equivalence of worlds"? A stricter one, equal shapes always, would make the theorem weaker: it would say nothing about a package with internal writes.
2. **`Files.read` as a sink.** The model makes a read send nothing. The real path of a read goes to the operating system. Should a read's path be a sink at `Files.read`'s `accepts` (`public`), as the checker already requires? I propose yes, as an event `read path`, for symmetry with `req`.
3. **The re-check (§6 item 4).** In M5, or after it?

## 10. What is proven

M5 staged the model in `formal/UnaFormal/F1H/`, where `scripts/formal.sh` passed with 140 stated theorems, none of them depending on `sorry`. #459 folded it into F1 (§11): the files below are now in `formal/UnaFormal/F1/`.

**The model.** M5 staged F1's core beside F1, as F1H, with the host as state, and #459 made it F1 itself:
- `Syntax.lean`: the forms `read`, `write` and `req`, and host effects (`HostFx`: read, write, request) in function types;
- `Eval.lean`: an abstract `Host` (file system, servers), with H1 and H2 as `Host.Lawful`, and an evaluation that threads the `World` and returns the world it leaves;
- `LTyping.lean`: ADR 0085's rules. Failures go through `Program.fsFail` and `Program.reqFail`. `Bound` has a write case that asks `pc ⊑ fileState`. The switch `oldHost` states the rules before the ADR.

Every proof of M3 and M4 is carried over (T1, the unary and binary halves, instances, B1).

**What the re-checker's "accepted" rests on**: §11.

| Part | Theorems |
| --- | --- |
| 1, the old rules leak | `OldLeaks.s14_old_typed`, `s14_old_leaks`, `s15_old_typed`, `s15_old_leaks`, on a lawful host (`demo_lawful`) |
| 2, the new rules reject the probes | `Untyped.s14_untyped`, `s15_untyped`, `s14rbw_untyped` (read before write), `s14cond_untyped` (write under a secret condition): no typing for any file-state label and any annotation. `NewRules.s14_rejected`, `s15_rejected`, `shapes_rejected` instantiate them, and `s14_same_probe` and `s15_same_probe` show by `rfl` that they are the very programs `OldLeaks` types. |
| 3, T6 with a stateful host | `t6_host`, `t6_host_ground`: two runs from worlds the observer cannot tell apart (`WorldRel`), in related environments, show it the same events and related outcomes, and leave worlds it cannot tell apart. `t6_release_host` is the same with declassification. `across_runs` and `across_runs_events` cover a sequence of runs (decision 4). |
| Non-vacuity | `HostExamples.exA`, `exA_runs`: public writes, from worlds with the same files and different contents. `exB`, `exB_runs`: an internal write, from worlds with different files and different secret paths, whose runs return 1 and 2 and still agree for a public observer. |

**The premises** (`Program.HostWF`, `Binary.lean`):
- the rules of ADR 0085 (`oldHost = false`);
- every stateful sink accepts at most `internal`: `Files.read`'s path, `Files.write`'s path and content, and every `Http` argument;
- what the host returns, a read's content and a response, and what decides a request's failure, are at least `internal`.

`acceptsRead ⊑ internal` was added in the proof. Without it, a read's content could reveal a path the observer cannot see. `Files.read` accepts `public`, so it holds. The host must also be lawful: H1 and H2 (`Host.Lawful`).

**What Una-1's review found about the real host** (2026-10-09):
- **H1 does not hold of today's `Files.read` (S-16).** A read of a file that is not valid UTF-8 fails, so whether a read fails depends on content, not only on the path and the shape. `una check` accepts a program that prints, at `public`, whether an `internal` file is valid UTF-8 (v0.3.4). The fix is in the runtime, under its own ADR. Once it lands, H1 holds of the real host. The theorem needs no change: it assumes H1 and says so. **Fixed in v0.3.5** (ADR 0086): invalid text aborts the run with U6005, and H1 holds of the real host.
- **The file system and the servers are disjoint.** The model keeps their states apart, so a request never changes the file system. A server on the same machine that writes into the granted directory would join them. A request at an `internal` program counter would then change the shape, and a later read whose failure is `public` would reveal it. This is outside the model, as races and other processes are (§5, assumption 2).

**How the proof goes.**
- The unary half records that a run changes the file system only if its trace holds a write, and the servers only if it holds a request. A write is allowed only at a program counter below the file-state label.
- So a run at a program counter the observer does not see leaves a world it cannot tell from the one it started from (`confined`).
- When the observer sees `internal`, the two worlds are equal and so are the sinks' arguments, so the host operations coincide.
- Otherwise, contents and responses are hidden. Read and write failures agree by H1 and H2 when the observer sees the file-state label, and so the path. A request's failure is hidden.

## 11. One model (#459)

F1H is folded into F1, so one model remains: `formal/UnaFormal/F1/`, with the host forms, the threaded world and the host effects. F1's stateless operation `get` is gone; an `Http.get` is a request. The re-checker and its soundness theorems are ported to it.

**The re-checker's labels for the host** (as #459 left them; M5b replaces them, §12). The importer reads no `Files` or `Http` operation, and the checker rejects the host forms until M5b. Every program the re-checker certifies gives the host the strictest labels `Program.HostWF` allows, every stateful sink at `⊥` and every result and failure floor at `internal` (`Import.mkProgram`). They decide no verdict, and they make `HostWF` hold by construction: `progOk` checks it (`Check.hostWfB`), so `progOk_sound` proves it.

**Binding on M5b** (the Coordinator, from Una-1's review, 2026-10-09). Once the importer reads host operations, the host's labels in `Import.mkProgram` MUST come from the document's host signatures (`Files.read`, `Files.write` and `Http`'s `accepts`, result labels and failure floor), not be fixed by construction. Otherwise the certificate would describe a host other than the document's. This is an acceptance criterion of M5b. The importer no longer reads `Http.get`'s labels from the document, so its demotion for them is gone; no declaration of the corpus had it.

**On any host.** `t6_release_host` asks the host to be lawful, and the real host met H1 only from v0.3.5 (S-16). A run of a program without host operations never reads or writes a file, so the premise is never used:
- `eval_hostFree` (`HostFree.lean`): a program none of whose functions performs a host operation, run on a body that performs none, from values whose closures perform none, runs on any host as on the host without state (`Host.unit`), and leaves the world as it found it.
- `t6_release_hostFree`: `t6_release_host`'s conclusion on any host, lawful or not, for such a program.
- `t6_release_erasure_obs_hostFree`: `t6_release_erasure_obs` on any host, for a document whose program, body and inputs perform no host operation.
- `Import.finalize` asks each certificate's program to be host-free (`Program.hostFreeB`), and `finalize_sound` states it. Since M5b, a program that uses `Files` is certified too, on a lawful host (§12).

So an "accepted" of the re-checker is a corollary of `t6_release_erasure_obs_hostFree`, and asks nothing of the real host. Its premises are the certificate's, the reading of `una.core/1` (`Import.lean`, the trusted base), and inputs whose closures perform no host operation.

**Non-vacuity** (`HostFreeExample.lean`). `s16` is a host whose read fails on a file holding invalid text, as the real `Files.read` did before v0.3.5 (S-16). Two file systems with the same files, one of them holding such a file, are told apart by a read (`s16_unlawful`). `s16_guesses` applies `t6_release_hostFree` to `ReleaseExample`'s program from exactly those two file systems, and `s16_caller_obs` applies `t6_release_erasure_obs_hostFree` to `MonomorphExample`'s document.

**Other changes.**
- Fuel monotonicity (`eval_mono`, `t6_ended`) is ported, from `t6_host_ground`.
- Erasure (`eval_anon`, `eval_erasure`) asks a third property of the host, beside H1 and H2: **(H3) its responses read no annotation** (`Host.AnonInvariant`). A request whose arguments differ only in the annotations inside closures gets the same response. The model passes values to the host, and a real host is given their text, so it holds. `Host.unit` meets H1, H2 and H3, so the any-host theorems below ask none of them.
- F1's examples run on any lawful host, or on `Host.unit`.
- `t6_release_host_agree` is `t6_release_agree` with a stateful host.

**Validation.**
- `scripts/formal.sh`: **PASS**, 144 theorems, no `sorry`. F1's `t6`, `t6_ground` and `t6_release` are gone; `t6_host`, `t6_host_ground` and `t6_release_host` stand for them. The theorems M5 proved twice are listed once.
- The corpus (`main`'s export with #443's keys, 1,327 documents) is unchanged: all 4,203 verdicts, with their reasons, are identical, and 638 declarations are accepted.
- The replay of S-01 to S-12 is unchanged: identical to the replay at A3.

**Events are first-order (#457).** `t6_release_erasure_obs` and `t6_release_erasure_obs_hostFree` no longer assume that the document's events carry first-order values. `events_firstOrder` proves it of every typed run, in a program within B1, on any host:
- a release carries its `via` function's value, which the unary half now records as typed at the policy's `to` (`Allowed`), and B1 makes `to` scalar;
- a request carries its arguments, which T-Request types as texts, and `Allowed` now records it.

The document's events erase to those of `P₁`'s typed run, so they are first-order too (`Event.firstOrder_anon`).

## 12. M5b: the re-check of `Files` (#467)

The re-check reads `Files.read` and `Files.write` (Coordinator's decision N-74). `Http`'s real signature does not fit the model's request: it takes `List[Header]` and `Option[Text]` and returns a `Response` record. So it stays "not re-checked: Http's signature is outside F1", and the model's extension is M5c (#470).

**The export.** `una core --json` gains a document-level key, `file_state`: the package's file-state label (ADR 0085 decision 1), which the checker computes (`spec/13 §8`).

**The labels come from the document** (the binding rule of §11). `Import.HostLabels` reads:
- `Files.read`'s `accepts` and result label, and `Files.write`'s `accepts`, from the document's host signatures;
- `Http`'s, likewise;
- the file-state label, from `file_state`.

`Header`'s value label is not exported, so a request's failure floor stays `internal`, as `Header` declares; M5c reads it. A declaration that reaches `Files` in a document without `file_state` is not re-checked. A document that declares its own capability named `Files` or `Http` is not re-checked either: `uses` names a capability without its module, so it could not be told from the host's.

**`file_state` is untrusted** (`spec/13 §8`, "may rely on"). The checker types `read` and `write` by the model's rules (`Program.fsFail`; the write's `pc ⊔ ℓ(path) ⊑ fileState`; its `accepts`), with their cases in `CheckSoundCore`. `progOk` caps the file-state label at `internal` (`Check.hostWfB`). So a value too low gives a rejection or a declaration not re-checked, never an accepted one. A value too high only labels failures higher.

**The whole package** (Una-1's finding on #471, CONFIRMED). The re-check checks `file_state` only against the writes of the programs it certifies. A write elsewhere never meets it: in a declaration rejected or not re-checked, in a handler, or in a node the reading cannot tell from one. Under a lowered label, a reader that branches on a read's failure would then be accepted, which is S-14's shape. So `Import.guardFiles` looks for every write site of the document, in a declaration of any kind (`writeSite`). The test is default-deny (the coordinator's rule, after Una-1 found renamed nodes that escaped a list of names):
- an operation is safe only when its capability path resolves to a capability the document declares (its own or the host's), its `capability` field is that path's name, its operation is one the capability declares, and it is not `std.host`'s `Files.write`;
- a `uses` entry is safe only when it names a declared capability, grants only operations that capability declares, and does not grant `Files.write`;
- every other node counts as a site, so a renamed, stripped or inconsistent node demotes instead of escaping.

Unless every declaration that holds one is a function accepted in a certified program, every declaration accepted on a lawful host is not re-checked: "the file-state label is not verified for the whole package". `guardFiles_sub` proves that the guard only demotes. `ImportTests` checks it on Una-1's repros, each leaving the reader not re-checked: a writer rejected, one not re-checked through `Http`, one with its capability path stripped, one whose capability and `uses` are renamed while its path stays `Files` (the h-shape), one whose capability and path disagree, and a handler that writes. The honest control is still accepted.

**The tamper test** (`ImportTests`). `mark` writes an `internal` path, so the true label is `internal`, and the export claims `public`. `mark` is rejected ("a write whose path or program counter is above the file-state label"), and `probe`, whose read's failure the lowered label would make public, is not re-checked.

**Certificates.**
- A program without host operations keeps the any-host theorem.
- A program that uses `Files` is certified too. Its theorem is `t6_release_erasure_obs`, on a lawful host (H1 to H3), and the verdict's note says so: "rests on a lawful host (H1 to H3): the program uses `Files`" (`Import.withHost`, stated by `finalize_sound`).
- H1 holds of the real host from v0.3.5 and v0.1.7 (S-16).

**Non-vacuity** (`FilesCertExample.lean`). A program that writes a file is certified by `progOk` evaluated (`certified`), and `t6_host_ground` relates its runs on the lawful host `files` from two file systems that differ in a file's content (`mark_runs`).

**The corpus.** All 1,329 documents were re-exported with the new key, and `main`'s checker and this one were run on the same export:
- 4 declarations move from not re-checked to accepted, each with the lawful-host note: `import_std_host.load`, `n39_ok_public_path_content_internal.probe`, `s14_ok_failing_writer.mark` and `.probe`. `s14_ok_helper_public_call.mark` stays not re-checked: `probe`, in its package, has a `uses` that grants `Files.write` and is not re-checked;
- `import_std_host.greet` stays accepted, with the note, since its certificate's program uses `Files`;
- 5 declarations stay not re-checked, with another reason;
- none moves into rejected. Accepted: 638 → 642.

The replay of S-01 to S-12 is unchanged.
