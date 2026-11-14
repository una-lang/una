//! The named cases of the owner replay (ADR 0122 Amendment 14, for the kinds this release
//! replays), and the bounded runs of the oracle on the literal reference and on the product's
//! replay: the known failing shapes of the design's table (rows 1 to 27), which the reference
//! and the product must pass once the shape is lifted to the key floor, and must refuse or find
//! unavailable as a lax registry would log them; the rules of acceptance one by one; the
//! exhaustive runs and the seeded walks, on the reference alone and on the product compared
//! with the reference at every question ([`Sut::Both`]); the bound on the replay's work.
//! Test-only.

#![allow(
    clippy::indexing_slicing,
    clippy::unreachable,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::cast_possible_truncation,
    clippy::too_many_lines
)]

use super::alphabet::{
    Config, Shape, exhaustive, log_all, reg as floor, registrations, run_shape, unsupported, walks,
};
use super::contract::{Answer, Sut, Tally, judge, take_disagreements};
use super::model::{Act, K, Keys, Set, Step, set_modeled};
use super::reference::{Reference, Release, Verdict, set_cap, supported};

use Act::{Add, Cosigners, Dispute, Recover, Register, Revoke};
use K::{A, B, C, D, E, F, Log, M, N1, N2, R, T, X, Y};

fn o(act: Act, s: &[K]) -> Step {
    Step::owner(act, s.to_vec())
}
fn t(act: Act, s: &[K]) -> Step {
    Step {
        act,
        signers: s.to_vec(),
        thief: true,
    }
}
fn regc(keys: &[K], threshold: usize, cosigners: Set) -> Step {
    o(
        Register {
            keys: keys.to_vec(),
            threshold,
            cosigners,
        },
        keys,
    )
}

/// The reference of a chain.
fn read(steps: &[Step]) -> Reference {
    Reference::new(steps)
}

/// The verdict on a chain by the product, compared with the reference's ([`Sut::Both`]).
fn verdict_of(steps: &[Step]) -> Verdict {
    let keys = Keys::new();
    let r = Sut::Both.read(&keys, steps);
    if r.refused() {
        Verdict::Refused
    } else if r.unsupported() {
        Verdict::Unsupported
    } else {
        Verdict::Resolves
    }
}

/// Whether a registry logs `step` after `steps`, by the product, compared with the
/// reference's.
fn accepted(steps: &[Step], step: &Step) -> bool {
    Sut::Both.read(&Keys::new(), steps).accepts(step)
}

/// No question on this thread had different answers from the product and the reference.
fn agreed() {
    let (n, examples) = take_disagreements();
    assert_eq!(n, 0, "{examples:#?}");
}

/// Rows 1 to 27 of the table of known failing shapes.
fn shapes() -> Vec<Shape> {
    let x1 = Some((1, vec![X]));
    let xy = Some((2, vec![X, Y]));
    let s = |row, pre: Vec<Step>, theft, tail: Vec<Step>| Shape {
        row,
        pre,
        theft,
        tail,
    };
    vec![
        s(
            "1 original #1997",
            vec![
                regc(&[A], 1, None),
                o(Add(B, 1), &[A, B]),
                t(Add(T, 1), &[A, T]),
                t(Revoke(A, 1, Some(0)), &[T]),
            ],
            1,
            vec![o(Revoke(A, 1, Some(1)), &[B])],
        ),
        s(
            "2 class II (former key)",
            vec![
                regc(&[A, B], 2, None),
                o(Revoke(B, 1, None), &[A, B]),
                t(Revoke(B, 1, Some(0)), &[A]),
            ],
            1,
            vec![],
        ),
        s(
            "2b class II, cosigners",
            vec![
                regc(&[A, B], 2, x1.clone()),
                o(Revoke(B, 1, None), &[A, B]),
                t(Revoke(B, 1, Some(0)), &[A]),
            ],
            1,
            vec![],
        ),
        s(
            "3 class III",
            vec![
                regc(&[A, B], 1, None),
                t(Add(T, 1), &[A, T]),
                t(Revoke(B, 1, Some(0)), &[T]),
                t(Revoke(T, 1, Some(0)), &[A]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B])],
        ),
        s(
            "4 classes IV-VI (account, freeze)",
            vec![
                regc(&[A, B], 1, None),
                o(Dispute(N1), &[Log]),
                t(Dispute(M), &[Log]),
                t(Revoke(B, 1, Some(0)), &[A]),
            ],
            1,
            vec![o(Revoke(A, 1, Some(0)), &[B, A])],
        ),
        s(
            "5 class VII (account, no freeze)",
            vec![
                regc(&[A, B], 1, None),
                t(Add(T, 1), &[A, T]),
                t(Recover(M), &[Log]),
                t(Revoke(B, 1, None), &[T]),
                t(Revoke(T, 1, Some(0)), &[A, T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B, A])],
        ),
        s(
            "6 class VIII",
            vec![
                regc(&[A, B], 2, x1.clone()),
                o(Cosigners(None, 2), &[A, B]),
                o(Revoke(B, 1, None), &[A, B]),
                t(Add(T, 1), &[A, T]),
                t(Revoke(A, 1, Some(0)), &[A, T]),
            ],
            2,
            vec![],
        ),
        s(
            "7 rule-7 chain",
            vec![
                regc(&[A, B], 1, None),
                t(Add(T, 1), &[A, T]),
                t(Revoke(B, 1, Some(0)), &[T]),
                t(Revoke(T, 1, Some(0)), &[A, T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B, A])],
        ),
        s(
            "8 non-settling self-claims",
            vec![
                regc(&[A, B, C], 1, None),
                t(Add(T, 1), &[A, T]),
                t(Revoke(A, 1, Some(0)), &[A]),
                t(Revoke(A, 1, Some(0)), &[A, T]),
            ],
            0,
            vec![],
        ),
        // Signed with keys no key-only thief holds: an unrestricted owner, only [N] and [D] apply.
        s(
            "9 t2 non-settling (unrestricted owner)",
            vec![
                regc(&[A, B, C], 2, None),
                o(Revoke(A, 1, Some(0)), &[A, B]),
                o(Add(T, 1), &[A, C, T]),
                o(Revoke(B, 1, Some(0)), &[A, B, T]),
            ],
            3,
            vec![],
        ),
        s(
            "10 F1",
            vec![
                regc(&[A, B], 1, None),
                t(Add(T, 1), &[A, T]),
                t(Revoke(B, 1, Some(0)), &[A]),
                t(Revoke(A, 1, Some(1)), &[T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B])],
        ),
        s(
            "11 G1/W1",
            vec![
                regc(&[A, B, C], 2, None),
                o(Add(N2, 1), &[A, B, C, N2]),
                t(Revoke(C, 1, Some(0)), &[A]),
                t(Revoke(B, 1, Some(0)), &[A]),
            ],
            1,
            vec![o(Revoke(A, 1, Some(1)), &[N2])],
        ),
        s(
            "12 W2",
            vec![
                regc(&[A, B, C], 1, None),
                t(Add(T, 2), &[A, T]),
                o(Add(N2, 2), &[B, C, N2]),
                t(Revoke(B, 1, Some(0)), &[A, T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[C])],
        ),
        s(
            "13 W3 / round-3 case 2",
            vec![
                regc(&[A, B], 1, None),
                t(Add(T, 1), &[A, T]),
                o(Add(C, 2), &[B, C]),
                t(Revoke(B, 1, Some(2)), &[A, T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[C])],
        ),
        s(
            "14 W4 (co-signer change pending)",
            vec![
                regc(&[A, B, C], 2, xy.clone()),
                o(Revoke(B, 1, None), &[A, B, C]),
                o(Cosigners(x1.clone(), 1), &[C]),
                t(Revoke(C, 1, Some(0)), &[A]),
            ],
            1,
            vec![],
        ),
        s(
            "15 W5",
            vec![
                regc(&[A, B], 1, x1.clone()),
                o(Add(C, 3), &[B, C]),
                t(Revoke(B, 1, Some(0)), &[A]),
            ],
            0,
            vec![],
        ),
        s(
            "16 W6",
            vec![
                regc(&[A, B], 2, x1.clone()),
                o(Recover(N1), &[Log, X]),
                o(Add(C, 1), &[A, B, C]),
                t(Add(T, 1), &[A, T]),
                t(Revoke(C, 3, Some(1)), &[T]),
                t(Revoke(T, 1, Some(3)), &[A]),
            ],
            2,
            vec![o(Revoke(A, 1, Some(2)), &[B, C, A])],
        ),
        s(
            "17 H1",
            vec![
                regc(&[A, B], 1, None),
                o(Add(C, 2), &[A, B, C]),
                o(Revoke(C, 1, None), &[A, B, C]),
                o(Add(N2, 2), &[B, N2]),
                o(Revoke(B, 1, None), &[B, N2]),
                t(Revoke(B, 1, Some(5)), &[A]),
            ],
            2,
            vec![
                o(Revoke(A, 1, Some(2)), &[N2]),
                t(Revoke(B, 1, Some(2)), &[A]),
            ],
        ),
        s(
            "18 H2/Y3",
            vec![
                regc(&[A, B, C], 1, None),
                o(Add(N2, 3), &[C, N2]),
                t(Revoke(C, 1, Some(0)), &[A]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B, C, N2])],
        ),
        s(
            "19 H2/Y1",
            vec![
                regc(&[A, B, C], 1, None),
                t(Add(T, 1), &[A, T]),
                t(Revoke(B, 2, None), &[A]),
                t(Revoke(A, 1, Some(6)), &[A, T]),
                o(Add(N2, 3), &[C, N2]),
                t(Revoke(C, 1, Some(2)), &[T]),
                t(Revoke(B, 1, Some(1)), &[T]),
            ],
            0,
            vec![
                o(Dispute(R), &[Log]),
                Step::owner(Revoke(A, 1, Some(0)), vec![R]),
            ],
        ),
        s(
            "20 H3/P1",
            vec![
                regc(&[A, B, C], 1, x1.clone()),
                o(Revoke(B, 1, Some(0)), &[C]),
                o(Add(N2, 3), &[C, N2]),
                t(Revoke(C, 1, Some(0)), &[A]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B, C, N2])],
        ),
        s(
            "21 rev2000c (P), post-response moves",
            vec![
                regc(&[A, B], 1, None),
                o(Add(C, 2), &[A, B, C]),
                o(Revoke(C, 1, None), &[A, B, C]),
                o(Add(N2, 2), &[B, N2]),
                o(Revoke(B, 1, None), &[B, N2]),
                t(Revoke(B, 1, Some(5)), &[A]),
            ],
            2,
            vec![
                o(Revoke(A, 1, Some(2)), &[N2]),
                t(Revoke(B, 1, Some(2)), &[A]),
                t(Revoke(C, 1, Some(7)), &[A, T]),
                t(Revoke(N2, 1, Some(0)), &[A]),
            ],
        ),
        s(
            "22 rev2000c (R3)/(R2c)/YANK(3), W3 releases",
            vec![
                regc(&[A, B], 1, None),
                t(Add(T, 1), &[A, T]),
                o(Add(C, 2), &[B, C]),
                t(Revoke(B, 1, Some(2)), &[A, T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B, C])],
        ),
        s(
            "23 rev2000c YANK(2), owner self-claim (M1)",
            vec![
                regc(&[A, B, C], 1, None),
                o(Revoke(B, 1, Some(0)), &[C]),
                t(Add(T, 1), &[A, T]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[C])],
        ),
        s(
            "24 rev2000c (I), P1 without cosigners",
            vec![
                regc(&[A, B, C], 1, None),
                o(Revoke(B, 1, Some(0)), &[C]),
                o(Add(N2, 3), &[C, N2]),
                t(Revoke(C, 1, Some(0)), &[A]),
            ],
            0,
            vec![o(Revoke(A, 1, Some(0)), &[B, C, N2])],
        ),
        s(
            "25 rev2000c (Q), thief dispute after the response",
            vec![regc(&[A, B], 1, None), t(Add(T, 1), &[A, T])],
            0,
            vec![
                o(Revoke(A, 1, Some(0)), &[B]),
                t(Dispute(M), &[Log]),
                o(Dispute(R), &[Log]),
            ],
        ),
        s(
            "26 rev2000b walks, G1 family (cosigners, B removed)",
            vec![
                regc(&[A, B, C], 2, xy.clone()),
                o(Add(N2, 1), &[A, B, C, N2]),
                o(Revoke(C, 1, Some(2)), &[A, B, N2]),
                o(Revoke(B, 1, None), &[A, B, N2]),
                t(Revoke(C, 1, Some(0)), &[A]),
                t(Revoke(B, 1, Some(0)), &[A]),
            ],
            3,
            vec![o(Revoke(A, 1, Some(3)), &[N2])],
        ),
        s(
            "27 rev2000 walks, F1 family (cosigners, owner add C)",
            vec![
                regc(&[A, B], 1, x1),
                o(Add(C, 1), &[A, B, C]),
                t(Add(T, 1), &[A, T]),
                t(Revoke(B, 1, Some(0)), &[A]),
                t(Revoke(A, 1, Some(2)), &[T]),
            ],
            1,
            vec![o(Revoke(A, 1, Some(1)), &[B, C])],
        ),
    ]
}

/// A shape lifted to the key floor: its registration with keys added (`D`, then `E`) up to one
/// more than a threshold of at least 2, and no cosigners; every other entry as it was, so the
/// registry refuses those the floor or their threshold no longer allows.
fn lift(shape: &Shape) -> Shape {
    let Register {
        keys, threshold, ..
    } = &shape.pre[0].act
    else {
        unreachable!("a shape starts with its registration")
    };
    let threshold = (*threshold).max(2);
    let mut keys = keys.clone();
    for k in [D, E] {
        if keys.len() < threshold + 1 {
            keys.push(k);
        }
    }
    let mut pre = vec![floor(&keys, threshold)];
    pre.extend(shape.pre[1..].iter().cloned());
    Shape {
        row: shape.row,
        pre,
        theft: shape.theft,
        tail: shape.tail.clone(),
    }
}

/// Each known shape: the reference and the product pass the oracle on the shape lifted to the
/// key floor, answering alike; the shape as a lax registry logged it never resolves, on either
/// (its registration is below the floor, an entry breaks a rule, or it holds a kind this
/// release does not replay); and a row's first entry of such a kind, after a registration at
/// the floor, makes the names unavailable.
#[test]
fn the_oracle_sees_every_known_shape() {
    assert!(known_shapes() > 0);
}

/// The body of [`the_oracle_sees_every_known_shape`]: the number of shapes checked.
fn known_shapes() -> usize {
    let keys = Keys::new();
    let mut count = 0;
    let (mut fails, mut lax_resolves, mut not_unsupported) = (Vec::new(), Vec::new(), Vec::new());
    for shape in shapes() {
        count += 1;
        Config { cap: 64 }.apply();
        let lifted = lift(&shape);
        let reference = run_shape(Sut::Reference, &keys, &lifted);
        let product = run_shape(Sut::Both, &keys, &lifted);
        let lax: Vec<Step> = [shape.pre.clone(), shape.tail.clone()].concat();
        let verdict = verdict_of(&lax);
        println!(
            "=== row {}: as a lax registry logged it, {verdict:?} {:?}",
            shape.row,
            read(&lax).unacceptable(lax.len()).first()
        );
        println!("{}", reference.report("reference, lifted to the key floor"));
        println!("{}", product.report("product, lifted to the key floor"));
        if reference.total() > 0 || product.total() > 0 {
            fails.push(shape.row);
        }
        if verdict == Verdict::Resolves {
            lax_resolves.push(shape.row);
        }
        if let Some(first) = lax.iter().find(|s| !supported(&s.act)) {
            let u = if matches!(first.act, Register { .. }) {
                vec![first.clone()]
            } else {
                vec![lifted.pre[0].clone(), first.clone()]
            };
            if verdict_of(&u) != Verdict::Unsupported {
                not_unsupported.push(shape.row);
            }
        }
    }
    agreed();
    set_modeled(false);
    assert!(fails.is_empty(), "{fails:?}");
    assert!(lax_resolves.is_empty(), "{lax_resolves:?}");
    assert!(not_unsupported.is_empty(), "{not_unsupported:?}");
    count
}

/// The rules of acceptance of this release, one by one, on the product, compared with the
/// reference at every question.
#[test]
fn the_rules_of_acceptance_one_by_one() {
    rules_of_acceptance();
}

/// The body of [`the_rules_of_acceptance_one_by_one`].
fn rules_of_acceptance() {
    set_modeled(true);
    set_cap(64);
    let accepts = |steps: &[Step], step: Step| accepted(steps, &step);
    let verdict = verdict_of;
    // The key floor at registration: at least three keys, a threshold of at least 2 and at
    // least one key more than it.
    for (keys, th) in [
        (&[A, B][..], 1),
        (&[A, B], 2),
        (&[A, B, C], 1),
        (&[A, B, C], 3),
    ] {
        assert_eq!(
            verdict(&[regc(keys, th, None)]),
            Verdict::Refused,
            "{keys:?} {th}"
        );
    }
    assert_eq!(verdict(&[floor(&[A, B, C], 2)]), Verdict::Resolves);
    // Every kind this release does not replay makes the names unavailable, and no registry of
    // this release logs it: a registration with cosigners, and each request kind after one.
    let x1 = Some((1, vec![X]));
    assert_eq!(verdict(&[regc(&[A, B, C], 2, x1)]), Verdict::Unsupported);
    let base = vec![floor(&[A, B, C], 2)];
    for u in unsupported(2) {
        assert!(!accepts(&base, u.clone()), "{:?}", u.act);
        let with = [base.clone(), vec![u.clone()]].concat();
        assert_eq!(verdict(&with), Verdict::Unsupported, "{:?}", u.act);
        assert!(!read(&with).counts(&[A, B, C], 1, 2), "{:?}", u.act);
    }
    // An entry that breaks a rule before an unsupported one refuses the names.
    let broken = [
        base.clone(),
        vec![o(Add(D, 1), &[A, B, D]), o(Dispute(N1), &[Log])],
    ]
    .concat();
    assert_eq!(verdict(&broken), Verdict::Refused);
    // One stolen key logs nothing: every requirement needs two counted signers.
    for step in [
        t(Add(T, 2), &[A, T]),
        t(Revoke(B, 2, Some(0)), &[A]),
        t(Revoke(B, 2, None), &[A]),
        t(Revoke(B, 1, Some(0)), &[A, T]),
    ] {
        assert!(!accepts(&base, step.clone()), "{:?}", step.act);
    }
    // At the floor the owner's claim alone would leave it below it; it adds a key first.
    assert!(!accepts(&base, o(Revoke(A, 2, Some(0)), &[B, C])));
    assert!(!accepts(&base, o(Revoke(B, 2, None), &[A, B, C])));
    let added = [base.clone(), vec![o(Add(D, 2), &[B, C, D])]].concat();
    assert_eq!(verdict(&added), Verdict::Resolves);
    let claim = o(Revoke(A, 2, Some(0)), &[B, C]);
    assert!(accepts(&added, claim.clone()));
    let answered = [added.clone(), vec![claim]].concat();
    let r = read(&answered);
    assert_eq!(r.verdict(3), Verdict::Resolves);
    assert!(r.counts(&[B, C], 3, 3) && !r.counts(&[A, D], 1, 3) && r.counts(&[B, D], 3, 3));
    // An addition that leaves the threshold below 2, or too few keys over it, is refused.
    assert!(!accepts(&base, o(Add(D, 1), &[A, B, D])));
    assert!(!accepts(&base, o(Add(D, 4), &[A, B, D])));
    assert!(accepts(&base, o(Add(D, 3), &[A, B, D])));
    // A claim signed by the key it names, or stating a `p` at or after its own index, is not
    // logged.
    assert!(!accepts(&added, o(Revoke(A, 2, Some(0)), &[A, B, C])));
    assert!(!accepts(&added, o(Revoke(A, 2, Some(2)), &[B, C])));
    assert!(!accepts(&added, o(Revoke(A, 2, Some(3)), &[B, C])));
    // A claim that repeats a compromise that already counts is redundant; one from an earlier
    // `p` is not.
    let later = [added.clone(), vec![o(Revoke(A, 2, Some(1)), &[B, C])]].concat();
    assert_eq!(verdict(&later), Verdict::Resolves);
    assert!(!accepts(&later, o(Revoke(A, 2, Some(1)), &[B, C])));
    assert!(accepts(&later, o(Revoke(A, 2, Some(0)), &[B, C])));
    // Names are never used again.
    assert!(!accepts(&answered, o(Add(A, 2), &[B, C, A])));
    // The most claims of a chain: at a cap of one, a second claim is not logged.
    set_cap(1);
    let five = vec![
        floor(&[A, B, C, D, E], 2),
        o(Revoke(E, 2, Some(0)), &[A, B]),
    ];
    assert_eq!(verdict(&five), Verdict::Resolves);
    assert!(!accepts(&five, o(Revoke(D, 2, Some(0)), &[A, B])));
    set_cap(2);
    assert!(accepts(&five, o(Revoke(D, 2, Some(0)), &[A, B])));
    set_cap(64);
    // A claim on a former key keeps the keys and the threshold, by the current threshold.
    let retired = [added.clone(), vec![o(Revoke(D, 2, None), &[A, B, C])]].concat();
    assert_eq!(verdict(&retired), Verdict::Resolves);
    assert!(accepts(&retired, o(Revoke(D, 2, Some(1)), &[B, C])));
    assert!(!accepts(&retired, o(Revoke(D, 1, Some(1)), &[B, C])));
    // Two stolen keys meet a threshold of 2: the attacker controls the name (not covered).
    assert!(accepts(&base, t(Add(T, 2), &[A, B, T])));
    // An owner claim after the response, not of the honest form, can undo the response (B3):
    // the claim on B since 0 taints the response's signer B.
    let undo = vec![
        floor(&[A, B, C, D], 2),
        o(Revoke(A, 2, Some(0)), &[B, C]),
        o(Add(E, 2), &[C, D, E]),
        o(Revoke(B, 2, Some(0)), &[C, D]),
    ];
    let (logged, refused) = log_all(Sut::Reference, &Keys::new(), &undo);
    assert!(refused.is_empty(), "{refused:?}");
    let r = read(&logged);
    let end = r.ctx(logged.len());
    assert!(
        !end.sem.effective.contains(&1),
        "the response no longer counts"
    );
    assert!(end.state.keys.contains(&A), "A is current again");
    // The key floor counts keys, not keys that can still sign (`spec/30 §16` rule 1): an owner
    // that retires `D` with a plain revocation `D` signs, then claims `D` from before that
    // signature (not of the honest form), voids its own revocation, so `D` is current again and
    // compromised. The owner stays at the floor by count, four keys at threshold 3, with three
    // keys that can sign: without `A` it can log no key event, and only a new owner name helps
    // an owner whose `A` is stolen.
    let dead = vec![
        floor(&[A, B, C, D], 3),
        o(Revoke(D, 2, None), &[B, C, D]),
        o(Revoke(D, 2, Some(0)), &[B, C]),
    ];
    let (logged, refused) = log_all(Sut::Reference, &Keys::new(), &dead);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(verdict(&logged), Verdict::Resolves);
    let end = read(&logged).ctx(logged.len()).clone();
    assert_eq!((end.state.keys.len(), end.state.threshold), (4, 3));
    assert!(end.state.keys.contains(&D), "D is current again");
    assert!(!accepts(&logged, o(Add(F, 3), &[B, C, F])));
    assert!(!accepts(&logged, o(Revoke(A, 3, Some(0)), &[B, C])));
    assert!(accepts(&logged, o(Add(F, 3), &[A, B, C, F])));
    // A response signed by the key it names is not loggable, but the owner's response check
    // passes for the prescribed one.
    let keys = Keys::new();
    let read_added = Sut::Reference.read(&keys, &added);
    assert!(read_added.accepts(&o(Revoke(A, 2, Some(0)), &[B, C])));
    assert!(!read_added.accepts(&o(Revoke(A, 2, Some(0)), &[A, B, C])));
    // Rule 26 against a release a later claim skips: noted, never a refusal (two stolen keys).
    let f7 = vec![
        floor(&[A, B, C, D], 2),
        t(Revoke(C, 2, Some(0)), &[A, B]),
        o(Revoke(A, 2, Some(0)), &[C, D]),
    ];
    let (f7, refused) = log_all(Sut::Reference, &keys, &f7);
    println!("rule 26 chain: {f7:?}, refused {refused:?}");
    let r = read(&f7);
    let list = vec![(1, vec![C, D], 1), (2, vec![A, B], 2), (2, vec![C, D], 2)];
    let judged = r.releases(&list, f7.len());
    assert!(!judged.contains(&Release::Refuses), "{judged:?}");
    agreed();
    set_modeled(false);
}

/// The oracle on a literal chain with its response logged: the failures.
fn oracle(keys: &Keys, chain: &[Step], theft: u64) -> u64 {
    let end = Sut::Reference.read(keys, chain);
    let mut tally = Tally::default();
    let a = Answer {
        chain,
        theft,
        stolen: &[A],
        responded: true,
        since: theft,
        honest: true,
    };
    judge(&end, &a, &mut tally);
    if tally.total() > 0 {
        println!("{}", tally.report("oracle"));
    }
    tally.total()
}

/// The response at the floor, by an addition and the claim, through the oracle.
#[test]
fn the_response_at_the_floor() {
    response_at_the_floor();
}

/// The body of [`the_response_at_the_floor`].
fn response_at_the_floor() {
    let keys = Keys::new();
    set_modeled(true);
    set_cap(64);
    let chain = vec![
        floor(&[A, B, C], 2),
        o(Add(D, 2), &[B, C, D]),
        o(Revoke(A, 2, Some(0)), &[B, C]),
    ];
    let (logged, refused) = log_all(Sut::Reference, &keys, &chain);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(oracle(&keys, &logged, 0), 0);
    set_modeled(false);
}

fn summary(name: &str, tally: &Tally, secs: f64) {
    println!("{}", tally.report(name));
    println!("{name}: {secs:.1}s");
}

/// The coverage counters that a full run of the one-key mode must reach.
const COVERAGE: [&str; 9] = [
    "F4 refused",
    "redundant refused",
    "attacker candidates refused",
    "owner candidates refused",
    "unsupported entries",
    "key path by the claim",
    "key path by an addition then the claim",
    "post-response attacker candidates refused",
    "yanks after the response",
];

fn covered(tally: &Tally) {
    for c in COVERAGE {
        assert!(tally.count(c) > 0, "coverage counter {c} is zero");
    }
}

/// The reference against the oracle, every chain to `depth`: the one-key mode at the caps 2,
/// 3 and 64, and the two-key mode at 64.
fn reference_to(depth: usize) -> u64 {
    let mut total = 0;
    for (cap, two) in [(2, false), (3, false), (64, false), (64, true)] {
        let start = std::time::Instant::now();
        let tally = exhaustive(
            Sut::Reference,
            depth,
            registrations(),
            8,
            Config { cap },
            two,
        );
        let name = format!("reference depth {depth} cap {cap} two keys {two}");
        summary(&name, &tally, start.elapsed().as_secs_f64());
        if !two {
            covered(&tally);
        }
        total += tally.total();
    }
    total
}

#[test]
#[ignore = "the reference against the oracle, exhaustive to depth 3"]
fn the_reference_holds_the_contract_to_depth_three() {
    assert_eq!(reference_to(3), 0);
}

#[test]
#[ignore = "the reference against the oracle, exhaustive to depth 4"]
fn the_reference_holds_the_contract_to_depth_four() {
    assert_eq!(reference_to(4), 0);
}

#[test]
#[ignore = "the reference against the oracle, exhaustive to depth 5"]
fn the_reference_holds_the_contract_to_depth_five() {
    assert_eq!(reference_to(5), 0);
}

/// The reference on seeded walks of depths 6 to 9: four seeds of 160,000 walks in the one-key
/// mode, cap 3, and two seeds in the two-key mode.
#[test]
#[ignore = "the reference against the oracle, seeded walks of depths 6 to 9"]
fn the_reference_holds_the_contract_on_seeded_walks() {
    let mut total = 0;
    for (seed, two) in [
        (11, false),
        (23, false),
        (37, false),
        (41, false),
        (11, true),
        (23, true),
    ] {
        let start = std::time::Instant::now();
        let tally = walks(
            Sut::Reference,
            160_000,
            seed,
            (6, 9),
            8,
            Config { cap: 3 },
            two,
        );
        let name = format!("reference walks seed {seed} two keys {two}");
        summary(&name, &tally, start.elapsed().as_secs_f64());
        total += tally.total();
    }
    assert_eq!(total, 0);
}

/// The product against the oracle and against the reference, every chain to `depth` in the
/// configurations of [`reference_to`]: the oracle's failures plus the disagreements.
fn product_to(depth: usize) -> u64 {
    let mut all = Tally::default();
    for (cap, two) in [(2, false), (3, false), (64, false), (64, true)] {
        let start = std::time::Instant::now();
        let tally = exhaustive(Sut::Both, depth, registrations(), 8, Config { cap }, two);
        let name = format!("product depth {depth} cap {cap} two keys {two}");
        summary(&name, &tally, start.elapsed().as_secs_f64());
        if !two {
            covered(&tally);
        }
        assert!(tally.count("Ed25519 samples") > 0, "no Ed25519 sample");
        all.merge(tally);
    }
    gate_line(&format!("depth {depth}"), &all);
    all.total()
}

/// The line the `owner-replay` release gate reads: the run's deterministic counts, which the
/// gate compares with the values it records, and its failures and disagreements.
fn gate_line(run: &str, tally: &Tally) {
    println!(
        "owner-replay: {run} chains {} responses {} walks {} unsupported {} failures {}",
        tally.count("chains"),
        tally.count("responses"),
        tally.count("walks"),
        tally.count("unsupported entries"),
        tally.total()
    );
}

#[test]
fn the_replay_agrees_with_the_reference_to_depth_three() {
    assert_eq!(product_to(3), 0);
}

#[test]
#[ignore = "the product against the oracle and the reference, exhaustive to depth 4"]
fn the_replay_agrees_with_the_reference_to_depth_four() {
    assert_eq!(product_to(4), 0);
}

#[test]
#[ignore = "the product against the oracle and the reference, exhaustive to depth 5"]
fn the_replay_agrees_with_the_reference_to_depth_five() {
    assert_eq!(product_to(5), 0);
}

/// The product on seeded walks of depths 6 to 9, 160,000 walks a seed at a cap of 3 claims,
/// compared with the reference at every question: the oracle's failures plus the
/// disagreements, and the gate's summary line for `run`.
fn product_walks(run: &str, seeds: &[(u64, bool)]) -> u64 {
    let mut all = Tally::default();
    for (seed, two) in seeds {
        let start = std::time::Instant::now();
        let tally = walks(
            Sut::Both,
            160_000,
            *seed,
            (6, 9),
            8,
            Config { cap: 3 },
            *two,
        );
        let name = format!("product walks seed {seed} two keys {two}");
        summary(&name, &tally, start.elapsed().as_secs_f64());
        if !two {
            covered(&tally);
        }
        all.merge(tally);
    }
    gate_line(run, &all);
    all.total()
}

/// The walks of the `owner-replay` release gate: seeds 11 and 23 with one stolen key, seed 11
/// with two.
#[test]
#[ignore = "the product against the oracle and the reference, seeded walks of depths 6 to 9"]
fn the_replay_agrees_with_the_reference_on_seeded_walks() {
    assert_eq!(
        product_walks("walks", &[(11, false), (23, false), (11, true)]),
        0
    );
}

/// The walks of the soak (card #2018), beside exhaustive depth 5 on the whole alphabet: seeds
/// 37 and 41 with one stolen key, seed 23 with two.
#[test]
#[ignore = "the product against the oracle and the reference, more seeded walks (a soak)"]
fn the_replay_agrees_with_the_reference_on_more_seeded_walks() {
    assert_eq!(
        product_walks("more walks", &[(37, false), (41, false), (23, true)]),
        0
    );
}

/// The narrow alphabet of exhaustive depth 5 in the `owner-replay` release gate: the least
/// registration at the key floor, three keys at threshold 2, where the owner holds exactly one
/// key more than its threshold, so every response needs an addition first unless the owner
/// added a key before; every other move of the alphabet is kept. Measured: with the
/// registrations of four keys at thresholds 2 and 3 too, depth 5 took about 50 minutes on a
/// loaded machine (0 failures), over the gate's budget.
fn narrow() -> Vec<Step> {
    vec![floor(&[A, B, C], 2)]
}

/// The product against the oracle and the reference, every chain to `depth` from the
/// registrations `regs`, in the configurations of [`reference_to`], each configuration on its
/// own thread: the oracle's failures plus the disagreements, and the gate's summary line for
/// `run`.
fn product_from(run: &str, depth: usize, regs: &[Step]) -> u64 {
    let handles: Vec<_> = [(2, false), (3, false), (64, false), (64, true)]
        .into_iter()
        .map(|(cap, two)| {
            let (regs, run) = (regs.to_vec(), run.to_owned());
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    let start = std::time::Instant::now();
                    let tally = exhaustive(Sut::Both, depth, regs, 8, Config { cap }, two);
                    let name = format!("product {run} cap {cap} two keys {two}");
                    summary(&name, &tally, start.elapsed().as_secs_f64());
                    if !two {
                        covered(&tally);
                    }
                    assert!(tally.count("Ed25519 samples") > 0, "no Ed25519 sample");
                    tally
                })
                .expect("a thread")
        })
        .collect();
    let mut all = Tally::default();
    for h in handles {
        all.merge(h.join().expect("a configuration"));
    }
    gate_line(run, &all);
    all.total()
}

/// Exhaustive depth 5 on the narrow alphabet ([`narrow`]), in the `owner-replay` release gate.
#[test]
#[ignore = "the product against the oracle and the reference, exhaustive to depth 5 on the narrow alphabet"]
fn the_replay_agrees_with_the_reference_to_depth_five_on_the_narrow_alphabet() {
    assert_eq!(product_from("depth 5 narrow", 5, &narrow()), 0);
}

/// The named rows in the `owner-replay` release gate: the known shapes, the rules of acceptance
/// one by one and the response at the floor, which the ordinary run also runs, and the gate's
/// summary line.
#[test]
#[ignore = "the named rows, as the owner-replay release gate runs them"]
fn the_named_rows_hold() {
    let shapes = known_shapes();
    rules_of_acceptance();
    response_at_the_floor();
    println!("owner-replay: named rows shapes {shapes} failures 0");
}

/// A chain of `claims` claims on the former key `D` of an owner registered with `A` to `E` at
/// threshold 2, after `claims` / 2 + 1 fillers, each an addition of a fresh key and its plain
/// revocation, so that every claim's `p` is before it; each claim from an earlier position than
/// the last, signed by `A` and `B`: the product's replay, signatures by the model's mark.
fn claims_chain(keys: &Keys, claims: usize) -> Result<super::History, super::ReplayError> {
    use super::super::note::{Key, Note, Signature};
    use super::{Entry, Event, Logged, replay};
    let mark = |text: String, signers: &[&Key]| Note {
        text,
        signatures: signers
            .iter()
            .map(|k| Signature {
                name: k.name.clone(),
                id: k.id(),
                bytes: super::model::MARK.to_vec(),
            })
            .collect(),
    };
    let (a, b, c) = (keys.key(A), keys.key(B), keys.key(C));
    let five: Vec<Key> = [A, B, C, D, E]
        .iter()
        .map(|k| keys.key(*k).clone())
        .collect();
    let mut events: Vec<(Event, usize, Vec<Key>)> = vec![(
        Event::Register {
            keys: five.clone(),
            cosigners: None,
        },
        2,
        five,
    )];
    let revoke = |name: &str, since| Event::Revoke {
        name: name.to_owned(),
        since,
    };
    events.push((
        revoke(&keys.key(D).name, None),
        2,
        vec![a.clone(), b.clone(), c.clone()],
    ));
    for i in 0..=claims / 2 {
        let fresh = Key {
            name: format!("o.filler{i}"),
            public: [u8::try_from(i % 200).expect("a byte") + 1; 32],
        };
        events.push((
            Event::Add(fresh.clone()),
            2,
            vec![a.clone(), b.clone(), fresh.clone()],
        ));
        events.push((revoke(&fresh.name, None), 2, vec![a.clone(), b.clone()]));
    }
    for i in 0..claims {
        let since = Some((claims - 1 - i) as u64);
        events.push((
            revoke(&keys.key(D).name, since),
            2,
            vec![a.clone(), b.clone()],
        ));
    }
    let mut notes = Vec::new();
    let mut entries = Vec::new();
    for (index, (event, threshold, signers)) in events.into_iter().enumerate() {
        let signers: Vec<&Key> = signers.iter().collect();
        notes.push(mark(format!("work {index}\n"), &signers));
        entries.push(Entry {
            source: "model".to_owned(),
            owner: "acme".to_owned(),
            previous: None,
            event,
            threshold,
        });
    }
    let logged: Vec<Logged<'_>> = notes
        .iter()
        .zip(&entries)
        .enumerate()
        .map(|(index, (note, entry))| Logged {
            index: index as u64,
            note,
            entry,
        })
        .collect();
    replay("acme", "model", &logged)
}

/// The replay's work is bounded by the claims of a chain: C claims make at most
/// C(C + 1)/2 + 2C + 2 forward passes ([`super::History::passes`]), and a claim past the most
/// claims of a chain ([`super::MAX_CLAIMS`]) is not logged.
#[test]
fn replay_work_is_bounded_by_the_claims_of_a_chain() {
    let keys = Keys::new();
    set_modeled(true);
    set_cap(super::MAX_CLAIMS);
    for c in [0, 1, 2, 8, 32, super::MAX_CLAIMS] {
        let h = claims_chain(&keys, c).expect("a chain within the most claims");
        let bound = (c * (c + 1) / 2 + 2 * c + 2) as u64;
        println!("{c} claims: {} passes, bound {bound}", h.passes);
        assert!(h.passes <= bound, "{c} claims: {} passes", h.passes);
        assert_eq!(h.compromised.len(), usize::from(c > 0));
    }
    assert!(matches!(
        claims_chain(&keys, super::MAX_CLAIMS + 1),
        Err(super::ReplayError::Unacceptable(_))
    ));
    set_modeled(false);
}
