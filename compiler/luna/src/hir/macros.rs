/// Macros for simplifying HIR construction and handling circular references
///
/// These macros provide a more ergonomic way to construct HIR nodes, especially
/// when dealing with complex nested structures and circular references that require
/// the use of `hir.update()` calls.

/// Create a HIR mapping entry and get its ID, with optional update later
///
/// Usage:
/// ```
/// let id = hir_put!(hir, Definition(def, owner_id));
/// let id = hir_put!(hir, Unresolved(file_id, node_idx, owner_id));
/// ```
#[macro_export]
macro_rules! hir_put {
    ($hir:expr, $mapping:ident($($args:expr),*)) => {{
        $hir.put($crate::hir::HirMapping::$mapping($($args),*))
    }};
}

/// Create a placeholder HIR ID that will be updated later
/// Useful for handling circular references
///
/// Usage:
/// ```
/// let placeholder_id = hir_placeholder!(hir);
/// // ... construct something that references placeholder_id ...
/// hir_update!(hir, placeholder_id, Definition(actual_def, owner));
/// ```
#[macro_export]
macro_rules! hir_placeholder {
    ($hir:expr) => {{ $hir.put($crate::hir::HirMapping::Invalid) }};
}

/// Update a HIR mapping
///
/// Usage:
/// ```
/// hir_update!(hir, id, Definition(new_def, owner_id));
/// ```
#[macro_export]
macro_rules! hir_update {
    ($hir:expr, $id:expr, $mapping:ident($($args:expr),*)) => {{
        $hir.update($id, $crate::hir::HirMapping::$mapping($($args),*))
    }};
}

// Convenience macros for interning and putting expressions/definitions

/// Create an expression, intern it, and put it in HIR, returning the expression ID
///
/// Usage:
/// ```
/// let expr_id = hir_put_expr!(hir, IntLiteral(42), owner_id);
/// let expr_id = hir_put_expr!(hir, Ref(some_id), owner_id);
/// ```
#[macro_export]
macro_rules! hir_put_expr {
    ($hir:expr, $expr:expr, $owner_id:expr) => {{
        let expr = $expr;
        let interned_expr = $hir.intern_expr(expr);
        $hir.put($crate::hir::HirMapping::Expr(interned_expr, $owner_id))
    }};
}

/// Create a definition, and put it in HIR, returning the definition ID
///
/// Usage:
/// ```
/// let def_id = hir_put_def!(hir, Definition::Module(module), owner_id);
/// let def_id = hir_put_def!(hir, Definition::Struct(struct_def), owner_id);
/// ```
#[macro_export]
macro_rules! hir_put_def {
    ($hir:expr, $def:expr, $owner_id:expr) => {{ $hir.put($crate::hir::HirMapping::Definition($def, $owner_id)) }};
}

/// Create a pattern, intern it, and put it in HIR, returning the pattern ID  
///
/// Usage:
/// ```
/// let pattern_id = hir_put_pattern!(hir, Wildcard, owner);
/// let pattern_id = hir_put_pattern!(hir, Literal(expr_id), owner);
/// ```
#[macro_export]
macro_rules! hir_put_pattern {
    ($hir:expr, $pattern:expr, $owner_id:expr) => {{
        let pattern = $pattern;
        let interned_pattern = $hir.intern_pattern(pattern);
        $hir.put($crate::hir::HirMapping::Pattern(
            interned_pattern,
            $owner_id,
        ))
    }};
}

/// Build a Definition::Module with common defaults
///
/// Usage:
/// ```
/// let module = hir_module!(hir, "my_module", scope_id);
/// let module = hir_module!(hir, "my_module", scope_id, clauses: my_clauses);
/// ```
#[macro_export]
macro_rules! hir_module {
    ($hir:expr, $name:expr, $scope_id:expr) => {{
        $crate::hir::Definition::Module($crate::hir::Module {
            name: $hir.intern_str($name),
            clauses: $hir.empty.clauses,
            scope_id: $scope_id,
        })
    }};
    ($hir:expr, $name:expr, $scope_id:expr, clauses: $clauses:expr) => {{
        $crate::hir::Definition::Module($crate::hir::Module {
            name: $hir.intern_str($name),
            clauses: $clauses,
            scope_id: $scope_id,
        })
    }};
}

/// Build a Definition::Struct with common defaults
///
/// Usage:
/// ```
/// let struct_def = hir_struct!(hir, "MyStruct", scope_id); // Empty fields
/// let struct_def = hir_struct!(hir, "MyStruct", fields, scope_id);
/// let struct_def = hir_struct!(hir, "MyStruct", fields, scope_id, clauses: my_clauses);
/// ```
#[macro_export]
macro_rules! hir_struct {
    ($hir:expr, $name:expr, $scope_id:expr) => {{
        $crate::hir::Definition::Struct($crate::hir::Struct {
            name: $hir.intern_str($name),
            fields: $hir.empty.definitions,
            clauses: $hir.empty.clauses,
            scope_id: $scope_id,
        })
    }};
    ($hir:expr, $name:expr, $fields:expr, $scope_id:expr) => {{
        $crate::hir::Definition::Struct($crate::hir::Struct {
            name: $hir.intern_str($name),
            fields: $fields,
            clauses: $hir.empty.clauses,
            scope_id: $scope_id,
        })
    }};
    ($hir:expr, $name:expr, $fields:expr, $scope_id:expr, clauses: $clauses:expr) => {{
        $crate::hir::Definition::Struct($crate::hir::Struct {
            name: $hir.intern_str($name),
            fields: $fields,
            clauses: $clauses,
            scope_id: $scope_id,
        })
    }};
}

/// Build a Definition::Package
///
/// Usage:
/// ```
/// let package = hir_package!(hir, "my_package", items, scope_id);
/// ```
#[macro_export]
macro_rules! hir_package {
    ($hir:expr, $name:expr, $scope_id:expr) => {{
        $crate::hir::Definition::Package {
            name: $hir.intern_str($name),
            scope_id: $scope_id,
        }
    }};
}

/// Build a Definition::FileScope
///
/// Usage:
/// ```
/// let file_scope = hir_file_scope!(hir, "file_name", items, scope_id);
/// ```
#[macro_export]
macro_rules! hir_file_scope {
    ($hir:expr, $name:expr, $items:expr, $scope_id:expr) => {{
        $crate::hir::Definition::FileScope {
            name: $hir.intern_str($name),
            items: $items,
            scope_id: $scope_id,
        }
    }};
}

/// Build common expression types
///
/// Usage:
/// ```
/// let int_lit = hir_expr!(hir, IntLiteral(42));
/// let var_ref = hir_expr!(hir, Variable(symbol));
/// let binary = hir_expr!(hir, BinaryApply { left, right, op: BinaryOp::Add });
/// ```
#[macro_export]
macro_rules! hir_expr {
    ($hir:expr, IntLiteral($value:expr)) => {{ $hir.intern_expr($crate::hir::Expr::IntLiteral($value)) }};
    ($hir:expr, Variable($symbol:expr)) => {{ $hir.intern_expr($crate::hir::Expr::Variable($symbol)) }};
    ($hir:expr, BinaryApply { left: $left:expr, right: $right:expr, op: $op:expr }) => {{
        $hir.intern_expr($crate::hir::Expr::BinaryApply {
            left: $left,
            right: $right,
            op: $op,
        })
    }};
    ($hir:expr, $expr:expr) => {{ $hir.intern_expr($expr) }};
}

/// Construct and intern collections with convenience
///
/// Usage:
/// ```
/// let exprs = hir_exprs!(hir, [expr1, expr2, expr3]);
/// let exprs_empty = hir_exprs!(hir); // Uses hir.empty.exprs
/// let defs = hir_definitions!(hir, [def1, def2]);
/// let defs_empty = hir_definitions!(hir); // Uses hir.empty.definitions
/// let params = hir_params!(hir, [param1, param2]);
/// let params_empty = hir_params!(hir); // Uses hir.empty.params for functions without parameters
/// ```
#[macro_export]
macro_rules! hir_exprs {
    ($hir:expr) => {{
        $hir.empty.exprs
    }};
    ($hir:expr, [$($expr:expr),* $(,)?]) => {{
        $hir.intern_exprs(vec![$($expr),*])
    }};
}

#[macro_export]
macro_rules! hir_definitions {
    ($hir:expr) => {{
        $hir.empty.definitions
    }};
    ($hir:expr, [$($def:expr),* $(,)?]) => {{
        $hir.intern_definitions(vec![$($def),*])
    }};
}

#[macro_export]
macro_rules! hir_params {
    ($hir:expr) => {{
        $hir.empty.params
    }};
    ($hir:expr, [$($param:expr),* $(,)?]) => {{
        $hir.intern_params(vec![$($param),*])
    }};
}

#[macro_export]
macro_rules! hir_clauses {
    ($hir:expr) => {{
        $hir.empty.clauses
    }};
    ($hir:expr, [$($clause:expr),* $(,)?]) => {{
        $hir.intern_clauses(vec![$($clause),*])
    }};
}

#[macro_export]
macro_rules! hir_patterns {
    ($hir:expr) => {{
        $hir.empty.patterns
    }};
    ($hir:expr, [$($pattern:expr),* $(,)?]) => {{
        $hir.intern_patterns(vec![$($pattern),*])
    }};
}

/// Construct a struct field definition
///
/// Usage:
/// ```
/// let field = hir_struct_field!(hir, "name", field_type);
/// let field_with_default = hir_struct_field!(hir, "name", field_type, Some(default_expr));
/// ```
#[macro_export]
macro_rules! hir_struct_field {
    ($hir:expr, $name:expr, $ty:expr) => {{ $crate::hir::Definition::StructField($hir.intern_str($name), $hir.intern_expr($ty), None) }};
    ($hir:expr, $name:expr, $ty:expr, $default:expr) => {{
        $crate::hir::Definition::StructField(
            $hir.intern_str($name),
            $hir.intern_expr($ty),
            $default.map(|e| $hir.intern_expr(e)),
        )
    }};
}

/// Common expression construction macros
///
/// Usage:
/// ```
/// let int_expr = hir_int!(hir, 42);
/// let bool_expr = hir_bool!(hir, true);
/// let str_expr = hir_str!(hir, "hello");
/// let list_expr = hir_list!(hir, [expr1, expr2]);
/// let list_empty = hir_list!(hir); // Empty list
/// ```

#[macro_export]
macro_rules! hir_int {
    ($hir:expr, $value:expr) => {{ $crate::hir::Expr::IntLiteral($value) }};
}

#[macro_export]
macro_rules! hir_bool {
    ($hir:expr, $value:expr) => {{ $crate::hir::Expr::BoolLiteral($value) }};
}

#[macro_export]
macro_rules! hir_str {
    ($hir:expr, $value:expr) => {{ $crate::hir::Expr::StrLiteral($hir.intern_str($value)) }};
}

#[macro_export]
macro_rules! hir_char {
    ($hir:expr, $value:expr) => {{ $crate::hir::Expr::CharLiteral($value) }};
}

#[macro_export]
macro_rules! hir_symbol {
    ($hir:expr, $value:expr) => {{ $crate::hir::Expr::SymbolLiteral($hir.intern_str($value)) }};
}

#[macro_export]
macro_rules! hir_unit {
    ($hir:expr) => {{ $crate::hir::Expr::Unit }};
}

#[macro_export]
macro_rules! hir_ref {
    ($hir:expr, $id:expr) => {{ $crate::hir::Expr::Ref($id) }};
}

#[macro_export]
macro_rules! hir_null {
    ($hir:expr) => {{ $crate::hir::Expr::Null }};
}

#[macro_export]
macro_rules! hir_list {
    ($hir:expr) => {{
        $crate::hir::Expr::List($hir.empty.exprs)
    }};
    ($hir:expr, [$($expr:expr),* $(,)?]) => {{
        $crate::hir::Expr::List($hir.intern_exprs(vec![$($expr),*]))
    }};
}

#[macro_export]
macro_rules! hir_tuple {
    ($hir:expr) => {{
        $crate::hir::Expr::Tuple($hir.empty.exprs)
    }};
    ($hir:expr, [$($expr:expr),* $(,)?]) => {{
        $crate::hir::Expr::Tuple($hir.intern_exprs(vec![$($expr),*]))
    }};
}

#[macro_export]
macro_rules! hir_block {
    ($hir:expr) => {{
        $crate::hir::Expr::Block($hir.empty.exprs)
    }};
    ($hir:expr, [$($expr:expr),* $(,)?]) => {{
        $crate::hir::Expr::Block($hir.intern_exprs(vec![$($expr),*]))
    }};
}

/// Operation expression (binary or unary based on argument count)
///
/// Usage:
/// ```
/// let add_expr = hir_op!(hir, Add, left_expr, right_expr);  // Binary
/// let neg_expr = hir_op!(hir, Neg, expr);                   // Unary
/// let mul_expr = hir_op!(hir, Mul, a, b);                   // Binary
/// let not_expr = hir_op!(hir, Not, bool_expr);              // Unary
/// ```
#[macro_export]
macro_rules! hir_op {
    // Unary operation: op + one expression
    ($hir:expr, $op:ident, $expr:expr) => {{
        $crate::hir::Expr::UnaryApply {
            expr: $hir.intern_expr($expr),
            op: $crate::hir::UnaryOp::$op,
        }
    }};

    // Binary operation: op + two expressions
    ($hir:expr, $op:ident, $left:expr, $right:expr) => {{
        $crate::hir::Expr::BinaryApply {
            left: $hir.intern_expr($left),
            right: $hir.intern_expr($right),
            op: $crate::hir::BinaryOp::$op,
        }
    }};
}

// Backward compatibility aliases (optional)
/// Backward compatibility alias for binary operations
#[macro_export]
macro_rules! hir_binary {
    ($hir:expr, $left:expr, $op:ident, $right:expr) => {{ hir_op!($hir, $op, $left, $right) }};
}

/// Backward compatibility alias for unary operations  
#[macro_export]
macro_rules! hir_unary {
    ($hir:expr, $op:ident, $expr:expr) => {{ hir_op!($hir, $op, $expr) }};
}

/// Function application expression
///
/// Usage:
/// ```
/// let call_expr = hir_call!(hir, callee_expr, [arg1, arg2]);
/// let call_no_args = hir_call!(hir, callee_expr);
/// ```
#[macro_export]
macro_rules! hir_call {
    ($hir:expr, $callee:expr) => {{
        $crate::hir::Expr::FnApply {
            callee: $hir.intern_expr($callee),
            args: $hir.empty.exprs,
            optional_args: $hir.empty.properties,
        }
    }};
    ($hir:expr, $callee:expr, [$($arg:expr),* $(,)?]) => {{
        $crate::hir::Expr::FnApply {
            callee: $hir.intern_expr($callee),
            args: $hir.intern_exprs(vec![$($arg),*]),
            optional_args: $hir.empty.properties,
        }
    }};
}

/// If expression
///
/// Usage:
/// ```
/// let if_expr = hir_if!(hir, condition, then_branch);
/// let if_else_expr = hir_if!(hir, condition, then_branch, else_branch);
/// ```
#[macro_export]
macro_rules! hir_if {
    ($hir:expr, $condition:expr, $then_branch:expr) => {{
        $crate::hir::Expr::If {
            condition: $hir.intern_expr($condition),
            then_branch: $hir.intern_expr($then_branch),
            else_branch: None,
        }
    }};
    ($hir:expr, $condition:expr, $then_branch:expr, $else_branch:expr) => {{
        $crate::hir::Expr::If {
            condition: $hir.intern_expr($condition),
            then_branch: $hir.intern_expr($then_branch),
            else_branch: Some($hir.intern_expr($else_branch)),
        }
    }};
}

/// Common pattern construction macros
///
/// Usage:
/// ```
/// let wildcard = hir_wildcard!(hir);
/// let int_pattern = hir_pattern_literal!(hir, int_expr);
/// let var_pattern = hir_pattern_var!(hir, nested_pattern);
/// ```

#[macro_export]
macro_rules! hir_wildcard {
    ($hir:expr) => {{ $crate::hir::Pattern::Wildcard }};
}

#[macro_export]
macro_rules! hir_pattern_literal {
    ($hir:expr, $expr:expr) => {{ $crate::hir::Pattern::Literal($hir.intern_expr($expr)) }};
}

#[macro_export]
macro_rules! hir_pattern_var {
    ($hir:expr, $pattern:expr) => {{ $crate::hir::Pattern::Variable($hir.intern_pattern($pattern)) }};
}

/// Common type expression construction macros
///
/// Usage:
/// ```
/// let int_type = hir_ty_int!(hir);
/// let str_type = hir_ty_str!(hir);
/// let optional_int = hir_ty_optional!(hir, int_type);
/// let tuple_type = hir_ty_tuple!(hir, [type1, type2]);
/// ```

#[macro_export]
macro_rules! hir_ty_int {
    ($hir:expr) => {{ $crate::hir::Expr::TyInteger }};
}

#[macro_export]
macro_rules! hir_ty_str {
    ($hir:expr) => {{ $crate::hir::Expr::TyStr }};
}

#[macro_export]
macro_rules! hir_ty_bool {
    ($hir:expr) => {{ $crate::hir::Expr::TyBool }};
}

#[macro_export]
macro_rules! hir_ty_unit {
    ($hir:expr) => {{ $crate::hir::Expr::TyVoid }};
}

#[macro_export]
macro_rules! hir_ty_optional {
    ($hir:expr, $inner_type:expr) => {{ $crate::hir::Expr::TyOptional($hir.intern_expr($inner_type)) }};
}

#[macro_export]
macro_rules! hir_ty_tuple {
    ($hir:expr) => {{
        $crate::hir::Expr::TyTuple($hir.empty.exprs)
    }};
    ($hir:expr, [$($type:expr),* $(,)?]) => {{
        $crate::hir::Expr::TyTuple($hir.intern_exprs(vec![$($type),*]))
    }};
}

/// Parameter construction macros
///
/// Usage:
/// ```
/// let self_param = hir_param_self!(hir, false); // not ref
/// let self_ref = hir_param_self!(hir, true); // ref self
/// let typed_param = hir_param_typed!(hir, "x", int_type);
/// let typed_with_default = hir_param_typed!(hir, "x", int_type, default_expr);
/// ```

#[macro_export]
macro_rules! hir_param_self {
    ($hir:expr, $is_ref:expr) => {{ $crate::hir::Param::Itself { is_ref: $is_ref } }};
}

#[macro_export]
macro_rules! hir_param_typed {
    ($hir:expr, $name:expr, $type:expr) => {{ $crate::hir::Param::Typed($hir.intern_str($name), $hir.intern_expr($type), None) }};
    ($hir:expr, $name:expr, $type:expr, $default:expr) => {{
        $crate::hir::Param::Typed(
            $hir.intern_str($name),
            $hir.intern_expr($type),
            Some($hir.intern_expr($default)),
        )
    }};
}

#[macro_export]
macro_rules! hir_param_tuple_collect {
    ($hir:expr, $name:expr, $type:expr) => {{ $crate::hir::Param::AutoCollectToTuple($hir.intern_str($name), $hir.intern_expr($type)) }};
}

#[macro_export]
macro_rules! hir_param_object_collect {
    ($hir:expr, $name:expr, $type:expr) => {{ $crate::hir::Param::AutoCollectToObject($hir.intern_str($name), $hir.intern_expr($type)) }};
}

/// Property construction macro
///
/// Usage:
/// ```
/// let prop = hir_property!(hir, "name", value_expr);
/// ```
#[macro_export]
macro_rules! hir_property {
    ($hir:expr, $name:expr, $value:expr) => {{
        $crate::hir::Property {
            name: $hir.intern_str($name),
            value: $hir.intern_expr($value),
        }
    }};
}

/// Properties collection macro (with empty support)
///
/// Usage:
/// ```
/// let props = hir_properties!(hir, [prop1, prop2]);
/// let empty_props = hir_properties!(hir);
/// ```
#[macro_export]
macro_rules! hir_properties {
    ($hir:expr) => {{
        $hir.empty.properties
    }};
    ($hir:expr, [$($prop:expr),* $(,)?]) => {{
        $hir.intern_properties(vec![$($prop),*])
    }};
}

/// Control flow expression macros
///
/// Usage:
/// ```
/// let return_expr = hir_return!(hir, value_expr);
/// let return_void = hir_return!(hir);
/// let break_expr = hir_break!(hir, "label");
/// let break_unlabeled = hir_break!(hir);
/// ```

#[macro_export]
macro_rules! hir_return {
    ($hir:expr) => {{ $crate::hir::Expr::Return(None) }};
    ($hir:expr, $value:expr) => {{ $crate::hir::Expr::Return(Some($hir.intern_expr($value))) }};
}

#[macro_export]
macro_rules! hir_break {
    ($hir:expr) => {{ $crate::hir::Expr::Break(None) }};
    ($hir:expr, $label:expr) => {{ $crate::hir::Expr::Break(Some($hir.intern_str($label))) }};
}

#[macro_export]
macro_rules! hir_continue {
    ($hir:expr) => {{ $crate::hir::Expr::Continue(None) }};
    ($hir:expr, $label:expr) => {{ $crate::hir::Expr::Continue(Some($hir.intern_str($label))) }};
}

/// Let binding expression
///
/// Usage:
/// ```
/// let let_expr = hir_let!(hir, pattern, value, body);
/// ```
#[macro_export]
macro_rules! hir_let {
    ($hir:expr, $pattern:expr, $value:expr, $body:expr) => {{
        $crate::hir::Expr::Let {
            pattern: $hir.intern_pattern($pattern),
            value: $hir.intern_expr($value),
            body: $hir.intern_expr($body),
        }
    }};
}

/// Const binding expression
///
/// Usage:
/// ```
/// let const_expr = hir_const!(hir, pattern, value, body);
/// ```
#[macro_export]
macro_rules! hir_const {
    ($hir:expr, $pattern:expr, $value:expr, $body:expr) => {{
        $crate::hir::Expr::Const {
            pattern: $hir.intern_pattern($pattern),
            value: $hir.intern_expr($value),
            body: $hir.intern_expr($body),
        }
    }};
}

/// Chain multiple HIR construction operations with automatic error handling
///
/// Usage:
/// ```
/// let result = hir_chain! {
///     hir,
///     let id1 = put Definition(def1, owner);
///     let id2 = put Definition(def2, owner);
///     update id1, Definition(updated_def1, owner);
///     return id2;
/// };
/// ```
#[macro_export]
macro_rules! hir_chain {
    ($hir:expr, $($stmt:tt)*) => {{
        (|| -> Result<_, $crate::ast_lower::LowerError> {
            hir_chain_impl!($hir, $($stmt)*);
        })()
    }};
}

/// Internal implementation macro for hir_chain
#[macro_export]
macro_rules! hir_chain_impl {
    // Base case
    ($hir:expr,) => {};

    // let id = put Mapping(...);
    ($hir:expr, let $id:ident = put $mapping:ident($($args:expr),*); $($rest:tt)*) => {
        let $id = hir_put!($hir, $mapping($($args),*));
        hir_chain_impl!($hir, $($rest)*);
    };

    // update id, Mapping(...);
    ($hir:expr, update $id:expr, $mapping:ident($($args:expr),*); $($rest:tt)*) => {
        hir_update!($hir, $id, $mapping($($args),*));
        hir_chain_impl!($hir, $($rest)*);
    };

    // return expr;
    ($hir:expr, return $expr:expr;) => {
        return Ok($expr);
    };

    // Any other statement
    ($hir:expr, $stmt:stmt; $($rest:tt)*) => {
        $stmt;
        hir_chain_impl!($hir, $($rest)*);
    };
}

/// Create a scope and HIR mapping in one go, handling the common pattern
///
/// Usage:
/// ```
/// let (scope_id, hir_id) = hir_scope_and_mapping!(
///     hir, ctx,
///     name: "module_name",
///     parent: Some(parent_scope),
///     is_transparent: false,
///     mapping: Definition(module_def, owner_id)
/// );
/// ```
#[macro_export]
macro_rules! hir_scope_and_mapping {
    (
        $hir:expr, $ctx:expr,
        name: $name:expr,
        parent: $parent:expr,
        is_transparent: $transparent:expr,
        mapping: $mapping:ident($($args:expr),*)
    ) => {{
        let hir_id = hir_put!($hir, $mapping($($args),*));
        let scope_id = $ctx.scope_manager
            .add_scope(
                Some($hir.intern_str($name)),
                $parent,
                $transparent,
                hir_id,
            )
            .map_err(|e| $crate::ast_lower::LowerError::ScopeError(format!("{:?}", e)))?;
        (scope_id, hir_id)
    }};
}

// /// Convenience macro for common builtin module setup pattern
// ///
// /// Usage:
// /// ```
// /// hir_setup_builtin_modules!(hir, ctx, builtin_scope, ["std", "math", "meta"]);
// /// ```
// #[macro_export]
// macro_rules! hir_setup_builtin_modules {
//     ($hir:expr, $ctx:expr, $parent_scope:expr, [$($module_name:expr),*]) => {
//         {
//             use std::collections::HashMap;
//             let mut module_ids = HashMap::new();
//             $(
//                 let name = $hir.intern_str($module_name);
//                 let placeholder_id = $crate::hir_placeholder!($hir);
//                 let scope_id = match $ctx.scope_manager
//                     .add_scope(Some(name), Some($parent_scope), false, placeholder_id) {
//                         Ok(id) => id,
//                         Err(e) => panic!("Failed to create builtin module scope: {:?}", e),
//                     };

//                 let module_def = $crate::hir_module!($hir, $module_name, scope_id);
//                 let final_id = $crate::hir_put!($hir, Definition($hir.intern_definition(module_def), placeholder_id));
//                 $crate::hir_update!($hir, placeholder_id, Intrinsic(final_id));

//                 module_ids.insert($module_name, final_id);
//             )*
//             Ok::<_, $crate::ast_lower::LowerError>(module_ids)
//         }
//     };
// }
