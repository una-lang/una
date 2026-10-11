# 0094. On whose behalf an operation answers

- Status: Accepted
- Date: 2026-10-11
- Deciders: Una project (accepted on 2026-10-11)
- Resolves: issue #520, part C ("on whose behalf"), for B09. B07 is not covered: #560. Parts A and B are ADR 0092 and ADR 0093.
- Target: edition `0.3`. Editions `0.1` and `0.2` keep today's rules, and the Gate 1 pin does not move.
- Implementation: #566 (edition `0.3`), with its formal extension #563. The spec amends below land with the implementation.
- Amends: `spec/01 §5`, `spec/02 §3`, `spec/05 §3`, `§4`, `§9`, `spec/07 §2`, `§3`, `spec/13 §6`

## Context

The evidence is ADR 0092's table. This ADR answers row B09, a confused deputy, and says why it does not answer B07:

- **B09** (una-bench `tasks/B09-share-link`). The share service holds two bindings of one capability `Files` (`role`, `owner`, `create_link`):
  - `@service_files`, the only account that can create links;
  - `@requester_files`, which sees what the requester sees.
  
  The secure reference creates links through the service and looks up a file's owner through the requester (`owner_seen`). The violation looks the owner up through the service's binding, for a requester who cannot see the file, and names the owner in the refusal it returns. U3002 flags the lookup, but nothing says that the service's answer about ownership must not reach the requester.
- **B07** (`tasks/B07-document-export`). The export reads its template through `@service_storage.read` (legitimate: the bundle's layout is the template) and an archived user document through the same binding and the same operation (the violation). Both reads are `read` on one binding, with a key of one type. Whatever is said of the binding or of the operation marks both alike, so a sound answer has to depend on which key is read. That is #560's question, and this ADR claims nothing for B07.

A capability says which operations, not on whose behalf they answer. Una already has a precise account of where a value may go: labels and their categories (`spec/07 §2`), checked by T6. What is missing is a way to say that what an operation answers through a given binding belongs to the principal that binding acts for.

## Decision

1. **A label on an operation of a binding.** In edition `0.3`, an operation listed in the `[ops]` of a `uses` clause MAY carry `gives` and a label:

   ```una
   pub create_share_links(request: ShareRequest) -> ShareSummary
     uses @service_files: Files[create_link, role gives internal[Service], owner gives internal[Service]]
     uses @requester_files: Files[role, owner]
     uses @mailer: Mailer[send]
   ```

   - The label is a label of `spec/07 §2`, so a principal is a category declared in `una.toml` under `[labels] categories`. An undeclared one is U4007.
   - An operation without `gives` answers at its declared result label, as today.
   - It follows the grammar of ADR 0092 rule 1 (`cap_op`). It is not allowed in a function type's `uses` (U0101), nor in a `[boundaries]` ceiling (U7002). A named function whose `uses` carry a label may not be used as a function value (U3006).
   - It composes with ADR 0092's condition: `journal where not dry_run`, `owner gives internal[Service]`, and both, in that order, `owner gives internal[Service] where c`.
   - Next to the forms it extends:

     | Today | Edition `0.3` |
     | --- | --- |
     | `accepts internal` on an operation's declaration (`spec/05 §2`): the most its arguments may carry, for every binding | `owner gives internal[Service]` in a `uses` line: the least its answers carry, for this binding |
     | `uses @service_files: Files[create_link]` (`spec/05 §3`) | `uses @service_files: Files[create_link, owner gives internal[Service]]` |
     | `secret[PII] Phone` (`spec/07 §2`) | the same label syntax, after `gives` |

2. **Meaning.** Through the binding, the operation's answer carries the operation's declared result label joined with the `gives` label. That covers:
   - the value it returns;
   - **whether it failed**, the occurrence of the failure and not only its payload: whether `owner` succeeded tells whether the file exists for the service, which is B09's question. As ADR 0090 did for a policy call's failure, the decision on the operation's outcome is labeled, so the `when` or `attempt` that branches on it runs under a program counter raised to the label;
   - the failure's payload.

   **A labeled failure passed on** is a failure at the program counter joined with the label. That covers `if error pass`, a `fail` in a branch on the decision, and a function's own failure. So, as for any failure under a raised program counter (S-18, ADR 0090), it is U4002 where the function's failure is observed below the label.

   This is the only change to typing: T-Op (`spec/10`) joins one more label into the result and into the decision on its failure. Everything downstream is today's information flow. An answer given on the service's behalf, flowing to what the requester receives, is U4001 or U4003 at the flow, or U4002 for an effect under it, unless a policy declared in a trusted module releases it (`spec/07 §5`).

3. **Passing a labeled binding.** A binding flows downward by name or with `with` (`spec/05 §4`), and its labels go with it:
   - **A callee's requirement** `uses @x: C[op gives l, ...]` is met by a caller's binding whose `op` gives at most `l`. The callee reads the answers at `l` or above, which is safe. A binding whose `op` gives more is U3022: "`@service_files.owner` gives `internal[Service]`, which is not within what `owner_seen` reads it at".
   - **A callee that lists the operation without `gives`** reads its answers at the declared label, so a labeled operation is passed to it only when the label is `⊥` (U3022 otherwise). A helper written before the operation was labeled has to say what it reads at, which is a header change that shows in the diff.
   - **A private function with inferred `uses`** (`spec/05 §9` rule 2) infers the labels too, the join over the bindings its callers pass. This reuses the fixpoint of `uses`.
   - **A rebinding** `with @x = @y` keeps `@y`'s labels, and ADR 0093's `@y[ops]` keeps those of the operations it keeps.

4. **Handlers.** A handler installed at a binding answers through it, so its answers carry the binding's labels. A handler cannot lower them.

5. **The host premise.** T6 needs one more hypothesis on the host, written in the shape of H1 to H4 (`docs/plan/formal-track-status.md`, "What accepted rests on"):
   - **H5: an answer depends only on the host state at its label.** What an operation answers through a binding where it is labeled `l′`, and whether it fails, is a function only of the arguments and of the part of the host state at or below `l′`.
   
   This is the premise T6's proof uses. Two runs whose host states agree at or below an observer's label give equal answers through every operation labeled at or below that observer, so those answers are low-equivalent. An answer labeled above the observer may differ, and its label carries it.
   - H5 does not forbid the same fact being readable through two bindings. In B09 an owner is answered through `@service_files.owner`, labeled `Service`, and through `@requester_files.owner`, at `⊥`. H5 requires that the unlabeled answer depend only on what the requester may see, the host's state at `⊥`. A file the requester cannot see answers there as `NotFound`, which is what the scaffold's requester handler does.
   - H5 is a premise on how the deployment wires its bindings, as H1 to H4 are on the host.
   - It is listed in the build report's Assumed zone (`spec/13 §6`) with every operation label it is used for, and named in the re-check's note where it is used.

6. **Keyword.** `gives` is a contextual keyword, recognized only after an operation name inside the brackets of a `uses` clause in edition `0.3`, and an identifier elsewhere, as `of` is after `uses` (`spec/02 §3`, ADR 0062).
   - A program of edition `0.1` or `0.2` that writes `gives` there is a parse error, U0103, with the message "`gives` is a keyword of edition 0.3 (operation labels)".
   - It is not reserved in those editions. Reserving it as an identifier would reject programs they accept today, and their pins do not move.

7. **Repairs** (`spec/13 §3`).
   - **For U3022:**
     1. pass a binding of the right principal, when the caller holds one with the capability and operation (B09's reference: `@requester_files`);
     2. raise the callee's `gives` label, which changes the callee's header and so its callers' flows.
     
     Neither repair lowers a label.
   - **For the U4001, U4002 or U4003 that a labeled answer causes,** today's repairs stand: read it through another binding, change the sink, or release it through a policy in a trusted module. The repair text names the binding and operation whose label the value carries, so the agent sees whose answer it is.

8. **Formal model.** No new theorem: a principal is a category, so T6 covers the flows (`spec/07 §1`), under H5. F1 needs one extension, a label on an operation of a binding that joins into its answers and into the decision on its failure, with T6's proof extended to it. That is a milestone of the formal track. Until it lands, the re-check reads a labeled operation as not re-checked.

## Consequences

- **B09 fails at build, and its reference builds,** once the scaffold's entry point labels the service's `role` and `owner` as above. The reference's bodies do not change.
  - The violating variant's `owner` through `@service_files` answers at `internal[Service]`. The refusal that names the owner flows into the `ShareSummary` the requester receives, which is U4001 there. A branch on whether that lookup failed raises the program counter, so a mail sent under it is U4002.
  - The reference reads `owner` and `role` through `@requester_files` and creates links through `@service_files`. `create_link` is unlabeled, so the URL reaches the requester as today.
- **B07 is not covered** (Context). #560 asks for a key-dependent form, decided statically, and no coverage is claimed until B07's secure variant builds under it.
- **The decision moves to a policy, into the TCB.** "May the service's answer reach this requester?" becomes a `policy` in a trusted module, listed in the build report's TCB section (`spec/07 §6`). That is where Una already puts every release, so a deputy's escape hatch is reviewed like every other.
- **Agent cost.**
  - One new form: `gives` and a label after an operation in a `uses` line's brackets, in the label syntax every Una program already uses.
  - One new contextual keyword, `gives`.
  - One new code, U3022, met only when a labeled operation is passed to code that reads it lower.
  - The flows themselves give today's U4001, U4002 and U4003, whose repairs agents already follow.
- **Cost for existing code: none.** An operation without `gives` answers at its declared label, as today.
- **Locked labels.** The labels of a public entry point can be locked in `una.toml` under `[contracts]` (ADR 0092 rule 8), so that an agent cannot drop the `gives` from the header. B09's scaffold locks `create_share_links`'s `role` and `owner` labels.
- **Complexity.** One label join per operation answer, and one comparison per labeled operation passed at a call: linear in operation sites and call edges. The flow checks keep their cost.
- **Deferred.**
  - Key-dependent answers (B07, #560).
  - A per-binding `accepts`, which would bound what may be written on a principal's behalf: the operation's `accepts` already bounds the sink, and B09 is a read.
  - Principals chosen at run time, such as one per user.

## Alternatives considered

- **A label on the whole binding** (`uses @service_files: Files[...] gives internal[Service]`, the first draft). Rejected:
  - it labels the legitimate answers of the same binding too: B09's link URL, created through `@service_files.create_link`, reaches the requester in the reference, so the reference would fail;
  - B07's template read is the same binding as its archive read, so the binding is not that deputy's grain either.
- **Nominal purpose types** (a capability per purpose, `capability ArchiveStorage` beside `capability TemplateStorage`). Possible today: two capability declarations are two types, and the checker tells their bindings apart (`spec/05 §2`). It separates which binding a call uses, but it says nothing about where the answer goes, and it relies on every author splitting capabilities by purpose in advance. It stays available, and the repairs mention it.
- **A principal as a new kind of label component** (beside level and categories). Rejected: it duplicates what categories express, and it would need its own lattice, policies and theorem, where categories reuse T6.
- **Labeling results in the capability declaration** (`op owner(file: Text) -> internal[Service] Text`). Already possible today, but it labels every binding of the capability alike: B09's requester binding needs `owner` unlabeled.
- **Labeling the payload only.** Rejected: whether the lookup failed is the information B09 leaks (rule 2, ADR 0090's reasoning for policies).
- **Reserving `gives` in editions `0.1` and `0.2`** with U0103 as an identifier. Rejected: it would reject programs those editions accept today, under pins that do not move. The contextual keyword (rule 6) gives the same parse in `0.3` and no break before.
- **Runtime provenance tracking.** Rejected: Una's guarantee is static (T6), and a runtime tag would make a confused deputy a run-time failure on paths the tests miss.

## Open obligations

- F1's extension with operation labels and labeled failure decisions, and T6's proof over it under H5 (rule 8), a milestone of the formal track (#563).
- H5 in the status page's list of host premises and in the build report's Assumed zone.
- Assigned on acceptance: U3022. It joins `spec/13 §4`'s registry with the implementation (#566).
- B07: #560.
