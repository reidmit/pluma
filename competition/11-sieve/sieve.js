// Sieve of Eratosthenes — a mutable array marked in place, then an order-
// sensitive checksum over the survivors. An explicit inner loop so every
// language runs the same per-element mark/scan work.
const n = 10000000;
const sieve = new Uint8Array(n + 1); // 0 = prime candidate, 1 = composite
sieve[0] = 1;
sieve[1] = 1;

for (let p = 2; p * p <= n; p++) {
  if (sieve[p] === 0) {
    for (let j = p * p; j <= n; j += p) sieve[j] = 1;
  }
}

let count = 0;
let checksum = 0;
for (let i = 2; i <= n; i++) {
  if (sieve[i] === 0) {
    count++;
    checksum = (checksum + i) % 1000000007;
  }
}

console.log(count);
console.log(checksum);
