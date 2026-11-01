//! Operation values use ordinary checked function bodies and invocation.
//! Aliases retain the original binding identity; no VM-specific callable tag.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn compiler_operation_value(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let Some((name, canonical, signature)) = self.resolve_compiler_operation(owner, span)?
        else {
            return Ok(None);
        };
        let ty = signature
            .ok_or_else(|| error(span, "compiler operation binding requires a direct call until its callable contract is available"))?;
        let identity = declaration(self.package, &self.package_identities, canonical);
        self.required_steps = self
            .required_steps
            .checked_sub(self.generated_functions.len() as u64 + 1)
            .ok_or_else(|| error(span, "compiler operation function budget exhausted"))?;
        if !self
            .generated_functions
            .iter()
            .any(|function| function.declaration == identity)
        {
            let Term::Function { parameters, result } = ty.callable_signature() else {
                unreachable!()
            };
            let origin = source(self.package.item(canonical).unwrap().span);
            let mut bindings = Vec::with_capacity(parameters.len());
            let mut inputs = Vec::with_capacity(parameters.len());
            for parameter in parameters {
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index: self.next_temporary_binding,
                };
                self.next_temporary_binding = self
                    .next_temporary_binding
                    .checked_add(1)
                    .ok_or_else(|| error(span, "compiler operation input identity exhausted"))?;
                bindings.push(binding);
                inputs.push(n::Expr {
                    source: origin,
                    ty: parameter.ty.clone(),
                    kind: K::Local(binding),
                });
            }
            let body = n::Expr {
                source: origin,
                ty: result.as_ref().clone(),
                kind: operation_body(name.as_str(), inputs, origin),
            };
            nir::copy_budget::charge_expression(&body, &mut self.required_steps)
                .map_err(|_| error(span, "compiler operation body budget exhausted"))?;
            self.generated_functions.push(n::Function {
                declaration: identity,
                name: format!("<compiler-operation.{}>", name.as_str()),
                phase: if matches!(ty, Term::ComptimeFunction(_)) {
                    n::FunctionPhase::Comptime
                } else {
                    n::FunctionPhase::Runtime
                },
                inline: false,
                pure: !signatures::publishes_definition(name.as_str()),
                is_unsafe: false,
                signature: ty.callable_signature().clone(),
                parameters: bindings,
                body,
                defaults: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
            });
        }
        Ok(Some(n::Expr {
            source: source(span),
            ty,
            kind: K::Function(identity),
        }))
    }
}

fn operation_body(name: &str, mut inputs: Vec<n::Expr>, source: n::SourceRange) -> K {
    if let Some(query) = signatures::text_query(name) {
        return K::TextQuery {
            query,
            subject: Box::new(packet(inputs, source)),
        };
    }
    use n::{
        EnumBitQuery as B, EnumStructureQuery as Q, ObjectEditKind as E, ObjectQueryKind as O,
    };
    if matches!(name, "list_length" | "list_concat") {
        inputs.remove(0);
        return if name == "list_length" {
            K::ListLength(Box::new(inputs.remove(0)))
        } else {
            let right = inputs.pop().unwrap();
            K::ListConcat(Box::new(inputs.pop().unwrap()), Box::new(right))
        };
    }
    if signatures::publishes_definition(name) {
        if name == "build_bits_enum_paths" {
            let encoding = inputs.split_off(6);
            inputs.push(packet(encoding, source));
        }
        let request = Box::new(packet(inputs, source));
        return match name {
            "build_newtype" => K::BuildNewtype(request),
            "build_struct" => K::BuildStruct(request),
            _ => K::BuildEnum(request),
        };
    }
    if matches!(
        name,
        "comptime_integer_div_trunc" | "comptime_integer_rem_trunc"
    ) {
        let right = inputs.pop().unwrap();
        let left = inputs.pop().unwrap();
        return K::IntegerBinary {
            op: if name == "comptime_integer_div_trunc" {
                n::IntegerOp::DivideTrunc
            } else {
                n::IntegerOp::RemainderTrunc
            },
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    if name.starts_with("object_") {
        // The explicit Type input is already bound by ordinary invocation.
        // Its identity lives in the dependent domains, not the operation packet.
        if matches!(
            name,
            "object_value" | "object_get" | "object_append" | "object_replace"
        ) {
            inputs.remove(0);
        }
        let subject = Box::new(n::Expr {
            source,
            ty: Term::Tuple(inputs.iter().map(|input| input.ty.clone()).collect()),
            kind: K::Tuple(inputs),
        });
        return match name {
            "object_length" => K::ObjectQuery {
                subject,
                query: O::Length,
            },
            "object_key" => K::ObjectQuery {
                subject,
                query: O::Key,
            },
            "object_entry_type" => K::ObjectQuery {
                subject,
                query: O::EntryType,
            },
            "object_value" => K::ObjectQuery {
                subject,
                query: O::Value,
            },
            "object_get" => K::ObjectQuery {
                subject,
                query: O::NamedValue,
            },
            "object_concat" => K::ObjectEdit {
                subject,
                operation: E::Concat,
            },
            "object_append" => K::ObjectEdit {
                subject,
                operation: E::Append,
            },
            "object_replace" => K::ObjectEdit {
                subject,
                operation: E::Replace,
            },
            "object_remove" => K::ObjectEdit {
                subject,
                operation: E::Remove,
            },
            "object_slice" => K::ObjectEdit {
                subject,
                operation: E::Slice,
            },
            _ => unreachable!("Object callable contract has no body"),
        };
    }
    let subject = Box::new(inputs.pop().unwrap());
    match name {
        "newtype_representation" => K::NewtypeRepresentation(subject),
        "struct_field_types" => K::StructFieldTypes(subject),
        "struct_field_names" => K::StructFieldNames(subject),
        _ => K::EnumStructure {
            subject,
            query: match name {
                "enum_group_paths" => Q::GroupPaths,
                "enum_variant_paths" => Q::VariantPaths,
                "enum_variant_shapes" => Q::Shapes,
                "enum_variant_field_names" => Q::FieldNames,
                "enum_variant_field_types" => Q::FieldTypes,
                "enum_symbol_values" => Q::SymbolValues,
                "enum_bit_width" => Q::BitEncoding(B::Width),
                "enum_bit_base_type" => Q::BitEncoding(B::BaseType),
                "enum_bit_masks" => Q::BitEncoding(B::Masks),
                "enum_bit_values" => Q::BitEncoding(B::Values),
                "enum_bit_offsets" => Q::BitEncoding(B::Offsets),
                _ => unreachable!("fixed operation contract has no body"),
            },
        },
    }
}

fn packet(inputs: Vec<n::Expr>, source: n::SourceRange) -> n::Expr {
    n::Expr {
        source,
        ty: Term::Tuple(inputs.iter().map(|input| input.ty.clone()).collect()),
        kind: K::Tuple(inputs),
    }
}
