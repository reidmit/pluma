; Highlight queries for Pluma. Scopes follow the tree-sitter/Helix/nvim
; convention (@keyword, @function, @type, ...).

; ---- comments / literals -----------------------------------------------------
(comment) @comment

(integer) @number
(float) @number
(boolean) @constant.builtin.boolean
(string) @string
(bytes) @string.special
(escape_sequence) @string.escape

; ---- regex -------------------------------------------------------------------
(regex) @string.regexp
(regex_string) @string
(regex_class) @constant.builtin
(regex_anchor) @operator
(regex_operator) @operator
(regex_named_capture name: (identifier) @variable)
(regex_named_capture ["<" ">" ":"] @punctuation.bracket)

; ---- interpolation -----------------------------------------------------------
(interpolation ["$(" ")"] @punctuation.special)

; ---- keywords ----------------------------------------------------------------
[
  "use" "as" "def" "alias" "enum" "trait" "implement" "test" "where" "built-in"
] @keyword

["let" "try"] @keyword

["fun"] @keyword.function

["if" "else" "when" "is" "while"] @keyword.control.conditional

; ---- operators / punctuation -------------------------------------------------
[
  "+" "-" "*" "/" "%" "**" "++"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "!" "??" "|" ".." "..." "->" "="
] @operator

["::" "."] @punctuation.delimiter
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," ":"] @punctuation.delimiter

; ---- definitions -------------------------------------------------------------
(definition name: (identifier) @function)
(method_signature name: (identifier) @function)
(enum_declaration name: (identifier) @type)
(enum_variant name: (identifier) @constructor)
(alias_declaration name: (identifier) @type)
(trait_declaration name: (identifier) @type.definition)
(constraint trait: (identifier) @type.definition)

(function parameter: (identifier) @variable.parameter)
(constraint parameter: (identifier) @type)

; ---- use / namespaces --------------------------------------------------------
(module_path (identifier) @namespace)
(use_declaration alias: (identifier) @namespace)

; ---- types -------------------------------------------------------------------
(type_identifier module: (identifier) @namespace)
(type_identifier name: (identifier) @type)
((type_identifier name: (identifier) @type.builtin)
 (#match? @type.builtin "^(int|float|string|bool|regex|bytes|nothing|list|option|result|pair|ref|dict)$"))

; ---- calls / field access ----------------------------------------------------
(call function: (identifier) @function.call)
(call function: (field_access field: (identifier) @function.call))
(field_access field: (identifier) @variable.member)
(qualified_identifier . (identifier) @namespace)

; ---- patterns ----------------------------------------------------------------
(constructor_pattern constructor: (identifier) @constructor)
(rest_pattern (identifier) @variable)
(wildcard) @variable.builtin

; ---- records -----------------------------------------------------------------
(record_field key: (identifier) @variable.member)
(record_pattern_field key: (identifier) @variable.member)
(record_type_field name: (identifier) @variable.member)

; ---- fallback: plain identifiers ---------------------------------------------
(identifier) @variable
