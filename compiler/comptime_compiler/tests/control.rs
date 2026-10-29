use comptime_compiler::{Error, compile};
use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
use nir::{Primitive, Term};
fn expr(kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Void),
        kind,
    }
}
#[test]
fn linear_compilation_has_bounded_work_and_deterministic_dump() {
    let body = expr(K::Block {
        statements: vec![expr(K::Unit)],
        result: Some(Box::new(expr(K::Unit))),
    });
    let first = compile(&body, &mut 100).unwrap().to_string();
    assert!(first.contains("(enter-scope)"));
    assert!(first.contains("(leave-scope)"));
    assert_eq!(first, compile(&body, &mut 100).unwrap().to_string());
    assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
}
#[test]
fn unsupported_work_and_invalid_transfers_never_publish_partial_code() {
    let body = expr(K::If {
        condition: Box::new(expr(K::Unit)),
        then_branch: Box::new(expr(K::Unit)),
        else_branch: Some(Box::new(expr(K::OptionalAbsent))),
    });
    assert!(matches!(
        compile(&body, &mut 100),
        Err(Error::Unsupported(_))
    ));
    let invalid = expr(K::Break(LocalId { owner: 1, index: 0 }));
    assert!(matches!(
        compile(&invalid, &mut 100),
        Err(Error::InvalidControl(_))
    ));
}
#[test]
fn compiler_control_nesting_does_not_use_the_host_stack() {
    let mut body = expr(K::Unit);
    for _ in 0..2048 {
        body = expr(K::Block {
            statements: vec![],
            result: Some(Box::new(body)),
        });
    }
    let code = compile(&body, &mut 30_000).unwrap();
    assert!(code.instructions().len() > 4096);
    drop(code);
    // Release the test input iteratively as well; Rust's generated recursive
    // destructor is unrelated to the compiler's traversal stack.
    loop {
        match body.kind {
            K::Block {
                result: Some(child),
                ..
            } => body = *child,
            _ => break,
        }
    }
}
