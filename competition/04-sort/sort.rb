n = 100000
xs = Array.new(n) { |i| (i * 2654435761) % 100003 }
xs.sort!
checksum = 0
xs.each { |v| checksum = (checksum * 31 + v) % 1000000007 }
puts xs.length
puts checksum
