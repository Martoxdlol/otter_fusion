use std::hint::black_box;
use std::time::Instant;

trait Shape {
    fn area(&self) -> i64;
}

struct Circle {
    r: i64,
}
struct Square {
    s: i64,
}

impl Shape for Circle {
    fn area(&self) -> i64 {
        self.r * self.r * 355 / 113
    }
}
impl Shape for Square {
    fn area(&self) -> i64 {
        self.s * self.s
    }
}

fn run() -> i64 {
    let k: i64 = black_box(2000);
    let rounds: i64 = black_box(5000);
    let p: i64 = 1_000_000_007;

    let mut shapes: Vec<Box<dyn Shape>> = Vec::new();
    for i in 0..k {
        let v = i % 100 + 1;
        if i % 2 == 0 {
            shapes.push(Box::new(Circle { r: v }));
        } else {
            shapes.push(Box::new(Square { s: v }));
        }
    }

    let mut acc: i64 = 0;
    for _ in 0..rounds {
        for sh in &shapes {
            acc = (acc + sh.area()) % p;
        }
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
