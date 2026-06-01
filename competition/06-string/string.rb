parts = Array.new(50000) { |i| "item-#{i}" }
joined = parts.join(",")
back = joined.split(",")
upper = joined.upcase
puts back.length
puts joined.length
puts upper.length
