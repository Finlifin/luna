//! Fixed compiler-operation contracts. Dependent operations require their own
//! ordinary callable telescope; a monomorphic annotation cannot specialize them.
use super::*;

pub(super) fn fixed(
    name: &str,
    list_head: Option<DeclarationId>,
) -> Result<Option<Term>, &'static str> {
    let list = |ty| {
        Ok::<_, &'static str>(Term::NormalForm {
            head: list_head.ok_or("compiler operation signature requires a bound meta.List")?,
            sort: Sort::Type,
            arguments: vec![ty],
        })
    };
    let (inputs, output) = match name {
        "comptime_integer_div_trunc" | "comptime_integer_rem_trunc" => {
            (vec![p(P::Integer), p(P::Integer)], p(P::Integer))
        }
        "object_length" => (vec![p(P::Object)], p(P::Integer)),
        "object_key" => (
            vec![p(P::Object), p(P::Integer)],
            Term::Optional(Box::new(p(P::Symbol))),
        ),
        "object_entry_type" => (vec![p(P::Object), p(P::Integer)], p(P::Type)),
        "object_concat" => (vec![p(P::Object), p(P::Object)], p(P::Object)),
        "object_remove" => (vec![p(P::Object), p(P::Integer)], p(P::Object)),
        "object_slice" => (
            vec![p(P::Object), p(P::Integer), p(P::Integer)],
            p(P::Object),
        ),
        "build_newtype" => (vec![p(P::Symbol), p(P::Type)], p(P::Type)),
        "build_struct" => (
            vec![p(P::Symbol), list(p(P::Symbol))?, list(p(P::Type))?],
            p(P::Type),
        ),
        "build_enum" | "build_enum_paths" | "build_symbol_enum_paths" | "build_bits_enum_paths" => {
            let paths = if name == "build_enum" {
                list(p(P::Symbol))?
            } else {
                list(list(p(P::Symbol))?)?
            };
            let mut inputs = vec![
                p(P::Symbol),
                paths.clone(),
                paths,
                list(p(P::Symbol))?,
                list(list(p(P::Symbol))?)?,
                list(list(p(P::Type))?)?,
            ];
            if name == "build_symbol_enum_paths" {
                inputs.push(list(p(P::Symbol))?);
            } else if name == "build_bits_enum_paths" {
                inputs.extend([
                    p(P::Integer),
                    Term::Optional(Box::new(p(P::Type))),
                    list(p(P::Integer))?,
                    list(p(P::Integer))?,
                    list(list(p(P::Integer))?)?,
                ]);
            }
            (inputs, p(P::Type))
        }
        "newtype_representation" => (vec![p(P::Type)], p(P::Type)),
        "struct_field_types" => (vec![p(P::Type)], list(p(P::Type))?),
        "struct_field_names" | "enum_variant_shapes" | "enum_symbol_values" => {
            (vec![p(P::Type)], list(p(P::Symbol))?)
        }
        "enum_group_paths" | "enum_variant_paths" | "enum_variant_field_names" => {
            (vec![p(P::Type)], list(list(p(P::Symbol))?)?)
        }
        "enum_variant_field_types" => (vec![p(P::Type)], list(list(p(P::Type))?)?),
        "enum_bit_width" => (vec![p(P::Type)], p(P::Integer)),
        "enum_bit_base_type" => (vec![p(P::Type)], Term::Optional(Box::new(p(P::Type)))),
        "enum_bit_masks" | "enum_bit_values" => (vec![p(P::Type)], list(p(P::Integer))?),
        "enum_bit_offsets" => (vec![p(P::Type)], list(list(p(P::Integer))?)?),
        _ => return Ok(None),
    };
    Ok(Some(Term::ComptimeFunction(Box::new(Term::Function {
        parameters: inputs
            .into_iter()
            .map(|ty| Parameter {
                sort: Sort::Runtime,
                ty,
            })
            .collect(),
        result: Box::new(output),
    }))))
}

// Definition publication allocates a new nominal identity on every invocation.
// A Type result alone does not make these operations pure or memoizable.
pub(super) fn publishes_definition(name: &str) -> bool {
    matches!(
        name,
        "build_newtype"
            | "build_struct"
            | "build_enum"
            | "build_enum_paths"
            | "build_symbol_enum_paths"
            | "build_bits_enum_paths"
    )
}
