// Tagged-object discriminated union + switch.
const NUM = 0, ADD = 1, MUL = 2;

function evaluate(e) {
  switch (e.t) {
    case NUM: return e.v;
    case ADD: return evaluate(e.l) + evaluate(e.r);
    case MUL: return evaluate(e.l) * evaluate(e.r);
  }
}

function run() {
  const count = 2000000, p = 1000000007;
  let acc = 0;
  for (let i = 0; i < count; i++) {
    const expr = {
      t: ADD,
      l: { t: NUM, v: i % 1000 },
      r: { t: MUL, l: { t: NUM, v: i % 7 }, r: { t: NUM, v: 3 } },
    };
    acc = (acc + evaluate(expr)) % p;
  }
  return acc;
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
