//! The test-only hooks (ADR 0119 rule 9), alone in this test binary: the hooks are global to the
//! process, so no other test may run beside them.
use una_hash::{KeyError, OrderedTable, ProcessKey, SipKey, hash, hooks, table_key};

const TEST_KEY: SipKey = SipKey { k0: 0x0706_0504_0302_0100, k1: 0x0f0e_0d0c_0b0a_0908 };

fn h(n: i64) -> u64 {
    hash(TEST_KEY, |e| e.int(n))
}

#[test]
fn hooks_fixed_key_constant_hash_probes_counter_and_missing_source() {
    // One test, since the hooks are global to the process.
    hooks::reset();

    // Every key collides in the constant-hash mode, and the table stays correct.
    hooks::set_constant_hash(true);
    assert_eq!(hash(TEST_KEY, |e| e.int(1)), 0);
    let mut t = OrderedTable::new();
    for k in 0..64i64 {
        t.insert(hash(TEST_KEY, |e| e.int(k)), k, k, |x| *x == k);
    }
    for k in 0..64i64 {
        assert_eq!(t.get(0, |x| *x == k), Some(&k));
    }
    hooks::set_constant_hash(false);

    // Under a random-looking key, a lookup of a present key reads few slots.
    hooks::reset();
    let mut t = OrderedTable::new();
    for k in 0..10_000i64 {
        t.insert(h(k), k, k, |x| *x == k);
    }
    let before = hooks::probes();
    for k in 0..10_000i64 {
        assert!(t.contains(h(k), |x| *x == k));
    }
    let probes = hooks::probes() - before;
    assert!(probes < 10_000 * 3, "{probes} probes for 10,000 present keys");

    // A fixed process key gives tables 0, 1 and 2 six distinct key halves.
    hooks::fix_key(TEST_KEY);
    let process = match ProcessKey::from_host() {
        Ok(p) => p,
        Err(e) => {
            assert_eq!(Some(e), None, "the fixed key is given");
            return;
        }
    };
    let mut halves = std::collections::BTreeSet::new();
    for n in 0..3u64 {
        let key = process.next_table_key().unwrap_or(SipKey { k0: 0, k1: 0 });
        assert_eq!(key, table_key(TEST_KEY, n));
        assert!(halves.insert(key.k0));
        assert!(halves.insert(key.k1));
    }
    assert_eq!(halves.len(), 6);

    // A counter at 2^64 - 1 refuses the next table (U6003).
    hooks::start_tables_at(u64::MAX);
    let process = match ProcessKey::from_host() {
        Ok(p) => p,
        Err(e) => {
            assert_eq!(Some(e), None, "the fixed key is given");
            return;
        }
    };
    assert_eq!(process.next_table_key().err(), Some(KeyError::CounterExhausted));

    // A missing random source gives no process key (U6009), and no fallback key.
    hooks::reset();
    hooks::simulate_missing_source();
    assert_eq!(ProcessKey::from_host().err(), Some(KeyError::NoRandomBytes));

    // The host's own source, on the machine that runs the tests, gives one.
    hooks::reset();
    assert!(ProcessKey::from_host().is_ok());
}
