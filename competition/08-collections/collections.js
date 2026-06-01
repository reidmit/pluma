const nums = [];
for (let i = 1; i <= 1000000; i++) nums.push(i);
const squared = nums.map((x) => x * x);
const evens = squared.filter((x) => x % 2 === 0);
const total = evens.reduce((acc, x) => (acc + x) % 1000000007, 0);
console.log(total);
