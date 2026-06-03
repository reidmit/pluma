-- Sieve of Eratosthenes — a mutable unboxed array marked in place (ST monad),
-- then an order-sensitive checksum over the survivors.
{-# LANGUAGE BangPatterns #-}

import Control.Monad (when)
import Control.Monad.ST (ST)
import Data.Array.ST (STUArray, newArray, readArray, runSTUArray, writeArray)
import Data.Array.Unboxed (UArray, (!))
import Data.List (foldl')

sieveArr :: Int -> UArray Int Bool
sieveArr n = runSTUArray $ do
  arr <- newArray (0, n) False -- False = prime candidate, True = composite
  writeArray arr 0 True
  writeArray arr 1 True
  let loop p
        | p * p > n = return ()
        | otherwise = do
            composite <- readArray arr p
            when (not composite) (mark (p * p))
            loop (p + 1)
        where
          mark j
            | j > n = return ()
            | otherwise = writeArray arr j True >> mark (j + p)
  loop 2
  return arr

main :: IO ()
main = do
  let n = 10000000 :: Int
      arr = sieveArr n
      (cnt, chk) =
        foldl'
          (\(!c, !s) i -> if not (arr ! i) then (c + 1, (s + i) `mod` 1000000007) else (c, s))
          (0 :: Int, 0 :: Int)
          [2 .. n]
  print cnt
  print chk
