# Sieve of Eratosthenes — a mutable array marked in place, then an order-
# sensitive checksum over the survivors. An explicit inner loop (not slice
# assignment) so every language runs the same per-element mark/scan work.
n = 10000000
sieve = bytearray(n + 1)  # 0 = prime candidate, 1 = composite
sieve[0] = 1
sieve[1] = 1

p = 2
while p * p <= n:
    if sieve[p] == 0:
        j = p * p
        while j <= n:
            sieve[j] = 1
            j += p
    p += 1

count = 0
checksum = 0
i = 2
while i <= n:
    if sieve[i] == 0:
        count += 1
        checksum = (checksum + i) % 1000000007
    i += 1

print(count)
print(checksum)
