# Sieve of Eratosthenes — a mutable array marked in place, then an order-
# sensitive checksum over the survivors. An explicit inner loop (not the Prime
# library) so every language runs the same per-element mark/scan work.
n = 10000000
sieve = Array.new(n + 1, 0) # 0 = prime candidate, 1 = composite
sieve[0] = 1
sieve[1] = 1

p = 2
while p * p <= n
  if sieve[p] == 0
    j = p * p
    while j <= n
      sieve[j] = 1
      j += p
    end
  end
  p += 1
end

count = 0
checksum = 0
i = 2
while i <= n
  if sieve[i] == 0
    count += 1
    checksum = (checksum + i) % 1000000007
  end
  i += 1
end

puts count
puts checksum
