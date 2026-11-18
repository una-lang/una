# ADR 0120 slice S6: native dense values and dispatch (design note)

- Issue: #989 (part of #894, ADR 0120 rules 26 to 43, 55 to 57, Implementation plan step 6)
- Base: slice S5 (#1126, `dev/988-kernels`), which carries S4 (#1125): the dense types in the front end and the verifier, the interpreter's dense values, and the kernels of `una-core`.
- Scope: no construct is added. Native code runs the dense programs the interpreter runs; the kernels gain target variants chosen at start; the release gate learns the FMA allowlist.

This note fixes how S6 is built and lists the decisions it takes where the records leave a choice or state something the tree does not have yet. Each decision takes the most secure option the records allow.

## 1. What the tree has, and has not

- ADR 0107 (headers, layout ids, packed lists) is **not implemented** in the native runtime (#736). Every heap object is an `Object` of `crates/una-rt/src/values.rs`; a list is `Object::List(Vec<i64>)` of words; an `F64` outside a register is a boxed `Object::F64`.
- `una-core` has the kernels (`dense.rs`, `dense/blocked.rs`, `dense/fma.rs`) and no target variant. `una-core` is `#![forbid(unsafe_code)]`; the only crate that may hold `unsafe` is `una-rt`, in `ffi.rs` (ADR 0058).
- The FMA disassembly gate of ADR 0114 Appendix C rules 7 and 8 **does not exist** as a script or a test. There is no C release backend.
- The native differential test (`crates/una-cli/tests/native.rs`) requires `tests/native/unsupported.txt` to be exact; seven dense cases are listed there.

So S6 adds the storage, the dispatch and the gate; it does not extend them.

## 2. Two pull requests

- **Part 1, native dense values** (`Refs #989`): the runtime object, the codegen lowering, the budget check, sharing at `together`, rendering and equality, and the seven dense cases run natively. Draft until built.
- **Part 2, dispatch and the gate** (`Closes #989`, `Closes #1216`): the target variants in `una-core`, the one `unsafe` call site in `una-rt`'s `ffi.rs`, the interpreter on the same table, the feature-equality test, the allowlist in the build report, and the disassembly gate. Stacked on part 1. Draft until built.

Both get the security review ADR 0120 asks for (the dispatch macro and the shared buffers) before merge.

## 3. Part 1: the runtime object

**Decision D1: one heap object per dense value, with the elements in a separate store.**

```rust
// crates/una-rt/src/values/dense.rs
enum Object { /* ... */ Dense(Dense), }

pub(crate) struct Dense {
    kind: DenseKind,          // Array, Matrix, Tensor, Lu
    store: Store,
    offset: usize,
    shape: Shape,             // 1 to 8 lengths; Lu is n + 1 rows of n (the interpreter's layout)
    strides: Shape,
}

enum Store {
    /// Canonical F64 elements, raw, atomically counted: shared by the views of one value and,
    /// across tasks, by the copies of a `together` block (ADR 0113 amendment of 2026-10-19).
    F64(Arc<Vec<f64>>),
    /// Every other element, as words: a handle to an `Object::Words` of this task's heap.
    Words(i64),
}
```

- `Store::Words` is a heap object (`Object::Words(Vec<i64>)`, counted non-atomically like a list), so the header's `cells()` are `[store]` and a view retains the same store object: release, export and import work as they do for a list. Integer elements, fixed-width included, are tagged words here. **Packed integer storage at the element's width waits for the raw cells of #736, together with #1089**, as the interpreter's `Elements::Values` does; the representation is not observable (`spec/16 §4`).
- `Store::F64` holds no handle, so a buffer shared across threads reaches no other heap's objects (ADR 0120 Security). It is `Arc` in safe code. A view is a new header over the same `Arc` and store: O(1).
- `kind` mirrors the interpreter's `DenseKind`, `Lu` included, so `==`, rendering (`<lu size n>`) and `factors()`/`pivots()` give the same text and bits in both engines.
- The dense value's node for a copy at `together` is `Node::Dense { kind, store: NodeStore, offset, shape, strides }` with `NodeStore::F64(Arc<Vec<f64>>)` (the same `Arc`, count + 1, charged O(1) as `node_bytes` of the header only) or `NodeStore::Words(Cell)` (the words object copied as a list is). `held()` is process-wide, so a buffer is counted once, at its allocation.

**Decision D2: every allocation is reserved first.** A dense constructor, `compact`, a copy on write, `to_list`, `map` and the kernels that return a new value call `runtime::try_reserve(bytes)` (the process-wide reservation of ADR 0113 rule 4) and then allocate fallibly (`Vec::try_reserve_exact`); a refusal is `refuse(site, None)`, U6003 at the call's site, never an abort. The bytes are computed in checked arithmetic from the shape, so a product that overflows is refused. For `Store::Words` filled from one element, the check adds `node_bytes(element) × n` as the interpreter's `reserve_with` does (`dense_memory_budget_integers`), conservative in both engines.

**Decision D3: in place when uniquely held.** The codegen passes the receiver of an `edit` method at its last use as owned (`_owned` entry points, the pattern of `set_owned`). The runtime writes in place when the header's slot count is 1 and `Arc::get_mut` (or the words object's count) says the store is held once; otherwise it copies its own elements into a fresh compact store first, after D2's check (rule 31). A view edited in place copies only its elements.

**Decision D4: a precondition that fails at run time is an internal abort.** `get` and `set` outside the shape, a kernel with mismatched shapes, a `block` outside the matrix: U8001 at the site through `abort_at(site, Code::U8001, ...)`, exit code 3, as ADR 0112 EL-2 and rule 30 state. No dense method fails (rule 31), so no `catch` path is added.

**Entry points.** One `una_rt_dense_<method>` per prelude function of `crates/una-check/src/std/prelude_0_4.una` (`dense_array_*`, `dense_matrix_*`, `dense_lu_*`, `dense_tensor_*`), the site first for every one that can allocate or abort (nearly all), through the `exported!` table of `ffi.rs` (fixed arity, words in and out). **Decision D5: the tensor functions of each rank are exported per rank** (`una_rt_dense_tensor_get_3(site, t, i, j, k)` and so on, 1 to 8), one-liners generated by a macro over a slice-based implementation: no allocation per access and no pointer passed across the boundary. The codegen maps `dense_tensor_get_3` to `una_rt_dense_tensor_get_3` by name.

**Higher-order methods.** `map`, `zip_map`, `update` and `fold` take a closure. **Decision D6: they go through the uniform word in native code for now**, as a function value that is not an inline lambda does by rule 35: the codegen emits the loop (the shape of `higher_order` for lists) over `una_rt_dense_count(d)`, reads element k in row-major order with `una_rt_dense_item(d, k)` (a boxed `F64` or the element's word), invokes the closure, and writes the result into a builder (`una_rt_dense_builder(site, kind, like: d, count)` then `una_rt_dense_push`), or into the owned receiver for `update`. The result has the bits of the interpreter's. The lowering with the lambda's body inlined over raw elements, which rule 35 asks of every profile, needs raw `F64` cells, which the native backend does not have (`Kind::F64` is a boxed handle until #736); it is filed as a follow-up of #736 and noted in the PR. No boxing is lost that the backend does not lose today for every `F64`.

**Kernels.** `sum`, `dot`, `norm2`, `multiply`, `multiply_array`, `add_product`, `add_scaled`, `scale`, `row_sums`, `column_sums`, `sum_axis`, `contract`, `lu`, `solve*`, `cholesky`, `solve_lower`, `solve_upper` call the `una_core::dense::blocked` kernels on the raw `F64` store, through a view's offset and strides exactly as the interpreter does (compact copy of a strided operand where the kernel wants contiguous input, after D2). Integer `sum_exact` and `dot_exact` run on words with the arbitrary-precision integer path.

**Rendering and equality.** `render.rs` renders a dense value as its construction call in row-major order (rule 41) and `Lu` as `<lu size n>`. `equal_words` compares two dense values by shape, then each element by its own case, within the equality budget (rule 27); a dense value is a composite with parts. Hashing (ADR 0119 rule 9 amendment) is not in this slice: the hash tables are not native yet (`hash_*` in `unsupported.txt`); part 2 of the collections' native lane adds the dense case there.

**Codegen.** `Kind::Dense` (a handle) for `Array`, `Matrix`, `Tensor` and `Lu` in `kind()`; `describe.rs` describes them as `{"shape": "dense"}`; `prelude()` routes every `dense_*` name to its entry point, with the owned variants for `set`, `update`, `add_product`, `add_scaled`, `scale` at the receiver's last use (`moving`). `Prim::Shape` (`len`, `rows`, `columns`, `rank`, `dim`) lowers to `una_rt_dense_shape(d, axis)`.

**Tests.** The seven dense cases leave `tests/native/unsupported.txt` and run in the differential test; `loop_state_dense` too. Runtime unit tests: views share, copy on write, the budget refused before allocation (with `REFUSE_STEP`), export and import of an `F64` store across a task (count + 1, bytes charged once), the words store copied as a list, the `exported_table` fuzz entries for the new words (every entry point survives arbitrary words, #766). A scaling test of `set` in nested loops as `loop_state_scaling` does.

## 4. Part 2: the variants and the dispatch

**Decision D7: the variants live in `una-core`, safe; the one `unsafe` call lives in `una-rt`'s `ffi.rs`; the interpreter stays on the baseline.**

- `crates/una-core/src/dense/variants.rs`: for each kernel with an fma seam (`dot_from`, `dot_pairs`, `add_product`, `multiply_array`, `lu`, `cholesky`, `solve_triangular`, `lu_solve`, `sum`, `norm2`), a module per target with a safe function under `#[target_feature(enable = "...")]` (declaring one needs no `unsafe`; `una-core` keeps its `forbid`):
  - `x86_64_v3`: `avx,avx2,fma`; `x86_64_v4`: `avx,avx2,fma,avx512f,avx512vl,avx512dq,avx512bw`; `aarch64`: the baseline (no attribute; `fma::baseline` is the `fmadd` instruction already); `baseline` on x86-64: `fma::emulated`.
  - The fma of a variant is the processor's instruction, through `core::arch::x86_64::_mm_fmadd_sd` on the scalar lanes (a safe intrinsic from a function with `fma` enabled), never a call. The `_with` kernels of `blocked.rs` become `#[inline(always)]` so that each variant's symbol holds the whole loop; a closure made inside a `#[target_feature]` function inherits its features.
  - `pub struct Kernels { pub dot_from: fn(f64, &[f64], &[f64]) -> f64, /* ... */ }` of safe function pointers, `Kernels::BASELINE`, and `pub const ALLOWLIST: &[&str]` with the demangled module prefixes of the variant functions (`una_core::dense::variants::x86_64_v3`, `..._v4`).
- `crates/una-rt/src/ffi.rs`: `dispatched() -> &'static Kernels`, a `OnceLock` filled once at start with the standard library's safe detection (`is_x86_feature_detected!` for exactly the listed features; AArch64 fixed), and **one macro, `variant_call!`, the one new `unsafe` site**, with its `// SAFETY:` argument: the variant is called only after the detection of exactly its feature list succeeded, and neither the inputs nor the environment select it (rule 56). Its wrappers are safe functions whose pointers fill the table.
- **The interpreter runs the blocked baseline** (rule 53, "The interpreter's cost", and Alternatives: `una-cli` does not depend on `una-rt`, and both crates define a global allocator, so it cannot). Its `Limits` carry a `kernels` table, `BASELINE` by default, the hook rule 53's alternative leaves for a later change; `una-cli` passes `BASELINE`. Native code runs the variant, with the same bits (rule 53), which the differential test compares. The `una-interp` crate stays free of `unsafe` and of `una-rt`.
- **Feature lists.** Rule 56 asks that every feature a variant's attribute implies be detected. The toolchain's `avx` implies `sse3`, `ssse3`, `sse4.1` and `sse4.2`, and `avx512f` implies `f16c`, so each variant enables and detects the closed list: x86-64-v3 is `avx, avx2, fma, sse3, ssse3, sse4.1, sse4.2`; x86-64-v4 adds `f16c, avx512f, avx512vl, avx512dq, avx512bw`. One macro builds the attribute and the detection from the same tokens; the rule 56 test confirms the closure.
- **Feature equality (rule 56).** A test runs the toolchain's `rustc --print cfg -C target-feature=+<list> --target <triple>` for each variant and for the bare triple, takes the difference of the `target_feature` values, and asserts it equals the list the detection checks. It runs on every host, since it asks the toolchain about x86-64 whatever the host is.
- **Bits.** The corpus test of rule 53 (`dense_corpus.rs`) runs every variant the host can run and asserts the reference's bits; on a host without a feature the variant is skipped and the skip is printed.
- **Report.** `NativeDoc` gains `kernels: { variants: [...], fma_allowlist: [...] }` from `ALLOWLIST` (ADR 0106 amendment, "recorded in the build report"); `docs/spec/13` gains the field.
- **The gate.** `scripts/check-release.sh` gains, for an x86-64 artifact, a disassembly scan (`objdump -d -C`, or `llvm-objdump`): any `vfmadd`, `vfmsub`, `vfnmadd` or `vfnmsub` outside a symbol whose demangled name starts with an allowlist prefix fails; a test plants one in a symbol outside the allowlist (a test-only object) and asserts the failure. On AArch64 hosts the scan reports "not applicable" and the test is skipped; the x86-64 leg is the emulated `--linux` leg of `local-ci.sh`. The contraction canary of ADR 0114 Appendix C is confirmed or filed as missing in the PR.

## 5. What this slice does not do

- Raw cells and packed integer stores (#736, #1089); the inlined lambda lowering of rule 35 (follow-up of #736).
- The hash encoding of dense values in native tables (with the native hash tables).
- `simd required` (S8, #991) and its lowering in release builds (the C release backend's vector forms are withdrawn by ADR 0106's amendment of 2026-10-31; open question OQ-6 there, #1308).
- `F32`, `Complex` and `Decimal` elements in native dense values: the checker admits `F64` and the fixed-width integers today (`dense_numeric`); the other element types follow their own slices.

## 6. Costs

- Element access in native code: O(rank) through the entry point, one call per access, as a list's `at` is today; no allocation per access (D5).
- A view: O(1). A copy on write: O(size of the receiver's own elements). A `together` copy of a numeric dense value: O(1) for the buffer, O(1) for the header.
- Kernels: the complexities of ADR 0120 rules 44 to 54, unchanged; a variant changes the constant only.
