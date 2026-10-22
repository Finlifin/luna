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
// These fixtures are closed declared nominal instances with distinct boolean
// indices. This is fixture evidence, not a structural-inequality type solver.
fn identity(_: &Term, _: &Term, _: &mut usize) -> Result<Identity, Error> {
    Ok(Identity::Distinct)
}
fn row(labels: Vec<Term>) -> Row {
    Row::new(labels, &mut 10000, &mut identity).unwrap()
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
