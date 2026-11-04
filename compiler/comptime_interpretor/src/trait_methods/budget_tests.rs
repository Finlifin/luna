use super::*;
use nir::program::{MethodContract, Trait, TraitPhase};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 96, local }
}

#[test]
fn method_host_unwind_and_refill_cannot_leave_budget_or_selection() {
    let signature = Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    }));
    let package = Package {
        traits: vec![Trait {
            declaration: id(1),
            self_parameter: id(2),
            name: "Predicate".into(),
            comptime_only: true,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            associated: vec![],
            methods: vec![MethodContract {
                declaration: id(3),
                name: "accept".into(),
                phase: TraitPhase::Comptime,
                is_quote: false,
                pure: false,
                is_unsafe: false,
                type_arguments: vec![],
                type_parameters: vec![],
                capabilities: vec![],
                program: Default::default(),
                parameter_bindings: vec![],
                by_value_receiver: false,
                by_reference_receiver: false,
                signature: signature.clone().into(),
                requirements: vec![],
                default: None,
            }],
        }],
        ..Default::default()
    };
    let source = SourceRange { start: 1, end: 2 };
    let subject = Term::Primitive(Primitive::Bool);
    for unwinds in [false, true] {
        let calls = std::cell::Cell::new(0);
        let resolver = |_: &Expr, fuel: &mut u64| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                *fuel = u64::MAX;
                if unwinds {
                    panic!("method host interrupted");
                }
            } else {
                *fuel -= 1;
            }
            Err(Error {
                kind: ErrorKind::UnmaterializedDispatch,
                source: Some(source),
            })
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_method_resolver(&resolver);
        let select = |vm: &mut Interpreter<'_>| {
            vm.select_trait_method(
                View {
                    interface: id(1),
                    interface_arguments: &[],
                    subject: &subject,
                    member: id(3),
                    method_arguments: &[],
                    output: &signature,
                    source,
                },
                true,
            )
            .map(|_| ())
        };
        let result = catch_unwind(AssertUnwindSafe(|| select(&mut vm)));
        if unwinds {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().unwrap_err().kind, ErrorKind::StepsExhausted);
        }
        assert_eq!(vm.remaining_steps(), 0);
        assert!(vm.resolved_method_bodies.is_empty());
        assert_eq!(calls.get(), 1);
        vm.budget.steps = Budget::default().steps;
        assert_eq!(
            select(&mut vm).unwrap_err().kind,
            ErrorKind::UnmaterializedDispatch
        );
        assert_eq!(calls.get(), 2);
        assert!(vm.remaining_steps() < Budget::default().steps);
        assert!(vm.resolved_method_bodies.is_empty());
    }
}
