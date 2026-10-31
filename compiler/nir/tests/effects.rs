use nir::effects::{Abi, Entry, Frame, Operation};
use nir::{DeclarationId, Term};

#[test]
fn effect_descriptor_dump_preserves_order_identity_abi_and_escaped_inputs() {
    let frame = Frame {
        entries: vec![Entry {
            operation: Operation {
                declaration: DeclarationId {
                    package: 7,
                    local: 3,
                },
                arguments: vec![Term::String("x\n\"\\".into()), Term::Bool(true)],
            },
            abi: Abi::Capturing,
            total: false,
        }],
    };
    assert_eq!(
        frame.to_string(),
        "(effect-frame (entries (effect-entry (operation (declaration (declaration (package 7) (local 3))) (arguments (string \"x\\n\\\"\\\\\") (bool true))) (abi capturing) (total false))))"
    );
    assert_eq!(Abi::Direct.to_string(), "direct");
    assert_eq!(Abi::DirectPassing.to_string(), "direct-passing");
    assert_eq!(Abi::Async.to_string(), "async");
}

#[test]
fn computation_request_dump_and_substitution_keep_all_operation_annotations() {
    use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
    use nir::{Parameter, Primitive, Sort};
    let parameter = DeclarationId {
        package: 3,
        local: 1,
    };
    let ty = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let operation = nir::effects::Operation {
        declaration: DeclarationId {
            package: 3,
            local: 2,
        },
        arguments: vec![ty.clone()],
    };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: ty.clone(),
        }],
        result: Box::new(ty.clone()),
    };
    let source = SourceRange { start: 1, end: 7 };
    let body = Expr {
        ty: ty.clone(),
        source,
        kind: K::EffectRequest {
            operation: operation.clone(),
            abi: Abi::Direct,
            allowed_index: 0,
            signature: Box::new(signature),
            arguments: vec![Expr {
                ty: ty.clone(),
                source,
                kind: K::Local(LocalId { owner: 1, index: 2 }),
            }],
        },
    };
    let computation = Expr {
        ty: Term::computation(vec![operation.clone()], ty),
        source,
        kind: K::EffectComputation {
            effects: vec![operation],
            body: Box::new(body),
        },
    };
    let result = computation
        .instantiate_types(&[parameter], &[Term::Primitive(Primitive::Bool)])
        .unwrap();
    let K::EffectComputation { effects, body } = &result.kind else {
        panic!()
    };
    assert_eq!(effects[0].arguments, [Term::Primitive(Primitive::Bool)]);
    assert_eq!(
        result.ty,
        Term::computation(effects.clone(), body.ty.clone())
    );
    let K::EffectRequest {
        operation,
        signature,
        arguments,
        allowed_index,
        ..
    } = &body.kind
    else {
        panic!()
    };
    assert_eq!(operation, &effects[0]);
    assert_eq!(*allowed_index, 0);
    assert_eq!(arguments[0].ty, Term::Primitive(Primitive::Bool));
    assert!(
        matches!(signature.as_ref(),Term::Function {parameters,result} if parameters[0].ty == arguments[0].ty && **result == body.ty)
    );
    let dump = result.to_string();
    assert!(dump.contains("(effect-computation (allowed (operation"));
    assert!(dump.contains("(effect-request (operation"));
    assert!(dump.contains("(allowed-index 0) (signature (function"));
    assert!(!dump.contains("(rigid"));
}
