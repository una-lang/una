//! The hash encoding's two properties (ADR 0119 rule 9): equal values encode equally, whatever
//! their representation, and distinct values of one type encode differently.
use una_hash::Encoder;

fn enc(f: impl FnOnce(&mut Encoder<'_, Vec<u8>>)) -> Vec<u8> {
    let mut out = Vec::new();
    f(&mut Encoder::new(&mut out));
    out
}

/// The magnitude of `v`, little-endian, with `pad` zero bytes at its end.
fn magnitude(v: u128, pad: usize) -> Vec<u8> {
    let mut m: Vec<u8> = v.to_le_bytes().to_vec();
    while m.last() == Some(&0) {
        m.pop();
    }
    m.extend(std::iter::repeat_n(0, pad));
    m
}

#[test]
fn a_small_integer_is_the_byte_0_and_8_bytes_little_endian() {
    assert_eq!(enc(|e| e.int(1)), [0, 1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(enc(|e| e.int(-1)), [0, 255, 255, 255, 255, 255, 255, 255, 255]);
    assert_eq!(enc(|e| e.int(0)), [0; 9]);
}

#[test]
fn a_promoted_int_encodes_as_a_tagged_one_of_the_same_value() {
    for v in [0i64, 1, -1, 42, -42, i64::MAX, i64::MIN, i64::MAX - 1, i64::MIN + 1] {
        let tagged = enc(|e| e.int(v));
        let m = magnitude(u128::from(v.unsigned_abs()), 0);
        for pad in [0, 1, 7, 9] {
            let promoted = enc(|e| e.big(v < 0, &magnitude(u128::from(v.unsigned_abs()), pad)));
            assert_eq!(promoted, tagged, "{v} with {pad} zero bytes");
        }
        assert_eq!(enc(|e| e.big(v < 0, &m)), tagged);
    }
}

#[test]
fn a_u64_from_2_to_the_63_takes_the_big_form_as_an_int_of_that_value() {
    for v in [1u64 << 63, u64::MAX, (1u64 << 63) + 5] {
        let unsigned = enc(|e| e.uint(v));
        let int = enc(|e| e.big(false, &magnitude(u128::from(v), 2)));
        assert_eq!(unsigned, int, "{v}");
        assert_eq!(unsigned.first(), Some(&1));
        assert_eq!(unsigned.get(1), Some(&0), "positive");
        assert_eq!(unsigned.get(2..10), Some(8u64.to_le_bytes().as_slice()), "an 8-byte magnitude");
    }
    // Up to 2^63 - 1, the small form.
    assert_eq!(enc(|e| e.uint(5)), enc(|e| e.int(5)));
    assert_eq!(enc(|e| e.uint(i64::MAX as u64)), enc(|e| e.int(i64::MAX)));
}

#[test]
fn the_big_form_has_no_zero_byte_at_the_end_of_its_magnitude() {
    let v = 1u128 << 70;
    let a = enc(|e| e.big(true, &magnitude(v, 0)));
    let b = enc(|e| e.big(true, &magnitude(v, 5)));
    assert_eq!(a, b);
    assert_eq!(a.first(), Some(&1));
    assert_eq!(a.get(1), Some(&1), "negative");
    assert_eq!(a.last().copied().map(|x| x != 0), Some(true));
}

#[test]
fn small_values_of_every_kind_share_the_integer_form() {
    assert_eq!(enc(|e| e.bool(true)), enc(|e| e.int(1)));
    assert_eq!(enc(|e| e.bool(false)), enc(|e| e.int(0)));
    assert_eq!(enc(|e| e.unit()), enc(|e| e.int(0)));
    assert_eq!(enc(|e| e.index(3)), enc(|e| e.int(3)));
    assert_eq!(enc(|e| e.f32_bits(0x3f80_0000)), enc(|e| e.int(0x3f80_0000)));
}

#[test]
fn an_f64_is_the_byte_2_and_its_bits() {
    let one = enc(|e| e.f64_bits(1.0f64.to_bits()));
    assert_eq!(one.first(), Some(&2));
    assert_eq!(one.get(1..), Some(1.0f64.to_bits().to_le_bytes().as_slice()));
    // -0.0 and 0.0 differ, as their `==` does in Una.
    assert_ne!(enc(|e| e.f64_bits(0.0f64.to_bits())), enc(|e| e.f64_bits((-0.0f64).to_bits())));
}

#[test]
fn a_text_is_its_length_then_its_bytes() {
    let t = enc(|e| e.bytes(b"ab"));
    let mut expected = enc(|e| e.int(2));
    expected.extend_from_slice(b"ab");
    assert_eq!(t, expected);
}

#[test]
fn distinct_values_of_one_type_encode_differently() {
    // Texts that a missing length would confuse.
    let pairs: [(&[&[u8]], &[&[u8]]); 3] = [
        (&[b"ab", b"c"], &[b"a", b"bc"]),
        (&[b"", b"x"], &[b"x", b""]),
        (&[b"a"], &[b"a", b""]),
    ];
    for (a, b) in pairs {
        let ea = enc(|e| {
            e.len(a.len());
            for t in a {
                e.bytes(t);
            }
        });
        let eb = enc(|e| {
            e.len(b.len());
            for t in b {
                e.bytes(t);
            }
        });
        assert_ne!(ea, eb, "{a:?} and {b:?}");
    }
    // Integers on both sides of the small range.
    let mut seen = std::collections::BTreeSet::new();
    for v in [i64::MIN, -1, 0, 1, i64::MAX] {
        assert!(seen.insert(enc(|e| e.int(v))));
    }
    for v in [1u128 << 63, (1u128 << 64) - 1, 1u128 << 64, 1u128 << 100] {
        assert!(seen.insert(enc(|e| e.big(false, &magnitude(v, 0)))), "+{v}");
        assert!(seen.insert(enc(|e| e.big(true, &magnitude(v + 1, 0)))), "-{}", v + 1);
    }
}
