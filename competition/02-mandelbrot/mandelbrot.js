function inSet(cr, ci) {
  let zr = 0.0;
  let zi = 0.0;
  for (let i = 0; i < 1000; i++) {
    const zr2 = zr * zr;
    const zi2 = zi * zi;
    if (zr2 + zi2 > 4.0) return 0;
    zi = 2.0 * zr * zi + ci;
    zr = zr2 - zi2 + cr;
  }
  return 1;
}

const width = 150;
const dx = 2.5 / width;
const dy = 2.5 / width;
let count = 0;
for (let py = 0; py < width; py++) {
  const ci = -1.25 + py * dy;
  for (let px = 0; px < width; px++) {
    const cr = -2.0 + px * dx;
    count += inSet(cr, ci);
  }
}
console.log(count);
