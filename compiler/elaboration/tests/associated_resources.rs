//! Resource evidence for unresolved associated values; selection remains a
//! separate prerequisite enforced by semantic publication and materialization.
use elaboration::verification::moves::{self, Delivery};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn projection(ty: Term) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty,
        kind: K::AssociatedValue {
            interface: id(1).into(),
            member: id(2),
            subject: Term::Primitive(P::Bool),
        },
    }
}
fn function(body: Expr) -> Function {
    Function {
        declaration: id(3),
        name: "read".into(),
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: true,
        is_unsafe: false,
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        parameters: vec![],
        defaults: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(body.ty.clone()),
        },
        body,
    }
}

#[test]
fn associated_scalar_reads_require_copy_without_inventing_provenance() {
    let boolean = Term::Primitive(P::Bool);
    for (ty, delivery, accepted) in [
        (boolean.clone(), Delivery::Copy, true),
        (boolean.clone(), Delivery::Move, false),
        (Term::Pointer(Box::new(boolean)), Delivery::Copy, false),
        (
            Term::Nominal {
                declaration: id(9),
                arguments: vec![],
            },
            Delivery::Copy,
            false,
        ),
    ] {
        let function = function(projection(ty));
        let report = moves::analyze(&function, |_| Ok(delivery), 10_000);
        assert_eq!(report.is_ok(), accepted, "{report:?}");
    }
}

#[test]
fn associated_boolean_guard_observes_a_value_without_claiming_its_truth() {
    let boolean = Term::Primitive(P::Bool);
    let literal = |flag| Expr {
        source: SourceRange { start: 3, end: 4 },
        ty: boolean.clone(),
        kind: K::Literal(Literal::Bool(flag)),
    };
    let function = function(Expr {
        source: SourceRange { start: 0, end: 5 },
        ty: boolean.clone(),
        kind: K::If {
            condition: Box::new(projection(boolean.clone())),
            then_branch: Box::new(literal(true)),
            else_branch: Some(Box::new(literal(false))),
        },
    });
    moves::analyze(&function, |_| Ok(Delivery::Copy), 10_000).unwrap();
    assert!(moves::analyze(&function, |_| Ok(Delivery::Copy), 0).is_err());
    moves::analyze(&function, |_| Ok(Delivery::Copy), 10_000).unwrap();
}
