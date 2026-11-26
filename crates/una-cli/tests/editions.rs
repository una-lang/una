//! Editions are cumulative (`spec/13 §7`): every edition `0.2` package of the conformance suite,
//! checked as edition `0.3`, reports byte-identical `una --json check` output and exit status.
//! The rules that target edition `0.3` change some of them: a construct such a rule adds, which
//! edition `0.2` rejects as a syntax error, an error such a rule adds, or an error such a rule
//! removes under the amended preview rule (ADR 0103 rule 9). Each such package is
//! listed in `EXPECTED_DIFFERENCES` with the exact result of both editions, and a listed package
//! that does not differ fails the test, so the list stays exact.
//!
//! Every edition `0.3` package, checked as edition `0.4`, does the same, but for the rules that
//! target edition `0.4` and list what they may break, such as the words that ADR 0116 rule 0
//! reserves. Each is in `EXPECTED_DIFFERENCES_0_4`. Every edition `0.4` package, checked as
//! edition `0.5`, does the same, but for the rules of ADRs 0119 and 0120, which ADR 0139 rule 6
//! moves from edition `0.4` to edition `0.5`: the functions named like a built-in method of a
//! collection, or `update` on a `Map` (ADR 0119 rules 0 and 18), and the built-in names of the
//! numeric foundation. Each is in `EXPECTED_DIFFERENCES_0_5`, the edition test of ADR 0119 rule
//! 0 as its amendment of 2026-11-06 makes it. Each `0.3` package is moved to edition `0.4` by `una migrate` before the comparison, as the
//! note of ADR 0122 Question 8.1 on ADR 0121 rule 11 says, since edition `0.4` is written in
//! `ita.una`; what the migration alone changes, the manifest's own diagnostics, is not compared.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// An edition `0.2` package whose output in edition `0.3` differs by design, because of a rule
/// that targets edition `0.3` (`spec/13 §7`).
enum Difference {
    /// The package writes a construct that the rule adds and edition `0.2` rejects, as a syntax
    /// error or an unknown table of `una.toml`. The construct only narrows what a program may do, so erasing it gives a program
    /// that edition `0.2` accepts: the twin.
    Syntax {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The one diagnostic of edition `0.2`: its code and `file:line:col`.
        edition_0_2: (&'static str, &'static str),
        /// The `effect_condition` obligations of edition `0.3`, each `status at file:line:col`,
        /// in order; the check itself reports nothing.
        edition_0_3: &'static [&'static str],
        /// The same package with the construct erased, relative to `tests/conformance`.
        twin: &'static str,
        /// The file of the package, relative to it, and the text, written there exactly once,
        /// whose removal gives the twin; every other file is the twin's.
        erased: (&'static str, &'static str),
    },
    /// The rule only adds errors: the diagnostics document of edition `0.3` is that of edition
    /// `0.2` with exactly these diagnostics added, each a code and `file:line:col`.
    Stricter {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The added diagnostics.
        edition_0_3: &'static [(&'static str, &'static str)],
    },
    /// The message of a diagnostic names the test declaration by the edition (ADR 0105 rule 6):
    /// the diagnostics document of edition `0.3` is that of edition `0.2` with "its declaration
    /// and examples" written "its declaration and tests" in its messages, and with exactly
    /// these diagnostics added, each a code and `file:line:col`.
    Renamed {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The added diagnostics, none when only the wording differs.
        edition_0_3: &'static [(&'static str, &'static str)],
    },
    /// The rule only removes errors, as ADR 0103 rule 9 lets edition `0.3` accept refined type
    /// arguments and the facts they give (`spec/13 §7`): the diagnostics document of edition
    /// `0.2` is that of edition `0.3` with exactly these diagnostics added, each a code and
    /// `file:line:col`.
    Looser {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// The diagnostics of edition `0.2` that edition `0.3` does not report.
        edition_0_2: &'static [(&'static str, &'static str)],
    },
    /// Only the repairs differ (ADR 0093 rule 6): edition `0.3` reports the same diagnostics,
    /// messages and positions as edition `0.2`, and ranks a narrowed rebinding first and a
    /// widening of a public header last, marked. Each diagnostic keeps at least its repairs'
    /// count, since edition `0.3` only adds routes that keep authority and changes texts, and
    /// `fixable` may grow with them. The repairs of edition `0.3` are pinned exactly, and an
    /// edit that `una fix` may apply in edition `0.3` and not in `0.2` is one narrowed
    /// rebinding, which widens nothing.
    Repairs {
        /// The package, relative to `tests/conformance`.
        case: &'static str,
        /// Every diagnostic of edition `0.3`, in output order: its code, line, column and
        /// repairs, each as (widening kind or "", description, edit replacement or "").
        edition_0_3: &'static [PinnedRepairs],
    },
}

impl Difference {
    /// The package, relative to `tests/conformance`.
    fn case(&self) -> &'static str {
        match self {
            Difference::Syntax { case, .. }
            | Difference::Stricter { case, .. }
            | Difference::Renamed { case, .. }
            | Difference::Looser { case, .. }
            | Difference::Repairs { case, .. } => case,
        }
    }

    /// The edition `0.2` packages the entry names.
    fn packages(&self) -> Vec<&'static str> {
        match self {
            Difference::Syntax { case, twin, .. } => vec![*case, *twin],
            Difference::Stricter { case, .. }
            | Difference::Renamed { case, .. }
            | Difference::Looser { case, .. }
            | Difference::Repairs { case, .. } => vec![*case],
        }
    }
}

/// The edition `0.2` packages whose output in edition `0.3` differs by design, with the exact
/// result of each edition.
const EXPECTED_DIFFERENCES: &[Difference] = &[
    // ADR 0093 rule 6: U3001 and U3002 rank a narrowed rebinding first and mark a widening of a
    // public header as an authority change, last; nothing else differs.
    Difference::Repairs {
        case: "authority/narrowing_repairs_edition_0_2",
        edition_0_3: &[
            (
                "U3002",
                22,
                3,
                &[
                    (
                        "",
                        "add `with @files = @store[read]` to this statement, since `@store` permits `read`",
                        " with @files = @store[read]",
                    ),
                    (
                        "authority",
                        "add `read` to the `uses` clause of `@files`; this is an authority change of the public header of `more`, which every caller in every module must then hold",
                        "uses @files: Files[read, write]",
                    ),
                ],
            ),
            (
                "U3001",
                27,
                3,
                &[
                    (
                        "",
                        "add `with @files = @store[read]` to this statement, since `@store` permits `read`",
                        " with @files = @store[read]",
                    ),
                    (
                        "authority",
                        "declare `uses @files: Files[read]` in the header; this is an authority change of the public header of `missing`, which every caller in every module must then hold",
                        "\n  uses @files: Files[read]",
                    ),
                ],
            ),
            (
                "U3002",
                32,
                3,
                &[(
                    "authority",
                    "add `read` to the `uses` clause of `@files`",
                    "uses @files: Files[read, write]",
                )],
            ),
            (
                "U3002",
                37,
                10,
                &[(
                    "authority",
                    "add `read` to the `uses` clause of `@files`; this is an authority change of the public header of `direct`, which every caller in every module must then hold",
                    "uses @files: Files[read, write]",
                )],
            ),
        ],
    },
    Difference::Repairs {
        case: "authority/value_uses_at_reference_0_2",
        edition_0_3: &[
            (
                "U3001",
                39,
                16,
                &[(
                    "authority",
                    "declare `uses @log: Log[write]` in the header; this is an authority change of the public header of `no_binding`, which every caller in every module must then hold",
                    "\n  uses @log: Log[write]",
                )],
            ),
            (
                "U3003",
                41,
                40,
                &[(
                    "authority",
                    "declare `uses @log: Log[write]` in the header",
                    "\n  uses @log: Log[write]\n=\n  ",
                )],
            ),
        ],
    },
    // ADR 0105 rule 3: an `example` declaration is U0103 in edition 0.3, where the test
    // declaration is written `test`, with the exact rewrite that `una fix` applies.
    Difference::Stricter {
        case: "runtime/print_hashed",
        edition_0_3: &[("U0103", "src/hashed/main.una:36:1")],
    },
    Difference::Stricter {
        case: "security/edit_label_variables",
        edition_0_3: &[
            ("U0103", "src/main.una:35:1"),
            ("U0103", "src/main.una:39:1"),
        ],
    },
    Difference::Stricter {
        case: "security/s14_ok_write_from_example",
        edition_0_3: &[("U0103", "src/main.una:26:1")],
    },
    Difference::Stricter {
        case: "security/s14_ok_write_in_do_line",
        edition_0_3: &[("U0103", "src/main.una:19:1")],
    },
    Difference::Stricter {
        case: "security/via_hmac_events_shape",
        edition_0_3: &[("U0103", "src/main.una:20:1")],
    },
    // ADR 0105 rules 3 and 6: an `example` is U0103, and the message of a `Keys` handler named
    // outside its declaration names the declaration `tests`.
    Difference::Renamed {
        case: "security/via_hmac_handler_in_comment",
        edition_0_3: &[("U0103", "src/main.una:23:1")],
    },
    // ADR 0105 rule 6: only the message's word for the test declaration differs.
    Difference::Renamed {
        case: "security/via_hmac_handler_delegates",
        edition_0_3: &[],
    },
    Difference::Renamed {
        case: "security/via_hmac_handler_forwards",
        edition_0_3: &[],
    },
    Difference::Renamed {
        case: "security/via_hmac_handler_in_with",
        edition_0_3: &[],
    },
    Difference::Renamed {
        case: "security/via_hmac_second_handler",
        edition_0_3: &[],
    },
    // ADR 0105 rule 4: a local named `test` is U0101 in edition 0.3, where `test` is a keyword.
    Difference::Stricter {
        case: "security/generic_result_summary_leaks",
        edition_0_3: &[("U0101", "src/summaryleaks.una:41:7")],
    },
    Difference::Stricter {
        case: "types/memory_example_do",
        edition_0_3: &[
            ("U0103", "src/memdo.una:18:1"),
            ("U0103", "src/memdo.una:22:1"),
        ],
    },
    // ADR 0105 rule 4: `test` is a keyword in edition 0.3, so a function and a local named
    // `test` are U0101 there; the local's rename is the one edit.
    Difference::Stricter {
        case: "names/test_name_0_2",
        edition_0_3: &[("U0101", "src/m.una:6:1"), ("U0101", "src/m.una:10:7")],
    },
    // ADR 0104 rule 1: a unit declaration, a parse error before edition 0.3.
    Difference::Syntax {
        case: "types/units_edition_0_2",
        edition_0_2: ("U0101", "src/m.una:6:13"),
        edition_0_3: &[],
        twin: "types/units_edition_0_2_erased",
        erased: ("src/m.una", "pub measure Meter\n\n"),
    },
    // ADR 0092 rule 1: a condition on an operation of a `uses`.
    Difference::Syntax {
        case: "authority/op_condition_edition_0_2",
        edition_0_2: ("U0101", "src/main.una:10:24"),
        edition_0_3: &["proven at src/main.una:13:5"],
        twin: "authority/op_condition_edition_0_2_erased",
        erased: ("src/main.una", " where not dry_run"),
    },
    // ADR 0092 rule 8: a lock of `[contracts]`, an unknown table before edition 0.3.
    Difference::Syntax {
        case: "authority/contracts_edition_0_2",
        edition_0_2: ("U7002", "una.toml:6:2"),
        edition_0_3: &[],
        twin: "authority/contracts_edition_0_2_erased",
        erased: (
            "una.toml",
            "\n[contracts]\n\"main.update\" = [\"@catalog: Catalog[price]\"]\n",
        ),
    },
    // ADR 0093 rule 4: a narrowed rebinding, `with @x = @y[ops]`.
    Difference::Syntax {
        case: "authority/narrowed_rebinding_edition_0_2",
        edition_0_2: ("U0101", "src/main.una:20:39"),
        edition_0_3: &[],
        twin: "authority/narrowed_rebinding_edition_0_2_erased",
        erased: ("src/main.una", "[write, read]"),
    },
    // ADR 0093 rule 1: the `[boundaries]` table of `una.toml`.
    Difference::Syntax {
        case: "project/boundaries_edition_0_2",
        edition_0_2: ("U7002", "una.toml:6:2"),
        edition_0_3: &[],
        twin: "project/boundaries_edition_0_2_erased",
        erased: (
            "una.toml",
            "\n[boundaries]\n\"gallery.thumbnailer\" = [\"Files[read]\"]\n",
        ),
    },
    // ADR 0100 rule 2: `attempt` of a call that cannot fail.
    Difference::Stricter {
        case: "authority/dead_attempt_edition_0_2",
        edition_0_3: &[("U3026", "src/main.una:8:42")],
    },
    // ADR 0100 rule 1: an `if error` clause on an expression that cannot fail.
    Difference::Stricter {
        case: "authority/dead_clause_edition_0_2",
        edition_0_3: &[
            ("U3025", "src/main.una:22:5"),
            ("U3025", "src/main.una:24:5"),
            ("U3025", "src/main.una:31:5"),
            ("U3025", "src/main.una:37:5"),
        ],
    },
    // ADR 0100 rule 2: `attempt` of a `fails of` call given a function that cannot fail.
    Difference::Stricter {
        case: "runtime/fails_of_relay",
        edition_0_3: &[
            ("U3026", "src/relay.una:81:12"),
            ("U3026", "src/relay.una:83:12"),
        ],
    },
    // ADR 0114 rule 15: a float literal, a parse error before edition 0.3. Unlike the
    // constructs above it does not narrow (the carve-out of rule 15), so its twin is the
    // package without the function that holds it.
    Difference::Syntax {
        case: "types/float_literal_edition_0_2",
        edition_0_2: ("U0101", "src/m.una:8:23"),
        edition_0_3: &[],
        twin: "types/float_literal_edition_0_2_erased",
        erased: ("src/m.una", "\n\npub half() -> F64 = 1.0"),
    },
    // ADR 0094 rule 6: `gives` after an operation of a header's `uses`, U0103 before edition
    // 0.3. Erasing it gives a program that edition 0.2 accepts.
    Difference::Syntax {
        case: "syntax/gives_before_0_3",
        edition_0_2: ("U0103", "src/m.una:13:36"),
        edition_0_3: &[],
        twin: "syntax/gives_before_0_3_erased",
        erased: ("src/m.una", " gives internal[Service]"),
    },
    // ADR 0114 rule 15: a declaration named `F64`, built in from edition 0.3.
    Difference::Stricter {
        case: "types/float_names_edition_0_2",
        edition_0_3: &[("U1002", "src/m.una:6:10")],
    },
    // ADR 0114 rule 15: a package function `to_f64` on integers, which the built-in
    // conversion resolves ahead of in edition 0.3.
    Difference::Stricter {
        case: "types/float_to_f64_edition_0_2",
        edition_0_3: &[("U1002", "src/m.una:6:5")],
    },
    // ADR 0103 rule 1: a refined type argument, U2002 before edition 0.3.
    Difference::Looser {
        case: "verification/refined_elements_0_2",
        edition_0_2: &[("U2002", "src/edition_0_2.una:8:22")],
    },
    // ADR 0103 rule 6: `[p]` is a `List[Pos]` in edition 0.3, so `first`'s element carries `Pos`.
    Difference::Looser {
        case: "verification/refined_elements_0_2_facts",
        edition_0_2: &[("U5001", "src/edition_0_2_facts.una:14:25")],
    },
    // ADR 0109 rule 1: `[package] profile`, an unknown key before edition 0.3, where the
    // package checks clean.
    Difference::Looser {
        case: "project/profile_edition_0_2",
        edition_0_2: &[("U7002", "una.toml:5:1")],
    },
];

/// An edition `0.3` package whose output in edition `0.4` differs by design: the diagnostics a
/// rule that targets edition `0.4` adds, and those it removes, a code and `file:line:col` each.
struct Difference04 {
    /// The package, relative to `tests/conformance`.
    case: &'static str,
    /// The diagnostics edition `0.4` reports and edition `0.3` does not.
    added: &'static [(&'static str, &'static str)],
    /// The diagnostics edition `0.3` reports and edition `0.4` does not.
    removed: &'static [(&'static str, &'static str)],
}

/// The edition `0.3` packages whose output in edition `0.4` differs by design. The rules of ADRs
/// 0119 and 0120 are no longer among them: ADR 0139 rule 6 moves them to edition `0.5`, so their
/// packages are in `EXPECTED_DIFFERENCES_0_5`.
const EXPECTED_DIFFERENCES_0_4: &[Difference04] = &[
    // ADR 0073 J5, accepted amendment of 2026-11-09 (#917): a `Json` constructor accepts a
    // labeled argument from edition 0.4 and labels its result, so the U4001 at the argument
    // moves to where the `secret Json` meets the declared `Json` result.
    Difference04 {
        case: "security/json_labels_0_3",
        added: &[("U4001", "src/json_labels_0_3.una:25:43")],
        removed: &[("U4001", "src/json_labels_0_3.una:25:47")],
    },
    // ADR 0116 amendment of 2026-11-09 (#898, C1, C6): a reading of `Clock.now` carries its
    // timing label from edition 0.4, so the elapsed time of work on a `secret` prints no more.
    Difference04 {
        case: "security/clock_reading_edition_0_3",
        added: &[("U4003", "src/leak.una:15:18")],
        removed: &[],
    },
    // ADR 0123 rule 0 (`spec/31 §1`): `std.host` exports `HttpServer` from edition 0.4.
    Difference04 {
        case: "names/host_server_import_0_3",
        added: &[],
        removed: &[("U1001", "src/m.una:6:18")],
    },
    // ADR 0116 rule 12: `HttpError.Interrupted` is a variant from edition 0.4, and so is
    // `RateLimited` (ADR 0117 rule 8), which the `when` then lacks.
    Difference04 {
        case: "names/http_interrupted_0_3",
        added: &[("U2007", "src/poll.una:9:3")],
        removed: &[("U1001", "src/poll.una:18:8")],
    },
    // ADR 0117 rule 8: `HttpError.RateLimited` is a variant from edition 0.4, and so is
    // `Interrupted` (ADR 0116 rule 12), which the `when` then lacks.
    Difference04 {
        case: "names/http_rate_limited_0_3",
        added: &[("U2007", "src/poll.una:9:3")],
        removed: &[("U1001", "src/poll.una:18:8")],
    },
    // ADR 0117 rule 9 (`spec/27 §10`): `std.limit` is an importable module from edition 0.4.
    Difference04 {
        case: "names/limit_0_3",
        added: &[],
        removed: &[("U1001", "src/gate.una:6:8")],
    },
    // ADR 0123 rule 21 (`spec/31 §8`): `test_request` is a name of `std.testing` from edition
    // 0.4.
    Difference04 {
        case: "names/test_request_import_0_3",
        added: &[],
        removed: &[("U1001", "src/m.una:6:21")],
    },
    // ADR 0117 rule 6: `first of` is an expression from edition 0.4.
    Difference04 {
        case: "syntax/first_of_before_0_4",
        added: &[],
        removed: &[("U0101", "src/late.una:10:17")],
    },
    // ADR 0116 rule 0: `diverges` and `within` are reserved words.
    Difference04 {
        case: "syntax/long_running_words_in_0_3",
        added: &[
            ("U0103", "src/words03.una:8:7"),
            ("U0103", "src/words03.una:9:7"),
            ("U0103", "src/words03.una:10:3"),
            ("U0103", "src/words03.una:10:14"),
        ],
        removed: &[],
    },
    // ADR 0116 rule 5: `stop when` and `skip` are statements from edition 0.4.
    Difference04 {
        case: "syntax/loop_exits_before_0_4",
        added: &[],
        removed: &[("U0101", "src/before.una:10:10")],
    },
    // ADR 0105, amendment of 2026-11-05 (`spec/01 §5`): `example` is reserved in edition 0.3
    // and an identifier from edition 0.4. A name `example` is accepted there, and an `example`
    // declaration loses its U0103 and rewrite: it is the ordinary syntax error of a name at
    // column 1 followed by a text literal, and its body is no longer read as a test's.
    Difference04 {
        case: "fmt/example_in_0_3_untouched",
        added: &[("U0101", "src/m.una:8:9")],
        removed: &[("U0103", "src/m.una:8:1")],
    },
    Difference04 {
        case: "names/example_name_0_3",
        added: &[],
        removed: &[("U0101", "src/m.una:8:7")],
    },
    Difference04 {
        case: "syntax/example_in_0_3",
        added: &[("U0101", "src/m.una:8:9")],
        removed: &[("U0103", "src/m.una:8:1")],
    },
    Difference04 {
        case: "syntax/example_recovery_0_3",
        added: &[("U0101", "src/m.una:9:9")],
        removed: &[("U0103", "src/m.una:9:1"), ("U1001", "src/m.una:11:20")],
    },
    // ADR 0116 rule 19: an example's `stop after` line exists from edition 0.4.
    Difference04 {
        case: "syntax/stop_after_before_0_4",
        added: &[],
        removed: &[("U0101", "src/before.una:11:3")],
    },
    // ADR 0116 rule 5 (spec/26 §3 rule 1, #1807): `stop` and `skip` are statements from
    // edition 0.4 and names before it.
    Difference04 {
        case: "syntax/bare_stop_skip_before_0_4",
        added: &[("U2040", "src/loops.una:11:5")],
        removed: &[
            ("U1001", "src/loops.una:10:5"),
            ("U1001", "src/loops.una:12:7"),
        ],
    },
    Difference04 {
        case: "syntax/stop_skip_bound_before_0_4",
        added: &[
            ("U2034", "src/names.una:11:5"),
            ("U2034", "src/names.una:13:5"),
        ],
        removed: &[],
    },
    Difference04 {
        case: "syntax/stop_when_before_0_4",
        added: &[],
        removed: &[("U0101", "src/loops.una:9:10")],
    },
    // ADR 0117 rules 1 and 2: `tasks` and `start` are statements from edition 0.4.
    Difference04 {
        case: "syntax/tasks_before_0_4",
        added: &[],
        removed: &[("U0101", "src/late.una:11:5")],
    },
    // ADR 0116 rule 7: `for until` is a loop form from edition 0.4.
    Difference04 {
        case: "syntax/until_before_0_4",
        added: &[],
        removed: &[("U0101", "src/before.una:9:13")],
    },
    // ADR 0117 rule 13: `e within d` is an expression from edition 0.4, and there its unitless
    // deadline `5` is refused where a duration is required.
    Difference04 {
        case: "syntax/within_before_0_4",
        added: &[("U2028", "src/late.una:10:26")],
        removed: &[("U0101", "src/late.una:10:19")],
    },
];

/// The edition `0.4` packages whose output in edition `0.5` differs by design: those that use,
/// or clash with, a construct of ADR 0119 or ADR 0120, which ADR 0139 rule 6 moves from edition
/// `0.4` to edition `0.5`. Each is the edition `0.4` twin of an edition `0.3` package that this
/// comparison made with edition `0.4` before the move (ADR 0119 and ADR 0120, amendments of
/// 2026-11-06: "each comparison with edition `0.3` is made with edition `0.4`").
const EXPECTED_DIFFERENCES_0_5: &[Difference04] = &[
    // ADR 0126 rule 1: a module constant exists from edition 0.5.
    Difference04 {
        case: "syntax/module_constant_before_0_5",
        added: &[],
        removed: &[("U0101", "src/m.una:6:7")],
    },
    // ADR 0119 rule 0: `std.collections` is an importable module from edition 0.5.
    Difference04 {
        case: "names/collections_import_0_4",
        added: &[],
        removed: &[("U1001", "src/m.una:6:8")],
    },
    // ADR 0120 rule 4 (`spec/29 §18` rule 6, `§16` rule 5): the built-in `to_f32` and
    // `to_decimal` resolve ahead of a package function of their name from edition 0.5.
    Difference04 {
        case: "names/conversion_functions_in_0_4",
        added: &[
            ("U1002", "src/a.una:3:5"),
            ("U1002", "src/b.una:3:5"),
            ("U1002", "src/c.una:3:5"),
            ("U1002", "src/d.una:3:5"),
            ("U1002", "src/e.una:3:5"),
            ("U1002", "src/m.una:6:5"),
        ],
        removed: &[],
    },
    // ADR 0120 (`spec/29 §16`): `Decimal[s]` is a type from edition 0.5.
    Difference04 {
        case: "names/decimal_edition_0_4",
        added: &[],
        removed: &[("U0101", "src/m.una:6:22")],
    },
    // ADR 0119 rule 18: a package function `update` whose first parameter is a `Map`.
    Difference04 {
        case: "names/map_update_function_in_0_4",
        added: &[("U1002", "src/a.una:6:5")],
        removed: &[],
    },
    // ADR 0120 rule 1: `F32`, `Decimal`, `Complex`, `Array`, `Matrix`, `Tensor` and `Lu` are
    // built-in names from edition 0.5, so a kind, variant or unit of one of those names is
    // refused there.
    Difference04 {
        case: "names/numeric_names_in_0_4",
        added: &[
            ("U1002", "src/m.una:6:10"),
            ("U1002", "src/m.una:8:10"),
            ("U1002", "src/m.una:11:10"),
            ("U1002", "src/m.una:14:10"),
            ("U1002", "src/m.una:16:18"),
            ("U1002", "src/m.una:16:27"),
            ("U1002", "src/m.una:18:13"),
        ],
        removed: &[],
    },
    // ADR 0120: the names of the numeric foundation resolve from edition 0.5, where
    // `as_decimal` returns the `Option[Decimal[2]]` that edition 0.4 cannot write.
    Difference04 {
        case: "names/numerics_in_0_4",
        added: &[("U2001", "src/m.una:17:38")],
        removed: &[
            ("U1001", "src/m.una:11:13"),
            ("U1001", "src/m.una:13:13"),
            ("U2002", "src/m.una:15:13"),
            ("U1001", "src/m.una:17:38"),
            ("U1001", "src/m.una:19:15"),
            ("U1001", "src/m.una:19:26"),
        ],
    },
    // ADR 0137 rule 1 (`spec/01 §5`): `const` and `var` are keywords of edition 0.5, so the
    // bindings edition 0.4 refuses are accepted there.
    Difference04 {
        case: "syntax/const_var_in_0_4",
        added: &[],
        removed: &[("U0101", "src/m.una:9:9"), ("U0103", "src/m.una:14:3")],
    },
    // ADR 0120 rule 1 (`spec/01 §5`): `simd` is reserved before edition 0.5 and an identifier
    // from it.
    Difference04 {
        case: "syntax/simd_word_in_0_4",
        added: &[],
        removed: &[
            ("U0103", "src/simd04.una:9:7"),
            ("U0103", "src/simd04.una:10:3"),
        ],
    },
    // ADR 0120 (`spec/29 §17`): `Matrix`, `Tensor` and `Lu` are built in from edition 0.5.
    Difference04 {
        case: "types/dense_edition_0_4",
        added: &[
            ("U1002", "src/dense.una:6:10"),
            ("U1002", "src/dense.una:8:10"),
            ("U2003", "src/dense.una:13:33"),
            ("U2001", "src/dense.una:15:36"),
        ],
        removed: &[],
    },
];

const EDITION_0_2: &str = "edition = \"0.2\"";
const EDITION_0_3: &str = "edition = \"0.3\"";

/// The start of every `una --json check` document (`spec/13 §5`).
const DOCUMENT: &[u8] = b"{\"schema\":\"una.diagnostics/1\"";

/// The start of every `una --json obligations` document.
const OBLIGATIONS: &[u8] = b"{\"schema\":\"una.obligations/1\"";

/// An edition `0.2` package that uses fixed-width integers, so it checks differently as edition
/// `0.1`: the positive control that the copied manifest is the one read.
const CONTROL: &str = "types/fixed_width_methods";

/// The conformance packages whose expected result is U8001: a declaration that type-checked but
/// did not elaborate rejects the package with exit code 3 (`spec/09 §3`). Each of them may, and
/// must, exit 3 here; its diagnostics are still compared across editions. Every other package
/// exits 0 or 1.
const REJECTED_AS_INTERNAL: &[&str] = &[
    "verification/assign_list_len_rejected",
    "verification/assign_nonlocal_root_rejected",
    "verification/cap_in_opaque_predicate_rejected",
    "verification/edit_marker_builtin_method_rejected",
];

/// Whether `package` is one of [`REJECTED_AS_INTERNAL`].
fn rejected_as_internal(package: &Path) -> bool {
    package
        .strip_prefix(conformance())
        .is_ok_and(|name| REJECTED_AS_INTERNAL.contains(&name.to_string_lossy().as_ref()))
}

/// Whether `status` is an exit code that a check of `package` may end with.
fn expected_status(package: &Path, status: Option<i32>) -> bool {
    if rejected_as_internal(package) {
        status == Some(3)
    } else {
        matches!(status, Some(0 | 1))
    }
}

fn conformance() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

/// Every package directory under `dir`, in a stable order.
fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.join("una.toml").is_file() {
            out.push(entry);
        } else {
            packages(&entry, out);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The exit status and output of `una --json check .` on a fresh copy of `package` at `work`,
/// with its manifest's edition line replaced by `edition`.
fn check_as(package: &Path, work: &Path, edition: &str) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    run_as(package, work, edition, "check", DOCUMENT)
}

/// The exit status and output of `una --json <command> .` on a fresh copy of `package` at
/// `work`, with its manifest's edition line replaced by `edition`; stdout starts with `document`.
fn run_as(
    package: &Path,
    work: &Path,
    edition: &str,
    command: &str,
    document: &[u8],
) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    copy_dir(package, work);
    // The manifest's edition line, whichever edition it names, becomes `edition`, on the same
    // line, so a position in `una.toml` stays where it was.
    let manifest = work.join("una.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let replaced: String = text
        .lines()
        .map(|line| {
            if line.trim() == EDITION_0_2 || line.trim() == EDITION_0_3 {
                edition
            } else {
                line
            }
        })
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&manifest, replaced).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_una"))
        .env("HOME", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("UNA_PROOF_KEY")
        .args(["--json", command, "."])
        .current_dir(work)
        .output()
        .unwrap();
    std::fs::remove_dir_all(work).unwrap();
    // A run that fails the same way in both editions, before it checks anything, must not count
    // as the same result: each run exits 0 or 1 with a diagnostics document, or 3 for a package
    // of `REJECTED_AS_INTERNAL`.
    assert!(
        expected_status(package, output.status.code()) && output.stdout.starts_with(document),
        "{} as {edition}: status {:?}, stdout {:?}, stderr {:?}",
        package.display(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), output.stdout, output.stderr)
}

#[test]
fn every_edition_0_2_package_checks_the_same_in_edition_0_3() {
    let root = conformance();
    let mut all = Vec::new();
    packages(&root, &mut all);
    // An edition line that names `0.2` in another spelling would drop its package from the test
    // without a word.
    for package in &all {
        let text = std::fs::read_to_string(package.join("una.toml")).unwrap();
        for line in text.lines() {
            let line = line.trim();
            assert!(
                !(line.starts_with("edition") && line.contains("0.2")) || line == EDITION_0_2,
                "{}: the edition line `{line}` is not `{EDITION_0_2}`",
                package.display()
            );
        }
    }
    let cases: Vec<(String, PathBuf)> = all
        .into_iter()
        .filter(|package| {
            std::fs::read_to_string(package.join("una.toml"))
                .unwrap()
                .lines()
                .any(|line| line.trim() == EDITION_0_2)
        })
        .map(|package| {
            let name = package
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (name, package)
        })
        .collect();
    let (_, control) = cases
        .iter()
        .find(|(name, _)| name == CONTROL)
        .expect("the control is an edition 0.2 package");
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions/control");
    assert_ne!(
        check_as(control, &work, EDITION_0_2),
        check_as(control, &work, "edition = \"0.1\""),
        "{CONTROL} checks the same as edition 0.1"
    );
    for expected in EXPECTED_DIFFERENCES {
        for name in expected.packages() {
            assert!(
                cases.iter().any(|(case, _)| case == name),
                "{name} is not an edition 0.2 package"
            );
        }
    }

    // Linear in the packages: each one is checked twice, on its own copy.
    let workers = 4;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions");
    let results: Vec<(String, bool)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let cases = &cases;
                let work = scratch.join(format!("w{worker}"));
                scope.spawn(move || {
                    cases
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(name, package)| {
                            let before = check_as(package, &work, EDITION_0_2);
                            let after = check_as(package, &work, EDITION_0_3);
                            (name.clone(), before == after)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    let mut wrong = Vec::new();
    for (name, same) in &results {
        let expected = EXPECTED_DIFFERENCES
            .iter()
            .any(|difference| difference.case() == name);
        if *same == expected {
            wrong.push(if expected {
                format!("{name}: listed as differing, but checks the same in edition 0.3")
            } else {
                format!("{name}: checks differently in edition 0.3")
            });
        }
    }
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Every edition `0.3` package of the conformance suite, MIGRATED to edition `0.4` by
/// `una migrate`, checks as it does in edition `0.3`, but for the packages of
/// `EXPECTED_DIFFERENCES_0_4`, each of which gains exactly its listed diagnostics (ADR 0119 rule
/// 0, ADR 0121 rule 11 with the note of ADR 0122 Question 8.1). The invariant is over migrated
/// packages: `una migrate` also rewrites the modules in the canonical form of edition `0.4`,
/// which wraps long lines (ADR 0125, amendment of 2026-10-25), so a package is compared after
/// it, never as edition `0.3` wrote it. That rewrite changes only white space, which is checked,
/// and the positions of edition `0.3` are moved to the same characters of the rewritten modules.
/// A difference that the migration alone causes is not a clash: the diagnostics of the manifest
/// and the lock, whose files the migration renames, are left out of both sides. A package whose
/// `una.toml` `una migrate` refuses is one that edition `0.3` already reports at its manifest.
/// Linear in the packages, each checked twice.
#[test]
fn every_edition_0_3_package_checks_the_same_in_edition_0_4() {
    let root = conformance();
    let mut all = Vec::new();
    packages(&root, &mut all);
    let cases: Vec<(String, PathBuf)> = all
        .into_iter()
        .filter(|package| {
            std::fs::read_to_string(package.join("una.toml"))
                .unwrap()
                .lines()
                .any(|line| line.trim() == EDITION_0_3)
        })
        .map(|package| {
            let name = package
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (name, package)
        })
        .collect();
    for Difference04 { case: name, .. } in EXPECTED_DIFFERENCES_0_4 {
        assert!(
            cases.iter().any(|(case, _)| case == name),
            "{name} is not an edition 0.3 package"
        );
    }
    for name in REJECTED_AS_INTERNAL {
        assert!(
            cases.iter().any(|(case, _)| case == name),
            "{name} is not an edition 0.3 package"
        );
    }
    let workers = 4;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions_0_4");
    let results: Vec<(String, Result<(), String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let cases = &cases;
                let work = scratch.join(format!("w{worker}"));
                scope.spawn(move || {
                    cases
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(name, package)| {
                            let listed = EXPECTED_DIFFERENCES_0_4
                                .iter()
                                .find(|difference| difference.case == name);
                            let added = listed.map_or(&[][..], |difference| difference.added);
                            let removed = listed.map_or(&[][..], |difference| difference.removed);
                            (
                                name.clone(),
                                compare_migrated(package, &work, added, removed),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    let mut wrong: Vec<String> = results
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|why| format!("{name}: {why}")))
        .collect();
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The edition line of `ita.una` that the comparison of editions `0.4` and `0.5` replaces.
const ITA_EDITION_0_4: &str = "  edition 0.4";

/// Every package directory under `dir` described by `ita.una`, dependencies included, in a
/// stable order.
fn ita_packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    for entry in entries {
        if entry.join("ita.una").is_file() {
            out.push(entry.clone());
        }
        ita_packages(&entry, out);
    }
}

/// The exit status and the code and `file:line:col` of each diagnostic of `una --json check .`
/// on a fresh copy of `package` at `work`, its `ita.una` naming `edition`.
fn ita_check_as(
    package: &Path,
    work: &Path,
    edition: &str,
) -> (Option<i32>, Vec<(String, String)>) {
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    copy_dir(package, work);
    let manifest = work.join("ita.una");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let replaced: String = text
        .lines()
        .map(|line| {
            if line == ITA_EDITION_0_4 {
                edition
            } else {
                line
            }
        })
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&manifest, replaced).unwrap();
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .args(["--json", "check", "."])
            .current_dir(work)
            .output()
            .unwrap()
    };
    // ADR 0137: a package moves to edition 0.5 with the rewrite of its `let`s to `const` and
    // `var`, the exact repairs of their U7007, which `una fix` applies; the positions of the
    // check after the rewrite are mapped back to the package as written.
    let moved = if edition == ITA_EDITION_0_5 {
        binding_words_of_0_5(work, &check())
    } else {
        Moved::default()
    };
    let output = check();
    std::fs::remove_dir_all(work).unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 1)) && output.stdout.starts_with(DOCUMENT),
        "{} as {edition}: status {:?}, stdout {:?}, stderr {:?}",
        package.display(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    // A rewrite that lengthens a line may take it past the canonical width: `una fix` formats
    // it after the rewrite, so the U7001 of a rewritten line is the move's, not the edition's.
    let all = sites(&document);
    let sites: Vec<(String, String)> = all
        .iter()
        .filter(|(code, at)| !(code == "U7001" && moved.rewrote(at)))
        .map(|(code, at)| (code.clone(), moved.back(at)))
        .collect();
    // Without those, a package with no other diagnostic checks.
    let status = if sites.is_empty() && sites.len() < all.len() {
        Some(0)
    } else {
        output.status.code()
    };
    (status, sites)
}

/// The edition line of `ita.una` that moves a package to edition `0.5`.
const ITA_EDITION_0_5: &str = "  edition 0.5";

/// A replaced range of a line: its start column, its end column and the replacement's length.
type Rewrite = (u64, u64, u64);

/// The rewrites of a package moved to edition `0.5`: each replaced range, by file and line.
#[derive(Default)]
struct Moved {
    edits: std::collections::BTreeMap<(String, u64), Vec<Rewrite>>,
}

impl Moved {
    /// Whether the line of `at`, a `file:line:col`, holds a rewrite.
    fn rewrote(&self, at: &str) -> bool {
        let mut parts = at.rsplitn(3, ':');
        let (Some(_), Some(line), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
            return false;
        };
        line.parse::<u64>()
            .is_ok_and(|line| self.edits.contains_key(&(file.to_owned(), line)))
    }

    /// The `file:line:col` of the package as written for `at`, a position after the rewrites.
    fn back(&self, at: &str) -> String {
        let mut parts = at.rsplitn(3, ':');
        let (Some(col), Some(line), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
            return at.to_owned();
        };
        let (Ok(col), Ok(line)) = (col.parse::<u64>(), line.parse::<u64>()) else {
            return at.to_owned();
        };
        let Some(edits) = self.edits.get(&(file.to_owned(), line)) else {
            return at.to_owned();
        };
        let mut shift: i64 = 0;
        for &(start, end, written) in edits {
            let new_start = (start as i64 + shift) as u64;
            let new_end = new_start + written;
            if col < new_end {
                if col >= new_start {
                    return format!("{file}:{line}:{start}");
                }
                break;
            }
            shift += written as i64 - (end - start) as i64;
        }
        format!("{file}:{line}:{}", (col as i64 - shift) as u64)
    }
}

/// Applies, in the package at `work`, the first repair of each U7007 of `output`, a check of
/// it, and returns where the rewrites are.
fn binding_words_of_0_5(work: &Path, output: &std::process::Output) -> Moved {
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut moved = Moved::default();
    for diagnostic in document["diagnostics"].as_array().unwrap() {
        if diagnostic["code"] != "U7007" {
            continue;
        }
        let edit = &diagnostic["repairs"][0]["edit"];
        let file = edit["file"].as_str().unwrap().to_owned();
        let line = edit["start"]["line"].as_u64().unwrap();
        assert_eq!(edit["end"]["line"].as_u64(), Some(line), "{edit}");
        let replacement = edit["replacement"].as_str().unwrap();
        moved.edits.entry((file, line)).or_default().push((
            edit["start"]["col"].as_u64().unwrap(),
            edit["end"]["col"].as_u64().unwrap(),
            replacement.chars().count() as u64,
        ));
    }
    moved
        .edits
        .values_mut()
        .for_each(|edits| edits.sort_unstable());
    let mut by_file: std::collections::BTreeMap<&str, Vec<(u64, u64, u64, String)>> =
        std::collections::BTreeMap::new();
    for diagnostic in document["diagnostics"].as_array().unwrap() {
        if diagnostic["code"] == "U7007" {
            let edit = &diagnostic["repairs"][0]["edit"];
            by_file
                .entry(edit["file"].as_str().unwrap())
                .or_default()
                .push((
                    edit["start"]["line"].as_u64().unwrap(),
                    edit["start"]["col"].as_u64().unwrap(),
                    edit["end"]["col"].as_u64().unwrap(),
                    edit["replacement"].as_str().unwrap().to_owned(),
                ));
        }
    }
    for (file, mut edits) in by_file {
        // From the last edit back, so that each edit's columns are those of the file as read.
        edits.sort_unstable();
        edits.reverse();
        let path = work.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        for (line, start, end, replacement) in edits {
            let chars: Vec<char> = lines[line as usize - 1].chars().collect();
            let before: String = chars[..start as usize - 1].iter().collect();
            let after: String = chars[end as usize - 1..].iter().collect();
            lines[line as usize - 1] = format!("{before}{replacement}{after}");
        }
        std::fs::write(&path, lines.join("\n")).unwrap();
    }
    moved
}

/// Every edition `0.4` package of the conformance suite, checked as edition `0.5`, reports the
/// same diagnostics, codes and positions, in order, and the same exit status, but for the
/// packages of `EXPECTED_DIFFERENCES_0_5`, each of which gains exactly its `added` diagnostics
/// and loses exactly its `removed` ones: edition `0.5` accepts every package that edition `0.4`
/// accepts, and moving a package to edition `0.5` changes nothing else (ADR 0139 rules 6 and 8).
/// The move rewrites the package's `let`s to `const` and `var` by the exact repairs of their
/// U7007 (ADR 0137 rules 11 and 12, the equivalence of its Conformance), and the positions are
/// compared in the package as written. Messages may differ, as a message that lists what an
/// edition has does, or names its binding words. Linear in the packages, each checked three
/// times.
#[test]
fn every_edition_0_4_package_checks_the_same_in_edition_0_5() {
    let root = conformance();
    let mut all = Vec::new();
    ita_packages(&root, &mut all);
    let cases: Vec<(String, PathBuf)> = all
        .into_iter()
        .filter(|package| {
            std::fs::read_to_string(package.join("ita.una"))
                .unwrap()
                .lines()
                .any(|line| line == ITA_EDITION_0_4)
        })
        .map(|package| {
            let name = package
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (name, package)
        })
        .collect();
    for Difference04 { case: name, .. } in EXPECTED_DIFFERENCES_0_5 {
        assert!(
            cases.iter().any(|(case, _)| case == name),
            "{name} is not an edition 0.4 package"
        );
    }
    let workers = 4;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions_0_5");
    let results: Vec<(String, Result<(), String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let cases = &cases;
                let work = scratch.join(format!("w{worker}"));
                scope.spawn(move || {
                    cases
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|(name, package)| {
                            let listed = EXPECTED_DIFFERENCES_0_5
                                .iter()
                                .find(|difference| difference.case == name);
                            let added = listed.map_or(&[][..], |difference| difference.added);
                            let removed = listed.map_or(&[][..], |difference| difference.removed);
                            let (status_0_4, mut before) =
                                ita_check_as(package, &work, ITA_EDITION_0_4);
                            let (status_0_5, mut after) =
                                ita_check_as(package, &work, ITA_EDITION_0_5);
                            let mut missing = Vec::new();
                            for (list, wanted, which) in [
                                (&mut after, added, "added"),
                                (&mut before, removed, "removed"),
                            ] {
                                for (code, at) in wanted {
                                    match list.iter().position(|(c, a)| c == code && a == at) {
                                        Some(index) => {
                                            list.remove(index);
                                        }
                                        None => missing.push(format!("{which} {code} at {at}")),
                                    }
                                }
                            }
                            let result = if !missing.is_empty() {
                                Err(format!("lacks the listed {}", missing.join(", ")))
                            } else if before != after {
                                Err(format!(
                                    "edition 0.5 reports {after:?} besides the listed diagnostics, and edition 0.4 {before:?}"
                                ))
                            } else if listed.is_none() && status_0_4 != status_0_5 {
                                Err(format!("exits {status_0_5:?} in edition 0.5 and {status_0_4:?} in edition 0.4"))
                            } else {
                                Ok(())
                            };
                            (name.clone(), result)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    let mut wrong: Vec<String> = results
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|why| format!("{name}: {why}")))
        .collect();
    wrong.sort();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The manifest and lock files of a package: the diagnostics there are the migration's, not the
/// edition's.
const MANIFEST_FILES: [&str; 4] = ["una.toml", "verification.lock", "ita.una", "ita.lock"];

/// The code and `file:line:col` of each diagnostic of `document` outside the manifest files.
fn sites_outside_manifest(document: &Value) -> Vec<(String, String)> {
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| !MANIFEST_FILES.contains(&d["primary"]["file"].as_str().unwrap_or("")))
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                location(&d["primary"]),
            )
        })
        .collect()
}

/// Checks `package` in edition `0.3`, then a fresh copy moved to edition `0.4` by `una migrate`,
/// and compares their diagnostics outside the manifest files: those of `0.4` must be those of
/// `0.3`, in order, with exactly `added` among them and exactly `removed` gone.
fn compare_migrated(
    package: &Path,
    work: &Path,
    added: &[(&str, &str)],
    removed: &[(&str, &str)],
) -> Result<(), String> {
    let (_, before, _) = check_as(package, work, EDITION_0_3);
    let before: Value = serde_json::from_slice(&before).unwrap();
    if work.exists() {
        std::fs::remove_dir_all(work).unwrap();
    }
    copy_dir(package, work);
    let una = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .args(args)
            .current_dir(work)
            .output()
            .unwrap()
    };
    let migrated = una(&["migrate", "."]);
    let result = match migrated.status.code() {
        Some(0) => {
            let checked = una(&["--json", "check", "."]);
            assert!(
                expected_status(package, checked.status.code())
                    && checked.stdout.starts_with(DOCUMENT),
                "{} migrated: status {:?}, stdout {:?}, stderr {:?}",
                package.display(),
                checked.status.code(),
                String::from_utf8_lossy(&checked.stdout),
                String::from_utf8_lossy(&checked.stderr)
            );
            let after: Value = serde_json::from_slice(&checked.stdout).unwrap();
            let mut gained = sites_outside_manifest(&after);
            let mut missing = Vec::new();
            for (code, at) in added {
                match gained.iter().position(|(c, a)| c == code && a == at) {
                    Some(index) => {
                        gained.remove(index);
                    }
                    None => missing.push(format!("{code} at {at}")),
                }
            }
            let mut kept = sites_outside_manifest(&before);
            for (code, at) in removed {
                match kept.iter().position(|(c, a)| c == code && a == at) {
                    Some(index) => {
                        kept.remove(index);
                    }
                    None => missing.push(format!(
                        "{code} at {at} (removed, but edition 0.3 lacks it)"
                    )),
                }
            }
            // The migration rewrites the modules in the canonical form of edition 0.4, which
            // changes only white space: each site of edition 0.3 is at the same character.
            let mut rewritten = Vec::new();
            for module in modules(&package.join("src")) {
                let relative = module.strip_prefix(package).unwrap().to_path_buf();
                let old = std::fs::read_to_string(&module).unwrap();
                let new = std::fs::read_to_string(work.join(&relative)).unwrap();
                if !old
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .eq(new.chars().filter(|c| !c.is_whitespace()))
                {
                    rewritten.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
            if !rewritten.is_empty() {
                missing.push(format!(
                    "una migrate changed more than white space in {rewritten:?}"
                ));
            }
            for (_, at) in &mut kept {
                let file = at.rsplitn(3, ':').nth(2).unwrap_or("").to_owned();
                if let (Ok(old), Ok(new)) = (
                    std::fs::read_to_string(package.join(&file)),
                    std::fs::read_to_string(work.join(&file)),
                ) && old != new
                {
                    *at = moved(at, &old, &new);
                }
            }
            if !missing.is_empty() {
                Err(format!(
                    "edition 0.4 lacks the listed {}",
                    missing.join(", ")
                ))
            } else if gained != kept {
                Err(format!(
                    "edition 0.4 reports {gained:?} besides the listed diagnostics, and edition 0.3 {kept:?}"
                ))
            } else {
                Ok(())
            }
        }
        // A refusal: only a package whose manifest edition 0.3 already reports, in any of the
        // manifest files, as a package that has both `una.toml` and `ita.una`.
        Some(2)
            if before["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| MANIFEST_FILES.contains(&d["primary"]["file"].as_str().unwrap_or(""))) =>
        {
            if added.is_empty() && removed.is_empty() {
                Ok(())
            } else {
                Err("listed, but una migrate refuses its manifest".to_owned())
            }
        }
        other => Err(format!(
            "una migrate exits {other:?}: {}",
            String::from_utf8_lossy(&migrated.stderr)
        )),
    };
    std::fs::remove_dir_all(work).unwrap();
    result
}

/// The `.una` files under `dir`, recursively, in a stable order.
fn modules(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    let mut paths: Vec<PathBuf> = entries.map(|entry| entry.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            found.extend(modules(&path));
        } else if path.extension().is_some_and(|extension| extension == "una") {
            found.push(path);
        }
    }
    found
}

/// The site `at`, `file:line:col` of the text `old`, at the same character of `new`, which
/// differs from `old` in white space only: the character there is the one with as many
/// characters that are not white space before it. Columns count Unicode scalar values.
fn moved(at: &str, old: &str, new: &str) -> String {
    let mut parts = at.rsplitn(3, ':');
    let (Some(col), Some(line), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
        return at.to_owned();
    };
    let (Ok(line), Ok(col)) = (line.parse::<usize>(), col.parse::<usize>()) else {
        return at.to_owned();
    };
    // The number of characters that are not white space before the site in `old`.
    let mut before = 0;
    for (index, text) in old.split('\n').enumerate() {
        if index + 1 < line {
            before += text.chars().filter(|c| !c.is_whitespace()).count();
        } else {
            before += text
                .chars()
                .take(col.saturating_sub(1))
                .filter(|c| !c.is_whitespace())
                .count();
            break;
        }
    }
    let mut seen = 0;
    for (index, text) in new.split('\n').enumerate() {
        for (column, c) in text.chars().enumerate() {
            if c.is_whitespace() {
                continue;
            }
            if seen == before {
                return format!("{file}:{}:{}", index + 1, column + 1);
            }
            seen += 1;
        }
    }
    at.to_owned()
}

/// The `file:line:col` of a location of the JSON documents.
fn location(at: &Value) -> String {
    let file = at["file"].as_str().unwrap();
    let start = &at["start"];
    format!("{file}:{}:{}", start["line"], start["col"])
}

/// Every file of a package by its path relative to the package, a module without its leading
/// comment lines, which state the test.
fn package_files(package: &Path) -> std::collections::BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
                continue;
            }
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).unwrap();
            let text = if name.ends_with(".una") {
                text.lines()
                    .skip_while(|line| line.starts_with("--"))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                text
            };
            out.insert(name, text);
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(package, package, &mut out);
    out
}

/// The code and `file:line:col` of each diagnostic of a diagnostics document.
fn sites(document: &Value) -> Vec<(String, String)> {
    document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                location(&d["primary"]),
            )
        })
        .collect()
}

/// Each expected difference is exactly the pinned one, so any other drift of its package still
/// fails.
#[test]
fn every_expected_difference_is_exact() {
    let root = conformance();
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("editions/differences");
    for expected in EXPECTED_DIFFERENCES {
        match expected {
            Difference::Syntax {
                case,
                edition_0_2,
                edition_0_3,
                twin,
                erased,
            } => syntax(
                &root,
                &scratch,
                case,
                *edition_0_2,
                edition_0_3,
                twin,
                *erased,
            ),
            Difference::Stricter { case, edition_0_3 } => {
                stricter(&root, &scratch, case, edition_0_3);
            }
            Difference::Renamed { case, edition_0_3 } => {
                renamed(&root, &scratch, case, edition_0_3);
            }
            Difference::Looser { case, edition_0_2 } => {
                looser(&root, &scratch, case, edition_0_2);
            }
            Difference::Repairs { case, edition_0_3 } => {
                repairs_only(&root, &scratch, case, edition_0_3);
            }
        }
    }
}

/// A [`Difference::Syntax`]: the pinned syntax error in edition `0.2`, a clean check with
/// exactly the pinned obligations in edition `0.3`, and an erasure twin that edition `0.2`
/// accepts and edition `0.3` checks the same.
fn syntax(
    root: &Path,
    scratch: &Path,
    name: &str,
    edition_0_2: (&str, &str),
    edition_0_3: &[&str],
    twin_name: &str,
    erased: (&str, &str),
) {
    let case = root.join(name);
    let twin = root.join(twin_name);

    // Edition 0.2: exactly one diagnostic, the pinned syntax error.
    let (status, stdout, _) = check_as(&case, scratch, EDITION_0_2);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    let (code, at) = edition_0_2;
    assert_eq!(status, Some(1), "{name} as edition 0.2");
    assert_eq!(
        sites(&document),
        vec![(code.to_owned(), at.to_owned())],
        "{name} as edition 0.2"
    );

    // Edition 0.3: a clean check, and exactly the pinned obligations of the construct.
    let (status, stdout, _) = check_as(&case, scratch, EDITION_0_3);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(status, Some(0), "{name} as edition 0.3");
    assert_eq!(sites(&document), Vec::new(), "{name} as edition 0.3");
    let (_, stdout, _) = run_as(&case, scratch, EDITION_0_3, "obligations", OBLIGATIONS);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    let found: Vec<String> = document["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "effect_condition")
        .map(|o| {
            format!(
                "{} at {}",
                o["status"].as_str().unwrap(),
                location(&o["source"])
            )
        })
        .collect();
    assert_eq!(found, edition_0_3, "{name} as edition 0.3");

    // The twin is the case with the construct erased: the same files, each the same, except
    // the one that held the construct exactly once, without it.
    let (file, text) = erased;
    let mut expected = package_files(&case);
    let held = expected
        .get_mut(file)
        .unwrap_or_else(|| panic!("{name}: no file `{file}`"));
    assert_eq!(
        held.matches(text).count(),
        1,
        "{name}: `{text}` is not written exactly once in `{file}`"
    );
    *held = held.replacen(text, "", 1);
    assert_eq!(
        expected,
        package_files(&twin),
        "{twin_name} is not {name} with `{text}` erased from `{file}`"
    );

    // Edition 0.2 accepts the twin, and edition 0.3 checks it the same, with no obligation of
    // the construct.
    let before = check_as(&twin, scratch, EDITION_0_2);
    let document: Value = serde_json::from_slice(&before.1).unwrap();
    assert_eq!(before.0, Some(0), "{twin_name} as edition 0.2");
    assert_eq!(sites(&document), Vec::new(), "{twin_name} as edition 0.2");
    assert_eq!(
        check_as(&twin, scratch, EDITION_0_3),
        before,
        "{twin_name} as edition 0.3"
    );
    let (_, stdout, _) = run_as(&twin, scratch, EDITION_0_3, "obligations", OBLIGATIONS);
    let document: Value = serde_json::from_slice(&stdout).unwrap();
    assert!(
        !document["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "effect_condition"),
        "{twin_name} as edition 0.3"
    );
}

/// A [`Difference::Stricter`]: the whole diagnostics document of edition `0.3` is that of
/// edition `0.2` with exactly the listed diagnostics added, nothing removed and the others in
/// their order, and edition `0.3` exits 1.
fn stricter(root: &Path, scratch: &Path, name: &str, added: &[(&str, &str)]) {
    let case = root.join(name);
    let (_, before, _) = check_as(&case, scratch, EDITION_0_2);
    let (status, after, _) = check_as(&case, scratch, EDITION_0_3);
    let before: Value = serde_json::from_slice(&before).unwrap();
    let after: Value = serde_json::from_slice(&after).unwrap();
    assert!(!added.is_empty(), "{name}: a stricter entry adds nothing");
    assert_eq!(status, Some(1), "{name} as edition 0.3");
    differs_by(name, "edition 0.3", "edition 0.2", after, &before, added);
}

/// A diagnostic's code, line, column and repairs, as [`Difference::Repairs`] pins them.
type PinnedRepairs = (
    &'static str,
    u64,
    u64,
    &'static [(&'static str, &'static str, &'static str)],
);

/// A repair as (widening kind or "", description, edit replacement or "").
type RepairRow = (String, String, String);

/// A diagnostic's code, line, column and repairs, as found.
type FoundRepairs = (String, u64, u64, Vec<RepairRow>);

/// Each repair of `diagnostic` as a [`RepairRow`].
fn repair_rows(diagnostic: &Value) -> Vec<RepairRow> {
    diagnostic["repairs"]
        .as_array()
        .map(|repairs| {
            repairs
                .iter()
                .map(|repair| {
                    (
                        repair["widens"]["kind"].as_str().unwrap_or("").to_owned(),
                        repair["description"].as_str().unwrap_or("").to_owned(),
                        repair["edit"]["replacement"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `text` is one narrowed rebinding added to a statement, ` with @x = @y[ops]` or
/// `, @x = @y[ops]` (ADR 0093 rule 6): it passes the callee only the operations listed, and
/// adds nothing to a header.
fn narrowed_rebinding(text: &str) -> bool {
    let name = |part: &str| {
        part.strip_prefix('@').is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
    };
    text.strip_prefix(" with ")
        .or_else(|| text.strip_prefix(", "))
        .and_then(|binding| binding.split_once(" = "))
        .and_then(|(target, source)| {
            let (held, ops) = source.strip_suffix(']')?.split_once('[')?;
            Some(
                name(target)
                    && name(held)
                    && ops.split(", ").all(|op| {
                        !op.is_empty() && op.chars().all(|c| c.is_alphanumeric() || c == '_')
                    }),
            )
        })
        .unwrap_or(false)
}

/// A [`Difference::Repairs`]: the two documents are the same once each diagnostic's repairs
/// and the `fixable` count are set aside, they differ, no diagnostic of edition `0.3` has fewer
/// repairs than in edition `0.2`, `fixable` never falls, the repairs of edition `0.3` are
/// exactly `pinned`, and every edit that `una fix` may apply in edition `0.3` and not in `0.2`
/// is a narrowed rebinding that widens nothing.
fn repairs_only(root: &Path, scratch: &Path, name: &str, pinned: &[PinnedRepairs]) {
    let case = root.join(name);
    let (before_status, before, _) = check_as(&case, scratch, EDITION_0_2);
    let (after_status, after, _) = check_as(&case, scratch, EDITION_0_3);
    let before: Value = serde_json::from_slice(&before).unwrap();
    let after: Value = serde_json::from_slice(&after).unwrap();
    assert_eq!(before_status, after_status, "{name}");
    assert_ne!(before, after, "{name}: a repairs entry differs in nothing");
    // A narrowing route with an edit makes a diagnostic fixable (ADR 0096 rule 6), never less.
    assert!(
        after["fixable"].as_u64() >= before["fixable"].as_u64(),
        "{name}: edition 0.3 is less fixable"
    );
    let without = |document: &Value| {
        let mut document = document.clone();
        document.as_object_mut().unwrap().remove("fixable");
        for diagnostic in document["diagnostics"].as_array_mut().unwrap() {
            diagnostic.as_object_mut().unwrap().remove("repairs");
        }
        document
    };
    assert_eq!(
        without(&before),
        without(&after),
        "{name}: more than repairs differ"
    );
    let counts = |document: &Value| -> Vec<usize> {
        document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["repairs"].as_array().map_or(0, Vec::len))
            .collect()
    };
    for (old, new) in counts(&before).into_iter().zip(counts(&after)) {
        assert!(new >= old, "{name}: edition 0.3 drops a repair");
    }
    let found: Vec<FoundRepairs> = after["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                d["primary"]["start"]["line"].as_u64().unwrap(),
                d["primary"]["start"]["col"].as_u64().unwrap(),
                repair_rows(d),
            )
        })
        .collect();
    let expected: Vec<FoundRepairs> = pinned
        .iter()
        .map(|(code, line, col, repairs)| {
            (
                (*code).to_owned(),
                *line,
                *col,
                repairs
                    .iter()
                    .map(|(w, d, e)| ((*w).to_owned(), (*d).to_owned(), (*e).to_owned()))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(found, expected, "{name}: the repairs of edition 0.3");
    // An edit `una fix` may apply carries no widening; one that edition 0.2 does not offer is a
    // narrowed rebinding (ADR 0093 rule 6, ADR 0084).
    let applicable = |document: &Value| -> Vec<String> {
        document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(repair_rows)
            .filter(|(widens, _, edit)| widens.is_empty() && !edit.is_empty())
            .map(|(_, _, edit)| edit)
            .collect()
    };
    let old = applicable(&before);
    for edit in applicable(&after) {
        assert!(
            old.contains(&edit) || narrowed_rebinding(&edit),
            "{name}: edition 0.3 makes an edit applicable that is not a narrowed rebinding: {edit:?}"
        );
    }
}

/// A [`Difference::Renamed`]: edition `0.3` reports what edition `0.2` does, with the test
/// declaration named `tests` in the messages that name it, and exactly the pinned diagnostics
/// more.
fn renamed(root: &Path, scratch: &Path, name: &str, added: &[(&str, &str)]) {
    let case = root.join(name);
    let (_, before, _) = check_as(&case, scratch, EDITION_0_2);
    let (_, after, _) = check_as(&case, scratch, EDITION_0_3);
    let mut before: Value = serde_json::from_slice(&before).unwrap();
    let after: Value = serde_json::from_slice(&after).unwrap();
    let mut renamed = 0;
    rename_declaration(&mut before, &mut renamed);
    assert!(renamed > 0, "{name}: a renamed entry renames nothing");
    differs_by(name, "edition 0.3", "edition 0.2", after, &before, added);
}

/// Writes "its declaration and examples" as edition `0.3` does, "its declaration and tests", in
/// every message of `value`, counting the messages changed.
fn rename_declaration(value: &mut Value, renamed: &mut usize) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if key == "message"
                    && let Value::String(text) = entry
                    && text.contains("its declaration and examples")
                {
                    *text =
                        text.replace("its declaration and examples", "its declaration and tests");
                    *renamed += 1;
                } else {
                    rename_declaration(entry, renamed);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                rename_declaration(item, renamed);
            }
        }
        _ => {}
    }
}

/// A [`Difference::Looser`]: edition `0.2` reports exactly the pinned diagnostics more than
/// edition `0.3` does, and nothing else differs.
fn looser(root: &Path, scratch: &Path, name: &str, removed: &[(&str, &str)]) {
    let case = root.join(name);
    let (status, before, _) = check_as(&case, scratch, EDITION_0_2);
    let (_, after, _) = check_as(&case, scratch, EDITION_0_3);
    let before: Value = serde_json::from_slice(&before).unwrap();
    let after: Value = serde_json::from_slice(&after).unwrap();
    assert!(
        !removed.is_empty(),
        "{name}: a looser entry removes nothing"
    );
    assert_eq!(status, Some(1), "{name} as edition 0.2");
    differs_by(name, "edition 0.2", "edition 0.3", before, &after, removed);
}

/// That `more`, the document of `more_edition`, is `less` with exactly the `listed`
/// diagnostics added.
fn differs_by(
    name: &str,
    more_edition: &str,
    less_edition: &str,
    mut more: Value,
    less: &Value,
    listed: &[(&str, &str)],
) {
    // Each listed diagnostic is taken out once, so a duplicate or a missing one fails, and
    // `fixable` loses those that `una fix` applies (`spec/13 §5`).
    let mut fixable = 0;
    let diagnostics = more["diagnostics"].as_array_mut().unwrap();
    for (code, at) in listed {
        let found = diagnostics
            .iter()
            .position(|d| d["code"] == *code && location(&d["primary"]) == *at);
        let Some(index) = found else {
            panic!("{name} as {more_edition}: no {code} at {at}");
        };
        let removed = diagnostics.remove(index);
        let first = &removed["repairs"][0];
        if !first["edit"].is_null()
            && first["escape_cost"] == 0
            && first.get("widens").is_none_or(Value::is_null)
        {
            fixable += 1;
        }
    }
    more["fixable"] = Value::from(more["fixable"].as_u64().unwrap() - fixable);
    assert_eq!(
        &more, less,
        "{name}: {more_edition} differs from {less_edition} by more than the listed diagnostics"
    );
}

/// A dependency of edition 0.4 under a root of edition 0.5 is read under its own edition (ADR
/// 0122 rules 48 and 49): its `let` bindings check, and its diagnostics keep the words of edition
/// 0.4 (ADR 0137 rule 10, review of #2258, H1).
#[test]
fn a_dependency_of_edition_0_4_keeps_its_words_under_a_root_of_edition_0_5() {
    for (case, expected) in [
        ("project/ita_path_dep_0_4_in_0_5", None),
        (
            "project/ita_path_dep_0_4_in_0_5_error",
            Some(
                "`both` is not editable; declare it `let edit both`, or the parameter `edit both`",
            ),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_una"))
            .env("HOME", env!("CARGO_TARGET_TMPDIR"))
            .env_remove("UNA_PROOF_KEY")
            .args(["--json", "check", "."])
            .current_dir(conformance().join(case))
            .output()
            .unwrap();
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        let messages: Vec<&str> = document["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["message"].as_str().unwrap())
            .collect();
        match expected {
            None => assert!(messages.is_empty(), "{case}: {messages:?}"),
            Some(message) => assert_eq!(messages, [message], "{case}"),
        }
    }
}
