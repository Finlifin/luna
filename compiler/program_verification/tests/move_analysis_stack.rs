use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Literal, LocalId, SourceRange};
use nir::{DeclarationId, Primitive, Term};
use program_verification::moves::{self, Delivery};

fn nested_bindings(layers: u32) -> Function {
    let boolean = Term::Primitive(Primitive::Bool);
    let expression = |ty, kind, offset| Expr {
        ty,
        kind,
        source: SourceRange {
            start: offset,
            end: offset + 1,
        },
    };
    let mut body = expression(boolean.clone(), K::Literal(Literal::Bool(true)), 0);
    for local in 0..layers {
        let binding = LocalId {
            owner: 1,
            index: local,
        };
        let statement = expression(
            Term::Primitive(Primitive::Void),
            K::Let {
                binding,
                value: Box::new(body),
            },
            local * 3 + 1,
        );
        body = expression(
            boolean.clone(),
            K::Block {
                statements: vec![statement],
                result: Some(Box::new(expression(
                    boolean.clone(),
                    K::Local(binding),
                    local * 3 + 2,
                ))),
            },
            local * 3 + 3,
        );
    }
    Function {
        interface: nir::program::FunctionDeclaration {
            source: nir::program::SourceRange { start: 0, end: 0 },
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            name: "nested_bindings".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(boolean),
            },
            parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
        },
        body,
    }
}

#[test]
fn nested_bindings_preserve_copies_and_report_limits_on_an_ordinary_stack() {
    // Fix the stack independently of the test runner's RUST_MIN_STACK. Both
    // successful analysis and its depth-limit failure must return normally.
    std::thread::Builder::new()
        .name("nested-resource-analysis".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let input = nested_bindings(18);
            let report = moves::analyze(&input, |_| Ok(Delivery::Copy), 100_000).unwrap();
            assert_eq!(report.copies.len(), 18);
            assert!(report.transfers.is_empty());

            let exhausted = moves::analyze(&input, |_| Ok(Delivery::Copy), 20).unwrap_err();
            assert!(exhausted.reason.contains("budget exhausted"), "{exhausted}");
            let deep = nested_bindings(128);
            let limited = moves::analyze(&deep, |_| Ok(Delivery::Copy), 100_000).unwrap_err();
            assert!(limited.reason.contains("budget exhausted"), "{limited}");

            let retried = moves::analyze(&input, |_| Ok(Delivery::Copy), 100_000).unwrap();
            assert_eq!(report.to_string(), retried.to_string());
        })
        .unwrap()
        .join()
        .unwrap();
}
