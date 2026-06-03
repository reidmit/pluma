// JSON round-trip — build a big document as text (byte-identical across all
// languages), parse it, aggregate integer fields, then re-serialize and re-parse.
// Output is integer aggregates, independent of key ordering.
function buildInput(n) {
  const objs = [];
  for (let i = 0; i < n; i++) {
    const v = (i * 2654435761) % 100003;
    const flag = i % 2 === 0 ? "true" : "false";
    objs.push(`{"id":${i},"name":"item-${i}","value":${v},"flag":${flag}}`);
  }
  return "[" + objs.join(",") + "]";
}

function aggregate(arr) {
  let sum = 0;
  let trues = 0;
  for (const o of arr) {
    sum = (sum + o.value) % 1000000007;
    if (o.flag === true) trues++;
  }
  return [sum, trues];
}

const input = buildInput(20000);
const v = JSON.parse(input);
const [sum, trues] = aggregate(v);
const round = JSON.stringify(v);
const v2 = JSON.parse(round);
const [sum2] = aggregate(v2);
console.log(v.length);
console.log(sum);
console.log(trues);
console.log(sum2);
