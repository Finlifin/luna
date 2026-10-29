use elaboration::ptc::{Budget, Context, Formula as F, Outcome};
use nir::{DeclarationId, Primitive as P, Sort, Term};
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn variable(index: u32) -> Term {
    Term::Rigid {
        declaration: DeclarationId {
            package: 0,
            local: index,
        },
        sort: Sort::Type,
    }
}
fn head(index: u32, arguments: Vec<Term>) -> Term {
    Term::NormalForm {
        head: DeclarationId {
            package: 1,
            local: index,
        },
        sort: Sort::Type,
        arguments,
    }
}
fn eq(a: Term, b: Term) -> F {
    F::equal(a, b)
}
fn proved(context: &Context, goal: F) {
    let outcome = context.prove(goal, Budget::default());
    assert!(matches!(outcome, Outcome::Entailed(_)), "{outcome}");
    if let Outcome::Entailed(proof) = outcome {
        assert_eq!(context.verify_proof(&proof, Budget::default()), Ok(()));
    }
}

#[test]
fn owned_package_context_restores_only_valid_declaration_theory() {
    use nir::normal_forms::{Head, Kind, Parameter};
    use nir::program::{LocalId, Package};
    let declaration = DeclarationId {
        package: 1,
        local: 7,
    };
    let mut package = Package::default();
    package.normal_forms.push(Head {
        collector: None,
        role: nir::normal_forms::Role::Opaque,
        declaration,
        parameters: vec![Parameter {
            declaration: DeclarationId {
                package: 1,
                local: 8,
            },
            binding: LocalId { owner: 7, index: 0 },
            kind: Kind::Type,
            ty: ty(P::Type),
        }],
        requirements: vec![],
    });
    let mut context = Context::from_package(&package).unwrap();
    context.assume(eq(variable(1), variable(2)));
    proved(
        &context,
        eq(head(7, vec![variable(1)]), head(7, vec![variable(2)])),
    );
    let fresh = Context::from_package(&package).unwrap();
    assert!(!matches!(
        fresh.prove(eq(variable(1), variable(2)), Budget::default()),
        Outcome::Entailed(_)
    ));
    assert!(!matches!(
        fresh.prove(eq(head(7, vec![ty(P::U8)]), ty(P::U8)), Budget::default()),
        Outcome::Entailed(_)
    ));
    package.normal_forms[0].parameters[0].ty = ty(P::Bool);
    assert!(Context::from_package(&package).is_err());
    assert!(
        elaboration::materialization::dispatch_instances(&package, Budget::default())
            .unwrap_err()
            .contains("dispatch type context")
    );
}
