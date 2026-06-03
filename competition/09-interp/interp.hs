-- A tiny arithmetic-expression interpreter: build an AST, evaluate it many
-- times. Arithmetic is mod 1000003 so every language agrees bit-for-bit.
import Data.List (foldl')

data Expr = Lit !Int | Add Expr Expr | Sub Expr Expr | Mul Expr Expr | Neg Expr

m :: Int
m = 1000003

build :: Int -> Int -> Expr
build 0 seed = Lit (seed `mod` 7)
build depth seed =
  let l = build (depth - 1) (seed * 2 + 1)
      r = build (depth - 1) (seed * 2 + 2)
   in case seed `mod` 4 of
        0 -> Add l r
        1 -> Sub l r
        2 -> Mul l r
        _ -> Neg l

eval :: Expr -> Int
eval (Lit v) = v
eval (Add l r) = (eval l + eval r) `mod` m
eval (Sub l r) = (eval l - eval r + m) `mod` m
eval (Mul l r) = (eval l * eval r) `mod` m
eval (Neg x) = (m - eval x) `mod` m

main :: IO ()
main = do
  let tree = build 18 1
  print (foldl' (\acc _ -> (acc + eval tree) `mod` m) 0 [1 .. 2000 :: Int])
