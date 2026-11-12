[
  (block)
  (declaration_body)
  (struct_body)
  (enum_body)
  (object_expression)
  (list_expression)
  (tuple_expression)
  (parameter_list)
  (type_parameter_list)
  (when_expression)
  (match_expression)
  (effect_handling_expression)
  (error_handling_expression)
] @indent.begin

[
  "}"
  "]"
  ")"
] @indent.end @indent.branch

[
  (line_comment)
  (block_comment)
  (string_literal)
  (raw_string_literal)
] @indent.ignore
