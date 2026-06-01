nums = list(range(1, 1000001))
squared = [x * x for x in nums]
evens = [x for x in squared if x % 2 == 0]
total = 0
for x in evens:
    total = (total + x) % 1000000007
print(total)
