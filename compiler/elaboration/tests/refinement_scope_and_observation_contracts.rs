//! Production-API regression contracts preserved from archived gradual/demand suites:
//! PTC proposition validation, scoped hypothesis isolation, normal-form head
//! opacity, Never non-equality, and comptime-index trait queries.

use elaboration::ptc::{Budget, Context, Formula, Outcome, TraitQuery, TypeKind, ValidationError};
use nir::{DeclarationId, Primitive, Sort, Term};

#[test]
fn ptc_validates_proposition_sorts_and_scopes_without_assuming_false() {
    let subject = DeclarationId {
        package: 3,
        local: 4,
    };
    let context = Context::default();
    for formula in [
        Formula::equal(Term::Bool(true), Term::Bool(false)),
        Formula::kind(Term::Bool(true), TypeKind::Boolean),
        Formula::Trait(TraitQuery {
            interface: subject,
            arguments: vec![Term::Runtime(nir::RuntimeId {
                instance: 1,
                local: 0,
            })],
            subject: Term::Primitive(Primitive::Bool),
        }),
    ] {
        assert!(matches!(
            context.validate_formula(&formula, &mut Budget::default()),
            Err(ValidationError::IllFormed(_))
        ));
    }

    assert_eq!(
        context.validate_formula(&Formula::False, &mut Budget::default()),
        Ok(())
    );
    assert!(matches!(
        context.prove(Formula::False, Budget::default()),
        Outcome::NotEntailed(_)
    ));
}

#[test]
fn ptc_definition_scope_facts_remain_isolated_and_accept_comptime_trait_indices() {
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
    let parent = Context::default();
    let before = parent.to_string();
    let mut scoped = parent.fork();
    let fact = Formula::kind(rigid(subject), TypeKind::Boolean);
    assert_eq!(
        scoped.validate_formula(&fact, &mut Budget::default()),
        Ok(())
    );
    scoped.assume(fact.clone());
    assert!(matches!(
        scoped.prove(fact.clone(), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        parent.prove(fact, Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        scoped.prove(
            Formula::kind(rigid(other), TypeKind::Boolean),
            Budget::default()
        ),
        Outcome::Entailed(_)
    ));
    assert_eq!(parent.to_string(), before);

    // Trait queries with comptime value arguments are well-formed without
    // proving satisfaction for arbitrary subjects.
    let comptime_indexed = Formula::Trait(TraitQuery {
        interface: DeclarationId {
            package: 3,
            local: 5,
        },
        arguments: vec![Term::Bool(true)],
        subject: Term::Primitive(Primitive::Bool),
    });
    assert_eq!(
        scoped.validate_formula(&comptime_indexed, &mut Budget::default()),
        Ok(())
    );
    assert!(!matches!(
        scoped.prove(comptime_indexed, Budget::default()),
        Outcome::Entailed(_)
    ));
}

#[test]
fn ptc_preserves_normal_form_heads_and_rejects_never_type_equality() {
    let mut context = Context::default();
    let head = DeclarationId {
        package: 1,
        local: 2,
    };
    context.admit_congruence(head);
    let normal = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    assert!(matches!(
        context.prove(Formula::equal(normal.clone(), normal), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(matches!(
        context.prove(
            Formula::equal(
                Term::Primitive(Primitive::Never),
                Term::Primitive(Primitive::Bool)
            ),
            Budget::default()
        ),
        Outcome::NotEntailed(_)
    ));
}
