//! Program facts observe characters in Unicode scalar order, not storage bits.
use super::*;
use nir::{DeclarationId, Parameter, Primitive, Sort, program::FunctionPhase};

fn expr(kind: K, ty: Primitive) -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(ty),
        kind,
    }
}
fn local() -> Expr {
    expr(K::Local(LocalId { owner: 1, index: 0 }), Primitive::Char)
}
fn character(value: char, frozen: bool) -> Expr {
    expr(
        if frozen {
            K::ComptimeValue(Term::Char(value))
        } else {
            K::Literal(Literal::Char(value))
        },
        Primitive::Char,
    )
}
fn equality(value: char, frozen: bool) -> Expr {
    expr(
        K::ScalarEqual(Box::new(local()), Box::new(character(value, frozen))),
        Primitive::Bool,
    )
}
fn function(preconditions: Vec<Expr>, postconditions: Vec<Expr>) -> Function {
    Function {
        interface: nir::program::FunctionDeclaration {
            source: nir::program::SourceRange { start: 0, end: 0 },
            declaration: DeclarationId {
                package: 5,
                local: 1,
            },
            name: "character contract".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: local().ty,
                }],
                result: Box::new(Term::Primitive(Primitive::Void)),
            },
            parameters: vec![LocalId { owner: 1, index: 0 }],
            env_parameters: vec![],
            type_parameters: vec![],
            defaults: vec![],
            preconditions,
            postconditions,
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        },
        body: expr(K::Unit, Primitive::Void),
        runtime_deliveries: Default::default(),
    }
}

#[test]
fn literal_and_frozen_characters_agree_without_discarding_failed_obligations() {
    for value in ['\0', '界', '\u{d7ff}', '\u{e000}', '\u{10ffff}'] {
        for frozen in [false, true] {
            analyze(
                &function(
                    vec![equality(value, frozen)],
                    vec![equality(value, !frozen)],
                ),
                &mut Limits::default(),
            )
            .unwrap();
            let invalid = function(vec![equality(value, frozen)], vec![equality('x', !frozen)]);
            assert_eq!(
                analyze(&invalid, &mut Limits::default())
                    .err()
                    .unwrap()
                    .reason,
                Reason::MissingPostconditionProof
            );
        }
    }
}

#[test]
fn character_order_bounds_are_conservative_and_writes_invalidate_prior_facts() {
    let lower = expr(
        K::ScalarGreaterEqual(Box::new(local()), Box::new(character('\0', false))),
        Primitive::Bool,
    );
    let upper = expr(
        K::ScalarLessEqual(Box::new(local()), Box::new(character('\u{10ffff}', false))),
        Primitive::Bool,
    );
    analyze(
        &function(vec![], vec![lower, upper]),
        &mut Limits::default(),
    )
    .unwrap();
    let mut changed = function(vec![equality('界', false)], vec![equality('界', false)]);
    changed.body = expr(
        K::Assign {
            place: nir::program::Place {
                root: LocalId { owner: 1, index: 0 },
                fields: vec![],
            },
            value: Box::new(character('x', false)),
        },
        Primitive::Void,
    );
    assert_eq!(
        analyze(&changed, &mut Limits::default())
            .err()
            .unwrap()
            .reason,
        Reason::MissingPostconditionProof
    );
}
