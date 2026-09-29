# M3 plan: project and names

Status: written during the autonomous run (`OVERNIGHT.md` C.2); not reviewed by the owner. Decisions are logged in `docs/reviews/open-decisions.md` and tagged `DECISION D-NNN` in the code.

Read list: `spec/13 §7`, `spec/01 §4`, `spec/08 §1`, `spec/07 §6`, `spec/12 §9`; also `spec/02 §1`, `spec/04 §5`, `spec/14 §1, §3, §4` (prelude, `std.testing`), `compiler/testing.md §2` (single-module and multi-module cases), ADR 0015 (D3 expires in M3, D4 stays).

Codes: U1001 to U1008, U7002, U7004, U7005.

## 1. Spec gaps and their ADRs

- **ADR 0021, packages and project files** (Provisional): a directory is a package when it has `una.toml` and `src/`; the keys and values of `una.toml` and `verification.lock`; a single file is a package with default settings and the default lock.
- **ADR 0022, diagnostics without a source position** (Provisional), the ADR requested at the acceptance of M0: `primary` stays required; a missing `verification.lock` is U7004 at `una.toml` 1:1.
- **ADR 0023, name resolution** (Provisional): namespaces, scopes and shadowing, what an import of a kind brings, the boundary between U1001 and U2002, what M3 resolves and what waits for types, and how modules with syntax errors are isolated.

## 2. Tasks and commits

1. **Plan and ADRs** (`docs(plan)`).
2. **Project files** (`feat(project)`, task 1): `una-project` parses `una.toml` (U7002) and `verification.lock` (U7004) into settings and located errors, with `toml`, from the allowlist; package discovery replaces interim loading (ADR 0021, ADR 0015 D3 expires); U1008 for module paths; each example package gains its `verification.lock` (carried over from M0).
3. **Names** (`feat(check)`, tasks 2 and 3): `una-check` builds the declarations of every module, resolves imports (U1001, U1003, U1004), checks duplicates (U1002), resolves every name of every body (U1001), naming conventions (U1005), requirements (U1002, U1006, U1007) and trusted modules (U7005). Unit tests for each rule.
4. **Conformance and explain** (`test(names)`, `docs(diag)`): the cases of section 3 and eleven explain texts. Package codes need package examples: an example section may list several files, each introduced by its path (D-043).
5. **Runner** (`test(conformance)`): a pending case fails when it reports a code that is already implemented and not expected, generalizing the M1 rule for syntax codes (D-044).
6. **Self-review** (`docs(plan)`).

## 3. Conformance cases

Cases in `tests/conformance/names/` for T1xxx and `tests/conformance/project/` for T7xxx; single-module unless marked (package), which are directories with `una.toml`, `verification.lock` and `src/`.

| Code | Case | Expected |
| --- | --- | --- |
| U1001 | `unresolved_function` | a call to an undeclared function |
| U1001 | `unresolved_type` | an undeclared parameter type |
| U1001 | `unresolved_constructor` | a bare constructor that no scope holds (variants after `fail` or in patterns wait for M4, D-046) |
| U1001 | `unresolved_local` | a name used after the block that bound it |
| U1001 | `unknown_module` (package) | an import of a module the package does not have |
| U1001 | `unknown_imported_name` (package) | an import of a name the module does not declare |
| U1002 | `duplicate_declaration` | two functions with one name |
| U1002 | `duplicate_variant` | two kinds of a module with a variant of the same name |
| U1002 | `duplicate_parameter` | two parameters with one name |
| U1002 | `local_rebinding` | a `let` that rebinds a parameter |
| U1002 | `duplicate_requirement` | two requirements with one identifier |
| U1002 | `duplicate_example` | two examples with one name |
| U1003 | `import_cycle` (package) | two modules importing each other |
| U1004 | `private_import` (package) | an import of a private function |
| U1005 | `kind_name_convention` | `kind Seat_Count`, with an underscore |
| U1005 | `function_name_convention` | a function name ending with `_` |
| U1005 | `requirement_id_convention` | `requirement EVENTS3` |
| U1006 | `undeclared_requirement` | `derived from` an undeclared requirement |
| U1007 | `generated_requirement_active` | `source generated` with `status active` |
| U1008 | `module_path_mismatch` (package) | `module a.c` in `src/a/b.una` |
| U7002 | `unknown_manifest_key` (package) | an unknown key in `[package]` |
| U7002 | `manifest_edition` (package) | `edition = "0.2"` |
| U7002 | `unknown_entry` (package) | `[run] entry` names no module of the package |
| U7004 | `missing_lock` (package) | no `verification.lock`: at `una.toml` 1:1 |
| U7004 | `lock_encoding` (package) | `encoding_version = "una-smt-0"` |
| U7005 | `trusted_header_not_listed` | a single file with `trusted module` |
| U7005 | `listed_module_not_trusted` (package) | `[trusted] modules` names a module whose header is not trusted |
| ok | `resolution_tour` | locals, patterns, lambdas, type parameters, the prelude, and a declaration shadowing a prelude variant |
| ok | `package_tour` (package) | imports of kinds with their variants, `std.testing`, a trusted module listed in `una.toml`, requirements across modules |

Every case is otherwise valid through M10, so later milestones add no diagnostic to it.

## 4. Explain texts

Eleven texts in the M1 format (`plan/runs/M1-plan.md §6`); the package codes (U1003, U1004, U1008, U7002, U7004, U7005) use package examples (D-043). Each failing example yields its code and nothing else at tier 0; each fixed example yields nothing.

## 5. Acceptance criteria

| Criterion | Evidence |
| --- | --- |
| Conformance cases for every listed code | section 3, runner summary |
| All examples resolve | `una check examples/events` and `examples/hello` exit 0 without output; both have their `verification.lock` |

## 6. Known gaps

- Type-directed resolution among several candidates, fields, methods and argument labels: M4 (ADR 0023 rule 3).
- Capability references: M5. Label categories: M6.
- `[run] entry` is checked to name a module of the package; that it has `pub main() -> Unit` is checked by `una run` (M9).

## 7. Self-review checklist

Every rule of the read list (`spec/13 §7`, `spec/01 §4`, `spec/08 §1`, `spec/07 §6`, `spec/12 §9`) and of the sections M3 also implements. Test names are in `crates/una-check/tests/names.rs` unless a path is given; cases are in `tests/conformance/names/` and `tests/conformance/project/`.

| Rule | Status | Evidence |
| --- | --- | --- |
| `spec/13 §7` `una.toml` and `verification.lock` at the root, sources under `src/` | Done (ADR 0021, D-032) | `crates/una-project/tests/project.rs` `a_package_is_a_directory_with_una_toml_and_src`; `una-cli/tests/cli.rs` `check_requires_a_package` |
| `spec/13 §7` Module `a.b.c` lives in `src/a/b/c.una` (U1008) | Done | case `module_path_mismatch` (with an edit); `module_paths_follow_file_paths`, which also checks the exemption of single files (`compiler/testing.md §2`) |
| `spec/13 §7` Unknown keys are U7002 | Done (D-033) | cases `unknown_manifest_key`, `manifest_edition`, `unknown_entry`; `project.rs` `manifest_errors_are_located` |
| `spec/13 §7` The keys of the example: labels, trusted modules, escape budget, run entry and grants | Parsed and validated; their meaning belongs to M6 (categories, U4007), M5 (budget, U7003) and M9 (`una run`) | `project.rs` `the_specification_example_and_the_examples_are_valid`, `grants_and_module_paths` |
| `spec/12 §9` The six keys of `verification.lock` (U7004) | Done (D-034); a missing lock is located at `una.toml` 1:1 (ADR 0022, D-036) | cases `missing_lock`, `lock_encoding`; `project.rs` `lock_errors_are_located`, `the_repository_lock_is_the_default_lock` |
| `spec/12 §9` The proof cache | Deferred to M8 | |
| `spec/01 §4` Lower-case names contain no `__` and do not end with `_` (U1005) | Done (D-042) | case `function_name_convention`; `naming_conventions_are_t1005` |
| `spec/01 §4` Kinds, variants, capabilities and categories are UpperCamelCase (U1005) | Done | case `kind_name_convention`; `naming_conventions_are_t1005`; `project.rs` for categories |
| `spec/01 §4` Requirement identifiers match the pattern (U1005) | Done | case `requirement_id_convention`; convention unit tests |
| `spec/08 §1` Rule 1: requirement identifiers are unique per package (U1002) | Done | case `duplicate_requirement`; `requirements_are_unique_declared_and_generated_ones_proposed` |
| `spec/08 §1` Rule 2: a generated requirement is proposed (U1007) | Done, with an edit | case `generated_requirement_active` |
| `spec/08 §1` Rule 3: `derived from` names declared requirements (U1006) | Done | case `undeclared_requirement` |
| `spec/08 §1` Rule 4 and `una trace` | Deferred to M9 | build report and `una trace` |
| `spec/08 §2` Rule 3: example names are unique per module (U1002) | Done | case `duplicate_example` |
| `spec/07 §6` A trusted module is declared in its header and in `una.toml` (U7005) | Done (D-045) | cases `trusted_header_not_listed`, `listed_module_not_trusted`; `trusted_modules_are_declared_twice` |
| `spec/07 §6` Policies only in trusted modules (U4004); the TCB section of the report | Deferred to M6 and M9 | |
| `spec/02 §1` Top-level names are visible throughout the module | Done | every declaration is registered before any body is resolved |
| `spec/04 §5` Declarations are private unless `pub` (U1004) | Done (D-040), with an edit adding `pub` | case `private_import` |
| `spec/04 §5` `pub` functions declare `uses` and `fails` | Deferred to M5 | U3003, U3009 |
| `spec/14 §1, §3` The prelude is imported into every module | Done | `names_resolve_through_locals_declarations_imports_and_the_prelude`; `prelude.rs` `the_prelude_matches_spec_14` reads the spec |
| `spec/14 §4` `std.testing` is importable; only in handlers and examples | Importable; the restriction (U3015) is M5 | `imports_bring_public_names_and_the_variants_of_kinds`; case `package_tour` |
| ADR 0023: imports, cycles, duplicates, shadowing, contextual variants | Done (D-037 to D-041, D-046) | cases `unknown_module`, `unknown_imported_name`, `import_cycle`, `duplicate_*`, `local_rebinding`, `resolution_tour`, `package_tour`; the corresponding tests |
| M3 acceptance: conformance cases for every listed code | Done | 29 cases, all passing; runner summary 92 cases, 89 passed, 3 pending |
| M3 acceptance: all examples resolve | Done | `una check examples/events` and `examples/hello` exit 0 without output; both packages gained their `verification.lock` (carried over from M0) |
| Explain texts for every M3 code | Done (D-043) | `una-cli/tests/explain_examples.rs` |
| Pending cases cannot hide implemented codes | Done (D-044) | the conformance runner |
| Determinism (`compiler/testing.md §4`) | Done | `una-cli/tests/cli.rs` `check_json_is_byte_identical_across_runs` covers the names and project cases |

### Known gaps carried forward

- **M4**: variants in patterns, `is` tests, after `fail` and after `expect fails` are resolved by their kind (D-046); fields, methods and argument labels; U2002 for a name that is not a type; the type checker suppresses cascades from unresolved names.
- **M5**: capability references; `std.testing` only in handlers and examples (U3015); the escape budget (U7003).
- **M6**: label categories against `[labels]` (U4007); policies only in trusted modules (U4004).
- **M9**: requirements in the build report, `una trace`, and the `[run]` settings.
