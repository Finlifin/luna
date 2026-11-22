use nir::{DeclarationId, DeclarationParameter, NominalAdmissionError, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 99, local }
}
fn rigid(local: u32, sort: Sort) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort,
    }
}
fn p(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}
fn admit<'a>(
    ids: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
) -> Result<nir::AdmittedArguments<'a>, NominalAdmissionError> {
    nir::admit_declaration_arguments_with_evidence_in(
        ids,
        domains,
        arguments,
        fuel,
        |_, _, _| Err("unexpected layout request".into()),
        |_, _, _| false,
        None,
    )
}

#[test]
fn completed_domains_authorize_type_values_without_weakening_fixed_sort_substitution() {
    let arguments = [p(Primitive::Type), p(Primitive::Bool)];
    let domains = [p(Primitive::Type), rigid(1, Sort::Type)];
    let parameters = [
        DeclarationParameter {
            declaration: id(1),
            sort: Sort::Type,
        },
        DeclarationParameter {
            declaration: id(2),
            sort: Sort::ComptimeValue,
        },
    ];
    let body = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![rigid(1, Sort::Type), rigid(2, Sort::ComptimeValue)],
    };
    assert!(
        body.instantiate_parameters(&parameters, &arguments)
            .is_err()
    );
    let admitted = admit(&[id(1), id(2)], &domains, &arguments, &mut 10000).unwrap();
    assert_eq!(
        admitted.instantiate(&body, &mut 10000).unwrap(),
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: arguments.to_vec(),
        }
    );
    assert!(
        admitted
            .to_string()
            .starts_with("(admitted-arguments (input ")
    );
    // The original occurrence sort remains part of the definition contract.
    assert!(
        admitted
            .instantiate(&rigid(2, Sort::Type), &mut 10000)
            .is_err()
    );
    let predicate = nir::program::TypePredicate::Equal(body.clone(), body.clone());
    let expected = admitted.instantiate(&body, &mut 10000).unwrap();
    assert_eq!(
        admitted
            .instantiate_predicate(&predicate, &mut 10000)
            .unwrap(),
        nir::program::TypePredicate::Equal(expected.clone(), expected)
    );
    assert!(admitted.instantiate_predicate(&predicate, &mut 0).is_err());
    assert!(admitted.instantiate(&body, &mut 0).is_err());
}

#[test]
fn captured_domains_are_resolved_before_dependent_value_inputs() {
    let domains = [rigid(1, Sort::Type), p(Primitive::Type)];
    let arguments = [p(Primitive::Bool), p(Primitive::Type)];
    let admitted = admit(&[id(2), id(1)], &domains, &arguments, &mut 10000).unwrap();
    assert_eq!(
        admitted
            .instantiate(&rigid(2, Sort::ComptimeValue), &mut 10000)
            .unwrap(),
        p(Primitive::Bool)
    );
}

#[test]
fn actual_domains_reject_mismatches_and_exhaustion_is_not_rejection() {
    let domains = [p(Primitive::Type), rigid(1, Sort::Type)];
    for arguments in [
        [p(Primitive::Bool), p(Primitive::Bool)],
        [p(Primitive::Type), Term::Bool(true)],
        [
            p(Primitive::Bool),
            Term::Integer(comptime_runtime::Integer::from_u128(1)),
        ],
    ] {
        assert!(matches!(
            admit(&[id(1), id(2)], &domains, &arguments, &mut 10000),
            Err(NominalAdmissionError::Invalid(_))
        ));
    }
    assert!(matches!(
        admit(
            &[id(1)],
            &[p(Primitive::Type)],
            &[p(Primitive::Bool)],
            &mut 0
        ),
        Err(NominalAdmissionError::Budget)
    ));
    let cycle = [2, 1].map(|input| Term::Nominal {
        declaration: id(9),
        arguments: vec![rigid(input, Sort::ComptimeValue)],
    });
    assert!(
        admit(
            &[id(1), id(2)],
            &cycle,
            &[Term::Bool(true), Term::Bool(false)],
            &mut 10000
        )
        .is_err()
    );
}

#[test]
fn structural_substitution_checks_completed_sorts_and_preserves_caller_scope() {
    let ids = [id(1), id(2)];
    let domains = [p(Primitive::Type), rigid(1, Sort::Type)];
    let frames = [vec![Sort::Type]];
    let arguments = [
        p(Primitive::Type),
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ];
    let substitution =
        nir::DeclarationSubstitution::bounded(&ids, &domains, &arguments, &frames, &mut 10000)
            .unwrap();
    let body = Term::NormalForm {
        head: id(3),
        sort: Sort::Type,
        arguments: vec![rigid(2, Sort::ComptimeValue)],
    };
    assert_eq!(
        substitution.instantiate_bounded(&body, &mut 10000).unwrap(),
        Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![arguments[1].clone()]
        }
    );
    let nested = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(body.clone()),
    };
    let expected_nested = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Type,
            ty: p(Primitive::Type),
        }],
        result: Box::new(Term::NormalForm {
            head: id(3),
            sort: Sort::Type,
            arguments: vec![Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            }],
        }),
    };
    assert_eq!(substitution.instantiate(&nested).unwrap(), expected_nested);
    assert!(substitution.to_string().contains("(frames (frame type))"));
    // The evidence returned by completed domain admission has to preserve
    // exactly the same outer frame when the receiver later uses it.
    let mut queries = nir::value_admission::DomainQueries {
        layout: &mut |_, _, _| Err("unexpected layout query".into()),
        declared: &mut |_, _, _| Ok(false),
        collection: &mut |_, _| Ok(None),
        callable: &mut |_, _, _| Ok(None),
        target: None,
    };
    let admitted = nir::admit_declaration_arguments_in_scope(
        &ids,
        &domains,
        &arguments,
        &frames,
        &mut 10000,
        &mut queries,
    )
    .unwrap();
    assert_eq!(
        admitted.instantiate(&nested, &mut 10000).unwrap(),
        expected_nested
    );
    assert!(admitted.to_string().contains("(frames (frame type))"));
    assert!(
        nir::admit_declaration_arguments_in_scope(
            &ids,
            &domains,
            &arguments,
            &[],
            &mut 10000,
            &mut queries
        )
        .is_err()
    );
    assert!(nir::DeclarationSubstitution::new(&ids, &domains, &arguments, &[]).is_err());
    assert!(
        nir::DeclarationSubstitution::bounded(&ids, &domains, &arguments, &frames, &mut 0).is_err()
    );
    for arguments in [
        [p(Primitive::Type), Term::Bool(true)],
        [p(Primitive::Bool), p(Primitive::Bool)],
    ] {
        assert!(nir::DeclarationSubstitution::new(&ids, &domains, &arguments, &[]).is_err());
    }
    // An opaque domain does not become Type by looking at the supplied value.
    let opaque = [p(Primitive::Type), body];
    assert!(
        nir::DeclarationSubstitution::new(
            &ids,
            &opaque,
            &[p(Primitive::Type), p(Primitive::Bool)],
            &[]
        )
        .is_err()
    );
}

#[test]
fn sparse_forward_dependencies_admit_once_in_declaration_sweeps() {
    // Even slots form a long chain of forward dependencies. The odd slots
    // are independent and query their receiving world in declaration order.
    let width = 1024u32;
    let ids: Vec<_> = (0..width).map(id).collect();
    let arguments: Vec<_> = (0..width)
        .map(|slot| rigid(width + slot, Sort::ComptimeValue))
        .collect();
    let template = |input| Term::NormalForm {
        head: id(4 * width),
        sort: Sort::Type,
        arguments: vec![input],
    };
    let domains: Vec<_> = (0..width)
        .map(|slot| {
            if slot % 2 == 0 && slot + 2 < width {
                template(rigid(slot + 2, Sort::ComptimeValue))
            } else {
                p(Primitive::Bool)
            }
        })
        .collect();
    let mut queried = Vec::new();
    let mut fuel = 100 * width as usize;
    let admitted = nir::admit_declaration_arguments_with_evidence_in(
        &ids,
        &domains,
        &arguments,
        &mut fuel,
        |_, _, _| panic!("symbolic inputs do not request a layout"),
        |value, domain, _| {
            let Term::Rigid { declaration, .. } = value else {
                return false;
            };
            let slot = declaration.local - width;
            queried.push(slot);
            if slot % 2 == 0 && slot + 2 < width {
                *domain == template(rigid(width + slot + 2, Sort::ComptimeValue))
            } else {
                *domain == p(Primitive::Bool)
            }
        },
        None,
    )
    .unwrap();
    let expected: Vec<_> = (0..width)
        .filter(|slot| slot % 2 == 1 || *slot == width - 2)
        .chain((0..width - 2).rev().filter(|slot| slot % 2 == 0))
        .collect();
    assert_eq!(queried, expected);
    // Once admitted, projecting one field costs only its template and payload.
    let body = template(rigid(0, Sort::ComptimeValue));
    assert_eq!(
        admitted.instantiate(&body, &mut 16).unwrap(),
        template(arguments[0].clone())
    );
}

#[test]
fn checked_substitutions_charge_only_referenced_payloads_on_each_projection() {
    use nir::program::{Expr, ExprKind, SourceRange, TypePredicate};
    let ids = [id(1), id(2)];
    let arguments = [Term::Tuple(vec![p(Primitive::Bool); 512]), p(Primitive::U8)];
    let checked =
        nir::DeclarationSubstitution::bounded(&ids, &[], &arguments, &[], &mut 2000).unwrap();
    let expression = |ty| Expr {
        source: SourceRange { start: 3, end: 6 },
        ty: p(Primitive::Type),
        kind: ExprKind::TypeValue(ty),
    };
    let large = expression(rigid(1, Sort::Type));
    assert!(matches!(
        checked.instantiate_expression_bounded(&large, &mut 32),
        Err(nir::ExpressionInstantiationError::Copy(
            nir::copy_budget::CopyBudgetExhausted::Steps
        ))
    ));
    let small = expression(rigid(2, Sort::Type));
    assert_eq!(
        checked
            .instantiate_expression_bounded(&small, &mut 32)
            .unwrap(),
        expression(p(Primitive::U8))
    );
    let repeated = TypePredicate::Equal(rigid(1, Sort::Type), rigid(1, Sort::Type));
    assert!(matches!(
        checked.instantiate_predicate_bounded(&repeated, &mut 800),
        Err(nir::type_computations::Error::Budget)
    ));
    let projected = TypePredicate::Equal(rigid(2, Sort::Type), p(Primitive::U8));
    assert_eq!(
        checked
            .instantiate_predicate_bounded(&projected, &mut 32)
            .unwrap(),
        TypePredicate::Equal(p(Primitive::U8), p(Primitive::U8))
    );
}

#[test]
fn newtype_captures_query_before_independent_own_slots_without_reordering_identity() {
    let declaration = nir::newtypes::Newtype {
        declaration: id(8),
        name: "Captured".into(),
        type_parameters: vec![id(0), id(1), id(2)],
        parameter_types: vec![p(Primitive::Bool); 3],
        capture_count: 1,
        representation: p(Primitive::Bool),
        requirements: vec![],
        capabilities: vec![],
    };
    let arguments = [10, 11, 12].map(|index| rigid(index, Sort::ComptimeValue));
    let mut seen = Vec::new();
    let mut queries = nir::value_admission::DomainQueries {
        declared: &mut |value, _, _| {
            seen.push(value.clone());
            Ok(true)
        },
        collection: &mut |_, _| Ok(None),
        layout: &mut |_, _, _| unreachable!(),
        callable: &mut |_, _, _| unreachable!(),
        target: None,
    };
    let admitted = declaration
        .admit_arguments_in_scope(&arguments, &[], &mut 1000, &mut queries)
        .unwrap();
    assert_eq!(
        seen,
        [
            arguments[2].clone(),
            arguments[0].clone(),
            arguments[1].clone()
        ]
    );
    for slot in 0..3 {
        assert_eq!(
            admitted
                .instantiate(&rigid(slot, Sort::ComptimeValue), &mut 16)
                .unwrap(),
            arguments[slot as usize]
        );
    }
}

#[test]
fn closed_clause_inputs_and_ordinary_contract_binders_have_distinct_scopes() {
    use nir::{
        DeclarationInputScope as Scope, InputAdmissionStage, value_admission::DomainQueries,
    };
    let frames = [vec![Sort::Type]];
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let ids = [id(11), id(12)];
    let domains = [p(Primitive::Type), rigid(11, Sort::Type)];
    let mut queries = DomainQueries {
        layout: &mut |_, _, _| Err("unexpected layout".into()),
        declared: &mut |_, _, _| {
            Err(NominalAdmissionError::Unresolved(
                "waiting for binder".into(),
            ))
        },
        callable: &mut |_, _, _| Ok(None),
        collection: &mut |_, _| Ok(None),
        target: None,
    };
    let inputs = [p(Primitive::Type), bound.clone()];
    let mut fuel = 100_000;
    assert!(
        nir::admit_declaration_inputs(
            &ids,
            &domains,
            &inputs,
            Scope::ClosedInBody(&frames),
            0,
            &mut fuel,
            &mut queries
        )
        .is_err()
    );
    let admitted = nir::admit_declaration_inputs(
        &ids,
        &domains,
        &inputs,
        Scope::Caller(&frames),
        0,
        &mut fuel,
        &mut queries,
    )
    .unwrap();
    assert_eq!(
        admitted
            .instantiate(&rigid(12, Sort::ComptimeValue), &mut fuel)
            .unwrap(),
        bound
    );
    let inputs = [p(Primitive::Type), p(Primitive::Bool)];
    let admitted = nir::admit_declaration_inputs(
        &ids,
        &domains,
        &inputs,
        Scope::ClosedInBody(&frames),
        0,
        &mut fuel,
        &mut queries,
    )
    .unwrap();
    // The same admitted values specialize the whole signature outside the
    // ordinary parameter frame and the declaration contracts within it.
    assert!(
        admitted
            .instantiate_in_input_scope(&bound, &mut fuel)
            .is_err()
    );
    assert_eq!(
        admitted
            .instantiate_in_input_scope(&rigid(12, Sort::ComptimeValue), &mut fuel)
            .unwrap(),
        p(Primitive::Bool)
    );
    let template = Term::Tuple(vec![bound.clone(), rigid(12, Sort::ComptimeValue)]);
    assert_eq!(
        admitted.instantiate(&template, &mut fuel).unwrap(),
        Term::Tuple(vec![bound, p(Primitive::Bool)])
    );

    let failure = nir::admit_declaration_inputs(
        &ids,
        &domains,
        &[p(Primitive::Bool), rigid(13, Sort::ComptimeValue)],
        Scope::Caller(&[]),
        0,
        &mut fuel,
        &mut queries,
    )
    .err()
    .unwrap();
    assert_eq!(failure.slot, Some((1, InputAdmissionStage::Value)));
    assert_eq!(
        failure.reason,
        NominalAdmissionError::Unresolved("waiting for binder".into())
    );
}

#[test]
fn lexical_type_inputs_shadow_substitution_only_inside_the_body() {
    use nir::program::{Expr, ExprKind as K, NormalFormArgument as A, SourceRange};
    let ids = [id(1), id(2)];
    let types = [p(Primitive::Bool), p(Primitive::Char)];
    let substitution =
        nir::DeclarationSubstitution::bounded(&ids, &[], &types, &[], &mut 200).unwrap();
    let source = SourceRange { start: 1, end: 2 };
    let read = |ty| Expr {
        source,
        ty: p(Primitive::Type),
        kind: K::TypeValue(ty),
    };
    let outer = rigid(1, Sort::Type);
    let scope = Expr {
        source,
        ty: p(Primitive::Type),
        kind: K::WithTypeInputs {
            projections: Default::default(),
            parameters: vec![id(1)],
            preparations: vec![],
            arguments: vec![A::Value(read(outer.clone()))],
            body: Box::new(read(Term::Tuple(vec![outer.clone(), rigid(2, Sort::Type)]))),
        },
    };
    let mut closed_input = scope.clone();
    let K::WithTypeInputs { arguments, .. } = &mut closed_input.kind else {
        unreachable!()
    };
    arguments[0] = A::Value(read(p(Primitive::Bool)));
    assert_eq!(
        nir::copy_budget::expression_declaration_inputs(&closed_input, &mut 1000).unwrap(),
        [id(2)].into_iter().collect(),
    );
    assert_eq!(
        nir::copy_budget::expression_declaration_inputs(&scope, &mut 1000).unwrap(),
        [id(1), id(2)].into_iter().collect(),
    );
    let actual = substitution
        .instantiate_expression_bounded(&scope, &mut 1000)
        .unwrap();
    let K::WithTypeInputs {
        arguments, body, ..
    } = actual.kind
    else {
        panic!()
    };
    assert_eq!(arguments, [A::Value(read(p(Primitive::Bool)))]);
    assert_eq!(*body, read(Term::Tuple(vec![outer, p(Primitive::Char)])));
}

#[test]
fn lexical_snapshot_domains_use_the_new_binders_before_body_specialization() {
    use nir::program::{Expr, ExprKind as K, LocalId, NormalFormArgument as A, SourceRange};
    let source = SourceRange { start: 2, end: 5 };
    let local = LocalId { owner: 3, index: 8 };
    let inner_type = rigid(1, Sort::Type);
    let outside_type = rigid(2, Sort::Type);
    let scope = Expr {
        source,
        ty: p(Primitive::Bool),
        kind: K::WithTypeInputs {
            projections: Default::default(),
            preparations: vec![],
            parameters: vec![id(1), id(3)],
            arguments: vec![
                A::Value(Expr {
                    source,
                    ty: p(Primitive::Type),
                    kind: K::TypeValue(outside_type),
                }),
                A::Value(Expr {
                    source,
                    ty: inner_type.clone(),
                    kind: K::Local(local),
                }),
            ],
            body: Box::new(Expr {
                source,
                ty: p(Primitive::Bool),
                kind: K::Literal(nir::program::Literal::Bool(true)),
            }),
        },
    };
    assert_eq!(
        nir::copy_budget::expression_declaration_inputs(&scope, &mut 1000).unwrap(),
        [id(2)].into_iter().collect()
    );
    let args = [p(Primitive::Bool), p(Primitive::Char)];
    let substitution =
        nir::DeclarationSubstitution::bounded(&[id(1), id(2)], &[], &args, &[], &mut 200).unwrap();
    let actual = substitution
        .instantiate_expression_bounded(&scope, &mut 1000)
        .unwrap();
    let K::WithTypeInputs { arguments, .. } = actual.kind else {
        panic!()
    };
    assert_eq!(
        arguments[0].expression().unwrap().kind,
        K::TypeValue(p(Primitive::Char))
    );
    assert_eq!(arguments[1].expression().unwrap().ty, inner_type);
}
