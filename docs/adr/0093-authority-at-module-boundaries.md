# 0093. Authority at module boundaries

- Status: Proposed
- Date: 2026-10-11
- Deciders: Una project (pending acceptance)
- Resolves: issue #520, part B ("with which authority"). Part A is ADR 0092, part C ADR 0094. Related: #59 (attenuation by parameters), ADR 0084 (`una fix`).
- Target: edition `0.3`. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin does not move.
- Amends, once accepted: `spec/02 §4`, `spec/05 §4`, `§9`, `spec/13 §3`, `§4`, `§7`

## Context

The evidence is ADR 0092's table. This ADR answers rows B10, B11 and B12, and the repair ranking of B09:

- **B10.** `gallery.thumbnails` holds the user's `@files: Files[read, write]`. It calls the vendored library `gallery.thumbnailer`, whose tiled mode declares `pub thumbnail_tiled(path: Text, size: Nat) -> Image` with `uses @files: Files[read, write]`. The library receives the binding by name, and nothing in the program says that it should only read.
- **B11.** `webhooks.delivery` holds the service's `@http: Http[post]`. It calls the customer-supplied plug-in `webhooks.template`, whose `render_alert` declares `uses @http: Http[post]`, through a private helper. The helper's `uses` are inferred from its callers (`spec/05 §9` rule 2), so the plug-in gets the client with no declaration anywhere.
- **B12.** `intake.importer` holds `@records: Records[contains, insert, remove]` and `@reference: Reference[lookup]`. It hands the records to the third-party rule pack `intake.rules`, whose `review` declares `uses @records: Records[contains, insert, remove]`. The checker reports U3002 at a helper with a narrower header, and its rank-1 repair widens that header, which produces the violation. The pack's `check(row)` legitimately needs `Reference[lookup]`.
- **B09.** The rank-1 repair of a U3002 widens the editable headers up the call chain, and the widened variant builds and violates.

Three rules of today combine here:

- **Downward by name** (`spec/05 §4`). A callee's `uses @x: C[ops]` is met by any binding `@x` of the caller that has at least `ops`. Passing more operations than needed is allowed, and the callee can use only its own `ops`.
- **Inference** (`spec/05 §9` rule 2). A private function without `uses` gets the least set that covers its body and callees, taking each capability from its callers' bindings.
- **Ranking** (`spec/13 §3`, ADR 0084). For a missing or insufficient capability, the first repair widens the caller's header. `una fix` never applies a widening (ADR 0084), but an agent following rank 1 does.

All three are right inside one author's code. They are wrong at the edge of code the author does not control: a vendored library, a plug-in, a rule pack. There, authority moves without anyone writing it.

Una has no package dependencies yet. A package is its own modules (`spec/13 §7`), and Suite B's vendored and plug-in code are modules of the task's package. So the boundary has to be declarable on modules of the package now, and on dependencies when they arrive.

## Decision

1. **The consumer declares the boundary.** `una.toml` gains a table `[boundaries]`. Each key is a module path of the package, and each value is the module's **ceiling**, the most authority it may ever receive:

   ```toml
   [boundaries]
   "gallery.thumbnailer" = ["Files[read]"]                        # B10
   "webhooks.template" = []                                       # B11
   "intake.rules" = ["Records[contains]", "Reference[lookup]"]    # B12
   ```

   - The module itself never says it is a boundary. Code that wants authority would never mark itself.
   - `una.toml` is the consumer's file, outside the sources an agent edits for a task. A benchmark scaffold, or a repository's review rules, protects it as they protect `[trusted]` (`spec/07 §6`).
   - A ceiling is a list of `C[ops]`, the form of a `uses` clause's type (`spec/05 §3`).
   - Unknown module paths and capabilities are U7002.

2. **The secure default for code from outside the package.** When packages gain dependencies, every module of a dependency is a boundary with an empty ceiling, unless the consumer's `una.toml` lists the dependency under a table `[inherit]`, which opts it into inheritance. Until then, the default concerns only `[boundaries]` entries, and a package that writes none is unchanged.

3. **No authority crosses a boundary by name.** A call from outside a boundary module to a function of it does not meet the callee's `uses` from the caller's bindings by name.
   - Every capability the callee needs is granted at the call with `with`: a rebinding, a narrowed rebinding (rule 4), or a handler.
   - A missing grant is a new code, U3020: "the boundary module `gallery.thumbnailer` needs `@files: Files[read, write]`, which a call into it grants only with `with`". It is reported at the call, with the callee's requirement as the related span.
   - **A function value is a call.** A function of a boundary module used as a value outside it, as the argument of a higher-order function (`spec/05 §9` rules 6 and 7), is a call into the boundary at the point of the reference. Its `uses` are granted there, by the `with` of the statement that holds the reference, and checked against the ceiling there. The higher-order function that later calls the value supplies nothing by name.
   - **Installing a handler is a call.** A handler declared in a boundary module and installed outside it (`with @x = h(args)` or `given @x = h(args)`) runs the boundary's code. So the installation is a call into the boundary: each of the handler's own `uses` is granted by a binding of the same `with` or `given` clause, not by the installer's context by name (`spec/05 §9` rule 4), and it is checked against the ceiling.
   - Inference (`spec/05 §9` rule 2) never takes a capability across a boundary: a private function of a boundary module takes its `uses` from callers inside the module only, and a private function outside takes none from inside.

4. **A narrowed rebinding.** `with @x = @y[ops]` binds the callee's `@x` to the caller's `@y`, restricted to `ops`, which MUST be a subset of `@y`'s operations (U3002 otherwise):

   ```una
   let verdict = check(row) with @reference = @importer_reference[lookup]
   ```

   Next to the forms it extends:

   | Today | Edition `0.3` |
   | --- | --- |
   | `uses @files: Files[read]` (`spec/05 §3`): the type `C[ops]` in a header | `@files[read]`: the same brackets on a binding, at a `with` |
   | `with @files = @user_files` (`spec/05 §4`): a rebinding with every operation | `with @files = @user_files[read]`: a rebinding with the listed ones |
   | `with @files = read_only_files()`: a handler adapter, written by hand (B10's reference) | the same, still allowed; the narrowed form saves writing one |

   A narrowed binding lacks the other operations: a callee whose `uses` list one of them is U3002 at the call, as for any binding with too few operations. B10's library declares `Files[read, write]`, so a narrowed `@files[read]` does not satisfy it. The reference grants a handler, whose `write` fails, instead.

   The narrowed rebinding is allowed everywhere, not only at a boundary. Inside a boundary module's callers, it is the only way to grant less than a whole binding without writing a handler.

5. **The ceiling.** At every call into a boundary module (rule 3, function values and handler installations included), the authority granted, every `C[ops]` of its `with` bindings, MUST be within the module's ceiling. Otherwise the call is U3021: "the call grants `@files: Files[read, write]` to `gallery.thumbnailer`, whose ceiling in `una.toml` is `Files[read]`".
   - A handler granted at a boundary counts with its own `uses`, since the handler runs with them on the boundary's behalf.
   - The ceiling is what turns B10, B11 and B12 into build failures. A grant written in plain sight would still violate the task's rule. The ceiling is where the consumer writes that rule.

6. **Repairs never widen authority first** (Q1 direction 3; `spec/13 §3`, ADR 0084).
   - **U3020:** rank 1 grants the callee's requirement narrowed to what it declares, `with @x = @y[ops]`, when the caller holds a binding `@y` of that capability with those operations, and only when the grant lies within the ceiling. `una fix` applies it only then: a grant beyond the ceiling is never applied, nor offered as rank 1, since it would widen the authority of code the consumer does not trust (ADR 0084). When the requirement exceeds the ceiling, rank 1 is a handler skeleton whose `uses` lie within the ceiling (not mechanical), and the diagnostic names the operations the ceiling withholds.
   - **U3021:** rank 1 narrows the grant to the ceiling. If the callee needs more than the ceiling, there is no repair that keeps both, and the diagnostic says so: the consumer decides between the library and the ceiling.
   - **U3001 and U3002 everywhere:**
     - first the narrowing repairs: pass a narrower binding, or rebind one the caller holds;
     - then a widening of a private header;
     - a widening of a public header or of an entry point comes last, marked as an authority change in its repair text.
     
     This reorders today's U3001 and U3002 repairs in edition `0.3`. It changes no code's meaning.

7. **Formal model.** The theorem this ADR needs, *authority confinement*, is an open obligation and a separate milestone of the formal track. It states that every effect performed during a call into a boundary module is an operation that the call's `with` bindings grant, within the ceiling. Until it lands, this ADR is "accepted, not proved". The re-check reads a call into a boundary module as not re-checked until then.

## Consequences

- **B10, B11 and B12 fail at build** once the scaffold's `una.toml` declares the vendored library, the plug-in and the rule pack with their ceilings:
  - **B10:** the violation passes `@files` by name into `thumbnail_tiled`, which is U3020. Granting it as it is (`with @files = @files`) is U3021, since `Files[read, write]` exceeds `Files[read]`. The reference, `thumbnail_tiled(path, size: preset.size) with @files = read_only_files()`, grants a handler whose own `uses` are `Files[read]`, within the ceiling, and builds.
  - **B11:** the violation's helper no longer infers `@http` across the boundary, and the call into `render_alert` is U3020. Any grant of `@http` is U3021, since the ceiling is empty. The reference, `render_alert(event, failed) with @http = no_requests()`, grants a handler with no `uses` and builds.
  - **B12:** the violation grants the writable records to `review`, which is U3021. The reference, `review(inserted) with @records = records_read_only()`, grants a handler whose `uses` are `Records[contains]`. Its call of `check(row)` grants `with @reference = @reference[lookup]` (U3020 until written, and `una fix`'s rank 1, within the ceiling). Both are within `intake.rules`'s ceiling, so the reference builds.
  
  The rank-1 repairs lead to the secure variant, not away from it.
- **B09's widening path** moves to the last rank, marked as an authority change.
- **Agent cost.**
  - Two new codes (U3020, U3021), met only at calls into declared boundaries.
  - One new form: brackets on a binding at a `with`, the same brackets as in a header.
  - One new `una.toml` table, which the agent does not write in a scaffolded task.
  - `una fix` applies the U3020 narrowing within the ceiling, and nothing beyond it.
  - The reordered U3001 and U3002 repairs change which edit an agent sees first, not how many diagnostics it meets.
- **Cost for existing packages.** None until a package writes `[boundaries]`. Once it does:
  - every call into a declared module needs explicit grants, which is about one `with` per capability per call site;
  - private helpers that relied on inference across the boundary need explicit `uses`.
  
  When dependencies arrive, the empty-ceiling default makes every call into a dependency that needs authority a U3020 until the consumer grants it or lists the dependency under `[inherit]`. That is the price of the secure default, and it is paid once, at the first build.
- **Complexity.** The checks are linear in call edges: one ceiling comparison and one grant lookup per call into a boundary module. Inference keeps its fixpoint over the module call graph, restricted to edges inside a boundary or inside the rest.
- **Deferred.** Attenuation by parameters other than operations (hosts, paths, rates; #59), and revocation, are not here. A ceiling names operations only.

## Alternatives considered

- **The module marks itself** (`boundary module gallery.thumbnailer`). Rejected: a malicious or careless module would not mark itself, and the protection would depend on the code it protects against.
- **No ceiling, only explicit grants.** It makes every grant visible and removes B11's silent path. But B10 and B12 grant read-write in plain sight and still build, so the task's rule has no place to live. Rejected as insufficient on its own: it is rule 3 without rule 5.
- **Forbidding boundary function values and handlers outside their module.** Simpler, but it rules out plain uses such as `photos.map(each: thumbnail)` of a pure library function. Treating them as calls (rule 3) keeps them with the same checks.
- **Attenuation by default without a declaration:** every call into another module receives only the operations the callee declares. That is today's rule (`spec/05 §4`: the callee can use only its `ops`). It does not help, because the callee's header asks for read-write.
- **A handler adapter only, no narrowed rebinding** (`with @storage = read_only(of: @user_files)`). Already possible, and it stays. But each capability needs its own adapter written by hand, and the full run showed that agents pay for every extra construct. The narrowed rebinding is the same brackets as a header.
- **Inheritance by default for in-package modules, boundaries only for dependencies.** That is the default for unlisted in-package modules (rule 2), since a package's own modules are one author's code. The consumer lists the in-package modules it does not trust.

## Open obligations

- The authority-confinement theorem in F1 (rule 7), a milestone of the formal track.
- The `[inherit]` table's exact form, with the dependency ADR that introduces packages' dependencies.
- U3020 and U3021 join `spec/13 §4`'s registry when this ADR is accepted (codes are assigned at acceptance).
