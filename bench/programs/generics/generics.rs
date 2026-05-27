use std::hint::black_box;
use std::time::Instant;

struct Wrapper<T> {
    value: T,
}

impl<T: Copy> Wrapper<T> {
    fn get(&self) -> T {
        self.value
    }
}

fn run() -> i64 {
    let n: i64 = black_box(10_000_000);
    let p: i64 = 1_000_000_007;
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let w = Wrapper { value: i };
        acc = (acc + w.get() * 2) % p;
        i += 1;
    }
    acc
}

fn main() {
    black_box(run()); // warmup
    let t0 = Instant::now();
    let ans = black_box(run());
    let dt = t0.elapsed().as_nanos() as i64;
    println!("{}", ans);
    println!("{}", dt);
}
