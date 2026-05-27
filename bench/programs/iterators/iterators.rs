use std::hint::black_box;
use std::time::Instant;

fn run() -> i64 {
    let n: i64 = black_box(10_000_000);
    let p: i64 = 1_000_000_007;
    // Idiomatic lazy iterator chain: range -> filter -> map -> fold.
    (0..n)
        .filter(|i| i % 3 == 0)
        .map(|i| (i * i) % p)
        .fold(0i64, |a, b| (a + b) % p)
}

fn main() {
    black_box(run()); // warmup
    let t0 = Instant::now();
    let ans = black_box(run());
    let dt = t0.elapsed().as_nanos() as i64;
    println!("{}", ans);
    println!("{}", dt);
}
