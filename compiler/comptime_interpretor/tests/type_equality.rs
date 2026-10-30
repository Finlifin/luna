use comptime_interpretor::{Budget, ErrorKind, Event, Interpreter, Value};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
fn value(ty: Term) -> Expr {
    expr(p(P::Type), K::TypeValue(ty))
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        declaration: id(local),
        name: format!("Type{local}"),
        phase: FunctionPhase::Comptime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(body.ty.clone()),
        },
        parameters: vec![],
        defaults: vec![],
        body,
    }
}
fn call(local: u32) -> Expr {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(p(P::Type)),
    }
    .with_comptime_phase(true);
    expr(
        p(P::Type),
        K::Call {
            parameter_bindings: vec![],
            callee: Box::new(expr(signature, K::Function(id(local)))),
            arguments: vec![],
        },
    )
}

#[test]
fn absence_of_equality_proof_is_not_a_false_boolean() {
    let package = Package {
        functions: vec![
            function(
                0,
                expr(
                    p(P::Bool),
                    K::TypeEqual(Box::new(value(p(P::Bool))), Box::new(value(p(P::Char)))),
                ),
            ),
            function(
                1,
                expr(
                    p(P::Bool),
                    K::TypeEqual(Box::new(value(p(P::Bool))), Box::new(value(p(P::Bool)))),
                ),
            ),
        ],
        ..Default::default()
    };
    for outcome in [
        comptime_interpretor::ContractOutcome::Unknown,
        comptime_interpretor::ContractOutcome::Rejected,
    ] {
        let solver = |_: &nir::program::TypePredicate| outcome;
        let mut interpreter = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_type_solver(&solver);
        assert_eq!(
            interpreter.invoke(id(0), vec![]).unwrap_err().kind,
            ErrorKind::TypeContractUnresolved
        );
        assert_eq!(
            interpreter.invoke(id(1), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn disequality_requires_its_own_proof_and_operands_execute_once_in_order() {
    let package = Package {
        functions: vec![
            function(
                0,
                expr(
                    p(P::Bool),
                    K::TypeEqual(Box::new(call(1)), Box::new(call(2))),
                ),
            ),
            function(1, value(p(P::Bool))),
            function(2, value(p(P::Char))),
        ],
        ..Default::default()
    };
    let calls = std::cell::Cell::new(0);
    let solver = |predicate: &nir::program::TypePredicate| {
        calls.set(calls.get() + 1);
        match predicate {
            nir::program::TypePredicate::Not(_) => comptime_interpretor::ContractOutcome::Satisfied,
            _ => comptime_interpretor::ContractOutcome::Rejected,
        }
    };
    let mut interpreter = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_type_solver(&solver);
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(calls.get(), 2);
    let entered: Vec<_> = interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(entered, vec![id(0), id(1), id(2)]);
}

#[test]
fn malformed_operands_and_comparison_budgets_fail_without_fabricating_results() {
    let bad = expr(
        p(P::Bool),
        K::TypeEqual(
            Box::new(expr(
                p(P::Bool),
                K::Literal(nir::program::Literal::Bool(true)),
            )),
            Box::new(call(1)),
        ),
    );
    let package = Package {
        functions: vec![
            function(0, bad),
            function(1, value(p(P::Bool))),
            function(
                2,
                expr(
                    p(P::Bool),
                    K::TypeEqual(Box::new(value(p(P::Bool))), Box::new(value(p(P::Bool)))),
                ),
            ),
        ],
        ..Default::default()
    };
    let mut interpreter = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        interpreter.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::Type
    );
    assert!(!interpreter.events().contains(&Event::EnterFunction(id(1))));
    let mut bounded = Interpreter::new(&package, Budget::default()).unwrap();
    bounded
        .charge_preparation((bounded.remaining_steps() - 1) as usize)
        .unwrap();
    assert_eq!(
        bounded.invoke(id(2), vec![]).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(
        interpreter.invoke(id(2), vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn runtime_direct_calls_prove_residual_clauses_and_recover_after_rejection() {
    use nir::program::{Literal, TraitRequirement, TypePredicate};
    let runtime = |local, requirement| {
        let mut f = function(local, expr(p(P::Bool), K::Literal(Literal::Bool(true))));
        f.phase = FunctionPhase::Runtime;
        f.requirements = vec![requirement];
        f
    };
    let mut missing_trait = runtime(4, TypePredicate::Bool(true));
    missing_trait.capabilities.push(TraitRequirement {
        interface: id(90),
        interface_arguments: vec![],
        subject: p(P::Bool),
    });
    let mut generic = runtime(5, TypePredicate::Bool(true));
    generic.type_parameters.push(id(91));
    let package = Package {
        functions: vec![
            runtime(0, TypePredicate::Equal(p(P::Bool), p(P::Bool))),
            runtime(1, TypePredicate::Bool(false)),
            runtime(
                2,
                TypePredicate::Equal(
                    Term::Rigid {
                        declaration: id(92),
                        sort: nir::Sort::Type,
                    },
                    p(P::Bool),
                ),
            ),
            runtime(3, TypePredicate::Equal(p(P::Bool), p(P::U8))),
            missing_trait,
            generic,
        ],
        ..Default::default()
    };
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    for (local, expected) in [
        (1, ErrorKind::TypeContractRejected),
        (2, ErrorKind::TypeContractUnresolved),
        // Without a supplied solver, distinct types do not manufacture a
        // disequality proof; preserve the existing unknown result.
        (3, ErrorKind::TypeContractUnresolved),
        (4, ErrorKind::InvalidCallPlan),
        (5, ErrorKind::Type),
    ] {
        assert_eq!(vm.invoke(id(local), vec![]).unwrap_err().kind, expected);
        assert_eq!(vm.invoke(id(0), vec![]).unwrap(), Value::Bool(true));
    }
    let mut comptime = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        comptime.invoke(id(0), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}
