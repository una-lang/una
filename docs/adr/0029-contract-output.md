# 0029. The output of `una contract`

- Status: Provisional (implemented, pending human review)
- Date: 2026-09-29
- Deciders: Claude (overnight run), pending review by Leonardo

## Context

`spec/13 §5` defines `una contract <qualified.name>`, which prints "the full header, declared or inferred, with doc comments and requirements", and M5 requires a snapshot of `una contract --json`. `spec/13` defines no JSON shape for it, although the JSON shapes of `spec/13` are the Agent ABI, and says neither which package is read nor how an inferred clause is shown.

Decision log entries: D-081 (`docs/reviews/open-decisions.md`).

## Decision

1. **Package.** `una contract` reads the package in the current directory, as `una check` does without a path. The qualified name is the module path followed by the function's name, as in `events.checkin.check_in`. An unknown name is a usage error (exit code 2).
2. **Human output**, on stdout: the doc comments, then the header in canonical form with every clause, declared or inferred, in canonical order; each inferred clause ends with the comment `-- inferred`; then one line per requirement of `derived from`: `-- requirement ID (status): "text"`.
3. **JSON output**, on stdout, compact, with keys in this order:

   ```json
   {
     "schema": "una.contract/1",
     "function": "events.checkin.check_in",
     "public": true,
     "docs": ["Looks up a registration by phone, shows its availability, and records it."],
     "header": "pub check_in(phone: secret[PII] Phone) -> Availability",
     "requires": [],
     "ensures": [],
     "uses": [ { "cap": "registrations", "capability": "Registrations", "ops": ["find"], "inferred": false } ],
     "fails": { "kind": "CheckInError", "inferred": false },
     "derived_from": [ { "id": "EVENTS_3", "status": "active", "text": "..." } ]
   }
   ```

   `fails` is `null` for a function that cannot fail; `ops` lists the operations in declaration order of the capability; `docs` holds the doc comments without their `---` marker.
4. The schema follows the versioning rule of `spec/13`: a change of shape bumps `una.contract/1`.

### Changes to other documents on acceptance

- `spec/13 §5` gains the rules above and the schema.

## Consequences

- Agents read a function's authority and failure contract, inferred or declared, without parsing source.
- The human output is valid Una header syntax with comments, so it can be pasted into code to make inferred clauses explicit.

## Alternatives considered

- **A path argument:** `spec/13 §5` defines none, and adding one extends the command line.
- **Marking inferred clauses in a separate section:** separates the clause from its function, while a trailing comment keeps the header readable in place.
