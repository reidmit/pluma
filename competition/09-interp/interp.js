// A tiny arithmetic-expression interpreter: build an AST, evaluate it many
// times. Arithmetic is mod 1000003 so every language agrees bit-for-bit.
const LIT = 0,
  ADD = 1,
  SUB = 2,
  MUL = 3,
  NEG = 4;
const M = 1000003;

function build(depth, seed) {
  if (depth === 0) return { t: LIT, v: seed % 7 };
  const l = build(depth - 1, seed * 2 + 1);
  const r = build(depth - 1, seed * 2 + 2);
  switch (seed % 4) {
    case 0:
      return { t: ADD, l, r };
    case 1:
      return { t: SUB, l, r };
    case 2:
      return { t: MUL, l, r };
    default:
      return { t: NEG, l };
  }
}

function ev(e) {
  switch (e.t) {
    case LIT:
      return e.v;
    case ADD:
      return (ev(e.l) + ev(e.r)) % M;
    case SUB:
      return (ev(e.l) - ev(e.r) + M) % M;
    case MUL:
      return (ev(e.l) * ev(e.r)) % M;
    default:
      return (M - ev(e.l)) % M;
  }
}

const tree = build(18, 1);
let acc = 0;
for (let i = 0; i < 2000; i++) acc = (acc + ev(tree)) % M;
console.log(acc);
