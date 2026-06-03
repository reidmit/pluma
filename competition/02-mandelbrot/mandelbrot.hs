-- Mandelbrot set — pure float64 arithmetic in a tight escape loop over a grid.
inSet :: Double -> Double -> Int
inSet cr ci = go 0.0 0.0 (0 :: Int)
  where
    go zr zi i
      | i >= 1000 = 1
      | zr2 + zi2 > 4.0 = 0
      | otherwise = go (zr2 - zi2 + cr) (2.0 * zr * zi + ci) (i + 1)
      where
        zr2 = zr * zr
        zi2 = zi * zi

main :: IO ()
main = do
  let width = 150 :: Int
      dx = 2.5 / fromIntegral width
      dy = 2.5 / fromIntegral width
      count =
        sum
          [ inSet (-2.0 + fromIntegral px * dx) (-1.25 + fromIntegral py * dy)
          | py <- [0 .. width - 1]
          , px <- [0 .. width - 1]
          ]
  print count
