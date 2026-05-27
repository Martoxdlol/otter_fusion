use std::hint::black_box;
use std::time::Instant;

fn run() -> i64 {
    let n = black_box(2_000_000usize);
    let mut sieve = vec![true; n];
    let mut count = 0i64;
    let mut p = 2usize;
    while p < n {
        if sieve[p] {
            count += 1;
            let mut m = p * 2;
            while m < n {
                sieve[m] = false;
                m += p;
            }
        }
        p += 1;
    }
    count
}

fn main() {
    black_box(run()); // warmup
    let t0 = Instant::now();
    let ans = black_box(run());
    let dt = t0.elapsed().as_nanos() as i64;
    println!("{}", ans);
    println!("{}", dt);
}
