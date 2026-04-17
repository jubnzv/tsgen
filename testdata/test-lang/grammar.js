/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

module.exports = grammar({
  name: "test_lang",

  extras: $ => [
    /\s/,
    $.line_comment,
  ],

  word: $ => $.identifier,

  rules: {
    // Root: zero or more statements (REPEAT)
    program: $ => repeat($.statement),

    // CHOICE over statement kinds
    statement: $ => choice(
      $.let_stmt,
      $.expr_stmt,
      $.if_stmt,
      $.block,
    ),

    // SEQ + FIELD + STRING
    let_stmt: $ => seq(
      "let",
      field("name", $.identifier),
      "=",
      field("value", $.expression),
      ";",
    ),

    // SEQ
    expr_stmt: $ => seq(
      $.expression,
      ";",
    ),

    // SEQ + BLANK (via optional)
    if_stmt: $ => seq(
      "if",
      "(",
      field("condition", $.expression),
      ")",
      field("consequence", $.block),
      optional(seq("else", field("alternative", $.block))),
    ),

    // REPEAT1
    block: $ => seq(
      "{",
      repeat1($.statement),
      "}",
    ),

    // CHOICE over expression kinds
    expression: $ => choice(
      $.number,
      $.identifier,
      $.binary_expr,
      $.paren_expr,
      $.call_expr,
    ),

    // PREC_LEFT + CHOICE (operators)
    binary_expr: $ => prec.left(1, seq(
      field("left", $.expression),
      field("operator", choice("+", "-", "*", "==")),
      field("right", $.expression),
    )),

    // SEQ (parenthesized expression)
    paren_expr: $ => seq(
      "(",
      $.expression,
      ")",
    ),

    // FIELD + ALIAS + BLANK (via optional)
    call_expr: $ => seq(
      field("func", alias($.identifier, $.func_name)),
      "(",
      optional(field("argument", $.expression)),
      ")",
    ),

    // PATTERN — identifier
    identifier: $ => /[a-zA-Z_][a-zA-Z0-9_]*/,

    // PATTERN — number
    number: $ => /[0-9]+/,

    // TOKEN + SEQ + PATTERN
    line_comment: $ => token(seq("//", /.*/)),
  },
});
