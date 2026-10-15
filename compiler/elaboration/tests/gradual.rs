use elaboration::gradual::{self, Compatibility as C, Description as D, Join};
use elaboration::ptc::{Budget, Context, Formula, Outcome};
use elaboration::{InferenceContext, InferenceTerm as I};
use nir::{DeclarationId, Primitive, Sort, Term};
fn known(ty: Term) -> D {
    D::Known(I::Stable(ty))
}
fn boolean() -> D {
    known(Term::Primitive(Primitive::Bool))
}
fn byte() -> D {
    known(Term::Primitive(Primitive::U8))
}

#[test]
fn any_compatibility_does_not_create_type_equality_or_mutate_inference() {
    let inference = InferenceContext::default();
    let types = Context::default();
    let before = types.to_string();
    for (left, right) in [(D::Any, boolean()), (byte(), D::Any), (D::Any, D::Any)] {
        let result =
            gradual::compatible(&left, &right, &inference, &types, &mut Budget::default()).unwrap();
        assert!(matches!(result, C::Gradual));
        assert_eq!(result.to_string(), "(gradual-compatibility)");
    }
    assert!(matches!(
        gradual::compatible(
            &boolean(),
            &byte(),
            &inference,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::NotEntailed(_))
    ));
    assert!(matches!(
        types.prove(
            Formula::equal(
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::U8)
            ),
            Budget::default()
        ),
        Outcome::NotEntailed(_)
    ));
    assert_eq!(types.to_string(), before);
}

#[test]
fn signature_descriptions_exist_before_invocation_and_keep_declaration_scope() {
    use gradual::{FrozenDescription as F, SignatureDescription};
    use nir::deferred_signatures::{Annotation as A, Input, Signature};
    use nir::type_computations::{Input as Argument, Invocation};
    let scope = vec![Sort::Type, Sort::Runtime, Sort::Runtime];
    let residual = Invocation::new(
        DeclarationId {
            package: 44,
            local: 9,
        },
        scope,
        vec![Argument {
            ty: Term::Primitive(Primitive::Type),
            value: Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            },
        }],
        &mut 1000,
    )
    .unwrap();
    let callable = |result| Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: Term::Primitive(Primitive::Bool),
        }],
        result: Box::new(result),
    };
    let nested = callable(Term::Bound {
        depth: 1,
        index: 0,
        sort: Sort::Type,
    });
    let signature = Signature::new(
        vec![
            Input {
                sort: Sort::Type,
                annotation: A::Known(Term::Primitive(Primitive::Type)),
            },
            Input {
                sort: Sort::Runtime,
                annotation: A::Computed(0),
            },
            Input {
                sort: Sort::Runtime,
                annotation: A::Known(nested.clone()),
            },
        ],
        A::Computed(0),
        vec![residual],
        &mut 1000,
    )
    .unwrap();
    let before = signature.to_string();
    let description = SignatureDescription::from_residual(&signature, &mut 1000).unwrap();
    assert_eq!(
        description.parameters(),
        &[
            (Sort::Type, F::Known(Term::Primitive(Primitive::Type))),
            (Sort::Runtime, F::Any),
            (Sort::Runtime, F::Known(nested)),
        ]
    );
    assert_eq!(description.result(), &F::Any);
    assert_eq!(description.annotation(3), Some(description.result()));
    assert_eq!(description.annotation(4), None);
    assert_eq!(
        description
            .to_string()
            .matches("(any-type-description)")
            .count(),
        2
    );
    assert!(
        description
            .to_string()
            .starts_with("(signature-description (parameters")
    );
    let inference = InferenceContext::default();
    let types = Context::default();
    assert!(
        matches!(
            gradual::compatible(
                &description.annotation(1).unwrap().description(),
                &description.result().description(),
                &inference,
                &types,
                &mut Budget::default()
            )
            .unwrap(),
            C::Gradual
        ),
        "shared residual identity must not imply equality in ordinary checking"
    );
    let instantiated = description
        .annotation(2)
        .unwrap()
        .instantiate(vec![I::Stable(Term::Primitive(Primitive::U8))]);
    assert!(matches!(
        gradual::compatible(
            &instantiated,
            &known(callable(Term::Primitive(Primitive::U8))),
            &inference,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::Entailed(_))
    ));
    assert!(matches!(
        SignatureDescription::from_residual(&signature, &mut 0),
        Err(gradual::Error::Budget)
    ));
    assert_eq!(signature.to_string(), before);
    assert_eq!(
        SignatureDescription::from_residual(&signature, &mut 1000).unwrap(),
        description
    );
    assert!(matches!(
        F::from_annotation(&A::Known(Term::Bool(true)), &mut 100),
        Err(gradual::Error::Scope(_))
    ));
}

#[test]
fn inference_waiting_is_not_gradual_information_loss() {
    let mut inference = InferenceContext::default();
    let types = Context::default();
    let original = inference.fresh(Sort::Type);
    let alias = inference.fresh(Sort::Type);
    inference.assign(original, I::Meta(alias)).unwrap();
    let value = D::Known(I::Meta(original));
    assert!(
        matches!(gradual::compatible(&value,&D::Any,&inference,&types,&mut Budget::default()).unwrap(),C::Waiting(ids) if ids.iter().copied().collect::<Vec<_>>()==vec![alias])
    );
    assert!(matches!(
        gradual::join_without_target(&value, &byte(), &inference, &types, &mut Budget::default())
            .unwrap(),
        Join::Waiting(_)
    ));
    assert!(inference.freeze(&I::Meta(alias)).is_err());
    inference
        .assign(alias, I::Stable(Term::Primitive(Primitive::Bool)))
        .unwrap();
    assert!(matches!(
        gradual::compatible(
            &value,
            &boolean(),
            &inference,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::Entailed(_))
    ));
    assert!(matches!(
        gradual::join_without_target(&value, &byte(), &inference, &types, &mut Budget::default())
            .unwrap(),
        Join::Ready(D::Any)
    ));
}

#[test]
fn branch_join_preserves_equal_heads_and_never_unfolds_them() {
    let inference = InferenceContext::default();
    let mut types = Context::default();
    let head = DeclarationId {
        package: 1,
        local: 2,
    };
    types.admit_congruence(head);
    let normal = known(Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    });
    let result =
        gradual::join_without_target(&normal, &normal, &inference, &types, &mut Budget::default())
            .unwrap();
    assert!(matches!(&result,Join::Ready(value) if *value==normal));
    assert!(
        result
            .to_string()
            .starts_with("(description-join (known-type-description")
    );
    assert!(matches!(
        gradual::join_without_target(
            &normal,
            &boolean(),
            &inference,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        Join::Ready(D::Any)
    ));
    assert!(matches!(
        gradual::join_without_target(
            &boolean(),
            &D::Any,
            &inference,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        Join::Ready(D::Any)
    ));
    // Budget exhaustion is not evidence that these types differ.
    assert!(matches!(
        gradual::join_without_target(
            &normal,
            &normal,
            &inference,
            &types,
            &mut Budget {
                steps: 1,
                max_depth: 128
            }
        ),
        Err(gradual::Error::Budget) | Ok(Join::Unproven(Outcome::Unknown { .. }))
    ));
}

#[test]
fn any_does_not_mask_invalid_sorts_scopes_or_budget_failures() {
    let inference = InferenceContext::default();
    let types = Context::default();
    assert!(
        gradual::compatible(
            &known(Term::Bool(true)),
            &D::Any,
            &inference,
            &types,
            &mut Budget::default()
        )
        .is_err()
    );
    let bound = known(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    });
    assert!(
        gradual::compatible(&D::Any, &bound, &inference, &types, &mut Budget::default()).is_err()
    );
    let scoped = Context::with_scope(vec![vec![Sort::Type]]);
    assert!(matches!(
        gradual::compatible(&D::Any, &bound, &inference, &scoped, &mut Budget::default()).unwrap(),
        C::Gradual
    ));
    assert!(matches!(
        gradual::compatible(
            &D::Any,
            &D::Any,
            &inference,
            &types,
            &mut Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(gradual::Error::Budget)
    ));
}

#[test]
fn description_equality_uses_only_the_supplied_type_facts() {
    let inference = InferenceContext::default();
    let subject = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let plain = Context::with_scope(vec![vec![Sort::Type]]);
    let mut refined = plain.fork();
    refined.assume(Formula::equal(
        subject.clone(),
        Term::Primitive(Primitive::Bool),
    ));
    let description = known(subject);
    assert!(matches!(
        gradual::compatible(
            &description,
            &boolean(),
            &inference,
            &refined,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::Entailed(_))
    ));
    assert!(!matches!(
        gradual::compatible(
            &description,
            &boolean(),
            &inference,
            &plain,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::Entailed(_))
    ));
    assert!(matches!(
        gradual::compatible(
            &description,
            &D::Any,
            &inference,
            &refined,
            &mut Budget::default()
        )
        .unwrap(),
        C::Gradual
    ));
}

#[test]
fn refinement_requirements_are_rechecked_instead_of_assumed() {
    use elaboration::ptc::{TraitQuery, TypeKind};
    use gradual::{Refinement, RefinementCheck as R};
    let subject = DeclarationId {
        package: 3,
        local: 4,
    };
    let itself = Term::Rigid {
        declaration: subject,
        sort: Sort::Type,
    };
    let refinement = Refinement {
        subject,
        requirement: Formula::kind(itself.clone(), TypeKind::Boolean),
    };
    let mut inference = InferenceContext::default();
    let types = Context::default();
    assert!(matches!(
        refinement
            .check(&D::Any, &inference, &types, &mut Budget::default())
            .unwrap(),
        R::NeedsInstantiation
    ));
    let pending = inference.fresh(Sort::Type);
    let description = D::Known(I::Meta(pending));
    assert!(matches!(
        refinement
            .check(&description, &inference, &types, &mut Budget::default())
            .unwrap(),
        R::Waiting(_)
    ));
    inference
        .assign(pending, I::Stable(Term::Primitive(Primitive::U8)))
        .unwrap();
    assert!(matches!(
        refinement
            .check(&description, &inference, &types, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::NotEntailed(_))
    ));
    assert!(matches!(
        refinement
            .check(&boolean(), &inference, &types, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    let interface = DeclarationId {
        package: 3,
        local: 5,
    };
    let trait_requirement = Refinement {
        subject,
        requirement: Formula::Trait(TraitQuery {
            interface,
            arguments: vec![],
            subject: itself,
        }),
    };
    assert!(!matches!(
        trait_requirement
            .check(&boolean(), &inference, &types, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    let mut evidence = types.fork();
    evidence.assume(Formula::Trait(TraitQuery {
        interface,
        arguments: vec![],
        subject: Term::Primitive(Primitive::Bool),
    }));
    assert!(matches!(
        trait_requirement
            .check(&boolean(), &inference, &evidence, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    assert!(
        refinement
            .to_string()
            .starts_with("(type-description-refinement")
    );
}

#[test]
fn refinements_substitute_inside_nested_binders_and_reject_wrong_subject_sorts() {
    use gradual::{Refinement, RefinementCheck as R};
    let subject = DeclarationId {
        package: 3,
        local: 4,
    };
    let itself = Term::Rigid {
        declaration: subject,
        sort: Sort::Type,
    };
    let callback = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let requirement = Formula::equal(
        callback(itself),
        callback(Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::Type,
        }),
    );
    let refinement = Refinement {
        subject,
        requirement,
    };
    let inference = InferenceContext::default();
    let scoped = Context::with_scope(vec![vec![Sort::Type]]);
    let actual = known(Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    });
    assert!(matches!(
        refinement
            .check(&actual, &inference, &scoped, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    let malformed = Refinement {
        subject,
        requirement: Formula::int_le(
            elaboration::ptc::IntegerTerm::variable(subject, 0),
            elaboration::ptc::IntegerTerm::constant(1),
        ),
    };
    assert!(
        malformed
            .check(&boolean(), &inference, &scoped, &mut Budget::default())
            .is_err()
    );
    assert!(
        malformed
            .check(&D::Any, &inference, &scoped, &mut Budget::default())
            .is_err()
    );
    assert!(matches!(
        refinement.check(
            &D::Any,
            &inference,
            &scoped,
            &mut Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(gradual::Error::Budget)
    ));
}

#[test]
fn deferred_refinements_validate_the_proposition_language_before_waiting() {
    use elaboration::ptc::{TraitQuery, TypeKind, ValidationError};
    use gradual::{Refinement, RefinementCheck as R};
    let subject = DeclarationId {
        package: 3,
        local: 4,
    };
    let mut inference = InferenceContext::default();
    let unknown = inference.fresh(Sort::Type);
    let context = Context::default();
    for requirement in [
        Formula::equal(Term::Bool(true), Term::Bool(false)),
        Formula::kind(Term::Bool(true), TypeKind::Boolean),
        Formula::Trait(TraitQuery {
            interface: subject,
            arguments: vec![Term::Bool(true)],
            subject: Term::Primitive(Primitive::Bool),
        }),
    ] {
        let refinement = Refinement {
            subject,
            requirement,
        };
        for description in [D::Any, D::Known(I::Meta(unknown)), boolean()] {
            assert!(matches!(
                refinement.check(&description, &inference, &context, &mut Budget::default()),
                Err(gradual::Error::InvalidRequirement(
                    ValidationError::IllFormed(_)
                ))
            ));
        }
    }
    // Well-formed does not mean true; validation must not smuggle in a proof.
    let false_requirement = Refinement {
        subject,
        requirement: Formula::False,
    };
    assert!(matches!(
        false_requirement
            .check(&D::Any, &inference, &context, &mut Budget::default())
            .unwrap(),
        R::NeedsInstantiation
    ));
    assert!(matches!(
        false_requirement
            .check(&boolean(), &inference, &context, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::NotEntailed(_))
    ));
}

#[test]
fn definition_refinement_facts_are_local_and_do_not_prove_instance_admission() {
    use elaboration::ptc::TypeKind;
    use gradual::{Refinement, RefinementCheck as R, RefinementScope};
    let placeholder = DeclarationId {
        package: 5,
        local: 1,
    };
    let subject = DeclarationId {
        package: 5,
        local: 2,
    };
    let other = DeclarationId {
        package: 5,
        local: 3,
    };
    let rigid = |id| Term::Rigid {
        declaration: id,
        sort: Sort::Type,
    };
    let requirement = Refinement {
        subject: placeholder,
        requirement: Formula::kind(rigid(placeholder), TypeKind::Boolean),
    };
    let parent = Context::default();
    let before = parent.to_string();
    let scope =
        RefinementScope::new(&parent, subject, &[requirement], &mut Budget::default()).unwrap();
    let goal = Formula::kind(rigid(subject), TypeKind::Boolean);
    assert!(matches!(
        scope.prove(goal.clone(), &mut Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        parent.prove(goal, Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        scope.prove(
            Formula::kind(rigid(other), TypeKind::Boolean),
            &mut Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    let mut inference = InferenceContext::default();
    assert!(matches!(
        scope
            .check_instance(&D::Any, &inference, &mut Budget::default())
            .unwrap(),
        R::NeedsInstantiation
    ));
    assert!(matches!(
        scope
            .check_instance(&boolean(), &inference, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    assert!(matches!(
        scope
            .check_instance(&byte(), &inference, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::NotEntailed(_))
    ));
    // Even the declaration's own symbolic subject is not admitted using the
    // facts provisionally available only inside its body.
    assert!(!matches!(
        scope
            .check_instance(&known(rigid(subject)), &inference, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    let waiting = inference.fresh(Sort::Type);
    assert!(
        matches!(scope.check_instance(&D::Known(I::Meta(waiting)),&inference,&mut Budget::default()).unwrap(),R::Waiting(ids) if ids.contains(&waiting))
    );
    inference
        .assign(waiting, I::Stable(Term::Primitive(Primitive::U8)))
        .unwrap();
    assert!(matches!(
        scope
            .check_instance(
                &D::Known(I::Meta(waiting)),
                &inference,
                &mut Budget::default()
            )
            .unwrap(),
        R::Checked(Outcome::NotEntailed(_))
    ));
    assert_eq!(parent.to_string(), before);
    assert!(
        scope
            .to_string()
            .starts_with("(definition-refinement-scope (type-description-refinement")
    );
}

#[test]
fn definition_refinements_validate_all_facts_and_preserve_enclosing_binders() {
    use gradual::{Refinement, RefinementCheck as R, RefinementScope};
    let original = DeclarationId {
        package: 6,
        local: 1,
    };
    let local = DeclarationId {
        package: 6,
        local: 2,
    };
    let parent = Context::with_scope(vec![vec![Sort::Type]]);
    let caller = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let callback = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let requirement = Refinement {
        subject: original,
        requirement: Formula::equal(
            callback(Term::Rigid {
                declaration: original,
                sort: Sort::Type,
            }),
            callback(Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            }),
        ),
    };
    let scope =
        RefinementScope::new(&parent, local, &[requirement], &mut Budget::default()).unwrap();
    let inference = InferenceContext::default();
    assert!(matches!(
        scope
            .check_instance(&known(caller), &inference, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    let impossible = Refinement {
        subject: original,
        requirement: Formula::False,
    };
    let malformed = Refinement {
        subject: original,
        requirement: Formula::equal(Term::Bool(true), Term::Bool(false)),
    };
    assert!(matches!(
        RefinementScope::new(
            &parent,
            local,
            &[impossible.clone(), malformed],
            &mut Budget::default()
        ),
        Err(gradual::Error::InvalidRequirement(_))
    ));
    let scope =
        RefinementScope::new(&parent, local, &[impossible], &mut Budget::default()).unwrap();
    assert!(matches!(
        scope.prove(Formula::False, &mut Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        scope
            .check_instance(&boolean(), &inference, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::NotEntailed(_))
    ));
    assert!(matches!(
        scope
            .check_instance(&D::Any, &inference, &mut Budget::default())
            .unwrap(),
        R::NeedsInstantiation
    ));
    assert!(matches!(
        RefinementScope::new(
            &parent,
            local,
            &[],
            &mut Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(gradual::Error::Budget)
    ));
}

#[test]
fn definition_refinement_conjunction_rebinds_each_subject_and_checks_every_requirement() {
    use elaboration::ptc::{TraitQuery, TypeKind};
    use gradual::{Refinement, RefinementCheck as R, RefinementScope};
    let id = |local| DeclarationId { package: 7, local };
    let rigid = |local| Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    };
    let query = |subject| {
        Formula::Trait(TraitQuery {
            interface: id(9),
            arguments: vec![],
            subject,
        })
    };
    let refinements = [
        Refinement {
            subject: id(1),
            requirement: Formula::kind(rigid(1), TypeKind::Boolean),
        },
        Refinement {
            subject: id(2),
            requirement: query(rigid(2)),
        },
    ];
    let parent = Context::default();
    let scope = RefinementScope::new(&parent, id(3), &refinements, &mut Budget::default()).unwrap();
    assert!(matches!(
        scope.prove(query(rigid(3)), &mut Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        scope.prove(
            Formula::kind(rigid(3), TypeKind::Boolean),
            &mut Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        scope.prove(query(rigid(2)), &mut Budget::default()),
        Outcome::Entailed(_)
    ));
    let inference = InferenceContext::default();
    assert!(
        !matches!(
            scope
                .check_instance(&boolean(), &inference, &mut Budget::default())
                .unwrap(),
            R::Checked(Outcome::Entailed(_))
        ),
        "a Boolean type alone supplies no trait implementation"
    );
    let mut evidence = parent.fork();
    evidence.assume(query(Term::Primitive(Primitive::Bool)));
    evidence.assume(query(Term::Primitive(Primitive::U8)));
    let scope =
        RefinementScope::new(&evidence, id(3), &refinements, &mut Budget::default()).unwrap();
    assert!(matches!(
        scope
            .check_instance(&boolean(), &inference, &mut Budget::default())
            .unwrap(),
        R::Checked(Outcome::Entailed(_))
    ));
    let rejected = scope
        .check_instance(&byte(), &inference, &mut Budget::default())
        .unwrap();
    assert!(
        !matches!(rejected, R::Checked(Outcome::Entailed(_))),
        "a trait implementation alone cannot satisfy the Boolean classification: {rejected}"
    );
}

#[test]
fn frozen_descriptions_keep_any_distinct_from_waiters_and_drop_inference_ownership() {
    use gradual::{Freeze, FrozenDescription as F};
    let types = Context::default();
    let mut inference = InferenceContext::default();
    let original = inference.fresh(Sort::Type);
    let alias = inference.fresh(Sort::Type);
    inference.assign(original, I::Meta(alias)).unwrap();
    let description = D::Known(I::Meta(original));
    assert!(
        matches!(description.freeze(&inference,&types,&mut Budget::default()).unwrap(),
        Freeze::Waiting(ids) if ids.iter().copied().collect::<Vec<_>>()==vec![alias])
    );
    assert_eq!(
        D::Any
            .freeze(&inference, &types, &mut Budget::default())
            .unwrap(),
        Freeze::Ready(F::Any)
    );
    let mut candidate = inference.trial();
    candidate
        .assign(alias, I::Stable(Term::Primitive(Primitive::Bool)))
        .unwrap();
    assert!(matches!(
        description.freeze(&candidate, &types, &mut Budget::default()),
        Err(gradual::Error::SpeculativeContext)
    ));
    inference.commit(candidate).unwrap();
    let Freeze::Ready(saved) = description
        .freeze(&inference, &types, &mut Budget::default())
        .unwrap()
    else {
        panic!("ready")
    };
    drop(inference);
    let other = InferenceContext::default();
    assert_eq!(saved, F::Known(Term::Primitive(Primitive::Bool)));
    assert!(matches!(
        gradual::compatible(
            &saved.description(),
            &boolean(),
            &other,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::Entailed(_))
    ));
    assert!(matches!(
        gradual::compatible(
            &F::Any.description(),
            &byte(),
            &other,
            &types,
            &mut Budget::default()
        )
        .unwrap(),
        C::Gradual
    ));
    assert_eq!(
        saved.to_string(),
        "(known-type-description (primitive bool))"
    );
    assert_eq!(
        Freeze::Ready(F::Any).to_string(),
        "(frozen-description (any-type-description))"
    );
}

#[test]
fn frozen_descriptions_preserve_declaration_scope_without_exporting_hypotheses() {
    use gradual::{Freeze, FrozenDescription as F};
    let inference = InferenceContext::default();
    let binder = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let mut definition = Context::with_scope(vec![vec![Sort::Type]]);
    definition.assume(Formula::equal(
        binder.clone(),
        Term::Primitive(Primitive::Bool),
    ));
    let Freeze::Ready(saved) = known(binder.clone())
        .freeze(&inference, &definition, &mut Budget::default())
        .unwrap()
    else {
        panic!("ready")
    };
    assert_eq!(saved, F::Known(binder));
    assert!(
        saved
            .description()
            .freeze(&inference, &Context::default(), &mut Budget::default())
            .is_err()
    );
    let plain = Context::with_scope(vec![vec![Sort::Type]]);
    assert!(!matches!(
        gradual::compatible(
            &saved.description(),
            &boolean(),
            &inference,
            &plain,
            &mut Budget::default()
        )
        .unwrap(),
        C::Exact(Outcome::Entailed(_))
    ));
    let mut caller = InferenceContext::default();
    let input = caller.fresh(Sort::Type);
    let instance = saved.instantiate(vec![I::Meta(input)]);
    assert!(
        matches!(instance.freeze(&caller,&Context::default(),&mut Budget::default()).unwrap(),Freeze::Waiting(ids) if ids.contains(&input))
    );
    let tuple = Term::NormalForm {
        head: DeclarationId {
            package: 9,
            local: 3,
        },
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::U8),
        ])],
    };
    caller.assign(input, I::Stable(tuple.clone())).unwrap();
    assert_eq!(
        instance
            .freeze(&caller, &Context::default(), &mut Budget::default())
            .unwrap(),
        Freeze::Ready(F::Known(tuple))
    );
    assert!(matches!(F::Any.instantiate(vec![I::Meta(input)]), D::Any));
}

#[test]
fn frozen_descriptions_reject_invalid_domains_and_exhausted_work() {
    let inference = InferenceContext::default();
    let types = Context::default();
    assert!(
        known(Term::Bool(true))
            .freeze(&inference, &types, &mut Budget::default())
            .is_err()
    );
    for description in [D::Any, boolean()] {
        assert!(matches!(
            description.freeze(
                &inference,
                &types,
                &mut Budget {
                    steps: 0,
                    max_depth: 128
                }
            ),
            Err(gradual::Error::Budget)
        ));
    }
    let wide = known(Term::Function {
        parameters: vec![
            nir::Parameter {
                sort: Sort::Runtime,
                ty: Term::Primitive(Primitive::Bool)
            };
            1000
        ],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    });
    assert!(matches!(
        wide.freeze(
            &inference,
            &types,
            &mut Budget {
                steps: 20,
                max_depth: 128
            }
        ),
        Err(gradual::Error::Budget)
    ));
}

#[test]
fn diverging_branch_joins_preserve_the_normal_result_without_type_equality() {
    let inference = InferenceContext::default();
    let types = Context::default();
    let never = known(Term::Primitive(Primitive::Never));
    for normal in [boolean(), byte(), D::Any, never.clone()] {
        for (left, right) in [(&never, &normal), (&normal, &never)] {
            assert!(
                matches!(gradual::join_without_target(left,right,&inference,&types,&mut Budget::default()).unwrap(),Join::Ready(result) if result==normal)
            );
        }
    }
    assert!(
        matches!(
            gradual::compatible(
                &never,
                &boolean(),
                &inference,
                &types,
                &mut Budget::default()
            )
            .unwrap(),
            C::Exact(Outcome::NotEntailed(_))
        ),
        "non-completion must not imply type equality"
    );
    assert!(
        matches!(
            gradual::join_without_target(
                &known(Term::Primitive(Primitive::Void)),
                &boolean(),
                &inference,
                &types,
                &mut Budget::default()
            )
            .unwrap(),
            Join::Ready(D::Any)
        ),
        "void completes normally and must not disappear"
    );
}

#[test]
fn symbolic_divergence_uses_only_supplied_type_evidence() {
    let inference = InferenceContext::default();
    let subject = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    let plain = Context::with_scope(vec![vec![Sort::Type]]);
    let mut scoped = plain.fork();
    scoped.assume(Formula::equal(
        subject.clone(),
        Term::Primitive(Primitive::Never),
    ));
    let description = known(subject);
    for (left, right) in [(&description, &boolean()), (&boolean(), &description)] {
        assert!(
            matches!(gradual::join_without_target(left,right,&inference,&scoped,&mut Budget::default()).unwrap(),Join::Ready(result) if result==boolean())
        );
    }
    assert!(matches!(
        gradual::join_without_target(
            &description,
            &boolean(),
            &inference,
            &plain,
            &mut Budget::default()
        )
        .unwrap(),
        Join::Ready(D::Any)
    ));
    let mut opaque = Context::default();
    let head = DeclarationId {
        package: 17,
        local: 6,
    };
    opaque.admit_congruence(head);
    let normal = known(Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![],
    });
    assert!(matches!(
        gradual::join_without_target(
            &normal,
            &boolean(),
            &inference,
            &opaque,
            &mut Budget::default()
        )
        .unwrap(),
        Join::Ready(D::Any)
    ));
}

#[test]
fn divergence_does_not_hide_waiting_or_malformed_branch_descriptions() {
    let mut inference = InferenceContext::default();
    let variable = inference.fresh(Sort::Type);
    let pending = D::Known(I::Meta(variable));
    let types = Context::default();
    let never = known(Term::Primitive(Primitive::Never));
    assert!(
        matches!(gradual::join_without_target(&never,&pending,&inference,&types,&mut Budget::default()).unwrap(),Join::Waiting(ids) if ids.contains(&variable))
    );
    inference
        .assign(variable, I::Stable(Term::Primitive(Primitive::U8)))
        .unwrap();
    assert!(
        matches!(gradual::join_without_target(&never,&pending,&inference,&types,&mut Budget::default()).unwrap(),Join::Ready(result) if result==pending)
    );
    for invalid in [
        known(Term::Bool(true)),
        known(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
    ] {
        assert!(
            gradual::join_without_target(
                &never,
                &invalid,
                &inference,
                &types,
                &mut Budget::default()
            )
            .is_err()
        );
        assert!(
            gradual::join_without_target(
                &invalid,
                &never,
                &inference,
                &types,
                &mut Budget::default()
            )
            .is_err()
        );
    }
    assert!(matches!(
        gradual::join_without_target(
            &never,
            &boolean(),
            &inference,
            &types,
            &mut Budget {
                steps: 0,
                max_depth: 128
            }
        ),
        Err(gradual::Error::Budget)
    ));
}

#[test]
fn gradual_observation_honors_depth_limits_even_without_an_equality_query() {
    let inference = InferenceContext::default();
    let types = Context::default();
    let pointer = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
    let description = known(Term::Pointer(Box::new(pointer.clone())));
    let budget = || Budget {
        steps: 10_000,
        max_depth: 2,
    };
    assert!(matches!(
        gradual::compatible(&description, &D::Any, &inference, &types, &mut budget()),
        Err(gradual::Error::Budget)
    ));
    assert!(matches!(
        gradual::join_without_target(&description, &D::Any, &inference, &types, &mut budget()),
        Err(gradual::Error::Budget)
    ));
    assert!(matches!(
        description.freeze(&inference, &types, &mut budget()),
        Err(gradual::Error::Budget)
    ));
    assert!(matches!(
        description
            .freeze(
                &inference,
                &types,
                &mut Budget {
                    steps: 10_000,
                    max_depth: 3
                }
            )
            .unwrap(),
        gradual::Freeze::Ready(_)
    ));
    let instantiated = D::Known(I::Instantiate {
        template: Term::Pointer(Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        })),
        arguments: vec![I::Stable(pointer)],
    });
    // Template and argument separately fit. Their expanded result does not.
    assert!(matches!(
        instantiated.freeze(&inference, &types, &mut budget()),
        Err(gradual::Error::Budget)
    ));
    assert!(matches!(
        instantiated
            .freeze(
                &inference,
                &types,
                &mut Budget {
                    steps: 10_000,
                    max_depth: 3
                }
            )
            .unwrap(),
        gradual::Freeze::Ready(_)
    ));
}
