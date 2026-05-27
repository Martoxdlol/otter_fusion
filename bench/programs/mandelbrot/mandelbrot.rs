use std::hint::black_box;
use std::time::Instant;

fn run() -> i64 {
    let w: i64 = black_box(800);
    let h: i64 = black_box(800);
    let maxiter: i64 = 255;
    let mut checksum: i64 = 0;

    let mut py = 0i64;
    while py < h {
        let mut px = 0i64;
        while px < w {
            let cre = -2.0 + (px as f64) * (2.5 / (w as f64));
            let cim = -1.25 + (py as f64) * (2.5 / (h as f64));
            let mut zr = 0.0f64;
            let mut zi = 0.0f64;
            let mut zr2 = 0.0f64;
            let mut zi2 = 0.0f64;
            let mut i = 0i64;
            while zr2 + zi2 <= 4.0 {
                if i >= maxiter {
                    break;
                }
                zi = 2.0 * zr * zi + cim;
                zr = zr2 - zi2 + cre;
                zr2 = zr * zr;
                zi2 = zi * zi;
                i += 1;
            }
            checksum += i;
            px += 1;
        }
        py += 1;
    }
    checksum
}

fn main() {
    black_box(run()); // warmup
    let t0 = Instant::now();
    let ans = black_box(run());
    let dt = t0.elapsed().as_nanos() as i64;
    println!("{}", ans);
    println!("{}", dt);
}
