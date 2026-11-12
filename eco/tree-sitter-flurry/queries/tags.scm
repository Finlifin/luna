(function_definition
  name: (identifier) @name) @definition.function

(impl_definition
  body: (declaration_body
    (function_definition
      name: (identifier) @name) @definition.method))

(trait_definition
  name: (identifier) @name) @definition.interface

(struct_definition
  name: (identifier) @name) @definition.class

(enum_definition
  name: (identifier) @name) @definition.class

(union_definition
  name: (identifier) @name) @definition.class

(newtype_definition
  name: (identifier) @name) @definition.type

(typealias_definition
  name: (identifier) @name) @definition.type

(mod_definition
  name: (identifier) @name) @definition.module

(effect_definition
  name: (identifier) @name) @definition.function

(effect_enum_definition
  name: (identifier) @name) @definition.interface

(const_definition
  pattern: (identifier) @name) @definition.constant

(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    member: (identifier) @name)) @reference.call
