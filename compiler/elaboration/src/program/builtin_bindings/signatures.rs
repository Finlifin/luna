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
