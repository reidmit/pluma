# N-body gravitation — float64 arithmetic over body structs. Same constants,
# same operation order, same step count as every other language, so the scaled
# integer energy agrees bit-for-bit. Each body is accelerated toward every
# other (self skipped where the squared distance is exactly 0).
import math

dt = 0.01
pi = 3.141592653589793
solar = 4.0 * pi * pi
dpy = 365.24

# [x, y, z, vx, vy, vz, m]
bodies = [
    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, solar],  # sun
    [
        4.8414314424647209,
        -1.16032004402742839,
        -0.103622044471123109,
        0.00166007664274403694 * dpy,
        0.00769901118419740425 * dpy,
        -0.0000690460016972063023 * dpy,
        0.000954791938424326609 * solar,
    ],
    [
        8.34336671824457987,
        4.12479856412430479,
        -0.403523417114321381,
        -0.00276742510726862411 * dpy,
        0.00499852801234917238 * dpy,
        0.0000230417297573763929 * dpy,
        0.000285885980666130812 * solar,
    ],
    [
        12.894369562139131,
        -15.1111514016986312,
        -0.223307578892655734,
        0.00296460137564761618 * dpy,
        0.0023784717395948095 * dpy,
        -0.0000296589568540237556 * dpy,
        0.0000436624404335156298 * solar,
    ],
    [
        15.3796971148509165,
        -25.9193146099879641,
        0.179258772950371181,
        0.00268067772490389322 * dpy,
        0.00162824170038242295 * dpy,
        -0.000095159225451971587 * dpy,
        0.000230417297573763929 * solar,
    ],
]

n = len(bodies)
X, Y, Z, VX, VY, VZ, M = 0, 1, 2, 3, 4, 5, 6


def offset_momentum(b):
    px = py = pz = 0.0
    for i in range(n):
        px += b[i][VX] * b[i][M]
        py += b[i][VY] * b[i][M]
        pz += b[i][VZ] * b[i][M]
    b[0][VX] = 0.0 - px / solar
    b[0][VY] = 0.0 - py / solar
    b[0][VZ] = 0.0 - pz / solar


def step(b):
    nvx = [0.0] * n
    nvy = [0.0] * n
    nvz = [0.0] * n
    for i in range(n):
        ax = ay = az = 0.0
        for j in range(n):
            dx = b[j][X] - b[i][X]
            dy = b[j][Y] - b[i][Y]
            dz = b[j][Z] - b[i][Z]
            d2 = dx * dx + dy * dy + dz * dz
            if d2 != 0.0:
                dist = math.sqrt(d2)
                mag = b[j][M] / (d2 * dist)
                ax += dx * mag
                ay += dy * mag
                az += dz * mag
        nvx[i] = b[i][VX] + dt * ax
        nvy[i] = b[i][VY] + dt * ay
        nvz[i] = b[i][VZ] + dt * az
    for i in range(n):
        b[i][VX] = nvx[i]
        b[i][VY] = nvy[i]
        b[i][VZ] = nvz[i]
    for i in range(n):
        b[i][X] += dt * b[i][VX]
        b[i][Y] += dt * b[i][VY]
        b[i][Z] += dt * b[i][VZ]


def energy(b):
    e = 0.0
    for i in range(n):
        e += 0.5 * b[i][M] * (b[i][VX] * b[i][VX] + b[i][VY] * b[i][VY] + b[i][VZ] * b[i][VZ])
    for i in range(n):
        for j in range(i + 1, n):
            dx = b[i][X] - b[j][X]
            dy = b[i][Y] - b[j][Y]
            dz = b[i][Z] - b[j][Z]
            dist = math.sqrt(dx * dx + dy * dy + dz * dz)
            e -= (b[i][M] * b[j][M]) / dist
    return e


def report(e):
    return math.floor(e * 1000000000.0)


offset_momentum(bodies)
print(report(energy(bodies)))
for _ in range(100000):
    step(bodies)
print(report(energy(bodies)))
