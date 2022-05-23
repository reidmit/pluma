# operators

## binary operators

these are all the operators that Pluma supports

operators can not be overloaded

math:
note that ints and floats have different sets of operators!

- logical and: `&& :: bool bool -> bool`
- logical or: `|| :: bool bool -> bool`

- integer addition: `+ :: int int -> int`
- integer subtraction: `- :: int int -> int`
- integer multiplication: `* :: int int -> int`
- integer division: `/ :: int int -> int`
- integer remainder: `% :: int int -> int`

- float addition: `+. :: float float -> float`
- float subtraction: `-. :: float float -> float`
- float multiplication: `*. :: float float -> float`
- float division: `/. :: float float -> float`
- float remainder: `%. :: float float -> float`

- string concatenation: `++ :: string string -> string`

- optional coalescing: `?? :: (option 'a) 'a -> 'a`

