# 0076. Agent ABI versioning: added keys and open value sets

- Status: Accepted
- Date: 2026-10-08
- Deciders: Owner (2026-10-08, written); the wording about key position was clarified by the coordinator, following the stated intent of the chosen option; proposal by Owner (Una-1)
- Resolves: issue #370
- Amends: the preamble of `spec/13`

## Context

The JSON shapes of `spec/13` are the Agent ABI. Each document names its shape and version in its `schema` string. The schemas today are:
- `una.diagnostic/1` and `una.diagnostics/1`;
- `una.explain/1`;
- `una.obligations/1`;
- `una.contract/1`;
- `una.report/2`;
- `una.run/1` and `una.test/1`;
- `una.version/1`.

The preamble of `spec/13` says that schema versions froze at the `v0.1.0` tag, and that after it, "any change bumps the version".

Published releases did not follow that sentence. Version 1 schemas changed after `v0.1.0` without a bump, in two ways.

**Keys were added.**
- ADR 0066 added `constraints` to `una.contract/1`, after `header`. `spec/13` calls it "additive, and the schema stays `una.contract/1`".
- ADR 0067 added `result_labels` and `of_raises` to the same document, after `constraints`.
- Neither key sits at the end of the object.

**Values were added to enum-like fields.**
- The diagnostic `code` of `una.diagnostic/1` gained U2017 to U2021 (ADR 0056), U2022 (ADR 0057), U2023 and U2024 (ADR 0065), U2025 to U2027 (ADR 0066), and U6004 (N-40). Each arrived as a row of the registry of `spec/13 §4`.
- The obligation `kind` gained `overflow` (ADR 0057, `spec/12 §3`).

**One schema did bump.** `una.report/2` (ADR 0055) changed the meaning of an existing value: the `out_of_model` entry "interpreter stack exhaustion" became "stack exhaustion".

Agents parse these documents by key, and the reference implementation prints keys in a fixed order. A consumer that ignores keys it does not know is not affected by a new key, wherever that key is printed. A consumer that meets an unknown value of a field it branches on cannot, in general, know what that value means.

## Decision

1. **Adding a key does not bump the version.** A key added to an object, at any position, keeps the schema's version, as long as no existing key changes. Consumers MUST ignore unknown keys and MUST NOT depend on the position of a key. The order that `spec/13` gives stays the printed order, and the reference implementation MUST print keys deterministically in that order.
2. **Changing a key bumps the version.** Each of the following MUST bump the version in the `schema` string:
   - removing a key;
   - renaming a key;
   - changing the type of a key's value;
   - changing what a key or a value means, as ADR 0055 did for `una.report/2`.
3. **Enum-like fields are closed by default.** A field whose value is drawn from a set of strings, such as `severity`, `domain`, an obligation's `status` or a run's `status`, is closed. Adding a value to a closed set MUST bump the version, because a consumer cannot safely ignore a value it does not know.
4. **Two fields are open, as already practiced.**
   - **The diagnostic `code`.** Adding a registered code does not bump the version. A new code MUST have a `domain` and a `severity` from their closed sets.
   - **The obligation `kind`.** Adding a kind does not bump the version. A new kind's obligations MUST use the existing `status` values.

   Consumers MUST accept unknown values of these two fields and decide what to do from the closed fields: `severity`, `domain` and the obligation's `status`. `una explain <code>` gives the text of any code.

   An open field MUST NOT gain a value whose safe handling needs anything beyond the closed fields. A value that would need more is a bump.
5. **Scope.** These rules apply to every `una.*/N` schema, the ones above and those added later. `spec/13` lists the open fields, and a field is open only when `spec/13` declares it so.

## Consequences

- No schema changes version. `una.contract/1` and `una.report/2` stay as they are. The added keys and values listed in the context conform to this decision, so it needs no exception.
- Agents written against version 1 keep working on later releases, provided they ignore unknown keys and handle unknown codes and obligation kinds through the closed fields. `spec/13` now says so.
- A future change that removes, renames or retypes a key, changes a meaning, or adds a value to a closed set goes through the ADR process and bumps the version, as before.
- Accepted ADRs keep their text. ADR 0066's "the key is additive" and ADR 0067's added keys are now covered by this rule.
- The decision has no algorithmic cost: it changes no code or output.

## Alternatives considered

- **Keep "any change bumps the version".** `una.contract/1` and `una.diagnostic/1` would then have to become version 2 retroactively, for keys and codes that did not break any consumer that ignores unknown keys. Agents written against version 1 would have to update for nothing.
- **Add keys only at the end of an object.** The keys of ADRs 0066 and 0067 sit after `header`, in the middle of `una.contract/1`. They would be exceptions to their own rule. JSON object members are unordered, so a key's position carries no compatibility meaning for a consumer that parses by key.
- **Treat every enum-like field as open.** A consumer cannot act safely on an unknown `severity`, `domain` or `status`. It could not tell whether an unknown status means proven or refuted. Only `code` and the obligation `kind` can be open, because every code and kind is qualified by those closed fields.
