// Idiomatic lazy iteration via a generator + for-of.
function* range(n) {
  for (let i = 0; i < n; i++) yield i;
}

function run() {
  const n = 10000000, p = 1000000007;
  let acc = 0;
  for (const i of range(n)) {
    if (i % 3 === 0) acc = (acc + (i * i) % p) % p;
  }
  return acc;
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
