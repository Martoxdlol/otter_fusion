function run() {
  const n = 2000000;
  const sieve = new Uint8Array(n).fill(1);
  let count = 0;
  for (let p = 2; p < n; p++) {
    if (sieve[p]) {
      count++;
      for (let m = p * 2; m < n; m += p) sieve[m] = 0;
    }
  }
  return count;
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
