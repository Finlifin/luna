use super::*;
use nir::program::{Enum, Struct, Variant, VariantShape};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 93, local }
}
fn ty(local: u32, argument: Term) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![argument],
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn newtype(local: u32, representation: Term) -> iir::NominalDefinition {
    iir::NominalDefinition::Newtype(nir::newtypes::Newtype {
        declaration: id(local),
        name: "Wrapper".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation,
    })
}
fn graph(padding: u32) -> iir::Package {
    let mut nominals: Vec<_> = (100..100 + padding)
        .map(|local| iir::NominalInstance {
            ty: ty(local, boolean()),
            definition: newtype(local, boolean()),
        })
        .collect();
    for scalar in [boolean(), Term::Primitive(Primitive::Char)] {
        nominals.push(iir::NominalInstance {
            ty: ty(1, scalar.clone()),
            definition: iir::NominalDefinition::Struct(Struct {
                declaration: id(1),
                name: "Record".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![("value".into(), scalar.clone())],
            }),
        });
        nominals.push(iir::NominalInstance {
            ty: ty(2, scalar.clone()),
            definition: iir::NominalDefinition::Enum(Enum {
                declaration: id(2),
                name: "Choice".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                is_intrinsic: false,
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                variants: vec![Variant {
                    name: "some".into(),
                    shape: VariantShape::Tuple,
                    fields: vec![scalar.clone()],
                }],
            }),
        });
        nominals.push(iir::NominalInstance {
            ty: ty(3, scalar.clone()),
            definition: newtype(3, ty(1, scalar)),
        });
    }
    iir::Package {
        nominals,
        ..Default::default()
    }
}
fn frozen(local: u32, payload: Term, scalar: Term) -> Term {
    Term::NominalValue {
        ty: Box::new(ty(local, scalar)),
        variant: if local == 2 { Some(0) } else { None },
        fields: vec![payload],
    }
}
fn value(local: u32, payload: Value, scalar: Term) -> Value {
    match local {
        1 => Value::Record {
            declaration: id(1),
            arguments: vec![scalar],
            fields: vec![payload],
        },
        2 => Value::Variant {
            declaration: id(2),
            arguments: vec![scalar],
            index: 0,
            fields: vec![payload],
        },
        3 => Value::Newtype {
            ty: ty(3, scalar),
            value: Box::new(payload),
        },
        _ => unreachable!(),
    }
}

#[test]
fn concrete_value_and_reflection_costs_ignore_unrelated_instances() {
    let mut costs = vec![];
    for padding in [0, 128] {
        let graph = graph(padding);
        let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
        let before = vm.remaining_steps();
        for local in [1, 2, 3] {
            let (payload, expected) = if local == 3 {
                (
                    frozen(1, Term::Bool(true), boolean()),
                    value(1, Value::Bool(true), boolean()),
                )
            } else {
                (Term::Bool(true), Value::Bool(true))
            };
            let input = frozen(local, payload, boolean());
            let expected = value(local, expected, boolean());
            let decoded = vm
                .decode_immutable_input(&input, &ty(local, boolean()))
                .unwrap();
            assert_eq!(decoded, expected);
            assert!(vm.has_type(&decoded, &ty(local, boolean())));
            assert!(!vm.has_type(&decoded, &ty(local, Term::Primitive(Primitive::Char))));
        }
        assert_eq!(
            vm.reflect_struct(&ty(1, boolean())).unwrap().fields[0].ty,
            boolean()
        );
        assert_eq!(
            vm.reflect_enum(&ty(2, boolean())).unwrap().variants[0].fields,
            vec![boolean()]
        );
        let source = SourceRange { start: 1, end: 2 };
        for (from, to) in [
            (ty(1, boolean()), ty(3, boolean())),
            (ty(3, boolean()), ty(1, boolean())),
        ] {
            let plan = vm.prepare_newtype_conversion(&from, &to, source).unwrap();
            assert_eq!(plan.source(), &from);
            assert_eq!(plan.target(), &to);
        }
        costs.push(before - vm.remaining_steps());
    }
    assert_eq!(
        costs[0], costs[1],
        "unrelated nominal instances enter per-value work"
    );
}

#[test]
fn indexed_admission_rejects_wrong_instance_payloads_and_recovers() {
    let graph = graph(5);
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    for local in [1, 2] {
        let subject = ty(local, Term::Primitive(Primitive::Char));
        let invalid = frozen(local, Term::Bool(true), Term::Primitive(Primitive::Char));
        assert_eq!(
            vm.decode_immutable_input(&invalid, &subject)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        let input = frozen(local, Term::Bool(true), boolean());
        let before = vm.remaining_steps();
        let expected = vm
            .decode_immutable_input(&input, &ty(local, boolean()))
            .unwrap();
        let cost = before - vm.remaining_steps();
        for steps in 0..cost {
            vm.budget.steps = steps;
            assert_eq!(
                vm.decode_immutable_input(&input, &ty(local, boolean()))
                    .unwrap_err()
                    .kind,
                ErrorKind::StepsExhausted,
                "local={local} fuel={steps}"
            );
            vm.budget.steps = 100_000;
            assert_eq!(
                vm.decode_immutable_input(&input, &ty(local, boolean()))
                    .unwrap(),
                expected
            );
        }
    }
    let absent = ty(1, Term::Primitive(Primitive::Integer));
    assert!(vm.reflect_struct(&absent).is_err());
    assert!(
        vm.prepare_newtype_conversion(
            &ty(1, boolean()),
            &ty(3, Term::Primitive(Primitive::Char)),
            SourceRange { start: 1, end: 2 }
        )
        .is_err()
    );
}

#[test]
fn bit_observation_uses_checked_value_identity_after_associated_selection() {
    use comptime_runtime::bitvec::BitVec;
    use nir::enum_encoding::{Layout, Representation, Variant as BitVariant};
    let mut graph = graph(5);
    let mut budget = comptime_runtime::Budget::default();
    for nominal in &mut graph.nominals {
        let iir::NominalDefinition::Enum(definition) = &mut nominal.definition else {
            continue;
        };
        let bit = u32::from(nominal.ty == ty(2, boolean()));
        definition.variants[0].fields.clear();
        definition.variants[0].shape = VariantShape::Unit;
        let mask = BitVec::from_words(1, &[1], &mut budget).unwrap();
        let value = BitVec::from_words(1, &[bit], &mut budget).unwrap();
        let variant = BitVariant::new("some".into(), mask, value, vec![], &mut budget).unwrap();
        definition.encoding = Some(Representation {
            base_type: None,
            layout: Layout::new(1, vec![variant], &mut budget).unwrap(),
        });
    }
    let origin = nir::AssociatedTypeOrigin {
        interface: id(90),
        member: id(91),
    };
    let query = Term::AssociatedType {
        origin: Box::new(origin),
        arguments: vec![],
        subject: Box::new(boolean()),
    };
    let source = SourceRange { start: 1, end: 2 };
    for scalar in [boolean(), Term::Primitive(Primitive::Char)] {
        let selected = Expr {
            source,
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(ty(2, scalar.clone())),
        };
        let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
        vm.associated_values.insert(
            (
                InterfaceInstance {
                    declaration: id(90),
                    arguments: vec![],
                },
                boolean(),
                id(91),
            ),
            &selected,
        );
        let value = Value::Variant {
            declaration: id(2),
            arguments: vec![scalar.clone()],
            index: 0,
            fields: vec![],
        };
        let observed = vm
            .take_bits_value(
                value,
                &query,
                &Term::Primitive(Primitive::BitVec(1)),
                source,
            )
            .unwrap();
        let expected =
            BitVec::from_words(1, &[u32::from(scalar == boolean())], &mut budget).unwrap();
        assert_eq!(observed, Value::BitVec(expected));
    }
}

fn fresh(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
#[test]
fn generated_graph_definitions_are_transactional_and_reject_artifact_collisions() {
    let mut graph = graph(0);
    graph.interfaces.push(iir::InterfaceContract {
        interface: InterfaceInstance {
            declaration: id(77),
            arguments: vec![],
        },
        subject: boolean(),
        methods: vec![],
    });
    graph.dispatch.push(nir::program::DispatchInstance {
        interface: id(77),
        interface_arguments: vec![],
        subject: boolean(),
        implementation: id(78),
        arguments: vec![],
        methods: vec![],
        associated: vec![],
        associated_initializers: vec![],
    });
    let baseline = graph.clone();
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    for local in [1, 2, 3, 77, 78] {
        let iir::NominalDefinition::Newtype(definition) = newtype(local, boolean()) else {
            panic!()
        };
        assert!(matches!(
            vm.install_owned_generated_newtypes_with(vec![definition].into(), || Ok::<_, ()>(())),
            Err(NewtypePublicationError::Admission(Error {
                kind: ErrorKind::InvalidCallPlan,
                ..
            }))
        ));
    }
    let definitions: Arc<[Struct]> = vec![Struct {
        declaration: id(10),
        name: "Generated".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![("base".into(), ty(1, boolean()))],
    }]
    .into();
    assert!(
        vm.install_owned_generated_structs_with(definitions.clone(), || Err::<(), _>(
            "host rejected"
        ))
        .is_err()
    );
    assert!(vm.reflect_struct(&fresh(10)).is_err());
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = vm
            .install_owned_generated_structs_with(definitions.clone(), || -> Result<(), ()> {
                panic!("publisher unwound")
            });
    }));
    assert!(unwound.is_err());
    assert!(vm.reflect_struct(&fresh(10)).is_err());
    vm.install_owned_generated_structs_with(definitions, || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(
        vm.reflect_struct(&fresh(10)).unwrap().fields[0].ty,
        ty(1, boolean())
    );
    let payload = frozen(1, Term::Bool(true), boolean());
    let input = Term::NominalValue {
        ty: Box::new(fresh(10)),
        variant: None,
        fields: vec![payload],
    };
    assert!(vm.decode_immutable_input(&input, &fresh(10)).is_ok());
    assert!(vm.reflect_struct(&ty(10, boolean())).is_err());
    assert_eq!(graph, baseline);
}

#[test]
fn generated_concrete_views_release_ownership_and_check_inactive_storage() {
    let graph = graph(0);
    let definitions: Arc<[Enum]> = vec![Enum {
        declaration: id(20),
        name: "Generated".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        is_intrinsic: false,
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
        variants: vec![
            Variant {
                name: "empty".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "pointer".into(),
                shape: VariantShape::Tuple,
                fields: vec![Term::Pointer(Box::new(boolean()))],
            },
        ],
    }]
    .into();
    let weak = Arc::downgrade(&definitions);
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    vm.install_owned_generated_enums_with(definitions, || Ok::<_, ()>(()))
        .unwrap();
    assert_eq!(vm.reflect_enum(&fresh(20)).unwrap().variants.len(), 2);
    assert_eq!(
        vm.check_comptime_domain(&fresh(20), SourceRange { start: 1, end: 2 })
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
    assert!(weak.upgrade().is_some());
    drop(vm);
    assert!(weak.upgrade().is_none());
}

#[test]
fn generated_iir_batches_rollback_at_every_insufficient_admission_budget() {
    let graph = graph(0);
    let make = || -> Arc<[nir::newtypes::Newtype]> {
        [(30, ty(1, boolean())), (31, fresh(30))]
            .into_iter()
            .map(|(local, representation)| {
                let iir::NominalDefinition::Newtype(definition) = newtype(local, representation)
                else {
                    panic!()
                };
                definition
            })
            .collect()
    };
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    let before = vm.remaining_steps();
    vm.install_owned_generated_newtypes_with(make(), || Ok::<_, ()>(()))
        .unwrap();
    let cost = before - vm.remaining_steps();
    for steps in 0..cost {
        let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
        vm.budget.steps = steps;
        let committed = std::cell::Cell::new(false);
        assert!(matches!(
            vm.install_owned_generated_newtypes_with(make(), || {
                committed.set(true);
                Ok::<_, ()>(())
            }),
            Err(NewtypePublicationError::Admission(Error {
                kind: ErrorKind::StepsExhausted,
                ..
            }))
        ));
        assert!(!committed.get());
        assert!(vm.concrete_nominal(&fresh(30)).is_none());
        assert!(vm.concrete_nominal(&fresh(31)).is_none());
        vm.budget.steps = 100_000;
        vm.install_owned_generated_newtypes_with(make(), || Ok::<_, ()>(()))
            .unwrap();
        let plan = vm
            .prepare_newtype_conversion(&fresh(30), &fresh(31), SourceRange { start: 1, end: 2 })
            .unwrap();
        assert_eq!(plan.target(), &fresh(31));
        assert_eq!(
            vm.reflect_newtype(&fresh(30)).unwrap().representation,
            ty(1, boolean())
        );
    }
}
