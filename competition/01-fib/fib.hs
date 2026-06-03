-- Naive recursive Fibonacci — function-call overhead and non-tail recursion.
fib :: Int -> Int
fib n = if n < 2 then n else fib (n - 1) + fib (n - 2)

main :: IO ()
main = print (fib 32)
