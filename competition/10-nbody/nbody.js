// N-body gravitation — float64 arithmetic over body structs. Same constants,
// same operation order, same step count as every other language, so the scaled
// integer energy agrees bit-for-bit. Each body is accelerated toward every
// other (self skipped where the squared distance is exactly 0).
const dt = 0.01;
const pi = 3.141592653589793;
const solar = 4.0 * pi * pi;
const dpy = 365.24;

const bodies = [
  { x: 0.0, y: 0.0, z: 0.0, vx: 0.0, vy: 0.0, vz: 0.0, m: solar }, // sun
  {
    x: 4.8414314424647209,
    y: -1.16032004402742839,
    z: -0.103622044471123109,
    vx: 0.00166007664274403694 * dpy,
    vy: 0.00769901118419740425 * dpy,
    vz: -0.0000690460016972063023 * dpy,
    m: 0.000954791938424326609 * solar,
  },
  {
    x: 8.34336671824457987,
    y: 4.12479856412430479,
    z: -0.403523417114321381,
    vx: -0.00276742510726862411 * dpy,
    vy: 0.00499852801234917238 * dpy,
    vz: 0.0000230417297573763929 * dpy,
    m: 0.000285885980666130812 * solar,
  },
  {
    x: 12.894369562139131,
    y: -15.1111514016986312,
    z: -0.223307578892655734,
    vx: 0.00296460137564761618 * dpy,
    vy: 0.0023784717395948095 * dpy,
    vz: -0.0000296589568540237556 * dpy,
    m: 0.0000436624404335156298 * solar,
  },
  {
    x: 15.3796971148509165,
    y: -25.9193146099879641,
    z: 0.179258772950371181,
    vx: 0.00268067772490389322 * dpy,
    vy: 0.00162824170038242295 * dpy,
    vz: -0.000095159225451971587 * dpy,
    m: 0.000230417297573763929 * solar,
  },
];

const n = bodies.length;

function offsetMomentum(b) {
  let px = 0.0,
    py = 0.0,
    pz = 0.0;
  for (let i = 0; i < n; i++) {
    px += b[i].vx * b[i].m;
    py += b[i].vy * b[i].m;
    pz += b[i].vz * b[i].m;
  }
  b[0].vx = 0.0 - px / solar;
  b[0].vy = 0.0 - py / solar;
  b[0].vz = 0.0 - pz / solar;
}

function step(b) {
  const nvx = new Array(n),
    nvy = new Array(n),
    nvz = new Array(n);
  for (let i = 0; i < n; i++) {
    let ax = 0.0,
      ay = 0.0,
      az = 0.0;
    for (let j = 0; j < n; j++) {
      const dx = b[j].x - b[i].x;
      const dy = b[j].y - b[i].y;
      const dz = b[j].z - b[i].z;
      const d2 = dx * dx + dy * dy + dz * dz;
      if (d2 !== 0.0) {
        const dist = Math.sqrt(d2);
        const mag = b[j].m / (d2 * dist);
        ax += dx * mag;
        ay += dy * mag;
        az += dz * mag;
      }
    }
    nvx[i] = b[i].vx + dt * ax;
    nvy[i] = b[i].vy + dt * ay;
    nvz[i] = b[i].vz + dt * az;
  }
  for (let i = 0; i < n; i++) {
    b[i].vx = nvx[i];
    b[i].vy = nvy[i];
    b[i].vz = nvz[i];
  }
  for (let i = 0; i < n; i++) {
    b[i].x += dt * b[i].vx;
    b[i].y += dt * b[i].vy;
    b[i].z += dt * b[i].vz;
  }
}

function energy(b) {
  let e = 0.0;
  for (let i = 0; i < n; i++) {
    e += 0.5 * b[i].m * (b[i].vx * b[i].vx + b[i].vy * b[i].vy + b[i].vz * b[i].vz);
  }
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      const dx = b[i].x - b[j].x;
      const dy = b[i].y - b[j].y;
      const dz = b[i].z - b[j].z;
      const dist = Math.sqrt(dx * dx + dy * dy + dz * dz);
      e -= (b[i].m * b[j].m) / dist;
    }
  }
  return e;
}

const report = (e) => Math.floor(e * 1000000000.0);

offsetMomentum(bodies);
console.log(report(energy(bodies)));
for (let s = 0; s < 100000; s++) step(bodies);
console.log(report(energy(bodies)));
