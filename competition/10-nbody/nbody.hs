-- N-body gravitation — float64 arithmetic over body records. Same constants,
-- same operation order, same step count as every other language, so the scaled
-- integer energy agrees bit-for-bit. Bodies are rebuilt immutably each step.
{-# LANGUAGE BangPatterns #-}

import Data.List (foldl')

data Body = Body
  { x, y, z, vx, vy, vz, m :: !Double
  }

dt, pii, solar, dpy :: Double
dt = 0.01
pii = 3.141592653589793
solar = 4.0 * pii * pii
dpy = 365.24

bodies0 :: [Body]
bodies0 =
  [ Body 0.0 0.0 0.0 0.0 0.0 0.0 solar -- sun
  , Body
      4.8414314424647209
      (-1.16032004402742839)
      (-0.103622044471123109)
      (0.00166007664274403694 * dpy)
      (0.00769901118419740425 * dpy)
      (-0.0000690460016972063023 * dpy)
      (0.000954791938424326609 * solar)
  , Body
      8.34336671824457987
      4.12479856412430479
      (-0.403523417114321381)
      (-0.00276742510726862411 * dpy)
      (0.00499852801234917238 * dpy)
      (0.0000230417297573763929 * dpy)
      (0.000285885980666130812 * solar)
  , Body
      12.894369562139131
      (-15.1111514016986312)
      (-0.223307578892655734)
      (0.00296460137564761618 * dpy)
      (0.0023784717395948095 * dpy)
      (-0.0000296589568540237556 * dpy)
      (0.0000436624404335156298 * solar)
  , Body
      15.3796971148509165
      (-25.9193146099879641)
      0.179258772950371181
      (0.00268067772490389322 * dpy)
      (0.00162824170038242295 * dpy)
      (-0.000095159225451971587 * dpy)
      (0.000230417297573763929 * solar)
  ]

offsetMomentum :: [Body] -> [Body]
offsetMomentum bs = case bs of
  (sun : rest) -> sun {vx = 0.0 - px / solar, vy = 0.0 - py / solar, vz = 0.0 - pz / solar} : rest
  [] -> []
  where
    (px, py, pz) =
      foldl'
        (\(ax, ay, az) b -> (ax + vx b * m b, ay + vy b * m b, az + vz b * m b))
        (0.0, 0.0, 0.0)
        bs

step :: [Body] -> [Body]
step bs = map drift kicked
  where
    kicked = map kick bs
    kick b = b {vx = vx b + dt * ax, vy = vy b + dt * ay, vz = vz b + dt * az}
      where
        (ax, ay, az) = foldl' acc (0.0, 0.0, 0.0) bs
        acc (sx, sy, sz) o =
          let dx = x o - x b
              dy = y o - y b
              dz = z o - z b
              d2 = dx * dx + dy * dy + dz * dz
           in if d2 == 0.0
                then (sx, sy, sz)
                else
                  let dist = sqrt d2
                      mag = m o / (d2 * dist)
                   in (sx + dx * mag, sy + dy * mag, sz + dz * mag)
    drift b = b {x = x b + dt * vx b, y = y b + dt * vy b, z = z b + dt * vz b}

energy :: [Body] -> Double
energy bs = kinetic + potential
  where
    kinetic = foldl' (\a b -> a + 0.5 * m b * (vx b * vx b + vy b * vy b + vz b * vz b)) 0.0 bs
    n = length bs
    pairs = [(bs !! i, bs !! j) | i <- [0 .. n - 1], j <- [i + 1 .. n - 1]]
    potential = foldl' (\a (bi, bj) -> a - (m bi * m bj) / dist bi bj) 0.0 pairs
    dist bi bj =
      let dx = x bi - x bj
          dy = y bi - y bj
          dz = z bi - z bj
       in sqrt (dx * dx + dy * dy + dz * dz)

forceBodies :: [Body] -> ()
forceBodies = foldr seq ()

report :: Double -> Int
report e = floor (e * 1000000000.0)

run :: [Body] -> Int -> [Body]
run b 0 = b
run b k = let b' = step b in forceBodies b' `seq` run b' (k - 1)

main :: IO ()
main = do
  let b0 = offsetMomentum bodies0
  print (report (energy b0))
  let bf = run b0 (100000 :: Int)
  print (report (energy bf))
