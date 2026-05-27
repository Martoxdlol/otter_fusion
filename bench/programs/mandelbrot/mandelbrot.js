function run() {
  const w = 800, h = 800, maxiter = 255;
  let checksum = 0;
  for (let py = 0; py < h; py++) {
    for (let px = 0; px < w; px++) {
      const cre = -2.0 + px * (2.5 / w);
      const cim = -1.25 + py * (2.5 / h);
      let zr = 0.0, zi = 0.0, zr2 = 0.0, zi2 = 0.0, i = 0;
      while (zr2 + zi2 <= 4.0) {
        if (i >= maxiter) break;
        zi = 2.0 * zr * zi + cim;
        zr = zr2 - zi2 + cre;
        zr2 = zr * zr;
        zi2 = zi * zi;
        i++;
      }
      checksum += i;
    }
  }
  return checksum;
}

run(); // warmup the JIT
const t0 = process.hrtime.bigint();
const ans = run();
const dt = process.hrtime.bigint() - t0;
console.log(ans.toString());
console.log(dt.toString());
