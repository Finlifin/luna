; Scopes
[
  (source_file)
  (mod_definition)
  (function_definition)
  (case_definition)
  (predicate_definition)
  (deriver_definition)
  (trait_definition)
  (impl_definition)
  (struct_definition)
  (enum_definition)
  (union_definition)
  (test_definition)
  (block)
  (for_statement)
  (while_statement)
  (if_expression)
  (match_arm)
  (lambda_expression)
  (forall_expression)
  (exists_expression)
] @local.scope

; Definitions
(function_definition
  name: (identifier) @local.definition.function)

(parameter
  name: (identifier) @local.definition.parameter)

(lambda_parameter
  name: (identifier) @local.definition.parameter)

(decl_clause
  name: (identifier) @local.definition.type)

(let_statement
  pattern: (identifier) @local.definition.var)

(var_statement
  pattern: (identifier) @local.definition.var)

(const_definition
  pattern: (identifier) @local.definition.constant)

(as_pattern
  name: (identifier) @local.definition.var)

(use_as_clause
  alias: (identifier) @local.definition.import)

; References
(identifier) @local.reference
