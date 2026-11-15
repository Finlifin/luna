use super::*;
use crate::program::{
    ArgumentPassing, CallArgument, Capture, EffectHandler, FunctionPhase, LocalId,
    NormalFormArgument as A, SourceRange,
};
use crate::{DeclarationId, Primitive, Term};
use std::sync::Arc;

const WIDTH: usize = 4096;
const SOURCE: SourceRange = SourceRange { start: 0, end: 0 };

fn expr(kind: K) -> Expr {
    Expr {
        ty: Term::Primitive(Primitive::Void),
        source: SOURCE,
        kind,
    }
}

fn assert_bounded(root: &Expr) {
    let mut visited = 0;
    let result = expression_pruned_checked(root, &mut 8, |_, _| {
        visited += 1;
        Ok::<_, Error>(true)
    });
    assert!(matches!(result, Err(Error::Budget(_))));
    assert!(
        visited <= 3,
        "wide children must not be visited before rejection"
    );
    let mut rewritten = root.clone();
    let mut mutable_visits = 0;
    assert!(
        expression_mut(&mut rewritten, &mut 8, |_, _| {
            mutable_visits += 1;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(visited, mutable_visits);
    assert_eq!(&rewritten, root);
}

#[test]
fn wide_expression_pattern_and_contextual_frontiers_are_bounded() {
    assert_bounded(&expr(K::Tuple(vec![expr(K::Unit); WIDTH])));
    assert_bounded(&expr(K::LetPattern {
        pattern: P::Tuple(vec![P::Wild; WIDTH]),
        value: Box::new(expr(K::Unit)),
        guard: None,
        otherwise: None,
    }));
    let input = A::List {
        source: SOURCE,
        items: vec![A::Unit { source: SOURCE }; WIDTH],
    };
    let result = argument_checked(&input, &mut 8, |_, _| Ok::<_, Error>(()));
    assert!(matches!(result, Err(Error::Budget(_))));
    assert_bounded(&expr(K::NormalFormType {
        head: DeclarationId {
            package: 0,
            local: 1,
        },
        arguments: vec![input],
        plan: None,
    }));
}

#[test]
fn pruning_does_not_charge_or_expand_descendants() {
    let mut root = expr(K::Tuple(vec![expr(K::Unit); WIDTH]));
    let mut fuel = 1;
    expression_pruned_checked(&root, &mut fuel, |_, _| Ok::<_, Error>(false)).unwrap();
    assert_eq!(fuel, 0);
    let mut fuel = 1;
    expression_mut_pruned(&mut root, &mut fuel, |_, _| Ok(false)).unwrap();
    assert_eq!(fuel, 0);
}

#[test]
fn metadata_scans_are_bounded_even_without_expression_children() {
    let mut closure = expr(K::Closure {
        parameters: vec![],
        captures: vec![
            Capture {
                binding: LocalId { owner: 0, index: 0 },
                ty: Term::Primitive(Primitive::Void),
                value: None,
            };
            WIDTH
        ],
        body: Arc::new(expr(K::Unit)),
    });
    assert!(expression_in_scope(&closure, &mut 8, |_, _| Ok(())).is_err());
    assert!(expression_mut_in_closure_environment(&mut closure, &mut 8, |_, _| Ok(())).is_err());

    let call = expr(K::Call {
        callee: Box::new(expr(K::Unit)),
        arguments: vec![
            CallArgument {
                passing: ArgumentPassing::Value,
                parameter: 0,
                value: expr(K::Unit),
                is_default: true,
            };
            WIDTH
        ],
        parameter_bindings: vec![],
    });
    assert!(expression_in_scope(&call, &mut 8, |_, _| Ok(())).is_err());
    let mut visits = 0;
    expression_in_scope(&call, &mut (WIDTH + 2), |_, _| {
        visits += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        visits, 2,
        "default expressions still belong to another scope"
    );

    let mut handled = expr(K::EffectHandle {
        computation: Box::new(expr(K::Unit)),
        handlers: vec![
            EffectHandler {
                operation: crate::effects::Operation {
                    declaration: DeclarationId {
                        package: 0,
                        local: 1
                    },
                    arguments: vec![],
                },
                abi: crate::effects::Abi::Direct,
                signature: Term::Primitive(Primitive::Void),
                arms: vec![],
                fallback: None,
            };
            WIDTH
        ],
        outer: vec![],
    });
    assert_bounded(&handled);
    expression_in_scope(&handled, &mut 2, |_, _| Ok(())).unwrap();
    expression_mut_in_handler_scope(&mut handled, &mut 2, |_, _| Ok(())).unwrap();
}

#[test]
fn excluded_function_defaults_leave_the_root_budget_untouched() {
    let function = Function {
        interface: crate::program::FunctionDeclaration {
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: DeclarationId {
                package: 0,
                local: 1,
            },
            name: "scope".into(),
            signature: Term::Primitive(Primitive::Void),
            parameters: vec![],
            defaults: vec![(0, expr(K::Tuple(vec![expr(K::Unit); 8]))); WIDTH],
        },
        body: expr(K::Unit),
    };
    expressions_in_scope(&function, &mut 1, |_, _| Ok(())).unwrap();
    assert!(expressions(&function, &mut 8, |_, _| Ok(())).is_err());
}

#[test]
fn successful_visits_keep_structural_order_after_budget_failure() {
    fn local(index: u32) -> Expr {
        expr(K::Local(LocalId { owner: 0, index }))
    }
    let mut root = expr(K::Tuple(vec![
        local(0),
        expr(K::Tuple(vec![local(1), local(2)])),
    ]));
    assert!(expression(&root, &mut 2, |_, _| Ok(())).is_err());
    let mut read_order = vec![];
    let mut fuel = 5;
    expression(&root, &mut fuel, |expr, _| {
        if let K::Local(id) = expr.kind {
            read_order.push(id.index);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(read_order, [2, 1, 0]);
    assert_eq!(fuel, 0);
    let mut write_order = vec![];
    expression_mut(&mut root, &mut 5, |expr, _| {
        if let K::Local(id) = expr.kind {
            write_order.push(id.index);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(read_order, write_order);
}
