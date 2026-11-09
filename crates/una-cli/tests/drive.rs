//! The interpreter's driver (ADR 0116 rule 3, `spec/26 §2`): `init` once, `step` once per tick,
//! then `on_stop`, stopped by `--stop-after-iterations`, with the `stop` event in the trace.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use una_interp::{
    Completion, Drive, DriveShape, EventOutcome, FileFault, Grant, Host, Limits, NotText, World,
};

/// A world that keeps the printed lines.
#[derive(Default)]
struct Lines(Vec<String>);

impl World for Lines {
    fn print(&mut self, line: &str) {
        self.0.push(line.to_owned());
    }
    /// A wall clock that reads one fixed moment, apart from any virtual one.
    fn now(&mut self) -> i64 {
        1234
    }
    fn env_var(&mut self, _name: &str) -> Result<Option<String>, NotText> {
        Ok(None)
    }
    fn read_file(&mut self, _path: &Path) -> Result<String, FileFault> {
        Err(FileFault::NotFound)
    }
    fn write_file(&mut self, _path: &Path, _content: &str) -> Result<(), FileFault> {
        Err(FileFault::NotFound)
    }
    fn canonical(&mut self, _path: &Path) -> Option<PathBuf> {
        None
    }
    fn is_link(&mut self, _path: &Path) -> bool {
        false
    }
}

/// A counter: each step adds one and prints the count, and `on_stop` prints the last one.
const COUNTER: &str = "module counter

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit count: Int, input: Tick) -> Unit
  uses @console: Console[print]
=
  count = count + 1
  @console.print(\"step {count}\")

pub on_stop(count: Int) -> Unit
  uses @console: Console[print]
=
  @console.print(\"stopped after {count}\")
";

/// The checked program of `source`, the module `counter` of a package of edition 0.4.
fn checked(name: &str, source: &str) -> una_core::Program {
    let dir = std::env::temp_dir().join(format!("una-drive-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("ita.una"),
        "ita counter\n  version 0.1.0\n  edition 0.4\n",
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/conformance/types/float_edition_0_4/ita.lock"),
        dir.join("ita.lock"),
    )
    .unwrap();
    std::fs::write(dir.join("src/counter.una"), source).unwrap();
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    checked.program
}

/// Drives `counter` of `program` with a tick of 1 ms on the real clock, stopping after
/// `iterations`.
fn drive(
    program: &una_core::Program,
    iterations: u64,
) -> (Result<una_interp::Run, DriveShape>, Vec<String>) {
    drive_on(program, iterations, 1, false)
}

/// Drives `counter` of `program` with a tick of `ticks` ms, on a virtual clock when
/// `virtual_time` holds, stopping after `iterations`.
fn drive_on(
    program: &una_core::Program,
    iterations: u64,
    ticks: u64,
    virtual_time: bool,
) -> (Result<una_interp::Run, DriveShape>, Vec<String>) {
    let host = Host {
        grants: vec![Grant::Console, Grant::Clock],
        keys: BTreeMap::new(),
        virtual_time,
        cost_clock: false,
    };
    let stack = una_interp::stack_bytes(10_000).unwrap();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn_scoped(scope, || {
                let mut lines = Lines::default();
                let run = una_interp::run_drive(
                    program,
                    "counter",
                    &mut lines,
                    &host,
                    Limits {
                        stop_after: Some(iterations),
                        ..Limits::depth(10_000)
                    },
                    Drive {
                        ticks,
                        budget: None,
                    },
                );
                (run, lines.0)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn the_driver_steps_until_the_stop_then_runs_on_stop() {
    let program = checked("counter", COUNTER);
    let (run, lines) = drive(&program, 3);
    let run = run.unwrap();
    assert!(
        matches!(run.completion, Completion::Ok(_)),
        "{:?}",
        run.completion
    );
    assert_eq!(
        lines,
        ["step 1", "step 2", "step 3", "stopped after 3"].map(str::to_owned)
    );
    // Three prints, the stop at the start of the fourth iteration, then `on_stop`'s print.
    let outcomes: Vec<bool> = run
        .trace
        .iter()
        .map(|event| matches!(event.outcome, EventOutcome::Stop))
        .collect();
    assert_eq!(outcomes, [false, false, false, true, false]);
    let json = una_interp::trace_json(&run.trace);
    assert!(
        json.contains(r#"{"seq":4,"outcome":{"kind":"stop"}}"#),
        "{json}"
    );
}

#[test]
fn a_module_without_step_is_a_shape_error() {
    let program = checked("no-step", "module counter\n\npub init() -> Int = 0\n");
    let (run, lines) = drive(&program, 1);
    assert_eq!(run.unwrap_err(), DriveShape::NoStep);
    assert!(lines.is_empty());
}

/// A step that prints its tick and the clock's reading.
const CLOCKED: &str = "module counter

import std.drive (Tick)

pub init() -> Int = 0

pub step(edit count: Int, input: Tick) -> Unit
  uses @console: Console[print]
  uses @clock: Clock[now]
=
  count = count + 1
  let now = @clock.now()
  @console.print(\"step {count} at {input.at} missed {input.missed} now {now}\")
";

#[test]
fn under_virtual_time_the_ticks_and_the_clock_are_exact_and_never_wait() {
    let program = checked("clocked", CLOCKED);
    let started = std::time::Instant::now();
    // A tick a day: on the real clock, three steps would take two days.
    let (run, lines) = drive_on(&program, 3, 86_400_000, true);
    assert!(started.elapsed() < std::time::Duration::from_secs(60));
    let run = run.unwrap();
    assert!(
        matches!(run.completion, Completion::Ok(_)),
        "{:?}",
        run.completion
    );
    assert_eq!(
        lines,
        [
            "step 1 at 0 missed 0 now 0",
            "step 2 at 86400000 missed 0 now 86400000",
            "step 3 at 172800000 missed 0 now 172800000",
        ]
        .map(str::to_owned)
    );
    // The same run gives the same trace: the clock's readings are in it.
    let (again, _) = drive_on(&program, 3, 86_400_000, true);
    assert_eq!(
        una_interp::trace_json(&run.trace),
        una_interp::trace_json(&again.unwrap().trace)
    );
}

#[test]
fn on_the_real_clock_the_clock_reads_the_wall_clock() {
    let program = checked("wall", CLOCKED);
    // A tick an hour: the one step is due at once, at tick 0, and no slow machine can make a
    // whole tick pass before it starts, so `at` and `missed` are 0 on any scheduler.
    let (run, lines) = drive_on(&program, 1, 3_600_000, false);
    assert!(matches!(run.unwrap().completion, Completion::Ok(_)));
    // `Clock.now` reads the world's wall clock, and the first tick is at 0.
    assert_eq!(lines, ["step 1 at 0 missed 0 now 1234"].map(str::to_owned));
}

/// The messages of U4001 of the driven state's hand-offs (`spec/26 §2` rule 2), in the
/// conformance case `name`.
fn state_label_messages(name: &str) -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/security")
        .join(name);
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    checked
        .diagnostics
        .iter()
        .filter(|d| d.code == una_diag::Code::U4001)
        .map(|d| {
            let repairs: Vec<String> = d.repairs.iter().map(|r| r.description.clone()).collect();
            format!("{} | {}", d.message, repairs.join(" | "))
        })
        .collect()
}

#[test]
fn a_state_hand_off_says_which_way_its_data_goes() {
    let invariant = state_label_messages("drive_state_label_invariant");
    assert_eq!(invariant.len(), 1, "{invariant:?}");
    assert!(
        invariant[0].contains("an invariant type argument of a kind carries data both ways")
            && invariant[0].contains("make the labels of that type argument equal"),
        "{invariant:?}"
    );
    assert!(!invariant[0].contains("function"), "{invariant:?}");
    let function = state_label_messages("drive_state_label_function");
    assert_eq!(function.len(), 1, "{function:?}");
    assert!(
        function[0].contains("would pass `secret` data to the function in the result of `init`")
            && function[0].contains("label that parameter position in the result of `init`"),
        "{function:?}"
    );
    let step = state_label_messages("drive_state_label_step");
    assert_eq!(step.len(), 1, "{step:?}");
    assert!(
        step[0].contains("which carries `secret` where `count` is `public`"),
        "{step:?}"
    );
}

/// The messages of U2028 of the driven state's hand-offs (`spec/26 §2` rule 2), in the
/// conformance case `name`.
fn state_unit_messages(name: &str) -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/security")
        .join(name);
    let checked = una_cli::check(&dir, una_cli::Tier::Zero).unwrap();
    checked
        .diagnostics
        .iter()
        .filter(|d| d.code == una_diag::Code::U2028)
        .map(|d| d.message.clone())
        .collect()
}

/// Issue #1647: Core IR erases units of measure, so before the fix an `Int of Second` start
/// checked clean against an `Int of Millisecond` state and `una run` took 7 seconds as 7
/// milliseconds. Each hand-off now compares units at every position, once per parameter, and
/// equal units pass.
#[test]
fn a_state_hand_off_compares_units_at_every_position() {
    for (name, needle) in [
        (
            "drive_state_units_step",
            "from the result of `init`, of type `Int of Second`, but `state` is `Int of Second / 1000`",
        ),
        ("drive_state_units_unitless", "of type `Int`, but"),
        ("drive_state_units_option", "`Option[Int of Second]`"),
        ("drive_state_units_list", "`List[Int of Second]`"),
        ("drive_state_units_record", "`Clock[Int of Second]`"),
        (
            "drive_state_units_on_stop",
            "the host hands `on_stop` its state",
        ),
    ] {
        let messages = state_unit_messages(name);
        assert_eq!(messages.len(), 1, "{name}: {messages:?}");
        assert!(
            messages[0].contains(needle) && messages[0].contains("the units of measure differ"),
            "{name}: {messages:?}"
        );
    }
    assert!(state_unit_messages("drive_state_units_equal").is_empty());
}

/// Issue #1647, end to end: `una run` of a module whose state changes units at a hand-off
/// prints nothing of the steps and exits with the checker's error, where it printed `8`, `9`
/// and `10` before.
#[test]
fn una_run_refuses_a_state_hand_off_in_other_units() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/security/drive_state_units_step");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_una"))
        .args([
            "run",
            "--virtual-time",
            "--stop-after-iterations",
            "3",
            "--grant",
            "console",
        ])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains('8'), "{stdout}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("U2028"),
        "{output:?}"
    );
}
