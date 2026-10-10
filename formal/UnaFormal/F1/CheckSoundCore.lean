/-
M4 completion, part A1: **the checker of M2 is sound**. A right-hand side or a body the checker
accepts against a type has a labeled typing whose type flows into it; a function it accepts
passes T-Fun.
-/
import UnaFormal.F1.CheckSound

namespace Una.F1.Check

variable {p : Program} {E : List String}

/-- The flags of a function type the checker compares. -/
theorem flags_of {a b : Bool} (h : (!a || b) = true) : a = true → b = true := by
  intro ha; simp_all

mutual
  theorem checkRhs_sound {Γ : LCtx} {pc : LExpr} {ann : LTy} :
      ∀ {r : Rhs} {ε : Eff}, checkRhs p E Γ pc ann r = .ok ε →
        ∃ t, LRhsTy p E Γ pc r t ε ∧ Flow p.top t ann
    | .atom a, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok] at h
      obtain ⟨t, ht, _, hf, rfl⟩ := h
      exact ⟨t, .atom (atomTy_sound ht), flowTo_ok hf⟩
    | .prim pr args, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, some_ok] at h
      obtain ⟨ts, hts, rt, hrt, _, hf, rfl⟩ := h
      exact ⟨_, .prim (atomsTy_sound hts) (primTy_sound hrt), flowTo_ok hf⟩
    | .call f targs args, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨fn, hc, ts, hts, _, hfl, _, hb, _, hrb, _, hf, rfl⟩ := h
      obtain ⟨hfind, hnv, hlen, hfc, heq, hg⟩ := callable_sound hc
      exact ⟨_, .call hfind hnv hlen hfc heq hg (atomsTy_sound hts) (flowsB_sound _ _ hfl)
        (boundB_sound hb) (relBoundB_sound hrb), flowTo_ok hf⟩
    | .apply f args, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨ft, hft, h⟩ := h
      cases ft with
      | fn ps r o g fl lf =>
        simp only [bind_ok, need_ok, pure_ok] at h
        obtain ⟨ts, hts, _, hfl, _, hb, _, hf, rfl⟩ := h
        exact ⟨_, .apply (atomTy_sound hft) (atomsTy_sound hts) (flowsB_sound _ _ hfl)
          (boundB_sound hb), flowTo_ok hf⟩
      | _ => simp at h
    | .op line, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, hs, _, hle, _, hsf, _, hf, rfl⟩ := h
      exact ⟨_, .op (atomTy_sound ht) (isScalar_sound hs) (le_ok hle) (symFree_sound hsf),
        flowTo_ok hf⟩
    | .read path, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, hs, _, hle, _, hsf, _, hf, rfl⟩ := h
      exact ⟨_, .read (atomTy_sound ht) (isText_sound hs) (le_ok hle) (symFree_sound hsf),
        flowTo_ok hf⟩
    | .write path content, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, hs, c, hc, _, hcs, _, hle, _, hfs, _, hsf, _, hf, rfl⟩ := h
      refine ⟨_, .write (atomTy_sound ht) (isText_sound hs) (atomTy_sound hc) (isText_sound hcs)
        (le_ok hle) (fun ho => ?_) (symFree_sound hsf), flowTo_ok hf⟩
      simp only [ho, Bool.false_or] at hfs
      exact leB_sound hfs
    | .req m args, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨ts, hts, _, hps, _, hrec, _, hle, _, hsf, _, hf, rfl⟩ := h
      have hm : httpParams m = some (ts.map LTy.erase) := by
        split at hps
        · rename_i ps hp
          rw [hp, tysEq_sound _ _ hps]
        · simp at hps
      exact ⟨_, .req (atomsTy_sound hts) hm (httpRecordsB_sound hrec) (le_ok hle) (symFree_sound hsf),
        flowTo_ok hf⟩
    | .construct record fields, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok, some_ok] at h
      obtain ⟨rec, hrec, ts, hts, _, hfl, _, hf, rfl⟩ := h
      exact ⟨_, .construct hrec (atomsTy_sound hts) (flowsB_sound _ _ hfl), flowTo_ok hf⟩
    | .field value index, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨vt, hvt, h⟩ := h
      cases vt with
      | record record l =>
        simp only [bind_ok, pure_ok, some_ok] at h
        obtain ⟨rec, hrec, t, ht, _, hf, rfl⟩ := h
        exact ⟨_, .field (atomTy_sound hvt) hrec ht, flowTo_ok hf⟩
      | _ => simp at h
    | .list items, ε, h => by
      simp only [checkRhs] at h
      cases ann with
      | list t l =>
        simp only [bind_ok, pure_ok, need_ok, List.all_eq_true] at h
        obtain ⟨ts, hts, _, hall, _, hip, _, hf, rfl⟩ := h
        exact ⟨_, .list (atomsTy_sound hts) (fun u hu => flowB_sound _ _ (hall u hu))
          (inPlace_sound hip), flowTo_ok hf⟩
      | _ => simp at h
    | .none, ε, h => by
      simp only [checkRhs] at h
      cases ann with
      | option t l =>
        simp only [bind_ok, pure_ok, need_ok] at h
        obtain ⟨_, hip, rfl⟩ := h
        exact ⟨_, .none (inPlace_sound hip), .option (Flow.refl t) (LExpr.bot_le l)⟩
      | _ => simp at h
    | .some a, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok] at h
      obtain ⟨t, ht, _, hf, rfl⟩ := h
      exact ⟨_, .some (atomTy_sound ht), flowTo_ok hf⟩
    | .lambda params body, ε, h => by
      simp only [checkRhs] at h
      cases ann with
      | fn ps r o g fl l =>
        simp only [bind_ok, pure_ok, need_ok, List.all_eq_true, List.isEmpty_iff, Bool.and_eq_true]
          at h
        obtain ⟨_, hip, εb, hb, _, hfail, _, hrl, _, hfl, _, ⟨⟨ho, hg⟩, hfs⟩, rfl⟩ := h
        obtain ⟨tb, htb, hflow⟩ := checkExpr_sound hb
        exact ⟨_, .lambda htb (le_ok hfail) hrl (fun q hq => inPlace_sound (hip q hq)),
          .fn (flowsB_sound _ _ hfl) hflow (flags_of ho) (hostLeB_sound hg) (flags_of hfs)
            (LExpr.bot_le l)⟩
      | _ => simp at h
    | .funRef f targs, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok, List.isEmpty_iff] at h
      obtain ⟨fn, hc, _, hrl, _, hf, rfl⟩ := h
      obtain ⟨hfind, hnv, hlen, hfc, heq, hg⟩ := callable_sound hc
      exact ⟨_, .funRef hfind hnv hlen hfc heq hg hrl, flowTo_ok hf⟩
    | .declassify pol a, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok, some_ok] at h
      obtain ⟨po, hpo, fn, hfn, t, ht, _, hsrc, _, hle, _, hsf, _, hf, rfl⟩ := h
      exact ⟨_, .declassify hpo hfn (atomTy_sound ht) (flowTo_ok hsrc) (le_ok hle)
        (symFree_sound hsf), flowTo_ok hf⟩
    | .map xs f, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨xt, hxt, ft, hft, h⟩ := h
      cases xt <;> try (simp at h; done)
      cases ft <;> try (simp at h; done)
      rename_i ps tr o g fl lf
      rcases ps with _ | ⟨tp, _ | ⟨_, _⟩⟩ <;> try (simp at h; done)
      simp only [bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hel, _, hb, _, hf, rfl⟩ := h
      exact ⟨_, .map (atomTy_sound hxt) (atomTy_sound hft) (flowTo_ok hel) (boundB_sound hb),
        flowTo_ok hf⟩
    | .filter xs keep, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨xt, hxt, kt, hkt, h⟩ := h
      cases xt <;> try (simp at h; done)
      cases kt <;> try (simp at h; done)
      rename_i ps tr o g fl lf
      rcases ps with _ | ⟨tp, _ | ⟨_, _⟩⟩ <;> try (simp at h; done)
      cases tr <;> try (simp at h; done)
      simp only [bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hel, _, hb, _, hf, rfl⟩ := h
      exact ⟨_, .filter (atomTy_sound hxt) (atomTy_sound hkt) (flowTo_ok hel) (boundB_sound hb),
        flowTo_ok hf⟩
    | .firstWhere xs keep, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨xt, hxt, kt, hkt, h⟩ := h
      cases xt <;> try (simp at h; done)
      cases kt <;> try (simp at h; done)
      rename_i ps tr o g fl lf
      rcases ps with _ | ⟨tp, _ | ⟨_, _⟩⟩ <;> try (simp at h; done)
      cases tr <;> try (simp at h; done)
      simp only [bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hel, _, hb, _, hf, rfl⟩ := h
      exact ⟨_, .firstWhere (atomTy_sound hxt) (atomTy_sound hkt) (flowTo_ok hel)
        (boundB_sound hb), flowTo_ok hf⟩
    | .fold xs init f, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨xt, hxt, it, hit, ft, hft, h⟩ := h
      cases xt <;> try (simp at h; done)
      cases ft <;> try (simp at h; done)
      rename_i ps tr o g fl lf
      rcases ps with _ | ⟨ta, _ | ⟨tp, _ | ⟨_, _⟩⟩⟩ <;> try (simp at h; done)
      simp only [bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hel, _, hinit, _, hres, _, hb, _, hf, rfl⟩ := h
      exact ⟨_, .fold (atomTy_sound hxt) (atomTy_sound hit) (atomTy_sound hft) (flowTo_ok hel)
        (flowTo_ok hinit) (flowTo_ok hres) (boundB_sound hb), flowTo_ok hf⟩
    | .parse s, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, hs, _, hf, rfl⟩ := h
      exact ⟨_, .parse (atomTy_sound ht) (isText_sound hs), flowTo_ok hf⟩
    | .checked a _ _, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, hs, _, hle, _, hsf, _, hf, rfl⟩ := h
      exact ⟨_, .checked (atomTy_sound ht) (isScalar_sound hs) (le_ok hle) (symFree_sound hsf),
        flowTo_ok hf⟩
    | .eq a b, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok, some_ok] at h
      obtain ⟨t₁, h₁, t₂, h₂, t, hj, _, heq, _, hf, rfl⟩ := h
      obtain ⟨f₁, f₂⟩ := joinTy_sound (top := p.top) hj
      exact ⟨_, .eq (atomTy_sound h₁) (atomTy_sound h₂) f₁ f₂ heq, flowTo_ok hf⟩
    | .toText a, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, heq, _, hf, rfl⟩ := h
      exact ⟨_, .toText (atomTy_sound ht) heq, flowTo_ok hf⟩
    | .envGet n, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok, need_ok] at h
      obtain ⟨t, ht, _, hs, _, hle, _, hf, rfl⟩ := h
      exact ⟨_, .envGet (atomTy_sound ht) (isText_sound hs) (le_ok hle), flowTo_ok hf⟩
    | .tuple items, ε, h => by
      simp only [checkRhs, bind_ok, pure_ok] at h
      obtain ⟨ts, hts, _, hf, rfl⟩ := h
      exact ⟨_, .tuple (atomsTy_sound hts), flowTo_ok hf⟩
    | .append xs item, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨xt, hxt, h⟩ := h
      cases xt <;> try (simp at h; done)
      simp only [bind_ok, pure_ok] at h
      obtain ⟨it, hit, _, hfl, _, hf, rfl⟩ := h
      exact ⟨_, .append (atomTy_sound hxt) (atomTy_sound hit) (flowTo_ok hfl), flowTo_ok hf⟩
    | .set xs index item, ε, h => by
      simp only [checkRhs, bind_ok] at h
      obtain ⟨xt, hxt, it, hit, h⟩ := h
      cases xt <;> try (simp at h; done)
      cases it <;> try (simp at h; done)
      simp only [bind_ok, pure_ok] at h
      obtain ⟨_, hle, vt, hvt, _, hfl, _, hf, rfl⟩ := h
      exact ⟨_, .set (atomTy_sound hxt) (atomTy_sound hit) (le_ok hle) (atomTy_sound hvt)
        (flowTo_ok hfl), flowTo_ok hf⟩

  theorem checkExpr_sound {Γ : LCtx} {pc : LExpr} {s : LTy} :
      ∀ {e : Expr} {ε : Eff}, checkExpr p E Γ pc s e = .ok ε →
        ∃ t, LExprTy p E Γ pc e t ε ∧ Flow p.top t s
    | .atom a, ε, h => by
      simp only [checkExpr, bind_ok, pure_ok] at h
      obtain ⟨t, ht, _, hf, rfl⟩ := h
      exact ⟨t, .atom (atomTy_sound ht), flowTo_ok hf⟩
    | .letR x ann value body, ε, h => by
      simp only [checkExpr, bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, rfl⟩ := h
      obtain ⟨t, ht, hflow⟩ := checkRhs_sound h₁
      obtain ⟨s', hs', hfs⟩ := checkExpr_sound h₂
      exact ⟨_, .letR ht hflow hs' (inPlace_sound hip), hfs⟩
    | .letE x ann value body, ε, h => by
      simp only [checkExpr, bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, rfl⟩ := h
      obtain ⟨t, ht, hflow⟩ := checkExpr_sound h₁
      obtain ⟨s', hs', hfs⟩ := checkExpr_sound h₂
      exact ⟨_, .letE ht hflow hs' (inPlace_sound hip), hfs⟩
    | .ite cond thenE elseE, ε, h => by
      simp only [checkExpr, bind_ok] at h
      obtain ⟨ct, hct, h⟩ := h
      cases ct with
      | bool lc =>
        cases s
        case tuple ts l =>
          simp only [bind_ok, pure_ok, need_ok] at h
          obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hcov, rfl⟩ := h
          obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
          obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
          exact ⟨_, .iteT (atomTy_sound hct) ht₁ ht₂ f₁ f₂ (inPlaceAll_sound hip) (flowsB_sound _ _ hcov),
            .tuple (Flows.refl _) (LExpr.bot_le _)⟩
        all_goals
          simp only [bind_ok, pure_ok, need_ok] at h
          obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hf, rfl⟩ := h
          obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
          obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
          exact ⟨_, .ite (atomTy_sound hct) ht₁ ht₂ f₁ f₂ (inPlace_sound hip), flowTo_ok hf⟩
      | _ => simp at h
    | .caseOption scrutinee noneE x someE, ε, h => by
      simp only [checkExpr, bind_ok] at h
      obtain ⟨st, hst, h⟩ := h
      cases st with
      | option te ls =>
        cases s
        case tuple ts l =>
          simp only [bind_ok, pure_ok, need_ok] at h
          obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hcov, rfl⟩ := h
          obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
          obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
          exact ⟨_, .caseOptionT (atomTy_sound hst) ht₁ ht₂ f₁ f₂ (inPlaceAll_sound hip)
            (flowsB_sound _ _ hcov), .tuple (Flows.refl _) (LExpr.bot_le _)⟩
        all_goals
          simp only [bind_ok, pure_ok, need_ok] at h
          obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hf, rfl⟩ := h
          obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
          obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
          exact ⟨_, .caseOption (atomTy_sound hst) ht₁ ht₂ f₁ f₂ (inPlace_sound hip), flowTo_ok hf⟩
      | _ => simp at h
    | .caseResult scrutinee x okE y errE, ε, h => by
      simp only [checkExpr, bind_ok] at h
      obtain ⟨st, hst, h⟩ := h
      cases st with
      | result te ls =>
        cases s
        case tuple ts l =>
          simp only [bind_ok, pure_ok, need_ok] at h
          obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hcov, rfl⟩ := h
          obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
          obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
          exact ⟨_, .caseResultT (atomTy_sound hst) ht₁ ht₂ f₁ f₂ (inPlaceAll_sound hip)
            (flowsB_sound _ _ hcov), .tuple (Flows.refl _) (LExpr.bot_le _)⟩
        all_goals
          simp only [bind_ok, pure_ok, need_ok] at h
          obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hf, rfl⟩ := h
          obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
          obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
          exact ⟨_, .caseResult (atomTy_sound hst) ht₁ ht₂ f₁ f₂ (inPlace_sound hip), flowTo_ok hf⟩
      | _ => simp at h
    | .fail code, ε, h => by
      simp only [checkExpr, bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hle, _, hsf, _, hip, rfl⟩ := h
      exact ⟨s, .fail (le_ok hle) (symFree_sound hsf) (inPlace_sound hip), Flow.refl s⟩
    | .attempt body, ε, h => by
      simp only [checkExpr] at h
      cases s with
      | result t l =>
        simp only [bind_ok, pure_ok] at h
        obtain ⟨εb, hb, _, hle, rfl⟩ := h
        obtain ⟨tb, htb, hflow⟩ := checkExpr_sound hb
        exact ⟨_, .attempt htb, .result hflow (le_ok hle)⟩
      | _ => simp at h
    | .handle body x handler, ε, h => by
      cases s
      case tuple ts l =>
        simp only [checkExpr, bind_ok, pure_ok, need_ok] at h
        obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hcov, rfl⟩ := h
        obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
        obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
        exact ⟨_, .handleT ht₁ ht₂ f₁ f₂ (inPlaceAll_sound hip) (flowsB_sound _ _ hcov),
          .tuple (Flows.refl _) (LExpr.bot_le _)⟩
      all_goals
        simp only [checkExpr, bind_ok, pure_ok, need_ok] at h
        obtain ⟨_, hip, ε₁, h₁, ε₂, h₂, _, hf, rfl⟩ := h
        obtain ⟨t₁, ht₁, f₁⟩ := checkExpr_sound h₁
        obtain ⟨t₂, ht₂, f₂⟩ := checkExpr_sound h₂
        exact ⟨_, .handle ht₁ ht₂ f₁ f₂ (inPlace_sound hip), flowTo_ok hf⟩
    | .letTuple bs value body, ε, h => by
      simp only [checkExpr, bind_ok] at h
      obtain ⟨vt, hvt, h⟩ := h
      cases vt <;> try (simp at h; done)
      simp only [bind_ok, need_ok] at h
      obtain ⟨_, hfl, _, hip, hb⟩ := h
      obtain ⟨t, ht, hft⟩ := checkExpr_sound hb
      simp only [List.all_eq_true] at hip
      exact ⟨t, .letTuple (atomTy_sound hvt) (flowsB_sound _ _ hfl)
        (fun q hq => inPlace_sound (hip q hq)) ht, hft⟩
    | .loop (.list xs) x xty st init body, ε, h => by
      simp only [checkExpr, bind_ok] at h
      obtain ⟨xt, hxt, h⟩ := h
      cases xt <;> try (simp at h; done)
      simp only [bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hfx, _, hipx, ts, hts, _, hfl, _, hips, εb, hb, _, hfe, _, hcov, _, hf, rfl⟩ := h
      obtain ⟨tb, htb, hftb⟩ := checkExpr_sound hb
      simp only [List.all_eq_true] at hips
      exact ⟨_, .loopList (atomTy_sound hxt) (flowTo_ok hfx) (inPlace_sound hipx)
        (atomsTy_sound hts) (flowsB_sound _ _ hfl) (fun q hq => inPlace_sound (hips q hq)) htb hftb
        (fun ho => by simp only [ho, Bool.false_or] at hfe; exact leB_sound hfe)
        (flowsB_sound _ _ hcov),
        flowTo_ok hf⟩
    | .loop (.range low high inc) x xty st init body, ε, h => by
      simp only [checkExpr, bind_ok] at h
      obtain ⟨lt, hlt, ht, hht, h⟩ := h
      cases lt <;> try (simp at h; done)
      cases ht <;> try (simp at h; done)
      simp only [bind_ok, pure_ok, need_ok] at h
      obtain ⟨_, hfx, _, hipx, ts, hts, _, hfl, _, hips, εb, hb, _, hfe, _, hcov, _, hf, rfl⟩ := h
      obtain ⟨tb, htb, hftb⟩ := checkExpr_sound hb
      simp only [List.all_eq_true] at hips
      exact ⟨_, .loopRange (atomTy_sound hlt) (atomTy_sound hht) (flowTo_ok hfx) (inPlace_sound hipx)
        (atomsTy_sound hts) (flowsB_sound _ _ hfl) (fun q hq => inPlace_sound (hips q hq)) htb hftb
        (fun ho => by simp only [ho, Bool.false_or] at hfe; exact leB_sound hfe)
        (flowsB_sound _ _ hcov),
        flowTo_ok hf⟩
end

/-- **A function the checker accepts passes T-Fun.** -/
theorem checkFun_sound {fn : Fun} (h : checkFun p fn = .ok ()) : p.FunOk fn := by
  simp only [checkFun, bind_ok, need_ok, List.all_eq_true, List.contains_iff_mem] at h
  obtain ⟨ε, hb, _, ho, _, hg, _, hf, _, hfail, hr⟩ := h
  obtain ⟨t, ht, hflow⟩ := checkExpr_sound hb
  exact ⟨t, ε, ht, hflow, flags_of ho, hostLeB_sound hg, flags_of hf, le_ok hfail,
    fun x hx => by simpa using hr x hx⟩

theorem exprPure_sound (e : Expr) (h : exprPure e = true) : e.Pure := by
  match e, h with
  | .atom _, _ => trivial
  | .letR _ _ v b, h =>
    simp only [exprPure, Bool.and_eq_true] at h
    obtain ⟨hv, hb⟩ := h
    refine ⟨?_, exprPure_sound b hb⟩
    cases v <;> first | trivial | exact absurd hv (by simp [rhsPure])
  | .letE _ _ v b, h =>
    simp only [exprPure, Bool.and_eq_true] at h
    exact ⟨exprPure_sound v h.1, exprPure_sound b h.2⟩
  | .ite _ t e, h =>
    simp only [exprPure, Bool.and_eq_true] at h
    exact ⟨exprPure_sound t h.1, exprPure_sound e h.2⟩
  | .caseOption _ t _ e, h =>
    simp only [exprPure, Bool.and_eq_true] at h
    exact ⟨exprPure_sound t h.1, exprPure_sound e h.2⟩
  | .caseResult _ _ t _ e, h =>
    simp only [exprPure, Bool.and_eq_true] at h
    exact ⟨exprPure_sound t h.1, exprPure_sound e h.2⟩
  | .handle t _ e, h =>
    simp only [exprPure, Bool.and_eq_true] at h
    exact ⟨exprPure_sound t h.1, exprPure_sound e h.2⟩
  | .attempt b, h =>
    have hb : exprPure b = true := by simpa [exprPure] using h
    exact exprPure_sound b hb
  | .fail _, h | .letTuple _ _ _, h | .loop _ _ _ _ _ _, h => simp [exprPure] at h
termination_by sizeOf e

theorem exprPureOrFails_sound (e : Expr) (h : exprPureOrFails e = true) : e.PureOrFails := by
  match e, h with
  | .atom _, _ => trivial
  | .letR _ _ v b, h =>
    simp only [exprPureOrFails, Bool.and_eq_true] at h
    obtain ⟨hv, hb⟩ := h
    refine ⟨?_, exprPureOrFails_sound b hb⟩
    cases v <;> first | trivial | exact absurd hv (by simp [rhsPureOrFails, rhsPure])
  | .letE _ _ v b, h =>
    simp only [exprPureOrFails, Bool.and_eq_true] at h
    exact ⟨exprPureOrFails_sound v h.1, exprPureOrFails_sound b h.2⟩
  | .ite _ t e, h =>
    simp only [exprPureOrFails, Bool.and_eq_true] at h
    exact ⟨exprPureOrFails_sound t h.1, exprPureOrFails_sound e h.2⟩
  | .caseOption _ t _ e, h =>
    simp only [exprPureOrFails, Bool.and_eq_true] at h
    exact ⟨exprPureOrFails_sound t h.1, exprPureOrFails_sound e h.2⟩
  | .caseResult _ _ t _ e, h =>
    simp only [exprPureOrFails, Bool.and_eq_true] at h
    exact ⟨exprPureOrFails_sound t h.1, exprPureOrFails_sound e h.2⟩
  | .handle t _ e, h =>
    simp only [exprPureOrFails, Bool.and_eq_true] at h
    exact ⟨exprPureOrFails_sound t h.1, exprPureOrFails_sound e h.2⟩
  | .attempt b, h =>
    have hb : exprPureOrFails b = true := by simpa [exprPureOrFails] using h
    exact exprPureOrFails_sound b hb
  | .fail _, _ => trivial
  | .letTuple _ _ _, h | .loop _ _ _ _ _ _, h => simp [exprPureOrFails] at h
termination_by sizeOf e

/-- **A program whose policies the checker accepts is within B1's restrictions.** -/
theorem b1Ok_sound (h : b1Ok p = true) : p.B1WellFormed := by
  simp only [b1Ok, List.all_eq_true] at h
  refine ⟨fun po hpo => ?_, fun po hpo => ?_, fun po hpo => ?_⟩ <;> have := h po hpo <;>
    simp only [Bool.and_eq_true] at this
  · exact isScalar_sound this.1.1.1
  · exact ⟨ground_sound this.1.1.2, isScalar_sound this.1.2⟩
  · obtain ⟨_, hv⟩ := this
    split at hv
    · rename_i fn hfn
      simp only [Bool.and_eq_true, Bool.not_eq_true', Bool.or_eq_true] at hv
      refine ⟨fn, hfn, hv.1.1.1, hv.1.1.2, exprPureOrFails_sound _ hv.1.2, fun hf => ?_⟩
      rcases hv.2 with h | h
      · rw [hf] at h; exact absurd h (by simp)
      · exact exprPure_sound _ h
    · exact absurd hv (by simp)

theorem checkB1_sound (h : checkB1 p = .ok ()) : p.B1WellFormed := by
  unfold checkB1 at h
  simp only [bind_ok] at h
  obtain ⟨_, _, h⟩ := h
  exact b1Ok_sound (need_ok.mp h)

end Una.F1.Check
