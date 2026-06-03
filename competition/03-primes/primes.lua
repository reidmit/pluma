-- Count the primes below a limit by trial division — integer arithmetic, modulo.
local function is_prime(n)
  local d = 2
  while d * d <= n do
    if n % d == 0 then return false end
    d = d + 1
  end
  return true
end

local count = 0
for n = 2, 299999 do
  if is_prime(n) then count = count + 1 end
end
print(count)
