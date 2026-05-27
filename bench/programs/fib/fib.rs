use std::hint::black_box;
use std::time::Instant;

fn fib(n: i64) -> i64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn run() -> i64 {
    // black_box on the input stops the optimizer from constant-folding the
    // whole call (the input isn't known at compile time in a real program).
    fib(black_box(35))
}

fn main() {
    black_box(run()); // warmup
    let t0 = Instant::now();
    let ans = black_box(run());
    let dt = t0.elapsed().as_nanos() as i64;
    println!("{}", ans);
    println!("{}", dt);
}
