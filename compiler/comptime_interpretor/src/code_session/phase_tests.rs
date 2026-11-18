//! The production session reuses phase declarations, never checked entry bodies.
use super::*;
use nir::program::{Function, FunctionPhase};

fn flag(value: bool) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(Literal::Bool(value)),
    }
}
fn package() -> Arc<Package> {
    Arc::new(Package {
        functions: (0..100)
            .map(|local| Function {
                interface: nir::program::FunctionDeclaration {
                    declaration: DeclarationId { package: 8, local },
                    name: format!("function{local}"),
                    phase: if local == 99 {
                        FunctionPhase::Runtime
                    } else {
                        FunctionPhase::Comptime
                    },
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    signature: Term::Function {
                        parameters: vec![],
                        result: Box::new(flag(true).ty),
                    },
                    parameters: vec![],
                    type_parameters: vec![],
                    defaults: vec![],
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    entry_snapshots: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                },
                body: flag(true),
            })
            .collect(),
        ..Default::default()
    })
}
fn unavailable<T>(_: &Expr, _: &mut u64) -> Result<T, Error> {
    Err(Error {
        kind: ErrorKind::UnmaterializedDispatch,
        source: None,
    })
}

#[test]
fn fresh_session_vms_reuse_phase_declarations_and_recheck_each_expression() {
    let source = package();
    let types = |_: &nir::program::TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    for resolving in [false, true] {
        let mut session = CodeSession::new(source.clone());
        let mut cold_cost = 0;
        let mut entry_cost = None;
        for (index, value) in [true, false, true].into_iter().enumerate() {
            let mut vm = if resolving {
                Interpreter::for_comptime_resolving_in_session(
                    &source,
                    &mut session,
                    SourceResolvers {
                        types: &types,
                        methods: &unavailable,
                        associated: &unavailable,
                    },
                    &mut Budget::default(),
                )
            } else {
                Interpreter::for_comptime_in_session(&source, &mut session, &mut Budget::default())
            }
            .unwrap();
            let preparation_cost = Budget::default().steps - vm.remaining_steps();
            let before = vm.remaining_steps();
            assert_eq!(
                vm.invoke_comptime_expression(&flag(value)).unwrap(),
                Value::Bool(value)
            );
            let cost = before - vm.remaining_steps();
            if index == 0 {
                cold_cost = preparation_cost;
            } else {
                assert!(
                    preparation_cost < cold_cost,
                    "fresh VMs must inherit only the source directory"
                );
            }
            if let Some(previous) = entry_cost {
                assert_eq!(
                    cost, previous,
                    "the first body already has prepared metadata"
                );
            }
            entry_cost = Some(cost);
            let target = &source.functions[99];
            let invalid = Expr {
                kind: K::Call {
                    callee: Box::new(Expr {
                        ty: target.callable_type(),
                        kind: K::Function(target.declaration),
                        ..flag(true)
                    }),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
                ..flag(true)
            };
            assert!(matches!(
                vm.invoke_comptime_expression(&invalid).unwrap_err().kind,
                ErrorKind::InvalidPhase(_)
            ));
            vm.budget.steps = 1;
            assert_eq!(
                vm.invoke_comptime_expression(&flag(value))
                    .unwrap_err()
                    .kind,
                ErrorKind::StepsExhausted
            );
            vm.budget.steps = 1000;
            assert_eq!(
                vm.invoke_comptime_expression(&flag(!value)).unwrap(),
                Value::Bool(!value)
            );
        }
    }
}

#[test]
fn borrowed_vm_keeps_constructor_metadata_without_caching_expression_results() {
    let source = package();
    let mut vm = Interpreter::for_comptime(&source, Budget::default()).unwrap();
    vm.budget.steps = 1;
    assert_eq!(
        vm.invoke_comptime_expression(&flag(true)).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(
        vm.phase_inspection
            .as_ref()
            .unwrap()
            .to_string()
            .contains("(declarations ready)")
    );
    vm.budget.steps = 100_000;
    assert_eq!(
        vm.invoke_comptime_expression(&flag(true)).unwrap(),
        Value::Bool(true)
    );
    let cost = 100_000 - vm.remaining_steps();
    let before = vm.remaining_steps();
    assert_eq!(
        vm.invoke_comptime_expression(&flag(false)).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(before - vm.remaining_steps(), cost);
}
