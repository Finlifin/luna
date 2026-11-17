//! Restore invocation-local Type identities in result data and capture graphs.
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
    // The VM may retain an authorized opaque Type input as TypeParameter.
    // Deliver the same identity carrier for root results as for restored data;
    // this conversion does not assign an inference variable or prove a trait.
    let value = match value {
        Value::TypeParameter(declaration) => {
            *steps = steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "required Type result restoration budget exhausted"))?;
            Value::Type(Term::Rigid {
                declaration,
                sort: Sort::Type,
            })
        }
        value => value,
    };
    if parameters.is_empty() {
        return Ok(value);
    }
    if depth >= 128 {
        return Err(error(span, "required type packet nesting limit exceeded"));
    }
    value
        .restore_scoped_types(parameters, arguments, frames, steps)
        .map_err(|kind| {
            error(
                span,
                format!("scoped invocation result restoration failed: {kind:?}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use comptime_interpretor::{ObjectValue, Storage};
    use comptime_runtime::object::{Entry, Object};

    #[test]
    fn root_opaque_type_delivery_is_metered_without_inference_assignment() {
        let declaration = DeclarationId {
            package: 99,
            local: 3,
        };
        let deliver = |steps: &mut u64| {
            restore_required_packet(
                steps,
                Value::TypeParameter(declaration),
                &[],
                &[],
                &[],
                Span::default(),
                0,
            )
        };
        assert!(deliver(&mut 0).is_err());
        let mut steps = 1;
        assert_eq!(
            deliver(&mut steps).unwrap(),
            Value::Type(Term::Rigid {
                declaration,
                sort: Sort::Type
            })
        );
        assert_eq!(steps, 0);
    }

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
        let Value::Storage(restored) = restore(Value::Storage(storage.clone()), &mut 1000).unwrap()
        else {
            panic!("storage")
        };
        assert_ne!(restored, storage);
        assert_eq!(restored.read().unwrap(), Value::Type(arguments[0].clone()));
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
        assert!(failure.message.contains("DepthExhausted"));
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
