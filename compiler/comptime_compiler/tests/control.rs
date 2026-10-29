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

#[test]
fn compiled_constants_outlive_their_source_and_charge_payload_copying() {
    let code = {
        let body = Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Str),
            kind: K::Literal(nir::program::Literal::String("x".repeat(4096))),
        };
        assert!(matches!(compile(&body, &mut 100), Err(Error::Budget(_))));
        compile(&body, &mut 20_000).unwrap()
    };
    assert!(code.storage_work() >= 4096);
    let comptime_compiler::Op::Constant { value, .. } = &code.instructions()[0].op else {
        panic!("missing owned constant")
    };
    assert_eq!(
        value.kind,
        K::Literal(nir::program::Literal::String("x".repeat(4096)))
    );
}

#[test]
fn deeply_nested_constant_types_fail_before_recursive_copying() {
    let mut ty = Term::Primitive(Primitive::Bool);
    for _ in 0..192 {
        ty = Term::Pointer(Box::new(ty));
    }
    let body = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(ty),
    };
    assert!(matches!(
        compile(&body, &mut 100_000),
        Err(Error::Budget(_))
    ));
}

#[test]
fn invocation_code_distinguishes_normal_form_application() {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Type)),
    };
    let mut body = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::Call {
            callee: Box::new(Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: Term::ComptimeFunction(Box::new(signature.clone())),
                kind: K::Local(LocalId { owner: 1, index: 0 }),
            }),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    let dump = compile(&body, &mut 1000).unwrap().to_string();
    assert!(dump.contains("(begin-call "));
    assert!(dump.contains("(invoke "));
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
    let K::Call { callee, .. } = &mut body.kind else {
        unreachable!()
    };
    callee.ty = Term::NormalFormFunction(Box::new(signature), None);
    assert!(matches!(
        compile(&body, &mut 1000),
        Err(Error::Unsupported(_))
    ));
}
