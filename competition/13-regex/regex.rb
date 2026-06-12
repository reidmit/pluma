def line(i)
  a = (i * 7) % 100000
  b = (i * 13) % 100000
  c = (i * 31) % 100000
  "user=#{a} noise word here id=#{b} and ok=#{c}"
end

text = (0...12000).map { |i| line(i) }.join("\n")
re = /([A-Za-z]+)=([0-9]+)/
count = 0
total = 0
text.scan(re) do
  m = $~
  count += 1
  total = (total + m[0].length) % 1000000007
end
puts count
puts total
