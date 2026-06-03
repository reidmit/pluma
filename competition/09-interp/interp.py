# A tiny arithmetic-expression interpreter: build an AST, evaluate it many
# times. Arithmetic is mod 1000003 so every language agrees bit-for-bit.
import sys

sys.setrecursionlimit(1 << 20)

LIT, ADD, SUB, MUL, NEG = 0, 1, 2, 3, 4
M = 1000003


def build(depth, seed):
    if depth == 0:
        return (LIT, seed % 7)
    l = build(depth - 1, seed * 2 + 1)
    r = build(depth - 1, seed * 2 + 2)
    k = seed % 4
    if k == 0:
        return (ADD, l, r)
    if k == 1:
        return (SUB, l, r)
    if k == 2:
        return (MUL, l, r)
    return (NEG, l)


def ev(e):
    t = e[0]
    if t == LIT:
        return e[1]
    if t == ADD:
        return (ev(e[1]) + ev(e[2])) % M
    if t == SUB:
        return ((ev(e[1]) - ev(e[2])) + M) % M
    if t == MUL:
        return (ev(e[1]) * ev(e[2])) % M
    return (M - ev(e[1])) % M


def main():
    tree = build(18, 1)
    acc = 0
    for _ in range(2000):
        acc = (acc + ev(tree)) % M
    print(acc)


main()
