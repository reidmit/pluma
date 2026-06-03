# JSON round-trip — build a big document as text (byte-identical across all
# languages), parse it, aggregate integer fields, then re-serialize and re-parse.
# Output is integer aggregates, independent of key ordering.
require 'json'

def build_input(n)
  objs = []
  n.times do |i|
    v = (i * 2654435761) % 100003
    flag = i % 2 == 0 ? 'true' : 'false'
    objs << %({"id":#{i},"name":"item-#{i}","value":#{v},"flag":#{flag}})
  end
  '[' + objs.join(',') + ']'
end

def aggregate(arr)
  sum = 0
  trues = 0
  arr.each do |o|
    sum = (sum + o['value']) % 1000000007
    trues += 1 if o['flag'] == true
  end
  [sum, trues]
end

data = build_input(20000)
v = JSON.parse(data)
sum, trues = aggregate(v)
roundtrip = JSON.generate(v)
v2 = JSON.parse(roundtrip)
sum2, = aggregate(v2)
puts v.length
puts sum
puts trues
puts sum2
