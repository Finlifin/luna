/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const PREC = {
  ASSIGN: 1,
  CLAUSE: 2,
  RANGE: 5,
  IMPLIES: 10,
  OR: 20,
  AND: 30,
  COMPARE: 40,
  ARROW: 50,
  ADD: 60,
  MUL: 70,
  PIPE: 80,
  PREFIX: 90,
  POSTFIX: 100,
  MEMBER: 110,
  SUFFIX: 115,
};

function commaSep(rule) {
  return optional(commaSep1(rule));
}

function commaSep1(rule) {
  return seq(rule, repeat(seq(',', rule)), optional(','));
}

module.exports = grammar({
  name: 'flurry',

  word: $ => $.identifier,

  extras: $ => [
    /\s/,
    $.line_comment,
    $.block_comment,
  ],

  externals: $ => [
    $.block_comment,
    $._sep_plus,
    $._sep_minus,
    $._sep_star,
    $._sep_slash,
    $._sep_percent,
    $._sep_lt,
    $._sep_gt,
    $._tight_lt,
    $._tight_gt,
    $._take_view_tick,
    $.char_literal,
    $.macro_content,
    $.string_content,
    $.raw_block_content,
    $._error_sentinel,
  ],

  supertypes: $ => [
    $._expression,
    $._pattern,
    $._definition,
    $._statement,
  ],

  conflicts: $ => [
    [$.object_expression, $.block],
    [$.function_modifier, $.parameter_modifier],
    [$._primary_expression, $.parameter_type],
    [$.enum_variant, $._primary_expression],
    [$._primary_expression, $.runtime_binding],
  ],

  rules: {
    source_file: $ => repeat($._file_item),

    _file_item: $ => choice(
      $.configuration_directive,
      $.macro_import,
      $.template_macro_definition,
      $.ast_macro_definition,
      $._definition,
      $._statement,
      ';',
    ),

    // Comments
    line_comment: _ => token(seq('--', /[^\n]*/)),

    // Identifiers and basic tokens
    identifier: _ => /[a-zA-Z_\u{2600}-\u{27BF}\u{1F000}-\u{1F9FF}][a-zA-Z0-9_\u{2600}-\u{27BF}\u{1F000}-\u{1F9FF}]*/u,

    _member_name: $ => choice(
      $.identifier,
      alias('default', $.identifier),
    ),

    wildcard: _ => '_',

    self_literal: _ => choice('self', 'Self', 'itself', 'Itself'),

    boolean_literal: _ => choice('true', 'false'),
    null_literal: _ => 'null',
    undefined_literal: _ => 'undefined',
    unit_literal: _ => seq('(', ')'),

    integer_literal: _ => token(choice(
      /0b[01][01_]*/,
      /0o[0-7][0-7_]*/,
      /0x[0-9a-fA-F][0-9a-fA-F_]*/,
      /[0-9][0-9_]*/,
    )),

    bit_vec_prefix: _ => token(choice('0b', '0o', '0x')),

    float_literal: _ => token(seq(
      /[0-9][0-9_]*/,
      '.',
      /[0-9][0-9_]*/,
      optional(seq(/[eE]/, optional(/[+-]/), /[0-9][0-9_]*/)),
    )),

    escape_sequence: _ => token.immediate(seq(
      '\\',
      choice(
        /[ntr\\'"0$]/,
        /x\{[0-9a-fA-F]{1,6}\}/,
      ),
    )),

    string_interpolation: $ => seq(
      token.immediate('${'),
      $._expression,
      '}',
    ),

    string_literal: $ => seq(
      '"',
      repeat(choice(
        $.string_content,
        $.escape_sequence,
        $.string_interpolation,
      )),
      '"',
    ),

    raw_string_literal: $ => prec(2, seq(
      '@',
      'builtin',
      '.',
      'str',
      '.',
      'raw',
      '{',
      optional($.raw_block_content),
      '}',
    )),

    suffixed_literal: $ => prec(PREC.SUFFIX, seq(
      field('value', choice(
        $.integer_literal,
        $.float_literal,
        $.char_literal,
        $.string_literal,
      )),
      field('suffix', $.identifier),
    )),

    symbol_literal: $ => prec.right(PREC.MEMBER, seq(
      '.',
      field('name', $._member_name),
    )),

    // Attributes & visibility
    attribute: $ => prec.right(seq('^', field('value', $._expr_no_ext))),

    visibility: _ => choice('pub', 'private'),

    _item_prefix: $ => repeat1(choice($.attribute, $.visibility)),

    // Configuration (.id expr;)
    configuration_directive: $ => prec.right(seq(
      '.',
      field('name', $._member_name),
      field('value', $._expression),
      optional(';'),
    )),

    // Use declarations & paths
    use_declaration: $ => prec.right(seq(
      optional($._item_prefix),
      'use',
      field('path', $._use_path),
      optional(';'),
    )),

    macro_import: $ => prec.right(seq(
      optional($._item_prefix),
      '@',
      'use',
      field('path', $._use_path),
      optional(';'),
    )),

    _use_path: $ => choice(
      $._member_name,
      $.super_use_path,
      $.package_use_path,
      $.scoped_use_path,
      $.use_list_path,
      $.use_wildcard_path,
      $.use_as_clause,
    ),

    super_use_path: $ => prec.right(1, seq('.', $._use_path)),
    package_use_path: $ => prec.right(1, seq('@', $._use_path)),

    scoped_use_path: $ => prec.left(2, seq(
      field('path', $._use_path),
      '.',
      field('name', $._member_name),
    )),

    use_list_path: $ => prec.left(2, seq(
      field('path', $._use_path),
      '.',
      '{',
      commaSep($._use_path),
      '}',
    )),

    use_wildcard_path: $ => prec.left(2, seq(
      field('path', $._use_path),
      '.',
      '*',
    )),

    use_as_clause: $ => prec.left(0, seq(
      field('path', $._use_path),
      'as',
      field('alias', $.identifier),
    )),

    // Macros
    template_macro_definition: $ => seq(
      'define',
      field('name', $.identifier),
      choice(
        seq($.integer_literal, optional('...')),
        seq('(', commaSep($.identifier), ')'),
      ),
      $.block,
    ),

    ast_macro_definition: $ => seq(
      '@',
      choice('define_macro1', 'define_macro2'),
      '{',
      optional($.raw_block_content),
      '}',
    ),

    ast_macro_call: $ => prec(PREC.POSTFIX, seq(
      '@',
      field('macro', $.macro_path),
      choice(
        seq('(', commaSep($.argument), ')'),
        $.block,
      ),
    )),

    macro_path: $ => prec.left(seq(
      $.identifier,
      repeat(seq('.', $._member_name)),
    )),

    template_macro_call: $ => prec.right(seq(
      '$',
      field('name', $.identifier),
      optional(seq('(', commaSep($._expression), ')')),
    )),

    token_buffer: $ => seq(
      '{{',
      repeat(choice(
        $._statement,
        $._definition,
        $.token_splice,
      )),
      '}}',
    ),

    token_splice: $ => seq(
      '$',
      choice($.integer_literal, $.parenthesized_expression),
    ),

    // Definitions
    _definition: $ => choice(
      $.function_definition,
      $.effect_definition,
      $.effect_enum_definition,
      $.case_definition,
      $.const_definition,
      $.newtype_definition,
      $.typealias_definition,
      $.trait_definition,
      $.assoc_declaration,
      $.impl_definition,
      $.derive_definition,
      $.deriver_definition,
      $.test_definition,
      $.mod_definition,
      $.struct_definition,
      $.enum_definition,
      $.union_definition,
      $.predicate_definition,
      $.use_declaration,
    ),

    function_modifier: _ => choice(
      'pure',
      'comptime',
      'inline',
      'unsafe',
      'spec',
      'verified',
      'atomic',
      'async',
      'quote',
      'template',
    ),

    function_definition: $ => prec.right(seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'fn',
      field('name', $.identifier),
      optional(field('parameters', choice(
        $.parameter_list,
        $.type_parameter_list,
      ))),
      optional(seq('use', field('template', $._expr_no_ext))),
      optional(seq('->', field('return_type', $.return_type))),
      optional($.where_clause),
      choice(
        field('body', $.block),
        seq('=', field('value', $._expression), optional(';')),
        ';',
      ),
    )),

    return_type: $ => choice(
      prec(1, seq(field('name', $.identifier), ':', field('type', $._expr_no_ext))),
      field('type', $._expr_no_ext),
    ),

    parameter_list: $ => seq(
      '(',
      commaSep($.parameter),
      ')',
    ),

    type_parameter_list: $ => seq(
      $._tight_lt,
      commaSep($.parameter),
      $._tight_gt,
    ),

    parameter_modifier: _ => choice(
      'comptime',
      'error',
      'catch',
      'lambda',
      'implicit',
      'quote',
      'assoc',
    ),

    parameter: $ => seq(
      repeat($.attribute),
      repeat($.parameter_modifier),
      choice(
        seq(optional('*'), $.self_literal),
        seq('.', field('name', $.identifier), ':', field('type', $._expr_no_ext), '=', field('default', $._expr_no_ext)),
        seq('...', field('name', $.identifier), ':', field('type', $._expr_no_ext)),
        seq('case', field('pattern', $._pattern)),
        seq(field('name', choice($.identifier, $.wildcard)), optional(seq(':', field('type', $._expr_no_ext)))),
      ),
    ),

    where_clause: $ => prec.right(PREC.CLAUSE, seq(
      'where',
      commaSep1($.clause),
    )),

    clause: $ => seq(
      repeat($.attribute),
      choice(
        $.requires_clause,
        $.ensures_clause,
        $.decreases_clause,
        $.outcomes_clause,
        $.decl_clause,
      ),
    ),

    requires_clause: $ => seq('requires', field('condition', $._expr_no_ext)),
    ensures_clause: $ => seq('ensures', field('condition', $._expr_no_ext)),
    decreases_clause: $ => seq('decreases', field('measure', $._expr_no_ext)),
    outcomes_clause: $ => seq('outcomes', field('outcomes', $._expr_no_ext)),

    decl_clause: $ => seq(
      optional('quote'),
      choice(
        seq('.', field('name', $.identifier), ':', field('type', $._expr_no_ext), '=', field('default', $._expr_no_ext)),
        seq('...', field('name', $.identifier), ':', field('type', $._expr_no_ext)),
        seq(field('name', $.identifier), ':-', field('bound', $._expr_no_ext)),
        seq(field('name', $.identifier), ':', field('type', $._expr_no_ext)),
        field('name', $.identifier),
      ),
    ),

    effect_definition: $ => prec.right(seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'effect',
      field('name', $.identifier),
      field('parameters', $.parameter_list),
      optional(seq('->', field('return_type', $.return_type))),
      optional($.where_clause),
      optional(';'),
    )),

    effect_enum_definition: $ => seq(
      optional($._item_prefix),
      'effect',
      'enum',
      field('name', $.identifier),
      optional($.where_clause),
      '{',
      repeat(seq($.effect_variant, optional(choice(',', ';')))),
      '}',
    ),

    effect_variant: $ => seq(
      repeat($.attribute),
      repeat(choice('async', 'inline')),
      field('name', $.identifier),
      field('parameters', $.parameter_list),
      optional(seq('->', field('return_type', $.return_type))),
    ),

    case_definition: $ => seq(
      optional($._item_prefix),
      'case',
      field('name', $.identifier),
      field('parameters', $.parameter_list),
      optional(seq('->', field('return_type', $.return_type))),
      optional($.where_clause),
      field('body', $.block),
    ),

    const_definition: $ => prec.right(seq(
      optional($._item_prefix),
      'const',
      field('pattern', $._pattern_no_ext),
      optional(seq(':', field('type', $._expr_no_ext))),
      '=',
      field('value', $._expression),
      optional(seq('else', field('else_branch', $.block))),
      optional(';'),
    )),

    newtype_definition: $ => prec.right(seq(
      optional($._item_prefix),
      'newtype',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameter_list)),
      optional($.where_clause),
      '=',
      field('type', $._expression),
      optional(';'),
    )),

    typealias_definition: $ => prec.right(seq(
      optional($._item_prefix),
      'typealias',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameter_list)),
      optional($.where_clause),
      '=',
      field('type', $._expression),
      optional(';'),
    )),

    trait_definition: $ => seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'trait',
      field('name', $.identifier),
      optional(seq(':-', field('bounds', $._expr_no_ext))),
      optional($.where_clause),
      field('body', $.declaration_body),
    ),

    assoc_declaration: $ => prec.right(seq(
      optional($._item_prefix),
      'assoc',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameter_list)),
      optional(seq(':', field('type', $._expr_no_ext))),
      optional(seq('=', field('default', $._expr_no_ext))),
      optional($.where_clause),
      optional(';'),
    )),

    impl_definition: $ => seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'impl',
      field('subject', $._expr_no_ext),
      optional(seq('for', field('target', $._expr_no_ext))),
      optional($.where_clause),
      field('body', $.declaration_body),
    ),

    derive_definition: $ => prec.right(seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'derive',
      field('trait', $._expr_no_ext),
      'for',
      field('target', $._expr_no_ext),
      optional($.where_clause),
      optional(';'),
    )),

    deriver_definition: $ => seq(
      optional($._item_prefix),
      'derive',
      'fn',
      field('name', $.identifier),
      optional(field('parameters', $.parameter_list)),
      'for',
      field('target', $._expr_no_ext),
      optional($.where_clause),
      field('body', $.declaration_body),
    ),

    test_definition: $ => seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'test',
      optional(field('name', choice($.identifier, $.string_literal))),
      optional(seq('use', field('template', $._expr_no_ext))),
      optional($.where_clause),
      field('body', $.block),
    ),

    mod_definition: $ => seq(
      optional($._item_prefix),
      'mod',
      field('name', $.identifier),
      field('body', $.declaration_body),
    ),

    declaration_body: $ => seq(
      '{',
      repeat(choice(
        $.configuration_directive,
        $._definition,
        $._statement,
        ';',
      )),
      '}',
    ),

    struct_definition: $ => seq(
      optional($._item_prefix),
      'struct',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameter_list)),
      optional($.where_clause),
      field('body', $.struct_body),
    ),

    struct_body: $ => seq(
      '{',
      repeat(choice(
        seq($.struct_field, optional(choice(',', ';'))),
        $.configuration_directive,
        $._definition,
        $._statement,
        ',',
        ';',
      )),
      '}',
    ),

    struct_field: $ => prec(2, seq(
      optional($._item_prefix),
      field('name', $.identifier),
      ':',
      field('type', $._expr_no_ext),
      optional(seq('=', field('default', $._expr_no_ext))),
    )),

    enum_definition: $ => seq(
      optional($._item_prefix),
      'enum',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameter_list)),
      optional($.where_clause),
      field('body', $.enum_body),
    ),

    enum_body: $ => seq(
      '{',
      repeat(choice(
        seq($.enum_variant, optional(choice(',', ';'))),
        $.configuration_directive,
        $._definition,
        $._statement,
        ',',
        ';',
      )),
      '}',
    ),

    enum_variant: $ => prec(2, seq(
      optional($._item_prefix),
      field('name', $.identifier),
      optional(choice(
        seq('(', commaSep($._expression), ')'),
        seq('{', commaSep($.struct_field), '}'),
        seq(':', field('pattern', $._pattern)),
        seq('=', field('value', $._expr_no_ext)),
        seq('.', '{', commaSep($.enum_variant), '}'),
      )),
    )),

    union_definition: $ => seq(
      optional($._item_prefix),
      'union',
      field('name', $.identifier),
      optional(field('type_parameters', $.type_parameter_list)),
      optional($.where_clause),
      field('body', $.struct_body),
    ),

    predicate_definition: $ => seq(
      optional($._item_prefix),
      repeat($.function_modifier),
      'predicate',
      field('name', $.identifier),
      field('parameters', $.parameter_list),
      optional(seq('->', field('return_type', $._expr_no_ext))),
      optional($.where_clause),
      choice(field('body', $.block), ';'),
    ),

    // Statements & Blocks
    block: $ => seq(
      '{',
      repeat(choice(
        $._definition,
        $._statement,
        ';',
      )),
      '}',
    ),

    _statement: $ => choice(
      $.attributed_statement,
      $._unattributed_statement,
    ),

    _unattributed_statement: $ => choice(
      $.let_statement,
      $.var_statement,
      $.assignment_statement,
      $.for_statement,
      $.while_statement,
      $.verification_statement,
      $.expression_statement,
    ),

    attributed_statement: $ => prec.right(seq(
      $._item_prefix,
      $._unattributed_statement,
    )),

    let_statement: $ => prec.right(seq(
      'let',
      field('pattern', $._pattern_no_ext),
      optional(seq(':', field('type', $._expr_no_ext))),
      '=',
      field('value', $._expression),
      optional(seq('else', field('else_branch', $.block))),
      optional(';'),
    )),

    var_statement: $ => prec.right(seq(
      'var',
      field('pattern', $._pattern_no_ext),
      optional(seq(':', field('type', $._expr_no_ext))),
      '=',
      field('value', $._expression),
      optional(seq('else', field('else_branch', $.block))),
      optional(';'),
    )),

    assignment_statement: $ => prec.right(PREC.ASSIGN, seq(
      field('left', $._expression),
      field('operator', choice('=', '+=', '-=', '*=', '/=', '%=')),
      field('right', $._expression),
      optional(';'),
    )),

    loop_label: $ => seq(':', field('name', $.identifier)),

    for_statement: $ => seq(
      'for',
      optional($.loop_label),
      field('pattern', $._pattern_no_ext),
      'in',
      field('iterator', $._expr_no_ext),
      field('body', $.block),
    ),

    while_statement: $ => seq(
      'while',
      optional($.loop_label),
      field('condition', $._expr_no_ext),
      choice(
        field('body', $.block),
        seq('is', 'do', '{', repeat($.match_arm), '}'),
        seq('is', field('pattern', $._pattern_no_ext), 'do', field('body', $.block)),
      ),
    ),

    if_expression: $ => prec.right(seq(
      'if',
      field('condition', $._expr_no_ext),
      choice(
        seq(
          field('consequence', $.block),
          optional(seq('else', field('alternative', choice($.block, $.if_expression)))),
        ),
        seq('is', 'do', '{', repeat($._match_entry), '}'),
        seq(
          'is',
          field('pattern', $._pattern_no_ext),
          'do',
          field('consequence', $.block),
          optional(seq('else', field('alternative', choice($.block, $.if_expression)))),
        ),
      ),
    )),

    when_expression: $ => seq(
      'when',
      '{',
      repeat(choice($.condition_arm, $.else_arm)),
      '}',
    ),

    inline_statement: $ => seq(
      'inline',
      choice(
        $.if_expression,
        $.when_expression,
        $.for_statement,
      ),
    ),

    return_expression: $ => prec.right(1, seq(
      'return',
      optional(field('value', $._expression)),
      optional(seq('while', field('condition', $._expression))),
    )),

    resume_expression: $ => prec.right(1, seq(
      'resume',
      optional(field('value', $._expression)),
      optional(seq('while', field('condition', $._expression))),
    )),

    break_expression: $ => prec.right(1, seq(
      'break',
      optional(field('label', $.identifier)),
      optional(seq('while', field('condition', $._expression))),
    )),

    continue_expression: $ => prec.right(1, seq(
      'continue',
      optional(field('label', $.identifier)),
      optional(seq('while', field('condition', $._expression))),
    )),

    verification_statement: $ => prec.right(seq(
      choice('asserts', 'assumes', 'invariant', 'decreases', 'axiom'),
      field('expression', $._expression),
      optional(';'),
    )),

    expression_statement: $ => prec.right(-1, seq(
      $._expression,
      optional(';'),
    )),

    // Match & Condition Arms
    _match_entry: $ => choice(
      $.match_arm,
      $.arm_import,
    ),

    arm_import: $ => prec.right(seq('use', $._expression, optional(choice(',', ';')))),

    match_arm: $ => prec.right(seq(
      repeat($.attribute),
      optional('catch'),
      field('pattern', $._pattern),
      '=>',
      field('body', choice($.block, $._statement, $._expression)),
      optional(choice(',', ';')),
    )),

    condition_arm: $ => prec.right(seq(
      repeat($.attribute),
      field('condition', $._expression),
      '=>',
      field('body', choice($.block, $._statement, $._expression)),
      optional(choice(',', ';')),
    )),

    else_arm: $ => prec.right(seq(
      repeat($.attribute),
      'else',
      '=>',
      field('body', choice($.block, $._statement, $._expression)),
      optional(choice(',', ';')),
    )),

    // Expressions (full and no-extended-application variants)
    _expression: $ => choice(
      $._primary_expression,
      $.unary_expression,
      $.binary_expression,
      $.range_expression,
      $.postfix_expression,
      $.member_expression,
      $.take_view_expression,
      $.call_expression,
      $.generic_application,
      $.extended_application,
      $.index_expression,
      $.cast_expression,
      $.matches_expression,
      $.match_expression,
      $.do_lambda_expression,
      $.effect_handling_expression,
      $.error_handling_expression,
      $.qualified_type,
      $.function_type,
      $.forall_expression,
      $.exists_expression,
      $.lambda_expression,
      $.contextual_expr_type,
      $.contextual_pattern_type,
    ),

    _expr_no_ext: $ => choice(
      $._primary_expression,
      alias($.unary_expression_no_ext, $.unary_expression),
      alias($.binary_expression_no_ext, $.binary_expression),
      alias($.range_expression_no_ext, $.range_expression),
      alias($.postfix_expression_no_ext, $.postfix_expression),
      alias($.member_expression_no_ext, $.member_expression),
      alias($.take_view_expression_no_ext, $.take_view_expression),
      alias($.call_expression_no_ext, $.call_expression),
      alias($.generic_application_no_ext, $.generic_application),
      alias($.index_expression_no_ext, $.index_expression),
      alias($.cast_expression_no_ext, $.cast_expression),
      alias($.matches_expression_no_ext, $.matches_expression),
      alias($.qualified_type_no_ext, $.qualified_type),
      $.function_type,
      $.contextual_expr_type,
      $.contextual_pattern_type,
    ),

    _primary_expression: $ => choice(
      $.identifier,
      $.self_literal,
      $.wildcard,
      $.integer_literal,
      $.float_literal,
      $.char_literal,
      $.string_literal,
      $.raw_string_literal,
      $.suffixed_literal,
      $.boolean_literal,
      $.null_literal,
      $.undefined_literal,
      $.unit_literal,
      $.range_all,
      $.symbol_literal,
      $.parenthesized_expression,
      $.tuple_expression,
      $.list_expression,
      $.object_expression,
      $.keyword_block_expression,
      $.atomic_block_expression,
      $.quote_expression,
      $.quote_pattern,
      $.splice_expression,
      $.macro_content,
      $.ast_macro_call,
      $.template_macro_call,
      $.token_buffer,
      $.if_expression,
      $.when_expression,
      $.inline_statement,
      $.return_expression,
      $.resume_expression,
      $.break_expression,
      $.continue_expression,
    ),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    tuple_expression: $ => seq(
      '(',
      $._expression,
      ',',
      commaSep($._expression),
      ')',
    ),

    list_expression: $ => seq(
      '[',
      commaSep(choice($.spread_argument, $._expression)),
      ']',
    ),

    object_expression: $ => seq(
      '{',
      commaSep(choice($.property, $.spread_argument, $._expression)),
      '}',
    ),

    property: $ => prec(1, seq(
      optional($._item_prefix),
      field('name', $.identifier),
      ':',
      field('value', $._expression),
    )),

    keyword_block_expression: $ => prec(2, seq(
      field('kind', choice('do', 'async', 'unsafe', 'comptime')),
      field('body', $.block),
    )),

    atomic_block_expression: $ => prec(2, seq(
      'atomic',
      '(',
      commaSep($.identifier),
      ')',
      field('body', $.block),
    )),

    quote_substitution: $ => seq(
      field('left', $.identifier),
      '=',
      field('right', $.identifier),
    ),

    quote_expression: $ => seq(
      '\'',
      optional(seq('[', commaSep($.quote_substitution), ']')),
      '(',
      $._expression,
      ')',
    ),

    splice_expression: $ => seq(
      '~',
      optional(seq('[', commaSep($.quote_substitution), ']')),
      $._tight_lt,
      $._expression,
      $._tight_gt,
    ),

    _lambda_return_type: $ => prec(-2, $._expr_no_ext),

    lambda_expression: $ => prec.right(PREC.ASSIGN, seq(
      choice(
        seq('|', commaSep($.lambda_parameter), '|'),
        '||',
      ),
      optional(seq('->', field('return_type', $._lambda_return_type))),
      field('body', choice($.block, $._expression)),
    )),

    lambda_parameter: $ => seq(
      field('name', choice($.identifier, $.wildcard)),
      optional(seq(':', field('type', $._lambda_type))),
    ),

    _lambda_type: $ => choice(
      $.identifier,
      $.self_literal,
      $.parenthesized_expression,
      $.tuple_expression,
      alias($.lambda_unary_type, $.unary_expression),
      alias($.lambda_member_type, $.member_expression),
      alias($.lambda_generic_type, $.generic_application),
      alias($.qualified_type_no_ext, $.qualified_type),
      $.function_type,
      $.contextual_expr_type,
      $.contextual_pattern_type,
    ),

    lambda_unary_type: $ => prec.right(PREC.PREFIX, seq(
      field('operator', choice('?', '*', 'dyn', 'lift')),
      field('operand', $._lambda_type),
    )),

    lambda_member_type: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._lambda_type),
      '.',
      field('member', $._member_name),
    )),

    lambda_generic_type: $ => prec.left(PREC.POSTFIX, seq(
      field('head', $._lambda_type),
      $._tight_lt,
      commaSep($.argument),
      $._tight_gt,
    )),

    forall_expression: $ => prec.right(PREC.ASSIGN, choice(
      seq('forall', $.type_parameter_list, $._expression),
      seq('forall', commaSep1($.parameter), '=>', $._expression),
    )),

    exists_expression: $ => prec.right(PREC.ASSIGN, seq(
      'exists',
      commaSep1($.parameter),
      '=>',
      $._expression,
    )),

    // Prefix & Type Expressions
    unary_expression: $ => prec.right(PREC.PREFIX, seq(
      field('operator', choice('not', 'error', '-', 'defer', '?', '*', 'dyn', 'lift')),
      field('operand', $._expression),
    )),

    unary_expression_no_ext: $ => prec.right(PREC.PREFIX, seq(
      field('operator', choice('not', 'error', '-', 'defer', '?', '*', 'dyn', 'lift')),
      field('operand', $._expr_no_ext),
    )),

    _qualifier_operand: $ => prec.right(choice(
      $.identifier,
      $.self_literal,
      $.symbol_literal,
      $.parenthesized_expression,
      $.list_expression,
      alias($.qualifier_member_expression, $.member_expression),
      alias($.qualifier_generic_application, $.generic_application),
      alias($.qualifier_call_expression, $.call_expression),
    )),

    qualifier_member_expression: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._qualifier_operand),
      token.immediate('.'),
      field('member', $._member_name),
    )),

    qualifier_generic_application: $ => prec.left(PREC.POSTFIX, seq(
      field('head', $._qualifier_operand),
      $._tight_lt,
      commaSep($.argument),
      $._tight_gt,
    )),

    qualifier_call_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('function', $._qualifier_operand),
      token.immediate('('),
      commaSep($.argument),
      ')',
    )),

    qualified_type: $ => prec.right(PREC.PREFIX, choice(
      seq(field('qualifier_kind', choice('!', '#', '&')), field('qualifier', $._qualifier_operand), field('type', $._expression)),
      seq(
        field('qualifier_kind', '%'),
        field('qualifier', choice($._qualifier_operand, $.closure_environment)),
        field('type', $._expression),
      ),
      seq('refines', $.block, field('type', $._expression)),
      seq('for', $.type_parameter_list, field('type', $._expression)),
    )),

    qualified_type_no_ext: $ => prec.right(PREC.PREFIX, choice(
      seq(field('qualifier_kind', choice('!', '#', '&')), field('qualifier', $._qualifier_operand), field('type', $._expr_no_ext)),
      seq(
        field('qualifier_kind', '%'),
        field('qualifier', choice($._qualifier_operand, $.closure_environment)),
        field('type', $._expr_no_ext),
      ),
      seq('refines', $.block, field('type', $._expr_no_ext)),
      seq('for', $.type_parameter_list, field('type', $._expr_no_ext)),
    )),

    closure_environment: $ => seq(
      '{',
      commaSep($.environment_binding),
      '}',
    ),

    environment_binding: $ => seq(
      field('name', $.identifier),
      ':',
      field('type', $._expr_no_ext),
      optional(seq('=', field('value', $._expr_no_ext))),
    ),

    function_type: $ => prec.right(PREC.PREFIX, seq(
      repeat($.function_modifier),
      'fn',
      choice(
        seq('(', commaSep($.parameter_type), ')'),
        seq($._tight_lt, commaSep($.parameter_type), $._tight_gt),
      ),
      optional(seq('->', field('return_type', $._expr_no_ext))),
    )),

    parameter_type: $ => seq(
      repeat($.parameter_modifier),
      choice(
        seq(optional('*'), $.self_literal),
        prec.dynamic(2, seq('.', field('name', $._member_name), ':', field('type', $._expr_no_ext), optional(seq('=', field('default', $._expr_no_ext))))),
        seq('...', field('name', $.identifier), ':', field('type', $._expr_no_ext)),
        prec.dynamic(2, seq(field('name', $.identifier), ':-', field('bound', $._expr_no_ext))),
        prec.dynamic(2, seq(field('name', $.identifier), ':', field('type', $._expr_no_ext))),
        $._expr_no_ext,
      ),
    ),

    contextual_expr_type: $ => seq(
      choice($._sep_lt, $._tight_lt),
      commaSep($.runtime_binding),
      '|-',
      field('type', $._expression),
      choice($._sep_gt, $._tight_gt),
    ),

    contextual_pattern_type: $ => seq(
      choice($._sep_lt, $._tight_lt),
      field('type', $._expression),
      '-|',
      commaSep($.runtime_binding),
      choice($._sep_gt, $._tight_gt),
    ),

    runtime_binding: $ => seq(
      field('name', $.identifier),
      ':',
      field('type', $._expr_no_ext),
    ),

    range_all: _ => prec(-1, '..'),

    // Range & Binary Expressions
    range_expression: $ => choice(
      prec.right(PREC.RANGE, seq(
        field('start', $._expression),
        field('operator', choice('..', '..=')),
        optional(field('end', $._expression)),
      )),
      prec.right(PREC.RANGE, seq(
        field('operator', choice('..', '..=')),
        field('end', $._expression),
      )),
    ),

    range_expression_no_ext: $ => choice(
      prec.right(PREC.RANGE, seq(
        field('start', $._expr_no_ext),
        field('operator', choice('..', '..=')),
        optional(field('end', $._expr_no_ext)),
      )),
      prec.right(PREC.RANGE, seq(
        field('operator', choice('..', '..=')),
        field('end', $._expr_no_ext),
      )),
    ),

    binary_expression: $ => choice(
      prec.right(PREC.IMPLIES, seq(field('left', $._expression), field('operator', '==>'), field('right', $._expression))),
      prec.left(PREC.OR, seq(field('left', $._expression), field('operator', 'or'), field('right', $._expression))),
      prec.left(PREC.AND, seq(field('left', $._expression), field('operator', 'and'), field('right', $._expression))),
      prec.left(PREC.COMPARE, seq(
        field('left', $._expression),
        field('operator', choice(
          '==', '!=', '<=', '>=', ':', ':-', '<:',
          alias($._sep_lt, '<'),
          alias($._sep_gt, '>'),
        )),
        field('right', $._expression),
      )),
      prec.right(PREC.ARROW, seq(field('left', $._expression), field('operator', '->'), field('right', $._expression))),
      prec.left(PREC.ADD, seq(
        field('left', $._expression),
        field('operator', choice(alias($._sep_plus, '+'), alias($._sep_minus, '-'))),
        field('right', $._expression),
      )),
      prec.left(PREC.MUL, seq(
        field('left', $._expression),
        field('operator', choice(
          alias($._sep_star, '*'),
          alias($._sep_slash, '/'),
          alias($._sep_percent, '%'),
          '++',
        )),
        field('right', $._expression),
      )),
      prec.left(PREC.PIPE, seq(
        field('left', $._expression),
        field('operator', choice('|', '|>')),
        field('right', $._expression),
      )),
    ),

    binary_expression_no_ext: $ => choice(
      prec.right(PREC.IMPLIES, seq(field('left', $._expr_no_ext), field('operator', '==>'), field('right', $._expr_no_ext))),
      prec.left(PREC.OR, seq(field('left', $._expr_no_ext), field('operator', 'or'), field('right', $._expr_no_ext))),
      prec.left(PREC.AND, seq(field('left', $._expr_no_ext), field('operator', 'and'), field('right', $._expr_no_ext))),
      prec.left(PREC.COMPARE, seq(
        field('left', $._expr_no_ext),
        field('operator', choice(
          '==', '!=', '<=', '>=', ':', ':-', '<:',
          alias($._sep_lt, '<'),
          alias($._sep_gt, '>'),
        )),
        field('right', $._expr_no_ext),
      )),
      prec.right(PREC.ARROW, seq(field('left', $._expr_no_ext), field('operator', '->'), field('right', $._expr_no_ext))),
      prec.left(PREC.ADD, seq(
        field('left', $._expr_no_ext),
        field('operator', choice(alias($._sep_plus, '+'), alias($._sep_minus, '-'))),
        field('right', $._expr_no_ext),
      )),
      prec.left(PREC.MUL, seq(
        field('left', $._expr_no_ext),
        field('operator', choice(
          alias($._sep_star, '*'),
          alias($._sep_slash, '/'),
          alias($._sep_percent, '%'),
          '++',
        )),
        field('right', $._expr_no_ext),
      )),
      prec.left(PREC.PIPE, seq(
        field('left', $._expr_no_ext),
        field('operator', choice('|', '|>')),
        field('right', $._expr_no_ext),
      )),
    ),

    // Postfix & Member Expressions
    member_expression: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._expression),
      '.',
      field('member', choice($._member_name, '*', 'ref', 'await')),
    )),

    member_expression_no_ext: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._expr_no_ext),
      '.',
      field('member', choice($._member_name, '*', 'ref', 'await')),
    )),

    take_view_expression: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._expression),
      alias($._take_view_tick, '\''),
      field('view', $.identifier),
    )),

    take_view_expression_no_ext: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._expr_no_ext),
      alias($._take_view_tick, '\''),
      field('view', $.identifier),
    )),

    cast_expression: $ => prec.left(PREC.MEMBER, seq(
      field('value', $._expression),
      '.',
      field('kind', choice('as', 'dyn', 'use')),
      '(',
      field('target', $._expression),
      ')',
    )),

    cast_expression_no_ext: $ => prec.left(PREC.MEMBER, seq(
      field('value', $._expr_no_ext),
      '.',
      field('kind', choice('as', 'dyn', 'use')),
      '(',
      field('target', $._expression),
      ')',
    )),

    call_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('function', $._expression),
      '(',
      commaSep($.argument),
      ')',
    )),

    call_expression_no_ext: $ => prec.left(PREC.POSTFIX, seq(
      field('function', $._expr_no_ext),
      '(',
      commaSep($.argument),
      ')',
    )),

    generic_application: $ => prec.left(PREC.POSTFIX, seq(
      field('head', $._expression),
      $._tight_lt,
      commaSep($.argument),
      $._tight_gt,
    )),

    generic_application_no_ext: $ => prec.left(PREC.POSTFIX, seq(
      field('head', $._expr_no_ext),
      $._tight_lt,
      commaSep($.argument),
      $._tight_gt,
    )),

    extended_application: $ => prec.left(PREC.POSTFIX, seq(
      field('head', $._expression),
      '{',
      commaSep(choice($.property, $.spread_argument, $._expression)),
      '}',
    )),

    index_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('collection', $._expression),
      '[',
      field('index', $._expression),
      ']',
    )),

    index_expression_no_ext: $ => prec.left(PREC.POSTFIX, seq(
      field('collection', $._expr_no_ext),
      '[',
      field('index', $._expression),
      ']',
    )),

    postfix_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('operand', $._expression),
      field('operator', choice('?', '!', '#')),
    )),

    postfix_expression_no_ext: $ => prec.left(PREC.POSTFIX, seq(
      field('operand', $._expr_no_ext),
      field('operator', choice('?', '!', '#')),
    )),

    matches_expression: $ => prec.left(PREC.COMPARE, seq(
      field('value', $._expression),
      'matches',
      field('pattern', $._pattern_no_ext),
    )),

    matches_expression_no_ext: $ => prec.left(PREC.COMPARE, seq(
      field('value', $._expr_no_ext),
      'matches',
      field('pattern', $._pattern_no_ext),
    )),

    match_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('value', $._expression),
      'match',
      '{',
      repeat($._match_entry),
      '}',
    )),

    do_lambda_expression: $ => prec.left(PREC.POSTFIX, seq(
      field('receiver', $._expression),
      'do',
      field('handler', choice($.lambda_expression, $.block, $._expression)),
    )),

    effect_handling_expression: $ => prec.left(PREC.POSTFIX + 1, seq(
      field('value', $._expression),
      '#',
      '{',
      repeat($.match_arm),
      '}',
    )),

    error_handling_expression: $ => prec.left(PREC.POSTFIX + 1, seq(
      field('value', $._expression),
      '!',
      '{',
      repeat($.match_arm),
      '}',
    )),

    argument: $ => seq(
      repeat($.attribute),
      choice(
        $.named_argument,
        $.spread_argument,
        $._expression,
      ),
    ),

    named_argument: $ => prec(2, seq(
      '.',
      field('name', $._member_name),
      '=',
      field('value', $._expression),
    )),

    spread_argument: $ => prec.right(1, seq(
      '...',
      field('value', $._expression),
    )),

    // Patterns
    _pattern: $ => choice(
      $._primary_pattern,
      $.prefix_pattern,
      $.or_pattern,
      $.range_pattern,
      $.postfix_pattern,
      $.application_pattern,
      $.generic_pattern,
      $.extended_pattern,
      $.member_pattern,
      $.guard_pattern,
      $.as_pattern,
      $.handles_pattern,
    ),

    _pattern_no_ext: $ => choice(
      $._primary_pattern,
      alias($.prefix_pattern_no_ext, $.prefix_pattern),
      alias($.or_pattern_no_ext, $.or_pattern),
      alias($.range_pattern_no_ext, $.range_pattern),
      alias($.postfix_pattern_no_ext, $.postfix_pattern),
      alias($.application_pattern_no_ext, $.application_pattern),
      alias($.generic_pattern_no_ext, $.generic_pattern),
      alias($.member_pattern_no_ext, $.member_pattern),
      alias($.as_pattern_no_ext, $.as_pattern),
      alias($.handles_pattern_no_ext, $.handles_pattern),
    ),

    _primary_pattern: $ => choice(
      $.identifier,
      $.wildcard,
      $.unit_literal,
      $.range_all,
      $.null_literal,
      $.boolean_literal,
      $.char_literal,
      $.string_literal,
      $.suffixed_literal,
      $.integer_literal,
      $.float_literal,
      $.negative_literal_pattern,
      $.symbol_literal,
      $.parenthesized_pattern,
      $.tuple_pattern,
      $.list_pattern,
      $.rest_pattern,
      $.struct_pattern,
      $.bit_vec_pattern,
      $.quote_pattern,
      $.splice_expression,
    ),

    negative_literal_pattern: $ => seq(
      '-',
      choice($.integer_literal, $.float_literal, $.suffixed_literal),
    ),

    parenthesized_pattern: $ => seq('(', $._pattern, ')'),

    tuple_pattern: $ => seq(
      '(',
      $._pattern,
      ',',
      commaSep($._pattern),
      ')',
    ),

    list_pattern: $ => seq('[', commaSep($._pattern), ']'),

    rest_pattern: $ => prec.right(seq('...', optional(choice($.identifier, $.wildcard)))),

    struct_pattern: $ => seq(
      '{',
      commaSep(choice($.property_pattern, $._pattern)),
      '}',
    ),

    property_pattern: $ => prec(1, seq(
      optional($._item_prefix),
      field('name', $.identifier),
      ':',
      field('pattern', $._pattern),
    )),

    bit_vec_pattern: $ => prec.left(PREC.POSTFIX + 5, choice(
      seq(choice($.bit_vec_prefix, $.integer_literal), $.bit_vec_segment),
      seq($.bit_vec_pattern, $.bit_vec_segment),
    )),

    bit_vec_segment: $ => choice(
      $.identifier,
      $.integer_literal,
      seq('(', field('pattern', $._pattern), ':', field('type', $._expression), ')'),
      seq(choice($._tight_lt, $._sep_lt), field('expression', $._expression), choice($._tight_gt, $._sep_gt)),
    ),

    quote_pattern: $ => seq(
      '\'',
      optional(seq('[', commaSep($.quote_substitution), ']')),
      $._tight_lt,
      $._pattern,
      $._tight_gt,
    ),

    prefix_pattern: $ => prec.right(PREC.PREFIX, seq(
      field('operator', choice('error', 'not', 'ref', 'async')),
      field('pattern', $._pattern),
    )),

    prefix_pattern_no_ext: $ => prec.right(PREC.PREFIX, seq(
      field('operator', choice('error', 'not', 'ref', 'async')),
      field('pattern', $._pattern_no_ext),
    )),

    or_pattern: $ => prec.left(PREC.OR, seq(
      field('left', $._pattern),
      'or',
      field('right', $._pattern),
    )),

    or_pattern_no_ext: $ => prec.left(PREC.OR, seq(
      field('left', $._pattern_no_ext),
      'or',
      field('right', $._pattern_no_ext),
    )),

    range_pattern: $ => choice(
      prec.left(PREC.RANGE, seq($._pattern, choice('..', '..='), $._pattern)),
      prec.right(PREC.PREFIX, seq(choice('..', '..='), $._pattern)),
    ),

    range_pattern_no_ext: $ => choice(
      prec.left(PREC.RANGE, seq($._pattern_no_ext, choice('..', '..='), $._pattern_no_ext)),
      prec.right(PREC.PREFIX, seq(choice('..', '..='), $._pattern_no_ext)),
    ),

    postfix_pattern: $ => prec.left(PREC.COMPARE, seq(
      field('pattern', $._pattern),
      field('operator', choice('?', '!')),
    )),

    postfix_pattern_no_ext: $ => prec.left(PREC.COMPARE, seq(
      field('pattern', $._pattern_no_ext),
      field('operator', choice('?', '!')),
    )),

    member_pattern: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._pattern),
      '.',
      field('member', $._member_name),
    )),

    member_pattern_no_ext: $ => prec.left(PREC.MEMBER, seq(
      field('object', $._pattern_no_ext),
      '.',
      field('member', $._member_name),
    )),

    application_pattern: $ => prec.left(PREC.POSTFIX, seq(
      field('constructor', $._pattern),
      '(',
      commaSep($._pattern),
      ')',
    )),

    application_pattern_no_ext: $ => prec.left(PREC.POSTFIX, seq(
      field('constructor', $._pattern_no_ext),
      '(',
      commaSep($._pattern),
      ')',
    )),

    generic_pattern: $ => prec.left(PREC.POSTFIX, seq(
      field('constructor', $._pattern),
      $._tight_lt,
      commaSep($.type_pattern_argument),
      $._tight_gt,
    )),

    generic_pattern_no_ext: $ => prec.left(PREC.POSTFIX, seq(
      field('constructor', $._pattern_no_ext),
      $._tight_lt,
      commaSep($.type_pattern_argument),
      $._tight_gt,
    )),

    type_pattern_argument: $ => choice(
      prec(2, seq(field('name', $.identifier), ':', field('pattern', $._pattern))),
      seq('\'', field('name', $.identifier)),
      $._pattern,
    ),

    extended_pattern: $ => prec.left(PREC.POSTFIX, seq(
      field('constructor', $._pattern),
      '{',
      commaSep(choice($.property_pattern, $._pattern)),
      '}',
    )),

    guard_pattern: $ => prec.left(10, choice(
      seq(field('pattern', $._pattern), 'if', field('condition', $._expression)),
      seq(field('pattern', $._pattern), 'and', field('value', $._expr_no_ext), 'is', field('subpattern', $._pattern)),
    )),

    as_pattern: $ => prec.left(PREC.OR - 2, seq(
      field('pattern', $._pattern),
      'as',
      field('name', $.identifier),
    )),

    as_pattern_no_ext: $ => prec.left(PREC.OR - 2, seq(
      field('pattern', $._pattern_no_ext),
      'as',
      field('name', $.identifier),
    )),

    handles_pattern: $ => prec.left(5, seq(
      field('pattern', $._pattern),
      'handles',
      field('effect', $._expr_no_ext),
    )),

    handles_pattern_no_ext: $ => prec.left(5, seq(
      field('pattern', $._pattern_no_ext),
      'handles',
      field('effect', $._expr_no_ext),
    )),
  },
});
