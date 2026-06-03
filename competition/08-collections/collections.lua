-- Functional collection pipeline: range -> map -> filter -> fold, materializing
-- each intermediate table.
local nums = {}
for i = 1, 1000000 do nums[i] = i end
local squared = {}
for i = 1, 1000000 do squared[i] = nums[i] * nums[i] end
local evens = {}
local k = 0
for i = 1, 1000000 do
  if squared[i] % 2 == 0 then
    k = k + 1
    evens[k] = squared[i]
  end
end
local total = 0
for i = 1, k do total = (total + evens[i]) % 1000000007 end
print(total)
