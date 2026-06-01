const n = 100000;
const xs = new Array(n);
for (let i = 0; i < n; i++) xs[i] = (i * 2654435761) % 100003;
xs.sort((a, b) => a - b);
let checksum = 0;
for (const v of xs) checksum = (checksum * 31 + v) % 1000000007;
console.log(xs.length);
console.log(checksum);
