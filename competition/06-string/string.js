const parts = new Array(50000);
for (let i = 0; i < 50000; i++) parts[i] = "item-" + i;
const joined = parts.join(",");
const back = joined.split(",");
const upper = joined.toUpperCase();
console.log(back.length);
console.log(joined.length);
console.log(upper.length);
