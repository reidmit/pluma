-- N-body gravitation — float64 arithmetic over body structs. Same constants,
-- same operation order, same step count as every other language, so the scaled
-- integer energy agrees bit-for-bit.
local DT = 0.01
local PI = 3.141592653589793
local SOLAR = 4.0 * PI * PI
local DPY = 365.24
local X, Y, Z, VX, VY, VZ, M = 1, 2, 3, 4, 5, 6, 7
local sqrt = math.sqrt
local floor = math.floor

local bodies = {
  { 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, SOLAR }, -- sun
  {
    4.8414314424647209,
    -1.16032004402742839,
    -0.103622044471123109,
    0.00166007664274403694 * DPY,
    0.00769901118419740425 * DPY,
    -0.0000690460016972063023 * DPY,
    0.000954791938424326609 * SOLAR,
  },
  {
    8.34336671824457987,
    4.12479856412430479,
    -0.403523417114321381,
    -0.00276742510726862411 * DPY,
    0.00499852801234917238 * DPY,
    0.0000230417297573763929 * DPY,
    0.000285885980666130812 * SOLAR,
  },
  {
    12.894369562139131,
    -15.1111514016986312,
    -0.223307578892655734,
    0.00296460137564761618 * DPY,
    0.0023784717395948095 * DPY,
    -0.0000296589568540237556 * DPY,
    0.0000436624404335156298 * SOLAR,
  },
  {
    15.3796971148509165,
    -25.9193146099879641,
    0.179258772950371181,
    0.00268067772490389322 * DPY,
    0.00162824170038242295 * DPY,
    -0.000095159225451971587 * DPY,
    0.000230417297573763929 * SOLAR,
  },
}

local n = #bodies

local function offset_momentum()
  local px, py, pz = 0.0, 0.0, 0.0
  for i = 1, n do
    px = px + bodies[i][VX] * bodies[i][M]
    py = py + bodies[i][VY] * bodies[i][M]
    pz = pz + bodies[i][VZ] * bodies[i][M]
  end
  bodies[1][VX] = 0.0 - px / SOLAR
  bodies[1][VY] = 0.0 - py / SOLAR
  bodies[1][VZ] = 0.0 - pz / SOLAR
end

local function step()
  local nvx, nvy, nvz = {}, {}, {}
  for i = 1, n do
    local ax, ay, az = 0.0, 0.0, 0.0
    for j = 1, n do
      local dx = bodies[j][X] - bodies[i][X]
      local dy = bodies[j][Y] - bodies[i][Y]
      local dz = bodies[j][Z] - bodies[i][Z]
      local d2 = dx * dx + dy * dy + dz * dz
      if d2 ~= 0.0 then
        local dist = sqrt(d2)
        local mag = bodies[j][M] / (d2 * dist)
        ax = ax + dx * mag
        ay = ay + dy * mag
        az = az + dz * mag
      end
    end
    nvx[i] = bodies[i][VX] + DT * ax
    nvy[i] = bodies[i][VY] + DT * ay
    nvz[i] = bodies[i][VZ] + DT * az
  end
  for i = 1, n do
    bodies[i][VX] = nvx[i]
    bodies[i][VY] = nvy[i]
    bodies[i][VZ] = nvz[i]
  end
  for i = 1, n do
    bodies[i][X] = bodies[i][X] + DT * bodies[i][VX]
    bodies[i][Y] = bodies[i][Y] + DT * bodies[i][VY]
    bodies[i][Z] = bodies[i][Z] + DT * bodies[i][VZ]
  end
end

local function energy()
  local e = 0.0
  for i = 1, n do
    e = e + 0.5 * bodies[i][M] * (bodies[i][VX] * bodies[i][VX] + bodies[i][VY] * bodies[i][VY] + bodies[i][VZ] * bodies[i][VZ])
  end
  for i = 1, n do
    for j = i + 1, n do
      local dx = bodies[i][X] - bodies[j][X]
      local dy = bodies[i][Y] - bodies[j][Y]
      local dz = bodies[i][Z] - bodies[j][Z]
      local dist = sqrt(dx * dx + dy * dy + dz * dz)
      e = e - (bodies[i][M] * bodies[j][M]) / dist
    end
  end
  return e
end

local function report(e)
  return floor(e * 1000000000.0)
end

offset_momentum()
print(report(energy()))
for _ = 1, 100000 do step() end
print(report(energy()))
