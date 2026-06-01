nums = (1..1000000).to_a
squared = nums.map { |x| x * x }
evens = squared.select { |x| x % 2 == 0 }
total = evens.reduce(0) { |acc, x| (acc + x) % 1000000007 }
puts total
