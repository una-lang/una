# Formal track: the environment as a source of secrets

- Issue: #487, under #46
- Previous step: `docs/plan/formal-track-m6b.md` (M6b, #485), §3
- Date: 2026-10-10
- Status: statement approved by the coordinator (2026-10-10); proven

## 1. Why

M6b models `Env.get` with the host's environment, `Host.env`, the same in both runs. So T6 says nothing about a program that leaks an environment variable's value: in the model, the two runs read the same value. The `internal` label that `spec/14 §3` gives the value is then conservative but not checked by any theorem. Una-1 flagged the gap in the review of #495.

`spec/14 §3` declares `Env.get(name: Text) -> Option[internal Text]`:
- the name is `public`, its `accepts`;
- the tag, whether the variable exists, is `public`;
- the content is `internal`.

## 2. The model

**The environment moves into the world.**

```lean
structure World (H : Host) where
  fs  : H.FS
  srv : H.Srv
  env : String → Option String   -- the process environment `Env.get` reads
```

`Host.env` goes away. `Env.get name` reads `w.env name`. No operation writes the environment, so every run leaves it as it found it. The unary half proves that, as it proves that a run without a write leaves the file system alone (`WEff`).

**Low-equivalence of worlds** (`WorldRel`) gains one conjunct:

```lean
def WorldRel (w₁ w₂ : World H) : Prop :=
  (Vis p L (.ofLabel p.fileState) → H.shape w₁.fs = H.shape w₂.fs) ∧
  (Vis p L (.ofLabel internalLabel) → w₁ = w₂) ∧
  (∀ x, (w₁.env x).isSome = (w₂.env x).isSome)     -- new: the same variables exist
```

- The variables that exist are the same in both runs, for every observer, since the tag is `public` (`spec/14 §3`).
- Their values may differ, unless the observer sees `internal`, in which case the second conjunct already makes the worlds equal.

So two runs from environments that differ in an `internal` value, a secret in `SECRET` for instance, are related.

## 3. What T6 then says

`t6_host` and its variants keep their statement over the larger world. They now cover two runs whose environments differ in values:
- the observer sees the same events and related outcomes;
- the worlds the runs leave are related, the environments untouched.

**The binary case of `Env.get`.** The name is `public` (`deep(name) ⊑ ⊥`), so both runs read the same variable. The variable exists in both runs or in neither (the new conjunct), so the tags agree. When it exists, the two values are related at `internal Text`: equal when the observer sees `internal`, and hidden otherwise.

**A premise, stated.** The variables that exist are the same in both runs, and their values are the secret. A program that would leak whether a variable exists is outside this premise: the label rules treat existence as `public`, which `spec/14 §3` declares. This is the honest reading of that declaration, written as a premise of the model rather than proven.

**Its consequence, plainly:** whether a variable exists is treated as public configuration. An operator must not encode a secret in which variables are set: a program may branch on it and print, and T6 does not cover that. The guide says so in its `Env` section (`docs/guide/una-for-agents.md`).

## 4. What else changes

- **The note** of a declaration whose program reads `Env` names the new premise and its consequence: "…, and `Env`, whose variables are the same in both runs: whether a variable exists is public configuration, so an operator must not encode a secret in which variables are set".
- **Erasure:** `Env.get`'s value is a text, which erasure keeps. The host's `AnonInvariant` is unchanged.
- **Host-free code:** `Env.get` stays outside host-free code, since it reads the world.
- **The re-check:** no verdict rule changes. Only the note's text changes on the declarations that read `Env`, and the corpus diff shows it.

## 5. Non-vacuity

- **A leak, untyped.** `main` reads `SECRET` and prints `"yes"` when it is `"1"`, at `Console`, which accepts `public`. It has no typing: the branch runs under `internal`.
- **The same read, kept at `internal`.** The value goes into an `internal` result. It is certified, and its two runs, from environments where `SECRET` is `"1"` and `"2"`, are related (`t6_host_ground`) and return different values, both hidden from a `public` observer.
- **Existence is public.** A program that branches on whether `SECRET` exists and prints is typed. Its runs from environments that both hold `SECRET` are related, which the premise of §3 requires.

## 6. Effort

`World` gains a field. Every proof that builds or compares worlds changes: the unary `WEff`, `confined`, `WorldRel.trans` and the T6 cases of the host operations. So does every example's world literal. On M5's evidence that is mostly mechanical, and the one new argument is `Env.get`'s binary case of §3. Lean-only, about half a day.

## 7. Decisions

The coordinator approved the statement on 2026-10-10:
1. **Existence stays `public`**, as a premise, matching `spec/14 §3` and the checker. No ADR. The premise's consequence is stated in this document, in the note, and in the guide.
2. **The note's text** as §4 gives it.

## 8. Result

- **The model.** `World` gains `env`, and `Host.env` is gone. `WEff` gains `w'.env = w.env`, so every run leaves the environment unchanged. `WorldRel` gains the existence conjunct. `t6_host` and every theorem above it keep their statements.
- **The one new argument** is `Env.get`'s binary case in `T6.lean`: the tags agree by the conjunct, and the values are equal under `internal` (the worlds are) or hidden.
- **Non-vacuity** (`EnvExample.lean`):
  - `leak_untyped`: the leak of §5 has no typing. The proof uses T6 itself: its two runs, from the related worlds of `set_rel`, print different lines (`leak_runs`).
  - `read_runs` and `read_differ`: the `internal` read is certified, its runs are related, and they return `"1"` and `"2"`.
  - `exists_runs`: the branch on existence is certified, and its runs from worlds that both set `SECRET` are related.
  - `exists_unrelated`: worlds where `SECRET` is set in one run only are not related, for any observer. This is the premise at work.
- **The gate:** `scripts/formal.sh` states 213 theorems, with no `sorry`.
- **The corpus:** no verdict changes, with 756 accepted and 0 rejected, the same as after #494. Only the `Env` note's text changes, and `ImportTests` pins it.
