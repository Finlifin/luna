use super::*;
use crate::{DeclarationId, Primitive, Sort, program::*};

fn source() -> SourceRange {
    SourceRange { start: 10, end: 20 }
}
fn binding(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn local(index: u32) -> Expr {
    Expr {
        source: SourceRange { start: 30, end: 40 },
        ty: boolean(),
        kind: ExprKind::Local(binding(index)),
    }
}
fn declaration() -> FunctionDeclaration {
    FunctionDeclaration {
        source: source(),
        declaration: DeclarationId {
            package: 1,
            local: 1,
        },
        name: "frame".into(),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        signature: Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::Runtime,
                    ty: boolean()
                };
                3
            ],
            result: Box::new(boolean()),
        },
        parameters: (0..3).map(binding).collect(),
        defaults: vec![(1, local(0)), (2, local(1))],
        preconditions: vec![],
        postconditions: vec![],
        entry_snapshots: vec![],
        result_binding: None,
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
    }
}

#[test]
fn declaration_frame_admission_requires_no_body_and_borrows_the_original_telescope() {
    let declaration = declaration();
    let view = CallableProgram::declaration(&declaration);
    let parameters = view.validate_input_frame(&mut 1000).unwrap();
    let Term::Function {
        parameters: original,
        ..
    } = &declaration.signature
    else {
        unreachable!()
    };
    assert!(std::ptr::eq(parameters, original.as_slice()));
    assert_eq!(
        view.roots().count(),
        2,
        "only defaults are present; there is no executable body"
    );
    assert!(view.to_string().ends_with("(body))"));
}

#[test]
fn declaration_defaults_observe_only_their_completed_prefix_and_local_bindings() {
    let mut declaration = declaration();
    for unavailable in [1, 2, 99] {
        declaration.defaults[0].1 = local(unavailable);
        let error = CallableProgram::declaration(&declaration)
            .validate_input_frame(&mut 1000)
            .unwrap_err();
        assert_eq!(error.kind, InputFrameErrorKind::UnavailableDefaultBinding);
        assert_eq!(error.source, declaration.defaults[0].1.source);
    }
    declaration.defaults[0].1 = Expr {
        source: source(),
        ty: boolean(),
        kind: ExprKind::Block {
            statements: vec![Expr {
                source: source(),
                ty: Term::Primitive(Primitive::Void),
                kind: ExprKind::Let {
                    binding: binding(99),
                    value: Box::new(local(0)),
                },
            }],
            result: Some(Box::new(local(99))),
        },
    };
    CallableProgram::declaration(&declaration)
        .validate_input_frame(&mut 1000)
        .unwrap();
}

#[test]
fn invalid_frames_do_not_collapse_into_budget_failures() {
    for (case, expected) in [
        (0, InputFrameErrorKind::Signature),
        (1, InputFrameErrorKind::Arity),
        (2, InputFrameErrorKind::DuplicateBinding),
        (3, InputFrameErrorKind::DefaultSlot),
        (4, InputFrameErrorKind::DefaultSlot),
    ] {
        let mut declaration = declaration();
        match case {
            0 => declaration.signature = boolean(),
            1 => {
                declaration.parameters.pop();
            }
            2 => declaration.parameters[1] = declaration.parameters[0],
            3 => declaration.defaults[1].0 = 1,
            _ => declaration.defaults[1].0 = 3,
        }
        let view = CallableProgram::declaration(&declaration);
        let error = view.validate_input_frame(&mut 1000).unwrap_err();
        assert_eq!(error.kind, expected);
        assert!(!error.is_budget());
        assert_eq!(error.source, source());
        assert!(view.validate_input_frame(&mut 0).unwrap_err().is_budget());
        assert_eq!(view.validate_input_frame(&mut 1000).unwrap_err(), error);
    }
}

#[test]
fn declaration_frame_budget_covers_defaults_and_retries_without_published_state() {
    let declaration = declaration();
    let view = CallableProgram::declaration(&declaration);
    let mut remaining = 1000;
    view.validate_input_frame(&mut remaining).unwrap();
    let cost = 1000 - remaining;
    let mut exact = cost;
    view.validate_input_frame(&mut exact).unwrap();
    assert_eq!(exact, 0);
    let mut default_exhaustion = false;
    for limit in 0..cost {
        let mut fuel = limit;
        let error = view.validate_input_frame(&mut fuel).unwrap_err();
        assert!(error.is_budget());
        assert!(fuel <= limit);
        default_exhaustion |= error.kind == InputFrameErrorKind::DefaultBudget;
    }
    assert!(default_exhaustion);
    view.validate_input_frame(&mut cost.clone()).unwrap();
}
