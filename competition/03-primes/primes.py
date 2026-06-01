def is_prime(n):
    d = 2
    while d * d <= n:
        if n % d == 0:
            return False
        d += 1
    return True


count = 0
for n in range(2, 300000):
    if is_prime(n):
        count += 1
print(count)
