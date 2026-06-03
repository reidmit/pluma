-- Sort a pseudo-random integer list, then fold an order-sensitive checksum.
local n = 100000
local xs = {}
for i = 0, n - 1 do xs[i + 1] = (i * 2654435761) % 100003 end
table.sort(xs)
local checksum = 0
for i = 1, n do checksum = (checksum * 31 + xs[i]) % 1000000007 end
print(n)
print(checksum)
