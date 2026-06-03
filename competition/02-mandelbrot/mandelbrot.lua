-- Mandelbrot set — pure float64 arithmetic in a tight escape loop over a grid.
local function in_set(cr, ci)
  local zr, zi = 0.0, 0.0
  for _ = 1, 1000 do
    local zr2 = zr * zr
    local zi2 = zi * zi
    if zr2 + zi2 > 4.0 then return 0 end
    zi = 2.0 * zr * zi + ci
    zr = zr2 - zi2 + cr
  end
  return 1
end

local width = 150
local dx = 2.5 / width
local dy = 2.5 / width
local count = 0
for py = 0, width - 1 do
  local ci = -1.25 + py * dy
  for px = 0, width - 1 do
    local cr = -2.0 + px * dx
    count = count + in_set(cr, ci)
  end
end
print(count)
