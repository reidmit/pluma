-- A tiny arithmetic-expression interpreter: build an AST, evaluate it many
-- times. Arithmetic is mod 1000003 so every language agrees bit-for-bit.
local LIT, ADD, SUB, MUL, NEG = 0, 1, 2, 3, 4
local M = 1000003

local function build(depth, seed)
  if depth == 0 then return { t = LIT, v = seed % 7 } end
  local l = build(depth - 1, seed * 2 + 1)
  local r = build(depth - 1, seed * 2 + 2)
  local k = seed % 4
  if k == 0 then return { t = ADD, l = l, r = r } end
  if k == 1 then return { t = SUB, l = l, r = r } end
  if k == 2 then return { t = MUL, l = l, r = r } end
  return { t = NEG, l = l }
end

local function ev(e)
  local t = e.t
  if t == LIT then return e.v end
  if t == ADD then return (ev(e.l) + ev(e.r)) % M end
  if t == SUB then return (ev(e.l) - ev(e.r) + M) % M end
  if t == MUL then return (ev(e.l) * ev(e.r)) % M end
  return (M - ev(e.l)) % M
end

local tree = build(18, 1)
local acc = 0
for _ = 1, 2000 do acc = (acc + ev(tree)) % M end
print(acc)
