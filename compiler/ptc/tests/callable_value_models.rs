//! Callable NF inputs retain identity without executing code or selecting an impl.
use nir::{DeclarationId, Primitive as P, Sort, Term, TraitMethodIdentity};
use ptc::{Budget, Context, Formula as F, Outcome, TypeInputOutcome as Search};

fn id(local: u32) -> DeclarationId {
    DeclarationId {
        package: 985,
        local,
    }
}
fn input(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    }
}
fn apply(value: Term) -> Term {
    Term::NormalForm {
        head: id(10),
        sort: Sort::Type,
        arguments: vec![value],
    }
}
fn function(ty: Term) -> Term {
    Term::FunctionValue {
        declaration: id(20),
        arguments: vec![ty, Term::Bool(true)],
    }
}
fn method(ty: Term) -> Term {
    Term::MethodValue(Box::new(TraitMethodIdentity {
        interface: nir::program::InterfaceInstance {
            declaration: id(30),
            arguments: vec![Term::Bool(true), ty.clone()],
        },
        subject: ty.clone(),
        member: id(31),
        method_arguments: vec![ty],
    }))
}

#[test]
fn callable_packets_preserve_symbolic_type_inputs_and_replay() {
    for packet in [function(input(9)), method(input(9))] {
        let mut context = Context::default();
        context.admit_congruence(id(10));
        let expected = apply(packet);
        let constraint = F::equal(input(0), expected.clone());
        let outcome =
            context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
        let Search::Unique(solution) = outcome else {
            panic!("{outcome}")
        };
        assert_eq!(solution.bindings[&id(0)].0, expected);
        let problem = context
            .type_input_context(&[id(0)], constraint, &mut Budget::default())
            .unwrap();
        problem
            .verify_witness(&solution.model, Budget::default())
            .unwrap();
        problem
            .verify_proof(&solution.bindings[&id(0)].1, Budget::default())
            .unwrap();
        let mut corrupted = solution.model;
        let argument = &mut corrupted.applications[0].arguments[0];
        match argument {
            Term::FunctionValue { declaration, .. } => *declaration = id(21),
            Term::MethodValue(method) => method.member = id(32),
            _ => panic!(),
        }
        assert!(
            problem
                .verify_witness(&corrupted, Budget::default())
                .is_err()
        );
        assert!(context.assumptions().is_empty());
    }
}

#[test]
fn callable_congruence_is_forward_only_and_preserves_parameter_groups() {
    for packet in [function as fn(Term) -> Term, method] {
        let mut context = Context::default();
        context.admit_congruence(id(10));
        context.assume(F::equal(input(1), input(2)));
        let result = context.prove(
            F::equal(apply(packet(input(1))), apply(packet(input(2)))),
            Budget::default(),
        );
        let Outcome::Entailed(proof) = result else {
            panic!("{result}")
        };
        context.verify_proof(&proof, Budget::default()).unwrap();

        let mut context = Context::default();
        context.admit_congruence(id(10));
        context.assume(F::equal(apply(packet(input(1))), apply(packet(input(2)))));
        let result = context.prove(F::equal(input(1), input(2)), Budget::default());
        let Outcome::NotEntailed(witness) = result else {
            panic!("{result}")
        };
        context.verify_witness(&witness, Budget::default()).unwrap();
        assert_ne!(witness.types[&id(1)], witness.types[&id(2)]);
        let value = packet(Term::Primitive(P::Bool));
        assert!(matches!(
            context.prove(F::equal(value.clone(), value), Budget::default()),
            Outcome::IllFormed(_)
        ));
    }

    let original = method(Term::Primitive(P::Bool));
    let Term::MethodValue(identity) = &original else {
        unreachable!()
    };
    for change in 0..4 {
        let mut changed = (**identity).clone();
        match change {
            0 => changed.member = id(32),
            1 => changed.interface.declaration = id(33),
            2 => changed.subject = Term::Primitive(P::U8),
            _ => changed
                .method_arguments
                .insert(0, changed.interface.arguments.pop().unwrap()),
        }
        let mut context = Context::default();
        context.admit_congruence(id(10));
        let result = context.prove(
            F::equal(
                apply(original.clone()),
                apply(Term::MethodValue(Box::new(changed))),
            ),
            Budget::default(),
        );
        let Outcome::NotEntailed(witness) = result else {
            panic!("{change}: {result}")
        };
        context.verify_witness(&witness, Budget::default()).unwrap();
    }
}

#[test]
fn callable_models_retain_lexical_type_bindings() {
    let mut context = Context::with_scope(vec![vec![Sort::Type]]);
    context.admit_congruence(id(10));
    let ty = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    for packet in [function(ty.clone()), method(ty)] {
        let expected = apply(packet);
        let constraint = F::equal(input(0), expected.clone());
        let outcome =
            context.search_type_inputs(&[id(0)], constraint.clone(), &mut Budget::default());
        let Search::Unique(solution) = outcome else {
            panic!("{outcome}")
        };
        assert_eq!(solution.bindings[&id(0)].0, expected);
        let problem = context
            .type_input_context(&[id(0)], constraint, &mut Budget::default())
            .unwrap();
        problem
            .verify_witness(&solution.model, Budget::default())
            .unwrap();
        problem
            .verify_proof(&solution.bindings[&id(0)].1, Budget::default())
            .unwrap();
    }
}

#[test]
fn unavailable_callable_payloads_and_exhausted_budgets_do_not_leak_models() {
    let mut context = Context::default();
    context.admit_congruence(id(10));
    for argument in [Term::Value { store: 1, index: 0 }, Term::Float64(0)] {
        let outcome = context.search_type_inputs(
            &[id(0)],
            F::equal(input(0), apply(function(argument))),
            &mut Budget::default(),
        );
        assert!(matches!(outcome, Search::Unknown(_)), "{outcome}");
    }
    for packet in [function(input(9)), method(input(9))] {
        for steps in [0, 1, 20] {
            let outcome = context.search_type_inputs(
                &[id(0)],
                F::equal(input(0), apply(packet.clone())),
                &mut Budget {
                    steps,
                    ..Budget::default()
                },
            );
            assert!(
                matches!(
                    outcome,
                    Search::Unknown(ptc::UnknownReason::BudgetExhausted)
                ),
                "{outcome}"
            );
        }
        assert!(matches!(
            context.search_type_inputs(
                &[id(0)],
                F::equal(input(0), apply(packet)),
                &mut Budget::default()
            ),
            Search::Unique(_)
        ));
    }
    assert!(context.assumptions().is_empty());
}

#[test]
fn callable_payloads_cannot_bypass_input_depth_and_work_limits() {
    let mut context = Context::default();
    context.admit_congruence(id(10));
    for packet in [function as fn(Term) -> Term, method] {
        let mut value = Term::Bool(true);
        for _ in 0..40 {
            value = function(value);
        }
        let goal = F::equal(apply(packet(value)), Term::Primitive(P::Bool));
        assert!(matches!(
            context.prove(
                goal,
                Budget {
                    max_depth: 16,
                    ..Budget::default()
                }
            ),
            Outcome::Unknown {
                reason: ptc::UnknownReason::BudgetExhausted,
                ..
            }
        ));
    }
    let value = Term::FunctionValue {
        declaration: id(20),
        arguments: vec![Term::Bool(true); 1000],
    };
    assert!(matches!(
        context.prove(
            F::equal(apply(value), Term::Primitive(P::Bool)),
            Budget {
                steps: 100,
                ..Budget::default()
            }
        ),
        Outcome::Unknown {
            reason: ptc::UnknownReason::BudgetExhausted,
            ..
        }
    ));
}
