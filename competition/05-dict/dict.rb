counts = Hash.new(0)
buckets = 20000
200000.times do |i|
  key = (i % buckets).to_s
  counts[key] += 1
end
puts counts.size
puts counts["0"]
