// The kernels as a Rust-emitting backend would write them: checked arithmetic, no unsafe.
fn slow() -> ! { eprintln!("overflow: promote"); std::process::exit(3) }
fn add(a: i64, b: i64) -> i64 { a.checked_add(b).unwrap_or_else(|| slow()) }
fn sub(a: i64, b: i64) -> i64 { a.checked_sub(b).unwrap_or_else(|| slow()) }
fn mul(a: i64, b: i64) -> i64 { a.checked_mul(b).unwrap_or_else(|| slow()) }
fn fib(n: i64) -> i64 { if n < 2 { n } else { add(fib(sub(n, 1)), fib(sub(n, 2))) } }
fn mix(n: i64) -> i64 { let mut h = 7i64; for i in 0..n { h = add(mul(h, 31), i).rem_euclid(1000000007); } h }
fn main() { if std::env::args().nth(1).as_deref() == Some("mix") { println!("{}", mix(50000000)); } else { println!("{}", fib(32)); } }
