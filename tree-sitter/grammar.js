/**
 * Tree-sitter grammar for Pluma (.pa).
 *
 * Design notes:
 * - `word: $.identifier` makes the lexer match the longest identifier, so
 *   kebab-case (`to-string`, `this-is-it`) is one token and keywords only fire
 *   as whole words. This is the whole reason no lookarounds are needed (unlike
 *   the TextMate/Sublime grammars).
 * - Statements are newline-separated (no `;`). An external scanner emits
 *   `_newline` only where the parser expects a terminator (see src/scanner.c),
 *   so newlines inside `()`/`[]` and after binary operators are ignored —
 *   line continuation falls out for free.
 * - Precedence mirrors compiler/src/operator.rs binding powers.
 */

const PREC = {
  pipe: 1,
  range: 10,
  or: 20,
  coalesce: 21,
  and: 30,
  not: 35,
  equality: 40,
  comparison: 50,
  additive: 60,
  multiplicative: 70,
  unary: 75,
  exponent: 80,
  call: 90,
  access: 100,
};

function sep1(sep, rule) {
  return seq(rule, repeat(seq(sep, rule)));
}
function commaSep1(rule) {
  return sep1(',', rule);
}

module.exports = grammar({
  name: 'pluma',

  word: $ => $.identifier,

  externals: $ => [$._newline],

  extras: $ => [/[ \t\r]/, $.comment],

  conflicts: $ => [
    [$.block, $.record],
    [$._type_atom, $.generic_type],
    [$._primary_expression, $.record_field],
    [$._primary_expression, $.if_expression],
    [$._primary_expression, $.while_expression],
    [$.when_expression],
    [$.if_expression],
    [$._pattern_atom, $.constructor_pattern],
    [$.record],
    [$.record_type],
    [$.record_pattern],
  ],

  supertypes: $ => [$._expression, $._statement, $._pattern, $._type],

  rules: {
    source_file: $ => statements($, $._top_level_item),

    comment: _ => token(seq('#', /[^\n]*/)),

    // ---- top level -------------------------------------------------------

    _top_level_item: $ => choice(
      $.use_declaration,
      $.definition,
      $.enum_declaration,
      $.alias_declaration,
      $.trait_declaration,
      $.implement_declaration,
      $.test_declaration,
    ),

    use_declaration: $ => seq(
      'use',
      field('path', $.module_path),
      optional(seq('as', field('alias', $.identifier))),
    ),

    module_path: $ => sep1('.', $.identifier),

    definition: $ => seq(
      'def',
      field('name', $.identifier),
      optional(seq('::', field('type', $._type))),
      optional($.where_clause),
      '=',
      field('value', $._expression),
    ),

    where_clause: $ => seq('where', '(', commaSep1($.constraint), ')'),
    constraint: $ => seq(field('trait', $.identifier), field('parameter', $.identifier)),

    enum_declaration: $ => seq(
      'enum',
      field('name', $.identifier),
      repeat(field('type_parameter', $.identifier)),
      $.enum_body,
    ),
    enum_body: $ => seq('{', statements($, $.enum_variant), '}'),
    enum_variant: $ => prec.left(seq(
      field('name', $.identifier),
      repeat($._type_atom),
    )),

    alias_declaration: $ => seq(
      'alias',
      field('name', $.identifier),
      field('type', $._type),
    ),

    trait_declaration: $ => seq(
      'trait',
      field('name', $.identifier),
      field('parameter', $.identifier),
      $.trait_body,
    ),
    trait_body: $ => seq('{', statements($, $._trait_member), '}'),
    _trait_member: $ => choice($.method_signature, $.definition),
    method_signature: $ => seq(field('name', $.identifier), '::', field('type', $._type)),

    implement_declaration: $ => seq(
      'implement',
      field('trait', $.identifier),
      field('type', $._type),
      optional($.where_clause),
      $.implement_body,
    ),
    implement_body: $ => seq('{', statements($, $.definition), '}'),

    test_declaration: $ => seq('test', field('description', $.string), $.block),

    // ---- statements ------------------------------------------------------

    _statement: $ => choice(
      $.let_binding,
      $.try_binding,
      $._expression,
    ),

    let_binding: $ => seq(
      'let',
      field('pattern', $._pattern),
      optional(seq('::', field('type', $._type))),
      '=',
      field('value', $._expression),
    ),

    try_binding: $ => seq(
      'try',
      field('pattern', $._pattern),
      '=',
      field('value', $._expression),
    ),

    // ---- expressions -----------------------------------------------------

    _expression: $ => choice(
      $.binary_expression,
      $.unary_expression,
      $.call,
      $._postfix_expression,
    ),

    _postfix_expression: $ => choice(
      $.field_access,
      $.element_access,
      $._primary_expression,
    ),

    _primary_expression: $ => choice(
      $.identifier,
      $.integer,
      $.float,
      $.string,
      $.bytes,
      $.regex,
      $.boolean,
      $.builtin,
      $.list,
      $.record,
      $.tuple,
      $.grouping,
      $.block,
      $.function,
      $.if_expression,
      $.when_expression,
      $.while_expression,
    ),

    // Application by juxtaposition: `f a b`. Arguments are postfix-level so
    // `f a + b` is `(f a) + b`, not `f (a + b)`.
    call: $ => prec.left(PREC.call, seq(
      field('function', $._expression),
      field('argument', $._argument),
    )),
    _argument: $ => $._postfix_expression,

    field_access: $ => prec.left(PREC.access, seq(
      field('record', $._postfix_expression),
      '.',
      field('field', $.identifier),
    )),
    element_access: $ => prec.left(PREC.access, seq(
      field('tuple', $._postfix_expression),
      '.',
      field('index', $.integer),
    )),

    unary_expression: $ => choice(
      prec(PREC.unary, seq('-', $._expression)),
      prec(PREC.not, seq('!', $._expression)),
    ),

    binary_expression: $ => {
      const table = [
        [PREC.pipe, '|', 'left'],
        [PREC.range, '..', 'left'],
        [PREC.or, '||', 'left'],
        [PREC.coalesce, '??', 'right'],
        [PREC.and, '&&', 'left'],
        [PREC.equality, '==', 'left'],
        [PREC.equality, '!=', 'left'],
        [PREC.comparison, '<', 'left'],
        [PREC.comparison, '<=', 'left'],
        [PREC.comparison, '>', 'left'],
        [PREC.comparison, '>=', 'left'],
        [PREC.additive, '+', 'left'],
        [PREC.additive, '-', 'left'],
        [PREC.additive, '++', 'left'],
        [PREC.multiplicative, '*', 'left'],
        [PREC.multiplicative, '/', 'left'],
        [PREC.multiplicative, '%', 'left'],
        [PREC.exponent, '**', 'right'],
      ];
      return choice(...table.map(([p, op, assoc]) => {
        const fn = assoc === 'left' ? prec.left : prec.right;
        return fn(p, seq(
          field('left', $._expression),
          field('operator', op),
          field('right', $._expression),
        ));
      }));
    },

    function: $ => seq(
      'fun',
      repeat(field('parameter', $.identifier)),
      field('body', $.block),
    ),

    if_expression: $ => seq(
      'if',
      field('subject', $._expression),
      optional(seq('is', field('pattern', $._pattern))),
      field('consequence', $.block),
      optional($.else_clause),
    ),
    else_clause: $ => seq(
      optional($._newline),
      'else',
      field('alternative', choice($.block, $.if_expression)),
    ),

    when_expression: $ => seq(
      'when',
      field('subject', $._expression),
      repeat1(seq(optional($._newline), $.when_case)),
      optional(seq(optional($._newline), $.when_else)),
    ),
    when_case: $ => seq('is', field('pattern', $._pattern), field('body', $.block)),
    when_else: $ => seq('else', field('body', $.block)),

    while_expression: $ => seq(
      'while',
      field('subject', $._expression),
      optional(seq('is', field('pattern', $._pattern))),
      field('body', $.block),
    ),

    block: $ => seq('{', statements($, $._statement), '}'),

    list: $ => seq('[', _listish($._list_element), ']'),
    _list_element: $ => choice($.spread_element, $._expression),
    spread_element: $ => seq('...', $._expression),

    record: $ => seq('{', recordItems($, $.record_field), '}'),
    record_field: $ => choice(
      seq(field('key', $.identifier), ':', field('value', $._expression)),
      field('key', $.identifier),
    ),

    // A tuple is empty `()` or has 2+ elements; a single `(x)` is grouping.
    tuple: $ => seq('(', optional(seq($._expression, repeat1(seq(',', $._expression)), optional(','))), ')'),
    grouping: $ => seq('(', $._expression, ')'),

    builtin: $ => seq('built-in', field('tag', $.string)),

    // ---- patterns --------------------------------------------------------

    _pattern: $ => choice(
      $.constructor_pattern,
      $._pattern_atom,
    ),
    _pattern_atom: $ => choice(
      $.identifier,
      $.qualified_identifier,
      $.wildcard,
      $.integer,
      $.float,
      $.string,
      $.bytes,
      $.boolean,
      $.tuple_pattern,
      $.record_pattern,
      $.list_pattern,
      $.grouping_pattern,
    ),
    constructor_pattern: $ => prec.left(seq(
      field('constructor', choice($.identifier, $.qualified_identifier)),
      repeat1($._pattern_atom),
    )),
    qualified_identifier: $ => prec(PREC.access, seq($.identifier, '.', $.identifier)),
    wildcard: _ => '_',
    grouping_pattern: $ => seq('(', $._pattern, ')'),
    tuple_pattern: $ => seq('(', optional(seq($._pattern, repeat1(seq(',', $._pattern)), optional(','))), ')'),
    record_pattern: $ => seq('{', recordItems($, choice($.record_pattern_field, $.rest_pattern)), '}'),
    record_pattern_field: $ => choice(
      seq(field('key', $.identifier), ':', field('pattern', $._pattern)),
      field('key', $.identifier),
    ),
    // `[]`, `[a, b]`, `[...rest]`, `[a, ...rest]` — rest is trailing.
    list_pattern: $ => seq('[', optional(choice(
      $.rest_pattern,
      seq($._pattern, repeat(seq(',', $._pattern)), optional(seq(',', $.rest_pattern))),
    )), ']'),
    rest_pattern: $ => seq('...', optional($.identifier)),

    // ---- types -----------------------------------------------------------

    _type: $ => choice(
      $.function_type,
      $.generic_type,
      $._type_atom,
    ),
    _type_atom: $ => choice(
      $.type_identifier,
      $.record_type,
      $.tuple_type,
      $.grouping_type,
    ),
    type_identifier: $ => seq(
      optional(seq(field('module', $.identifier), '.')),
      field('name', $.identifier),
    ),
    generic_type: $ => prec.left(seq(
      field('constructor', $.type_identifier),
      repeat1($._type_atom),
    )),
    function_type: $ => prec.right(seq(
      'fun',
      repeat($._type_atom),
      '->',
      $._type,
    )),
    record_type: $ => seq('{', recordItems($, $.record_type_field), '}'),
    record_type_field: $ => seq(field('name', $.identifier), '::', field('type', $._type)),
    tuple_type: $ => seq('(', optional(seq($._type, repeat1(seq(',', $._type)), optional(','))), ')'),
    grouping_type: $ => seq('(', $._type, ')'),

    // ---- tokens ----------------------------------------------------------

    identifier: _ => /[A-Za-z_][A-Za-z0-9_-]*/,

    integer: _ => token(choice(
      /0[xX][0-9A-Fa-f][0-9A-Fa-f_]*/,
      /0[oO][0-7][0-7_]*/,
      /0[bB][01][01_]*/,
      /[0-9][0-9_]*/,
    )),
    float: _ => token(/[0-9][0-9_]*\.[0-9][0-9_]*/),
    boolean: _ => choice('true', 'false'),

    string: $ => seq(
      '"',
      repeat(choice(
        $.escape_sequence,
        $.interpolation,
        token.immediate(prec(1, /[^"\\$]+/)),
        token.immediate('$'),
      )),
      '"',
    ),
    interpolation: $ => seq('$(', $._expression, ')'),
    escape_sequence: _ => token.immediate(/\\./),

    bytes: $ => seq(
      "'",
      repeat(choice(
        $.escape_sequence,
        token.immediate(prec(1, /[^'\\]+/)),
      )),
      "'",
    ),

    // Regex literal: `` `...` ``. Atoms are whitespace-separated (whitespace is
    // meaningless), so the interior tokens are ordinary (non-immediate) and the
    // global whitespace `extras` skips between them.
    regex: $ => seq('`', repeat($._regex_item), '`'),
    _regex_item: $ => choice(
      $.regex_string,
      $.regex_named_capture,
      $.regex_class,
      $.regex_anchor,
      $.regex_operator,
      $.regex_atom,
    ),
    // `<name: subpattern>`
    regex_named_capture: $ => seq('<', field('name', $.identifier), ':', repeat($._regex_item), '>'),
    regex_string: $ => seq('"', repeat(choice($.escape_sequence, token.immediate(/[^"\\]+/))), '"'),
    regex_class: _ => token(choice('digit', 'letter', 'word', 'whitespace', 'any')),
    regex_anchor: _ => token(/[$%^]/),
    regex_operator: _ => token(/[|?*+(){}]/),
    regex_atom: _ => token(/[^`"^$%|?*+(){}<>:\s]+/),
  },
});

// Statements separated by one or more newlines, tolerant of blank lines and
// of leading/trailing newlines (e.g. just inside a brace).
function statements($, rule) {
  return seq(
    repeat($._newline),
    optional(seq(
      rule,
      repeat(seq(repeat1($._newline), rule)),
      repeat($._newline),
    )),
  );
}

// Comma-separated items that may also span lines (newlines ignored, since the
// scanner won't emit `_newline` where it isn't grammatically valid).
function _listish(rule) {
  return optional(seq(commaSep1(rule), optional(',')));
}

// Record-style fields: separated by commas and/or newlines (Pluma allows both),
// with optional leading/trailing newlines just inside the braces.
function recordItems($, rule) {
  const sep = repeat1(choice(',', $._newline));
  return optional(seq(
    repeat($._newline),
    rule,
    repeat(seq(sep, rule)),
    optional(','),
    repeat($._newline),
  ));
}
