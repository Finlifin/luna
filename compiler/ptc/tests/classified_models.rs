//! Classification constrains candidate domains; it does not pick a type.
use nir::{DeclarationId, Primitive, Sort, Term};
use ptc::{Budget, Context, Formula as F, KindQuery, TypeInputOutcome, TypeKind};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 984,
        local,
    }
}

fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}

fn unsigned(local: u32) -> F {
    F::Kind(KindQuery {
        ty: input(local),
        kind: TypeKind::UnsignedInteger,
    })
}

fn constraint() -> F {
    F::And(vec![
        unsigned(0),
        unsigned(1),
        F::equal(input(0), Term::Primitive(Primitive::U32)).negate(),
        F::equal(input(0), input(1)).negate(),
        F::equal(input(2), Term::Optional(Box::new(input(0)))),
    ])
}

#[test]
fn classified_type_search_backtracks_across_related_inputs() {
    let context = Context::default();
    let inputs = [id(0), id(1), id(2)];
    let outcome = context.search_type_inputs(&inputs, constraint(), &mut Budget::default());
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = outcome
    else {
        panic!("classification and exclusions leave multiple types: {outcome}")
    };
    let problem = context
        .type_input_context(&inputs, constraint(), &mut Budget::default())
        .unwrap();
    for witness in [candidate, alternative] {
        problem.verify_witness(&witness, Budget::default()).unwrap();
        assert_eq!(
            TypeKind::UnsignedInteger.evaluate(&witness.types[&id(0)]),
            Some(true)
        );
        assert_ne!(witness.types[&id(0)], Term::Primitive(Primitive::U32));
        assert_ne!(witness.types[&id(0)], witness.types[&id(1)]);
        assert_eq!(
            witness.types[&id(2)],
            Term::Optional(Box::new(witness.types[&id(0)].clone()))
        );
        let mut forged = witness;
        forged.types.insert(id(0), Term::Primitive(Primitive::U32));
        assert!(problem.verify_witness(&forged, Budget::default()).is_err());
    }
    assert!(context.assumptions().is_empty());
}

#[test]
fn classified_search_budget_exhaustion_does_not_publish_a_choice() {
    let context = Context::default();
    let inputs = [id(0), id(1), id(2)];
    for steps in [0, 10, 100] {
        assert!(matches!(
            context.search_type_inputs(
                &inputs,
                constraint(),
                &mut Budget {
                    steps,
                    ..Budget::default()
                }
            ),
            TypeInputOutcome::Unknown(ptc::UnknownReason::BudgetExhausted)
        ));
    }
    assert!(matches!(
        context.search_type_inputs(&inputs, constraint(), &mut Budget::default()),
        TypeInputOutcome::Ambiguous { .. }
    ));
    assert!(context.assumptions().is_empty());
}

#[test]
fn classified_models_also_try_closed_types_mentioned_by_the_problem() {
    let context = Context::default();
    let pointer = |p| Term::Pointer(Box::new(Term::Primitive(p)));
    let constraint = F::And(vec![
        F::Kind(KindQuery {
            ty: input(0),
            kind: TypeKind::Pointer,
        }),
        F::equal(input(0), pointer(Primitive::Bool)).negate(),
        F::equal(input(1), pointer(Primitive::U8)),
    ]);
    let problem = context
        .type_input_context(&[id(0), id(1)], constraint, &mut Budget::default())
        .unwrap();
    let ptc::Outcome::NotEntailed(witness) = problem.prove(F::False, Budget::default()) else {
        panic!("the mentioned pointer type supplies a valid alternative")
    };
    problem.verify_witness(&witness, Budget::default()).unwrap();
    assert_eq!(witness.types[&id(0)], pointer(Primitive::U8));
    // This is only a satisfying model, never proof that the input must be u8*.
    assert!(!matches!(
        problem.prove(
            F::equal(input(0), pointer(Primitive::U8)),
            Budget::default()
        ),
        ptc::Outcome::Entailed(_)
    ));
}

#[test]
fn exhausting_classified_candidates_is_not_an_inconsistency_proof() {
    let mut constraints = vec![unsigned(0)];
    for primitive in [
        Primitive::U8,
        Primitive::U16,
        Primitive::U32,
        Primitive::U64,
        Primitive::U128,
        Primitive::Usize,
    ] {
        constraints.push(F::equal(input(0), Term::Primitive(primitive)).negate());
    }
    let outcome = Context::default().search_type_inputs(
        &[id(0)],
        F::And(constraints),
        &mut Budget::default(),
    );
    assert!(matches!(outcome, TypeInputOutcome::Unknown(_)), "{outcome}");
}

#[test]
fn structural_classifications_generate_distinct_shapes_without_source_examples() {
    for kind in [TypeKind::Pointer, TypeKind::Tuple, TypeKind::Function] {
        let context = Context::default();
        let inputs = [id(0), id(1), id(2)];
        let mut constraints: Vec<_> = (0..3).map(|n| F::kind(input(n), kind)).collect();
        for a in 0..3 {
            for b in a + 1..3 {
                constraints.push(F::equal(input(a), input(b)).negate());
            }
        }
        let constraint = F::And(constraints);
        let outcome =
            context.search_type_inputs(&inputs, constraint.clone(), &mut Budget::default());
        let TypeInputOutcome::Ambiguous {
            candidate,
            alternative,
            ..
        } = outcome
        else {
            panic!("three distinct {kind} types have multiple models: {outcome}")
        };
        let problem = context
            .type_input_context(&inputs, constraint, &mut Budget::default())
            .unwrap();
        for witness in [candidate, alternative] {
            problem.verify_witness(&witness, Budget::default()).unwrap();
            for a in 0..3 {
                assert_eq!(kind.evaluate(&witness.types[&id(a)]), Some(true));
                for b in a + 1..3 {
                    assert_ne!(witness.types[&id(a)], witness.types[&id(b)]);
                }
            }
        }
        assert!(context.assumptions().is_empty());
    }
}

#[test]
fn nominal_model_search_requires_formation_evidence_for_unmentioned_heads() {
    use nir::NominalKind;
    use nir::normal_forms::NominalValueDeclaration::Struct;
    use nir::program::{Package, TypePredicate};
    let record = |local| nir::program::Struct {
        declaration: id(local),
        name: format!("R{local}"),
        fields: vec![],
        packed: nir::program::PackedMode::None,
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
    };
    let mut generic = record(10);
    generic.type_parameters.push(id(110));
    let mut guarded = record(11);
    guarded.requirements.push(TypePredicate::Bool(false));
    let mut package = Package {
        structs: vec![generic, guarded, record(12), record(13)],
        ..Default::default()
    };
    let kind = TypeKind::Nominal(NominalKind::Struct);
    let constraint = F::kind(input(0), kind);
    let search = |ctx: &Context| {
        ctx.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default())
    };
    let problem = |ctx: &Context| {
        ctx.type_input_context(&[id(0)], constraint.clone(), &mut Budget::default())
            .unwrap()
    };
    let provisional = Context::from_package(&package).unwrap();
    assert!(matches!(search(&provisional), TypeInputOutcome::Unknown(_)));
    for record in &package.structs[..3] {
        package
            .formed_nullary_nominals
            .admit_checked(Struct(record));
    }
    // A singleton supply cannot discharge the uniqueness obligation.
    let one = Context::from_package(&package).unwrap();
    assert!(matches!(search(&one), TypeInputOutcome::Unknown(_)));
    package
        .formed_nullary_nominals
        .admit_checked(Struct(&package.structs[3]));
    let context = Context::from_package(&package).unwrap();
    let TypeInputOutcome::Ambiguous {
        candidate,
        alternative,
        ..
    } = search(&context)
    else {
        panic!("two distinct checked structs")
    };
    assert_ne!(candidate.types[&id(0)], alternative.types[&id(0)]);
    for witness in [&candidate, &alternative] {
        assert!(
            matches!(&witness.types[&id(0)], Term::Nominal { declaration, arguments } if [id(12),id(13)].contains(declaration) && arguments.is_empty())
        );
        problem(&context)
            .verify_witness(witness, Budget::default())
            .unwrap();
    }
    assert!(
        problem(&provisional)
            .verify_witness(&candidate, Budget::default())
            .is_err()
    );
    let mut forged = candidate;
    forged.types.insert(
        id(0),
        Term::Nominal {
            declaration: id(10),
            arguments: vec![],
        },
    );
    assert!(
        problem(&context)
            .verify_witness(&forged, Budget::default())
            .is_err()
    );
    // The same evidence is required when the only choice is an opaque
    // function's interpreted result and there are no exported Type bindings.
    let mut opaque = context.fork();
    opaque.admit_congruence(id(99));
    opaque.assume(F::kind(
        Term::NormalForm {
            head: id(99),
            sort: Sort::Type,
            arguments: vec![Term::Bool(true)],
        },
        kind,
    ));
    let ptc::Outcome::NotEntailed(mut model) = opaque.prove(F::False, Budget::default()) else {
        panic!("an opaque Type result can denote a checked struct")
    };
    opaque.verify_witness(&model, Budget::default()).unwrap();
    assert!(model.types.is_empty());
    model.applications[0].result = Term::Nominal {
        declaration: id(11),
        arguments: vec![],
    };
    assert_eq!(
        opaque.verify_witness(&model, Budget::default()),
        Err(ptc::ProofError::Invalid)
    );
    package.structs[3]
        .requirements
        .push(TypePredicate::Bool(false));
    assert!(Context::from_package(&package).is_err());
}
