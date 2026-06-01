function leaf(v) {
  return { tag: "leaf", v };
}

function node(l, r) {
  return { tag: "node", l, r };
}

function build(depth, start) {
  if (depth === 0) return leaf(start);
  return node(build(depth - 1, start), build(depth - 1, start + 1));
}

function treeSum(t) {
  if (t.tag === "leaf") return t.v;
  return treeSum(t.l) + treeSum(t.r);
}

console.log(treeSum(build(21, 1)));
