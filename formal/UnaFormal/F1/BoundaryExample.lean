/-
Non-vacuity of #562, ADR 0093's authority confinement, in the shapes of B10 to B12
(`docs/plan/formal-track-boundary.md` §4). A rejected case has a concrete run whose trace holds an
event outside the ceiling, so, by `body_confined` and `apply_confined`, no candidate set that holds
the boundary's code passes `confinedB`. An accepted case passes it, and its run's events are
allowed.

1. **B10**: `vendor.tiled` writes, ceiling `Files[read]`: rejected, the run writes.
   `vendor.thumbnail`, pure: accepted.
2. **B4**: `vendor.tiled` as a value, applied by an outside higher-order function: rejected, the
   application writes.
3. **B5**: the handler `vendor.fire` of `Hook.fire`, declared in the boundary and installed outside,
   requests, ceiling empty: rejected, the run requests.
4. **A1**: `vendor.process(f)` applies `f`, and the caller passes a lambda that writes, ceiling
   `Files[read]`: the argument is not within the ceiling, and the run writes.
5. **B12**: the rule pack invokes `Records.contains`, whose outside handler reads, ceiling
   `Files[read]` and `Records.contains`: accepted, the run reads. With a handler that writes:
   rejected, the run writes.
6. **B11**: the plug-in invokes `Net.post`, whose handler does nothing, ceiling `Net.post` only:
   accepted, no event. With a handler that requests: rejected, the run requests.
-/
import UnaFormal.F1.Confine

namespace Una.F1.BoundaryExample

/-- A program of `funs`, with the `with` scopes' handlers (#538). -/
def mk (funs : List Fun) (handlerOf : List (String × String) := [])
    (opRep : List (String × String) := []) : Program where
  categories := []
  acceptsOp := Label.bot
  acceptsRead := Label.bot
  acceptsWrite := Label.bot
  acceptsReq := Label.bot
  readResult := Label.bot
  reqResult := Label.bot
  reqFloor := Label.bot
  fileState := Label.bot
  records := []
  policies := []
  funs := funs
  handlerOf := handlerOf
  opRep := opRep

/-- A function with no effect declared: F1's confinement reads the body, not the header (R7). -/
def fn (name : String) (params : List (String × LTy)) (body : Expr) : Fun where
  name := name
  tparams := []
  eqParams := []
  params := params
  result := .unit .bot
  op := false
  host := HostFx.none
  fails := false
  body := body

def w0 : World Host.unit := ⟨(), (), fun _ => none⟩

/-- `let u = e in u`. -/
def seq (e : Rhs) : Expr := .letR "u" (.unit .bot) e (.atom (.var "u"))

def txt : LTy := .text .bot
def unitT : LTy := .unit .bot

/-- `Files[read]`. -/
def cRead : Auth := { rd := true }

/-! ### 1. B10: a grant beyond the ceiling -/

/-- The vendored library's tiled mode: it writes a tile next to the photo. -/
def tiled : Fun := fn "vendor.tiled" [("path", txt)] (seq (.write (.var "path") (.text "tile")))

/-- Its plain mode: pure. -/
def thumbnail : Fun := fn "vendor.thumbnail" [("x", txt)] (.atom (.var "x"))

def p10 : Program := mk [tiled, thumbnail]

theorem tiled_writes :
    (eval Host.unit 20 p10 w0 (bindParams tiled.params [.text "a.jpg"] []) tiled.body).1 =
      [.write "a.jpg" "tile"] := rfl

theorem tiled_rejected (S : List String) (hS : "vendor.tiled" ∈ S) : confinedB p10 cRead S = false :=
  confinedB_refutes Host.unit_respTyped (fn := tiled) rfl (vs := [.text "a.jpg"]) (n := 20) (w := w0)
    (by rw [tiled_writes]; simp) (by simp [Auth.allows, cRead]) S hS (by simp [Val.WithinAllIn, Val.WithinIn])

theorem thumbnail_accepted : confinedB p10 cRead ["vendor.thumbnail"] = true := by decide

/-! ### 2. B4: the boundary's function as a value -/

/-- An outside higher-order function: it applies what it is given. -/
def apply1 : Fun := fn "app.apply1" [("f", .fn [txt] unitT false HostFx.none false .bot), ("x", txt)]
  (seq (.apply (.var "f") [.var "x"]))

def p4 : Program := mk [tiled, apply1]

/-- The value `funRef vendor.tiled` gives. -/
def tiledValue : Val := .closure tiled.params tiled.body []

theorem funRef_gives : (evalRhs Host.unit 20 p4 w0 [] (.funRef "vendor.tiled" [])).2.1 = .ok tiledValue :=
  rfl

theorem applied_writes : (applyWith (eval Host.unit 20 p4) w0 tiledValue [.text "a.jpg"]).1 =
    [.write "a.jpg" "tile"] := rfl

/-- Wherever the value is applied, the application writes, so no candidate set holding
`vendor.tiled` passes: the reference is charged as a call into the boundary. -/
theorem value_rejected (S : List String) (hS : "vendor.tiled" ∈ S) : confinedB p4 cRead S = false := by
  cases h : confinedB p4 cRead S
  · rfl
  · have hv := funRef_within (confinedB_sound h) (f := "vendor.tiled") (fn := tiled) rfl hS [] trivial
    have := apply_confined Host.unit_respTyped (confinedB_sound h) hv
      (vs := [.text "a.jpg"]) (by simp [Val.WithinAllIn, Val.WithinIn]) 20 w0 (.write "a.jpg" "tile")
      (by simp only [tiledValue] at applied_writes ⊢; rw [applied_writes]; simp)
    simp [Auth.allows, cRead] at this

/-! ### 3. B5: a boundary handler installed outside -/

/-- The plug-in's handler of `Hook.fire`: it sends a request. -/
def fire : Fun := fn "vendor.fire" [] (seq (.req "POST" [.text "https://hook"]))

/-- The application installs it and fires the hook. -/
def go : Fun := fn "app.go" []
  (.withH "Hook.fire" "vendor.fire" (seq (.invokeOp "Hook.fire" [])))

def p5 : Program := mk [fire, go] [("vendor.fire", "Hook.fire")] [("Hook.fire", "vendor.fire")]

theorem go_requests : (eval Host.unit 20 p5 w0 [] go.body).1 = [.req "POST" [.text "https://hook"]] :=
  rfl

theorem fire_requests : (eval Host.unit 20 p5 w0 (bindParams fire.params [] []) fire.body).1 =
    [.req "POST" [.text "https://hook"]] := rfl

/-- The empty ceiling. -/
def cNone : Auth := {}

theorem handler_rejected (S : List String) (hS : "vendor.fire" ∈ S) : confinedB p5 cNone S = false :=
  confinedB_refutes Host.unit_respTyped (fn := fire) rfl (vs := []) (n := 20) (w := w0)
    (by rw [fire_requests]; simp) (by simp [Auth.allows, cNone]) S hS (by simp [Val.WithinAllIn])

/-! ### 4. A1: a lambda given to the boundary -/

/-- The boundary applies the callback it is given. -/
def process : Fun := fn "vendor.process" [("f", .fn [unitT] unitT false HostFx.none false .bot)]
  (seq (.apply (.var "f") [.unit]))

def pA1 : Program := mk [process]

/-- The caller's lambda: it writes. -/
def onDone : Val := .closure [("x", unitT)] (seq (.write (.text "out.txt") (.text "done"))) []

theorem process_writes :
    (eval Host.unit 20 pA1 w0 (bindParams process.params [onDone] []) process.body).1 =
      [.write "out.txt" "done"] := rfl

/-- The functional argument is a grant: it is not within `Files[read]`, so the call's grant check
refuses it, whatever the candidate set. -/
theorem lambda_not_within (S : String → Prop) : ¬ Val.WithinIn S cRead onDone := by
  simp [onDone, Val.WithinIn, seq, Expr.within, Rhs.within, cRead]

/-! ### 5. B12: a handler adapter within the ceiling -/

/-- The rule pack's review: it asks whether a record exists. -/
def review : Fun := fn "rules.review" [] (seq (.invokeOp "Records.contains" []))

/-- The importer's read-only handler of `Records.contains`: it reads. -/
def readOnly : Fun := fn "app.ro.contains" [] (seq (.read (.text "records")))

/-- A writable handler of `Records.contains`: it writes. -/
def writable : Fun := fn "app.rw.contains" [] (seq (.write (.text "records") (.text "x")))

def cRecords : Auth := { rd := true, ops := ["Records.contains"] }

def p12 : Program :=
  mk [review, readOnly] [("app.ro.contains", "Records.contains")] [("Records.contains", "app.ro.contains")]

theorem adapter_accepted : confinedB p12 cRecords ["rules.review", "app.ro.contains"] = true := by decide

theorem adapter_reads : (eval Host.unit 20 p12 w0 [] review.body).1 = [.read "records"] := rfl

theorem adapter_allowed : ∀ ev ∈ (eval Host.unit 20 p12 w0 [] review.body).1, cRecords.allows ev :=
  body_confined Host.unit_respTyped (confinedB_sound adapter_accepted) (f := "rules.review")
    (fn := review) rfl (by simp) (vs := []) (by simp [Val.WithinAllIn]) 20 w0

def p12w : Program :=
  mk [review, writable] [("app.rw.contains", "Records.contains")] [("Records.contains", "app.rw.contains")]

theorem writable_writes : (eval Host.unit 20 p12w w0 (bindParams review.params [] []) review.body).1 =
    [.write "records" "x"] := rfl

theorem writable_rejected (S : List String) (hS : "rules.review" ∈ S) :
    confinedB p12w cRecords S = false :=
  confinedB_refutes Host.unit_respTyped (fn := review) rfl (vs := []) (n := 20) (w := w0)
    (by rw [writable_writes]; simp) (by simp [Auth.allows, cRecords]) S hS (by simp [Val.WithinAllIn])

/-! ### 6. B11: an empty ceiling -/

/-- The plug-in's alert hook: it posts through `Net.post`. -/
def renderAlert : Fun := fn "tpl.render_alert" [] (seq (.invokeOp "Net.post" []))

/-- The delivery's handler: it does nothing. -/
def noRequests : Fun := fn "app.none.post" [] (.atom .unit)

/-- A handler that posts. -/
def posting : Fun := fn "app.http.post" [] (seq (.req "POST" [.text "https://customer"]))

def cPost : Auth := { ops := ["Net.post"] }

def p11 : Program :=
  mk [renderAlert, noRequests] [("app.none.post", "Net.post")] [("Net.post", "app.none.post")]

theorem quiet_accepted : confinedB p11 cPost ["tpl.render_alert", "app.none.post"] = true := by decide

theorem quiet_runs : (eval Host.unit 20 p11 w0 [] renderAlert.body).1 = [] := rfl

def p11r : Program :=
  mk [renderAlert, posting] [("app.http.post", "Net.post")] [("Net.post", "app.http.post")]

theorem posting_requests :
    (eval Host.unit 20 p11r w0 (bindParams renderAlert.params [] []) renderAlert.body).1 =
      [.req "POST" [.text "https://customer"]] := rfl

theorem posting_rejected (S : List String) (hS : "tpl.render_alert" ∈ S) :
    confinedB p11r cPost S = false :=
  confinedB_refutes Host.unit_respTyped (fn := renderAlert) rfl (vs := []) (n := 20) (w := w0)
    (by rw [posting_requests]; simp) (by simp [Auth.allows, cPost]) S hS (by simp [Val.WithinAllIn])

end Una.F1.BoundaryExample
