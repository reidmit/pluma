-- Sort a pseudo-random integer list with the stdlib stable sort, then fold an
-- order-sensitive checksum over the result.
import Data.List (foldl', sort)

main :: IO ()
main = do
  let n = 100000 :: Int
      xs = [(i * 2654435761) `mod` 100003 | i <- [0 .. n - 1]]
      sorted = sort xs
      checksum = foldl' (\acc v -> (acc * 31 + v) `mod` 1000000007) 0 sorted
  print n
  print checksum
