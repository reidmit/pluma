# A tiny arithmetic-expression interpreter: build an AST, evaluate it many
# times. Arithmetic is mod 1000003 so every language agrees bit-for-bit.
LIT, ADD, SUB, MUL, NEG = 0, 1, 2, 3, 4
M = 1000003

def build(depth, seed)
  return [LIT, seed % 7] if depth == 0
  l = build(depth - 1, seed * 2 + 1)
  r = build(depth - 1, seed * 2 + 2)
  case seed % 4
  when 0 then [ADD, l, r]
  when 1 then [SUB, l, r]
  when 2 then [MUL, l, r]
  else [NEG, l]
  end
end

def ev(e)
  case e[0]
  when LIT then e[1]
  when ADD then (ev(e[1]) + ev(e[2])) % M
  when SUB then ((ev(e[1]) - ev(e[2])) + M) % M
  when MUL then (ev(e[1]) * ev(e[2])) % M
  else (M - ev(e[1])) % M
  end
end

tree = build(18, 1)
acc = 0
2000.times { acc = (acc + ev(tree)) % M }
puts acc
