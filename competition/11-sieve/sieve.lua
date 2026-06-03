-- Sieve of Eratosthenes — a mutable array marked in place, then an order-
-- sensitive checksum over the survivors. An explicit inner loop so every
-- language runs the same per-element mark/scan work.
local n = 10000000
local sieve = {}
for i = 1, n do sieve[i] = 0 end -- sieve[k] is for number k (1..n); 0 = candidate
sieve[1] = 1

for p = 2, n do
  if p * p > n then break end
  if sieve[p] == 0 then
    local j = p * p
    while j <= n do
      sieve[j] = 1
      j = j + p
    end
  end
end

local count = 0
local checksum = 0
for i = 2, n do
  if sieve[i] == 0 then
    count = count + 1
    checksum = (checksum + i) % 1000000007
  end
end
print(count)
print(checksum)
