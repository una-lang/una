/-
M4 completion, part A2: **the whole-program check is sound** (`progOk_sound`). A program `P`
that passes `progOk` is accepted by the labeled typing (`Program.LWellFormed`) and is within
option B1's restrictions (`Program.B1WellFormed`), with the host's labels ordered as `t6` needs.
The functions checked at their instances (part A3) enter as a named hypothesis, and with none
the theorem has no hypothesis left (`progOk_sound_nil`).

`reach_closed` is the formal side of "no accept without a theorem": in a program the labeled
typing accepts, every function a typed body calls or takes as a value is a function of the
program that passes T-Fun.
-/
import UnaFormal.F1.CheckProgram
import UnaFormal.F1.CheckSoundCore
import UnaFormal.F1.Erase
import UnaFormal.F1.Binary

namespace Una.F1.Check


mutual
  theorem tyClosed_sound : ∀ (t : Ty), tyClosed t = true → ∀ σ, t.subst σ = t
    | .int, _, _ | .bool, _, _ | .text, _, _ | .unit, _, _ | .record _, _, _ => rfl
    | .list t, h, σ | .option t, h, σ => by
      simp only [tyClosed] at h; simp only [Ty.subst, tyClosed_sound t h σ]
    | .result t e, h, σ => by
      simp only [tyClosed, Bool.and_eq_true] at h
      simp only [Ty.subst, tyClosed_sound t h.1 σ, tyClosed_sound e h.2 σ]
    | .fn ps r, h, σ => by
      simp only [tyClosed, Bool.and_eq_true] at h
      simp only [Ty.subst, tysClosed_sound ps h.1 σ, tyClosed_sound r h.2 σ]
    | .var _, h, _ => by simp [tyClosed] at h
    | .tuple ts, h, σ => by
      simp only [tyClosed] at h; simp only [Ty.subst, tysClosed_sound ts h σ]

  theorem tysClosed_sound : ∀ (ts : List Ty), tysClosed ts = true → ∀ σ, Ty.substs σ ts = ts
    | [], _, _ => rfl
    | t :: ts, h, σ => by
      simp only [tysClosed, Bool.and_eq_true] at h
      simp only [Ty.substs, tyClosed_sound t h.1 σ, tysClosed_sound ts h.2 σ]
end

theorem labelCats_sound {cats : List String} {l : Label} (h : labelCats cats l = true) :
    ∀ c ∈ l.cats, c ∈ cats := by
  simpa [labelCats, List.all_eq_true] using h

mutual
  theorem tyCats_sound {cats : List String} : ∀ (t : LTy), tyCats cats t = true → t.CatsIn cats
    | .int _, h | .bool _, h | .text _, h | .unit _, h | .record _ _, h | .var _ _, h => by
      simp only [tyCats] at h; exact labelCats_sound h
    | .list t _, h | .option t _, h => by
      simp only [tyCats, Bool.and_eq_true] at h
      exact ⟨tyCats_sound t h.1, labelCats_sound h.2⟩
    | .result t e _, h => by
      simp only [tyCats, Bool.and_eq_true] at h
      exact ⟨tyCats_sound t h.1.1, tyCats_sound e h.1.2, labelCats_sound h.2⟩
    | .fn ps r _ _ _ _, h => by
      simp only [tyCats, Bool.and_eq_true] at h
      exact ⟨tysCats_sound ps h.1.1, tyCats_sound r h.1.2, labelCats_sound h.2⟩
    | .tuple ts _, h => by
      simp only [tyCats, Bool.and_eq_true] at h
      exact ⟨tysCats_sound ts h.1, labelCats_sound h.2⟩

  theorem tysCats_sound {cats : List String} :
      ∀ (ts : List LTy), tysCats cats ts = true → LTy.CatsInAll cats ts
    | [], _ => trivial
    | t :: ts, h => by
      simp only [tysCats, Bool.and_eq_true] at h
      exact ⟨tyCats_sound t h.1, tysCats_sound ts h.2⟩
end

theorem catsDeclared_sound {p : Program} (h : catsDeclared p = true) : p.CatsDeclared := by
  simp only [catsDeclared, Bool.and_eq_true, List.all_eq_true] at h
  obtain ⟨⟨⟨⟨⟨⟨⟨⟨⟨⟨ho, hr⟩, hw⟩, hq⟩, hrr⟩, hqr⟩, hqf⟩, hfs⟩, hf⟩, hrec⟩, hpo⟩ := h
  refine ⟨labelCats_sound ho, labelCats_sound hr, labelCats_sound hw, labelCats_sound hq,
    labelCats_sound hrr, labelCats_sound hqr, labelCats_sound hqf, labelCats_sound hfs,
    fun fn hfn => ?_,
    fun rec hrc t ht => tyCats_sound t (hrec rec hrc t ht), fun po hp => ?_⟩
  · obtain ⟨h₁, h₂⟩ := hf fn hfn
    exact ⟨fun q hq => tyCats_sound _ (h₁ q hq), tyCats_sound _ h₂⟩
  · obtain ⟨h₁, h₂⟩ := hpo po hp
    exact ⟨tyCats_sound _ h₁, tyCats_sound _ h₂⟩

theorem isVia_true {p : Program} {f : String} (h : isVia p f = true) : p.IsVia f := by
  simp only [isVia, List.any_eq_true, beq_iff_eq] at h
  obtain ⟨po, hpo, he⟩ := h
  exact ⟨po, hpo, he⟩

theorem _root_.Una.F1.LTy.erase_lowerTop (t : LTy) : t.lowerTop.erase = t.erase := by
  cases t <;> rfl

/-- **Lowering `let` annotations keeps the structural typing** (#510): `ExprTy` reads no
annotation of a `let`, and `lowerLets` changes nothing else. -/
theorem _root_.Una.F1.ExprTy.of_lowerLets {p : Program} :
    ∀ (e : Expr) {ctx : Ctx} {s : Ty}, ExprTy p ctx e.lowerLets s → ExprTy p ctx e s
  | .letR x t v b, _, _, h => by
    cases h with | letR hv hb => exact .letR hv (ExprTy.of_lowerLets b hb)
  | .letE x t v b, _, _, h => by
    cases h with
    | letE hv hb => exact .letE (ExprTy.of_lowerLets v hv) (ExprTy.of_lowerLets b hb)
  | .ite c a b, _, _, h => by
    cases h with
    | ite hc ha hb => exact .ite hc (ExprTy.of_lowerLets a ha) (ExprTy.of_lowerLets b hb)
  | .caseOption sc n x sm, _, _, h => by
    cases h with
    | caseOption hs hn hm =>
      exact .caseOption hs (ExprTy.of_lowerLets n hn) (ExprTy.of_lowerLets sm hm)
  | .caseResult sc x ok y er, _, _, h => by
    cases h with
    | caseResult hs ho he =>
      exact .caseResult hs (ExprTy.of_lowerLets ok ho) (ExprTy.of_lowerLets er he)
  | .attempt b, _, _, h => by
    cases h with | attempt hb => exact .attempt (ExprTy.of_lowerLets b hb)
  | .handle b x hd, _, _, h => by
    cases h with
    | handle hb hh => exact .handle (ExprTy.of_lowerLets b hb) (ExprTy.of_lowerLets hd hh)
  | .letTuple bs v b, _, _, h => by
    cases h with
    | letTuple hv hl hb => exact .letTuple hv hl (ExprTy.of_lowerLets b hb)
  | .loop src x xt st init b, _, _, h => by
    cases h with
    | loop hs hi hl hb => exact .loop hs hi hl (ExprTy.of_lowerLets b hb)
  | .caseSum sc i xs a b, _, _, h => by
    cases h with
    | caseSum hs hsm hf hl ha hb =>
      exact .caseSum hs hsm hf hl (ExprTy.of_lowerLets a ha) (ExprTy.of_lowerLets b hb)
  | .attemptK k b, _, _, h => by
    cases h with | attemptK hb => exact .attemptK (ExprTy.of_lowerLets b hb)
  | .handleK k b x hd, _, _, h => by
    cases h with
    | handleK hb hh => exact .handleK (ExprTy.of_lowerLets b hb) (ExprTy.of_lowerLets hd hh)
  | .atom a, _, _, h => h
  | .fail c, _, _, h => h
  | .raise k c args, _, _, h => h
  | .unreachable, _, _, h => h
  | .withH o f b, _, _, h => by
    cases h with | withH hb => exact .withH (ExprTy.of_lowerLets b hb)

theorem viaTypedAt_sound {p : Program} {fn : Fun} {ps : List (String × LTy)} {body : Expr}
    (h : viaTypedAt p fn ps body = true) : ExprTy p (LCtx.erase ps) body fn.result.erase := by
  unfold viaTypedAt at h
  split at h
  · rename_i ε hc
    obtain ⟨t, ht, hflow⟩ := checkExpr_sound hc
    have hs := LExprTy.toStruct ht
    rw [Flow.erase hflow, LTy.erase_raise] at hs
    exact hs
  · exact absurd h (by simp)

/-- A `via` function the checker types against its result raised to `⊤` has its structural
typing, under its parameters or under their top labels at `⊥`. -/
theorem viaTyped_sound {p : Program} {fn : Fun} (h : viaTyped p fn = true) :
    ExprTy p (bindTys fn.params (fn.paramTys [])) fn.body fn.result.erase := by
  have he : LCtx.erase fn.params = bindTys fn.params (fn.paramTys []) := by
    have := params_erase fn.params []
    simp only [List.append_nil, LCtx.erase, List.map_nil] at this
    rw [paramTys_nil]
    simpa [LCtx.erase] using this
  have hl : LCtx.erase (fn.params.map fun q => (q.1, q.2.lowerTop)) = LCtx.erase fn.params := by
    simp [LCtx.erase, LTy.erase_lowerTop]
  unfold viaTyped at h
  rcases Bool.or_eq_true_iff.mp h with h | h
  · rcases Bool.or_eq_true_iff.mp h with h | h
    · rw [← he]; exact viaTypedAt_sound h
    · rw [← he, ← hl]; exact viaTypedAt_sound h
  · rw [← he, ← hl]; exact ExprTy.of_lowerLets _ (viaTypedAt_sound h)

theorem funOk_sound {p : Program} {fn : Fun} (h : funOk p fn = true) : p.FunOk fn := by
  unfold funOk at h
  split at h
  · rename_i u hc
    cases u
    exact checkFun_sound hc
  · exact absurd h (by simp)

/-- A policy the whole-program check accepts has the structural facts `LWellFormed` asks. -/
theorem policyOk_sound {p : Program} {po : Policy}
    (h : (((tyClosed po.src.erase = true ∧ tyClosed po.dst.erase = true) ∧
      inPlace po.src = true) ∧ inPlace po.dst = true) ∧
      (match p.find po.via with
      | some fn => tysEq (fn.paramTys []) [po.src.erase] && tyEq fn.result.erase po.dst.erase
      | none => false) = true) :
    (po.src.erase.Closed ∧ po.dst.erase.Closed ∧ ∃ fn, p.find po.via = some fn ∧
      fn.paramTys [] = [po.src.erase] ∧ fn.result.erase = po.dst.erase) ∧
    po.src.InPlace ∧ po.dst.InPlace := by
  obtain ⟨⟨⟨⟨hs, hd⟩, hps⟩, hpd⟩, hv⟩ := h
  refine ⟨⟨tyClosed_sound _ hs, tyClosed_sound _ hd, ?_⟩, inPlace_sound hps, inPlace_sound hpd⟩
  split at hv
  · rename_i fn hfn
    simp only [Bool.and_eq_true] at hv
    exact ⟨fn, hfn, tysEq_sound _ _ hv.1, tyEq_sound _ _ hv.2⟩
  · exact absurd hv (by simp)

theorem hostWfB_sound {p : Program} (h : hostWfB p = true) : p.HostWF := by
  simp only [hostWfB, Bool.and_eq_true, Bool.not_eq_true'] at h
  obtain ⟨⟨⟨⟨⟨⟨⟨⟨ho, hr⟩, hw⟩, hq⟩, hrr⟩, hqr⟩, hqf⟩, _⟩, hl⟩ := h
  exact ⟨ho, leB_sound hr, leB_sound hw, leB_sound hq, leB_sound hrr, leB_sound hqr, leB_sound hqf, hl⟩

/-- **The whole-program check is sound.** A program that passes `progOk cond` is accepted by
the labeled typing and is within B1's restrictions, and its host's labels are as
`t6_release_host` asks (`Program.HostWF`). The functions named in `cond`, which are
checked at their instances, enter by the named hypothesis `hA3`: their T-Fun is part A3's. -/
theorem progOk_sound {cond : List String} {p : Program} (h : progOk cond p = true)
    (hA3 : ∀ fn ∈ p.funs, fn.name ∈ cond → ¬ p.IsVia fn.name → p.FunOk fn) :
    p.LWellFormed ∧ p.B1WellFormed ∧ p.HostWF := by
  simp only [progOk, progDocOk, recordOk, sumOk, policyOk, Bool.and_eq_true, List.all_eq_true] at h
  obtain ⟨⟨⟨⟨⟨⟨⟨⟨hhost, hrec⟩, hsum⟩, hops⟩, hhand⟩, hpol⟩, hcats⟩, hb1⟩, hfuns⟩ := h
  have hfun := fun fn hfn => by
    have := hfuns fn hfn
    simp only [progFunOk, Bool.and_eq_true, List.all_eq_true] at this
    exact this
  -- The fields of `Program.LWellFormed`, in order: funs, records, recordDeep, policies,
  -- categories, recordsGround, eqScoped, sigsScoped, sums, disjoint, ops, handlers, inPlace.
  refine ⟨⟨?_, ?_, ?_, ?_, catsDeclared_sound hcats, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ⟨?_, ?_, ?_⟩⟩,
    b1Ok_sound hb1, hostWfB_sound hhost⟩
  · intro fn hfn
    obtain ⟨_, hv⟩ := hfun fn hfn
    by_cases hvia : isVia p fn.name = true
    · simp only [hvia, ite_true] at hv
      exact .inr ⟨isVia_true hvia, viaTyped_sound hv⟩
    · simp only [hvia, Bool.false_eq_true, ite_false] at hv
      simp only [Bool.or_eq_true, List.contains_iff_mem] at hv
      rcases hv with hc | hc
      · exact .inl (hA3 fn hfn hc (isVia_false (by simpa using hvia)))
      · exact .inl (funOk_sound hc)
  · intro rec hr t ht
    simp only [Record.fieldTys, List.mem_map] at ht
    obtain ⟨u, hu, rfl⟩ := ht
    exact tyClosed_sound _ (hrec rec hr u hu).1.2
  · intro rec hr t ht
    exact leB_sound (hrec rec hr t ht).2
  · intro po hp
    exact (policyOk_sound (hpol po hp)).1
  · intro rec hr t ht
    exact ground_sound (hrec rec hr t ht).1.1.1
  · intro fn hfn a ha
    have := (hfun fn hfn).1.1.1.1.1
    simpa using this a ha
  · intro fn hfn
    have := (hfun fn hfn).1
    exact ⟨fun q hq => scopedIn_sound (this.1.1.1.2 q hq), scopedIn_sound this.1.1.2⟩
  · -- Sums (#512): `sumOk` checks every field of every variant.
    intro sm hsm fs hfs t ht
    obtain ⟨⟨⟨⟨hg, hip⟩, hcl⟩, hcat⟩, hle⟩ := (hsum sm hsm).2 fs hfs t ht
    exact ⟨ground_sound hg, inPlace_sound hip, tyCats_sound _ hcat, leB_sound hle, tyClosed_sound _ hcl⟩
  · -- No record has a sum's name (`sumOk`), so a lookup of a name finds one kind at most.
    intro k
    cases hs : p.sum k with
    | none => exact .inr rfl
    | some sm =>
      left
      have hmem : sm ∈ p.sums := List.mem_of_find?_eq_some hs
      have hname : sm.name = k := by simpa using List.find?_some hs
      have := (hsum sm hmem).1
      rw [hname] at this
      simpa using this
  · -- Handlers of operations (#524): `opOk` checks the effect floor and the restrictions.
    intro q hq
    have h := hops q hq
    unfold opOk at h
    split at h
    · rename_i fn hfn
      simp only [Bool.and_eq_true, List.isEmpty_iff, Bool.not_eq_true', List.all_eq_true,
        Bool.or_eq_true, HostFx.isNone] at h
      obtain ⟨⟨⟨⟨⟨htp, hrels⟩, ⟨⟨hrd, hwr⟩, hrq⟩⟩, hop⟩, hsc⟩, hnv⟩ := h
      refine ⟨fn, hfn, htp, hrels, ?_, fun ho => ?_, fun r hr => isScalar_sound (hsc r hr),
        isVia_false hnv⟩
      · have hn : ∀ g : HostFx, g.rd = false → g.wr = false → g.rq = false → g = HostFx.none := by
          rintro ⟨_, _, _⟩ h1 h2 h3
          simp only at h1 h2 h3
          subst h1 h2 h3
          rfl
        exact hn _ hrd hwr hrq
      · rcases hop with h | h
        · simp [ho] at h
        · exact leB_sound h
    · simp at h
  · -- Handlers of `with` scopes (#538): `handlersOk`.
    intro o f hr
    unfold handlersOk at hhand
    have h := (List.all_eq_true.mp hhand) (o, f) (lookup_some_mem hr)
    simp only [Bool.and_eq_true, beq_iff_eq] at h
    obtain ⟨hfo, h⟩ := h
    refine ⟨hfo, ?_⟩
    split at h
    · rename_i fn acc hf hacc
      refine ⟨fn, acc, hf, hacc, fun g hg => ?_⟩
      have h' := (List.all_eq_true.mp h) (g, o) (lookup_some_mem hg)
      simp only [bne_self_eq_false, Bool.false_or] at h'
      split at h'
      · rename_i gn acc' hgn hacc'
        simp only [Bool.and_eq_true, beq_iff_eq] at h'
        obtain ⟨⟨⟨⟨⟨⟨hle, hfl⟩, hres⟩, hfails⟩, hraises⟩, htp⟩, hop⟩ := h'
        exact ⟨gn, acc', hgn, hacc', leB_sound hle, flowsB_sound _ _ hfl, flowB_sound _ _ hres,
          hfails, hraises, htp, hop⟩
      · simp at h'
    · simp at h
  · intro fn hfn
    have := (hfun fn hfn).1
    exact ⟨fun q hq => inPlace_sound (this.1.2 q hq), inPlace_sound this.2⟩
  · intro rec hr t ht
    exact inPlace_sound (hrec rec hr t ht).1.1.2
  · intro po hp
    exact (policyOk_sound (hpol po hp)).2

/-- With no function checked at its instances, the whole-program check needs no hypothesis. -/
theorem progOk_sound_nil {p : Program} (h : progOk [] p = true) :
    p.LWellFormed ∧ p.B1WellFormed ∧ p.HostWF :=
  progOk_sound h (fun _ _ hc => by simp at hc)

end Una.F1.Check

namespace Una.F1

variable {p : Program} {E : List String}

/-- A function `find` returns is one of the program's, by that name. -/
theorem Program.find_some {f : String} {fn : Fun} (h : p.find f = some fn) :
    fn ∈ p.funs ∧ fn.name = f := by
  unfold Program.find at h
  exact ⟨List.mem_of_find?_eq_some h, by simpa using List.find?_some h⟩

/-- A function a typed body calls, in a program the labeled typing accepts, passes T-Fun: the
typing finds it in the program, and does not call a `via` function. -/
theorem found_funOk (hwf : p.LWellFormed) {f : String} {fn : Fun} (hf : p.find f = some fn)
    (hnv : ¬ p.IsVia f) : p.FunOk fn := by
  obtain ⟨hmem, hname⟩ := Program.find_some hf
  rcases hwf.funs fn hmem with h | ⟨hv, _⟩
  · exact h
  · exact absurd (hname ▸ hv) hnv

mutual
  theorem rhs_reach (hwf : p.LWellFormed) :
      ∀ (r : Rhs) {Γ : LCtx} {pc : LExpr} {t : LTy} {ε : Eff}, LRhsTy p E Γ pc r t ε →
        ∀ f ∈ r.callees, ∃ fn, p.find f = some fn ∧ p.FunOk fn
    | .call f _ _, _, _, _, _, h, g, hg => by
      simp only [Rhs.callees, List.mem_singleton] at hg
      subst hg
      cases h with
      | call hf hnv => exact ⟨_, hf, found_funOk hwf hf hnv⟩
    | .funRef f _, _, _, _, _, h, g, hg => by
      simp only [Rhs.callees, List.mem_singleton] at hg
      subst hg
      cases h with
      | funRef hf hnv => exact ⟨_, hf, found_funOk hwf hf hnv⟩
    | .invoke f _, _, _, _, _, h, g, hg => by
      simp only [Rhs.callees, List.mem_singleton] at hg
      subst hg
      cases h with
      | invoke hf hacc =>
        obtain ⟨fn', hf', _, _, _, _, _, hnv⟩ := hwf.ops _ (lookup_some_mem hacc)
        exact ⟨_, hf, found_funOk hwf hf hnv⟩
    | .lambda _ b, _, _, _, _, h, g, hg => by
      simp only [Rhs.callees] at hg
      cases h with
      | lambda hb => exact expr_reach hwf b hb g hg
    | .variant _ _ _, _, _, _, _, _, _, hg
    | .atom _, _, _, _, _, _, _, hg | .prim _ _, _, _, _, _, _, _, hg
    | .apply _ _, _, _, _, _, _, _, hg | .op _, _, _, _, _, _, _, hg
    | .read _, _, _, _, _, _, _, hg | .write _ _, _, _, _, _, _, _, hg
    | .req _ _, _, _, _, _, _, _, hg | .construct _ _, _, _, _, _, _, _, hg
    | .field _ _, _, _, _, _, _, _, hg | .list _, _, _, _, _, _, _, hg
    | .none, _, _, _, _, _, _, hg | .some _, _, _, _, _, _, _, hg
    | .first _, _, _, _, _, _, _, hg | .invokeOp _ _, _, _, _, _, _, _, hg
    | .mkOk _, _, _, _, _, _, _, hg | .mkErr _, _, _, _, _, _, _, hg
    | .declassify _ _, _, _, _, _, _, _, hg | .map _ _, _, _, _, _, _, _, hg
    | .filter _ _, _, _, _, _, _, _, hg | .firstWhere _ _, _, _, _, _, _, _, hg
    | .fold _ _ _, _, _, _, _, _, _, hg | .parse _, _, _, _, _, _, _, hg
    | .eq _ _, _, _, _, _, _, _, hg | .tuple _, _, _, _, _, _, _, hg
    | .append _ _, _, _, _, _, _, _, hg | .set _ _ _, _, _, _, _, _, _, hg
    | .checked _ _ _, _, _, _, _, _, _, hg
    | .toText _, _, _, _, _, _, _, hg | .envGet _, _, _, _, _, _, _, hg => by
      simp [Rhs.callees] at hg

  theorem expr_reach (hwf : p.LWellFormed) :
      ∀ (e : Expr) {Γ : LCtx} {pc : LExpr} {t : LTy} {ε : Eff}, LExprTy p E Γ pc e t ε →
        ∀ f ∈ e.callees, ∃ fn, p.find f = some fn ∧ p.FunOk fn
    | .atom _, _, _, _, _, _, _, hg | .fail _, _, _, _, _, _, _, hg
    | .raise _ _ _, _, _, _, _, _, _, hg | .unreachable, _, _, _, _, _, _, hg => by
      simp [Expr.callees] at hg
    | .withH _ _ b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees] at hg
      cases h with
      | withH _ hb => exact expr_reach hwf b hb g hg
    | .letR _ _ v b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | letR hv _ hb =>
        rcases hg with hg | hg
        · exact rhs_reach hwf v hv g hg
        · exact expr_reach hwf b hb g hg
    | .letE _ _ v b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | letE hv _ hb =>
        rcases hg with hg | hg
        · exact expr_reach hwf v hv g hg
        · exact expr_reach hwf b hb g hg
    | .ite _ t e, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | ite _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
      | iteT _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
    | .caseOption _ t _ e, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | caseOption _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
      | caseOptionT _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
    | .caseResult _ _ t _ e, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | caseResult _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
      | caseResultT _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
    | .handle t _ e, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | handle ht _ he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
      | handleT ht _ he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
    | .handleK _ t _ e, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | handleK ht _ _ he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
      | handleKT ht _ _ he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
    | .caseSum _ _ _ t e, _, _, _, _, h, g, hg => by
      simp only [Expr.callees, List.mem_append] at hg
      cases h with
      | caseSum _ _ _ _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
      | caseSumT _ _ _ _ ht he =>
        rcases hg with hg | hg
        · exact expr_reach hwf t ht g hg
        · exact expr_reach hwf e he g hg
    | .attempt b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees] at hg
      cases h with
      | attempt hb => exact expr_reach hwf b hb g hg
    | .attemptK _ b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees] at hg
      cases h with
      | attemptK hb => exact expr_reach hwf b hb g hg
    | .letTuple _ _ b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees] at hg
      cases h with
      | letTuple _ _ _ hb => exact expr_reach hwf b hb g hg
    | .loop _ _ _ _ _ b, _, _, _, _, h, g, hg => by
      simp only [Expr.callees] at hg
      cases h with
      | loopList _ _ _ _ _ _ hb _ _ => exact expr_reach hwf b hb g hg
      | loopRange _ _ _ _ _ _ _ hb _ _ => exact expr_reach hwf b hb g hg
end

/-- **No accept without a theorem, formally.** In a program the labeled typing accepts, every
function a function of the program calls or takes as a value, lambdas included, is a function of
the program that passes T-Fun; by induction, so is everything it reaches. A `via` function
calls nothing (`Program.B1WellFormed`, `Expr.Pure`), and a handler's body is not a function of
F1: an operation is an event of the trace. So the reach set the re-check computes is closed in
the program it certifies. -/
theorem reach_closed (hwf : p.LWellFormed) {fn : Fun} (hok : p.FunOk fn) :
    ∀ f ∈ fn.body.callees, ∃ g, p.find f = some g ∧ p.FunOk g := by
  obtain ⟨_, _, ht, _⟩ := hok
  exact expr_reach hwf fn.body ht

end Una.F1
