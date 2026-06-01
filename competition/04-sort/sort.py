n = 100000
xs = [(i * 2654435761) % 100003 for i in range(n)]
xs.sort()
checksum = 0
for v in xs:
    checksum = (checksum * 31 + v) % 1000000007
print(len(xs))
print(checksum)
