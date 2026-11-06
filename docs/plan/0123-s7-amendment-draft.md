# Draft: ADR 0123, Amendment (2026-11-06): the kind `TestRequestError` (slice S7, issue #1009)

- Status: Draft, for the project's decision. The accepted text of ADR 0123 does not change until the project accepts an amendment; `spec/31 §8` states what the implementation of the first part of slice S7 does meanwhile.
- Precedent: ADR 0117, "Amendment (2026-10-19): what slice S3 settles", item 7, which settled `Deadline = Expired` as `= | Expired` for the same reason.

## Why

ADR 0123 rule 21 declares `pub kind TestRequestError = Refused`. By the grammar (`spec/02 §2`, `alias_body`), `= Refused` declares an alias of a type named `Refused`, which does not exist; a sum kind of one variant is written with a leading bar. `fails` names a sum kind (`spec/06`, U2002 otherwise), so the text as written could not be the failure kind of `test_request`, which is what the rule means.

## Decided in the implementation

`std.testing` declares `pub kind TestRequestError = | Refused`, a sum kind of one variant, `Refused`, with the meaning rule 21 gives it: `test_request` fails with `Refused` where the host would answer `400`. Rule 21 reads `= | Refused` for `= Refused`. Nothing else changes.

## Proposed amendment text

> **Amendment (2026-11-06): `TestRequestError`.** Rule 21's `pub kind TestRequestError = Refused` is declared `pub kind TestRequestError = | Refused`: a sum kind of one variant, since `= Refused` would name an alias of a type `Refused` (`spec/02 §2`) and `fails` names a sum kind (`spec/06`). The variant and its meaning do not change.
