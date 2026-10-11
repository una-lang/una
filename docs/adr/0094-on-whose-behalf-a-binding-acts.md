# 0094. On whose behalf a binding acts

- Status: Proposed
- Date: 2026-10-11
- Deciders: Una project (pending acceptance)
- Resolves: issue #520, part C ("on whose behalf"). Parts A and B are ADR 0092 and ADR 0093.
- Target: edition `0.3`. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin does not move.
- Amends, once accepted: `spec/02 §3`, `spec/05 §3`, `§4`, `§9`, `spec/07 §2`, `§3`

## Context

The evidence is ADR 0092's table. This ADR answers rows B07 and B09, the confused deputies:

- **B07.** A document export holds `@service_storage: Storage[read]` for the service's templates. A document not found under its key may be archived, and the scaffold's `read_archived` reads the archive through the same binding name and capability. The export calls it, and the user's bundle gets a document read with the service's authority. The checker cannot tell the template read from the archive read: both are one binding of one capability type (`spec/05 §8`, and §9 rule 1 matches the call).
- **B09.** A share-link service looks up a file's owner through its own binding, for a requester who cannot see the file. It names the owner in the refusal. U3002 flags the lookup, but nothing says that the service's answer must not reach the requester.

A capability says which operations, not on whose behalf. Una already has a precise account of "where may this value go": labels and their categories (`spec/07 §2`), checked by T6. What is missing is a way to say that what a binding returns belongs to the principal it acts for.

## Decision

1. **A binding may carry a label for its results.** In edition `0.3`, a `uses` clause MAY end with `gives` and a label:

   ```una
   pub export(user: UserId) -> Bundle
     uses @user_storage: Storage[read] gives internal[User]
     uses @service_storage: Storage[read] gives internal[Service]
     uses @out: Bundles[write]
     fails ExportError
   ```

   - The label is a label of `spec/07 §2`, so the principals are categories declared in `una.toml` under `[labels] categories`. An undeclared one is U4007.
   - Next to the forms it extends:

     | Today | Edition `0.3` |
     | --- | --- |
     | `accepts internal` on an operation (`spec/05 §2`): the most its arguments may carry | `gives internal[Service]` on a binding: the least its results carry |
     | `uses @service_storage: Storage[read]` (`spec/05 §3`) | `uses @service_storage: Storage[read] gives internal[Service]` |
     | `secret[PII] Phone` (`spec/07 §2`) | the same label syntax, after `gives` |

   - A `uses` clause may carry both ADR 0092's condition and a label, in the order `C[ops] gives l if cond`.

2. **Meaning.** The result of every operation through the binding has the operation's declared result label joined with the binding's label. Its failure payload does too, since a failure through the binding is information about the principal's data.
   - This is the only change to typing: T-Op (`spec/10`) joins one more label into the result.
   - Everything downstream is today's information flow. A value read on the service's behalf flowing to the user's bundle is U4001 or U4003 at the flow, as for any label, unless a policy declared in a trusted module releases it (`spec/07 §5`).

3. **Passing a labeled binding.** A binding flows downward by name or with `with` (`spec/05 §4`), and its label goes with it:
   - **A callee's requirement** `uses @x: C[ops] gives l` is met by a caller's binding whose label is at most `l`. The callee then reads results at `l` or above, which is safe. A binding labeled above `l` is U3022: "`@service_storage` gives `internal[Service]`, which is not within `internal[User]`, the label `archive.read_archived` reads its results at".
   - **A callee without `gives`** reads its results at the operations' own labels. So a labeled binding is passed to it only if the label is `⊥` (U3022 otherwise). A helper written before the binding was labeled has to say what it reads at, which is a header change that shows in the diff.
   - **A private function with inferred `uses`** (`spec/05 §9` rule 2) infers the label too, the join of the labels of the bindings its callers pass. Callers that pass different labels make the helper read at their join.
   - **A rebinding** `with @x = @y` carries `@y`'s label, and ADR 0093's `@y[ops]` keeps it.

4. **Handlers.** A handler installed at a labeled binding runs with its own `uses`, and its results are labeled by the binding, as every result through the binding is. A handler cannot lower the label.

5. **Repairs** (`spec/13 §3`).
   - **For U3022:**
     - rank 1: pass a binding of the right principal, when the caller holds one with the capability and operations;
     - rank 2: raise the callee's `gives` label, which changes the callee's header and so its callers' flows.
     
     Neither repair lowers a label.
   - **For the U4001 or U4003 that a labeled result causes,** today's repairs stand: change the sink, or declassify through a policy in a trusted module. The repair text names the binding whose label the value carries, so the agent sees which principal's data it is.

6. **Formal model.** No new theorem: a principal is a category, so T6 covers the flows (`spec/07 §1`). F1 needs one extension, a label on a capability binding that joins into the results of its operations, with T6's proof extended to it. That is a milestone of the formal track. Until it lands, the re-check reads a labeled `uses` clause as not re-checked.

## Consequences

- **B07 fails at build** once the scaffold labels the bindings: the archived document read through `@service_storage` carries `Service`, and writing it into the user's bundle is U4003 at `@out.write` (its sink accepts `internal[User]`). The secure variant reads through `@user_storage` and builds.
- **B09 fails at build** the same way: the owner's name read through the service's binding carries `Service`, and the refusal sent to the requester is U4003, unless a policy in a trusted module decides that a requester may see an owner's name.
- **The decision moves to a policy, into the TCB.** "May the service's answer reach this user?" becomes a `policy` in a trusted module, listed in the build report's TCB section (`spec/07 §6`). That is where Una already puts every release, so a confused deputy's escape hatch is reviewed like every other.
- **Agent cost.**
  - One new form: a suffix on a `uses` line, in the label syntax every Una program already uses.
  - One new code (U3022), met only when a labeled binding is passed to code that reads it lower.
  - The flows themselves give today's U4001 and U4003, whose repairs agents already follow.
- **Cost for existing code: none.** A `uses` clause without `gives` reads results at the operations' labels, as today.
- **Complexity.** One label join per operation result, and one comparison per binding passed at a call: linear in operation sites and call edges. The flow checks keep their cost.
- **Deferred.**
  - A binding-level `accepts`, which would bound what may be written on a principal's behalf: the operation's `accepts` already bounds the sink, and B07 and B09 are reads.
  - Principals that are not categories, such as per-user principals chosen at run time.

## Alternatives considered

- **Nominal purpose types** (a capability per purpose, `capability ArchiveStorage` beside `capability TemplateStorage`, each declaring `Storage`'s operations). This is possible today: two capability declarations are two types, and the checker tells their bindings apart (`spec/05 §2`). It separates which binding a call uses, so B07's helper could require `ArchiveStorage` and the export could hold only `TemplateStorage`. Not chosen as the answer:
  - it says nothing about where the data goes, so B09, whose lookup is legitimately the service's, is not stopped;
  - it relies on every author splitting capabilities by purpose in advance, which an API that ships one `Storage` does not do.
  
  It stays available, and this ADR's repairs mention it.
- **A principal as a new kind of label component** (a third component beside level and categories). Rejected: it duplicates what categories already express. It would also need its own lattice, its own policies and its own theorem, where categories reuse T6.
- **Labeling results by the capability declaration** (`op read(...) -> internal[Service] Doc`). Already possible today, and it labels every binding of the capability alike. B07's two reads are one capability with two purposes, so the label has to be on the binding, not on the operation.
- **Runtime provenance tracking.** Rejected: Una's guarantee is static (T6), and a runtime tag would make a confused deputy a run-time failure on paths the tests miss, rather than a build failure.

## Open obligations

- F1's extension with binding labels, and T6's proof over it (rule 6), a milestone of the formal track.
- U3022 joins `spec/13 §4`'s registry when this ADR is accepted.
