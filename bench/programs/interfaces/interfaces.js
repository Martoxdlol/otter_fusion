class Circle {
  constructor(r) { this.r = r; }
  area() { return Math.trunc((this.r * this.r * 355) / 113); }
}
class Square {
  constructor(s) { this.s = s; }
  area() { return this.s * this.s; }
}

function run() {
  const k = 2000, rounds = 5000, p = 1000000007;
  const shapes = [];
  for (let i = 0; i < k; i++) {
    const v = (i % 100) + 1;
    shapes.push(i % 2 === 0 ? new Circle(v) : new Square(v));
  }
  let acc = 0;
  for (let r = 0; r < rounds; r++) {
    for (const sh of shapes) {
      acc = (acc + sh.area()) % p;
    }
  }
  return acc;
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
