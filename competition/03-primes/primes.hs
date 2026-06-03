-- Count the primes below a limit by trial division — integer arithmetic, modulo.
isPrime :: Int -> Bool
isPrime n = go 2
  where
    go d
      | d * d > n = True
      | n `mod` d == 0 = False
      | otherwise = go (d + 1)

main :: IO ()
main = print (length (filter isPrime [2 .. 299999]))
