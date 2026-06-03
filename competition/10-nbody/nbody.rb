# N-body gravitation — float64 arithmetic over body structs. Same constants,
# same operation order, same step count as every other language, so the scaled
# integer energy agrees bit-for-bit. Each body is accelerated toward every
# other (self skipped where the squared distance is exactly 0).
DT = 0.01
PI = 3.141592653589793
SOLAR = 4.0 * PI * PI
DPY = 365.24

# [x, y, z, vx, vy, vz, m]
X, Y, Z, VX, VY, VZ, M = 0, 1, 2, 3, 4, 5, 6
bodies = [
  [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, SOLAR], # sun
  [
    4.8414314424647209,
    -1.16032004402742839,
    -0.103622044471123109,
    0.00166007664274403694 * DPY,
    0.00769901118419740425 * DPY,
    -0.0000690460016972063023 * DPY,
    0.000954791938424326609 * SOLAR,
  ],
  [
    8.34336671824457987,
    4.12479856412430479,
    -0.403523417114321381,
    -0.00276742510726862411 * DPY,
    0.00499852801234917238 * DPY,
    0.0000230417297573763929 * DPY,
    0.000285885980666130812 * SOLAR,
  ],
  [
    12.894369562139131,
    -15.1111514016986312,
    -0.223307578892655734,
    0.00296460137564761618 * DPY,
    0.0023784717395948095 * DPY,
    -0.0000296589568540237556 * DPY,
    0.0000436624404335156298 * SOLAR,
  ],
  [
    15.3796971148509165,
    -25.9193146099879641,
    0.179258772950371181,
    0.00268067772490389322 * DPY,
    0.00162824170038242295 * DPY,
    -0.000095159225451971587 * DPY,
    0.000230417297573763929 * SOLAR,
  ],
]

N = bodies.length

def offset_momentum(b)
  px = py = pz = 0.0
  N.times do |i|
    px += b[i][VX] * b[i][M]
    py += b[i][VY] * b[i][M]
    pz += b[i][VZ] * b[i][M]
  end
  b[0][VX] = 0.0 - px / SOLAR
  b[0][VY] = 0.0 - py / SOLAR
  b[0][VZ] = 0.0 - pz / SOLAR
end

def step(b)
  nvx = Array.new(N, 0.0)
  nvy = Array.new(N, 0.0)
  nvz = Array.new(N, 0.0)
  N.times do |i|
    ax = ay = az = 0.0
    N.times do |j|
      dx = b[j][X] - b[i][X]
      dy = b[j][Y] - b[i][Y]
      dz = b[j][Z] - b[i][Z]
      d2 = dx * dx + dy * dy + dz * dz
      if d2 != 0.0
        dist = Math.sqrt(d2)
        mag = b[j][M] / (d2 * dist)
        ax += dx * mag
        ay += dy * mag
        az += dz * mag
      end
    end
    nvx[i] = b[i][VX] + DT * ax
    nvy[i] = b[i][VY] + DT * ay
    nvz[i] = b[i][VZ] + DT * az
  end
  N.times do |i|
    b[i][VX] = nvx[i]
    b[i][VY] = nvy[i]
    b[i][VZ] = nvz[i]
  end
  N.times do |i|
    b[i][X] += DT * b[i][VX]
    b[i][Y] += DT * b[i][VY]
    b[i][Z] += DT * b[i][VZ]
  end
end

def energy(b)
  e = 0.0
  N.times do |i|
    e += 0.5 * b[i][M] * (b[i][VX] * b[i][VX] + b[i][VY] * b[i][VY] + b[i][VZ] * b[i][VZ])
  end
  N.times do |i|
    (i + 1).upto(N - 1) do |j|
      dx = b[i][X] - b[j][X]
      dy = b[i][Y] - b[j][Y]
      dz = b[i][Z] - b[j][Z]
      dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
      e -= (b[i][M] * b[j][M]) / dist
    end
  end
  e
end

def report(e)
  (e * 1000000000.0).floor
end

offset_momentum(bodies)
puts report(energy(bodies))
100000.times { step(bodies) }
puts report(energy(bodies))
