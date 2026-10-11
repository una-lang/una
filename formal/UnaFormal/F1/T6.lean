/-
**T6, noninterference, for F1** (`spec/10 §10`, M3 and M4 of `docs/plan/formal-track.md`):
two runs of a body the labeled typing accepts, in environments related for an observer at `L`,
end, when both end at the same fuel, with the same events the observer sees and related outcomes,
and from worlds the observer cannot tell apart leave worlds it cannot tell apart (M5).
`t6_host` holds at every ground substitution `σ` of the type parameters in scope, so a generic
body is covered at each of its ground instances; `t6_host_ground` is the case of a body with no
type parameter in scope, every call site and `main`; `t6_ended` (`Mono.lean`) drops the common
fuel.

**Scope.** What the theorem covers is F1 as `Syntax.lean` defines it, and no more:
- one capability with one operation (`op`), and the host's stateful operations, `Files.read`,
  `Files.write` and a request of `Http`, each with one `accepts` label for the whole program;
- a lawful host (`Host.Lawful`, H1 and H2), whose file system and servers are disjoint, under
  ADR 0085's rules and `Program.HostWF`; a program without host operations needs no premise on
  the host (`HostFree.lean`);
- failures are codes, or failures with a payload of a sum kind (#512), whose fields keep their
  declared labels; that a failing `main`'s report shows only its `public` fields is the
  runtime's premise R1 (`docs/plan/formal-track-sums.md` §4), outside the model;
- sums: `Option`, `Result` and the program's sum kinds without type parameters (#512); no `Map`,
  no `with` or handlers of capabilities, no `together`;
- generic functions at ground instances only: the type arguments of a call are ground (the
  premise `LTy.GroundAll` of T-Call and of a function reference), so a generic body that calls a
  generic function at its own type parameters, two levels of generics, is outside the model;
- no policy (`p.policies = []`): declassification is `t6_release` (`Release.lean`, option B1),
  which `t6_soundR` proves for programs within B1's restrictions, up to the first visible release
  whose values differ; `t6_sound` and `t6_host` are its case without policies.

**What "related" means** (`V`): at a position the observer sees, scalars and texts are equal,
lists have the same length and related elements, options and results the same tag and related
contents, records related fields, values of a sum kind the same variant and related fields; closures are related, not equal (the same code and related
captured environments); a `Unit` position relates any two values, since it carries nothing.
-/
import UnaFormal.F1.BinaryInst
import UnaFormal.F1.BinaryR

namespace Una.F1

variable {p : Program} {L : Label} {H : Host}

theorem latom_both {Γ : LCtx} {e₁ e₂ : Env} {a : Atom} {t : LTy} (he₁ : LEnvTy p e₁ Γ)
    (he₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂) (h : LAtomTy Γ a t) :
    ∃ v₁ v₂, evalAtom e₁ a = some v₁ ∧ evalAtom e₂ a = some v₂ ∧ V p L t v₁ v₂ ∧
      LValTy p v₁ t ∧ LValTy p v₂ t := by
  obtain ⟨v₁, v₂, h₁, h₂, hv⟩ := latom_rel hrel h
  obtain ⟨u₁, hu₁, hut₁⟩ := latom_ltyped he₁ h
  obtain ⟨u₂, hu₂, hut₂⟩ := latom_ltyped he₂ h
  rw [h₁] at hu₁
  rw [h₂] at hu₂
  cases hu₁
  cases hu₂
  exact ⟨v₁, v₂, h₁, h₂, hv, hut₁, hut₂⟩

theorem latoms_both {Γ : LCtx} {e₁ e₂ : Env} {as : List Atom} {ts : List LTy}
    (he₁ : LEnvTy p e₁ Γ) (he₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂)
    (h : LAtomsTy Γ as ts) :
    ∃ vs₁ vs₂, evalAtoms e₁ as = some vs₁ ∧ evalAtoms e₂ as = some vs₂ ∧ VList p L ts vs₁ vs₂ ∧
      LValsTy p vs₁ ts ∧ LValsTy p vs₂ ts := by
  obtain ⟨v₁, v₂, h₁, h₂, hv⟩ := latoms_rel hrel h
  obtain ⟨u₁, hu₁, hut₁⟩ := latoms_ltyped he₁ h
  obtain ⟨u₂, hu₂, hut₂⟩ := latoms_ltyped he₂ h
  rw [h₁] at hu₁
  rw [h₂] at hu₂
  cases hu₁
  cases hu₂
  exact ⟨v₁, v₂, h₁, h₂, hv, hut₁, hut₂⟩

/-- Two runs with the same events and outcome, from worlds the observer cannot tell apart, are
related when their outcome is related to itself. -/
theorem brel_same {R : Val → Val → Prop} {vf : Prop} {tr : Trace} {r : Res} {w₁ w₂ : World H}
    (hw : WorldRel p L w₁ w₂) (h : ∀ v, r = .ok v → R v v)
    (hr : ∀ k c ps, r ≠ .raised k c ps := by
      intro _ _ _ h
      first
      | (split at h <;> simp_all)
      | (simp only [Option.elim] at h; split at h <;> simp_all)
      | simp_all) :
    BRel p L R vf (tr, r, w₁) (tr, r, w₂) := by
  intro ht _
  refine ⟨rfl, ?_, hw⟩
  cases r <;> simp only [Term] at ht <;> simp only [OutRel]
  · exact h _ rfl
  · exact fun _ => trivial
  · exact absurd rfl (hr _ _ _)

/-- Two runs of `checked` on the same value take the same branch (#453). -/
theorem brel_checked {R : Val → Val → Prop} {vf : Prop} {w₁ w₂ : World H} {v : Val}
    {pred : String} {code : Nat} (hw : WorldRel p L w₁ w₂) (hR : R v v) :
    BRel p L R vf (checkRun p w₁ v pred code) (checkRun p w₂ v pred code) := by
  unfold checkRun
  split
  · split
    · exact brel_done hw hR
    · exact brel_same hw (fun _ h => nomatch h)
  · exact brel_same hw (fun _ h => nomatch h)

/-- Two runs that show the observer the same events, end with values the relation always
relates, fail only unseen, and leave worlds it cannot tell apart. -/
theorem brel_quiet {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H}
    (hw : WorldRel p L r₁.2.2 r₂.2.2)
    (hp : proj p L r₁.1 = proj p L r₂.1) (hR : ∀ a b, R a b)
    (hf₁ : r₁.2.1.IsFail → ¬ vf) (hf₂ : r₂.2.1.IsFail → ¬ vf) :
    BRel p L R vf r₁ r₂ := by
  intro ht₁ ht₂
  refine ⟨hp, ?_, hw⟩
  obtain ⟨tr₁, o₁, _⟩ := r₁
  obtain ⟨tr₂, o₂, _⟩ := r₂
  simp only at hf₁ hf₂
  cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [OutRel] <;>
    first
    | exact hR _ _
    | exact hf₂ trivial
    | exact hf₁ trivial
    | exact fun hv => absurd hv (hf₁ trivial)

/-- The unary facts of a call of a function value, at the program counter of the call. -/
theorem hof_uok {pc : LExpr} {o : Bool} {g : HostFx} {f : Bool} {P : Val → Prop} {w : World H}
    {run : Run H} (hb : p.Bound pc o g f)
    (h : UOk (FnAllowed o g) P ⟨f = true, fun _ _ _ => False⟩ w run) :
    UOk (Allowed p pc (Eff.ofFn o g f)) P (FailsAs p pc (Eff.ofFn o g f)) w run :=
  UOk.mono (bound_allowed hb (LExpr.le_refl pc)) (fun _ h => h)
    (show FailCond.Le ⟨f = true, fun _ _ _ => False⟩ (FailsAs p pc (Eff.ofFn o g f)) from
      ⟨fun (hf : f = true) => ⟨bound_fails hb (LExpr.le_refl pc) hf, by simp [Eff.ofFn, hf]⟩,
        fun _ _ _ h => h.elim⟩) h

theorem not_vis_join_left {a b : LExpr} (h : ¬ Vis p L a) : ¬ Vis p L (a.join b) :=
  fun hv => h (Vis.left hv)

theorem not_vis_join_right {a b : LExpr} (h : ¬ Vis p L b) : ¬ Vis p L (a.join b) :=
  fun hv => h (Vis.right hv)

theorem V.ofScalar_refl {t : Ty} {l : LExpr} {u : Val} (h : LValTy p u (ofScalar t l)) :
    V p L (ofScalar t l) u u := by
  unfold ofScalar at h ⊢
  split at h <;> cases h <;> first | exact .bool | exact .text | exact .int

theorem ofScalar_top (t : Ty) (l : LExpr) : (ofScalar t l).top = l := by
  unfold ofScalar
  split <;> rfl

theorem le_pc_xl {top : Label} (pc a b : LExpr) :
    LExpr.le top (a.join b) ((pc.join a).join b) :=
  LExpr.join_mono (LExpr.le_join_right pc a) (LExpr.le_refl b)

theorem le_pc_xlb {top : Label} (pc a b c d : LExpr) :
    LExpr.le top ((a.join b).join d) ((((pc.join a).join b).join c).join d) :=
  LExpr.join_mono (LExpr.le_trans (le_pc_xl pc a b) (LExpr.le_join_left _ c)) (LExpr.le_refl d)

/-! ### The host's operations -/

theorem text_of_erase : ∀ {t : LTy}, t.erase = .text → ∃ l, t = .text l
  | .text l, _ => ⟨l, rfl⟩
  | .int _, h | .bool _, h | .unit _, h | .list _ _, h | .option _ _, h | .result _ _ _, h
  | .record _ _, h | .fn _ _ _ _ _ _, h | .var _ _, h => by simp [LTy.erase] at h

/-- `Http`'s argument types have equality: `Header`'s fields are texts (M5c). -/
theorem httpArg_eqIn (hr : p.HttpRecords) {E : List String} :
    ∀ {t : LTy}, t.erase.HttpArg → t.eqIn E (p.recordEq 2) = true
  | .text _, _ => rfl
  | .option t' _, h => by
    rcases h with h | h | h <;> simp [LTy.erase] at h
    obtain ⟨l, rfl⟩ := text_of_erase h
    rfl
  | .list t' _, h => by
    rcases h with h | h | h <;> simp [LTy.erase] at h
    cases t' <;> simp [LTy.erase] at h
    subst h
    obtain ⟨⟨rec, hrec, hf⟩, _⟩ := hr
    simp only [LTy.eqIn, Program.recordEq, hrec, List.all_eq_true]
    intro f hf'
    have := List.mem_map_of_mem (f := LTy.erase) hf'
    simp only [Record.fieldTys] at hf
    rw [hf] at this
    simp at this
    obtain ⟨l, rfl⟩ := text_of_erase this
    rfl
  | .int _, h | .bool _, h | .unit _, h | .result _ _ _, h | .record _ _, h | .fn _ _ _ _ _ _, h
  | .var _ _, h => by rcases h with h | h | h <;> simp [LTy.erase] at h

/-- Related values of `Http`'s argument types whose deep labels the observer sees are equal. -/
theorem VList.eq_http (hwf : p.LWellFormed) (hr : p.HttpRecords) :
    ∀ {ts : List LTy} {vs₁ vs₂ : List Val}, VList p L ts vs₁ vs₂ → LValsTy p vs₁ ts →
      LValsTy p vs₂ ts → (∀ t ∈ ts, t.erase.HttpArg ∧ Vis p L (p.deep t)) → vs₁ = vs₂
  | [], _, _, .nil, _, _, _ => rfl
  | _ :: _, _, _, .cons h hs, .cons ha has, .cons hb hbs, ht => by
    obtain ⟨hshape, hvis⟩ := ht _ List.mem_cons_self
    rw [V.eq_of_deep hwf (E := []) (n := 2) h ha hb (httpArg_eqIn hr hshape) hvis,
      VList.eq_http hwf hr hs has hbs (fun t ht' => ht t (List.mem_cons_of_mem _ ht'))]

/-- **Equal responses are related** at `Response`, whatever its label (H4). -/
theorem V.response_refl (hr : p.HttpRecords) {v : Val} (h : v.IsResponse) (l : LExpr) :
    V p L (.record "prelude.Response" l) v v := by
  obtain ⟨⟨hrec, hh, hhf⟩, ⟨rrec, hrr, hrf⟩⟩ := id hr
  unfold Val.IsResponse at h
  split at h
  · rename_i st hs body
    refine .record hrr ?_
    simp only [Record.fieldTys] at hrf
    rcases hfs : rrec.fields with _ | ⟨f₁, _ | ⟨f₂, _ | ⟨f₃, _ | ⟨f₄, rest⟩⟩⟩⟩ <;> rw [hfs] at hrf <;>
      simp at hrf
    obtain ⟨h₁, h₂, h₃⟩ := hrf
    refine .cons ?_ (.cons ?_ (.cons ?_ .nil))
    · cases f₁ <;> simp [LTy.erase] at h₁
      exact .int
    · cases f₂ <;> simp [LTy.erase] at h₂
      rename_i fe fl
      cases fe <;> simp [LTy.erase] at h₂
      subst h₂
      refine .list ?_
      induction hs with
      | nil => exact .nil
      | cons x xs ih =>
        obtain ⟨n, y, rfl⟩ := h x (by simp)
        refine .cons ?_ (ih fun z hz => h z (by simp [hz]))
        refine .record hh ?_
        simp only [Record.fieldTys] at hhf
        rcases hgs : hrec.fields with _ | ⟨g₁, _ | ⟨g₂, _ | ⟨g₃, rest⟩⟩⟩ <;> rw [hgs] at hhf <;>
          simp at hhf
        obtain ⟨k₁, k₂⟩ := hhf
        obtain ⟨_, rfl⟩ := text_of_erase k₁
        obtain ⟨_, rfl⟩ := text_of_erase k₂
        exact .cons .text (.cons .text .nil)
    · obtain ⟨_, rfl⟩ := text_of_erase h₃
      exact .text
  · exact h.elim

/-- Related texts the observer sees are equal. -/
theorem VList.eq_texts : ∀ {ts : List LTy} {vs₁ vs₂ : List Val}, VList p L ts vs₁ vs₂ →
    (∀ t ∈ ts, ∃ l, t = .text l ∧ Vis p L l) → vs₁ = vs₂
  | [], _, _, .nil, _ => rfl
  | _ :: _, _, _, .cons h hs, ht => by
    obtain ⟨l, rfl, hl⟩ := ht _ List.mem_cons_self
    rw [V.eq_text h hl, VList.eq_texts hs (fun t ht' => ht t (List.mem_cons_of_mem _ ht'))]

theorem vis_ofLabel_of_inst {s : List (String × LExpr)} {l : Label}
    (h : Vis p L ((LExpr.ofLabel l).inst s)) : Vis p L (.ofLabel l) := by
  rw [LExpr.inst_ofLabel] at h; exact Vis.right h

/-- Seeing an `accepts` shows the instance of what flows into it. -/
theorem vis_of_le_accepts {s : List (String × LExpr)} {x : LExpr} {l : Label}
    (hle : p.le x (.ofLabel l)) (h : Vis p L (.ofLabel l)) : Vis p L (x.inst s) :=
  Vis.of_le (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst s hle)) h

/-- The failure of a `Files` operation decided at `pc ⊔ ℓ`, as the observer sees it: it sees the
path's label and the file-state label. -/
theorem vis_fsFail {s : List (String × LExpr)} (hh : p.HostWF) {pc l : LExpr}
    (h : Vis p L ((p.fsFail (pc.join l)).inst s)) :
    Vis p L (l.inst s) ∧ Vis p L (.ofLabel p.fileState) := by
  simp only [Program.fsFail, hh.oldHost, Bool.false_eq_true, ite_false] at h
  exact ⟨Vis.of_le (LExpr.inst_le_join_inst _ _ _) (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) h),
    vis_ofLabel_of_inst (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h)⟩

/-- A request's failure, under ADR 0085's rules, is at least `internal`. -/
theorem vis_reqFail {s : List (String × LExpr)} (hh : p.HostWF) {pc : LExpr} {ts : List LTy}
    (h : Vis p L ((p.reqFail pc ts).inst s)) : Vis p L (.ofLabel internalLabel) := by
  simp only [Program.reqFail, hh.oldHost, Bool.false_eq_true, ite_false] at h
  exact Vis.of_le hh.reqFloor (vis_ofLabel_of_inst (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))

/-! ### M6: tuples, `append`, `set` and loops -/

/-- `brel_confined` for a relation that holds of the runs' typed values (M6b). -/
theorem brel_confined_typed {R : Val → Val → Prop} {vf : Prop} {pc : LExpr} {ε₁ ε₂ : Eff}
    {P₁ P₂ : Val → Prop} {w₁ w₂ : World H} {r₁ r₂ : Run H} (hwf : p.HostWF)
    (hw : WorldRel p L w₁ w₂)
    (h₁ : UOk (Allowed p pc ε₁) P₁ (FailsAs p pc ε₁) w₁ r₁)
    (h₂ : UOk (Allowed p pc ε₂) P₂ (FailsAs p pc ε₂) w₂ r₂)
    (hpc : ¬ Vis p L pc) (hR : ∀ a b, P₁ a → P₂ b → R a b) (hvf₁ : vf → Vis p L ε₁.failLabel)
    (hvf₂ : vf → Vis p L ε₂.failLabel) : BRel p L R vf r₁ r₂ := by
  intro ht₁ ht₂
  obtain ⟨hp₁, hf₁, hw₁⟩ := confined (L := L) hwf h₁ hpc
  obtain ⟨hp₂, hf₂, hw₂⟩ := confined (L := L) hwf h₂ hpc
  refine ⟨by rw [hp₁, hp₂], ?_, hw₁.symm.trans (hw.trans hw₂)⟩
  obtain ⟨tr₁, o₁, _⟩ := r₁
  obtain ⟨tr₂, o₂, _⟩ := r₂
  have u₁ := h₁.2.1
  have u₂ := h₂.2.1
  simp only at hf₁ hf₂
  cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [OutRel] <;>
    first
    | exact hR _ _ u₁ u₂
    | exact fun hv => hf₂ trivial (hvf₂ hv)
    | exact fun hv => absurd (hvf₁ hv) (hf₁ trivial)

/-- `brel_quiet` for a relation that holds of the runs' values when they end with one. -/
theorem brel_quiet_typed {R : Val → Val → Prop} {vf : Prop} {r₁ r₂ : Run H}
    (hw : WorldRel p L r₁.2.2 r₂.2.2)
    (hp : proj p L r₁.1 = proj p L r₂.1) (hR : ∀ a b, r₁.2.1 = .ok a → r₂.2.1 = .ok b → R a b)
    (hf₁ : r₁.2.1.IsFail → ¬ vf) (hf₂ : r₂.2.1.IsFail → ¬ vf) :
    BRel p L R vf r₁ r₂ := by
  intro ht₁ ht₂
  refine ⟨hp, ?_, hw⟩
  obtain ⟨tr₁, o₁, _⟩ := r₁
  obtain ⟨tr₂, o₂, _⟩ := r₂
  simp only at hf₁ hf₂ hR
  cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [OutRel] <;>
    first
    | exact hR _ _ rfl rfl
    | exact hf₂ trivial
    | exact hf₁ trivial
    | exact fun hv => absurd hv (hf₁ trivial)

/-- Payloads related at their declared ground types are related at those types raised and
instantiated (#512). -/
theorem VList.raise_lsubst_ground {σ : List (String × LTy)} {l : LExpr} :
    ∀ {fs : List LTy} {as bs : List Val}, (∀ f ∈ fs, f.Ground) → VList p L fs as bs →
      VList p L ((fs.map (LTy.raise l)).map (LTy.lsubst p.recordDeep σ)) as bs
  | [], [], [], _, .nil => .nil
  | _ :: _, _ :: _, _ :: _, hg, .cons h hs =>
    .cons (V.raiseσ _ ((V.ground_lsubst (hg _ List.mem_cons_self)).mpr h))
      (VList.raise_lsubst_ground (fun f hf => hg f (List.mem_cons_of_mem _ hf)) hs)

/-- The bindings of a `case` on a sum (#512): related fields, bound in front of related
environments. -/
theorem EnvRel.zip {Γ : LCtx} {e₁ e₂ : Env} (h : EnvRel p L Γ e₁ e₂) :
    ∀ {xs : List String} {ts : List LTy} {vs₁ vs₂ : List Val}, VList p L ts vs₁ vs₂ →
      ts.length = xs.length → EnvRel p L (xs.zip ts ++ Γ) (xs.zip vs₁ ++ e₁) (xs.zip vs₂ ++ e₂)
  | [], _, _, _, _, _ => by simpa using h
  | _ :: _, _, _, _, .nil, hl => by simp at hl
  | _ :: _, _, _, _, .cons hv hs, hl => by
    simp only [List.zip_cons_cons, List.cons_append]
    exact .cons hv (EnvRel.zip h hs (by simpa using hl))

theorem vis_bot_inst (s : List (String × LExpr)) : Vis p L (LExpr.bot.inst s) := by
  rw [show LExpr.bot = LExpr.ofLabel Label.bot from rfl, LExpr.inst_ofLabel]
  exact Vis.join Vis.bot Vis.bot

theorem VAll.append_one {t : LTy} : ∀ {xs ys : List Val} {a b : Val}, VAll p L t xs ys →
    V p L t a b → VAll p L t (xs ++ [a]) (ys ++ [b])
  | _, _, _, _, .nil, h => .cons h .nil
  | _, _, _, _, .cons h hs, hab => .cons h (VAll.append_one hs hab)

theorem VAll.set {t : LTy} : ∀ {xs ys : List Val} {a b : Val} (i : Nat), VAll p L t xs ys →
    V p L t a b → VAll p L t (xs.set i a) (ys.set i b)
  | _, _, _, _, _, .nil, _ => .nil
  | _, _, _, _, 0, .cons _ hs, hab => .cons hab hs
  | _, _, _, _, i + 1, .cons h hs, hab => .cons h (VAll.set i hs hab)

theorem VAll.refl_int {l : LExpr} : ∀ {xs : List Val}, (∀ x ∈ xs, ∃ k, x = .int k) →
    VAll p L (.int l) xs xs
  | [], _ => .nil
  | _ :: _, h => by
    obtain ⟨k, rfl⟩ := h _ List.mem_cons_self
    exact .cons .int (VAll.refl_int (fun x hx => h x (List.mem_cons_of_mem _ hx)))

theorem VAll.of_hidden {t : LTy} (ht : ¬ Vis p L t.top) :
    ∀ {xs ys : List Val}, xs.length = ys.length → VAll p L t xs ys
  | [], [], _ => .nil
  | _ :: _, _ :: _, h => .cons (.hidden ht) (VAll.of_hidden ht (by simpa using h))
  | [], _ :: _, h | _ :: _, [], h => by simp at h

theorem brel_abort_left {R : Val → Val → Prop} {vf : Prop} {w : World H} {r : Run H} :
    BRel p L R vf (abort w) r := fun h _ => by simp [abort, Term] at h

theorem brel_abort_right {R : Val → Val → Prop} {vf : Prop} {w : World H} {r : Run H} :
    BRel p L R vf r (abort w) := fun _ h => by simp [abort, Term] at h

section
variable {σ : List (String × LTy)}

theorem VList.raiseσ (e : LExpr) : ∀ {ts : List LTy} {xs ys : List Val},
    VList p L (ts.map (LTy.lsubst p.recordDeep σ)) xs ys →
    VList p L ((ts.map (LTy.raise e)).map (LTy.lsubst p.recordDeep σ)) xs ys
  | [], [], [], .nil => .nil
  | _ :: _, _ :: _, _ :: _, .cons h hs => .cons (V.raiseσ e h) (VList.raiseσ e hs)

theorem VList.hideσ {e : LExpr} (he : e.PlainFree) (hv : ¬ Vis p L (e.inst (p.sd σ))) :
    ∀ {ts : List LTy} {xs ys : List Val}, xs.length = ts.length → ys.length = ts.length →
      VList p L ((ts.map (LTy.raise e)).map (LTy.lsubst p.recordDeep σ)) xs ys
  | [], [], [], _, _ => .nil
  | _ :: _, _ :: _, _ :: _, h₁, h₂ =>
    .cons (V.hideσ he _ _ hv) (VList.hideσ he hv (by simpa using h₁) (by simpa using h₂))
  | [], _ :: _, _, h, _ | [], _, _ :: _, _, h | _ :: _, [], _, h, _ | _ :: _, _, [], _, h => by
    simp at h

/-- **A tuple decided by a label the observer does not see** (M6b): two tuples of the types `ts`,
each component of which is at least the label (`Flows (ts.map (raise e)) ts`), are related
component by component, each hidden. A `Unit` component needs nothing: any two units are related. -/
theorem VList.hide_covσ {e : LExpr} (he : e.PlainFree) (hv : ¬ Vis p L (e.inst (p.sd σ))) :
    ∀ {ts : List LTy} {xs ys : List Val}, Flows p.top (ts.map (LTy.raise e)) ts →
      LValsTy p xs (ts.map (LTy.lsubst p.recordDeep σ)) →
      LValsTy p ys (ts.map (LTy.lsubst p.recordDeep σ)) →
      VList p L (ts.map (LTy.lsubst p.recordDeep σ)) xs ys
  | [], [], [], _, _, _ => .nil
  | _ :: _, x :: _, y :: _, hcov, .cons _ hx, .cons _ hy => by
    cases hcov with
    | cons f fs => exact .cons (V.flow (V.hideσ he x y hv) (Flow.lsubst _ _ f)) (VList.hide_covσ he hv fs hx hy)
  | [], _ :: _, _, _, h, _ | [], _, _ :: _, _, _, h | _ :: _, [], _, _, h, _
  | _ :: _, _, [], _, _, h => by cases h

theorem V.tuple_covσ {e : LExpr} {ts : List LTy} (he : e.PlainFree) (hv : ¬ Vis p L (e.inst (p.sd σ)))
    (hcov : Flows p.top (ts.map (LTy.raise e)) ts) {a b : Val}
    (ha : LValTy p a ((LTy.tuple ts .bot).lsubst p.recordDeep σ))
    (hb : LValTy p b ((LTy.tuple ts .bot).lsubst p.recordDeep σ)) :
    V p L ((LTy.tuple ts .bot).lsubst p.recordDeep σ) a b := by
  simp only [LTy.lsubst, LTy.lsubsts_eq] at ha hb ⊢
  cases ha with
  | tuple ha =>
    cases hb with
    | tuple hb => exact .tuple (VList.hide_covσ he hv hcov ha hb)

/-- A body's tuple, at the type of a loop's state, is a related pair of states. -/
theorem stRel_of {S : List LTy} {r₁ r₂ : Val}
    (h : V p L ((LTy.tuple S .bot).lsubst p.recordDeep σ) r₁ r₂) :
    StRel (p := p) (L := L) (S.map (LTy.lsubst p.recordDeep σ)) r₁ r₂ := by
  simp only [LTy.lsubst, LTy.lsubsts_eq] at h
  cases h with
  | hidden h => exact absurd (vis_bot_inst _) h
  | tuple h => exact ⟨_, _, rfl, rfl, h⟩

theorem tupTy_of {S : List LTy} {r : Val} (h : LValTy p r ((LTy.tuple S .bot).lsubst p.recordDeep σ)) :
    TupTy p (S.map (LTy.lsubst p.recordDeep σ)) r := by
  simp only [LTy.lsubst, LTy.lsubsts_eq] at h
  cases h with
  | tuple h => exact ⟨_, rfl, h⟩

end

/-- A value of a scalar type is not `Err(c)`: a successful release and a failing one never carry
the same value (#453 Phase B). -/
theorem release_ok_not_err {r : Val} {t : Ty} (ht : t.Scalar) (h : ValTy p r t) :
    ∀ c, r ≠ .err c := by
  intro c hc
  subst hc
  cases h <;> simp [Ty.Scalar] at ht

-- One induction over every rule: with the rules of #512, its elaboration needs more than the
-- default budget.
set_option maxHeartbeats 800000 in
/-- **T6 for F1 with declassification (option B1), at every ground instance**: runs related up
to their first different release. -/
theorem t6_soundR (hwf : p.LWellFormed) (hb1 : p.B1WellFormed) (hh : p.HostWF)
    (hlaw : H.Lawful) : ∀ n : Nat,
    (∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
      {pc : LExpr} {r : Rhs} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace →
      pc.PlainFree → WorldRel p L w₁ w₂ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LRhsTy p E Γ pc r t ε →
      BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (evalRhs H n p w₁ e₁ r) (evalRhs H n p w₂ e₂ r)) ∧
    (∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
      {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace →
      pc.PlainFree → WorldRel p L w₁ w₂ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ pc e t ε →
      BRelR p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e))
  | 0 => ⟨fun _ _ _ _ _ _ _ _ _ h => by simp [evalRhs, Term] at h,
      fun _ _ _ _ _ _ _ _ _ h => by simp [eval, Term] at h⟩
  | n + 1 => by
    obtain ⟨ihR, ihE⟩ := t6_soundR hwf hb1 hh hlaw n
    obtain ⟨uR, uE⟩ := unary_soundR (H := H) hwf hb1 hlaw.resp_typed n
    have hrunU : ∀ {σ : List (String × LTy)} {w : World H} {env : Env} {Γ : LCtx} {E : List String}
        {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → LEnvTy p env (Γ.lsub p.recordDeep σ) →
        LExprTy p E Γ .bot e t ε →
        UGood p (LExpr.bot.inst (p.sd σ)) (t.lsubst p.recordDeep σ) (ε.inst (p.sd σ)) w
          (eval H n p w env e) :=
      fun hσ henv he => uE hσ henv he
    have hrunB : ∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx}
        {E : List String} {body : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E →
        LCtx.InPlace Γ → WorldRel p L w₁ w₂ →
        LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
        EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ .bot body t ε → ε.rels = [] →
        BRel p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
          (eval H n p w₁ e₁ body) (eval H n p w₂ e₂ body) :=
      fun hσ hE hΓ hw h₁ h₂ hr hb hrl =>
        BRelR.to_brel (noRelease_of_ugood (uE hσ h₁ hb) hrl) (ihE hσ hE hΓ rfl hw h₁ h₂ hr hb)
    constructor
    · intro σ w₁ w₂ e₁ e₂ Γ E pc r t ε hσ hE hΓ hpc hw he₁ he₂ hrel hr
      cases hr with
      | atom ha =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.elim]
        exact brel_done hw hv
      | @prim _ _ pr _ ts ty has hp =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.bind_some, Option.map_some, Option.getD_some]
        rw [ofScalar_lsubst]
        by_cases hv : Vis p L ((p.primLabel pr ts).inst (p.sd σ))
        · -- Seen: the arguments agree, so both runs abort, or neither does.
          rw [prim_agreeσ hp hvs hv, aborts_agreeσ hp hvs hv]
          by_cases ha : Prim.aborts pr vs₂ = true <;> simp only [ha, if_true, if_false,
            Bool.false_eq_true]
          · apply brel_same hw
            intro v hv'
            simp at hv'
          · apply brel_same hw
            intro v hv'
            cases hev : evalPrim pr vs₂ with
            | none => simp [hev] at hv'
            | some u =>
              simp only [hev, Option.elim] at hv'
              cases hv'
              exact V.ofScalar_refl (prim_ltyped (p := p) _ hp hev)
        · -- Hidden: either run may abort (#526); an abort is quiet, with no event and no failure.
          rcases Bool.eq_false_or_eq_true (Prim.aborts pr vs₁) with ha₁ | ha₁ <;>
          rcases Bool.eq_false_or_eq_true (Prim.aborts pr vs₂) with ha₂ | ha₂ <;>
          simp only [ha₁, ha₂, ↓reduceIte, Bool.false_eq_true] <;>
          refine brel_quiet hw rfl (fun a b => .hidden (by rw [ofScalar_top]; exact hv)) ?_ ?_ <;>
          (intro hc
           cases h₁ : evalPrim pr vs₁ <;> cases h₂ : evalPrim pr vs₂ <;>
             simp [h₁, h₂, Option.elim, Res.IsFail] at hc)
      | @call _ _ f targs args fn ts hf hnv hlen _ heqa hg has hfl hb _ =>
        obtain ⟨tb, εb, hbody, hflow, _, _, _, hfail, _⟩ := funOk_of_nvia hwf hnv hf
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        have hEτ := eqArgsOK_of_call hwf hf hlen hg heqa
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        have hpg : ∀ u ∈ fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2), u.Ground := by
          intro u hu
          obtain ⟨q, hq, rfl⟩ := List.mem_map.mp hu
          exact LTy.lsubst_ground _ hτ hcov (hps q hq)
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_bothσ he₁ he₂ hrel has
        have ha₁ := LValsTy.ground_lsubst hpg (LValsTy.flowsσ hu₁ hfl)
        have ha₂ := LValsTy.ground_lsubst hpg (LValsTy.flowsσ hu₂ hfl)
        have hva := VList.ground_lsubst hpg (VList.flowsσ hvs hfl)
        simp only [evalRhs, hf, h₁, h₂]
        rw [ite_eq_left (by rw [LValsTy.length ha₁, List.length_map]),
          ite_eq_left (by rw [LValsTy.length ha₂, List.length_map])]
        have hen : ∀ {vs : List Val}, LValsTy p vs (fn.params.map (fun q => p.lsub (fn.tparams.zip targs) q.2)) →
            LEnvTy p (bindParams fn.params vs []) (LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params) := by
          intro vs hvs'
          have := bindParams_ltyped (params := LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params)
            (by rw [LCtx.lsub_snd]; simpa [Program.lsub, List.map_map, Function.comp_def] using hvs')
            (LEnvTy.nil (p := p))
          simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
        have hrel' : EnvRel p L (LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params)
            (bindParams fn.params vs₁ []) (bindParams fn.params vs₂ []) := by
          have := bindParams_rel (params := LCtx.lsub p.recordDeep (fn.tparams.zip targs) fn.params)
            (by rw [LCtx.lsub_snd]; simpa [Program.lsub, List.map_map, Function.comp_def] using hva)
            (EnvRel.nil (p := p) (L := L))
          simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
        have hΓf : LCtx.InPlace fn.params := fun q hq => (hwf.inPlace.1 fn (find_mem hf)).1 q hq
        refine BRelR.mono (fun _ _ h => ?_) (fun _ => vis_failLabel_inst _ hfail)
          (ihE hτ hEτ hΓf rfl hw (by simpa using hen ha₁) (by simpa using hen ha₂) (by simpa using hrel') hbody)
        have hr := LTy.lsubst_ground p.recordDeep hτ hcov hres
        exact (V.ground_lsubst hr).mpr (V.flow h (Flow.lsubst _ _ hflow))
      | @apply _ _ f args ps r o g fl lf ts hf has hfl hb =>
        refine BRelR.of_brel ?_
        obtain ⟨fv₁, fv₂, hf₁, hf₂, hfv, hft₁, hft₂⟩ := latom_bothσ he₁ he₂ hrel hf
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hu₁, hu₂⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, hf₁, hf₂, h₁, h₂]
        have hfi := latom_inPlace hΓ hf
        simp only [LTy.InPlace] at hfi
        have hdp : (lf.join (decision (decisionsOf ts))).PlainFree :=
          LExpr.plainFree_join hfi.2.2 rfl
        simp only [LTy.lsubst, LTy.lsubsts_eq] at hfv hft₁ hft₂
        by_cases hvis : Vis p L ((lf.join (decision (decisionsOf ts))).inst (p.sd σ))
        · have hd : Vis p L ((decision (decisionsOf ts)).inst (p.sd σ)) :=
            Vis.of_le (LExpr.inst_le_join_inst _ _ _) hvis
          have hl : Vis p L (lf.inst (p.sd σ)) := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) hvis
          exact BRel.mono (fun _ _ h => V.raiseσ _ h) id
            (applyWith_brel hrunB hw hfv hl
              (VList.unraiseσ rfl hd (VList.flowsσ hvs hfl))
              (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₁ hfl))
              (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₂ hfl)) _)
        · have hpc' : ¬ Vis p L ((pc.join (lf.join (decision (decisionsOf ts)))).inst (p.sd σ)) :=
            fun h => hvis (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h)
          exact brel_confined hh hw
            (hof_uok (hb.inst _) (applyWith_uok hrunU hft₁ (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₁ hfl)) w₁))
            (hof_uok (hb.inst _) (applyWith_uok hrunU hft₂ (LValsTy.lsubst_unraise (LValsTy.flowsσ hu₂ hfl)) w₂))
            hpc' (fun a b => V.hideσ hdp a b hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
      | @op _ _ line t hl ht hle _ =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase, Ty.Scalar] at ht
        all_goals
          rename_i l
          obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hl
          simp only [LTy.lsubst] at hv hu₁ hu₂
          have hacc : Vis p L (.ofLabel p.acceptsOp) → Vis p L (l.inst (p.sd σ)) := fun h =>
            Vis.of_le (LExpr.le_trans (LExpr.inst_le_join_inst _ _ _)
              (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle))) h
          by_cases hd : Vis p L (l.inst (p.sd σ))
          · have hs : v₁ = v₂ := by
              first | exact V.eq_int hv hd | exact V.eq_bool hv hd | exact V.eq_text hv hd
            subst hs
            cases hu₁ <;> simp only [evalRhs, h₁, h₂, Val.line] <;>
              exact brel_same hw (fun _ h => by cases h; exact .unit)
          · cases hu₁ <;> cases hu₂ <;> simp only [evalRhs, h₁, h₂, Val.line] <;>
              exact brel_quiet hw
                (by simp only [proj_single_hidden (ev := .out _) (fun h => hd (hacc h))])
                (fun _ _ => .unit) (fun h => by simp [Res.IsFail] at h) (fun h => by simp [Res.IsFail] at h)
      | @read _ _ path t hu ht hle _ =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase] at ht
        rename_i l
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hu
        simp only [LTy.lsubst] at hv hu₁ hu₂
        cases hu₁
        cases hu₂
        rename_i s₁ s₂
        simp only [evalRhs, h₁, h₂]
        have hacc : Vis p L (.ofLabel p.acceptsRead) → Vis p L (l.inst (p.sd σ)) := fun h =>
          Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts hle h)
        have hpath : Vis p L (l.inst (p.sd σ)) → s₁ = s₂ := fun hd => by
          have := V.eq_text hv hd; injection this
        have hev : proj p L [Event.read s₁] = proj p L [Event.read s₂] := by
          by_cases ha : Vis p L (.ofLabel p.acceptsRead)
          · rw [hpath (hacc ha)]
          · rw [proj_single_hidden (ev := .read s₁) ha, proj_single_hidden (ev := .read s₂) ha]
        by_cases hi : Vis p L (.ofLabel internalLabel)
        · have hweq := hw.2.1 hi
          subst hweq
          have hs := hpath (hacc (Vis.of_le hh.read hi))
          subst hs
          refine brel_same (WorldRel.refl _) (fun v hv' => ?_)
          split at hv' <;> simp at hv'
          subst hv'
          exact .text
        · have hres : ¬ Vis p L ((LExpr.ofLabel p.readResult).inst (p.sd σ)) := fun h =>
            hi (Vis.of_le hh.readResult (vis_ofLabel_of_inst h))
          have hsame : Vis p L ((p.fsFail (pc.join (p.deep (.text l)))).inst (p.sd σ)) →
              ((H.read s₁ w₁.fs).isSome ↔ (H.read s₂ w₂.fs).isSome) := fun hvf => by
            obtain ⟨hl, hfs⟩ := vis_fsFail hh hvf
            rw [hpath hl]
            exact hlaw.read_fails s₂ (hw.1 hfs)
          intro ht₁ ht₂
          cases hr₁ : H.read s₁ w₁.fs <;> cases hr₂ : H.read s₂ w₂.fs <;>
            simp only [hr₁, hr₂] at ht₁ ht₂ ⊢ <;> refine ⟨hev, ?_, hw⟩ <;> simp only [OutRel]
          all_goals first
            | exact fun _ => rfl
            | trivial
            | exact .hidden hres
            | exact fun _ => trivial
            | exact fun hvf => by
                have := hsame (by simpa [Eff.inst, Eff.failLabel] using hvf)
                simp [hr₁, hr₂] at this
      | @write _ _ path content t c hu ht hc hct hle hst _ =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase] at ht
        cases c <;> simp [LTy.erase] at hct
        rename_i l lc
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hu
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hc
        simp only [LTy.lsubst] at hv hu₁ hu₂ hcv hcu₁ hcu₂
        cases hu₁
        cases hu₂
        cases hcu₁
        cases hcu₂
        rename_i s₁ s₂ cs₁ cs₂
        simp only [evalRhs, h₁, h₂, hc₁, hc₂]
        have hpath : Vis p L (l.inst (p.sd σ)) → s₁ = s₂ := fun hd => by
          have := V.eq_text hv hd; injection this
        have hcont : Vis p L (lc.inst (p.sd σ)) → cs₁ = cs₂ := fun hd => by
          have := V.eq_text hcv hd; injection this
        have hacc : Vis p L (.ofLabel p.acceptsWrite) →
            Vis p L (l.inst (p.sd σ)) ∧ Vis p L (lc.inst (p.sd σ)) := fun h =>
          ⟨Vis.of_le (LExpr.inst_le_join_inst _ _ _)
              (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) (vis_of_le_accepts hle h)),
            Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts hle h)⟩
        have hstate : Vis p L (.ofLabel p.fileState) → Vis p L (l.inst (p.sd σ)) := fun h =>
          Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts (hst hh.oldHost) h)
        have hev : proj p L [Event.write s₁ cs₁] = proj p L [Event.write s₂ cs₂] := by
          by_cases ha : Vis p L (.ofLabel p.acceptsWrite)
          · rw [hpath (hacc ha).1, hcont (hacc ha).2]
          · rw [proj_single_hidden (ev := .write s₁ cs₁) ha, proj_single_hidden (ev := .write s₂ cs₂) ha]
        by_cases hi : Vis p L (.ofLabel internalLabel)
        · have hweq := hw.2.1 hi
          subst hweq
          have ha := hacc (Vis.of_le hh.write hi)
          have hs := hpath ha.1
          have hcs := hcont ha.2
          subst hs hcs
          refine brel_same (WorldRel.refl _) (fun v hv' => ?_)
          split at hv' <;> simp at hv'
          subst hv'
          exact .unit
        · intro ht₁ ht₂
          by_cases hfv : Vis p L (.ofLabel p.fileState)
          · have hs := hpath (hstate hfv)
            subst hs
            have hlaw' := hlaw.write_shape s₁ cs₁ cs₂ (hw.1 hfv)
            cases hr₁ : H.write s₁ cs₁ w₁.fs <;> cases hr₂ : H.write s₁ cs₂ w₂.fs <;>
              simp only [hr₁, hr₂, Option.map_none, Option.map_some, reduceCtorEq,
                Option.some.injEq] at hlaw' ht₁ ht₂ ⊢
            · exact ⟨hev, fun _ => rfl, hw⟩
            · exact ⟨hev, .unit, fun _ => hlaw', fun h => absurd h hi, hw.2.2⟩
          · have hnvf : ¬ Vis p L ((p.fsFail (pc.join (p.deep (.text l)))).inst (p.sd σ)) :=
              fun h => hfv (vis_fsFail hh h).2
            have hw' : ∀ {f₁ f₂ : H.FS}, WorldRel p L { w₁ with fs := f₁ } { w₂ with fs := f₂ } :=
              ⟨fun h => absurd h hfv, fun h => absurd h hi, hw.2.2⟩
            cases hr₁ : H.write s₁ cs₁ w₁.fs <;> cases hr₂ : H.write s₂ cs₂ w₂.fs <;>
              simp only [hr₁, hr₂] at ht₁ ht₂ ⊢ <;> refine ⟨hev, ?_, hw'⟩ <;> simp only [OutRel]
            all_goals first
              | exact fun _ => rfl
              | trivial
              | exact .unit
              | exact fun h => absurd (by simpa [Eff.inst, Eff.failLabel] using h) hnvf
              | exact fun _ => trivial
      | @req _ _ m args ts has hts hrec hle _ =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, hty₁, hty₂⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂]
        have hargs : Vis p L (.ofLabel p.acceptsReq) → vs₁ = vs₂ := fun ha => by
          refine VList.eq_http hwf hrec hvs hty₁ hty₂ (fun t ht => ?_)
          obtain ⟨u, hu, rfl⟩ := List.mem_map.mp ht
          have hshape := httpParams_args hts _ (List.mem_map_of_mem hu)
          refine ⟨by rw [LTy.erase_lsubst, hshape.subst]; exact hshape, ?_⟩
          refine Vis.of_le (LTy.deep_lsub_le hσ hE (latoms_inPlace hΓ has u hu)
            (httpArg_eqIn hrec hshape)) ?_
          refine Vis.of_le (LExpr.le_inst _ ?_) (Vis.of_le (LExpr.inst_le_join_inst _ _ _) (vis_of_le_accepts hle ha))
          exact LExpr.le_joinAll (List.mem_map.mpr ⟨_, hu, rfl⟩)
        have hev : proj p L [Event.req m vs₁] = proj p L [Event.req m vs₂] := by
          by_cases ha : Vis p L (.ofLabel p.acceptsReq)
          · rw [hargs ha]
          · rw [proj_single_hidden (ev := .req m vs₁) ha, proj_single_hidden (ev := .req m vs₂) ha]
        by_cases hi : Vis p L (.ofLabel internalLabel)
        · have hweq := hw.2.1 hi
          subst hweq
          have hv := hargs (Vis.of_le hh.req hi)
          subst hv
          refine brel_same (WorldRel.refl _) (fun v hv' => ?_)
          generalize hq : H.request m vs₁ w₁.srv = q at hv'
          obtain ⟨o, srv'⟩ := q
          cases o <;> simp at hv'
          subst hv'
          simpa [LTy.lsubst] using V.response_refl hrec (hlaw.resp_typed m vs₁ w₁.srv srv' _ hq) _
        · have hres : ¬ Vis p L ((LExpr.ofLabel p.reqResult).inst (p.sd σ)) := fun h =>
            hi (Vis.of_le hh.reqResult (vis_ofLabel_of_inst h))
          have hnvf : ¬ Vis p L ((p.reqFail pc ts).inst (p.sd σ)) := fun h => hi (vis_reqFail hh h)
          intro ht₁ ht₂
          rcases hq₁ : H.request m vs₁ w₁.srv with ⟨o₁, srv₁⟩
          rcases hq₂ : H.request m vs₂ w₂.srv with ⟨o₂, srv₂⟩
          have hw' : WorldRel p L { w₁ with srv := srv₁ } { w₂ with srv := srv₂ } :=
            ⟨fun h => hw.1 h, fun h => absurd h hi, hw.2.2⟩
          cases o₁ <;> cases o₂ <;> simp only [hq₁, hq₂] at ht₁ ht₂ ⊢ <;> refine ⟨hev, ?_, hw'⟩ <;>
            simp only [OutRel]
          all_goals first
            | exact fun _ => rfl
            | trivial
            | exact .hidden hres
            | exact fun h => absurd (by simpa [Eff.inst, Eff.failLabel] using h) hnvf
            | exact fun _ => trivial
      | construct hrec has hfl =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.record hrec (VList.ground_lsubst (hwf.recordsGround _ (record_mem hrec))
          (VList.flowsσ hvs hfl)))
      | variant hs hf has hfl =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.variant hs hf (VList.ground_lsubst
          (fun u hu => (hwf.sums _ (sum_mem hs) _ (List.mem_of_getElem? hf) u hu).1)
          (VList.flowsσ hvs hfl)))
      | @field _ _ value record index rec l t hv hrec hi =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hvv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hv
        have hli := latom_inPlace hΓ hv
        simp only [LTy.InPlace] at hli
        simp only [LTy.lsubst] at hvv hu₁ hu₂
        cases hu₁ with
        | variant hs _ _ => exact (not_sum_of_record hwf hrec hs).elim
        | record hr₁ hfs₁ =>
          cases hu₂ with
          | variant hs _ _ => exact (not_sum_of_record hwf hrec hs).elim
          | record hr₂ hfs₂ =>
            rw [hrec] at hr₁ hr₂
            cases hr₁
            cases hr₂
            obtain ⟨a, ha, _⟩ := lfield_typed hfs₁ hi
            obtain ⟨b, hb', _⟩ := lfield_typed hfs₂ hi
            simp only [evalRhs, h₁, h₂, ha, hb', Option.elim]
            refine brel_done hw ?_
            cases hvv with
            | hidden h => exact V.hideσ hli _ _ h
            | record hr hrel' =>
              rw [hrec] at hr
              cases hr
              obtain ⟨a', b', ha', hb'', hab⟩ := VList.get hrel' hi
              rw [ha] at ha'
              rw [hb'] at hb''
              cases ha'
              cases hb''
              have hg := hwf.recordsGround _ (record_mem hrec) _ (List.mem_of_getElem? hi)
              exact V.raiseσ _ ((V.ground_lsubst hg).mpr hab)
      | list has hfl _ =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.list (VAll.of_flows hvs (fun u hu => by
          obtain ⟨u', hu', rfl⟩ := List.mem_map.mp hu
          exact Flow.lsubst _ _ (hfl u' hu'))))
      | none _ => exact .of_brel (brel_done hw .none)
      | some ha =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.some hv)
      | mkOk ha _ =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.ok hv)
      | mkErr ha _ =>
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst]
        exact brel_done hw (.err hv)
      | @lambda _ _ params body tb εl hb hfl hrl hrs hps =>
        refine BRelR.of_brel ?_
        have hΓ' : LCtx.InPlace (params ++ Γ) := fun q hq => by
          rcases List.mem_append.mp hq with hq | hq
          · exact hps q hq
          · exact hΓ q hq
        refine brel_done hw ?_
        simp only [LTy.lsubst]
        exact .closure hσ hE hΓ' hrel he₁ he₂ hb hfl hrl hrs (by rw [LTy.lsubsts_eq]; exact Flows.refl _)
          (Flow.refl _) id (HostFx.Le.refl _) id
      | @funRef _ _ f targs fn hf hnv hlen _ heqa hg hfr hfr2 =>
        refine BRelR.of_brel ?_
        obtain ⟨tb, εb, hbody, hflow, ho, hgo, hfs, hfail, hrs, _, hra⟩ := funOk_of_nvia hwf hnv hf
        have hrl : εb.rels = [] :=
          List.eq_nil_iff_forall_not_mem.mpr (fun x hx => by simpa [hfr] using hrs x hx)
        have hrsn : εb.raises = [] :=
          List.eq_nil_iff_forall_not_mem.mpr (fun x hx => by simpa [hfr2] using hra x hx)
        have hτ : GroundSubst (fn.tparams.zip targs) := zip_ground hg
        have hcov := lookup_zip_cover (xs := fn.tparams) hlen
        have hEτ := eqArgsOK_of_call hwf hf hlen hg heqa
        obtain ⟨hps, hres⟩ := hwf.sigsScoped fn (find_mem hf)
        have hΓf : LCtx.InPlace (fn.params ++ []) := by
          rw [List.append_nil]; exact fun q hq => (hwf.inPlace.1 fn (find_mem hf)).1 q hq
        simp only [evalRhs, hf]
        refine brel_done hw ?_
        simp only [LTy.lsubst, Program.lsub]
        refine .closure (Γ := []) (E := fn.eqParams) hτ hEτ hΓf .nil .nil .nil (by simpa using hbody)
          hfail hrl hrsn ?_ ?_ ho hgo hfs
        · rw [LTy.lsubsts_eq]
          have : ∀ qs : List (String × LTy), (∀ q ∈ qs, q.2.ScopedIn fn.tparams) →
              Flows p.top ((qs.map (fun q => q.2.lsubst p.recordDeep (fn.tparams.zip targs))).map
                (LTy.lsubst p.recordDeep σ))
                ((qs.map Prod.snd).map (LTy.lsubst p.recordDeep (fn.tparams.zip targs))) := by
            intro qs hqs
            induction qs with
            | nil => exact .nil
            | cons q qs ih =>
              exact .cons (Flow.lsubst_ground _ σ
                  (LTy.lsubst_ground _ hτ hcov (hqs q List.mem_cons_self))).1
                (ih (fun q' hq' => hqs q' (List.mem_cons_of_mem _ hq')))
          exact this fn.params hps
        · exact Flow.trans (Flow.lsubst _ _ hflow)
            (Flow.lsubst_ground _ σ (LTy.lsubst_ground _ hτ hcov hres)).2
      | @declassify _ _ pol a po fn t hpo hfn ha hflow hle _ =>
        obtain ⟨v₁, v₂, h₁, h₂, _, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel ha
        have hmem := policy_mem hpo
        obtain ⟨_, _, fn', hfn', hps, hres⟩ := hwf.wellFormed.policies _ hmem
        rw [hfn] at hfn'
        cases hfn'
        obtain ⟨fnv, hfnv, _, _, hpof, hpure⟩ := hb1.via po hmem
        rw [hfn] at hfnv
        cases hfnv
        obtain ⟨hdg, hds⟩ := hb1.dst po hmem
        have hsrc : (t.lsubst p.recordDeep σ).erase = po.src.erase := by
          rw [LTy.erase_lsubst, Flow.erase hflow, Ty.Scalar.subst _ (hb1.src po hmem)]
        have hlen := paramTys_length fn []
        rw [hps] at hlen
        have hds' : (po.dst.lsubst p.recordDeep σ).erase = po.dst.erase := by
          rw [LTy.erase_lsubst, Ty.Scalar.subst _ hds]
        -- Each run of `via`: no event, a failure only if it declares `fails`, and a value of the
        -- scalar `to`.
        have hvia : ∀ {w : World H} {v : Val}, LValTy p v (t.lsubst p.recordDeep σ) →
            Silent w (eval H n p w (bindParams fn.params [v] []) fn.body) ∧
            (fn.fails = false → ∀ c, (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 ≠ .failed c) ∧
            (∀ r, (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 = .ok r →
              ValTy p r po.dst.erase) ∧
            ∀ k c ps, (eval H n p w (bindParams fn.params [v] []) fn.body).2.1 ≠ .raised k c ps := by
          intro w v hvt
          have hvs : ValsTy p [v] (fn.paramTys []) := by
            rw [hps, ← hsrc]
            exact .cons (valTy_of_scalar (by rw [hsrc]; exact hb1.src po hmem) hvt) .nil
          have henv' := bindParams_typed hvs (paramTys_length fn []) (EnvTy.nil (p := p))
          rw [List.append_nil] at henv'
          have hbody := body_typed hwf.wellFormed hfn []
          rw [Ty.subst_nil, hres] at hbody
          refine ⟨(silent_run p w n).2 _ _ hpof,
            fun hf => ((pure_run p w n).2 _ _ (hpure.1 hf)).2.1, fun r hr => ?_,
            (noRaise_run p w n).2 _ _ hpof⟩
          have hg := (sound hwf.wellFormed hlaw.resp_typed n).2 (w := w) henv' hbody
          rw [hr] at hg
          exact hg
        have q₁ := hvia (w := w₁) hu₁
        have q₂ := hvia (w := w₂) hu₂
        simp only [evalRhs, hpo, h₁, h₂, hfn]
        have hl1 : 1 = fn.params.length := by simpa using hlen
        rw [if_pos hl1, if_pos hl1]
        rcases e₁r : eval H n p w₁ (bindParams fn.params [v₁] []) fn.body with ⟨t₁, o₁, w₁'⟩
        rcases e₂r : eval H n p w₂ (bindParams fn.params [v₂] []) fn.body with ⟨t₂, o₂, w₂'⟩
        rw [e₁r] at q₁
        rw [e₂r] at q₂
        obtain ⟨⟨htr₁, hw₁⟩, hnf₁, hok₁, hnr₁⟩ := q₁
        obtain ⟨⟨htr₂, hw₂⟩, hnf₂, hok₂, hnr₂⟩ := q₂
        simp only at htr₁ htr₂ hnf₁ hnf₂ hok₁ hok₂ hw₁ hw₂
        subst htr₁ htr₂ hw₁ hw₂
        have hev : ∀ r, EvVis p L (Event.release pol r) ↔ Vis p L po.dst.top := fun r =>
          ⟨fun ⟨po', hpo', hv⟩ => by rw [hpo] at hpo'; cases hpo'; exact hv, fun hv => ⟨po, hpo, hv⟩⟩
        -- The failure is seen only where the release is (`pc ⊔ ℓ(to)`).
        have hvf : fn.fails = true →
            Vis p L ((Eff.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol).inst (p.sd σ)).failLabel →
            Vis p L po.dst.top := fun hf h => by
          simp only [Eff.ofPolicy, Eff.inst, Eff.failLabel, hf, if_true, Option.map_some,
            Option.getD_some] at h
          exact Vis.of_le (LExpr.le_trans (LExpr.le_inst_symFree _ hdg.top_symFree).2
            (LExpr.le_inst _ (LExpr.le_join_right _ _))) h
        -- One release each: equal when seen, or a divergence.
        have hone : ∀ ρ₁ ρ₂ : Val, ∀ o₁ o₂ : Res,
            (Vis p L po.dst.top → ρ₁ = ρ₂ → OutRel p L (V p L (po.dst.lsubst p.recordDeep σ))
              (Vis p L ((Eff.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol).inst (p.sd σ)).failLabel)
              o₁ o₂) →
            (¬ Vis p L po.dst.top → OutRel p L (V p L (po.dst.lsubst p.recordDeep σ))
              (Vis p L ((Eff.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol).inst (p.sd σ)).failLabel)
              o₁ o₂) →
            (proj p L [Event.release pol ρ₁] = proj p L [Event.release pol ρ₂] ∧
              OutRel p L (V p L (po.dst.lsubst p.recordDeep σ))
                (Vis p L ((Eff.ofPolicy fn.op fn.host fn.fails pc po.dst.top pol).inst (p.sd σ)).failLabel)
                o₁ o₂ ∧ WorldRel p L w₁' w₂') ∨
            Diverged p L [Event.release pol ρ₁] [Event.release pol ρ₂] := by
          intro ρ₁ ρ₂ o₁ o₂ hsee hhid
          by_cases hvd : Vis p L po.dst.top
          · by_cases heq : ρ₁ = ρ₂
            · subst heq
              exact .inl ⟨rfl, hsee hvd rfl, hw⟩
            · exact .inr ⟨[], pol, ρ₁, ρ₂, [], [], heq, by simp [proj_single_vis ((hev ρ₁).mpr hvd)],
                by simp [proj_single_vis ((hev ρ₂).mpr hvd)]⟩
          · rw [proj_single_hidden (fun h => hvd ((hev ρ₁).mp h)),
              proj_single_hidden (fun h => hvd ((hev ρ₂).mp h))]
            exact .inl ⟨rfl, hhid hvd, hw⟩
        intro ht₁ ht₂
        cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;> simp only [List.nil_append]
        -- A `via` body never fails with a payload (#512).
        all_goals (try (first | exact absurd rfl (hnr₁ _ _ _) | exact absurd rfl (hnr₂ _ _ _)))
        · -- Both values.
          rename_i r₁ r₂
          have hr₁ := hok₁ r₁ rfl
          refine hone r₁ r₂ (.ok r₁) (.ok r₂) (fun _ heq => ?_) (fun hvd => ?_)
          · subst heq
            exact V.refl_scalar (by rw [hds']; exact hds) (by rw [hds']; exact hr₁)
          · refine .hidden ?_
            rw [top_lsubst_of_ground hdg]
            exact fun h => hvd (Vis.of_le (LExpr.le_join_right _ _) h)
        · -- A value against a failure: their releases differ.
          rename_i r₁ c₂
          have hr₁ := hok₁ r₁ rfl
          refine hone r₁ (.err (.int c₂)) (.ok r₁) (.failed c₂) (fun _ heq => ?_) (fun hvd hv => ?_)
          · exact absurd heq (release_ok_not_err hds hr₁ (.int c₂))
          · cases hf : fn.fails with
            | false => exact hnf₂ hf c₂ rfl
            | true => exact hvd (hvf hf hv)
        · rename_i c₁ r₂
          have hr₂ := hok₂ r₂ rfl
          refine hone (.err (.int c₁)) r₂ (.failed c₁) (.ok r₂) (fun _ heq => ?_) (fun hvd hv => ?_)
          · exact absurd heq.symm (release_ok_not_err hds hr₂ (.int c₁))
          · cases hf : fn.fails with
            | false => exact hnf₁ hf c₁ rfl
            | true => exact hvd (hvf hf hv)
        · -- Two failures: the same code when their releases agree.
          rename_i c₁ c₂
          refine hone (.err (.int c₁)) (.err (.int c₂)) (.failed c₁) (.failed c₂)
            (fun _ heq _ => by
              simp only [Val.err.injEq, Val.int.injEq] at heq
              obtain rfl : c₁ = c₂ := by omega
              rfl)
            (fun hvd hv => ?_)
          cases hf : fn.fails with
          | false => exact absurd rfl (hnf₁ hf c₁)
          | true => exact absurd (hvf hf hv) hvd
      | @map _ _ xs f te lx tp tr o g fl lf hxs hf hte hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hf
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂, LTy.lsubst]
            by_cases hvis : Vis p L ((lx.join lf).inst (p.sd σ))
            · have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvis
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkMap_brel (fun w₁ w₂ x y hw' hxy hx hy => applyWith_brel hrunB hw' hfv hvf
                    (.cons (V.flow hxy hte') .nil) (.cons (LValTy.flow hx hte') .nil)
                    (.cons (LValTy.flow hy hte') .nil) True) hw hall hi₁ hi₂)
                obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := h
                exact .list (VAll.flow hz (Flow.lsubst _ _ (Flow.raise _ _)))
            · have hpc' : ¬ Vis p L (((pc.join lx).join lf).inst (p.sd σ)) := fun h =>
                hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xl pc lx lf)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  (.list ((tr.raise lf).lsubst p.recordDeep σ) ((lx.join lf).inst (p.sd σ)))) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  (.list ((tr.raise lf).lsubst p.recordDeep σ) ((lx.join lf).inst (p.sd σ)))) (hb.inst _) ?_)
                hpc' (fun a b => .hidden hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact walkMap_uok _ (fun w v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) (FailCond.Le.refl _)
                  (applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte') .nil) w)) w₁ hi₁
              · exact walkMap_uok _ (fun w v hv => UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) (FailCond.Le.refl _)
                  (applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte') .nil) w)) w₂ hi₂
      | @filter _ _ xs keep te lx tp lb o g fl lf hxs hk hte hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂, LTy.lsubst]
            by_cases hvis : Vis p L ((lx.join lf).inst (p.sd σ))
            · have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvis
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkFilter_brel (fun w₁ w₂ x y hw' hxy hx hy => applyWith_brel hrunB hw' hfv hvf
                    (.cons (V.flow hxy hte') .nil) (.cons (LValTy.flow hx hte') .nil)
                    (.cons (LValTy.flow hy hte') .nil) True) hw hall hi₁ hi₂)
                by_cases hl : Vis p L (((lx.join lf).join lb).inst (p.sd σ))
                · obtain ⟨zs₁, zs₂, rfl, rfl, hz⟩ := h (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hl)
                  exact .list hz
                · exact .hidden hl
            · have hpc' : ¬ Vis p L (((pc.join lx).join lf).inst (p.sd σ)) := fun h =>
                hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xl pc lx lf)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  (.list (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  (.list (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                hpc' (fun a b => .hidden (fun h => hvis (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) h)))
                (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact walkFilter_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte') .nil) w) w₁ hi₁
              · exact walkFilter_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte') .nil) w) w₂ hi₂
      | @firstWhere _ _ xs keep te lx tp lb o g fl lf hxs hk hte hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hk
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        cases hxu₁ with
        | list hi₁ =>
          cases hxu₂ with
          | list hi₂ =>
            simp only [evalRhs, hx₁, hx₂, hf₁, hf₂, LTy.lsubst]
            by_cases hvis : Vis p L (((lx.join lf).join lb).inst (p.sd σ))
            · have hvxf := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ lb)) hvis
              have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvxf
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvxf
              have hvb := Vis.of_le (LExpr.inst_le_join_inst _ _ lb) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => ?_) (fun _ => trivial)
                  (walkFirst_brel hvb (fun w₁ w₂ x y hw' hxy hx hy => applyWith_brel hrunB hw' hfv hvf
                    (.cons (V.flow hxy hte') .nil) (.cons (LValTy.flow hx hte') .nil)
                    (.cons (LValTy.flow hy hte') .nil) True) hw hall hi₁ hi₂)
                rcases h with ⟨rfl, rfl⟩ | ⟨x, y, rfl, rfl, hxy⟩
                · exact .none
                · exact .some hxy
            · have hpc' : ¬ Vis p L (((((pc.join lx).join lf).join (p.deep te)).join lb).inst (p.sd σ)) :=
                fun h => hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xlb pc lx lf _ lb)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  (.option (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  (.option (te.lsubst p.recordDeep σ) (((lx.join lf).join lb).inst (p.sd σ)))) (hb.inst _) ?_)
                hpc' (fun a b => .hidden hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact walkFirst_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₁ (.cons (LValTy.flow hv hte') .nil) w) w₁ hi₁
              · exact walkFirst_uok _ (fun w v hv =>
                  applyWith_uok hrunU hfu₂ (.cons (LValTy.flow hv hte') .nil) w) w₂ hi₂
      | @fold _ _ xs init f te lx ti ta tp tr o g fl lf hxs hi hf hte hinit hres hb =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨i₁, i₂, hi₁', hi₂', hiv, hiu₁, hiu₂⟩ := latom_bothσ he₁ he₂ hrel hi
        obtain ⟨f₁, f₂, hf₁, hf₂, hfv, hfu₁, hfu₂⟩ := latom_bothσ he₁ he₂ hrel hf
        have hxi := latom_inPlace hΓ hxs
        have hfi := latom_inPlace hΓ hf
        simp only [LTy.InPlace] at hxi hfi
        have hdp : (lx.join lf).PlainFree := LExpr.plainFree_join hxi.2 hfi.2.2
        simp only [LTy.lsubst, LTy.lsubsts] at hxv hxu₁ hxu₂ hfv hfu₁ hfu₂
        have hte' := Flow.lsubst p.recordDeep σ hte
        have hinit' := Flow.lsubst p.recordDeep σ hinit
        have hres' := Flow.lsubst p.recordDeep σ hres
        cases hxu₁ with
        | list hl₁ =>
          cases hxu₂ with
          | list hl₂ =>
            simp only [evalRhs, hx₁, hx₂, hi₁', hi₂', hf₁, hf₂]
            have happU : ∀ (fv : Val), LValTy p fv (.fn [ta.lsubst p.recordDeep σ, tp.lsubst p.recordDeep σ]
                (tr.lsubst p.recordDeep σ) o g fl (lf.inst (p.sd σ))) →
                ∀ w c x, LValTy p c (ta.lsubst p.recordDeep σ) → LValTy p x (te.lsubst p.recordDeep σ) →
                  UOk (FnAllowed o g) (fun r => LValTy p r (ta.lsubst p.recordDeep σ))
                    ⟨fl = true, fun _ _ _ => False⟩ w
                    (applyWith (eval H n p) w fv [c, x]) := fun fv hfu w c x hc hx =>
              UOk.mono (fun _ h => h) (fun _ h => LValTy.flow h hres') (FailCond.Le.refl _)
                (applyWith_uok hrunU hfu (.cons hc (.cons (LValTy.flow hx hte') .nil)) w)
            by_cases hvis : Vis p L ((lx.join lf).inst (p.sd σ))
            · have hvx := Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left lx lf)) hvis
              have hvf := Vis.of_le (LExpr.inst_le_join_inst _ lx lf) hvis
              cases hxv with
              | hidden h => exact absurd hvx h
              | list hall =>
                refine BRel.mono (fun a b h => V.raiseσ _ h) (fun _ => trivial)
                  (walkFold_brel (fun w₁ w₂ c₁ c₂ x y hw' hc hc₁ hc₂ hxy hx hy =>
                    BRel.mono (fun _ _ h => V.flow h hres') id
                      (applyWith_brel hrunB hw' hfv hvf (.cons hc (.cons (V.flow hxy hte') .nil))
                        (.cons hc₁ (.cons (LValTy.flow hx hte') .nil))
                        (.cons hc₂ (.cons (LValTy.flow hy hte') .nil)) True))
                    (happU _ hfu₁) (happU _ hfu₂) hw (V.flow hiv hinit') (LValTy.flow hiu₁ hinit')
                    (LValTy.flow hiu₂ hinit') hall hl₁ hl₂)
            · have hpc' : ¬ Vis p L (((pc.join lx).join lf).inst (p.sd σ)) := fun h =>
                hvis (Vis.of_le (LExpr.le_inst _ (le_pc_xl pc lx lf)) h)
              refine brel_confined hh hw (hof_uok (P := fun v => LValTy p v
                  ((ta.raise (lx.join lf)).lsubst p.recordDeep σ)) (hb.inst _) ?_)
                (hof_uok (P := fun v => LValTy p v
                  ((ta.raise (lx.join lf)).lsubst p.recordDeep σ)) (hb.inst _) ?_)
                hpc' (fun a b => V.hideσ hdp a b hvis) (fun _ => Vis.ofFn_plain) (fun _ => Vis.ofFn_plain)
              · exact UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) (FailCond.Le.refl _)
                  (walkFold_uok (happU _ hfu₁) w₁ (LValTy.flow hiu₁ hinit') hl₁)
              · exact UOk.mono (fun _ h => h) (fun _ h => LValTy.lsubst_raise h) (FailCond.Le.refl _)
                  (walkFold_uok (happU _ hfu₂) w₂ (LValTy.flow hiu₂ hinit') hl₂)
      | @parse _ _ s₀ t hs ht =>
        refine BRelR.of_brel ?_
        cases t <;> simp [LTy.erase] at ht
        rename_i l
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        simp only [LTy.lsubst] at hv hu₁ hu₂
        cases hu₁
        cases hu₂
        rename_i s₁ s₂
        simp only [evalRhs, h₁, h₂, LTy.lsubst]
        by_cases hd : Vis p L (l.inst (p.sd σ))
        · have hs' : s₁ = s₂ := by have := V.eq_text hv hd; injection this
          subst hs'
          unfold parseInt
          split
          · exact brel_done hw .int
          · exact brel_same hw (fun _ h => nomatch h)
        · have hvf : ¬ Vis p L ((⟨false, HostFx.none, some (pc.join (p.deep (.text l))), [], true, []⟩ : Eff).inst (p.sd σ)).failLabel :=
            fun h => hd (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h)
          refine brel_quiet ?_ ?_ (fun _ _ => .hidden hd) (fun _ => hvf) (fun _ => hvf)
          · unfold parseInt
            split <;> split <;> exact hw
          · unfold parseInt
            split <;> split <;> rfl
      | @checked _ _ a t pred code ha hs hle hsf =>
        -- T-Checked (#453): the operand is public, so the two runs check the same value and take
        -- the same branch (`brel_checked`).
        refine BRelR.of_brel ?_
        obtain ⟨v₁, v₂, h₁, h₂, hv, hu₁, hu₂⟩ := latom_bothσ he₁ he₂ hrel ha
        have hvis : Vis p L ((p.deep t).inst (p.sd σ)) :=
          Vis.of_le (LExpr.inst_le_join_inst _ _ _)
            (Vis.of_le (LExpr.le_of_le_inst_ofLabel (LExpr.le_inst _ hle)) Vis.bot)
        simp only [evalRhs, h₁, h₂]
        cases t with
        | int l =>
          simp only [LTy.lsubst] at hv hvis ⊢
          cases hv with
          | hidden hh => exact absurd hvis hh
          | int => exact brel_checked hw .int
        | bool l =>
          simp only [LTy.lsubst] at hv hvis ⊢
          cases hv with
          | hidden hh => exact absurd hvis hh
          | bool => exact brel_checked hw .bool
        | text l =>
          simp only [LTy.lsubst] at hv hvis ⊢
          cases hv with
          | hidden hh => exact absurd hvis hh
          | text => exact brel_checked hw .text
        | _ => simp [LTy.erase, Ty.Scalar] at hs
      | @eq _ _ a b t₁ t₂ tt ha hb hfa hfb heq =>
        refine BRelR.of_brel ?_
        obtain ⟨a₁, a₂, ha₁, ha₂, hav, hau₁, hau₂⟩ := latom_bothσ he₁ he₂ hrel ha
        obtain ⟨b₁, b₂, hb₁, hb₂, hbv, hbu₁, hbu₂⟩ := latom_bothσ he₁ he₂ hrel hb
        simp only [evalRhs, ha₁, ha₂, hb₁, hb₂, LTy.lsubst]
        refine brel_done hw ?_
        by_cases hd : Vis p L (((p.deep t₁).join (p.deep t₂)).inst (p.sd σ))
        · have he₁' : t₁.eqIn E (p.recordEq (p.records.length + 1)) = true := by
            rw [eqIn_flow hfa]; exact heq
          have he₂' : t₂.eqIn E (p.recordEq (p.records.length + 1)) = true := by
            rw [eqIn_flow hfb]; exact heq
          have hd₁ : Vis p L (p.deep (t₁.lsubst p.recordDeep σ)) :=
            Vis.of_le (LTy.deep_lsub_le hσ hE (latom_inPlace hΓ ha) he₁')
              (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) hd)
          have hd₂ : Vis p L (p.deep (t₂.lsubst p.recordDeep σ)) :=
            Vis.of_le (LTy.deep_lsub_le hσ hE (latom_inPlace hΓ hb) he₂')
              (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hd)
          rw [V.eq_of_deep hwf hav hau₁ hau₂ (LTy.eqIn_lsub hE he₁') hd₁,
            V.eq_of_deep hwf hbv hbu₁ hbu₂ (LTy.eqIn_lsub hE he₂') hd₂]
          exact .bool
        · exact .hidden hd
      | @toText _ _ a t ha heq =>
        refine BRelR.of_brel ?_
        obtain ⟨a₁, a₂, ha₁, ha₂, hav, hau₁, hau₂⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [evalRhs, ha₁, ha₂]
        refine brel_done hw ?_
        simp only [LTy.lsubst]
        by_cases hd : Vis p L ((p.deep t).inst (p.sd σ))
        · have hd₁ : Vis p L (p.deep (t.lsubst p.recordDeep σ)) :=
            Vis.of_le (LTy.deep_lsub_le hσ hE (latom_inPlace hΓ ha) heq) hd
          rw [V.eq_of_deep hwf hav hau₁ hau₂ (LTy.eqIn_lsub hE heq) hd₁]
          exact .text
        · exact .hidden hd
      | @envGet _ _ name t ha ht hle =>
        refine BRelR.of_brel ?_
        obtain ⟨n₁, n₂, hn₁, hn₂, hnv, hnu₁, hnu₂⟩ := latom_bothσ he₁ he₂ hrel ha
        cases t <;> simp [LTy.erase] at ht
        rename_i l
        simp only [LTy.lsubst] at hnv hnu₁ hnu₂
        have hvl : Vis p L (l.inst (p.sd σ)) := Vis.of_le (LExpr.le_inst _ hle) (vis_bot_inst _)
        have hnn := V.eq_text hnv hvl
        subst hnn
        cases hnu₁ with
        | @text s _ =>
          simp only [evalRhs, hn₁, hn₂]
          refine brel_done hw ?_
          simp only [LTy.lsubst]
          have hex := hw.2.2 s
          cases h₁ : w₁.env s <;> cases h₂ : w₂.env s <;>
            simp only [h₁, h₂, Option.isSome, Bool.false_eq_true, Bool.true_eq_false] at hex ⊢
          · exact .none
          · -- The variable exists in both runs (`WorldRel`'s third conjunct, #487): its values
            -- are equal when the observer sees `internal`, and hidden otherwise.
            by_cases hv : Vis p L (.ofLabel internalLabel)
            · have hweq := hw.2.1 hv
              subst hweq
              rw [h₁] at h₂
              cases h₂
              exact .some .text
            · exact .some (.hidden fun h => hv (vis_ofLabel_of_inst h))
      | tuple has =>
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [evalRhs, h₁, h₂, Option.map, Option.elim, LTy.lsubst, LTy.lsubsts_eq]
        exact brel_done hw (.tuple hvs)
      | append hxs hi hflow =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨i₁, i₂, hi₁, hi₂, hiv, _, _⟩ := latom_bothσ he₁ he₂ hrel hi
        simp only [LTy.lsubst] at hxv hxu₁ hxu₂
        cases hxu₁ with
        | list _ =>
          cases hxu₂ with
          | list _ =>
            simp only [evalRhs, hx₁, hx₂, hi₁, hi₂]
            simp only [LTy.lsubst]
            cases hxv with
            | hidden h => exact brel_done hw (.hidden h)
            | list hall =>
              exact brel_done hw (.list (VAll.append_one hall (V.flow hiv (Flow.lsubst _ _ hflow))))
      | set hxs hi hle hv hflow =>
        refine BRelR.of_brel ?_
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨i₁, i₂, hi₁, hi₂, hiv, hiu₁, hiu₂⟩ := latom_bothσ he₁ he₂ hrel hi
        obtain ⟨v₁, v₂, hv₁, hv₂, hvv, _, _⟩ := latom_bothσ he₁ he₂ hrel hv
        simp only [LTy.lsubst] at hxv hxu₁ hxu₂ hiv hiu₁ hiu₂
        have hvi : Vis p L (_ : LExpr) := Vis.of_le (LExpr.le_inst _ hle) (vis_bot_inst (p.sd σ))
        have hii := V.eq_int hiv hvi
        subst hii
        cases hxu₁ with
        | list _ =>
          cases hxu₂ with
          | list _ =>
            cases hiu₁ with
            | int =>
              simp only [evalRhs, hx₁, hx₂, hi₁, hi₂, hv₁, hv₂]
              have hvv' := V.flow hvv (Flow.lsubst _ _ hflow)
              split
              · split
                · simp only [LTy.lsubst, LTy.lsubsts]
                  refine brel_done hw (.tuple (.cons ?_ (.cons .unit .nil)))
                  cases hxv with
                  | hidden h => exact .hidden h
                  | list hall => exact .list (VAll.set _ hall hvv')
                · exact brel_abort_right
              · exact brel_abort_left
    · intro σ w₁ w₂ e₁ e₂ Γ E pc e t ε hσ hE hΓ hpc hw he₁ he₂ hrel he
      have hcons : ∀ {x : String} {u : LTy}, u.InPlace → LCtx.InPlace ((x, u) :: Γ) :=
        fun hu q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · exact hu
          · exact hΓ q hq
      cases he with
      | atom ha =>
        obtain ⟨v₁, v₂, h₁, h₂, hv, _, _⟩ := latom_bothσ he₁ he₂ hrel ha
        simp only [eval, h₁, h₂, Option.elim]
        exact .of_brel (brel_done hw hv)
      | @letR _ _ x ann value body ty s ε₁ ε₂ hr hflow hb hann =>
        simp only [eval]
        have u₁ := uR (w := w₁) hσ he₁ hr
        have u₂ := uR (w := w₂) hσ he₂ hr
        have hε₁ := (LRhsTy.inPlace hwf hΓ hpc hr).2
        have hpc' : (pc.join ε₁.failLabel).PlainFree :=
          LExpr.plainFree_join hpc (Eff.failLabel_plainFree hε₁)
        have hflow' := Flow.lsubst p.recordDeep σ hflow
        have hnv : ¬ Vis p L (ε₁.inst (p.sd σ)).failLabel →
            ¬ Vis p L ((pc.join ε₁.failLabel).inst (p.sd σ)) := fun hn h =>
          hn (Vis.failLabel_inst.mpr (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
        refine andThen_brelR (ihR hσ hE hΓ hpc hw he₁ he₂ hrel hr) Vis.union_left_inst
          (fun v₁ v₂ k₁ k₂ hv hw' => ?_) ?_ ?_
        · have hv₁ : LValTy p v₁ (ty.lsubst p.recordDeep σ) := by
            have := u₁.2.1; rw [k₁] at this; exact this
          have hv₂ : LValTy p v₂ (ty.lsubst p.recordDeep σ) := by
            have := u₂.2.1; rw [k₂] at this; exact this
          exact BRelR.mono (fun _ _ h => h) Vis.union_right_inst
            (ihE hσ hE (hcons hann) hpc' hw' (.cons (LValTy.flow hv₁ hflow') he₁)
              (.cons (LValTy.flow hv₂ hflow') he₂) (.cons (V.flow hv hflow') hrel) hb)
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₁.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (evalRhs H n p w₁ e₁ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₁) hb) (hnv hn)
          exact ⟨hp, fun hc hvf => hf hc (Vis.union_right_inst hvf), hwc⟩
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₂.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (evalRhs H n p w₂ e₂ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₂) hb) (hnv hn)
          exact ⟨hp, fun hc hvf => hf hc (Vis.union_right_inst hvf), hwc⟩
      | @letE _ _ x ann value body ty s ε₁ ε₂ hr hflow hb hann =>
        simp only [eval]
        have u₁ := uE (w := w₁) hσ he₁ hr
        have u₂ := uE (w := w₂) hσ he₂ hr
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hr).2
        have hpc' : (pc.join ε₁.failLabel).PlainFree :=
          LExpr.plainFree_join hpc (Eff.failLabel_plainFree hε₁)
        have hflow' := Flow.lsubst p.recordDeep σ hflow
        have hnv : ¬ Vis p L (ε₁.inst (p.sd σ)).failLabel →
            ¬ Vis p L ((pc.join ε₁.failLabel).inst (p.sd σ)) := fun hn h =>
          hn (Vis.failLabel_inst.mpr (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
        refine andThen_brelR (ihE hσ hE hΓ hpc hw he₁ he₂ hrel hr) Vis.union_left_inst
          (fun v₁ v₂ k₁ k₂ hv hw' => ?_) ?_ ?_
        · have hv₁ : LValTy p v₁ (ty.lsubst p.recordDeep σ) := by
            have := u₁.2.1; rw [k₁] at this; exact this
          have hv₂ : LValTy p v₂ (ty.lsubst p.recordDeep σ) := by
            have := u₂.2.1; rw [k₂] at this; exact this
          exact BRelR.mono (fun _ _ h => h) Vis.union_right_inst
            (ihE hσ hE (hcons hann) hpc' hw' (.cons (LValTy.flow hv₁ hflow') he₁)
              (.cons (LValTy.flow hv₂ hflow') he₂) (.cons (V.flow hv hflow') hrel) hb)
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₁.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (eval H n p w₁ e₁ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₁) hb) (hnv hn)
          exact ⟨hp, fun hc hvf => hf hc (Vis.union_right_inst hvf), hwc⟩
        · intro hn v k
          have hv : LValTy p v (ty.lsubst p.recordDeep σ) := by have := u₂.2.1; rw [k] at this; exact this
          obtain ⟨hp, hf, hwc⟩ := confined (L := L) hh
            (uE (w := (eval H n p w₂ e₂ value).2.2) hσ (.cons (LValTy.flow hv hflow') he₂) hb) (hnv hn)
          exact ⟨hp, fun hc hvf => hf hc (Vis.union_right_inst hvf), hwc⟩
      | @ite _ _ cond thenE elseE lc t₁ t₂ s ε₁ ε₂ hc ht hel h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hc
        have hlc : lc.PlainFree := latom_inPlace hΓ hc
        have hpc' : (pc.join lc).PlainFree := LExpr.plainFree_join hpc hlc
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env cond = some v → LValTy p v (.bool (lc.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join lc).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (FailsAs p ((pc.join lc).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.ite cond thenE elseE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | @bool b _ =>
            cases b
            · simp only [eval, hcv']
              exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
                (FailsAs.union_right_inst (LExpr.le_refl _)) (uE hσ henv hel)
            · simp only [eval, hcv']
              exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) FailsAs.union_left_inst
                (uE hσ henv ht)
        by_cases hv : Vis p L (lc.inst (p.sd σ))
        · cases hcu₁ with
          | @bool b _ =>
            have := V.eq_bool hcv hv
            subst this
            cases b
            · simp only [eval, hc₁, hc₂]
              exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) Vis.union_right_inst
                (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hel)
            · simp only [eval, hc₁, hc₂]
              exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁))) Vis.union_left_inst
                (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel ht)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hlc a b hv) id id
      | @caseOption _ _ scr noneE x someE te ls t₁ t₂ s ε₁ ε₂ hs hn hsome h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        have hsi := latom_inPlace hΓ hs
        simp only [LTy.InPlace] at hsi
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi.2
        have hΓ' : LCtx.InPlace ((x, te.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2 hsi.1)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v →
            LValTy p v (.option (te.lsubst p.recordDeep σ) (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (FailsAs p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseOption scr noneE x someE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | none =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) FailsAs.union_left_inst
              (uE hσ henv hn)
          | some hw =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
              (FailsAs.union_right_inst (LExpr.le_refl _))
              (uE hσ (.cons (LValTy.lsubst_raise hw) henv) hsome)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | none =>
            simp only [eval, hc₁, hc₂]
            exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁))) Vis.union_left_inst
              (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hn)
          | some hab =>
            cases hcu₁ with
            | some hw₁ =>
              cases hcu₂ with
              | some hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂)))
                  Vis.union_right_inst
                  (ihE hσ hE hΓ' hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) hsome)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hsi.2 a b hv) id id
      | @caseResult _ _ scr x okE y errE te ee ls t₁ t₂ s ε₁ ε₂ hs hok herr h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        have hsi := latom_inPlace hΓ hs
        simp only [LTy.InPlace] at hsi
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi.2.2
        have hΓ₁ : LCtx.InPlace ((x, te.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2.2 hsi.1)
        have hΓ₂ : LCtx.InPlace ((y, ee.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2.2 hsi.2.1)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v →
            LValTy p v (.result (te.lsubst p.recordDeep σ) (ee.lsubst p.recordDeep σ) (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (FailsAs p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseResult scr x okE y errE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | ok hw =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) FailsAs.union_left_inst
              (uE hσ (.cons (LValTy.lsubst_raise hw) henv) hok)
          | err hw =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
              (FailsAs.union_right_inst (LExpr.le_refl _)) (uE hσ (.cons (LValTy.lsubst_raise hw) henv) herr)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | ok hab =>
            cases hcu₁ with
            | ok hw₁ =>
              cases hcu₂ with
              | ok hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁)))
                  Vis.union_left_inst
                  (ihE hσ hE hΓ₁ hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) hok)
          | err hab =>
            cases hcu₁ with
            | err hw₁ =>
              cases hcu₂ with
              | err hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) Vis.union_right_inst
                  (ihE hσ hE hΓ₂ hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) herr)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hsi.2.2 a b hv) id id
      | fail _ _ _ =>
        simp only [eval]
        exact .of_brel (brel_same hw (fun _ h => nomatch h))
      | raise hs hf has hfl _ _ _ =>
        -- Both runs raise the same variant; its fields are related at their declared types, a
        -- field the observer does not see may differ (#512).
        refine BRelR.of_brel ?_
        obtain ⟨vs₁, vs₂, h₁, h₂, hvs, _, _⟩ := latoms_bothσ he₁ he₂ hrel has
        simp only [eval, h₁, h₂]
        intro _ _
        exact ⟨rfl, fun _ => ⟨rfl, rfl, _, _, hs, hf, VList.ground_lsubst
          (fun u hu => (hwf.sums _ (sum_mem hs) _ (List.mem_of_getElem? hf) u hu).1)
          (VList.flowsσ hvs hfl)⟩, hw⟩
      | unreachable _ =>
        -- Never reached in a run that ends: `aborted` is not `Term` (#512).
        refine BRelR.of_brel ?_
        intro ht
        simp [eval, Term] at ht
      | @caseSum _ _ scr kind ctor binders thenE elseE ls sm fs t₁ t₂ s ε₁ ε₂ hsc hsm hf hlen hth hel h₁ h₂ _ =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hsc
        have hsi : ls.PlainFree := latom_inPlace hΓ hsc
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi
        have hg : ∀ f ∈ fs, f.Ground :=
          fun u hu => (hwf.sums _ (sum_mem hsm) _ (List.mem_of_getElem? hf) u hu).1
        have hΓ₁ := caseSum_ctx_inPlace hwf hΓ hsi hsm hf (xs := binders)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v → LValTy p v (.record kind (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun _ => True)
              (FailsAs p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseSum scr ctor binders thenE elseE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | record hrec _ => exact (not_sum_of_record hwf hrec hsm).elim
          | @variant _ i vs sm' fs' _ hsm' hf' hvals =>
            rw [hsm] at hsm'
            cases hsm'
            simp only [eval, hcv']
            by_cases hi : i = ctor
            · subst hi
              rw [hf] at hf'
              cases hf'
              rw [if_pos rfl, if_pos (by rw [LValsTy.length hvals]; exact hlen)]
              have henv' := zip_ltyped (xs := binders) henv
                (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hvals) (by simp [hlen])
              rw [← LCtx.lsub_zip, ← LCtx.lsub_append] at henv'
              exact UOk.mono allowed_union_left_inst (fun _ _ => trivial) FailsAs.union_left_inst
                (uE hσ henv' hth)
            · rw [if_neg hi]
              exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ _ => trivial)
                (FailsAs.union_right_inst (LExpr.le_refl _)) (uE hσ henv hel)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | record hrec _ => exact (not_sum_of_record hwf hrec hsm).elim
          | @variant _ _ sm' i fs' vs₁ vs₂ hsm' hf' hvals =>
            rw [hsm] at hsm'
            cases hsm'
            cases hcu₁ with
            | variant hs₁ hf₁ hu₁ =>
              cases hcu₂ with
              | variant hs₂ hf₂ hu₂ =>
                rw [hsm] at hs₁ hs₂
                cases hs₁
                cases hs₂
                rw [hf'] at hf₁ hf₂
                cases hf₁
                cases hf₂
                simp only [eval, hc₁, hc₂]
                by_cases hi : i = ctor
                · subst hi
                  rw [hf] at hf'
                  cases hf'
                  rw [if_pos rfl, if_pos (by rw [LValsTy.length hu₁]; exact hlen), if_pos rfl,
                    if_pos (by rw [LValsTy.length hu₂]; exact hlen)]
                  have henv₁ := zip_ltyped (xs := binders) he₁
                    (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hu₁) (by simp [hlen])
                  have henv₂ := zip_ltyped (xs := binders) he₂
                    (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hu₂) (by simp [hlen])
                  have hrelz := EnvRel.zip (xs := binders) hrel
                    (VList.raise_lsubst_ground (l := ls) (σ := σ) hg hvals) (by simp [hlen])
                  rw [← LCtx.lsub_zip, ← LCtx.lsub_append] at henv₁ henv₂ hrelz
                  exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₁)))
                    Vis.union_left_inst (ihE hσ hE hΓ₁ hpc' hw henv₁ henv₂ hrelz hth)
                · rw [if_neg hi, if_neg hi]
                  exact BRelR.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂)))
                    Vis.union_right_inst (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hel)
        · exact .of_brel <| brel_confined hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b => V.hideσ hsi a b hv) id id
      | @caseSumT _ _ scr kind ctor binders thenE elseE ls sm fs t₁ t₂ ts ε₁ ε₂ hsc hsm hf hlen hth hel h₁ h₂ _ hcov =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hsc
        have hsi : ls.PlainFree := latom_inPlace hΓ hsc
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi
        have hg : ∀ f ∈ fs, f.Ground :=
          fun u hu => (hwf.sums _ (sum_mem hsm) _ (List.mem_of_getElem? hf) u hu).1
        have hΓ₁ := caseSum_ctx_inPlace hwf hΓ hsi hsm hf (xs := binders)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v → LValTy p v (.record kind (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ)))
              (fun v => LValTy p v ((LTy.tuple ts .bot).lsubst p.recordDeep σ))
              (FailsAs p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseSum scr ctor binders thenE elseE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | record hrec _ => exact (not_sum_of_record hwf hrec hsm).elim
          | @variant _ i vs sm' fs' _ hsm' hf' hvals =>
            rw [hsm] at hsm'
            cases hsm'
            simp only [eval, hcv']
            by_cases hi : i = ctor
            · subst hi
              rw [hf] at hf'
              cases hf'
              rw [if_pos rfl, if_pos (by rw [LValsTy.length hvals]; exact hlen)]
              have henv' := zip_ltyped (xs := binders) henv
                (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hvals) (by simp [hlen])
              rw [← LCtx.lsub_zip, ← LCtx.lsub_append] at henv'
              exact UOk.mono allowed_union_left_inst (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁))
                FailsAs.union_left_inst (uE hσ henv' hth)
            · rw [if_neg hi]
              exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _))
                (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
                (FailsAs.union_right_inst (LExpr.le_refl _)) (uE hσ henv hel)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | record hrec _ => exact (not_sum_of_record hwf hrec hsm).elim
          | @variant _ _ sm' i fs' vs₁ vs₂ hsm' hf' hvals =>
            rw [hsm] at hsm'
            cases hsm'
            cases hcu₁ with
            | variant hs₁ hf₁ hu₁ =>
              cases hcu₂ with
              | variant hs₂ hf₂ hu₂ =>
                rw [hsm] at hs₁ hs₂
                cases hs₁
                cases hs₂
                rw [hf'] at hf₁ hf₂
                cases hf₁
                cases hf₂
                simp only [eval, hc₁, hc₂]
                by_cases hi : i = ctor
                · subst hi
                  rw [hf] at hf'
                  cases hf'
                  rw [if_pos rfl, if_pos (by rw [LValsTy.length hu₁]; exact hlen), if_pos rfl,
                    if_pos (by rw [LValsTy.length hu₂]; exact hlen)]
                  have henv₁ := zip_ltyped (xs := binders) he₁
                    (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hu₁) (by simp [hlen])
                  have henv₂ := zip_ltyped (xs := binders) he₂
                    (LValsTy.raise_lsubst_ground (l := ls) (σ := σ) hg hu₂) (by simp [hlen])
                  have hrelz := EnvRel.zip (xs := binders) hrel
                    (VList.raise_lsubst_ground (l := ls) (σ := σ) hg hvals) (by simp [hlen])
                  rw [← LCtx.lsub_zip, ← LCtx.lsub_append] at henv₁ henv₂ hrelz
                  exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₁))
                    Vis.union_left_inst (ihE hσ hE hΓ₁ hpc' hw henv₁ henv₂ hrelz hth)
                · rw [if_neg hi, if_neg hi]
                  exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₂))
                    Vis.union_right_inst (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hel)
        · exact .of_brel <| brel_confined_typed hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b ha hb => V.tuple_covσ hsi hv hcov ha hb) id id
      | @attempt _ _ body ty ε₀ hb hr0 =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have u₁ := uE (w := w₁) hσ he₁ hb
        have u₂ := uE (w := w₂) hσ he₂ hb
        simp only [eval]
        generalize eval H n p w₁ e₁ body = r₁ at h u₁ ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h u₂ ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        -- The body raises no payload (#512): `attempt` of a code asks `ε.raises = []`.
        have nr₁ : ∀ k c ps, o₁ ≠ .raised k c ps := fun k c ps he => by
          subst he; have := u₁.2.1.2.1; simp [Eff.inst, hr0] at this
        have nr₂ : ∀ k c ps, o₂ ≠ .raised k c ps := fun k c ps he => by
          subst he; have := u₂.2.1.2.1; simp [Eff.inst, hr0] at this
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> exact hd
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        refine BRelR.of_brel ?_
        intro ht₁ ht₂
        simp only [LTy.lsubst]
        cases o₁ <;> cases o₂ <;> simp only [Term] at ht₁ ht₂ <;>
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial <;> refine ⟨hp, ?_, hwr⟩ <;>
          simp only [OutRel] at ho ⊢
        all_goals (try (first | exact absurd rfl (nr₁ _ _ _) | exact absurd rfl (nr₂ _ _ _)))
        · exact .ok ho
        · exact .hidden (fun hv => ho (Vis.failLabel_inst.mpr hv))
        · exact .hidden (fun hv => ho (Vis.failLabel_inst.mpr hv))
        · by_cases hv : Vis p L (ε₀.failLabel.inst (p.sd σ))
          · rw [ho (Vis.failLabel_inst.mpr hv)]; exact .err .int
          · exact .hidden hv
      | @attemptK _ _ kind body ty ε₀ hb hcd hra =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have u₁ := uE (w := w₁) hσ he₁ hb
        have u₂ := uE (w := w₂) hσ he₂ hb
        simp only [eval]
        generalize eval H n p w₁ e₁ body = r₁ at h u₁ ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h u₂ ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        -- The body fails only with `kind` (#512): no code, and no other payload kind.
        have nf₁ : ∀ c, o₁ ≠ .failed c := fun c he => by
          subst he; have := u₁.2.1.2; simp [Eff.inst, hcd] at this
        have nf₂ : ∀ c, o₂ ≠ .failed c := fun c he => by
          subst he; have := u₂.2.1.2; simp [Eff.inst, hcd] at this
        have hk₁ : ∀ k c ps, o₁ = .raised k c ps → k = kind := fun k c ps he => by
          subst he; exact hra k (by simpa [Eff.inst] using u₁.2.1.2.1)
        have hk₂ : ∀ k c ps, o₂ = .raised k c ps → k = kind := fun k c ps he => by
          subst he; exact hra k (by simpa [Eff.inst] using u₂.2.1.2.1)
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> dsimp only <;> (try split) <;> (try split) <;> exact hd
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        refine BRelR.of_brel ?_
        cases o₁ <;> cases o₂
        all_goals first
          | exact absurd rfl (nf₁ _)
          | exact absurd rfl (nf₂ _)
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | (intro ht₁ ht₂; dsimp only at ht₁ ht₂; split at ht₁ <;> simp only [Term] at ht₁ ht₂; done)
          | (intro ht₁ ht₂; dsimp only at ht₁ ht₂; split at ht₂ <;> simp only [Term] at ht₁ ht₂; done)
          | skip
        · intro _ _
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          exact ⟨hp, by simp only [LTy.lsubst]; exact .ok ho, hwr⟩
        · rename_i v k c ps
          obtain rfl := hk₂ k c ps rfl
          intro _ _
          dsimp only
          rw [if_pos rfl]
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          refine ⟨hp, ?_, hwr⟩
          exact .hidden (fun hv => ho (Vis.failLabel_inst.mpr hv))
        · rename_i k c ps v
          obtain rfl := hk₁ k c ps rfl
          intro _ _
          dsimp only
          rw [if_pos rfl]
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          refine ⟨hp, ?_, hwr⟩
          exact .hidden (fun hv => ho (Vis.failLabel_inst.mpr hv))
        · rename_i k c ps k' c' ps'
          obtain rfl := hk₁ k c ps rfl
          obtain rfl := hk₂ k' c' ps' rfl
          intro _ _
          dsimp only
          rw [if_pos rfl]
          try rw [if_pos rfl]
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          refine ⟨hp, ?_, hwr⟩
          by_cases hv : Vis p L (ε₀.failLabel.inst (p.sd σ))
          · -- A seen failure: the same variant, with fields related at their declared types.
            obtain ⟨-, rfl, sm, fs, hs, hf, hvl⟩ := ho (Vis.failLabel_inst.mpr hv)
            simp only [OutRel, LTy.lsubst]
            exact .err (.variant hs hf hvl)
          · simp only [OutRel]
            exact .hidden hv
      | @handle _ _ body x handler t₁ t₂ s ε₁ ε₂ hb hr0 hh' h₁ h₂ _ =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have u₁ := uE (w := w₁) hσ he₁ hb
        have u₂ := uE (w := w₂) hσ he₂ hb
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hb).2
        have hlF := Eff.failLabel_plainFree hε₁
        have hpc' : (pc.join ε₁.failLabel).PlainFree := LExpr.plainFree_join hpc hlF
        have hΓ' : LCtx.InPlace ((x, .int ε₁.failLabel) :: Γ) := hcons hlF
        simp only [eval]
        generalize eval H n p w₁ e₁ body = r₁ at h u₁ ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h u₂ ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        -- The body raises no payload (#512): a handler of a code asks `ε₁.raises = []`.
        have nr₁ : ∀ k c ps, o₁ ≠ .raised k c ps := fun k c ps he => by
          subst he; have := u₁.2.1.2.1; simp [Eff.inst, hr0] at this
        have nr₂ : ∀ k c ps, o₂ ≠ .raised k c ps := fun k c ps he => by
          subst he; have := u₂.2.1.2.1; simp [Eff.inst, hr0] at this
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> dsimp only
          all_goals first
            | exact hd
            | exact hd.append _ _
            | simpa using hd.append [] _
            | simpa using hd.append _ []
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        cases o₁ <;> cases o₂
        all_goals first
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | exact absurd rfl (nr₁ _ _ _)
          | exact absurd rfl (nr₂ _ _ _)
          | skip
        · intro ht₁ ht₂
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          exact .inl ⟨hp, V.raiseσ _ (V.flow ho (Flow.lsubst _ _ h₁)), hwr⟩
        · rename_i a c
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₂') hσ (.cons (LValTy.int (n := (c : Int))) he₂) hh'
          generalize eval H n p w₂' ((x, Val.int (c : Int)) :: e₂) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          refine .of_brel <| brel_quiet (hwr.trans hwh)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hideσ hlF a b ho') (fun h => by simp [Res.IsFail] at h) (fun hc' => hfh hc')
        · rename_i c b
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₁') hσ (.cons (LValTy.int (n := (c : Int))) he₁) hh'
          generalize eval H n p w₁' ((x, Val.int (c : Int)) :: e₁) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          refine .of_brel <| brel_quiet (hwh.symm.trans hwr)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hideσ hlF a b ho') (fun hc' => hfh hc') (fun h => by simp [Res.IsFail] at h)
        · rename_i c₁ c₂
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have hcv : V p L ((LTy.int ε₁.failLabel).lsubst p.recordDeep σ) (.int (c₁ : Int)) (.int (c₂ : Int)) := by
            simp only [LTy.lsubst]
            by_cases hv : Vis p L (ε₁.failLabel.inst (p.sd σ))
            · rw [ho (Vis.failLabel_inst.mpr hv)]; exact .int
            · exact .hidden hv
          have hh'' := ihE hσ hE hΓ' hpc' hwr (.cons (LValTy.int (n := (c₁ : Int))) he₁)
            (.cons (LValTy.int (n := (c₂ : Int))) he₂) (.cons hcv hrel) hh'
          generalize eval H n p w₁' ((x, Val.int (c₁ : Int)) :: e₁) handler = rh₁ at hh'' ⊢
          generalize eval H n p w₂' ((x, Val.int (c₂ : Int)) :: e₂) handler = rh₂ at hh'' ⊢
          obtain ⟨th₁, oh₁, wh₁⟩ := rh₁
          obtain ⟨th₂, oh₂, wh₂⟩ := rh₂
          intro ht₁ ht₂
          rcases hh'' ht₁ ht₂ with ⟨hph, hoh, hwh⟩ | hdh
          · exact .inl ⟨by simp only [proj_append]; rw [hp, hph],
              OutRel.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) id hoh, hwh⟩
          · exact .inr (Diverged.prepend hp hdh)
      | @handleK _ _ kind body x handler t₁ t₂ s ε₁ ε₂ hb hcd hra hh' h₁ h₂ _ =>
        -- A handler of a payload kind (#512): it binds the failure's value, at the label of what
        -- decides the failure, so a field read from it is raised by that label.
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have u₁ := uE (w := w₁) hσ he₁ hb
        have u₂ := uE (w := w₂) hσ he₂ hb
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hb).2
        have hlF := Eff.failLabel_plainFree hε₁
        have hpc' : (pc.join ε₁.failLabel).PlainFree := LExpr.plainFree_join hpc hlF
        have hΓ' : LCtx.InPlace ((x, .record kind ε₁.failLabel) :: Γ) := hcons hlF
        simp only [eval]
        generalize eval H n p w₁ e₁ body = r₁ at h u₁ ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h u₂ ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        have nf₁ : ∀ c, o₁ ≠ .failed c := fun c he => by
          subst he; have := u₁.2.1.2; simp [Eff.inst, hcd] at this
        have nf₂ : ∀ c, o₂ ≠ .failed c := fun c he => by
          subst he; have := u₂.2.1.2; simp [Eff.inst, hcd] at this
        have hk₁ : ∀ k c ps, o₁ = .raised k c ps → k = kind := fun k c ps he => by
          subst he; exact hra k (by simpa [Eff.inst] using u₁.2.1.2.1)
        have hk₂ : ∀ k c ps, o₂ = .raised k c ps → k = kind := fun k c ps he => by
          subst he; exact hra k (by simpa [Eff.inst] using u₂.2.1.2.1)
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> (try dsimp only) <;> (try split) <;> (try split) <;> (try dsimp only)
          all_goals first
            | exact hd
            | exact hd.append _ _
            | simpa using hd.append [] _
            | simpa using hd.append _ []
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        cases o₁ <;> cases o₂
        all_goals first
          | exact absurd rfl (nf₁ _)
          | exact absurd rfl (nf₂ _)
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | (intro ht₁ ht₂; dsimp only at ht₁ ht₂; split at ht₁ <;> simp only [Term] at ht₁ ht₂; done)
          | (intro ht₁ ht₂; dsimp only at ht₁ ht₂; split at ht₂ <;> simp only [Term] at ht₁ ht₂; done)
          | skip
        · intro ht₁ ht₂
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          exact .inl ⟨hp, V.raiseσ _ (V.flow ho (Flow.lsubst _ _ h₁)), hwr⟩
        · rename_i a k c ps
          obtain rfl := hk₂ k c ps rfl
          dsimp only
          rw [if_pos rfl]
          obtain ⟨_, _, sm, fs, hs, hf, hps⟩ := u₂.2.1
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₂') hσ (.cons (LValTy.variant (l := _) hs hf hps) he₂) hh'
          generalize eval H n p w₂' ((x, Val.variant k c ps) :: e₂) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          refine .of_brel <| brel_quiet (hwr.trans hwh)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hideσ hlF a b ho') (fun h => by simp [Res.IsFail] at h) (fun hc' => hfh hc')
        · rename_i k c ps b
          obtain rfl := hk₁ k c ps rfl
          dsimp only
          rw [if_pos rfl]
          obtain ⟨_, _, sm, fs, hs, hf, hps⟩ := u₁.2.1
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₁') hσ (.cons (LValTy.variant (l := _) hs hf hps) he₁) hh'
          generalize eval H n p w₁' ((x, Val.variant k c ps) :: e₁) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          refine .of_brel <| brel_quiet (hwh.symm.trans hwr)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a b => V.hideσ hlF a b ho') (fun hc' => hfh hc') (fun h => by simp [Res.IsFail] at h)
        · rename_i k₁ c₁ ps₁ k₂ c₂ ps₂
          obtain rfl := hk₁ k₁ c₁ ps₁ rfl
          obtain rfl := hk₂ k₂ c₂ ps₂ rfl
          dsimp only
          rw [if_pos rfl]
          try rw [if_pos rfl]
          obtain ⟨_, _, sm₁, fs₁, hs₁, hf₁, hps₁⟩ := u₁.2.1
          obtain ⟨_, _, sm₂, fs₂, hs₂, hf₂, hps₂⟩ := u₂.2.1
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          -- The binder: the same variant with related fields when the observer sees what decides
          -- the failure, and hidden otherwise.
          have hcv : V p L ((LTy.record k₂ ε₁.failLabel).lsubst p.recordDeep σ)
              (.variant k₂ c₁ ps₁) (.variant k₂ c₂ ps₂) := by
            simp only [LTy.lsubst]
            by_cases hv : Vis p L (ε₁.failLabel.inst (p.sd σ))
            · obtain ⟨-, rfl, sm, fs, hs, hf, hvl⟩ := ho (Vis.failLabel_inst.mpr hv)
              exact .variant hs hf hvl
            · exact .hidden hv
          have hh'' := ihE hσ hE hΓ' hpc' hwr (.cons (LValTy.variant (l := _) hs₁ hf₁ hps₁) he₁)
            (.cons (LValTy.variant (l := _) hs₂ hf₂ hps₂) he₂) (.cons hcv hrel) hh'
          generalize eval H n p w₁' ((x, Val.variant k₂ c₁ ps₁) :: e₁) handler = rh₁ at hh'' ⊢
          generalize eval H n p w₂' ((x, Val.variant k₂ c₂ ps₂) :: e₂) handler = rh₂ at hh'' ⊢
          obtain ⟨th₁, oh₁, wh₁⟩ := rh₁
          obtain ⟨th₂, oh₂, wh₂⟩ := rh₂
          intro ht₁ ht₂
          rcases hh'' ht₁ ht₂ with ⟨hph, hoh, hwh⟩ | hdh
          · exact .inl ⟨by simp only [proj_append]; rw [hp, hph],
              OutRel.mono (fun _ _ h => V.raiseσ _ (V.flow h (Flow.lsubst _ _ h₂))) id hoh, hwh⟩
          · exact .inr (Diverged.prepend hp hdh)
      | @iteT _ _ cond thenE elseE lc t₁ t₂ ts ε₁ ε₂ hc ht hel h₁ h₂ _ hcov =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hc
        have hlc : lc.PlainFree := latom_inPlace hΓ hc
        have hpc' : (pc.join lc).PlainFree := LExpr.plainFree_join hpc hlc
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env cond = some v → LValTy p v (.bool (lc.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join lc).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun v => LValTy p v ((LTy.tuple ts .bot).lsubst p.recordDeep σ))
              (FailsAs p ((pc.join lc).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.ite cond thenE elseE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | @bool b _ =>
            cases b
            · simp only [eval, hcv']
              exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
                (FailsAs.union_right_inst (LExpr.le_refl _)) (uE hσ henv hel)
            · simp only [eval, hcv']
              exact UOk.mono allowed_union_left_inst (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁)) FailsAs.union_left_inst
                (uE hσ henv ht)
        by_cases hv : Vis p L (lc.inst (p.sd σ))
        · cases hcu₁ with
          | @bool b _ =>
            have := V.eq_bool hcv hv
            subst this
            cases b
            · simp only [eval, hc₁, hc₂]
              exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₂)) Vis.union_right_inst
                (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hel)
            · simp only [eval, hc₁, hc₂]
              exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₁)) Vis.union_left_inst
                (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel ht)
        · exact .of_brel <| brel_confined_typed hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b ha hb => V.tuple_covσ hlc hv hcov ha hb) id id
      | @caseOptionT _ _ scr noneE x someE te ls t₁ t₂ ts ε₁ ε₂ hs hn hsome h₁ h₂ _ hcov =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        have hsi := latom_inPlace hΓ hs
        simp only [LTy.InPlace] at hsi
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi.2
        have hΓ' : LCtx.InPlace ((x, te.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2 hsi.1)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v →
            LValTy p v (.option (te.lsubst p.recordDeep σ) (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun v => LValTy p v ((LTy.tuple ts .bot).lsubst p.recordDeep σ))
              (FailsAs p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseOption scr noneE x someE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | none =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left_inst (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁)) FailsAs.union_left_inst
              (uE hσ henv hn)
          | some hw =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
              (FailsAs.union_right_inst (LExpr.le_refl _))
              (uE hσ (.cons (LValTy.lsubst_raise hw) henv) hsome)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | none =>
            simp only [eval, hc₁, hc₂]
            exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₁)) Vis.union_left_inst
              (ihE hσ hE hΓ hpc' hw he₁ he₂ hrel hn)
          | some hab =>
            cases hcu₁ with
            | some hw₁ =>
              cases hcu₂ with
              | some hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₂))
                  Vis.union_right_inst
                  (ihE hσ hE hΓ' hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) hsome)
        · exact .of_brel <| brel_confined_typed hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b ha hb => V.tuple_covσ hsi.2 hv hcov ha hb) id id
      | @caseResultT _ _ scr x okE y errE te ee ls t₁ t₂ ts ε₁ ε₂ hs hok herr h₁ h₂ _ hcov =>
        obtain ⟨c₁, c₂, hc₁, hc₂, hcv, hcu₁, hcu₂⟩ := latom_bothσ he₁ he₂ hrel hs
        have hsi := latom_inPlace hΓ hs
        simp only [LTy.InPlace] at hsi
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hsi.2.2
        have hΓ₁ : LCtx.InPlace ((x, te.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2.2 hsi.1)
        have hΓ₂ : LCtx.InPlace ((y, ee.raise ls) :: Γ) := hcons (LTy.raise_inPlace hsi.2.2 hsi.2.1)
        simp only [LTy.lsubst] at hcv hcu₁ hcu₂
        have U : ∀ {w : World H} {env : Env} {v : Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
            evalAtom env scr = some v →
            LValTy p v (.result (te.lsubst p.recordDeep σ) (ee.lsubst p.recordDeep σ) (ls.inst (p.sd σ))) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) (fun v => LValTy p v ((LTy.tuple ts .bot).lsubst p.recordDeep σ))
              (FailsAs p ((pc.join ls).inst (p.sd σ)) ((ε₁.union ε₂).inst (p.sd σ))) w
              (eval H (n + 1) p w env (.caseResult scr x okE y errE)) := by
          intro w env v henv hcv' hvt
          cases hvt with
          | ok hw =>
            simp only [eval, hcv']
            exact UOk.mono allowed_union_left_inst (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₁)) FailsAs.union_left_inst
              (uE hσ (.cons (LValTy.lsubst_raise hw) henv) hok)
          | err hw =>
            simp only [eval, hcv']
            exact UOk.mono (allowed_union_right_inst (LExpr.le_refl _)) (fun _ h => LValTy.flow h (Flow.lsubst _ _ h₂))
              (FailsAs.union_right_inst (LExpr.le_refl _)) (uE hσ (.cons (LValTy.lsubst_raise hw) henv) herr)
        by_cases hv : Vis p L (ls.inst (p.sd σ))
        · cases hcv with
          | hidden h => exact absurd hv h
          | ok hab =>
            cases hcu₁ with
            | ok hw₁ =>
              cases hcu₂ with
              | ok hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₁))
                  Vis.union_left_inst
                  (ihE hσ hE hΓ₁ hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) hok)
          | err hab =>
            cases hcu₁ with
            | err hw₁ =>
              cases hcu₂ with
              | err hw₂ =>
                simp only [eval, hc₁, hc₂]
                exact BRelR.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₂)) Vis.union_right_inst
                  (ihE hσ hE hΓ₂ hpc' hw (.cons (LValTy.lsubst_raise hw₁) he₁)
                    (.cons (LValTy.lsubst_raise hw₂) he₂) (.cons (V.raiseσ _ hab) hrel) herr)
        · exact .of_brel <| brel_confined_typed hh hw (U he₁ hc₁ hcu₁) (U he₂ hc₂ hcu₂)
            (fun h => hv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
            (fun a b ha hb => V.tuple_covσ hsi.2.2 hv hcov ha hb) id id
      | @handleT _ _ body x handler t₁ t₂ ts ε₁ ε₂ hb hr0 hh' h₁ h₂ _ hcov =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hb).2
        have hlF := Eff.failLabel_plainFree hε₁
        have hpc' : (pc.join ε₁.failLabel).PlainFree := LExpr.plainFree_join hpc hlF
        have hΓ' : LCtx.InPlace ((x, .int ε₁.failLabel) :: Γ) := hcons hlF
        simp only [eval]
        have ub₁ := uE (w := w₁) hσ he₁ hb
        have ub₂ := uE (w := w₂) hσ he₂ hb
        generalize eval H n p w₁ e₁ body = r₁ at h ub₁ ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h ub₂ ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        have nr₁ : ∀ k c ps, o₁ ≠ .raised k c ps := fun k c ps he => by
          subst he; have := ub₁.2.1.2.1; simp [Eff.inst, hr0] at this
        have nr₂ : ∀ k c ps, o₂ ≠ .raised k c ps := fun k c ps he => by
          subst he; have := ub₂.2.1.2.1; simp [Eff.inst, hr0] at this
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> dsimp only
          all_goals first
            | exact hd
            | exact hd.append _ _
            | simpa using hd.append [] _
            | simpa using hd.append _ []
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        cases o₁ <;> cases o₂
        all_goals first
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | exact absurd rfl (nr₁ _ _ _)
          | exact absurd rfl (nr₂ _ _ _)
          | skip
        · intro ht₁ ht₂
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          exact .inl ⟨hp, V.flow ho (Flow.lsubst _ _ h₁), hwr⟩
        · rename_i a c
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₂') hσ (.cons (LValTy.int (n := (c : Int))) he₂) hh'
          generalize eval H n p w₂' ((x, Val.int (c : Int)) :: e₂) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          have ta : LValTy p a (t₁.lsubst p.recordDeep σ) := ub₁.2.1
          have tb := uh.2.1
          refine .of_brel <| brel_quiet_typed (hwr.trans hwh)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a' b' ea eb => by
              simp only [Res.ok.injEq] at ea eb
              subst ea eb
              exact V.tuple_covσ hlF ho' hcov (LValTy.flow ta (Flow.lsubst _ _ h₁))
                (LValTy.flow tb (Flow.lsubst _ _ h₂)))
            (fun h => by simp [Res.IsFail] at h) (fun hc' => hfh hc')
        · rename_i c b
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₁') hσ (.cons (LValTy.int (n := (c : Int))) he₁) hh'
          generalize eval H n p w₁' ((x, Val.int (c : Int)) :: e₁) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          have tb : LValTy p b (t₁.lsubst p.recordDeep σ) := ub₂.2.1
          have ta := uh.2.1
          refine .of_brel <| brel_quiet_typed (hwh.symm.trans hwr)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a' b' ea eb => by
              simp only [Res.ok.injEq] at ea eb
              subst ea eb
              exact V.tuple_covσ hlF ho' hcov (LValTy.flow ta (Flow.lsubst _ _ h₂))
                (LValTy.flow tb (Flow.lsubst _ _ h₁)))
            (fun hc' => hfh hc') (fun h => by simp [Res.IsFail] at h)
        · rename_i c₁ c₂
          dsimp only
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have hcv : V p L ((LTy.int ε₁.failLabel).lsubst p.recordDeep σ) (.int (c₁ : Int)) (.int (c₂ : Int)) := by
            simp only [LTy.lsubst]
            by_cases hv : Vis p L (ε₁.failLabel.inst (p.sd σ))
            · rw [ho (Vis.failLabel_inst.mpr hv)]; exact .int
            · exact .hidden hv
          have hh'' := ihE hσ hE hΓ' hpc' hwr (.cons (LValTy.int (n := (c₁ : Int))) he₁)
            (.cons (LValTy.int (n := (c₂ : Int))) he₂) (.cons hcv hrel) hh'
          generalize eval H n p w₁' ((x, Val.int (c₁ : Int)) :: e₁) handler = rh₁ at hh'' ⊢
          generalize eval H n p w₂' ((x, Val.int (c₂ : Int)) :: e₂) handler = rh₂ at hh'' ⊢
          obtain ⟨th₁, oh₁, wh₁⟩ := rh₁
          obtain ⟨th₂, oh₂, wh₂⟩ := rh₂
          intro ht₁ ht₂
          rcases hh'' ht₁ ht₂ with ⟨hph, hoh, hwh⟩ | hdh
          · exact .inl ⟨by simp only [proj_append]; rw [hp, hph],
              OutRel.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₂)) id hoh, hwh⟩
          · exact .inr (Diverged.prepend hp hdh)
      | @handleKT _ _ kind body x handler t₁ t₂ ts ε₁ ε₂ hb hcd hra hh' h₁ h₂ _ hcov =>
        have h := ihE hσ hE hΓ hpc hw he₁ he₂ hrel hb
        have hε₁ := (LExprTy.inPlace hwf hΓ hpc hb).2
        have hlF := Eff.failLabel_plainFree hε₁
        have hpc' : (pc.join ε₁.failLabel).PlainFree := LExpr.plainFree_join hpc hlF
        have hΓ' : LCtx.InPlace ((x, .record kind ε₁.failLabel) :: Γ) := hcons hlF
        simp only [eval]
        have ub₁ := uE (w := w₁) hσ he₁ hb
        have ub₂ := uE (w := w₂) hσ he₂ hb
        generalize eval H n p w₁ e₁ body = r₁ at h ub₁ ⊢
        generalize eval H n p w₂ e₂ body = r₂ at h ub₂ ⊢
        obtain ⟨tr₁, o₁, w₁'⟩ := r₁
        obtain ⟨tr₂, o₂, w₂'⟩ := r₂
        have nf₁ : ∀ c, o₁ ≠ .failed c := fun c he => by
          subst he; have := ub₁.2.1.2; simp [Eff.inst, hcd] at this
        have nf₂ : ∀ c, o₂ ≠ .failed c := fun c he => by
          subst he; have := ub₂.2.1.2; simp [Eff.inst, hcd] at this
        have hk₁ : ∀ k c ps, o₁ = .raised k c ps → k = kind := fun k c ps he => by
          subst he; exact hra k (by simpa [Eff.inst] using ub₁.2.1.2.1)
        have hk₂ : ∀ k c ps, o₂ = .raised k c ps → k = kind := fun k c ps he => by
          subst he; exact hra k (by simpa [Eff.inst] using ub₂.2.1.2.1)
        by_cases hd : Diverged p L tr₁ tr₂
        · intro _ _
          right
          cases o₁ <;> cases o₂ <;> (try dsimp only) <;> (try split) <;> (try split) <;> (try dsimp only)
          all_goals first
            | exact hd
            | exact hd.append _ _
            | simpa using hd.append [] _
            | simpa using hd.append _ []
        replace h : BRel p L _ _ (tr₁, o₁, w₁') (tr₂, o₂, w₂') := fun a b => (h a b).resolve_right hd
        cases o₁ <;> cases o₂
        all_goals first
          | (intro ht₁ ht₂; simp only [Term] at ht₁ ht₂; done)
          | exact absurd rfl (nf₁ _)
          | exact absurd rfl (nf₂ _)
          | (intro ht₁ ht₂; dsimp only at ht₁ ht₂; split at ht₁ <;> simp only [Term] at ht₁ ht₂; done)
          | (intro ht₁ ht₂; dsimp only at ht₁ ht₂; split at ht₂ <;> simp only [Term] at ht₁ ht₂; done)
          | skip
        · intro ht₁ ht₂
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          exact .inl ⟨hp, V.flow ho (Flow.lsubst _ _ h₁), hwr⟩
        · rename_i a k c ps
          obtain rfl := hk₂ k c ps rfl
          dsimp only
          rw [if_pos rfl]
          obtain ⟨_, _, sm, fs, hs, hf, hps⟩ := ub₂.2.1
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₂') hσ (.cons (LValTy.variant (l := _) hs hf hps) he₂) hh'
          generalize eval H n p w₂' ((x, Val.variant k c ps) :: e₂) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          have ta : LValTy p a (t₁.lsubst p.recordDeep σ) := ub₁.2.1
          have tb := uh.2.1
          refine .of_brel <| brel_quiet_typed (hwr.trans hwh)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a' b' ea eb => by
              simp only [Res.ok.injEq] at ea eb
              subst ea eb
              exact V.tuple_covσ hlF ho' hcov (LValTy.flow ta (Flow.lsubst _ _ h₁))
                (LValTy.flow tb (Flow.lsubst _ _ h₂)))
            (fun h => by simp [Res.IsFail] at h) (fun hc' => hfh hc')
        · rename_i k c ps b
          obtain rfl := hk₁ k c ps rfl
          dsimp only
          rw [if_pos rfl]
          obtain ⟨_, _, sm, fs, hs, hf, hps⟩ := ub₁.2.1
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have ho' : ¬ Vis p L (ε₁.failLabel.inst (p.sd σ)) := fun hv => ho (Vis.failLabel_inst.mpr hv)
          have uh := uE (w := w₁') hσ (.cons (LValTy.variant (l := _) hs hf hps) he₁) hh'
          generalize eval H n p w₁' ((x, Val.variant k c ps) :: e₁) handler = rh at uh ⊢
          obtain ⟨th, oh, wh⟩ := rh
          obtain ⟨hph, hfh, hwh⟩ := confined (L := L) hh uh
            (fun hv => ho' (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hv))
          have tb : LValTy p b (t₁.lsubst p.recordDeep σ) := ub₂.2.1
          have ta := uh.2.1
          refine .of_brel <| brel_quiet_typed (hwh.symm.trans hwr)
            (by simp only [proj_append]; rw [hph, List.append_nil]; exact hp)
            (fun a' b' ea eb => by
              simp only [Res.ok.injEq] at ea eb
              subst ea eb
              exact V.tuple_covσ hlF ho' hcov (LValTy.flow ta (Flow.lsubst _ _ h₂))
                (LValTy.flow tb (Flow.lsubst _ _ h₁)))
            (fun hc' => hfh hc') (fun h => by simp [Res.IsFail] at h)
        · rename_i k₁ c₁ ps₁ k₂ c₂ ps₂
          obtain rfl := hk₁ k₁ c₁ ps₁ rfl
          obtain rfl := hk₂ k₂ c₂ ps₂ rfl
          dsimp only
          rw [if_pos rfl]
          try rw [if_pos rfl]
          obtain ⟨_, _, sm₁, fs₁, hs₁, hf₁, hps₁⟩ := ub₁.2.1
          obtain ⟨_, _, sm₂, fs₂, hs₂, hf₂, hps₂⟩ := ub₂.2.1
          obtain ⟨hp, ho, hwr⟩ := h trivial trivial
          simp only [OutRel] at ho
          have hcv : V p L ((LTy.record k₂ ε₁.failLabel).lsubst p.recordDeep σ)
              (.variant k₂ c₁ ps₁) (.variant k₂ c₂ ps₂) := by
            simp only [LTy.lsubst]
            by_cases hv : Vis p L (ε₁.failLabel.inst (p.sd σ))
            · obtain ⟨-, rfl, sm, fs, hs, hf, hvl⟩ := ho (Vis.failLabel_inst.mpr hv)
              exact .variant hs hf hvl
            · exact .hidden hv
          have hh'' := ihE hσ hE hΓ' hpc' hwr (.cons (LValTy.variant (l := _) hs₁ hf₁ hps₁) he₁)
            (.cons (LValTy.variant (l := _) hs₂ hf₂ hps₂) he₂) (.cons hcv hrel) hh'
          generalize eval H n p w₁' ((x, Val.variant k₂ c₁ ps₁) :: e₁) handler = rh₁ at hh'' ⊢
          generalize eval H n p w₂' ((x, Val.variant k₂ c₂ ps₂) :: e₂) handler = rh₂ at hh'' ⊢
          obtain ⟨th₁, oh₁, wh₁⟩ := rh₁
          obtain ⟨th₂, oh₂, wh₂⟩ := rh₂
          intro ht₁ ht₂
          rcases hh'' ht₁ ht₂ with ⟨hph, hoh, hwh⟩ | hdh
          · exact .inl ⟨by simp only [proj_append]; rw [hp, hph],
              OutRel.mono (fun _ _ h => V.flow h (Flow.lsubst _ _ h₂)) id hoh, hwh⟩
          · exact .inr (Diverged.prepend hp hdh)
      | @letTuple _ _ bs value body ts l s ε hv hflows hbs hb =>
        obtain ⟨v₁, v₂, hv₁, hv₂, hvv, hvu₁, hvu₂⟩ := latom_bothσ he₁ he₂ hrel hv
        have hli := latom_inPlace hΓ hv
        simp only [LTy.InPlace] at hli
        simp only [LTy.lsubst] at hvv hvu₁ hvu₂
        cases hvu₁ with
        | @tuple vs₁ _ _ hs₁ =>
          cases hvu₂ with
          | @tuple vs₂ _ _ hs₂ =>
            rw [LTy.lsubsts_eq] at hs₁ hs₂
            have hrv : VList p L ((ts.map (LTy.raise l)).map (LTy.lsubst p.recordDeep σ)) vs₁ vs₂ := by
              by_cases hvl : Vis p L (l.inst (p.sd σ))
              · cases hvv with
                | hidden h => exact absurd hvl h
                | tuple hvs =>
                  rw [LTy.lsubsts_eq] at hvs
                  exact VList.raiseσ l hvs
              · exact VList.hideσ hli.2 hvl (by rw [LValsTy.length hs₁]; simp)
                  (by rw [LValsTy.length hs₂]; simp)
            have hbs₁ := LValsTy.flowsσ (LValsTy.lsubst_raise l hs₁) hflows
            have hbs₂ := LValsTy.flowsσ (LValsTy.lsubst_raise l hs₂) hflows
            have hbsv := VList.flowsσ hrv hflows
            simp only [eval, hv₁, hv₂]
            rw [ite_eq_left (by rw [LValsTy.length hbs₁]; simp),
              ite_eq_left (by rw [LValsTy.length hbs₂]; simp)]
            have mk : ∀ {env : Env} {vs : List Val}, LEnvTy p env (Γ.lsub p.recordDeep σ) →
                LValsTy p vs ((bs.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
                LEnvTy p (bindParams bs vs env) (LCtx.lsub p.recordDeep σ (bs ++ Γ)) := by
              intro env vs henv hvs
              rw [LCtx.lsub_append]
              have := bindParams_ltyped (params := LCtx.lsub p.recordDeep σ bs)
                (by rw [LCtx.lsub_snd]; exact hvs) henv
              simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
            have hrel' : EnvRel p L (LCtx.lsub p.recordDeep σ (bs ++ Γ)) (bindParams bs vs₁ e₁)
                (bindParams bs vs₂ e₂) := by
              rw [LCtx.lsub_append]
              have := bindParams_rel (params := LCtx.lsub p.recordDeep σ bs)
                (by rw [LCtx.lsub_snd]; exact hbsv) hrel
              simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
            have hΓ' : LCtx.InPlace (bs ++ Γ) := fun q hq => by
              rcases List.mem_append.mp hq with hq | hq
              · exact hbs q hq
              · exact hΓ q hq
            exact ihE hσ hE hΓ' hpc hw (mk he₁ hbs₁) (mk he₂ hbs₂) hrel' hb
      | @loopList _ _ xs x xty st init body te ls ts tb ε hxs hx hxi hi hflows hst hb hbt hfe hcov =>
        obtain ⟨x₁, x₂, hx₁, hx₂, hxv, hxu₁, hxu₂⟩ := latom_bothσ he₁ he₂ hrel hxs
        obtain ⟨s₁, s₂, hs₁, hs₂, hsv, hsu₁, hsu₂⟩ := latoms_bothσ he₁ he₂ hrel hi
        have hli := latom_inPlace hΓ hxs
        simp only [LTy.InPlace] at hli
        have hpc' : (pc.join ls).PlainFree := LExpr.plainFree_join hpc hli.2

        -- The body's environments: the variable, the state, then the loop's.
        have mkEnv : ∀ {env : Env} {v : Val} {stv : List Val},
            LEnvTy p env (Γ.lsub p.recordDeep σ) → LValTy p v (xty.lsubst p.recordDeep σ) →
            LValsTy p stv ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
            LEnvTy p ((x, v) :: bindParams st stv env) (LCtx.lsub p.recordDeep σ ((x, xty) :: (st ++ Γ))) := by
          intro env v stv henv hv hst
          have hb' : LEnvTy p (bindParams st stv env) (LCtx.lsub p.recordDeep σ (st ++ Γ)) := by
            rw [LCtx.lsub_append]
            have := bindParams_ltyped (params := LCtx.lsub p.recordDeep σ st)
              (by rw [LCtx.lsub_snd]; exact hst) henv
            simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
          exact .cons hv hb'
        have mkRel : ∀ {v₁ v₂ : Val} {st₁ st₂ : List Val}, V p L (xty.lsubst p.recordDeep σ) v₁ v₂ →
            VList p L ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) st₁ st₂ →
            EnvRel p L (LCtx.lsub p.recordDeep σ ((x, xty) :: (st ++ Γ)))
              ((x, v₁) :: bindParams st st₁ e₁) ((x, v₂) :: bindParams st st₂ e₂) := by
          intro v₁ v₂ st₁ st₂ hv hst
          have hb' : EnvRel p L (LCtx.lsub p.recordDeep σ (st ++ Γ)) (bindParams st st₁ e₁)
              (bindParams st st₂ e₂) := by
            rw [LCtx.lsub_append]
            have := bindParams_rel (params := LCtx.lsub p.recordDeep σ st)
              (by rw [LCtx.lsub_snd]; exact hst) hrel
            simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
          exact .cons hv hb'
        have hΓ' : LCtx.InPlace ((x, xty) :: (st ++ Γ)) := fun q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · exact hxi
          · rcases List.mem_append.mp hq with hq | hq
            · exact hst q hq
            · exact hΓ q hq
        -- One iteration, unary, at the program counter of the body.
        have UB : ∀ {w : World H} {env : Env} {v : Val} {stv : List Val},
            LEnvTy p env (Γ.lsub p.recordDeep σ) → LValTy p v (xty.lsubst p.recordDeep σ) →
            LValsTy p stv ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) (ε.inst (p.sd σ)))
              (TupTy p ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)))
              (FailsAs p ((pc.join ls).inst (p.sd σ)) (ε.inst (p.sd σ))) w
              (eval H n p w ((x, v) :: bindParams st stv env) body) :=
          fun henv hv hst => UOk.mono (fun _ h => h)
            (fun _ h => tupTy_of (LValTy.flow h (Flow.lsubst _ _ hbt))) (FailCond.Le.refl _) (uE hσ (mkEnv henv hv hst) hb)
        -- What the iterations' failures decide (T-Loop's premise).
        have hfe' := hfe hh.oldLoop
        have hnf : ¬ Vis p L (ε.inst (p.sd σ)).failLabel → ¬ Vis p L ((pc.join ls).inst (p.sd σ)) :=
          fun hn h => hn (Vis.failLabel_inst.mpr (Vis.of_le (LExpr.le_inst _ hfe') h))
        have hst₁ := LValsTy.flowsσ hsu₁ hflows
        have hst₂ := LValsTy.flowsσ hsu₂ hflows
        have hstv := VList.flowsσ hsv hflows
        have hvar : ∀ {v : Val}, LValTy p v (te.lsubst p.recordDeep σ) → LValTy p v (xty.lsubst p.recordDeep σ) :=
          fun hv => LValTy.flow (LValTy.lsubst_raise hv) (Flow.lsubst _ _ hx)
        have UL : ∀ {w : World H} {env : Env} {items : List Val} {stv : List Val},
            LEnvTy p env (Γ.lsub p.recordDeep σ) → (∀ v ∈ items, LValTy p v (te.lsubst p.recordDeep σ)) →
            LValsTy p stv ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
            UOk (Allowed p ((pc.join ls).inst (p.sd σ)) (ε.inst (p.sd σ)))
              (TupTy p ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)))
              (FailsAs p ((pc.join ls).inst (p.sd σ)) (ε.inst (p.sd σ))) w
              (walkLoop (fun w v st' => eval H n p w ((x, v) :: bindParams st st' env) body) w stv items) :=
          fun henv hitems hstv => walkLoop_uok (fun _ _ _ hv hst => UB henv (hvar hv) hst) _ hstv hitems
        simp only [LTy.lsubst] at hxv hxu₁ hxu₂
        cases hxu₁ with
        | @list items₁ _ _ hl₁ =>
          cases hxu₂ with
          | @list items₂ _ _ hl₂ =>
            simp only [eval, sourceVals, hx₁, hx₂, hs₁, hs₂]
            rw [ite_eq_left (by rw [LValsTy.length hst₁]; simp),
              ite_eq_left (by rw [LValsTy.length hst₂]; simp)]
            by_cases hvl : Vis p L (ls.inst (p.sd σ))
            · cases hxv with
              | hidden h => exact absurd hvl h
              | list hall =>
                refine BRelR.mono (fun r₁ r₂ h => ?_) id (walkLoop_brelR (a := te.lsubst p.recordDeep σ)
                  (fun w₁ w₂ v₁ v₂ st₁ st₂ hw' hv hv₁ hv₂ hsr hst₁' hst₂' => BRelR.mono
                    (fun _ _ h => stRel_of (V.flow h (Flow.lsubst _ _ hbt))) id
                    (ihE hσ hE hΓ' hpc' hw' (mkEnv he₁ (hvar hv₁) hst₁') (mkEnv he₂ (hvar hv₂) hst₂')
                      (mkRel (V.flow (V.raiseσ ls hv) (Flow.lsubst _ _ hx)) hsr) hb))
                  (fun _ _ _ hv hst => UB he₁ (hvar hv) hst) (fun _ _ _ hv hst => UB he₂ (hvar hv) hst)
                  (fun hn w stv its hstv hits => confined (L := L) hh (UL he₁ hits hstv) (hnf hn))
                  (fun hn w stv its hstv hits => confined (L := L) hh (UL he₂ hits hstv) (hnf hn))
                  hw hstv hst₁ hst₂ hall hl₁ hl₂)
                obtain ⟨_, _, rfl, rfl, h⟩ := h
                simp only [LTy.lsubst, LTy.lsubsts_eq]
                exact .tuple h
            · exact .of_brel (brel_confined_typed hh hw (UL he₁ (LValsAll.mem hl₁) hst₁)
                (UL he₂ (LValsAll.mem hl₂) hst₂)
                (fun h => hvl (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
                (fun a b ha hb => by
                  obtain ⟨_, rfl, ha⟩ := ha
                  obtain ⟨_, rfl, hb⟩ := hb
                  simp only [LTy.lsubst, LTy.lsubsts_eq]
                  exact .tuple (VList.hide_covσ hli.2 hvl hcov ha hb)) id id)
      | @loopRange _ _ low high inc x xty st init body l₁ l₂ ts tb ε hlo hhi hx hxi hi hflows hst hb hbt hfe hcov =>
        obtain ⟨a₁, a₂, ha₁, ha₂, hav, hau₁, hau₂⟩ := latom_bothσ he₁ he₂ hrel hlo
        obtain ⟨b₁, b₂, hb₁, hb₂, hbv, hbu₁, hbu₂⟩ := latom_bothσ he₁ he₂ hrel hhi
        obtain ⟨s₁, s₂, hs₁, hs₂, hsv, hsu₁, hsu₂⟩ := latoms_bothσ he₁ he₂ hrel hi
        have hl₁i : l₁.PlainFree := latom_inPlace hΓ hlo
        have hl₂i : l₂.PlainFree := latom_inPlace hΓ hhi
        have hpc' : (pc.join (l₁.join l₂)).PlainFree :=
          LExpr.plainFree_join hpc (LExpr.plainFree_join hl₁i hl₂i)

        -- The body's environments: the variable, the state, then the loop's.
        have mkEnv : ∀ {env : Env} {v : Val} {stv : List Val},
            LEnvTy p env (Γ.lsub p.recordDeep σ) → LValTy p v (xty.lsubst p.recordDeep σ) →
            LValsTy p stv ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
            LEnvTy p ((x, v) :: bindParams st stv env) (LCtx.lsub p.recordDeep σ ((x, xty) :: (st ++ Γ))) := by
          intro env v stv henv hv hst
          have hb' : LEnvTy p (bindParams st stv env) (LCtx.lsub p.recordDeep σ (st ++ Γ)) := by
            rw [LCtx.lsub_append]
            have := bindParams_ltyped (params := LCtx.lsub p.recordDeep σ st)
              (by rw [LCtx.lsub_snd]; exact hst) henv
            simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
          exact .cons hv hb'
        have mkRel : ∀ {v₁ v₂ : Val} {st₁ st₂ : List Val}, V p L (xty.lsubst p.recordDeep σ) v₁ v₂ →
            VList p L ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) st₁ st₂ →
            EnvRel p L (LCtx.lsub p.recordDeep σ ((x, xty) :: (st ++ Γ)))
              ((x, v₁) :: bindParams st st₁ e₁) ((x, v₂) :: bindParams st st₂ e₂) := by
          intro v₁ v₂ st₁ st₂ hv hst
          have hb' : EnvRel p L (LCtx.lsub p.recordDeep σ (st ++ Γ)) (bindParams st st₁ e₁)
              (bindParams st st₂ e₂) := by
            rw [LCtx.lsub_append]
            have := bindParams_rel (params := LCtx.lsub p.recordDeep σ st)
              (by rw [LCtx.lsub_snd]; exact hst) hrel
            simpa [bindParams, LCtx.lsub, List.map_map, Function.comp_def] using this
          exact .cons hv hb'
        have hΓ' : LCtx.InPlace ((x, xty) :: (st ++ Γ)) := fun q hq => by
          rcases List.mem_cons.mp hq with rfl | hq
          · exact hxi
          · rcases List.mem_append.mp hq with hq | hq
            · exact hst q hq
            · exact hΓ q hq
        -- One iteration, unary, at the program counter of the body.
        have UB : ∀ {w : World H} {env : Env} {v : Val} {stv : List Val},
            LEnvTy p env (Γ.lsub p.recordDeep σ) → LValTy p v (xty.lsubst p.recordDeep σ) →
            LValsTy p stv ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
            UOk (Allowed p ((pc.join (l₁.join l₂)).inst (p.sd σ)) (ε.inst (p.sd σ)))
              (TupTy p ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)))
              (FailsAs p ((pc.join (l₁.join l₂)).inst (p.sd σ)) (ε.inst (p.sd σ))) w
              (eval H n p w ((x, v) :: bindParams st stv env) body) :=
          fun henv hv hst => UOk.mono (fun _ h => h)
            (fun _ h => tupTy_of (LValTy.flow h (Flow.lsubst _ _ hbt))) (FailCond.Le.refl _) (uE hσ (mkEnv henv hv hst) hb)
        -- What the iterations' failures decide (T-Loop's premise).
        have hfe' := hfe hh.oldLoop
        have hnf : ¬ Vis p L (ε.inst (p.sd σ)).failLabel → ¬ Vis p L ((pc.join (l₁.join l₂)).inst (p.sd σ)) :=
          fun hn h => hn (Vis.failLabel_inst.mpr (Vis.of_le (LExpr.le_inst _ hfe') h))
        have hst₁ := LValsTy.flowsσ hsu₁ hflows
        have hst₂ := LValsTy.flowsσ hsu₂ hflows
        have hstv := VList.flowsσ hsv hflows
        have hvar : ∀ {k : Int}, LValTy p (.int k) (xty.lsubst p.recordDeep σ) :=
          LValTy.flow (.int (l := ((l₁.join l₂).inst (p.sd σ)))) (Flow.lsubst (t₁ := .int (l₁.join l₂)) _ _ hx)
        have hints : ∀ {lo hi : Int} {i : Bool}, ∀ v ∈ rangeVals lo hi i, ∃ k, v = .int k := by
          intro lo hi i v hv
          simp only [rangeVals, List.mem_map] at hv
          obtain ⟨_, _, rfl⟩ := hv
          exact ⟨_, rfl⟩
        have UL : ∀ {w : World H} {env : Env} {items : List Val} {stv : List Val},
            LEnvTy p env (Γ.lsub p.recordDeep σ) → (∀ v ∈ items, ∃ k, v = .int k) →
            LValsTy p stv ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)) →
            UOk (Allowed p ((pc.join (l₁.join l₂)).inst (p.sd σ)) (ε.inst (p.sd σ)))
              (TupTy p ((st.map Prod.snd).map (LTy.lsubst p.recordDeep σ)))
              (FailsAs p ((pc.join (l₁.join l₂)).inst (p.sd σ)) (ε.inst (p.sd σ))) w
              (walkLoop (fun w v st' => eval H n p w ((x, v) :: bindParams st st' env) body) w stv items) :=
          fun henv hitems hstv => walkLoop_uok (a := xty.lsubst p.recordDeep σ)
            (fun _ _ _ hv hst => UB henv hv hst) _ hstv
            (fun v hv => by obtain ⟨k, rfl⟩ := hitems v hv; exact hvar)
        simp only [LTy.lsubst] at hav hau₁ hau₂ hbv hbu₁ hbu₂
        cases hau₁ with
        | @int lo₁ _ =>
          cases hau₂ with
          | @int lo₂ _ =>
            cases hbu₁ with
            | @int hi₁ _ =>
              cases hbu₂ with
              | @int hi₂ _ =>
                simp only [eval, sourceVals, ha₁, ha₂, hb₁, hb₂, hs₁, hs₂]
                rw [ite_eq_left (by rw [LValsTy.length hst₁]; simp),
                  ite_eq_left (by rw [LValsTy.length hst₂]; simp)]
                by_cases hvl : Vis p L ((l₁.join l₂).inst (p.sd σ))
                · have e₁ := V.eq_int hav (Vis.of_le (LExpr.le_inst _ (LExpr.le_join_left _ _)) hvl)
                  have e₂ := V.eq_int hbv (Vis.of_le (LExpr.inst_le_join_inst _ _ _) hvl)
                  cases e₁
                  cases e₂
                  have hall : VAll p L (xty.lsubst p.recordDeep σ) (rangeVals lo₁ hi₁ inc)
                      (rangeVals lo₁ hi₁ inc) := by
                    have := VAll.refl_int (p := p) (L := L) (l := (l₁.join l₂).inst (p.sd σ))
                      (xs := rangeVals lo₁ hi₁ inc) (hints (lo := lo₁) (hi := hi₁) (i := inc))
                    exact VAll.flow this (Flow.lsubst (t₁ := .int (l₁.join l₂)) _ _ hx)
                  have hlv : LValsAll p (rangeVals lo₁ hi₁ inc) (xty.lsubst p.recordDeep σ) := by
                    have : ∀ (xs : List Val), (∀ v ∈ xs, ∃ k, v = .int k) →
                        LValsAll p xs (xty.lsubst p.recordDeep σ) := by
                      intro xs hxs
                      induction xs with
                      | nil => exact .nil
                      | cons v vs ih =>
                        obtain ⟨k, rfl⟩ := hxs _ List.mem_cons_self
                        exact .cons hvar (ih (fun u hu => hxs u (List.mem_cons_of_mem _ hu)))
                    exact this _ (hints (lo := lo₁) (hi := hi₁) (i := inc))
                  refine BRelR.mono (fun r₁ r₂ h => ?_) id (walkLoop_brelR (a := xty.lsubst p.recordDeep σ)
                    (fun w₁ w₂ v₁ v₂ st₁ st₂ hw' hv hv₁ hv₂ hsr hst₁' hst₂' => BRelR.mono
                      (fun _ _ h => stRel_of (V.flow h (Flow.lsubst _ _ hbt))) id
                      (ihE hσ hE hΓ' hpc' hw' (mkEnv he₁ hv₁ hst₁') (mkEnv he₂ hv₂ hst₂') (mkRel hv hsr) hb))
                    (fun _ _ _ hv hst => UB he₁ hv hst) (fun _ _ _ hv hst => UB he₂ hv hst)
                    (fun hn w stv its hstv hits => confined (L := L) hh
                      (walkLoop_uok (fun _ _ _ hv hst => UB he₁ hv hst) _ hstv hits) (hnf hn))
                    (fun hn w stv its hstv hits => confined (L := L) hh
                      (walkLoop_uok (fun _ _ _ hv hst => UB he₂ hv hst) _ hstv hits) (hnf hn))
                    hw hstv hst₁ hst₂ hall hlv hlv)
                  obtain ⟨_, _, rfl, rfl, h⟩ := h
                  simp only [LTy.lsubst, LTy.lsubsts_eq]
                  exact .tuple h
                · exact .of_brel (brel_confined_typed hh hw (UL he₁ hints hst₁) (UL he₂ hints hst₂)
                    (fun h => hvl (Vis.of_le (LExpr.inst_le_join_inst _ _ _) h))
                    (fun a b ha hb => by
                      obtain ⟨_, rfl, ha⟩ := ha
                      obtain ⟨_, rfl, hb⟩ := hb
                      simp only [LTy.lsubst, LTy.lsubsts_eq]
                      exact .tuple (VList.hide_covσ (LExpr.plainFree_join hl₁i hl₂i) hvl hcov ha hb))
                    id id)

end Una.F1

namespace Una.F1

variable {p : Program} {L : Label}

variable {H : Host}

/-- **T6 for F1 without policies, at every ground instance**, with a stateful host: `t6_soundR`,
where no release is seen, so no run diverges at one. -/
theorem t6_sound (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF) (hlaw : H.Lawful)
    (n : Nat) :
    ∀ {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
      {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff}, GroundSubst σ → p.EqArgsOK σ E → Γ.InPlace →
      pc.PlainFree → WorldRel p L w₁ w₂ →
      LEnvTy p e₁ (Γ.lsub p.recordDeep σ) → LEnvTy p e₂ (Γ.lsub p.recordDeep σ) →
      EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂ → LExprTy p E Γ pc e t ε →
      BRel p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
        (eval H n p w₁ e₁ e) (eval H n p w₂ e₂ e) :=
  fun hσ hE hΓ hpc hw h₁ h₂ hrel he =>
    BRelR.to_brel_nil hpol ((t6_soundR hwf (.of_nil hpol) hh hlaw n).2 hσ hE hΓ hpc hw h₁ h₂ hrel he)

/-- **T6 with a stateful host** (M5): two runs of a body the labeled typing accepts, at a ground
instance of its type parameters, in environments that the observer at `L` cannot tell apart and
from worlds it cannot tell apart, show it the same events and related outcomes whenever both end
at one fuel, and leave worlds it cannot tell apart. -/
theorem t6_host (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF) (hlaw : H.Lawful)
    {σ : List (String × LTy)} {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {E : List String}
    {pc : LExpr} {e : Expr} {t : LTy} {ε : Eff} (hσ : GroundSubst σ) (hE : p.EqArgsOK σ E)
    (hΓ : Γ.InPlace) (hpc : pc.PlainFree) (hw : WorldRel p L w₁ w₂)
    (h₁ : LEnvTy p e₁ (Γ.lsub p.recordDeep σ)) (h₂ : LEnvTy p e₂ (Γ.lsub p.recordDeep σ))
    (hrel : EnvRel p L (Γ.lsub p.recordDeep σ) e₁ e₂) (he : LExprTy p E Γ pc e t ε) (n : Nat) :
    Term (eval H n p w₁ e₁ e).2.1 → Term (eval H n p w₂ e₂ e).2.1 →
      proj p L (eval H n p w₁ e₁ e).1 = proj p L (eval H n p w₂ e₂ e).1 ∧
        OutRel p L (V p L (t.lsubst p.recordDeep σ)) (Vis p L (ε.inst (p.sd σ)).failLabel)
          (eval H n p w₁ e₁ e).2.1 (eval H n p w₂ e₂ e).2.1 ∧
        WorldRel p L (eval H n p w₁ e₁ e).2.2 (eval H n p w₂ e₂ e).2.2 :=
  t6_sound hwf hpol hh hlaw n hσ hE hΓ hpc hw h₁ h₂ hrel he

theorem LEnvTy.lsubst_nil : ∀ {env : Env} {Γ : LCtx}, LEnvTy p env Γ →
    LEnvTy p env (Γ.lsub p.recordDeep [])
  | _, _, .nil => .nil
  | _, _, .cons hv henv => .cons (LValTy.flow hv (Flow.lsubst_nil _ _).2) (LEnvTy.lsubst_nil henv)

theorem EnvRel.lsubst_nil : ∀ {Γ : LCtx} {e₁ e₂ : Env}, EnvRel p L Γ e₁ e₂ →
    EnvRel p L (Γ.lsub p.recordDeep []) e₁ e₂
  | _, _, _, .nil => .nil
  | _, _, _, .cons hv hrel => .cons (V.flow hv (Flow.lsubst_nil _ _).2) (EnvRel.lsubst_nil hrel)

/-- **T6 with a stateful host, for a body that is not generic**, stated without instantiation. -/
theorem t6_host_ground (hwf : p.LWellFormed) (hpol : p.policies = []) (hh : p.HostWF)
    (hlaw : H.Lawful) {w₁ w₂ : World H} {e₁ e₂ : Env} {Γ : LCtx} {pc : LExpr} {e : Expr} {t : LTy}
    {ε : Eff} (hΓ : Γ.InPlace) (hpc : pc.PlainFree) (hw : WorldRel p L w₁ w₂) (h₁ : LEnvTy p e₁ Γ)
    (h₂ : LEnvTy p e₂ Γ) (hrel : EnvRel p L Γ e₁ e₂) (he : LExprTy p [] Γ pc e t ε) (n : Nat) :
    Term (eval H n p w₁ e₁ e).2.1 → Term (eval H n p w₂ e₂ e).2.1 →
      proj p L (eval H n p w₁ e₁ e).1 = proj p L (eval H n p w₂ e₂ e).1 ∧
        OutRel p L (V p L t) (Vis p L ε.failLabel) (eval H n p w₁ e₁ e).2.1 (eval H n p w₂ e₂ e).2.1 ∧
        WorldRel p L (eval H n p w₁ e₁ e).2.2 (eval H n p w₂ e₂ e).2.2 := by
  intro ht₁ ht₂
  obtain ⟨hp, ho, hwr⟩ := t6_host (σ := []) hwf hpol hh hlaw (fun _ h => nomatch h)
    (fun _ h => by simp at h) hΓ hpc hw (LEnvTy.lsubst_nil h₁) (LEnvTy.lsubst_nil h₂)
    (EnvRel.lsubst_nil hrel) he n ht₁ ht₂
  refine ⟨hp, OutRel.mono (fun _ _ h => V.flow h (Flow.lsubst_nil _ _).1) (fun hv => ?_) ho, hwr⟩
  exact Vis.of_le (LExpr.le_trans (failLabel_inst_le _ ε).1 (LExpr.inst_nil_le _)) hv

end Una.F1
