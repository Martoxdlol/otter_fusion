// JS has no generics; a plain class is the equivalent abstraction.
class Wrapper {
  constructor(value) { this.value = value; }
  get() { return this.value; }
}

function run() {
  const n = 10000000, p = 1000000007;
  let acc = 0;
  for (let i = 0; i < n; i++) {
    const w = new Wrapper(i);
    acc = (acc + w.get() * 2) % p;
  }
  return acc;
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
