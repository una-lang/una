/-
Non-vacuity of #587 (ADR 0098): a policy whose `via` function signs its secret with a granted key,
on a program the whole-program check certifies.

- `sign` is of ADR 0098's shape: its only failure point is `Keys.hmac` with a literal key name, so
  B1 admits it with the host effect `Keys` (`progH_certified`).
- The same program is rejected when the key name comes from the secret, when the hmac call is under
  a branch, or when the `via` function performs another host operation (`#guard`s).
- Without the key granted, both runs fail with the same code; with it, both succeed: whether the
  call fails depends on the grants, never on the secret (`ungranted_alike`, `granted_alike`, and
  `sign_failCode` from S2).
-/
import UnaFormal.F1.CheckProgramSound
import UnaFormal.F1.HmacVia
import UnaFormal.F1.ReleaseExample

namespace Una.F1.HmacViaExample

open ReleaseExample Check

def secT : LTy := .text sec

/-- `sign(x: secret Text) -> Text = Keys.hmac("sig", x)`, the `via` function. -/
def sign : Fun where
  name := "sign"
  tparams := []
  eqParams := []
  params := [("x", secT)]
  result := .text .bot
  op := false
  host := HostFx.keys
  fails := true
  body := .letR "t" secT (.hmac (.text "sig") (.var "x")) (.atom (.var "t"))

/-- `policy stamp from secret Text to public Text via sign`. -/
def stamp : Policy := ⟨"stamp", secT, .text .bot, "sign"⟩

/-- `sign` as the `via` function of `stamp`. -/
def progH : Program := { prog with policies := [stamp], funs := [sign] }

#guard sign.hmacVia
#guard b1Ok progH
#guard progOk [] progH

-- `flowB` is defined by well-founded recursion: it is unsealed for the kernel to evaluate it.
unseal flowB flowsB in
/-- The whole-program check accepts `progH`. -/
theorem progH_certified : progH.LWellFormed ∧ progH.B1WellFormed ∧ progH.HostWF :=
  progOk_sound_nil (by rfl)

/-- The key name taken from the secret: outside ADR 0098's shape. -/
def signK : Fun :=
  { sign with
    params := [("x", secT), ("k", secT)]
    body := .letR "t" secT (.hmac (.var "k") (.var "x")) (.atom (.var "t")) }

/-- The hmac call under a branch on the secret. -/
def signB : Fun :=
  { sign with
    body := .letR "c" (.bool sec) (.eq (.var "x") (.text "a"))
      (.ite (.var "c") (.letR "t" secT (.hmac (.text "sig") (.var "x")) (.atom (.var "t")))
        (.atom (.var "x"))) }

/-- A `via` function that also reads a file. -/
def signR : Fun := { sign with host := { HostFx.keys with rd := true } }

#guard !signK.hmacVia
#guard !b1Ok { progH with funs := [signK] }
#guard !signB.hmacVia
#guard !b1Ok { progH with funs := [signB] }
#guard !signR.hmacVia
#guard !b1Ok { progH with funs := [signR] }

/-- A host that grants the key `sig`. -/
def Hk : Host :=
  { Host.unit with
    key := fun k => if k = "sig" then some "m" else none
    mac := fun m s => m ++ ":" ++ s }

def wU : World Host.unit := ⟨(), (), fun _ => none⟩
def wK : World Hk := ⟨(), (), fun _ => none⟩

def envT (s : String) : Env := [("x", .text s)]

/-- **Without the grant, two secrets fail alike.** -/
theorem ungranted_alike :
    eval Host.unit 4 progH wU (envT "a") sign.body = ([.keyUse "sig"], .failed hostFailure, wU) ∧
    eval Host.unit 4 progH wU (envT "b") sign.body = ([.keyUse "sig"], .failed hostFailure, wU) :=
  ⟨rfl, rfl⟩

/-- **With the grant, two secrets both succeed**; only the value depends on the secret. -/
theorem granted_alike :
    eval Hk 4 progH wK (envT "a") sign.body = ([.keyUse "sig"], .ok (.text "m:a"), wK) ∧
    eval Hk 4 progH wK (envT "b") sign.body = ([.keyUse "sig"], .ok (.text "m:b"), wK) :=
  ⟨rfl, rfl⟩

/-- S2 at `sign`: under any host, two environments that both end give the same failure code. -/
theorem sign_failCode {H : Host} (n : Nat) (w₁ w₂ : World H) (env₁ env₂ : Env)
    (h₁ : (eval H n progH w₁ env₁ sign.body).2.1.Ends)
    (h₂ : (eval H n progH w₂ env₂ sign.body).2.1.Ends) :
    (eval H n progH w₁ env₁ sign.body).2.1.failCode =
      (eval H n progH w₂ env₂ sign.body).2.1.failCode :=
  hmacOnly_failCode progH n w₁ w₂ env₁ env₂ _ (Fun.hmacVia_sound (by rfl)).2.2.2.2 h₁ h₂

end Una.F1.HmacViaExample
