function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

function run() {
  return fib(35);
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
