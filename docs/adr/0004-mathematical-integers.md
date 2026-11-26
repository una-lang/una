# 0004. Mathematical integers in v0.1

- Status: Accepted
- Date: 2026-09-28
- Amended by: 0057 (v0.2: fixed-width integer types with overflow obligations; `Int` stays mathematical)

## Context

Fixed-width integers require overflow obligations everywhere and a bitvector or bounded encoding. v0.1 runs on an interpreter where arbitrary precision is cheap.

## Decision

`Int` is an unbounded mathematical integer. The interpreter uses arbitrary precision (`num-bigint`). The SMT encoding uses `Int` directly. Division and modulo are Euclidean.

## Consequences

- No overflow obligations in v0.1, and SMT semantics match runtime semantics exactly.
- v0.2 adds fixed-width types (`I64`, `U8`, ...) with overflow obligations, together with the native backend.

## Alternatives considered

- **64-bit integers with overflow obligations now:** more realistic, but multiplies obligations before the core loop is validated.

## Amendment (2026-11-01, #1093): Una's own big integer

The arbitrary precision of `Int` is Una's own big integer, `una_core::bigint`, in place of the `num-bigint` crate the Decision names, in the interpreter and in the native runtime alike. The semantics do not change: `Int` is unbounded, `div` and `mod` are Euclidean, and every operation the language uses gives the same result bit for bit as before, checked against `num-bigint` on random operands before the switch. The algorithms and their costs are in `docs/plan/1093-bigint-design.md`.

## Amendment (2026-11-26, #1223): writing an `Int` has no digit bound

- Status: Accepted (2026-11-26), by the Owner's decision, for v0.5.2, on condition that the adversarial review of the pull request passes.

It changes the rule of #923 (2026-10-27) in `spec/11 §7`, the sentence of ADR 0073 rule F3 that writing stays O(d²), and the reason given for the reading bound of #914; it changes no other decision of this record or of ADR 0073 and ADR 0120.

**Context.** #923 refused to write an `Int` of more than 4300 digits as text: with `num-bigint`, converting d digits took O(d²), and a few multiplications build an integer of millions of digits, so the run aborted with U6003 instead. #1550 then made that abort the same with and without a trace. Una's own big integer (the amendment above) converts in subquadratic time, which is what the rule of the best average complexity asks for in place of a cap.

**Decision.**

1. **Writing.** An `Int` of any size is written in full: by interpolation, `to_text` and `to_json`, and in the Una form of traces, outputs, failures and test reports, in the interpreter and in native code, with a trace or without one. Writing an integer never aborts the run for its number of digits; it aborts with U6003 only when its text does not fit the memory budget (rule 3), as any allocation over the budget does.
2. **Cost.** Writing d digits takes O(M(d) log d) time, M(d) the cost of a product of two d-digit integers (Karatsuba from 40 limbs, Toom-3 from 256: `docs/plan/1093-bigint-design.md` §4), and O(d) space. Below 32 limbs, 617 digits, the value is written by repeated division by 10^9, two hardware divisions per limb, O(d²) on at most 617 digits; from 32 limbs it is split by 10^(19·2^i) at half its size, each split one Burnikel-Ziegler division, down to 8 limbs. A scaling test on the deterministic count of limb steps writes random values of 256 limbs (about 4,900 digits, past the old cap) to 16,384 limbs (about 315,000 digits): 35,654, 122,054, 403,299, 1,308,012, 4,136,847, 12,739,113 and 38,265,791 steps, 3.42x, 3.30x, 3.24x, 3.16x, 3.08x and 3.00x per doubling, where repeated division would grow 4x; the test fails at 3.6x.
3. **What replaces the cap's protection.** The cap bounded the cost of one write. Its time is now the cost class of one multiplication of the integer's size, which the language already lets a program spend on the same integer without a bound (`x * x`), so a write adds no new way to spend time beyond the operations that build and use the integer; like a multiplication, a conversion is not interrupted by a deadline, a host stop or a cancellation. Its space is bounded by the run's memory budget (`spec/11 §7`, ADR 0061) through reservations. Before converting an `Int`, or the coefficient of a `Decimal`, whose text and conversion may hold 64 KiB or more, each engine reserves an upper bound of those bytes, 96 per 64-bit limb (the digits, 20 bytes per limb, their copy into the written text, 20 more, and the divide and conquer's powers, quotients, remainders and division work space within 7 words per limb; measured about 74 bytes per limb in the interpreter at 184,000 limbs), known in O(1) from the bit length. The text that interpolation, an event or a message builds of several parts reserves each growth of its buffer to 64 KiB or more, O(1) amortized per part. When the bytes do not fit, the run aborts with U6003 (`this call exceeds the memory limit of N MiB`) at the write, with a well-formed trace, as at a call. What this guarantees: no such conversion and no such growth passes the allocator's hard ceiling of twice the budget. `to_json`, which checked the budget every 4096 values, also checks it every 64 KiB of the text of numbers and strings it copies and every 64 KiB it writes, so many mid-size values cannot pass the ceiling between two checks. What it does not cover: a write's other temporaries below 64 KiB, which the slack of the checks covers, as every allocation's. Reviews 1 to 3 of the pull request found the cases this closes: a 1.47 MB integer, its `Decimal` twin, a list of 20,000 references to a 6,924-digit integer, each written under `--max-memory 12`, and `to_json` of 4,095 such integers, or of as many strings of that length, under 14 MiB, ended the process at the hard ceiling (exit 134); each is now a test.
4. **Reading keeps its bound.** `parse_int`, `to_int` and `parse_json` still refuse an integer of more than 4300 digits, in O(b) before any conversion, and so do `parse_decimal`, `as_decimal`, decimal literals and constants (ADR 0120 rule 16, ADR 0126): reading costs O(d²) by Horner's rule up to 14,592 digits and O(M(d) log d) beyond, superlinear in a text that may come from outside the program, and the bound keeps the work a text asks for linear in its length, about 26,000 limb operations per number at most. The text of an `Int` of more than 4300 digits is therefore written and not read back: the round trip of `parse_int(to_text(n))` and of `parse_json(to_json(j))` holds up to 4300 digits, as `spec/23 §4` already states for `to_json`.
5. **Information flow.** Whether a write aborted depended on the size of the integer, a property of the value; #1550 kept it independent of the trace, and quiet code that wrote nothing could not abort. Without the cap, the outcome of an operation, its event, its `seq` and the run's status no longer depend on how large an integer is, in either engine, except through the memory budget, which is outside the noninterference model (`spec/07 §1`) as every use of memory is: the value-decided abort is removed, not moved. A write that is not made, an event that is not recorded, reserves nothing.

**Consequences.** `spec/09 §2` (the S-39 paragraph), `spec/11 §5` and §7, `spec/14 §1`, `spec/16 §3` rule 4, `spec/22 §4` and §6 rule 4, `spec/23 §3` rule 5 and §6 rule 1, `spec/26 §6` rule 4, and the explanation of U6003 change with this amendment. This amendment supersedes the sentence of ADR 0073 rule F3, "Writing an `Int` that the program built with more digits stays O(d²): that is a known limit": writing is O(M(d) log d) with no digit bound; ADR 0073 carries a dated pointer to it. The native runtime drops the decision of #923 and the untraced rendering of #1550: an event that is not recorded writes and reserves nothing; `una_rt_int_text`, `una_rt_write` and `una_rt_generic_write` keep their site, where a write that does not fit the budget aborts. The conformance cases `runtime/int_text_cap`, `runtime/int_text_bound`, `runtime/int_text_op_argument` and `runtime/int_text_op_result` now show integers of more than 4300 digits written in full in both engines, with and without a trace, and the bound of reading kept.

**Alternatives considered.** A higher fixed bound on writing (for example 10^6 or 10^8 digits) keeps a cap that no longer protects against a cost the language allows elsewhere, and keeps an abort whose occurrence depends on a value. Raising the reading bound with the writing one would let a text from outside ask for work that grows faster than its length, per byte as well as per number; it stays for a later decision, with its own measurements.
