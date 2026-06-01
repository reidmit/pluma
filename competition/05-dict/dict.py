counts = {}
buckets = 20000
for i in range(200000):
    key = str(i % buckets)
    counts[key] = counts.get(key, 0) + 1
print(len(counts))
print(counts.get("0", 0))
