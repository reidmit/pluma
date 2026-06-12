function line(i) {
  const a = (i * 7) % 100000;
  const b = (i * 13) % 100000;
  const c = (i * 31) % 100000;
  return `user=${a} noise word here id=${b} and ok=${c}`;
}

const parts = [];
for (let i = 0; i < 12000; i++) parts.push(line(i));
const text = parts.join("\n");

const re = /([A-Za-z]+)=([0-9]+)/g;
let count = 0;
let total = 0;
let m;
while ((m = re.exec(text)) !== null) {
  count++;
  total = (total + m[0].length) % 1000000007;
}
console.log(count);
console.log(total);
