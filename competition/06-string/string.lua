-- String round-trip: build many parts, join, split back, uppercase.
local parts = {}
for i = 0, 49999 do parts[i + 1] = "item-" .. i end
local joined = table.concat(parts, ",")
local back = {}
local k = 0
for tok in string.gmatch(joined, "[^,]+") do
  k = k + 1
  back[k] = tok
end
local upper = string.upper(joined)
print(#back)
print(#joined)
print(#upper)
