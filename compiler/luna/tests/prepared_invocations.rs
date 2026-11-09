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
    run.defaults.push((99, run.body.clone()));
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
