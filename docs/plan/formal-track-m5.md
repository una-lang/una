# Formal track: M5, a stateful host

- Issue: #46
- Plan: `docs/plan/formal-track.md` §7 (M5, renumbered with the owner's approval of 2026-10-09)
- Previous step: `docs/plan/formal-track-m4-completion.md`
- Date: 2026-10-09
- Status: **statement draft, for the coordinator's review before any proof**

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

**Events.** `out` (the console line), `req` (the request sent, a sink at `Http`'s `accepts`), `write` (a sink at `Files.write`'s `accepts`, path only), and `release`. A read sends nothing.

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

Non-vacuity: a package with only public writes, whose read failures stay `public`, and two runs whose public reads agree.

## 5. What stays assumed

Each assumption is a hypothesis of the theorem or part of the model, stated where it is used:

1. **(H1) and (H2)**, content-independence of failures and shapes. Una writes only UTF-8, so a read of invalid UTF-8 cannot follow a Una write (ADR 0085 decision 3). A file system where content decides a failure, such as a full disk, is resource consumption, outside the model (`spec/07 §1`).
2. **No other writer.** During a run, and between runs, only the package's own writes change the file system. Other programs and people are outside the model (`spec/07 §1`, external systems; decision 4).
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

## 7. Questions for the coordinator

1. **Shape for the public observer.** `WorldRel` asks equal shapes only when the file-state label is visible. Is that the reading you want of "low-equivalence of worlds"? A stricter one, equal shapes always, would make the theorem weaker: it would say nothing about a package with internal writes.
2. **`Files.read` as a sink.** The model makes a read send nothing. The real path of a read goes to the operating system. Should a read's path be a sink at `Files.read`'s `accepts` (`public`), as the checker already requires? I propose yes, as an event `read path`, for symmetry with `req`.
3. **The re-check (§6 item 4).** In M5, or after it?
