# Conformance suite

Each case tests one rule of `docs/spec/`. The format, directives and runner are defined in `docs/compiler/testing.md §2`.

```text
tests/conformance/
  syntax/  fmt/  names/  types/  authority/  security/  verification/  runtime/  examples/  project/
```

Rules for adding cases:

1. One rule per case. Name the file after the rule, not the bug (`cap_as_value.una`, not `issue_42.una`).
2. Every case has `-- test:` and `-- spec:` directives and at least one `-- expect:`.
3. Error positions are exact (`line:col`, 1-based, columns in Unicode scalar values).
4. Every diagnostic code gets at least one failing case, and the rule it enforces gets at least one passing case.

The three cases present at the start of the project anchor the format for authority, security and verification.
