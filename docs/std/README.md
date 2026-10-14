# Standard library listing

`listing.json` lists every public item of the prelude and of `std.testing`, as the checker reads them in each edition: its name, module, kind, the spec sections its doc comments cite, and one variant per run of editions in which its declaration and doc comments are the same. A variant holds the editions, the declaration without its body, and the doc comments.

**Unstable, not part of the specification.** The schema `una.std-listing/1` may change without notice. No `una` command prints it, and no tool should depend on it. The specification of the standard library is `spec/14` and `spec/20` to `spec/24`.

- **Generated.** `crates/una-cli/src/std_listing.rs` writes it from the standard modules (`crates/una-check/src/std/`), through the same per-edition source the checker loads. `crates/una-cli/tests/std_listing.rs` fails when this file is not current; `UNA_BLESS=1 cargo test -p una-cli --test std_listing` rewrites it.
- **Checked.** The same test requires a doc comment on every item in every edition, citing spec sections that exist. It also checks that each item is listed in exactly the editions in which `una check` resolves its name.
- **Built-in methods: `builtins.json`.** `.len`, the methods of maps (`spec/21 §2`), the conversions (`spec/18 §4`) and the wrapping, saturating and bit operations of fixed-width integers (`spec/18 §5`) have no declaration, so `crates/una-cli/src/std_builtins.rs` writes their signature lines and doc comments, in the same item shape, under the unstable schema `una.std-builtins/1`. The test keeps them in step with the checker (`una_check::builtin_methods`) both ways, and probes each written signature in every edition: a call with arguments of the written types bound to the written result type must check from the listed edition on and not before, and the same call with a wrongly typed argument or result must not.
- **Not listed.** Operators, comparisons, equality and interpolation are not methods (`spec/14 §2`).
- **Published.** The site bundle (`scripts/site-bundle.sh`) carries this directory, from which the website builds its standard library pages.
