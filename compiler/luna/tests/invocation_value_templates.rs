//! Prepared invocation retains live comptime value inputs and resolves their uses.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_executor::Context;
use comptime_interpretor::Value;
use nir::{DeclarationId, Primitive, Term, program::Package};

fn id(package: &Package, name: &str) -> DeclarationId {
    let mut matches = package.functions.iter().filter(|f| f.name == name);
    let declaration = matches.next().expect("named test entry").declaration;
    assert!(matches.next().is_none(), "ambiguous test entry: {name}");
    declaration
}
fn prepare(package: &Package, name: &str, arguments: Vec<Term>) -> luna::memoization::Invocation {
    luna::memoization::Invocation::prepare(
        package,
        &Default::default(),
        id(package, name),
        arguments,
        8,
        Default::default(),
    )
    .unwrap()
}

#[test]
fn memoized_preparation_bounds_source_capture_and_can_share_an_owned_source() {
    use elaboration::materialization::{DeclarationSnapshot, DeclarationSource};
    use luna::memoization::Invocation;
    let package = sysroot::compile("pure comptime fn identity(flag:bool)->bool {flag}").unwrap();
    let entry = id(&package, "identity");
    let error = Invocation::prepare(
        &package,
        &Default::default(),
        entry,
        vec![],
        8,
        elaboration::ptc::Budget {
            steps: 1,
            ..Default::default()
        },
    )
    .err()
    .unwrap();
    assert!(
        error.contains("memoization source retention budget exhausted"),
        "{error}"
    );
    let source = DeclarationSource::Borrowed(&package)
        .retain_declarations(&mut 100_000)
        .unwrap();
    let weak = std::sync::Arc::downgrade(source.package());
    let mut call = Invocation::prepare_in_source(
        source.clone(),
        &Default::default(),
        entry,
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    drop(source);
    for flag in [false, true, false] {
        let result = call
            .invoke(vec![Value::Bool(flag)], &mut Context::new(1_000_000))
            .unwrap();
        assert_eq!(*result.value, Value::Bool(flag));
        assert!(!result.reused);
    }
    assert!(weak.upgrade().is_some());
    drop(call);
    assert!(weak.upgrade().is_none());

    let mut dirty = package;
    dirty
        .dispatch_instances
        .push(nir::program::DispatchInstance {
            interface: entry,
            interface_arguments: vec![],
            subject: Term::Bool(false),
            implementation: entry,
            arguments: vec![],
            methods: vec![],
            associated: vec![],
            associated_initializers: vec![],
        });
    let mut call = Invocation::prepare_in_source(
        DeclarationSnapshot::new(std::sync::Arc::new(dirty)),
        &Default::default(),
        entry,
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    let outcome = call
        .invoke(vec![Value::Bool(true)], &mut Context::new(1_000_000))
        .unwrap();
    assert_eq!(*outcome.value, Value::Bool(true));
    assert!(!outcome.reused);
}

const SOURCE: &str = "
    struct Key where tag: bool { value: bool }
    pure comptime trait Reading {
        pure comptime fn read(self) -> bool;
        assoc initial: bool;
    }
    comptime impl Reading for Key<tag> where tag: bool {
        pure comptime fn read(self) -> bool { self.value == tag }
        assoc initial: bool = tag;
    }
    pure comptime fn inspect_key(value: Key<tag>) -> bool
    where tag: bool
    {
        value.value
    }
    pure comptime fn read_key(comptime enabled: bool, value: Key<tag>) -> bool
    where tag: bool
    {
        if enabled { value.as(Reading).read() } else { Key<tag>.as(Reading).initial }
    }
    pure comptime fn checked_key(comptime enabled: bool, value: Key<tag>) -> bool
    where tag: bool
    {
        assumes enabled;
        value.as(Reading).read()
    }
";

fn key(package: &Package, tag: bool, value: bool) -> Value {
    Value::Record {
        declaration: package
            .structs
            .iter()
            .find(|s| s.name == "Key")
            .unwrap()
            .declaration,
        arguments: vec![Term::Bool(tag)],
        fields: vec![Value::Bool(value)],
    }
}

#[test]
fn live_value_inputs_select_methods_and_associated_values_without_result_caching() {
    let package = sysroot::compile(SOURCE).unwrap();
    for tag in [false, true] {
        let mut inspect = prepare(&package, "inspect_key", vec![Term::Bool(tag)]);
        let mut read = prepare(&package, "read_key", vec![Term::Bool(tag)]);
        for (enabled, value) in [(true, false), (false, true), (true, true), (false, false)] {
            let outcome = inspect
                .invoke(
                    vec![key(&package, tag, value)],
                    &mut Context::new(1_000_000),
                )
                .unwrap();
            assert_eq!(*outcome.value, Value::Bool(value));
            assert!(!outcome.reused);
            let outcome = read
                .invoke(
                    vec![Value::Bool(enabled), key(&package, tag, value)],
                    &mut Context::new(1_000_000),
                )
                .unwrap();
            assert_eq!(
                *outcome.value,
                Value::Bool(if enabled { value == tag } else { tag })
            );
            assert!(!outcome.reused);
            assert_eq!(read.cached_results(), 0);
            assert_eq!(inspect.cached_results(), 0);
        }
        assert!(
            read.invoke(
                vec![Value::Bool(true), key(&package, !tag, true)],
                &mut Context::new(1_000_000)
            )
            .is_err()
        );
        assert_eq!(
            *read
                .invoke(
                    vec![Value::Bool(true), key(&package, tag, tag)],
                    &mut Context::new(1_000_000)
                )
                .unwrap()
                .value,
            Value::Bool(true)
        );
    }
}

#[test]
fn prepared_value_templates_recheck_contracts_and_recover_after_failures() {
    let package = sysroot::compile(SOURCE).unwrap();
    let mut call = prepare(&package, "checked_key", vec![Term::Bool(true)]);
    let inputs = |enabled| vec![Value::Bool(enabled), key(&package, true, true)];
    assert!(call.invoke(inputs(true), &mut Context::new(1)).is_err());
    for enabled in [true, false, true, false, true] {
        let outcome = call.invoke(inputs(enabled), &mut Context::new(1_000_000));
        if enabled {
            let outcome = outcome.unwrap();
            assert_eq!(*outcome.value, Value::Bool(true));
            assert!(!outcome.reused);
        } else {
            let failure = outcome
                .err()
                .expect("assumes must be checked on every call")
                .to_string();
            assert!(failure.contains("ProgramContractRejected"), "{failure}");
        }
        assert_eq!(call.cached_results(), 0);
    }
}

#[test]
fn type_results_use_real_meta_key_protocols_with_comptime_value_inputs() {
    let package = sysroot::compile(
        "pure comptime fn Select(comptime flag: bool) -> Type { if flag {bool} else {u8} }",
    )
    .unwrap();
    let items = key_protocols(&package);
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &items,
        id(&package, "Select"),
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    let mut phase_fuel = 1_000_000;
    nir::phases::Inspection::new(&package)
        .check_package(&mut phase_fuel)
        .unwrap();
    let phase_cost = 1_000_000 - phase_fuel;
    assert!(
        call.invoke(vec![Value::Bool(false)], &mut Context::new(phase_cost - 1))
            .is_err()
    );
    assert_eq!(call.cached_results(), 0);
    for (flag, reused) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut budget = Context::new(1_000_000);
        let outcome = call.invoke(vec![Value::Bool(flag)], &mut budget).unwrap();
        if !flag && !reused {
            let cost = 1_000_000 - budget.remaining();
            assert!(cost >= phase_cost, "cold phase traversal must be paid");
            assert!(
                cost < 2 * phase_cost,
                "key admission and invocation share source preparation"
            );
        }
        assert_eq!(
            *outcome.value,
            Value::Type(Term::Primitive(if flag {
                Primitive::Bool
            } else {
                Primitive::U8
            }))
        );
        assert_eq!(outcome.reused, reused);
    }
    assert_eq!(call.cached_results(), 2);
}

fn key_protocols(package: &Package) -> intrinsic::LangItems {
    let mut items = intrinsic::LangItems::new();
    for (item, name) in [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
    ] {
        let declaration = package
            .traits
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .declaration;
        items
            .bind_source_traits(package, &[(item, declaration)])
            .unwrap();
    }
    items
}

#[test]
fn type_input_dispatch_uses_source_methods_and_real_meta_cache_protocols() {
    let package = sysroot::compile(
        "pure comptime trait Choice {
            pure comptime fn Select() -> Type;
            assoc selected: Type;
        }
        comptime impl Choice for bool {
            pure comptime fn Select() -> Type { u8 }
            assoc selected: Type = u8;
        }
        comptime impl Choice for u8 {
            pure comptime fn Select() -> Type { bool }
            assoc selected: Type = bool;
        }
        pure comptime fn SelectFor(comptime T: Type) -> Type
        where requires T :- Choice
        {
            let select = T.as(Choice).Select;
            let actual = select();
            if actual == T.as(Choice).selected { actual } else { char }
        }
        pure comptime fn Resolve(comptime flag: bool) -> Type {
            if flag { SelectFor(bool) } else { SelectFor(u8) }
        }",
    )
    .unwrap();
    let items = key_protocols(&package);
    let prepare = |capacity| {
        luna::memoization::Invocation::prepare(
            &package,
            &items,
            id(&package, "Resolve"),
            vec![],
            capacity,
            Default::default(),
        )
        .unwrap()
    };
    let inputs = |flag| vec![Value::Bool(flag)];
    let mut call = prepare(8);
    for (input, output, reused) in [
        (true, Primitive::U8, false),
        (false, Primitive::Bool, false),
        (true, Primitive::U8, true),
        (false, Primitive::Bool, true),
    ] {
        let outcome = call
            .invoke(inputs(input), &mut Context::new(1_000_000))
            .unwrap();
        assert_eq!(*outcome.value, Value::Type(Term::Primitive(output)));
        assert_eq!(outcome.reused, reused);
        if reused {
            assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
        } else {
            assert!(call.last_execution_stats().bytecode_invocations > 0);
        }
    }
    assert_eq!(call.cached_results(), 2);
    assert!(
        call.invoke(
            vec![Value::UnsignedInteger(1)],
            &mut Context::new(1_000_000)
        )
        .is_err()
    );
    assert_eq!(call.cached_results(), 2);
    assert!(
        call.invoke(inputs(true), &mut Context::new(1_000_000))
            .unwrap()
            .reused
    );

    // With result retention disabled, every success includes execution and
    // source result admission. Optional proof minimization scales with fuel,
    // so observed warm work is not a lower bound on every successful query.
    let mut uncached = prepare(0);
    uncached
        .invoke(inputs(true), &mut Context::new(1_000_000))
        .unwrap();
    let mut warm = Context::new(1_000_000);
    uncached.invoke(inputs(true), &mut warm).unwrap();
    let work = 1_000_000 - warm.remaining();
    let warm_invocations = uncached.last_execution_stats().bytecode_invocations;
    // Associated resolution returns a fresh initializer body per invocation.
    // Its owned identity is not retained by the stable source-code session.
    // Repeated execution may compile that body, but not the warmed source code.
    let warm_compilations = uncached.last_execution_stats().bytecode_compilations;
    assert!(work > 1);
    assert!(warm_invocations > 0);
    for allowance in [0, 1, work / 2, work - 1] {
        let mut limited = Context::new(allowance);
        let observed = uncached.invoke(inputs(true), &mut limited);
        if allowance <= 1 {
            assert!(observed.is_err(), "input admission requires work");
        }
        if let Ok(outcome) = observed {
            assert_eq!(*outcome.value, Value::Type(Term::Primitive(Primitive::U8)));
            assert!(!outcome.reused);
            assert_eq!(
                uncached.last_execution_stats().bytecode_invocations,
                warm_invocations
            );
            assert_eq!(
                uncached.last_execution_stats().bytecode_compilations,
                warm_compilations
            );
        }
        assert!(limited.remaining() <= allowance);
        assert_eq!(uncached.cached_results(), 0);
        let mut recovered = Context::new(1_000_000);
        let outcome = uncached.invoke(inputs(true), &mut recovered).unwrap();
        assert_eq!(*outcome.value, Value::Type(Term::Primitive(Primitive::U8)));
        assert!(!outcome.reused);
        assert_eq!(1_000_000 - recovered.remaining(), work);
    }
}

#[test]
fn a_value_dependent_ordinary_requirement_is_checked_before_cache_lookup() {
    let package = sysroot::compile(
        "trait Enabled where flag: bool {}
         impl Enabled<true> for bool {}
         pure comptime fn Select(comptime flag: bool) -> Type
         where requires bool :- Enabled<flag>
         {
             bool
         }",
    )
    .unwrap();
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &key_protocols(&package),
        id(&package, "Select"),
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for (enabled, reused) in [(false, false), (true, false), (false, false), (true, true)] {
        let outcome = call.invoke(vec![Value::Bool(enabled)], &mut Context::new(1_000_000));
        if enabled {
            let outcome = outcome.unwrap();
            assert_eq!(
                *outcome.value,
                Value::Type(Term::Primitive(Primitive::Bool))
            );
            assert_eq!(outcome.reused, reused);
        } else {
            let error = outcome.err().unwrap();
            assert!(
                matches!(error, comptime_executor::Error::Protocol(_)),
                "{error}"
            );
            assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
        }
    }
    assert_eq!(call.cached_results(), 1);
}

#[test]
fn ordinary_data_binds_the_following_domain_in_the_activation_that_runs() {
    let package = sysroot::compile(
        "struct Packet where tag: bool { output: bool }
         pure comptime fn read(comptime tag: bool, packet: Packet<tag>) -> bool { packet.output }",
    )
    .unwrap();
    let packet = package
        .structs
        .iter()
        .find(|def| def.name == "Packet")
        .unwrap()
        .declaration;
    let mut call = luna::memoization::Invocation::prepare(
        &package,
        &Default::default(),
        id(&package, "read"),
        vec![],
        8,
        Default::default(),
    )
    .unwrap();
    for (tag, actual) in [(true, false), (true, true), (false, false)] {
        let outcome = call.invoke(
            vec![
                Value::Bool(tag),
                Value::Record {
                    declaration: packet,
                    arguments: vec![Term::Bool(actual)],
                    fields: vec![Value::Bool(!tag)],
                },
            ],
            &mut Context::new(1_000_000),
        );
        if actual == tag {
            let outcome = outcome.unwrap();
            assert_eq!(*outcome.value, Value::Bool(!tag));
            assert!(!outcome.reused);
        } else {
            assert!(outcome.is_err());
            assert_eq!(call.last_execution_stats().bytecode_invocations, 0);
        }
    }
    assert_eq!(call.cached_results(), 0);
}

#[test]
fn cancelling_an_admitted_root_restores_the_original_vm_without_entering_its_body() {
    let package = sysroot::compile(
        "pure comptime fn checked_flag(flag: bool) -> bool { assumes flag; flag }",
    )
    .unwrap();
    let declaration = id(&package, "checked_flag");
    for reference in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, reference, |vm| {
            let mut fuel = vm.remaining_steps() as usize;
            let initial = fuel;
            let entry = vm
                .admit_source_call(declaration, vec![], vec![Value::Bool(false)], &mut fuel)
                .unwrap();
            assert_eq!(entry.domains(), &[Term::Primitive(Primitive::Bool)]);
            assert_eq!(entry.values().next(), Some(&Value::Bool(false)));
            drop(entry);
            assert!(fuel < initial);
            assert!(!vm.events().iter().any(|event| matches!(event,
                comptime_interpretor::Event::EnterFunction(id) if *id == declaration)));
            let entry = vm
                .admit_source_call(declaration, vec![], vec![Value::Bool(true)], &mut fuel)
                .unwrap();
            assert_eq!(entry.resume(&mut fuel).unwrap(), Value::Bool(true));
            assert_eq!(fuel as u64, vm.remaining_steps());
        });
    }
}

#[test]
fn an_associated_domain_can_supply_type_to_an_ordinary_value_slot() {
    let package = sysroot::compile(
        "trait Domain { assoc Item: Type; }
         impl Domain for bool { assoc Item: Type = Type; }
         pure comptime fn accept_domain(comptime T: Type, comptime value: T.as(Domain).Item) -> bool
         where requires T :- Domain
         { true }",
    )
    .unwrap();
    let mut call = prepare(&package, "accept_domain", vec![]);
    assert!(
        call.invoke(
            vec![
                Value::Type(Term::Primitive(Primitive::Bool)),
                Value::Bool(false)
            ],
            &mut Context::new(1_000_000),
        )
        .is_err()
    );
    let result = call
        .invoke(
            vec![
                Value::Type(Term::Primitive(Primitive::Bool)),
                Value::Type(Term::Primitive(Primitive::Char)),
            ],
            &mut Context::new(1_000_000),
        )
        .unwrap();
    assert_eq!(*result.value, Value::Bool(true));
}
