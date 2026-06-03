-- Word-frequency tally over a stream of keys — hash table insert/lookup.
local counts = {}
local buckets = 20000
for i = 0, 199999 do
  local key = tostring(i % buckets)
  counts[key] = (counts[key] or 0) + 1
end
local size = 0
for _ in pairs(counts) do size = size + 1 end
print(size)
print(counts["0"] or 0)
