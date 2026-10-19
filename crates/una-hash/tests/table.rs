//! The ordered hash table and the keys (ADR 0119 rules 9, 10 and 19).
use una_hash::{OrderedTable, SipKey, hash, index_capacity};

const TEST_KEY: SipKey = SipKey { k0: 0x0706_0504_0302_0100, k1: 0x0f0e_0d0c_0b0a_0908 };

fn h(n: i64) -> u64 {
    hash(TEST_KEY, |e| e.int(n))
}

fn keys(t: &OrderedTable<i64, i64>) -> Vec<i64> {
    t.iter().map(|(k, _)| *k).collect()
}

#[test]
fn the_listing_is_the_order_keys_were_first_added() {
    let mut t = OrderedTable::new();
    for k in [5, 1, 9, 3] {
        assert_eq!(t.insert(h(k), k, k * 10, |x| *x == k), None);
    }
    // Setting a present key keeps its place.
    assert_eq!(t.insert(h(1), 1, 11, |x| *x == 1), Some(10));
    assert_eq!(keys(&t), [5, 1, 9, 3]);
    assert_eq!(t.get(h(1), |x| *x == 1), Some(&11));
    // Removing and adding again puts it last.
    assert_eq!(t.remove(h(9), |x| *x == 9), Some((9, 90)));
    assert_eq!(t.get(h(9), |x| *x == 9), None);
    t.insert(h(9), 9, 99, |x| *x == 9);
    assert_eq!(keys(&t), [5, 1, 3, 9]);
    assert_eq!(t.len(), 4);
}

#[test]
fn the_capacity_is_a_function_of_the_entries_alone() {
    assert_eq!(index_capacity(0), 8);
    assert_eq!(index_capacity(4), 8);
    assert_eq!(index_capacity(5), 16);
    let mut t = OrderedTable::new();
    for k in 0..1000i64 {
        t.insert(h(k), k, k, |x| *x == k);
        assert_eq!(t.capacity(), index_capacity(t.len()), "after {k}");
        assert!(2 * t.len() <= t.capacity());
    }
    // Removing more than half compacts the entries and shrinks the index.
    for k in 0..600i64 {
        assert!(t.remove(h(k), |x| *x == k).is_some());
    }
    assert_eq!(t.len(), 400);
    assert!(t.capacity() <= index_capacity(t.len() * 2));
    assert_eq!(keys(&t), (600..1000).collect::<Vec<_>>());
    for k in 600..1000i64 {
        assert_eq!(t.get(h(k), |x| *x == k), Some(&k));
    }
}
