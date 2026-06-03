-- Build a perfect binary tree of nominal nodes, then fold it — recursive data
-- construction and pattern matching at depth.
data Tree = Leaf !Int | Node Tree Tree

build :: Int -> Int -> Tree
build 0 start = Leaf start
build depth start = Node (build (depth - 1) start) (build (depth - 1) (start + 1))

treeSum :: Tree -> Int
treeSum (Leaf v) = v
treeSum (Node l r) = treeSum l + treeSum r

main :: IO ()
main = print (treeSum (build 21 1))
