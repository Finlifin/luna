//! Select the once-only preparation producers needed by staged call inputs.
use super::*;
use std::collections::BTreeMap;

pub struct Preparation {
    pub indices: BTreeSet<usize>,
    pub expression: Expr,
    bindings: Vec<(usize, LocalId, Term)>,
}

pub fn preparation(expr: &Expr, fuel: &mut usize) -> Result<Option<Preparation>, String> {
    preparation_with_layouts(&[], expr, fuel)
}

pub fn preparation_with_layouts<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
) -> Result<Option<Preparation>, String> {
    let heads = heads.into();
    let Some((indices, bindings)) = nir::preparation::inputs_with_layouts(heads.heads, expr, fuel)?
    else {
        return Ok(None);
    };
    let K::CallPreparation { statements, .. } = &expr.kind else {
        unreachable!()
    };
    let values = bindings
        .iter()
        .map(|(_, binding, ty)| Expr {
            source: expr.source,
            ty: ty.clone(),
            kind: K::Local(*binding),
        })
        .collect::<Vec<_>>();
    let ty = Term::Tuple(bindings.iter().map(|(_, _, ty)| ty.clone()).collect());
    let mut selected = Vec::new();
    for &index in &indices {
        crate::visit::expression(&statements[index], fuel, |_, _| Ok(()))?;
        selected.push(statements[index].clone());
    }
    let expression = Expr {
        source: expr.source,
        ty: ty.clone(),
        kind: K::Block {
            statements: selected,
            result: Some(Box::new(Expr {
                source: expr.source,
                ty,
                kind: K::Tuple(values),
            })),
        },
    };
    Ok(Some(Preparation {
        indices,
        expression,
        bindings,
    }))
}

impl Preparation {
    pub fn apply(self, expr: &mut Expr, completed: Expr, fuel: &mut usize) -> Result<(), String> {
        self.apply_with_layouts(&[], expr, completed, fuel)
    }

    pub fn apply_with_layouts<'layout>(
        self,
        heads: impl Into<Layouts<'layout>>,
        expr: &mut Expr,
        completed: Expr,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let heads = heads.into();
        if completed.ty != self.expression.ty {
            return Err("completed preparation type mismatch".into());
        }
        let K::Tuple(values) = completed.kind else {
            return Err("completed preparation must be a tuple".into());
        };
        if values.len() != self.bindings.len() {
            return Err("completed preparation arity mismatch".into());
        }
        let mut replacements = BTreeMap::new();
        let mut groups: BTreeMap<usize, Vec<Expr>> = BTreeMap::new();
        for ((index, binding, ty), value) in self.bindings.into_iter().zip(values) {
            if value.ty != ty {
                return Err("completed preparation binding type mismatch".into());
            }
            delivery_constant(heads, &value, fuel)?;
            replacements.insert(binding, value.clone());
            groups.entry(index).or_default().push(Expr {
                source: value.source,
                ty: Term::Primitive(Primitive::Void),
                kind: K::Let {
                    binding,
                    value: Box::new(value),
                },
            });
        }
        let K::CallPreparation {
            statements,
            result: Some(call),
        } = &mut expr.kind
        else {
            return Err("preparation changed before application".into());
        };
        let K::Call {
            callee, arguments, ..
        } = &mut call.kind
        else {
            return Err("preparation call changed before application".into());
        };
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Err("preparation callee is not callable".into());
        };
        for argument in arguments {
            if parameters[argument.parameter].sort != Sort::Runtime && !argument.is_default {
                argument.value =
                    prepare_input_with_layouts(heads, &argument.value, &replacements, fuel)?;
            }
        }
        // Completed inputs needed only for instance selection must not leave
        // Type/Integer-only bindings in runtime storage. Reconstruct only data
        // still referenced by the runtime preparation and result call.
        let mut live = nir::preparation::references(call, fuel)?;
        for (index, statement) in statements.iter().enumerate() {
            if !self.indices.contains(&index) {
                live.extend(nir::preparation::references(statement, fuel)?);
            }
        }
        for group in groups.values_mut() {
            group.retain(|expr| matches!(&expr.kind,K::Let {binding,..} if live.contains(binding)));
        }
        let mut result = Vec::new();
        for (index, statement) in std::mem::take(statements).into_iter().enumerate() {
            if self.indices.contains(&index) {
                result.extend(groups.remove(&index).unwrap_or_default());
            } else {
                result.push(statement);
            }
        }
        *statements = result;
        Ok(())
    }
}

// Delivery can include function identity without requiring a symbolic Term for
// the whole aggregate. Inspect every field; executable expressions are never
// accepted as if required invocation had already completed them.
fn delivery_constant<'layout>(
    heads: impl Into<Layouts<'layout>>,
    value: &Expr,
    fuel: &mut usize,
) -> Result<(), String> {
    let heads = heads.into();
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        charge(fuel, 1)?;
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
                if matches!(value.ty.callable_signature(), Term::Function { .. }) =>
            {
                crate::completed_type(&value.ty, &BTreeSet::new(), fuel)?;
                if let K::FunctionInstance { arguments, .. } = &value.kind {
                    for argument in arguments {
                        crate::completed_type(argument, &BTreeSet::new(), fuel)?;
                    }
                }
                if let K::TraitMethod {
                    subject,
                    interface_arguments,
                    method_arguments,
                    ..
                } = &value.kind
                {
                    // The origin is fixed by elaboration. Validate all parts of
                    // its instance without redoing member lookup or erasing the
                    // interface/implementation selection at delivery.
                    crate::completed_type(subject, &BTreeSet::new(), fuel)?;
                    for argument in interface_arguments.iter().chain(method_arguments) {
                        crate::completed_type(argument, &BTreeSet::new(), fuel)?;
                    }
                }
            }
            K::Tuple(fields) => {
                let types = if let Term::Tuple(types) = &value.ty {
                    types.as_slice()
                } else {
                    charge(fuel, heads.len())?;
                    heads
                        .iter()
                        .find_map(|head| head.tuple_elements(&value.ty))
                        .ok_or("completed tuple delivery requires trusted Tuple metadata")?
                };
                if fields.len() != types.len() {
                    return Err("completed tuple delivery arity mismatch".into());
                }
                for (field, ty) in fields.iter().zip(types) {
                    if field.ty != *ty {
                        return Err("completed tuple delivery field type mismatch".into());
                    }
                    pending.push(field);
                }
            }
            K::Record {
                declaration,
                fields,
            }
            | K::Variant {
                declaration,
                fields,
                ..
            } => {
                let Term::Nominal {
                    declaration: expected,
                    ..
                } = &value.ty
                else {
                    return Err("completed nominal delivery requires a nominal type".into());
                };
                if declaration != expected {
                    return Err("completed nominal delivery identity mismatch".into());
                }
                // Nominal layouts are checked when invocation values are
                // materialized. Preserve the same identity boundary as
                // nominal_constant, but recursively validate delivery fields
                // without forcing their function identities into Term values.
                nir::type_computations::charge_term(&value.ty, fuel).map_err(|e| e.to_string())?;
                if let K::Variant { index, .. } = &value.kind {
                    u32::try_from(*index)
                        .map_err(|_| "enum variant index exceeds frozen representation")?;
                }
                pending.extend(fields);
            }
            K::AnonymousRecord { fields } => {
                let Term::Record(types) = &value.ty else {
                    return Err("completed record delivery requires a record type".into());
                };
                if fields.len() != types.len() {
                    return Err("completed record delivery arity mismatch".into());
                }
                let mut names = BTreeSet::new();
                for (field, (name, ty)) in fields.iter().zip(types) {
                    charge(fuel, name.len())?;
                    if !names.insert(name) || field.ty != *ty {
                        return Err("completed record delivery field mismatch".into());
                    }
                    pending.push(field);
                }
            }
            _ => {
                constant(heads, value, fuel, 0)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    fn value(kind: K, ty: Term) -> Expr {
        Expr {
            kind,
            ty,
            source: nir::program::SourceRange { start: 0, end: 1 },
        }
    }
    #[test]
    fn aggregate_delivery_rejects_forged_fields_calls_and_missing_layouts() {
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        };
        let identity = value(
            K::Function(nir::DeclarationId {
                package: 1,
                local: 1,
            }),
            signature.clone(),
        );
        let tuple_type = Term::Tuple(vec![signature.clone()]);
        let tuple = value(K::Tuple(vec![identity.clone()]), tuple_type.clone());
        let record_type = Term::Record(vec![("callback".into(), tuple_type.clone())]);
        let record = value(
            K::AnonymousRecord {
                fields: vec![tuple.clone()],
            },
            record_type,
        );
        delivery_constant(&[], &record, &mut 1000).unwrap();
        assert!(delivery_constant(&[], &record, &mut 0).is_err());
        let mut wrong = tuple.clone();
        let K::Tuple(fields) = &mut wrong.kind else {
            unreachable!()
        };
        fields[0].ty = boolean.clone();
        assert!(delivery_constant(&[], &wrong, &mut 1000).is_err());
        let unevaluated = value(
            K::Call {
                callee: Box::new(identity),
                arguments: vec![],
                parameter_bindings: vec![],
            },
            signature.clone(),
        );
        let unfinished = value(K::Tuple(vec![unevaluated]), tuple_type.clone());
        assert!(delivery_constant(&[], &unfinished, &mut 1000).is_err());
        let duplicates = value(
            K::AnonymousRecord {
                fields: vec![tuple.clone(), tuple.clone()],
            },
            Term::Record(vec![
                ("same".into(), tuple_type.clone()),
                ("same".into(), tuple_type),
            ]),
        );
        assert!(delivery_constant(&[], &duplicates, &mut 1000).is_err());
        let mut opaque = tuple;
        opaque.ty = boolean;
        assert!(delivery_constant(&[], &opaque, &mut 1000).is_err());
    }
    #[test]
    fn nominal_delivery_rejects_mismatched_identity_and_unevaluated_fields() {
        let declaration = nir::DeclarationId {
            package: 1,
            local: 10,
        };
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let callback = value(
            K::Function(nir::DeclarationId {
                package: 1,
                local: 11,
            }),
            signature.clone(),
        );
        let valid = value(
            K::Record {
                declaration,
                fields: vec![callback.clone()],
            },
            ty.clone(),
        );
        delivery_constant(&[], &valid, &mut 1000).unwrap();
        let wrong = value(
            K::Record {
                declaration: nir::DeclarationId {
                    package: 1,
                    local: 12,
                },
                fields: vec![callback.clone()],
            },
            ty.clone(),
        );
        assert!(delivery_constant(&[], &wrong, &mut 1000).is_err());
        let pending = value(
            K::Call {
                callee: Box::new(callback),
                arguments: vec![],
                parameter_bindings: vec![],
            },
            signature,
        );
        let wrong = value(
            K::Variant {
                declaration,
                index: 0,
                fields: vec![pending],
            },
            ty,
        );
        assert!(delivery_constant(&[], &wrong, &mut 1000).is_err());
    }
    #[test]
    fn trait_method_delivery_requires_completed_instance_types() {
        let id = nir::DeclarationId {
            package: 1,
            local: 1,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        };
        let method = value(
            K::TraitMethod {
                interface: id,
                member: id,
                subject: boolean.clone(),
                interface_arguments: vec![boolean.clone()],
                method_arguments: vec![boolean.clone()],
            },
            signature,
        );
        delivery_constant(&[], &method, &mut 1000).unwrap();
        for component in 0..3 {
            let mut invalid = method.clone();
            let K::TraitMethod {
                subject,
                interface_arguments,
                method_arguments,
                ..
            } = &mut invalid.kind
            else {
                unreachable!()
            };
            let pending = Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            };
            match component {
                0 => *subject = pending,
                1 => interface_arguments[0] = pending,
                _ => method_arguments[0] = pending,
            }
            assert!(delivery_constant(&[], &invalid, &mut 1000).is_err());
        }
    }
}
