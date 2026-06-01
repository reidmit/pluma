Leaf = Struct.new(:v)
Node = Struct.new(:l, :r)

def build(depth, start)
  return Leaf.new(start) if depth == 0
  Node.new(build(depth - 1, start), build(depth - 1, start + 1))
end

def tree_sum(t)
  return t.v if t.is_a?(Leaf)
  tree_sum(t.l) + tree_sum(t.r)
end

puts tree_sum(build(21, 1))
