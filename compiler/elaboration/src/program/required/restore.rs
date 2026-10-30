//! Restore invocation-local Type identities throughout immutable result data.
use super::*;

pub(super) fn restore_required_packet(
    steps: &mut u64,
    value: Value,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    frames: &[Vec<Sort>],
    span: Span,
    depth: usize,
) -> Result<Value, Error> {
    if depth >= 128 {
        return Err(error(span, "required type packet nesting limit exceeded"));
    }
    *steps = steps
        .checked_sub(1)
        .ok_or_else(|| error(span, "required type packet budget exhausted"))?;
    Ok(match value {
        Value::TypeParameter(declaration) => Value::Type(restore_required_type_input(
            steps,
            Term::Rigid {
                declaration,
                sort: Sort::Type,
            },
            parameters,
            arguments,
            frames,
            span,
        )?),
        Value::Type(ty) => Value::Type(restore_required_type_input(
            steps, ty, parameters, arguments, frames, span,
        )?),
        Value::List { ty, values } => Value::List {
            ty: restore_required_type_input(steps, ty, parameters, arguments, frames, span)?,
            values: values
                .into_iter()
                .map(|value| {
                    restore_required_packet(
                        steps,
                        value,
                        parameters,
                        arguments,
                        frames,
                        span,
                        depth + 1,
                    )
                })
                .collect::<Result<_, _>>()?,
        },
        Value::Tuple(values) => Value::Tuple(
            values
                .into_iter()
                .map(|value| {
                    restore_required_packet(
                        steps,
                        value,
                        parameters,
                        arguments,
                        frames,
                        span,
                        depth + 1,
                    )
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::AnonymousRecord(fields) => Value::AnonymousRecord(
            fields
                .into_iter()
                .map(|(name, value)| {
                    Ok((
                        name,
                        restore_required_packet(
                            steps,
                            value,
                            parameters,
                            arguments,
                            frames,
                            span,
                            depth + 1,
                        )?,
                    ))
                })
                .collect::<Result<_, Error>>()?,
        ),
        Value::Record {
            declaration,
            arguments: inputs,
            fields,
        } => Value::Record {
            declaration,
            arguments: inputs
                .into_iter()
                .map(|ty| {
                    restore_required_type_input(steps, ty, parameters, arguments, frames, span)
                })
                .collect::<Result<_, _>>()?,
            fields: fields
                .into_iter()
                .map(|value| {
                    restore_required_packet(
                        steps,
                        value,
                        parameters,
                        arguments,
                        frames,
                        span,
                        depth + 1,
                    )
                })
                .collect::<Result<_, _>>()?,
        },
        Value::Variant {
            declaration,
            arguments: inputs,
            index,
            fields,
        } => Value::Variant {
            declaration,
            index,
            arguments: inputs
                .into_iter()
                .map(|ty| {
                    restore_required_type_input(steps, ty, parameters, arguments, frames, span)
                })
                .collect::<Result<_, _>>()?,
            fields: fields
                .into_iter()
                .map(|value| {
                    restore_required_packet(
                        steps,
                        value,
                        parameters,
                        arguments,
                        frames,
                        span,
                        depth + 1,
                    )
                })
                .collect::<Result<_, _>>()?,
        },
        Value::Optional { ty, value } => Value::Optional {
            ty: restore_required_type_input(steps, ty, parameters, arguments, frames, span)?,
            value: value
                .map(|value| {
                    restore_required_packet(
                        steps,
                        *value,
                        parameters,
                        arguments,
                        frames,
                        span,
                        depth + 1,
                    )
                    .map(Box::new)
                })
                .transpose()?,
        },
        Value::ErrorResult {
            ty,
            error: tag,
            value,
        } => Value::ErrorResult {
            ty: restore_required_type_input(steps, ty, parameters, arguments, frames, span)?,
            error: tag,
            value: Box::new(restore_required_packet(
                steps,
                *value,
                parameters,
                arguments,
                frames,
                span,
                depth + 1,
            )?),
        },
        Value::Newtype { ty, value } => Value::Newtype {
            ty: restore_required_type_input(steps, ty, parameters, arguments, frames, span)?,
            value: Box::new(restore_required_packet(
                steps,
                *value,
                parameters,
                arguments,
                frames,
                span,
                depth + 1,
            )?),
        },
        Value::RuntimeFunction {
            declaration,
            arguments: inputs,
        } => Value::RuntimeFunction {
            declaration,
            arguments: inputs
                .into_iter()
                .map(|ty| {
                    restore_required_type_input(steps, ty, parameters, arguments, frames, span)
                })
                .collect::<Result<_, _>>()?,
        },
        Value::AbstractValue { declaration, ty } => Value::AbstractValue {
            declaration,
            ty: restore_required_type_input(steps, ty, parameters, arguments, frames, span)?,
        },
        Value::Object(object) => {
            use comptime_runtime::object::{Entry, Object};
            let entries = object
                .into_entries()
                .into_iter()
                .map(|entry| {
                    let restore = |steps: &mut u64, entry: comptime_interpretor::ObjectValue| {
                        Ok::<_, Error>(comptime_interpretor::ObjectValue {
                            ty: restore_required_type_input(
                                steps, entry.ty, parameters, arguments, frames, span,
                            )?,
                            value: restore_required_packet(
                                steps,
                                entry.value,
                                parameters,
                                arguments,
                                frames,
                                span,
                                depth + 1,
                            )?,
                        })
                    };
                    Ok(match entry {
                        Entry::Value(value) => Entry::Value(restore(steps, value)?),
                        Entry::Property { key, value } => Entry::Property {
                            key,
                            value: restore(steps, value)?,
                        },
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Value::Object(Object::from_entries(entries))
        }
        Value::Code(_)
        | Value::Closure(_)
        | Value::RuntimeTraitMethod(_)
        | Value::Pointer(_)
        | Value::Storage(_)
        | Value::ComptimeReference(_)
            if !parameters.is_empty() =>
        {
            return Err(error(
                span,
                "scoped invocation result retains executable or mutable state; restoration is not implemented",
            ));
        }
        value => value,
    })
}

pub(super) fn restore_required_type_input(
    steps: &mut u64,
    ty: Term,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    frames: &[Vec<Sort>],
    span: Span,
) -> Result<Term, Error> {
    if parameters.is_empty() {
        return Ok(ty);
    }
    let mut fuel = usize::try_from(*steps).unwrap_or(usize::MAX);
    let before = fuel;
    let bounded = (|| {
        nir::type_computations::charge_term(&ty, &mut fuel)?;
        fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(nir::type_computations::Error::Budget)?;
        Ok::<_, nir::type_computations::Error>(())
    })();
    *steps -= (before - fuel) as u64;
    bounded.map_err(|e| error(span, e.to_string()))?;
    ty.instantiate_parameters_in_scope(parameters, arguments, frames)
        .map_err(|e| error(span, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use comptime_interpretor::{ObjectValue, Storage};
    use comptime_runtime::object::{Entry, Object};

    #[test]
    fn object_entry_domains_and_nested_values_restore_without_mutating_storage() {
        let id = DeclarationId {
            package: 99,
            local: 4,
        };
        let rigid = Term::Rigid {
            declaration: id,
            sort: Sort::Type,
        };
        let slot = Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        };
        let packet = |ty: Term| {
            Value::Object(Object::from_entries(vec![
                Entry::Property {
                    key: "kind".into(),
                    value: ObjectValue {
                        ty: p(P::Type),
                        value: Value::Type(ty.clone()),
                    },
                },
                Entry::Value(ObjectValue {
                    ty: Term::Optional(Box::new(p(P::Type))),
                    value: Value::Optional {
                        ty: Term::Optional(Box::new(p(P::Type))),
                        value: Some(Box::new(Value::Type(ty))),
                    },
                }),
            ]))
        };
        let parameters = [nir::DeclarationParameter {
            declaration: id,
            sort: Sort::Type,
        }];
        let arguments = [slot.clone()];
        let frames = [vec![Sort::Type]];
        let restore = |value, steps: &mut u64| {
            restore_required_packet(
                steps,
                value,
                &parameters,
                &arguments,
                &frames,
                Span::default(),
                0,
            )
        };
        let original = packet(rigid.clone());
        let mut budget = 1000;
        assert_eq!(
            restore(original.clone(), &mut budget).unwrap(),
            packet(slot)
        );
        let used = 1000 - budget;
        assert!(used > 1);
        assert!(restore(original.clone(), &mut (used - 1)).is_err());
        let mut exact = used;
        assert!(restore(original.clone(), &mut exact).is_ok());
        assert_eq!(exact, 0);
        assert_eq!(original, packet(rigid.clone()));

        let storage = Storage::new(Value::Type(rigid.clone()));
        assert!(restore(Value::Storage(storage.clone()), &mut 1000).is_err());
        assert_eq!(storage.read().unwrap(), Value::Type(rigid));
    }

    #[test]
    fn nested_results_obey_depth_and_scope_limits() {
        let id = DeclarationId {
            package: 99,
            local: 4,
        };
        let parameters = [nir::DeclarationParameter {
            declaration: id,
            sort: Sort::Type,
        }];
        let arguments = [Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }];
        let leaf = Value::TypeParameter(id);
        assert!(
            restore_required_packet(
                &mut 1000,
                leaf.clone(),
                &parameters,
                &arguments,
                &[],
                Span::default(),
                0
            )
            .is_err()
        );
        let mut deep = leaf.clone();
        for _ in 0..128 {
            deep = Value::Tuple(vec![deep]);
        }
        let failure = restore_required_packet(
            &mut 10000,
            deep,
            &parameters,
            &arguments,
            &[vec![Sort::Type]],
            Span::default(),
            0,
        )
        .unwrap_err();
        assert!(failure.message.contains("nesting limit"));
        assert!(
            restore_required_packet(
                &mut 1000,
                leaf,
                &parameters,
                &arguments,
                &[vec![Sort::Type]],
                Span::default(),
                0
            )
            .is_ok()
        );
    }
}
