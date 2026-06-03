# JSON round-trip — build a big document as text (byte-identical across all
# languages), parse it, aggregate integer fields, then re-serialize and re-parse.
# Output is integer aggregates, independent of key ordering.
import json


def build_input(n):
    objs = []
    for i in range(n):
        v = (i * 2654435761) % 100003
        flag = "true" if i % 2 == 0 else "false"
        objs.append('{"id":%d,"name":"item-%d","value":%d,"flag":%s}' % (i, i, v, flag))
    return "[" + ",".join(objs) + "]"


def aggregate(arr):
    s = 0
    trues = 0
    for o in arr:
        s = (s + o["value"]) % 1000000007
        if o["flag"] is True:
            trues += 1
    return s, trues


data = build_input(20000)
v = json.loads(data)
s, trues = aggregate(v)
roundtrip = json.dumps(v)
v2 = json.loads(roundtrip)
s2, _ = aggregate(v2)
print(len(v))
print(s)
print(trues)
print(s2)
