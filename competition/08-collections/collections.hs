-- Functional collection pipeline: range -> map -> filter -> fold. Lazy lists
-- fuse the intermediates away, which is idiomatic Haskell.
import Data.List (foldl')

main :: IO ()
main =
  print
    ( foldl'
        (\acc x -> (acc + x) `mod` 1000000007)
        0
        (filter even (map (\x -> x * x) [1 .. 1000000 :: Int]))
    )
