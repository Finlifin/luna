; Comments
(line_comment) @comment.line
(block_comment) @comment.block

; Directives (--% formatter directives)
((line_comment) @keyword.directive
  (#match? @keyword.directive "^--%"))

; Literals
(integer_literal) @number
(float_literal) @number.float
(boolean_literal) @boolean
(char_literal) @character
(string_literal) @string
(raw_string_literal) @string.special
(escape_sequence) @string.escape
(null_literal) @constant.builtin
(undefined_literal) @constant.builtin
(unit_literal) @constant.builtin
(self_literal) @variable.builtin
(wildcard) @variable.builtin
(symbol_literal
  "." @punctuation.special
  name: (identifier) @string.special.symbol)

(suffixed_literal
  suffix: (identifier) @type.builtin)

(string_interpolation
  "${" @punctuation.special
  "}" @punctuation.special) @embedded

; Attributes
(attribute
  "^" @attribute
  value: (_) @attribute)

; Keywords & Modifiers
(visibility) @keyword.modifier
(function_modifier) @keyword.modifier
(parameter_modifier) @keyword.modifier

[
  "fn"
  "struct"
  "enum"
  "union"
  "trait"
  "impl"
  "mod"
  "typealias"
  "newtype"
  "const"
  "assoc"
  "effect"
  "case"
  "derive"
  "predicate"
  "test"
  "define"
] @keyword

[
  "use"
] @keyword.import

[
  "let"
  "var"
] @keyword.storage

[
  "if"
  "else"
  "when"
  "match"
  "matches"
  "is"
  "do"
] @keyword.conditional

[
  "for"
  "while"
  "in"
  "break"
  "continue"
] @keyword.repeat

[
  "return"
  "resume"
  "defer"
] @keyword.return

[
  "where"
  "requires"
  "ensures"
  "decreases"
  "outcomes"
  "asserts"
  "assumes"
  "invariant"
  "axiom"
  "forall"
  "exists"
  "refines"
  "lift"
  "dyn"
  "handles"
  "catch"
  "error"
  "as"
] @keyword

; Operators
[
  "and"
  "or"
  "not"
] @keyword.operator

[
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "=="
  "!="
  "<="
  ">="
  "<"
  ">"
  "+"
  "-"
  "*"
  "/"
  "%"
  "++"
  "==>"
  "->"
  "=>"
  ":-"
  "<:"
  "|-"
  "-|"
  "|"
  "|>"
  ".."
  "..="
  "..."
  "?"
  "!"
  "#"
  "&"
  "~"
  "'"
] @operator

; Punctuation
[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "{{"
  "}}"
] @punctuation.bracket

[
  ","
  ";"
  ":"
  "."
] @punctuation.delimiter

; Declarations
(function_definition
  name: (identifier) @function)

(effect_definition
  name: (identifier) @function)

(effect_variant
  name: (identifier) @function)

(case_definition
  name: (identifier) @function)

(predicate_definition
  name: (identifier) @function)

(deriver_definition
  name: (identifier) @function.macro)

(template_macro_definition
  name: (identifier) @function.macro)

(struct_definition
  name: (identifier) @type.definition)

(enum_definition
  name: (identifier) @type.definition)

(union_definition
  name: (identifier) @type.definition)

(trait_definition
  name: (identifier) @type.definition)

(newtype_definition
  name: (identifier) @type.definition)

(typealias_definition
  name: (identifier) @type.definition)

(assoc_declaration
  name: (identifier) @type.definition)

(effect_enum_definition
  name: (identifier) @type.definition)

(mod_definition
  name: (identifier) @module)

(test_definition
  name: (identifier) @function)

(const_definition
  pattern: (identifier) @constant)

; Fields, Variants, Properties & Parameters
(struct_field
  name: (identifier) @variable.member)

(enum_variant
  name: (identifier) @constructor)

(property
  name: (identifier) @variable.member)

(property_pattern
  name: (identifier) @variable.member)

(named_argument
  name: (identifier) @variable.parameter)

(parameter
  name: (identifier) @variable.parameter)

(lambda_parameter
  name: (identifier) @variable.parameter)

(decl_clause
  name: (identifier) @type.parameter)

(loop_label
  name: (identifier) @label)

; Calls & Member Access
(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (member_expression
    member: (identifier) @function.method.call))

(member_expression
  member: (identifier) @variable.member)

(member_expression
  member: [
    "*"
    "ref"
    "await"
  ] @keyword.operator)

(cast_expression
  kind: [
    "as"
    "dyn"
    "use"
  ] @keyword.operator)

(take_view_expression
  view: (identifier) @attribute)

(ast_macro_call
  "@" @function.macro
  macro: (macro_path) @function.macro)

(template_macro_call
  "$" @function.macro
  name: (identifier) @function.macro)

; Type naming convention (PascalCase identifiers & primitive types)
((identifier) @type.builtin
  (#match? @type.builtin "^(bool|char|str|void|NoReturn|Type|Any|any|Symbol|BitVec|Tuple|u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize|f16|bf16|f32|f64)$"))

((identifier) @type
  (#match? @type "^[A-Z][a-zA-Z0-9_]*$"))
