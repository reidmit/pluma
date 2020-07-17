if exists("b:current_syntax")
  finish
endif

highlight default link plumaKeyword Keyword
syntax keyword plumaKeyword as
syntax keyword plumaKeyword enum
syntax keyword plumaKeyword let
syntax keyword plumaKeyword match
syntax keyword plumaKeyword mut
syntax keyword plumaKeyword use
syntax keyword plumaKeyword where

highlight default link plumaTopLevelKeyword Keyword
syntax keyword plumaTopLevelKeyword intrinsic_def
syntax keyword plumaTopLevelKeyword intrinsic_type
syntax keyword plumaTopLevelKeyword const
syntax keyword plumaTopLevelKeyword def
syntax keyword plumaTopLevelKeyword struct
syntax keyword plumaTopLevelKeyword trait
syntax keyword plumaTopLevelKeyword private
syntax keyword plumaTopLevelKeyword internal

highlight default link plumaComment Comment
syntax match plumaComment "\v#.*$"

highlight default link plumaString String
syntax region plumaString start='"' end='"' contained

highlight default link plumaNumber Number
syntax match plumaNumber "\v\d+(\.\d+)?" display

highlight default link plumaOperator Operator
syntax match plumaOperator "\v[:|=.*+-/<>~!%&@^?]+"

highlight default link plumaString String
syntax region plumaString start=/"/ skip=/\\"/ end=/"/ contains=plumaInterpolation

highlight default link plumaInterpolation NONE
syntax region plumaInterpolation start="\v\$\(\s*" end="\v\s*\)" contained containedin=plumaString
      \ contains=
      \   plumaComment,
      \   plumaOperator,
      \   plumaString,
      \   plumaNumber,
      \   plumaKeyword,

syntax region plumaBlock start="{" end="}" fold transparent
      \ contains=
      \   plumaComment,
      \   plumaOperator,
      \   plumaString,
      \   plumaNumber,
      \   plumaKeyword,

let b:current_syntax = "pluma"
