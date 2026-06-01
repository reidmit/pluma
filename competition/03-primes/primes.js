function isPrime(n) {
  for (let d = 2; d * d <= n; d++) {
    if (n % d === 0) return false;
  }
  return true;
}

let count = 0;
for (let n = 2; n < 300000; n++) {
  if (isPrime(n)) count++;
}
console.log(count);
