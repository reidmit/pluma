def in_set(cr, ci)
  zr = 0.0
  zi = 0.0
  1000.times do
    zr2 = zr * zr
    zi2 = zi * zi
    return 0 if zr2 + zi2 > 4.0
    zi = 2.0 * zr * zi + ci
    zr = zr2 - zi2 + cr
  end
  1
end

width = 150
dx = 2.5 / width
dy = 2.5 / width
count = 0
(0...width).each do |py|
  ci = -1.25 + py * dy
  (0...width).each do |px|
    cr = -2.0 + px * dx
    count += in_set(cr, ci)
  end
end
puts count
