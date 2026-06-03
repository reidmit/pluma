-- String round-trip: build many parts, join, split back, uppercase — using
-- Data.Text, the idiomatic high-throughput string type.
import Data.Text (Text)
import qualified Data.Text as T

main :: IO ()
main = do
  let parts = [T.pack ("item-" ++ show i) | i <- [0 .. 49999 :: Int]]
      joined = T.intercalate (T.pack ",") parts
      back = T.splitOn (T.pack ",") joined
      upper = T.toUpper joined
  print (length back)
  print (T.length joined)
  print (T.length upper)
