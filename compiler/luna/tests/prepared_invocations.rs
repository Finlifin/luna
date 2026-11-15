#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;
use luna::invocation::PreparedInvocation;
use std::sync::Arc;

#[test]
fn entry_selection_has_a_caller_budget_and_recovers_after_exhaustion() {
    let mut package = support::compile("comptime fn run()->bool {true}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .clone();
    for slot in 0..64 {
        let mut unrelated = function.clone();
        unrelated.name = format!("unrelated_{slot}");
        unrelated.declaration.local += 1000 + slot;
        package.functions.push(unrelated);
    }
    let package = Arc::new(package);
    let mut small = 1;
    let error = PreparedInvocation::new_in_budget(package.clone(), "run", None, &mut small)
        .err()
        .unwrap();
    assert!(
        error.contains("entry selection budget exhausted"),
        "{error}"
    );
    assert_eq!(small, 0);
    let error = luna::invocation::invoke(&package, "run", 1).unwrap_err();
    assert!(
        error.contains("entry selection budget exhausted"),
        "{error}"
    );
    let mut budget = 10_000;
    PreparedInvocation::new_in_budget(package.clone(), "run", None, &mut budget).unwrap();
    let cost = 10_000 - budget;
    assert!(cost > package.functions.len() as u64);
    let mut exact = cost;
    let mut invocation =
        PreparedInvocation::new_in_budget(package.clone(), "run", None, &mut exact).unwrap();
    assert_eq!(exact, 0);
    assert!(PreparedInvocation::new_in_budget(package, "run", None, &mut (cost - 1)).is_err());
    assert_eq!(invocation.invoke(100_000).unwrap(), Value::Bool(true));
}

#[test]
fn wide_default_selection_is_linear_and_rejects_malformed_slots() {
    use nir::Term;
    let mut package = support::compile("comptime fn run(.flag:bool=true)->bool {true}");
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.name == "run")
        .unwrap();
    let parameter = function.parameters[0];
    let value = function.defaults[0].1.clone();
    let Term::Function { parameters, .. } = &mut function.signature else {
        panic!("function signature")
    };
    *parameters = vec![parameters[0].clone(); 256];
    function.parameters = (0..256)
        .map(|slot| nir::program::LocalId {
            index: parameter.index + slot,
            ..parameter
        })
        .collect();
    function.defaults = (0..256).map(|slot| (slot, value.clone())).collect();
    let mut remaining = 2048;
    let mut invocation =
        PreparedInvocation::new_in_budget(Arc::new(package.clone()), "run", None, &mut remaining)
            .unwrap();
    assert!(
        remaining > 0,
        "selection must not scan all defaults for each parameter"
    );
    assert_eq!(invocation.invoke(100_000).unwrap(), Value::Bool(true));
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.name == "run")
        .unwrap();
    function.defaults[255].0 = 0;
    let error = PreparedInvocation::new(Arc::new(package), "run", None)
        .err()
        .unwrap();
    assert!(
        error.contains("duplicate entry default parameter"),
        "{error}"
    );
}

#[test]
fn prepared_calls_reuse_code_without_reusing_results_or_local_state() {
    let package = Arc::new(support::compile(
        "comptime fn run()->bool {
        var state=false; state=not state; state
    }",
    ));
    let mut invocation = PreparedInvocation::new(package, "run", None).unwrap();
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert!(invocation.last_stats().bytecode_compilations > 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert_eq!(invocation.last_stats().bytecode_compilations, 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
    assert!(invocation.invoke(1).is_err());
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert_eq!(invocation.last_stats().bytecode_compilations, 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
}

#[test]
fn prepared_default_type_inputs_preserve_checked_dispatch() {
    let package = Arc::new(support::compile(
        "typealias Type=.Type'builtin;
        trait Probe {comptime fn probe()->bool;}
        impl Probe for bool {comptime fn probe()->bool {true}}
        comptime fn run(.T:Type=bool)->bool {if T==bool {bool.as(Probe).probe()} else {false}}",
    ));
    let mut invocation = PreparedInvocation::new(package, "run", None).unwrap();
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert!(invocation.last_stats().bytecode_compilations > 0);
    assert_eq!(invocation.invoke(100000).unwrap(), Value::Bool(true));
    assert_eq!(invocation.last_stats().bytecode_compilations, 0);
    assert!(invocation.last_stats().bytecode_invocations > 0);
}

#[test]
fn default_entries_retain_the_original_source_and_release_it_after_use() {
    let package = Arc::new(support::compile(
        "comptime fn run(.first:bool=true,.second:bool=not first)->bool {
            var local=second; local=not local; first and local
        }",
    ));
    assert!(package.dispatch_instances.is_empty());
    let original = package.to_string();
    let weak = Arc::downgrade(&package);
    let mut invocation = PreparedInvocation::new(package.clone(), "run", None).unwrap();
    assert!(
        Arc::strong_count(&package) > 1,
        "entry preparation must retain the source, not a copy with an invented declaration"
    );
    assert!(invocation.invoke(1).is_err());
    for round in 0..3 {
        assert_eq!(invocation.invoke(100_000).unwrap(), Value::Bool(true));
        if round > 0 {
            assert_eq!(invocation.last_stats().bytecode_compilations, 0);
        }
        assert_eq!(package.to_string(), original);
    }
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    drop(package);
    assert!(weak.upgrade().is_some());
    drop(invocation);
    assert!(weak.upgrade().is_none());
}

#[test]
fn default_entry_failure_does_not_become_cached_success_or_contaminate_another_source() {
    let package = Arc::new(
        sysroot::compile("comptime fn run(.value:Integer=1.div_trunc(0))->bool {true}").unwrap(),
    );
    let mut failed = PreparedInvocation::new(package, "run", None).unwrap();
    for _ in 0..2 {
        let error = failed.invoke(1_000_000).unwrap_err();
        assert!(error.contains("DivisionByZero"), "{error}");
    }
    let package =
        Arc::new(sysroot::compile("comptime fn run(.value:Integer=1)->bool {value==1}").unwrap());
    let mut recovered = PreparedInvocation::new(package, "run", None).unwrap();
    assert_eq!(recovered.invoke(1_000_000).unwrap(), Value::Bool(true));
    let error = failed.invoke(1_000_000).unwrap_err();
    assert!(error.contains("DivisionByZero"), "{error}");
}

#[test]
fn default_entries_preserve_unrelated_runtime_traits_in_the_source() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         comptime fn run(.flag:bool=true)->bool {flag}",
    );
    let original = package.to_string();
    for _ in 0..2 {
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(package.to_string(), original);
    }
}

#[test]
fn declaration_view_ignores_selected_tables_without_copying_or_authorizing_them() {
    let clean = Arc::new(support::compile(
        "comptime fn run(.flag:bool=true)->bool {flag}",
    ));
    let mut package = clean.as_ref().clone();
    package
        .dispatch_instances
        .push(nir::program::DispatchInstance {
            interface: nir::DeclarationId {
                package: 999,
                local: 1,
            },
            interface_arguments: vec![nir::Term::Tuple(vec![nir::Term::Bool(false); 100_000])],
            subject: nir::Term::Bool(false),
            implementation: nir::DeclarationId {
                package: 999,
                local: 2,
            },
            arguments: vec![],
            methods: vec![],
            associated: vec![],
            associated_initializers: vec![],
        });
    let original = package.to_string();
    let package = Arc::new(package);
    // Physical dispatch payloads do not affect source construction or execution
    // fuel. Every allowance keeps the same success/failure as the clean view.
    for fuel in [1, 16, 64, 128, 512, 1024, 100_000] {
        assert_eq!(
            luna::invocation::invoke(&package, "run", fuel),
            luna::invocation::invoke(&clean, "run", fuel),
            "one-shot fuel={fuel}",
        );
        let mut selected = PreparedInvocation::new(package.clone(), "run", None).unwrap();
        let mut plain = PreparedInvocation::new(clean.clone(), "run", None).unwrap();
        assert_eq!(
            selected.invoke(fuel),
            plain.invoke(fuel),
            "cold fuel={fuel}"
        );
        assert_eq!(selected.invoke(100_000).unwrap(), Value::Bool(true));
        assert_eq!(plain.invoke(100_000).unwrap(), Value::Bool(true));
        assert_eq!(
            selected.invoke(fuel),
            plain.invoke(fuel),
            "warm fuel={fuel}"
        );
    }
    let weak = Arc::downgrade(&package);
    let mut prepared = PreparedInvocation::new(package.clone(), "run", None).unwrap();
    assert_eq!(prepared.invoke(100_000).unwrap(), Value::Bool(true));
    assert_eq!(prepared.invoke(100_000).unwrap(), Value::Bool(true));
    assert_eq!(prepared.last_stats().bytecode_compilations, 0);
    assert_eq!(package.to_string(), original);
    drop(package);
    assert!(
        weak.upgrade().is_some(),
        "the session retains its exact source owner"
    );
    assert_eq!(prepared.invoke(100_000).unwrap(), Value::Bool(true));
    drop(prepared);
    assert!(weak.upgrade().is_none());
}

#[test]
fn source_invocation_preserves_type_dispatch_higher_order_values_and_fresh_frames() {
    let package = Arc::new(
        sysroot::compile(
            "
        pure comptime trait Flag {
            pure comptime fn flag() -> bool;
            assoc initial: bool;
        }
        comptime impl Flag for bool {
            pure comptime fn flag() -> bool { true }
            assoc initial: bool = true;
        }
        comptime impl Flag for u8 {
            pure comptime fn flag() -> bool { false }
            assoc initial: bool = false;
        }
        comptime fn selected(comptime T: Type) -> comptime fn() -> bool
        where requires T :- Flag
        {
            T.as(Flag).flag
        }
        comptime fn apply(action: comptime fn() -> bool) -> bool { action() }
        comptime fn direct(comptime T: Type) -> bool
        where requires T :- Flag
        {
            T.as(Flag).flag() == T.as(Flag).initial
        }
        comptime fn source_entry() -> bool {
            var state = false;
            state = not state;
            let first = selected(bool);
            let second = selected(u8);
            state and apply(first) and not apply(second) and direct(bool) and direct(u8)
        }
        ",
        )
        .unwrap(),
    );
    let mut call = PreparedInvocation::new(package.clone(), "source_entry", None).unwrap();
    assert!(call.invoke(1).is_err());
    for _ in 0..3 {
        assert_eq!(call.invoke(1_000_000).unwrap(), Value::Bool(true));
        assert!(call.last_stats().bytecode_invocations > 0);
    }
    assert_eq!(
        luna::invocation::invoke(&package, "source_entry", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn source_sessions_reuse_metadata_without_reusing_closed_dispatch_or_host_authority() {
    use comptime_interpretor::{
        Budget, CodeSession, ContractOutcome, ErrorKind, Interpreter, SourceResolvers,
    };
    use std::cell::Cell;
    let mut package = support::compile(
        "trait Probe { comptime fn probe() -> bool; }
        impl Probe for bool { comptime fn probe() -> bool { true } }
        comptime fn run() -> bool { bool.as(Probe).probe() }",
    );
    package.dispatch_instances.clear();
    let package = Arc::new(package);
    let entry = support::id(&package, "run");
    let mut session = CodeSession::new(package.clone());
    // Warm the legacy mode and executable code in exactly the same snapshot.
    let mut closed =
        Interpreter::for_comptime_in_session(&package, &mut session, &mut Budget::default())
            .unwrap();
    assert_eq!(
        closed
            .with_code_session(&mut session, |vm| vm.invoke(entry, vec![]))
            .unwrap(),
        Value::Bool(true)
    );
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let allow = Cell::new(false);
    let demands = Cell::new(0);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| ContractOutcome::Unknown;
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| {
        demands.set(demands.get() + 1);
        if !allow.get() {
            return Err(comptime_interpretor::Error {
                kind: ErrorKind::TypeContractUnresolved,
                source: Some(callee.source),
            });
        }
        elaboration::materialization::resolve_method_for_invocation(
            &package,
            callee,
            &context,
            &[],
            fuel,
        )
    };
    let associated = |access: &nir::program::Expr, _: &mut u64| {
        Err(comptime_interpretor::Error {
            kind: ErrorKind::UnmaterializedDispatch,
            source: Some(access.source),
        })
    };
    let services = || SourceResolvers {
        types: &solver,
        methods: &methods,
        associated: &associated,
    };
    let mut cold_remaining = 0;
    for (index, permitted) in [false, true, false, true].into_iter().enumerate() {
        allow.set(permitted);
        let before = demands.get();
        let mut budget = Budget::default();
        let mut vm = Interpreter::for_comptime_resolving_in_session(
            &package,
            &mut session,
            services(),
            &mut budget,
        )
        .unwrap();
        assert_eq!(
            demands.get(),
            before,
            "construction cannot select executable methods"
        );
        if index == 0 {
            cold_remaining = budget.steps;
        } else {
            assert!(budget.steps > cold_remaining, "reuse declaration metadata");
        }
        let result = vm.with_code_session(&mut session, |vm| vm.invoke(entry, vec![]));
        if permitted {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractUnresolved);
        }
        assert_eq!(
            demands.get(),
            before + 1,
            "every fresh VM needs current selection evidence"
        );
        assert_eq!(
            vm.bytecode_compilations(),
            0,
            "code reuse grants no selection evidence"
        );
    }
    let mut exhausted = Budget {
        steps: 0,
        ..Default::default()
    };
    assert_eq!(
        Interpreter::for_comptime_resolving_in_session(
            &package,
            &mut session,
            services(),
            &mut exhausted,
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::StepsExhausted
    );
    assert_eq!(exhausted.steps, 0);

    // Identical declaration IDs in another allocation cannot inherit phase
    // validation, even after both construction modes have warmed the session.
    let mut changed = package.as_ref().clone();
    let run = changed
        .functions
        .iter_mut()
        .find(|f| f.declaration == entry)
        .unwrap();
    run.interface.defaults.push((99, run.body.clone()));
    assert!(matches!(
        Interpreter::for_comptime_resolving_in_session(
            &changed,
            &mut session,
            services(),
            &mut Budget::default(),
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::InvalidPhase(_)
    ));
    let mut recovered = Interpreter::for_comptime_resolving_in_session(
        &package,
        &mut session,
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(
        recovered
            .with_code_session(&mut session, |vm| vm.invoke(entry, vec![]))
            .unwrap(),
        Value::Bool(true)
    );
}
