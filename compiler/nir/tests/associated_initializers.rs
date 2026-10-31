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

#[test]
fn substitution_bounds_repeated_large_types_before_copying() {
    let declaration = DeclarationId {
        package: 9,
        local: 1,
    };
    let variable = Term::Rigid {
        declaration,
        sort: Sort::Type,
    };
    let body = expr(
        Term::Primitive(Primitive::Type),
        K::TypeValue(Term::Tuple(vec![variable; 32])),
    );
    // Closure bodies are shared in the frozen value, but substitution traverses
    // and copies their types. The preflight must enter that shared body too.
    let closure = expr(
        Term::Function {
            parameters: vec![],
            result: Box::new(body.ty.clone()),
        },
        K::Closure {
            parameters: vec![],
            captures: vec![],
            body: Arc::new(body),
        },
    );
    let template = AssociatedInitializer::new(closure, &mut 10000).unwrap();
    let parameters = [DeclarationParameter {
        declaration,
        sort: Sort::Type,
    }];
    let arguments = [Term::Tuple(vec![boolean(); 64])];
    let mut fuel = 500;
    assert!(
        template
            .instantiate(&parameters, &arguments, &mut fuel)
            .is_err()
    );
    assert!(fuel < 500, "failed preflight must retain consumed work");
    let specialized = template
        .instantiate(&parameters, &arguments, &mut 100000)
        .unwrap();
    let K::Closure { body, .. } = &specialized.expression().kind else {
        panic!("closure")
    };
    assert_eq!(
        body.kind,
        K::TypeValue(Term::Tuple(vec![arguments[0].clone(); 32]))
    );
    let K::Closure { body, .. } = &template.expression().kind else {
        panic!("closure")
    };
    assert_ne!(
        body.kind,
        K::TypeValue(Term::Tuple(vec![arguments[0].clone(); 32]))
    );
}

#[test]
fn substitution_charges_unused_inputs_once_instead_of_expanding_them() {
    let parameter = DeclarationParameter {
        declaration: DeclarationId {
            package: 9,
            local: 2,
        },
        sort: Sort::Type,
    };
    let body = expr(
        Term::Primitive(Primitive::Type),
        K::TypeValue(Term::Tuple(vec![boolean(); 32])),
    );
    let template = AssociatedInitializer::new(body, &mut 10000).unwrap();
    let argument = Term::Tuple(vec![boolean(); 64]);
    let mut fuel = 500;
    let result = template
        .instantiate(&[parameter], &[argument], &mut fuel)
        .unwrap();
    assert_eq!(result, template);
    assert!(fuel < 500);
    assert!(fuel > 0);
    // Unused parameters still obey the declared input sort.
    assert!(
        template
            .instantiate(&[parameter], &[Term::Bool(true)], &mut 500)
            .is_err()
    );
}
