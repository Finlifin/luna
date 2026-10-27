//! Composite interface/body memory footprints from real source contracts.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use elaboration::verification::memory;
use nir::program::{Expr, ExprKind as K, Package};

fn method(package: &Package) -> Expr {
    let run = package.functions.iter().find(|f| f.name == "run").unwrap();
    let mut found = None;
    nir::visit::expressions(run, &mut 10000, |expr, _| {
        if matches!(expr.kind, K::TraitMethod { .. }) {
            found = Some(expr.clone());
        }
        Ok(())
    })
    .unwrap();
    found.unwrap()
}

#[test]
fn interface_pointer_observation_survives_a_body_without_inherited_predicates() {
    let mut package = support::compile(
        "trait Read {fn read(p:*bool)->bool where requires p.*;}
        struct Host {} impl Read for Host {fn read(p:*bool)->bool where requires true {true}}
        fn run(p:*bool)->bool where requires p.* {Host.as(Read).read(p)}",
    );
    let callee = method(&package);
    // Source elaboration currently inherits the interface precondition. Model
    // a separately prepared body without that copy: the method query must
    // still recover the public contract from the full interface identity.
    let selected =
        elaboration::materialization::resolve_method_instance(&package, &callee, &mut 100000)
            .unwrap();
    package
        .functions
        .iter_mut()
        .find(|f| f.declaration == selected.body)
        .unwrap()
        .preconditions
        .clear();
    let before = package.clone();
    let composed = memory::infer_method_in_package(&package, &callee, &mut 100000).unwrap();
    assert_eq!(composed.callee, callee);
    assert!(composed.summary.may_reject);
    assert!(
        composed
            .summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.read_value && !a.write)
    );
    assert!(!composed.summary.accesses.iter().any(|a| a.write));
    let selected =
        elaboration::materialization::resolve_method_instance(&package, &callee, &mut 100000)
            .unwrap();
    let body = package
        .functions
        .iter()
        .find(|f| f.declaration == selected.body)
        .unwrap();
    let plain = memory::infer_in_package(&package, body.declaration, &[], &mut 100000).unwrap();
    assert!(
        !plain.accesses.iter().any(|a| a.read_value),
        "{plain} BODY: {body}"
    );
    assert_eq!(before, package);
    assert!(
        composed
            .to_string()
            .starts_with("(method-memory-summary (callee")
    );
    assert!(memory::infer_method_in_package(&package, &callee, &mut 0).is_err());
    assert_eq!(
        memory::infer_method_in_package(&package, &callee, &mut 100000).unwrap(),
        composed
    );
}

#[test]
fn interface_postcondition_observes_the_actual_body_return_and_keeps_writes() {
    let package=support::compile("trait Toggle {fn toggle(p:*bool)->result:*bool where requires p.*, ensures not result.*;}
        struct Host {} impl Toggle for Host {fn toggle(p:*bool)->*bool {p.* = false;assumes not p.*;p}}
        fn run(p:*bool)->*bool where requires p.* {Host.as(Toggle).toggle(p)}");
    let composed =
        memory::infer_method_in_package(&package, &method(&package), &mut 100000).unwrap();
    assert!(composed.summary.may_reject);
    assert!(
        composed
            .summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.write)
    );
    assert!(
        composed
            .summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.read_value)
    );
    let targets = composed.summary.result_targets.unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets.iter().next().unwrap().parameter, 0);
}

#[test]
fn composed_memory_body_includes_transitive_calls_without_publishing_cycle_seeds() {
    let package=support::compile("fn helper(p:*bool,flag:bool)->result:bool where ensures result == flag {let ignored=p.*;flag}
        trait Read {fn read(p:*bool,flag:bool)->result:bool where requires flag, ensures result == flag'old;}
        struct Host {} impl Read for Host {fn read(p:*bool,flag:bool)->result:bool where ensures result == flag'old {let ignored=helper(p,flag);flag}}
        fn run(p:*bool)->bool {Host.as(Read).read(p,true)}");
    let summary = memory::infer_method_in_package(&package, &method(&package), &mut 100000)
        .unwrap()
        .summary;
    assert!(summary.may_reject);
    assert!(
        summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.read_value)
    );
}

#[test]
fn composed_summary_imports_and_validates_prepared_cleanup_accesses() {
    let mut package = support::compile(
        "trait Read {fn read(p:*bool)->bool where requires p.*;} struct Host {} impl Read for Host {fn read(p:*bool)->bool {true}} fn run(p:*bool)->bool where requires p.* {Host.as(Read).read(p)}",
    );
    let callee = method(&package);
    let selected =
        elaboration::materialization::resolve_method_instance(&package, &callee, &mut 100000)
            .unwrap();
    let body = package
        .functions
        .iter()
        .find(|f| f.declaration == selected.body)
        .unwrap()
        .clone();
    // Model resource preparation's acquired cleanup overlay. The body itself
    // only reads; the completed instance additionally writes through p.
    package
        .runtime_instances
        .push(nir::program::RuntimeInstance {
            declaration: body.declaration,
            arguments: vec![],
            function: body,
            cleanup_accesses: vec![nir::program::RuntimeCleanupAccess {
                parameter: 0,
                input_fields: vec![],
                through: vec![],
                fields: vec![],
                write: true,
                read_value: false,
                ty: nir::Term::Primitive(nir::Primitive::Bool),
                conditions: Default::default(),
            }],
        });
    let summary = memory::infer_method_in_package(&package, &callee, &mut 100000).unwrap();
    assert!(
        summary
            .summary
            .accesses
            .iter()
            .any(|a| a.target.parameter == 0 && a.write)
    );
    package
        .runtime_instances
        .last_mut()
        .unwrap()
        .cleanup_accesses[0]
        .parameter = 9;
    assert!(memory::infer_method_in_package(&package, &callee, &mut 100000).is_err());
    package
        .runtime_instances
        .last_mut()
        .unwrap()
        .cleanup_accesses[0]
        .parameter = 0;
    assert_eq!(
        memory::infer_method_in_package(&package, &callee, &mut 100000).unwrap(),
        summary
    );
}

#[test]
fn higher_order_method_keeps_interface_reads_and_callback_specific_writes() {
    use memory::{CallableInput, CallableResult};
    use std::collections::BTreeMap;
    let package = support::compile(
        "fn read(p:*bool)->bool {p.*}
        fn write(p:*bool)->bool {p.* = false;false}
        fn wrong()->bool {false}
        trait Apply {fn apply(p:*bool,f:fn(*bool)->bool)->bool where requires p.*;}
        struct Host {} impl Apply for Host {fn apply(p:*bool,f:fn(*bool)->bool)->bool {f(p)}}
        fn run(p:*bool)->bool where requires p.* {Host.as(Apply).apply(p,read)}",
    );
    let callee = method(&package);
    let key = |name: &str| iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    let slot = CallableInput {
        parameter: 1,
        fields: vec![],
    };
    let infer = |value: CallableResult| {
        let inputs = BTreeMap::from([(slot.clone(), value)]);
        memory::infer_method_with_code_inputs(
            &package,
            &callee,
            &inputs,
            &BTreeMap::new(),
            &mut 100000,
        )
    };
    assert!(memory::infer_method_in_package(&package, &callee, &mut 100000).is_err());
    let read = infer(CallableResult::Known(key("read"))).unwrap();
    assert!(!read.summary.accesses.iter().any(|a| a.write));
    let write = infer(CallableResult::Known(key("write"))).unwrap();
    assert!(
        write
            .summary
            .accesses
            .iter()
            .any(|a| a.read_value && a.target.parameter == 0)
    );
    assert!(
        write
            .summary
            .accesses
            .iter()
            .any(|a| a.write && a.target.parameter == 0)
    );
    assert!(write.summary.may_reject);
    assert_ne!(read.summary.callable_inputs, write.summary.callable_inputs);
    let joined = infer(CallableResult::Candidates(vec![key("read"), key("write")])).unwrap();
    assert!(joined.summary.accesses.iter().any(|a| a.write));
    assert!(infer(CallableResult::Known(key("wrong"))).is_err());
    assert!(infer(CallableResult::Unknown).is_err());
    assert!(infer(CallableResult::Candidates(vec![])).is_err());
    assert_eq!(infer(CallableResult::Known(key("read"))).unwrap(), read);
}

#[test]
fn prepared_method_callbacks_retain_environments_and_validate_layouts() {
    use elaboration::verification::moves::{self, Delivery};
    use memory::{CallableInput, CodeDefinition};
    use std::collections::BTreeMap;
    for (signature, factory) in [
        ("fn(*bool)->bool", "{|p:*bool|{p.* = false;false}}"),
        (
            "%{flag:bool} fn(*bool)->bool",
            "{let flag=true;|p:*bool|{p.* = flag;flag}}",
        ),
    ] {
        let package = support::compile(&format!(
            "fn callback()->{signature} {factory}
            trait Apply {{fn apply(p:*bool,f:{signature})->bool where requires p.*;}}
            struct Host {{}} impl Apply for Host {{fn apply(p:*bool,f:{signature})->bool {{f(p)}}}}
            fn run(p:*bool)->bool where requires p.* {{Host.as(Apply).apply(p,callback())}}"
        ));
        let callee = method(&package);
        let callback = package
            .functions
            .iter()
            .find(|f| f.name == "callback")
            .unwrap();
        let evidence =
            moves::analyze_in_package(&package, callback, |_| Ok(Delivery::Copy), 100000).unwrap();
        let definition = &evidence.callable_evidence_exits[0]
            .as_ref()
            .unwrap()
            .definition;
        let infer = |definition: &Expr, fuel: &mut usize| {
            let code = BTreeMap::from([(
                CallableInput {
                    parameter: 1,
                    fields: vec![],
                },
                vec![CodeDefinition::Closure {
                    definition,
                    captures: Default::default(),
                    prepared_captures: Default::default(),
                }],
            )]);
            memory::infer_method_with_code_inputs(&package, &callee, &BTreeMap::new(), &code, fuel)
        };
        let summary = infer(definition, &mut 100000).unwrap();
        assert!(
            summary
                .summary
                .accesses
                .iter()
                .any(|a| a.write && a.target.parameter == 0)
        );
        assert!(
            summary
                .summary
                .accesses
                .iter()
                .any(|a| a.read_value && a.target.parameter == 0)
        );
        assert_eq!(summary.summary.prepared_inputs.len(), 1);
        let mut malformed = definition.clone();
        if let K::Closure { parameters, .. } = &mut malformed.kind {
            parameters.clear();
        }
        assert!(infer(&malformed, &mut 100000).is_err());
        assert!(infer(definition, &mut 0).is_err());
        assert_eq!(infer(definition, &mut 100000).unwrap(), summary);
    }
}

#[test]
fn interface_observations_do_not_reuse_reassigned_implementation_locals() {
    let package = support::compile("trait Observe {fn observe(p:*bool,q:*bool)->bool where requires q.*, ensures p.*;}
        struct Host {} impl Observe for Host {fn observe(p:*bool,q:*bool)->bool {p=q;true}}
        fn run(p:*bool,q:*bool)->bool where requires p.*, requires q.* {Host.as(Observe).observe(p,q)}");
    let callee = method(&package);
    let summary = memory::infer_method_in_package(&package, &callee, &mut 100000)
        .unwrap()
        .summary;
    // The implementation rebinds its local p to q. The interface's p remains
    // the value supplied to the invocation, so its exit observation reads input 0.
    for parameter in [0, 1] {
        assert!(
            summary
                .accesses
                .iter()
                .any(|a| a.target.parameter == parameter && a.read_value),
            "{summary}"
        );
    }
    assert!(
        !summary.accesses.iter().any(|a| a.write),
        "local rebinding does not write the pointee: {summary}"
    );
}

#[test]
fn reference_nir_method_exit_checks_the_interface_without_body_inheritance() {
    use comptime_interpretor::{ErrorKind, Interpreter, Value};
    let mut package = support::compile(
        "trait Echo {fn echo(flag:bool)->answer:bool where ensures answer == flag'old;}
        struct Host {} impl Echo for Host {fn echo(flag:bool)->bool {flag}}
        fn run()->bool {Host.as(Echo).echo(true)}",
    );
    let callee = method(&package);
    let selected =
        elaboration::materialization::resolve_method_instance(&package, &callee, &mut 100000)
            .unwrap();
    let run = package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration;
    let body = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == selected.body)
        .unwrap();
    body.postconditions.clear();
    let mut vm = Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(vm.invoke(run, vec![]).unwrap(), Value::Bool(true));
    package
        .functions
        .iter_mut()
        .find(|f| f.declaration == selected.body)
        .unwrap()
        .body
        .kind = K::Literal(nir::program::Literal::Bool(false));
    let mut vm = Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(run, vec![]).unwrap_err().kind,
        ErrorKind::ProgramContractRejected
    );
}

#[test]
fn expanding_recursive_memory_failure_identifies_instance_and_does_not_publish_a_seed() {
    let source = include_str!("../../../development/repros/host-recursive-node-memory-contract.fl");
    let package = support::compile(&format!("{source} fn observe(p:*bool)->bool {{p.*}}"));
    let before = package.clone();
    let declaration = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let observe = declaration("observe");
    let baseline = memory::infer_in_package(&package, observe, &[], &mut 100_000).unwrap();
    for budget in [20_000, 100_000] {
        let error = memory::infer_in_package(&package, declaration("enable"), &[], &mut { budget })
            .unwrap_err();
        assert!(error.contains("exhausted"), "{error}");
        assert!(
            error.contains("while inferring memory contract for \"enable\""),
            "{error}"
        );
        assert!(error.contains("discovered instances:"), "{error}");
        assert!(error.contains("previous accesses:"), "{error}");
        assert_eq!(package, before);
        assert_eq!(
            memory::infer_in_package(&package, observe, &[], &mut 100_000).unwrap(),
            baseline
        );
    }
    assert_eq!(baseline.accesses.len(), 1);
    assert!(baseline.accesses[0].read_value);
    assert!(!baseline.accesses[0].write);
}

#[test]
fn repeated_method_queries_share_dispatch_evidence_within_the_memory_round() {
    fn work(unrelated: usize, calls: usize) -> usize {
        let mut source = String::from(
            "trait Read {fn read(pointer:*bool)->bool;}\n\
             struct Host {}\n\
             impl Read for Host {fn read(pointer:*bool)->bool {pointer.*}}\n",
        );
        for index in 0..unrelated {
            source.push_str(&format!(
                "fn unrelated_{index}(value:T)->T where T {{value}}\n"
            ));
        }
        source.push_str("fn run(pointer:*bool)->bool {\n");
        for _ in 0..calls {
            source.push_str("let observed=Host.as(Read).read(pointer);\n");
        }
        source.push_str("true}");
        let package = support::compile(&source);
        let declaration = package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .declaration;
        let mut fuel = 2_000_000;
        let summary = memory::infer_in_package(&package, declaration, &[], &mut fuel).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value && !access.write)
        );
        assert!(memory::infer_in_package(&package, declaration, &[], &mut 1).is_err());
        assert_eq!(
            memory::infer_in_package(&package, declaration, &[], &mut 2_000_000).unwrap(),
            summary
        );
        2_000_000 - fuel
    }
    let small_once = work(0, 1);
    let small_many = work(0, 16);
    let large_once = work(128, 1);
    let large_many = work(128, 16);
    println!(
        "method query work: small_once={small_once}, small_many={small_many}, large_once={large_once}, large_many={large_many}"
    );
    // Unrelated declaration metadata may cost initial work, but must not be
    // rescanned for each repeated call in one immutable package query.
    assert!(large_many - large_once <= small_many - small_once + 2048);
}
