use nir::contracts::{ProgramContract, remap_predicate};
use nir::program::{EntrySnapshot, Expr, ExprKind, LocalId, SourceRange};
use nir::{Primitive, Term};
fn local(owner: u32, index: u32) -> LocalId {
    LocalId { owner, index }
}
fn read(id: LocalId) -> Expr {
    Expr {
        source: SourceRange { start: 7, end: 11 },
        ty: Term::Primitive(Primitive::Bool),
        kind: ExprKind::Local(id),
    }
}
fn contract() -> ProgramContract {
    ProgramContract {
        preconditions: vec![read(local(1, 0))],
        postconditions: vec![read(local(1, 2)), read(local(1, 3)), read(local(1, 4))],
        result_binding: Some(local(1, 2)),
        entry_snapshots: vec![
            EntrySnapshot {
                parameter: local(1, 0),
                binding: local(1, 3),
            },
            EntrySnapshot {
                parameter: local(1, 1),
                binding: local(1, 4),
            },
        ],
    }
}
#[test]
fn alignment_reuses_body_result_and_snapshots_and_retains_missing_observations() {
    let original = contract();
    let plan = original
        .binding_alignment(
            &[local(1, 0), local(1, 1)],
            &[local(2, 0), local(2, 1)],
            Some(local(2, 2)),
            &[EntrySnapshot {
                parameter: local(2, 0),
                binding: local(2, 3),
            }],
            &mut 1000,
        )
        .unwrap();
    assert_eq!(plan.result_binding, Some(local(2, 2)));
    assert_eq!(
        plan.entry_snapshots,
        vec![
            EntrySnapshot {
                parameter: local(2, 0),
                binding: local(2, 3)
            },
            EntrySnapshot {
                parameter: local(2, 1),
                binding: local(1, 4)
            }
        ]
    );
    let mut rewritten = original.clone();
    for predicate in rewritten
        .preconditions
        .iter_mut()
        .chain(&mut rewritten.postconditions)
    {
        remap_predicate(predicate, &plan.aliases).unwrap();
        assert_eq!(predicate.source, SourceRange { start: 7, end: 11 });
    }
    for (predicate, expected) in rewritten
        .preconditions
        .iter()
        .chain(&rewritten.postconditions)
        .zip([local(2, 0), local(2, 2), local(2, 3), local(1, 4)])
    {
        assert!(matches!(predicate.kind,ExprKind::Local(id) if id==expected));
    }
    let dump = plan.to_string();
    assert!(dump.starts_with("(contract-binding-alignment (aliases"));
    assert!(dump.contains("(result-binding (local 2 2))"), "{dump}");
    assert_eq!(original, contract());
}
#[test]
fn alignment_rejects_truncated_duplicate_and_colliding_frames_without_mutation() {
    let program = contract();
    let source = [local(1, 0), local(1, 1)];
    let target = [local(2, 0), local(2, 1)];
    assert!(
        program
            .binding_alignment(&source, &target[..1], None, &[], &mut 1000)
            .unwrap_err()
            .contains("count")
    );
    assert!(
        program
            .binding_alignment(&source, &[local(2, 0); 2], None, &[], &mut 1000)
            .unwrap_err()
            .contains("duplicate")
    );
    assert!(
        program
            .binding_alignment(&source, &target, Some(local(2, 0)), &[], &mut 1000)
            .unwrap_err()
            .contains("collides")
    );
    assert!(
        program
            .binding_alignment(&source, &[local(1, 2), local(2, 1)], None, &[], &mut 1000)
            .unwrap_err()
            .contains("retained interface result")
    );
    assert!(
        program
            .binding_alignment(&source, &[local(1, 3), local(2, 1)], None, &[], &mut 1000)
            .unwrap_err()
            .contains("retained interface snapshot")
    );
    let mut invalid = program.clone();
    invalid.entry_snapshots[0].parameter = local(8, 0);
    assert!(
        invalid
            .binding_alignment(&source, &target, None, &[], &mut 1000)
            .unwrap_err()
            .contains("does not refer")
    );
    invalid = program.clone();
    invalid.entry_snapshots[1].binding = local(1, 3);
    assert!(
        invalid
            .binding_alignment(&source, &target, None, &[], &mut 1000)
            .is_err()
    );
    let mut fuel = 0;
    assert!(
        program
            .binding_alignment(&source, &target, None, &[], &mut fuel)
            .unwrap_err()
            .contains("budget")
    );
    assert_eq!(fuel, 0);
    let aligned = program
        .binding_alignment(&source, &target, None, &[], &mut 1000)
        .unwrap();
    assert_eq!(aligned.result_binding, program.result_binding);
    assert_eq!(program, contract());
}
