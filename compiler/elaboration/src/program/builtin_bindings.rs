//! Named bindings for compiler operations retain their checked call lowering.
use super::*;

fn operation(name: &str) -> bool {
    matches!(
        name,
        "list_length"
            | "list_concat"
            | "comptime_integer_convert"
            | "comptime_integer_div_trunc"
            | "comptime_integer_rem_trunc"
            | "integer_representation"
            | "str_byte_length"
            | "str_byte"
            | "symbol_byte_length"
            | "symbol_byte"
            | "object_length"
            | "object_key"
            | "object_entry_type"
            | "object_value"
            | "object_get"
            | "object_concat"
            | "object_append"
            | "object_replace"
            | "object_remove"
            | "struct_field_types"
            | "struct_field_names"
            | "enum_group_paths"
            | "enum_variant_paths"
            | "enum_variant_shapes"
            | "enum_variant_field_names"
            | "enum_variant_field_types"
            | "build_struct"
            | "build_enum"
            | "build_enum_paths"
            | "build_symbol_enum_paths"
            | "build_bits_enum_paths"
    )
}

impl Checker<'_, '_> {
    pub(super) fn compiler_operation_binding(
        &mut self,
        mut owner: OwnerId,
        span: Span,
    ) -> Result<Option<hir::Symbol>, Error> {
        let mut visited = BTreeSet::new();
        loop {
            let Some(item) = self.package.item(owner) else {
                return Ok(None);
            };
            let ItemKind::Const(annotation, initializer) = item.kind else {
                return Ok(None);
            };
            if !matches!(annotation.kind, H::TyPlaceholder) {
                return Ok(None);
            }
            if !visited.insert(owner) || visited.len() > 64 {
                return Err(error(
                    span,
                    "cyclic or excessively deep constant dependency",
                ));
            }
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "compiler operation binding budget exhausted"))?;
            match &initializer.kind {
                H::Builtin(name) if operation(name.as_str()) => return Ok(Some(*name)),
                H::ResolvedDef { def_id, .. } => {
                    let Some(next) = self.package.definition_owner(*def_id) else {
                        return Ok(None);
                    };
                    owner = next;
                }
                _ => return Ok(None),
            }
        }
    }
}
