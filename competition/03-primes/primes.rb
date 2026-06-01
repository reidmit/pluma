def is_prime(n)
  d = 2
  while d * d <= n
    return false if n % d == 0
    d += 1
  end
  true
end

count = 0
(2...300000).each do |n|
  count += 1 if is_prime(n)
end
puts count
