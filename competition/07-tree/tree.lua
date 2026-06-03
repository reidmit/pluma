-- Build a perfect binary tree, then fold it — recursive data + dispatch.
local function build(depth, start)
  if depth == 0 then return { v = start } end
  return { l = build(depth - 1, start), r = build(depth - 1, start + 1) }
end

local function tree_sum(t)
  if t.l == nil then return t.v end
  return tree_sum(t.l) + tree_sum(t.r)
end

print(tree_sum(build(21, 1)))
