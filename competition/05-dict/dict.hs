-- Word-frequency tally over a stream of keys — exercises the hash/ordered map.
import Data.List (foldl')
import qualified Data.Map.Strict as M

main :: IO ()
main = do
  let buckets = 20000 :: Int
      m =
        foldl'
          (\acc i -> M.insertWith (+) (show (i `mod` buckets)) (1 :: Int) acc)
          M.empty
          [0 .. 199999]
  print (M.size m)
  print (M.findWithDefault 0 "0" m)
