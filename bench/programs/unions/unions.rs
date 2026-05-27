use std::hint::black_box;
use std::time::Instant;

enum Expr {
    Num(i64),
    Add(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
}

fn evaluate(e: &Expr) -> i64 {
    match e {
        Expr::Num(v) => *v,
        Expr::Add(l, r) => evaluate(l) + evaluate(r),
        Expr::Mul(l, r) => evaluate(l) * evaluate(r),
    }
}

fn run() -> i64 {
    let count: i64 = black_box(2_000_000);
    let p: i64 = 1_000_000_007;
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < count {
        let expr = Expr::Add(
            Box::new(Expr::Num(i % 1000)),
            Box::new(Expr::Mul(Box::new(Expr::Num(i % 7)), Box::new(Expr::Num(3)))),
        );
        acc = (acc + evaluate(&expr)) % p;
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
