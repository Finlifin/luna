use nir::error_results::{Error, Identity, ResultType, Row, Transport};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn label(index: u32, argument: bool) -> Term {
    Term::Nominal {
        declaration: DeclarationId {
            package: 1,
            local: index,
        },
        arguments: vec![Term::Bool(argument)],
    }
}
// Explicit noncanonical physical layouts remain valid transport fixtures.
// The ordinary type constructor canonicalizes source and substituted rows.
fn physical_result(errors: Vec<Term>, result: Term) -> Term {
    Term::ErrorResult {
        errors,
        result: Box::new(result),
    }
}
// These fixtures are closed declared nominal instances with distinct boolean
// indices. This is fixture evidence, not a structural-inequality type solver.
fn identity(_: &Term, _: &Term, _: &mut usize) -> Result<Identity, Error> {
    Ok(Identity::Distinct)
}
fn row(labels: Vec<Term>) -> Row {
    Row::new(labels, &mut 10000, &mut identity).unwrap()
}

fn tuple_layout(head: nir::DeclarationId) -> nir::normal_forms::Head {
    nir::normal_forms::Head {
        declaration: head,
        role: nir::normal_forms::Role::Tuple,
        collector: Some(0),
        parameters: vec![nir::normal_forms::Parameter {
            declaration: nir::DeclarationId {
                package: head.package,
                local: head.local + 1,
            },
            binding: nir::program::LocalId {
                owner: head.local,
                index: 0,
            },
            kind: nir::normal_forms::Kind::Value,
            ty: Term::NormalForm {
                head: nir::DeclarationId {
                    package: head.package,
                    local: head.local + 2,
                },
                sort: nir::Sort::Type,
                arguments: vec![Term::Primitive(nir::Primitive::Type)],
            },
        }],
        requirements: vec![],
    }
}
fn tuple_input(metadata: &nir::normal_forms::Head, fields: Vec<Term>) -> Term {
    Term::NominalValue {
        ty: Box::new(metadata.parameters[0].ty.clone()),
        variant: None,
        fields,
    }
}

#[test]
fn complete_type_instances_are_labels_and_duplicates_keep_first_position() {
    let a = label(1, false);
    let b = label(1, true);
    let result = row(vec![a.clone(), b.clone(), a.clone()]);
    assert_eq!(result.labels(), &[a, b]);
    assert_eq!(
        result.to_string(),
        row(result.labels().to_vec()).to_string()
    );
}
#[test]
fn embedding_reorders_error_tags_without_changing_identity_or_dropping_labels() {
    let a = label(1, false);
    let b = label(2, false);
    let c = label(3, false);
    let source = row(vec![a.clone(), b.clone()]);
    let target = row(vec![b.clone(), c, a.clone()]);
    let plan = source
        .embedding_into(&target, &mut 10000, &mut identity)
        .unwrap();
    assert_eq!(plan.map_error(0), Ok(2));
    assert_eq!(plan.map_error(1), Ok(0));
    assert_eq!(plan.map_error(2), Err(Error::InvalidTag));
    assert_eq!(plan.to_string(), "(error-embedding (tags 2 0))");
    assert_eq!(
        source.embedding_into(&row(vec![a]), &mut 10000, &mut identity),
        Err(Error::MissingLabel)
    );
}
#[test]
fn input_residual_is_independent_of_the_handlers_output_row() {
    let a = label(1, false);
    let b = label(2, false);
    let c = label(3, false);
    let input = row(vec![a.clone(), b.clone(), c.clone()]);
    let residual = input
        .subtract(&row(vec![b.clone()]), &mut 10000, &mut identity)
        .unwrap();
    assert_eq!(residual.row().labels(), &[a.clone(), c]);
    assert_eq!(residual.map_error(0), Ok(Some(0)));
    assert_eq!(residual.map_error(1), Ok(None));
    assert_eq!(residual.map_error(2), Ok(Some(1)));
    assert_eq!(residual.map_error(3), Err(Error::InvalidTag));
    let output = row(vec![b, a]);
    assert_eq!(output.labels().len(), 2);
    assert!(
        residual
            .to_string()
            .contains("(tags (remaining 0) (handled) (remaining 1))")
    );
    assert_eq!(
        input.subtract(&row(vec![label(4, false)]), &mut 10000, &mut identity),
        Err(Error::MissingLabel)
    );
}
#[test]
fn nested_qualification_merges_rows_and_empty_qualification_has_no_layer() {
    let success = Term::NormalForm {
        head: DeclarationId {
            package: 2,
            local: 8,
        },
        sort: Sort::Type,
        arguments: vec![Term::Primitive(P::Bool)],
    };
    let plain = ResultType::plain(success.clone(), &mut 10000).unwrap();
    assert!(
        ResultType::qualify(row(vec![]), plain.clone(), &mut 10000, &mut identity)
            .unwrap()
            .is_plain()
    );
    let inner =
        ResultType::qualify(row(vec![label(1, false)]), plain, &mut 10000, &mut identity).unwrap();
    let result = ResultType::qualify(
        row(vec![label(2, false), label(1, false)]),
        inner,
        &mut 10000,
        &mut identity,
    )
    .unwrap();
    assert_eq!(
        result.errors().labels(),
        &[label(2, false), label(1, false)]
    );
    assert_eq!(result.success(), &success);
    let other = ResultType::plain(Term::Primitive(P::Bool), &mut 10000).unwrap();
    assert_eq!(
        result.check_success_transport(&other, &mut 10000, &mut identity),
        Err(Error::IncompatibleSuccess)
    );
    assert!(
        result
            .to_string()
            .starts_with("(error-result (error-row (labels")
    );
}
#[test]
fn unknown_identity_is_not_distinctness_and_evidence_can_merge_representations() {
    let a = label(1, false);
    let b = label(2, false);
    assert_eq!(
        Row::new(vec![a.clone(), b.clone()], &mut 10000, &mut |_, _, _| Ok(
            Identity::Unknown
        )),
        Err(Error::UnresolvedIdentity)
    );
    let fields = vec![
        ("a".into(), Term::Primitive(P::Bool)),
        ("b".into(), Term::Primitive(P::Char)),
    ];
    let mut reversed = fields.clone();
    reversed.reverse();
    let first = Term::Record(fields);
    let equal = Row::new(
        vec![first.clone(), Term::Record(reversed)],
        &mut 10000,
        &mut |_, _, _| Ok(Identity::Same),
    )
    .unwrap();
    assert_eq!(equal.labels(), &[first]);
}
#[test]
fn malformed_labels_and_low_budget_fail_without_mutating_inputs() {
    for label in [
        Term::Bool(true),
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ] {
        assert_eq!(
            Row::new(vec![label], &mut 10000, &mut identity),
            Err(Error::InvalidType)
        );
    }
    let source = row(vec![label(1, false)]);
    let target = row(vec![label(2, false), label(1, false)]);
    let original = source.clone();
    assert_eq!(
        source.embedding_into(&target, &mut 0, &mut identity),
        Err(Error::Budget)
    );
    assert_eq!(source, original);
    assert_eq!(
        source
            .embedding_into(&target, &mut 10000, &mut identity)
            .unwrap()
            .map_error(0),
        Ok(1)
    );
    assert_eq!(
        ResultType::plain(Term::Bool(false), &mut 100),
        Err(Error::InvalidType)
    );
}

#[test]
fn success_injection_and_result_embedding_are_distinct_checked_operations() {
    let plain = ResultType::plain(Term::Primitive(P::Bool), &mut 1000).unwrap();
    let narrow = ResultType::qualify(
        row(vec![label(1, false)]),
        plain.clone(),
        &mut 1000,
        &mut identity,
    )
    .unwrap();
    let wide = ResultType::qualify(
        row(vec![label(2, false), label(1, false)]),
        plain.clone(),
        &mut 1000,
        &mut identity,
    )
    .unwrap();
    assert_eq!(
        plain.transport_into(&plain, &mut 1000, &mut identity),
        Ok(Transport::Plain)
    );
    assert_eq!(
        plain.transport_into(&wide, &mut 1000, &mut identity),
        Ok(Transport::InjectSuccess)
    );
    let Transport::Embed(plan) = narrow
        .transport_into(&wide, &mut 1000, &mut identity)
        .unwrap()
    else {
        panic!("embedding")
    };
    assert_eq!(plan.map_error(0), Ok(1));
    assert_eq!(
        wide.transport_into(&narrow, &mut 1000, &mut identity),
        Err(Error::MissingLabel)
    );
    assert_eq!(
        narrow.transport_into(&plain, &mut 1000, &mut identity),
        Err(Error::MissingLabel)
    );
    let other = ResultType::plain(Term::Primitive(P::Char), &mut 1000).unwrap();
    assert_eq!(
        other.transport_into(&wide, &mut 1000, &mut identity),
        Err(Error::IncompatibleSuccess)
    );
    assert_eq!(
        Transport::InjectSuccess.to_string(),
        "(result-transport inject-success)"
    );
}

#[test]
fn exact_membership_reuses_validated_row_uniqueness() {
    let target = row(vec![label(1, false), label(2, false)]);
    let source = row(vec![label(2, false)]);
    let mut no_new_proof = |_: &Term, _: &Term, _: &mut usize| -> Result<Identity, Error> {
        panic!("exact membership must not request unrelated identity proofs")
    };
    assert_eq!(
        source
            .embedding_into(&target, &mut 10000, &mut no_new_proof)
            .unwrap()
            .map_error(0),
        Ok(1)
    );
    assert_eq!(
        target
            .subtract(&source, &mut 10000, &mut identity)
            .unwrap()
            .row()
            .labels(),
        &[label(1, false)]
    );
}

fn parameter(local: u32, sort: Sort) -> nir::DeclarationParameter {
    nir::DeclarationParameter {
        declaration: DeclarationId { package: 9, local },
        sort,
    }
}
fn rigid(parameter: nir::DeclarationParameter) -> Term {
    Term::Rigid {
        declaration: parameter.declaration,
        sort: parameter.sort,
    }
}
#[test]
fn generic_error_occurrences_get_new_tags_after_type_substitution() {
    use nir::error_results::ResultTemplate;
    let t = parameter(1, Sort::Type);
    let u = parameter(2, Sort::Type);
    let success = Term::NormalForm {
        head: DeclarationId {
            package: 9,
            local: 8,
        },
        sort: Sort::Type,
        arguments: vec![rigid(t)],
    };
    let template = ResultTemplate::new(
        vec![t, u],
        vec![rigid(t), rigid(u), rigid(t)],
        success,
        &mut 10000,
    )
    .unwrap();
    let bool_ty = Term::Primitive(P::Bool);
    let char_ty = Term::Primitive(P::Char);
    let merged = template
        .instantiate(
            &[bool_ty.clone(), bool_ty.clone()],
            &mut 10000,
            &mut identity,
        )
        .unwrap();
    assert_eq!(merged.result().errors().labels(), &[bool_ty.clone()]);
    for occurrence in 0..3 {
        assert_eq!(merged.occurrence_tag(occurrence), Ok(0));
    }
    let distinct = template
        .instantiate(
            &[bool_ty.clone(), char_ty.clone()],
            &mut 10000,
            &mut identity,
        )
        .unwrap();
    assert_eq!(
        distinct.result().errors().labels(),
        &[bool_ty.clone(), char_ty]
    );
    assert_eq!(distinct.occurrence_tag(0), Ok(0));
    assert_eq!(distinct.occurrence_tag(1), Ok(1));
    assert_eq!(distinct.occurrence_tag(2), Ok(0));
    assert_eq!(distinct.occurrence_tag(3), Err(Error::InvalidTag));
    assert!(
        matches!(distinct.result().success(),Term::NormalForm {arguments,..} if arguments==&vec![bool_ty])
    );
    assert!(
        template
            .to_string()
            .starts_with("(error-result-template (parameters")
    );
    assert!(
        distinct
            .to_string()
            .contains("(occurrences (error-embedding (tags 0 1 0)))")
    );
}
#[test]
fn value_indexed_error_instances_merge_only_after_completed_arguments() {
    use nir::error_results::ResultTemplate;
    let n = parameter(3, Sort::ComptimeValue);
    let indexed = Term::Nominal {
        declaration: DeclarationId {
            package: 1,
            local: 1,
        },
        arguments: vec![rigid(n)],
    };
    let template = ResultTemplate::new(
        vec![n],
        vec![indexed, label(1, true)],
        Term::Primitive(P::Bool),
        &mut 10000,
    )
    .unwrap();
    for (value, count) in [(false, 2), (true, 1)] {
        let result = template
            .instantiate(&[Term::Bool(value)], &mut 10000, &mut identity)
            .unwrap();
        assert_eq!(result.result().errors().labels().len(), count);
        assert_eq!(result.occurrence_tag(1), Ok(count - 1));
    }
}
#[test]
fn generic_error_instantiation_validates_unused_inputs_and_recovers_from_failure() {
    use nir::error_results::ResultTemplate;
    let t = parameter(1, Sort::Type);
    let template = ResultTemplate::new(
        vec![t],
        vec![label(1, false)],
        Term::Primitive(P::Bool),
        &mut 10000,
    )
    .unwrap();
    let original = template.clone();
    assert_eq!(
        template.instantiate(&[], &mut 10000, &mut identity),
        Err(Error::InvalidParameters)
    );
    assert_eq!(
        template.instantiate(&[Term::Bool(true)], &mut 10000, &mut identity),
        Err(Error::InvalidParameters)
    );
    assert_eq!(
        template.instantiate(&[Term::Primitive(P::Bool)], &mut 0, &mut identity),
        Err(Error::Budget)
    );
    assert_eq!(template, original);
    let arguments = [Term::Primitive(P::Bool)];
    let first = template
        .instantiate(&arguments, &mut 10000, &mut identity)
        .unwrap();
    assert_eq!(
        first.to_string(),
        template
            .instantiate(&arguments, &mut 10000, &mut identity)
            .unwrap()
            .to_string()
    );
    assert_eq!(
        ResultTemplate::new(vec![t, t], vec![], Term::Primitive(P::Bool), &mut 10000),
        Err(Error::InvalidParameters)
    );
    assert_eq!(
        ResultTemplate::new(vec![], vec![rigid(t)], Term::Primitive(P::Bool), &mut 10000),
        Err(Error::InvalidParameters)
    );
}

#[test]
fn error_terms_round_trip_through_checked_layouts_and_normalize_substitution() {
    let t = parameter(1, Sort::Type);
    let ty = Term::error_result(
        vec![rigid(t), Term::Primitive(P::Bool)],
        Term::error_result(vec![rigid(t)], Term::Primitive(P::Char)),
    );
    assert!(ty.mentions_rigid(t.declaration));
    let instantiated = ty
        .instantiate_parameters(&[t], &[Term::Primitive(P::Bool)])
        .unwrap();
    assert_eq!(
        instantiated,
        Term::ErrorResult {
            errors: vec![Term::Primitive(P::Bool)],
            result: Box::new(Term::Primitive(P::Char))
        }
    );
    let layout = ResultType::from_term(&instantiated, &mut 10000, &mut identity).unwrap();
    assert_eq!(layout.to_term(&mut 10000).unwrap(), instantiated);
    assert_eq!(
        instantiated.to_string(),
        "(error-result-type (errors (primitive bool)) (result (primitive char)))"
    );
    assert_eq!(
        Term::error_result(vec![], Term::Primitive(P::Bool)),
        Term::Primitive(P::Bool)
    );
}
#[test]
fn error_term_scope_dependencies_and_budget_include_labels_and_success() {
    let runtime = nir::RuntimeId {
        instance: 0,
        local: 31,
    };
    let ty = Term::error_result(
        vec![Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            arguments: vec![Term::Runtime(runtime)],
        }],
        Term::Primitive(P::Bool),
    );
    assert!(ty.runtime_dependencies().contains(&runtime));
    let malformed = Term::ErrorResult {
        errors: vec![Term::Bool(true)],
        result: Box::new(Term::Primitive(P::Bool)),
    };
    assert!(malformed.check_scope(&[]).is_err());
    let open = Term::ErrorResult {
        errors: vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }],
        result: Box::new(Term::Primitive(P::Bool)),
    };
    assert!(open.check_scope(&[]).is_err());
    assert!(open.check_scope(&[vec![Sort::Type]]).is_ok());
    let mut deep = Term::Primitive(P::Bool);
    for _ in 0..140 {
        deep = Term::Pointer(Box::new(deep));
    }
    let deep = Term::error_result(vec![deep], Term::Primitive(P::Bool));
    assert!(nir::type_computations::charge_term(&deep, &mut 10000).is_err());
    deep.drop_iteratively();
}

#[test]
fn qualified_success_terms_cannot_hide_an_extra_error_layer() {
    use nir::error_results::ResultTemplate;
    let success = Term::error_result(vec![label(1, false)], Term::Primitive(P::Bool));
    assert_eq!(
        ResultType::plain(success.clone(), &mut 10000),
        Err(Error::InvalidType)
    );
    let template = ResultTemplate::new(
        vec![],
        vec![label(2, false), label(1, false)],
        success,
        &mut 10000,
    )
    .unwrap();
    let instance = template
        .instantiate(&[], &mut 10000, &mut identity)
        .unwrap();
    assert_eq!(
        instance.result().errors().labels(),
        &[label(2, false), label(1, false)]
    );
    assert_eq!(instance.result().success(), &Term::Primitive(P::Bool));
    assert_eq!(instance.occurrence_tag(0), Ok(0));
    assert_eq!(instance.occurrence_tag(1), Ok(1));
}

#[test]
fn executable_conversions_require_payload_identity_and_positive_membership() {
    use nir::error_results::{Conversion as C, validate_conversion};
    let boolean = Term::Primitive(nir::Primitive::Bool);
    let byte = Term::Primitive(nir::Primitive::U8);
    let target = Term::error_result(vec![byte.clone(), boolean.clone()], boolean.clone());
    let mut equal = |a: &Term, b: &Term| a == b;
    for (kind, input) in [
        (C::Success, boolean.clone()),
        (C::Failure, byte.clone()),
        (
            C::Embed,
            Term::error_result(vec![boolean.clone()], boolean.clone()),
        ),
        (
            C::Embed,
            Term::error_result(
                vec![boolean.clone()],
                Term::Primitive(nir::Primitive::Never),
            ),
        ),
    ] {
        assert!(validate_conversion(kind, &input, &target, &mut 1000, &mut equal).is_ok());
        assert_eq!(
            validate_conversion(kind, &input, &target, &mut 0, &mut equal),
            Err(Error::Budget)
        );
        assert!(validate_conversion(kind, &input, &target, &mut 1000, &mut equal).is_ok());
    }
    assert_eq!(
        validate_conversion(C::Success, &byte, &target, &mut 1000, &mut equal),
        Err(Error::IncompatibleSuccess)
    );
    assert_eq!(
        validate_conversion(C::Success, &target, &target, &mut 1000, &mut equal),
        Err(Error::InvalidType)
    );
    let absent = Term::Primitive(nir::Primitive::Char);
    assert_eq!(
        validate_conversion(C::Failure, &absent, &target, &mut 1000, &mut equal),
        Err(Error::MissingLabel)
    );
    assert_eq!(
        validate_conversion(
            C::Embed,
            &Term::error_result(vec![absent], boolean.clone()),
            &target,
            &mut 1000,
            &mut equal
        ),
        Err(Error::MissingLabel)
    );
    assert_eq!(
        validate_conversion(C::Failure, &boolean, &boolean, &mut 1000, &mut equal),
        Err(Error::InvalidType)
    );
}

#[test]
fn product_transport_maps_symbolic_paths_and_physical_guards() {
    use nir::error_results::transport_plan_with_layouts;
    let boolean = Term::Primitive(nir::Primitive::Bool);
    let character = Term::Primitive(nir::Primitive::Char);
    let input = Term::Tuple(vec![
        physical_result(vec![boolean.clone(), character.clone()], boolean.clone()),
        boolean.clone(),
    ]);
    let output = Term::Tuple(vec![
        physical_result(vec![character, boolean.clone()], boolean.clone()),
        boolean,
    ]);
    let plan = transport_plan_with_layouts(
        &input,
        &output,
        true,
        &mut 10000,
        &mut |a, b| a == b,
        &|_| None,
    )
    .unwrap();
    let mut path = vec![0, usize::MAX, 1, 0];
    plan.remap_path(&mut path, &mut 100).unwrap();
    assert_eq!(path, vec![0, usize::MAX, 2, 0]);
    assert_eq!(plan.remap_tag(&[0], 1, &mut 100).unwrap(), 2);
    assert!(plan.inverse_path(&mut path, &mut 100).unwrap());
    assert_eq!(path, vec![0, usize::MAX, 1, 0]);
    assert!(plan.to_string().starts_with("(tuple-layout (error-layout"));
    assert!(
        transport_plan_with_layouts(&input, &output, true, &mut 0, &mut |a, b| a == b, &|_| None)
            .is_err()
    );
}

#[test]
fn product_transport_rejects_row_widening_and_unregistered_tuple_heads() {
    use nir::error_results::transport_plan_with_layouts;
    let boolean = Term::Primitive(nir::Primitive::Bool);
    let character = Term::Primitive(nir::Primitive::Char);
    let a = physical_result(vec![boolean.clone()], boolean.clone());
    let b = physical_result(vec![boolean.clone(), character.clone()], boolean.clone());
    assert!(
        transport_plan_with_layouts(
            &Term::Tuple(vec![a]),
            &Term::Tuple(vec![b]),
            true,
            &mut 10000,
            &mut |a, b| a == b,
            &|_| None
        )
        .is_err()
    );
    let head = nir::DeclarationId {
        package: 9,
        local: 81,
    };
    let metadata = tuple_layout(head);
    let tuple = |row| Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![tuple_input(&metadata, vec![row])],
    };
    let input = tuple(physical_result(
        vec![boolean.clone(), character.clone()],
        boolean.clone(),
    ));
    let output = tuple(physical_result(vec![character, boolean.clone()], boolean));
    assert!(
        transport_plan_with_layouts(
            &input,
            &output,
            true,
            &mut 10000,
            &mut |a, b| a == b,
            &|_| None
        )
        .is_err()
    );
    assert!(
        transport_plan_with_layouts(
            &input,
            &output,
            true,
            &mut 10000,
            &mut |a, b| a == b,
            &|ty| metadata.tuple_elements(ty)
        )
        .is_ok()
    );
}

#[test]
fn frozen_error_substitution_tracks_full_labels_and_hidden_scope() {
    let id = DeclarationId {
        package: 1,
        local: 100,
    };
    let variable = Term::Rigid {
        declaration: id,
        sort: Sort::Type,
    };
    let term = Term::ErrorValue {
        ty: Box::new(Term::error_result(
            vec![variable.clone()],
            Term::Primitive(P::Bool),
        )),
        label: Some(Box::new(variable.clone())),
        value: Box::new(Term::Bool(true)),
    };
    let frozen = term
        .instantiate_parameters(
            &[nir::DeclarationParameter {
                declaration: id,
                sort: Sort::Type,
            }],
            &[Term::Primitive(P::Bool)],
        )
        .unwrap();
    frozen.check_scope(&[]).unwrap();
    assert!(
        frozen
            .to_string()
            .contains("(branch (error (primitive bool)))")
    );
    assert!(nir::type_computations::charge_term(&frozen, &mut 1).is_err());
    let bad = Term::ErrorValue {
        ty: Box::new(Term::error_result(
            vec![Term::Primitive(P::Bool)],
            Term::Primitive(P::Bool),
        )),
        label: Some(Box::new(Term::Primitive(P::Char))),
        value: Box::new(Term::Char('a')),
    };
    assert!(bad.check_scope(&[]).is_err());
    let hidden = Term::ErrorValue {
        ty: Box::new(Term::error_result(
            vec![Term::Primitive(P::Bool)],
            Term::Primitive(P::Bool),
        )),
        label: None,
        value: Box::new(Term::Hole {
            index: 0,
            sort: Sort::ComptimeValue,
        }),
    };
    assert!(hidden.check_scope(&[]).is_err());
}

#[test]
fn frozen_success_substitution_flattens_newly_exposed_error_branch() {
    let t = DeclarationId {
        package: 1,
        local: 101,
    };
    let v = DeclarationId {
        package: 1,
        local: 102,
    };
    let term = Term::ErrorValue {
        ty: Box::new(Term::error_result(
            vec![Term::Primitive(P::Char)],
            Term::Rigid {
                declaration: t,
                sort: Sort::Type,
            },
        )),
        label: None,
        value: Box::new(Term::Rigid {
            declaration: v,
            sort: Sort::ComptimeValue,
        }),
    };
    let inner = Term::error_result(vec![Term::Primitive(P::Bool)], Term::Primitive(P::Bool));
    let result = term
        .instantiate_parameters(
            &[
                nir::DeclarationParameter {
                    declaration: t,
                    sort: Sort::Type,
                },
                nir::DeclarationParameter {
                    declaration: v,
                    sort: Sort::ComptimeValue,
                },
            ],
            &[
                inner.clone(),
                Term::ErrorValue {
                    ty: Box::new(inner),
                    label: Some(Box::new(Term::Primitive(P::Bool))),
                    value: Box::new(Term::Bool(false)),
                },
            ],
        )
        .unwrap();
    result.check_scope(&[]).unwrap();
    assert_eq!(
        result,
        Term::ErrorValue {
            ty: Box::new(Term::error_result(
                vec![Term::Primitive(P::Char), Term::Primitive(P::Bool)],
                Term::Primitive(P::Bool)
            )),
            label: Some(Box::new(Term::Primitive(P::Bool))),
            value: Box::new(Term::Bool(false)),
        }
    );
}

#[test]
fn frozen_product_transport_preserves_fields_and_rejects_wrong_source_domain() {
    let a = physical_result(
        vec![Term::Primitive(P::Bool), Term::Primitive(P::Char)],
        Term::Primitive(P::Bool),
    );
    let b = physical_result(
        vec![Term::Primitive(P::Char), Term::Primitive(P::Bool)],
        Term::Primitive(P::Bool),
    );
    let record = |ty| {
        Term::Record(vec![
            ("result".into(), ty),
            ("flag".into(), Term::Primitive(P::Bool)),
        ])
    };
    let from = Term::Tuple(vec![record(a.clone())]);
    let to = Term::Tuple(vec![record(b.clone())]);
    let plan = nir::error_results::transport_plan_with_layouts(
        &from,
        &to,
        true,
        &mut 10000,
        &mut |a, b| a == b,
        &|_| None,
    )
    .unwrap();
    let value = |ty| {
        Term::TupleValue(vec![Term::RecordValue(vec![
            (
                "result".into(),
                Term::ErrorValue {
                    ty: Box::new(ty),
                    label: Some(Box::new(Term::Primitive(P::Bool))),
                    value: Box::new(Term::Bool(true)),
                },
            ),
            ("flag".into(), Term::Bool(false)),
        ])])
    };
    let original = value(a);
    let expected = value(b);
    assert_eq!(plan.frozen(&original, &mut 10000).unwrap(), expected);
    assert!(plan.frozen(&expected, &mut 10000).is_err());
    assert!(plan.frozen(&original, &mut 0).is_err());
    assert_eq!(plan.frozen(&original, &mut 10000).unwrap(), expected);
}

#[test]
fn frozen_success_rebuilding_requires_matching_inner_domain() {
    let boolean = Term::Primitive(P::Bool);
    let inner_type = Term::error_result(vec![boolean.clone()], boolean.clone());
    let inner = Term::ErrorValue {
        ty: Box::new(inner_type.clone()),
        label: Some(Box::new(boolean.clone())),
        value: Box::new(Term::Bool(true)),
    };
    let invalid = Term::ErrorValue {
        ty: Box::new(Term::error_result(
            vec![Term::Primitive(P::Char)],
            boolean.clone(),
        )),
        label: None,
        value: Box::new(inner.clone()),
    };
    assert_eq!(
        invalid.instantiate_parameters(&[], &[]),
        Err(nir::TypeError::InvalidErrorValue)
    );
    // An error whose label is itself an error-result type keeps that payload;
    // only the success qualification participates in flattening.
    let failure = Term::ErrorValue {
        ty: Box::new(Term::error_result(vec![inner_type.clone()], boolean)),
        label: Some(Box::new(inner_type)),
        value: Box::new(inner),
    };
    assert_eq!(failure.instantiate_parameters(&[], &[]).unwrap(), failure);
}

#[test]
fn unresolved_inner_error_branch_survives_partial_then_complete_substitution() {
    let t = parameter(310, Sort::Type);
    let v = parameter(311, Sort::ComptimeValue);
    let boolean = Term::Primitive(P::Bool);
    let inner_type = Term::error_result(vec![boolean.clone()], boolean.clone());
    let original = Term::ErrorValue {
        ty: Box::new(Term::error_result(vec![Term::Primitive(P::Char)], rigid(t))),
        label: None,
        value: Box::new(rigid(v)),
    };
    assert!(original.has_pending_error_branch());
    let partial = original
        .instantiate_parameters(&[t], std::slice::from_ref(&inner_type))
        .unwrap();
    assert!(partial.has_pending_error_branch());
    assert!(partial.to_string().contains("(branch pending-inner)"));
    let Term::ErrorValue { ty, value, .. } = &partial else {
        unreachable!()
    };
    assert_eq!(value.as_ref(), &rigid(v));
    assert!(
        matches!(ty.as_ref(), Term::ErrorResult { result, .. } if result.as_ref() == &inner_type)
    );
    for label in [None, Some(Box::new(boolean.clone()))] {
        let completed = Term::ErrorValue {
            ty: Box::new(inner_type.clone()),
            label: label.clone(),
            value: Box::new(Term::Bool(true)),
        };
        let sequential = partial
            .instantiate_parameters(&[v], std::slice::from_ref(&completed))
            .unwrap();
        let simultaneous = original
            .instantiate_parameters(&[t, v], &[inner_type.clone(), completed])
            .unwrap();
        assert_eq!(sequential, simultaneous);
        assert!(!sequential.has_pending_error_branch());
        assert!(!partial.index_values_definitely_differ(&sequential, &mut 10000));
        assert!(matches!(sequential, Term::ErrorValue { label: actual, .. } if actual == label));
    }
}

#[test]
fn nested_pending_error_wrappers_complete_without_losing_the_inner_domain() {
    let v = parameter(320, Sort::ComptimeValue);
    let boolean = Term::Primitive(P::Bool);
    let leaf = Term::error_result(vec![boolean.clone()], boolean.clone());
    let inner =
        Term::error_value(vec![Term::Primitive(P::Char)], leaf.clone(), None, rigid(v)).unwrap();
    let Term::ErrorValue { ty: inner_type, .. } = &inner else {
        unreachable!()
    };
    let outer = Term::error_value(
        vec![Term::Primitive(P::U8)],
        inner_type.as_ref().clone(),
        None,
        inner.clone(),
    )
    .unwrap();
    let Term::ErrorValue { value, .. } = &outer else {
        unreachable!()
    };
    assert_eq!(value.as_ref(), &inner);
    let complete = Term::ErrorValue {
        ty: Box::new(leaf),
        label: Some(Box::new(boolean.clone())),
        value: Box::new(Term::Bool(false)),
    };
    let still_pending = outer.instantiate_parameters(&[], &[]).unwrap();
    assert!(still_pending.has_pending_error_branch());
    let result = outer
        .instantiate_parameters(&[v], std::slice::from_ref(&complete))
        .unwrap();
    assert_eq!(
        still_pending
            .instantiate_parameters(&[v], &[complete])
            .unwrap(),
        result
    );
    assert_eq!(
        result,
        Term::ErrorValue {
            ty: Box::new(Term::error_result(
                vec![
                    Term::Primitive(P::U8),
                    Term::Primitive(P::Char),
                    boolean.clone()
                ],
                boolean.clone()
            )),
            label: Some(Box::new(boolean)),
            value: Box::new(Term::Bool(false)),
        }
    );
}

#[test]
fn expression_specialization_reclassifies_only_newly_exposed_error_domains() {
    use nir::error_results::{Conversion, validate_conversion};
    use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
    let t = parameter(400, Sort::Type);
    let target = Term::error_result(vec![Term::Primitive(P::Char)], rigid(t));
    let input = Expr {
        ty: target,
        source: SourceRange { start: 1, end: 2 },
        kind: K::ErrorConvert {
            conversion: Conversion::Success,
            value: Box::new(Expr {
                ty: rigid(t),
                source: SourceRange { start: 3, end: 4 },
                kind: K::Local(LocalId { owner: 0, index: 0 }),
            }),
        },
    };
    for ty in [
        Term::Primitive(P::Bool),
        Term::error_result(vec![Term::Primitive(P::Bool)], Term::Primitive(P::Bool)),
    ] {
        let result = input
            .instantiate_parameters(&[t], std::slice::from_ref(&ty))
            .unwrap();
        let K::ErrorConvert { conversion, value } = &result.kind else {
            unreachable!()
        };
        assert_eq!(
            *conversion,
            if matches!(ty, Term::ErrorResult { .. }) {
                Conversion::Embed
            } else {
                Conversion::Success
            }
        );
        assert_eq!(value.source.start, 3);
        validate_conversion(
            *conversion,
            &value.ty,
            &result.ty,
            &mut 10000,
            &mut |a, b| a == b,
        )
        .unwrap();
        if *conversion == Conversion::Embed {
            let malformed = Expr {
                kind: K::ErrorConvert {
                    conversion: Conversion::Success,
                    value: value.clone(),
                },
                ..result
            };
            let unchanged = malformed.instantiate_parameters(&[], &[]).unwrap();
            let K::ErrorConvert { conversion, value } = unchanged.kind else {
                unreachable!()
            };
            assert_eq!(conversion, Conversion::Success);
            assert!(
                validate_conversion(
                    conversion,
                    &value.ty,
                    &unchanged.ty,
                    &mut 10000,
                    &mut |a, b| a == b
                )
                .is_err()
            );
        }
    }
}

#[test]
fn success_projection_completion_supports_staged_substitution_and_distinct_origins() {
    use nir::error_results::{Conversion, validate_conversion};
    use nir::program::{Expr, ExprKind as K, LocalId, MatchArm, Pattern, SourceRange};
    let t = parameter(410, Sort::Type);
    let u = parameter(411, Sort::Type);
    let char_ty = Term::Primitive(P::Char);
    let qualified = Term::error_result(vec![char_ty.clone()], rigid(t));
    let local = |index, ty| Expr {
        source: SourceRange {
            start: index * 2,
            end: index * 2 + 1,
        },
        ty,
        kind: K::Local(LocalId { owner: 0, index }),
    };
    let binding = |index| {
        Box::new(Pattern::Binding {
            binding: LocalId { owner: 0, index },
            inner: Box::new(Pattern::Wild),
        })
    };
    let template = Expr {
        source: SourceRange { start: 20, end: 21 },
        ty: rigid(t),
        kind: K::Match {
            value: Box::new(local(0, qualified.clone())),
            arms: vec![
                MatchArm {
                    pattern: Pattern::ErrorResult {
                        label: None,
                        inner: binding(1),
                    },
                    guard: None,
                    body: local(1, rigid(t)),
                },
                MatchArm {
                    pattern: Pattern::ErrorResult {
                        label: Some(char_ty.clone()),
                        inner: binding(2),
                    },
                    guard: None,
                    body: Expr {
                        source: SourceRange { start: 10, end: 11 },
                        ty: Term::Primitive(P::Never),
                        kind: K::Return(Some(Box::new(Expr {
                            source: SourceRange { start: 12, end: 13 },
                            ty: qualified,
                            kind: K::ErrorConvert {
                                conversion: Conversion::Failure,
                                value: Box::new(local(2, char_ty)),
                            },
                        }))),
                    },
                },
            ],
        },
    };
    let first = template
        .instantiate_parameters(
            &[t],
            &[Term::error_result(vec![Term::Primitive(P::Bool)], rigid(u))],
        )
        .unwrap();
    let mut completed = first
        .instantiate_parameters(
            &[u],
            &[Term::error_result(
                vec![Term::Primitive(P::U8)],
                Term::Primitive(P::Bool),
            )],
        )
        .unwrap();
    let K::Match { arms, .. } = &completed.kind else {
        unreachable!()
    };
    assert_eq!(arms.len(), 4);
    assert_eq!(
        completed.instantiate_parameters(&[], &[]).unwrap(),
        completed
    );
    nir::visit::expression_mut(&mut completed, &mut 10000, |node, _| {
        if let K::ErrorConvert { conversion, value } = &node.kind {
            validate_conversion(*conversion, &value.ty, &node.ty, &mut 10000, &mut |a, b| {
                a == b
            })
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .unwrap();
    let mut sources = vec![];
    nir::error_results::specialize::anchor_completed_projections(
        &mut completed,
        &mut sources,
        &mut 10000,
    )
    .unwrap();
    assert!(!sources.is_empty());
    for index in 0..sources.len() {
        assert!(
            SourceRange::generated(index as u32)
                .original(&sources)
                .is_some()
        );
    }
    let snapshot = (completed.clone(), sources.clone());
    nir::error_results::specialize::anchor_completed_projections(
        &mut completed,
        &mut sources,
        &mut 10000,
    )
    .unwrap();
    assert_eq!((completed, sources), snapshot);
    assert!(
        nir::error_results::specialize::anchor_completed_projections(
            &mut template.clone(),
            &mut vec![],
            &mut 0
        )
        .is_err()
    );
}

#[test]
fn ignored_success_completion_preserves_guards_and_error_payload_domains() {
    use nir::program::{Expr, ExprKind as K, Literal, LocalId, MatchArm, Pattern, SourceRange};
    let t = parameter(420, Sort::Type);
    let u = parameter(421, Sort::Type);
    let boolean = Term::Primitive(P::Bool);
    let nested_label = Term::error_result(vec![boolean.clone()], boolean.clone());
    let node = |index, ty, kind| Expr {
        source: SourceRange {
            start: index,
            end: index + 1,
        },
        ty,
        kind,
    };
    let success = || Pattern::ErrorResult {
        label: None,
        inner: Box::new(Pattern::Wild),
    };
    let unrelated_payload = Pattern::ErrorResult {
        label: Some(nested_label.clone()),
        inner: Box::new(success()),
    };
    let template = node(
        0,
        boolean.clone(),
        K::Match {
            value: Box::new(node(
                1,
                Term::error_result(vec![nested_label], rigid(t)),
                K::Local(LocalId { owner: 0, index: 0 }),
            )),
            arms: vec![
                MatchArm {
                    pattern: Pattern::Binding {
                        binding: LocalId { owner: 0, index: 1 },
                        inner: Box::new(Pattern::Guard {
                            pattern: Box::new(Pattern::Or(vec![
                                success(),
                                unrelated_payload.clone(),
                            ])),
                            condition: Box::new(node(
                                10,
                                boolean.clone(),
                                K::Literal(Literal::Bool(false)),
                            )),
                        }),
                    },
                    guard: None,
                    body: node(11, boolean.clone(), K::Literal(Literal::Bool(true))),
                },
                MatchArm {
                    pattern: Pattern::Wild,
                    guard: None,
                    body: node(12, boolean.clone(), K::Literal(Literal::Bool(false))),
                },
            ],
        },
    );
    let first = template
        .instantiate_parameters(
            &[t],
            &[Term::error_result(vec![Term::Primitive(P::Char)], rigid(u))],
        )
        .unwrap();
    let completed = first
        .instantiate_parameters(
            &[u],
            &[Term::error_result(vec![Term::Primitive(P::U8)], boolean)],
        )
        .unwrap();
    let K::Match { arms, .. } = &completed.kind else {
        unreachable!()
    };
    let Pattern::Binding { inner, .. } = &arms[0].pattern else {
        unreachable!()
    };
    let Pattern::Guard { pattern, .. } = inner.as_ref() else {
        unreachable!()
    };
    let Pattern::Or(alternatives) = pattern.as_ref() else {
        unreachable!()
    };
    assert_eq!(alternatives.len(), 2);
    assert_eq!(alternatives[1], unrelated_payload);
    let dump = alternatives[0].to_string();
    assert!(dump.contains("char") && dump.contains("u8"), "{dump}");
    let mut guards = 0;
    nir::visit::expression(&completed, &mut 10000, |node, _| {
        if node.source.start == 10 {
            guards += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(guards, 1);
    assert_eq!(
        completed.instantiate_parameters(&[], &[]).unwrap(),
        completed
    );
}

#[test]
fn computed_pattern_completion_uses_its_own_subject_domain() {
    use nir::program::{Expr, ExprKind, Literal, Pattern, SourceRange};
    let t = parameter(430, Sort::Type);
    let boolean = Term::Primitive(P::Bool);
    let success = || Pattern::ErrorResult {
        label: None,
        inner: Box::new(Pattern::Wild),
    };
    let computed = Term::error_result(vec![Term::Primitive(P::Char)], rigid(t));
    let mut pattern = Pattern::Then {
        first: Box::new(success()),
        value: Box::new(Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: computed,
            kind: ExprKind::Literal(Literal::Bool(true)),
        }),
        next: Box::new(success()),
    };
    // This is a type-rewrite fixture, not an executable expression. The first
    // subject is unknown; only the computed expression supplies a domain.
    pattern
        .rewrite_types(&mut |ty: &Term| -> Result<Term, ()> {
            Ok(ty
                .instantiate_parameters(
                    &[t],
                    &[Term::error_result(
                        vec![Term::Primitive(P::U8)],
                        boolean.clone(),
                    )],
                )
                .unwrap())
        })
        .unwrap();
    let Pattern::Then { first, next, .. } = &pattern else {
        unreachable!()
    };
    assert_eq!(first.as_ref(), &success());
    let Pattern::Or(alternatives) = next.as_ref() else {
        panic!("{next}")
    };
    assert_eq!(
        alternatives,
        &vec![
            success(),
            Pattern::ErrorResult {
                label: Some(Term::Primitive(P::U8)),
                inner: Box::new(Pattern::Wild),
            }
        ]
    );
}

#[test]
fn constructed_error_rows_have_stable_receiver_layout_after_substitution() {
    let a = DeclarationId {
        package: 91,
        local: 1,
    };
    let b = DeclarationId {
        package: 91,
        local: 2,
    };
    let rigid = |id| Term::Rigid {
        declaration: id,
        sort: Sort::Type,
    };
    let boolean = Term::Primitive(P::Bool);
    let character = Term::Primitive(P::Char);
    let left = Term::error_result(vec![rigid(a), rigid(b)], boolean.clone());
    let right = Term::error_result(vec![rigid(b), rigid(a), rigid(a)], boolean.clone());
    assert_eq!(left, right);
    let instantiated = left
        .instantiate_rigids(&[a, b], &[character.clone(), boolean.clone()])
        .unwrap();
    assert_eq!(
        instantiated,
        Term::error_result(vec![boolean.clone(), character], boolean.clone())
    );
    let qualified_label = Term::error_result(vec![boolean.clone()], Term::Primitive(P::Char));
    let head = Term::NormalForm {
        head: DeclarationId {
            package: 91,
            local: 3,
        },
        sort: Sort::Type,
        arguments: vec![boolean.clone()],
    };
    let complete = Term::error_result(vec![qualified_label.clone(), head.clone()], boolean.clone());
    assert_eq!(
        complete,
        Term::error_result(vec![head.clone(), qualified_label.clone()], boolean)
    );
    let Term::ErrorResult { errors, .. } = complete else {
        panic!("result")
    };
    assert_eq!(errors.len(), 2);
    assert!(errors.contains(&qualified_label) && errors.contains(&head));
}

#[test]
fn physical_result_layout_round_trip_preserves_noncanonical_tags() {
    let physical = physical_result(
        vec![Term::Primitive(P::Char), Term::Primitive(P::Bool)],
        Term::Primitive(P::Bool),
    );
    let checked = ResultType::from_term(&physical, &mut 10000, &mut identity).unwrap();
    assert_eq!(checked.to_term(&mut 10000).unwrap(), physical);
    assert_ne!(
        physical,
        Term::error_result(
            vec![Term::Primitive(P::Char), Term::Primitive(P::Bool)],
            Term::Primitive(P::Bool)
        )
    );
}

#[test]
fn completed_identity_pattern_matches_subject_without_acquiring_another_owner() {
    use nir::program::{Expr, ExprKind, LocalId, Pattern, SourceRange};
    let t = parameter(431, Sort::Type);
    let alias = LocalId {
        owner: 431,
        index: 0,
    };
    let make = || Pattern::Then {
        first: Box::new(Pattern::Observe {
            binding: alias,
            inner: Box::new(Pattern::Wild),
        }),
        value: Box::new(Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::error_result(vec![Term::Primitive(P::Char)], rigid(t)),
            kind: ExprKind::Local(alias),
        }),
        next: Box::new(Pattern::ErrorResult {
            label: None,
            inner: Box::new(Pattern::Wild),
        }),
    };
    let complete = |ty: &Term| -> Result<Term, ()> {
        Ok(ty
            .instantiate_parameters(
                &[t],
                &[Term::error_result(
                    vec![Term::Primitive(P::U8)],
                    Term::Primitive(P::Bool),
                )],
            )
            .unwrap())
    };
    let mut pattern = make();
    pattern
        .rewrite_types(&mut |ty| Ok::<_, ()>(ty.clone()))
        .unwrap();
    assert!(
        matches!(pattern, Pattern::Then { .. }),
        "retain the abstract domain until completion"
    );
    pattern.rewrite_types(&mut { complete }).unwrap();
    let Pattern::Or(alternatives) = &pattern else {
        panic!("{pattern}")
    };
    assert_eq!(alternatives.len(), 2);
    assert!(matches!(
        &alternatives[1],
        Pattern::ErrorResult {
            label: Some(Term::Primitive(P::U8)),
            ..
        }
    ));
    let before = pattern.clone();
    pattern.rewrite_types(&mut { complete }).unwrap();
    assert_eq!(pattern, before);

    // A normal-form success type stays opaque; it will not unfold into an
    // error result, so it also needs no temporary ownership observation.
    let mut opaque = make();
    opaque
        .rewrite_types(&mut |ty: &Term| -> Result<Term, ()> {
            Ok(ty
                .instantiate_parameters(
                    &[t],
                    &[Term::NormalForm {
                        head: nir::DeclarationId {
                            package: 91,
                            local: 432,
                        },
                        sort: Sort::Type,
                        arguments: vec![],
                    }],
                )
                .unwrap())
        })
        .unwrap();
    assert!(matches!(opaque, Pattern::ErrorResult { label: None, .. }));

    // A real computation or a later read of the observation alias cannot be
    // discarded by the identity simplification.
    for uses_alias in [false, true] {
        let mut pattern = make();
        let Pattern::Then { value, next, .. } = &mut pattern else {
            unreachable!()
        };
        if uses_alias {
            *next = Box::new(Pattern::Guard {
                pattern: next.clone(),
                condition: value.clone(),
            });
        } else {
            value.kind = ExprKind::Local(LocalId {
                owner: 431,
                index: 1,
            });
        }
        pattern.rewrite_types(&mut { complete }).unwrap();
        assert!(matches!(pattern, Pattern::Then { .. }));
    }
}
