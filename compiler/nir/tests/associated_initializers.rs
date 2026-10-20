use nir::associated_initializers::AssociatedInitializer;
use nir::program::{Capture, Expr, ExprKind as K, LocalId, SourceRange};
use nir::{DeclarationId, DeclarationParameter, Primitive, Sort, Term};
use std::sync::Arc;
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 0, end: 1 },
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn signature() -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(boolean()),
    }
}
fn call(comptime: bool) -> Expr {
    let ty = if comptime {
        Term::ComptimeFunction(Box::new(signature()))
    } else {
        signature()
    };
    expr(
        boolean(),
        K::Call {
            callee: Box::new(expr(
                ty,
                K::Function(DeclarationId {
                    package: 1,
                    local: 2,
                }),
            )),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    )
}
#[test]
fn programs_are_not_completed_values_and_cannot_capture_callers() {
    let template = AssociatedInitializer::new(call(true), &mut 1000).unwrap();
    assert!(nir::completed_values::validate(template.expression(), &mut 1000).is_err());
    assert!(AssociatedInitializer::new(call(false), &mut 1000).is_err());
    assert!(
        AssociatedInitializer::new(
            expr(boolean(), K::Local(LocalId { owner: 3, index: 4 })),
            &mut 1000
        )
        .is_err()
    );
    let dump = template.to_string();
    assert!(
        dump.starts_with("(associated-initializer (result (primitive bool)) (body "),
        "{dump}"
    );
    assert_eq!(dump.matches('(').count(), dump.matches(')').count());
}
#[test]
fn closure_bodies_are_not_invoked_but_captures_are_checked() {
    let closure = expr(
        signature(),
        K::Closure {
            parameters: vec![],
            captures: vec![],
            body: Arc::new(call(false)),
        },
    );
    AssociatedInitializer::new(closure.clone(), &mut 1000).unwrap();
    let mut invalid = closure;
    let K::Closure { captures, .. } = &mut invalid.kind else {
        unreachable!()
    };
    captures.push(Capture {
        binding: LocalId { owner: 3, index: 1 },
        ty: boolean(),
        value: Some(Box::new(call(false))),
    });
    assert!(AssociatedInitializer::new(invalid, &mut 1000).is_err());
}
#[test]
fn substitution_keeps_programs_and_recovers_after_budget_failure() {
    let id = DeclarationId {
        package: 4,
        local: 5,
    };
    let template = AssociatedInitializer::new(
        expr(
            boolean(),
            K::ComptimeValue(Term::Rigid {
                declaration: id,
                sort: Sort::ComptimeValue,
            }),
        ),
        &mut 1000,
    )
    .unwrap();
    let binders = [DeclarationParameter {
        declaration: id,
        sort: Sort::ComptimeValue,
    }];
    assert!(
        template
            .instantiate(&binders, &[Term::Bool(true)], &mut 1)
            .is_err()
    );
    let value = template
        .instantiate(&binders, &[Term::Bool(true)], &mut 1000)
        .unwrap();
    assert_eq!(value.expression().kind, K::ComptimeValue(Term::Bool(true)));
    assert!(
        template
            .instantiate(&binders, &[boolean()], &mut 1000)
            .is_err()
    );
    assert!(matches!(
        template.expression().kind,
        K::ComptimeValue(Term::Rigid { .. })
    ));
}
