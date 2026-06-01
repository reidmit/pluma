import sys

sys.setrecursionlimit(1 << 20)


class Leaf:
    __slots__ = ("v",)

    def __init__(self, v):
        self.v = v


class Node:
    __slots__ = ("l", "r")

    def __init__(self, l, r):
        self.l = l
        self.r = r


def build(depth, start):
    if depth == 0:
        return Leaf(start)
    return Node(build(depth - 1, start), build(depth - 1, start + 1))


def tree_sum(t):
    if isinstance(t, Leaf):
        return t.v
    return tree_sum(t.l) + tree_sum(t.r)


print(tree_sum(build(21, 1)))
