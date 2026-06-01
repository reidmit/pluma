const counts = new Map();
const buckets = 20000;
for (let i = 0; i < 200000; i++) {
  const key = String(i % buckets);
  counts.set(key, (counts.get(key) || 0) + 1);
}
console.log(counts.size);
console.log(counts.get("0") || 0);
