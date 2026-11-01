//! Fixed operation values use ordinary checked function bodies and invocation.
//! Aliases retain the original binding identity; no VM-specific callable tag.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn compiler_operation_value(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let Some((name, canonical)) = self.resolve_compiler_operation(owner, span)? else {
            return Ok(None);
        };
        let ty = signatures::fixed(name.as_str(), self.protocols.meta_list)
            .map_err(|message| error(span, message))?
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
                phase: n::FunctionPhase::Comptime,
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
    use n::{
        EnumBitQuery as B, EnumStructureQuery as Q, ObjectEditKind as E, ObjectQueryKind as O,
    };
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
            "object_concat" => K::ObjectEdit {
                subject,
                operation: E::Concat,
            },
            "object_remove" => K::ObjectEdit {
                subject,
                operation: E::Remove,
            },
            "object_slice" => K::ObjectEdit {
                subject,
                operation: E::Slice,
            },
            _ => unreachable!("fixed object contract has no body"),
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
