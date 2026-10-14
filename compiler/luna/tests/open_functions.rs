//! Deferred source signatures: ordinary checking, specialization and runtime contracts.
use comptime_interpretor::{Budget, Interpreter, Value};
#[path = "support/execution.rs"]
mod execution_support;
use execution_support::{compile, compile_with_protocols, compile_with_source_bindings, id, calls};

#[test]
fn open_deferred_functions_check_any_definitions_and_each_concrete_delivery() {
    let prefix = "typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    let source = format!("{prefix}
        fn identity(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {{let saved=value;saved}}
        fn returned(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {{return value;}}
        fn select(comptime flag:bool,left:defer Pick(flag),right:defer Pick(flag),choose:bool)->defer Pick(flag) {{
            if choose {{left}} else {{right}}
        }}
        fn byte()->.u8'builtin {{identity(false,7)}}
        fn flag()->bool {{returned(true,true)}}
        fn selected()->.u8'builtin {{select(false,3,9,false)}}");
    let package = compile(&source);
    for (entry, expected) in [("byte",Value::UnsignedInteger(7)), ("flag",Value::Bool(true)),
        ("selected",Value::UnsignedInteger(9))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),expected);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for declaration in [
        // Definition checking applies even when nobody calls the function.
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {value.member}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {value + value}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {if value {true} else {false}}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool {9}",
    ] {
        let failure=compile_with_protocols(&format!("{prefix}{declaration}"),&Default::default()).unwrap_err();
        assert!(!failure.contains("not yet implemented"),"definition must be rejected for missing evidence or incompatibility: {failure}");
    }
    for use_site in [
        "fn run()->bool {identity(false,true)}",
        "fn run()->bool {identity(true,7)}",
        "fn run()->bool {identity(false,7)}",
    ] {
        assert!(compile_with_protocols(&format!("{source}{use_site}"),&Default::default()).is_err(),"{use_site}");
    }
    // Equal spelling of two deferred computations does not prove an equality
    // between unrelated actual Types; concrete result delivery is rechecked.
    let bad_result=format!("{prefix}
        fn changed(comptime flag:bool,value:defer Pick(flag))->defer Pick(not flag) {{value}}
        fn run()->bool {{changed(false,7)}}");
    assert!(compile_with_protocols(&bad_result,&Default::default()).is_err());
}

#[test]
fn open_deferred_instances_preserve_opaque_types_and_reject_failed_computations() {
    let prefix = "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Identity<bool>}}
        fn pass(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}";
    let package=compile(&format!("{prefix} fn run()->bool {{pass(true,true)}}"));
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    assert!(compile_with_protocols(&format!("{prefix} fn bad()->bool {{pass(false,true)}}"),&Default::default()).is_err());
    let divergent="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Pick(flag)}}
        fn pass(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn run()->bool {pass(false,true)}";
    let error=compile_with_protocols(divergent,&Default::default()).unwrap_err();
    assert!(error.contains("comptime") && !error.contains("not yet implemented"),"{error}");
}

#[test]
fn open_deferred_functions_forward_any_without_granting_operation_evidence() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn accept(value:bool)->bool {value}
        fn leaf(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
        fn wrapper(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {leaf(flag,value)}
        fn narrow(comptime flag:bool,value:defer Pick(flag))->bool {accept(value)}
        fn run()->bool {wrapper(true,true) and narrow(true,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    assert!(graph.functions.iter().any(|instance|instance.function.name=="leaf"));
    assert!(compile_with_protocols(&format!("{source} fn bad()->bool {{narrow(false,1)}}"),&Default::default()).is_err());
    let prefix="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    for bad in [
        "comptime fn ct()->bool {true} fn bad(comptime flag:bool,value:defer Pick(flag))->bool {ct()}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->Type {bool}",
        "fn bad(comptime flag:bool,value:defer Pick(flag),illegal:Type)->defer Pick(flag) {value}",
        "fn other(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
            fn bad(comptime flag:bool,value:defer Pick(flag),runtime:bool)->defer Pick(flag) {other(runtime,value)}",
        "fn other(comptime flag:bool,value:defer Pick(flag))->defer Pick(flag) {value}
            fn bad(comptime flag:bool,value:defer Pick(flag),runtime:bool)->defer Pick(flag) {other(flag and runtime,value)}",
    ] {
        assert!(compile_with_protocols(&format!("{prefix}{bad}"),&Default::default()).is_err(),"unused definition must still reject: {bad}");
    }
}

#[test]
fn open_deferred_annotations_observe_concrete_type_inputs_and_keep_runtime_order() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(T:Type,flag:bool)->Type {if flag and T == bool {bool} else {.u8'builtin}}
        fn choose(comptime T:Type,comptime flag:bool,left:defer Pick(T,flag),right:defer Pick(T,flag))->defer Pick(T,flag) {left}
        fn first()->bool {true}
        fn second()->bool {false}
        fn run()->bool {choose(bool,true,first(),second())}
        fn byte()->.u8'builtin {choose(.u8'builtin,true,7,9)}";
    let package=compile(source);
    let mut interpreter=Interpreter::new(&package,Budget::default()).unwrap();
    assert_eq!(interpreter.invoke(id(&package,"run"),vec![]).unwrap(),Value::Bool(true));
    assert_eq!(calls(&package,&interpreter),["run","first","second","choose"]);
    assert_eq!(luna::execution::interpret(&package,"byte",100_000).unwrap(),Value::UnsignedInteger(7));
    for entry in ["run","byte"] {luna::instantiation::instantiate(&package,entry,100_000).unwrap();}
}

#[test]
fn open_deferred_recursion_reuses_instances_and_literals_use_concrete_context() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn recur(comptime flag:bool,value:defer Pick(flag),n:.u8'builtin)->defer Pick(flag) {
            if n == 0 {value} else {recur(flag,value,n - 1)}
        }
        fn make(comptime flag:bool)->defer Pick(flag) {7}
        fn run()->.u8'builtin {recur(false,make(false),3)}";
    let package=compile(source);
    assert_eq!(package.functions.iter().filter(|function|function.name=="recur").count(),1);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::UnsignedInteger(7));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    assert!(compile_with_protocols(&format!("{source} fn bad()->bool {{make(true)}}"),&Default::default()).is_err());
    let second=compile(source);
    assert_eq!(package.to_string(),second.to_string(),"instance identities and dumps must be deterministic");
}

#[test]
fn open_deferred_defaults_prepare_staged_chains_and_only_selected_suppliers() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        comptime fn invert(flag:bool)->bool {not flag}
        fn choose(comptime .seed:bool=false,comptime .flag:bool=invert(seed),.value:defer Pick(flag)=true)->defer Pick(flag) {value}
        fn flag()->bool {choose()}
        fn byte()->.u8'builtin {choose(.value=7,.seed=true)}
        fn override_flag()->bool {choose(.seed=true,.flag=true,.value=false)}";
    let package=compile(source);
    for (entry,result) in [("flag",Value::Bool(true)),("byte",Value::UnsignedInteger(7)),("override_flag",Value::Bool(false))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),result);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    assert!(compile_with_protocols(&format!("{source} fn bad()->.u8'builtin {{choose(.seed=true)}}"),&Default::default()).is_err());
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        comptime fn forever()->bool {forever()}
        fn choose(comptime .flag:bool=defer forever(),.value:defer Pick(flag)=true)->defer Pick(flag) {value}
        fn run()->bool {choose(.flag=true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    let error=compile_with_protocols(&format!("{source} fn bad()->bool {{choose()}}"),&Default::default()).unwrap_err();
    assert!(error.contains("comptime") && !error.contains("not yet implemented"),"{error}");
}

#[test]
fn open_deferred_runtime_defaults_observe_prepared_parameters_in_order() {
    let source="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}
        fn first()->bool {true}
        fn second()->bool {false}
        fn choose(comptime .flag:bool=true,input:bool,.next:bool=input,.value:defer Pick(flag)=next)->defer Pick(flag) {value}
        fn run()->bool {choose(first(),.next=second())}
        fn omitted()->bool {choose(first())}
        fn scope()->bool {let next=true;choose(false)}
        fn previous(comptime .flag:bool=true,limit:.u8'builtin,.prior:.u8'builtin=if limit > 0 {limit - 1} else {0},.value:defer Pick(flag)=true)->.u8'builtin {prior}
        fn zero()->.u8'builtin {previous(0)}
        fn six()->.u8'builtin {previous(7)}";
    let package=compile(source);
    let mut vm=Interpreter::new(&package,Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package,"run"),vec![]).unwrap(),Value::Bool(false));
    assert_eq!(calls(&package,&vm),["run","first","second","choose"]);
    for (entry,result) in [("omitted",Value::Bool(true)),("scope",Value::Bool(false))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),result);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for (entry,result) in [("zero",Value::UnsignedInteger(0)),("six",Value::UnsignedInteger(6))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),result);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
}

#[test]
fn open_deferred_defaults_check_unused_declarations_and_never_capture_future_inputs() {
    let prefix="typealias Type=.Type'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {.u8'builtin}}";
    for bad in [
        "fn noisy()->bool {true} fn bad(comptime .flag:bool=true,.value:defer Pick(flag)=noisy())->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=true,.value:defer Pick(flag)=later,.later:bool=true)->defer Pick(flag) {value}",
        "fn bad(runtime:bool,comptime .flag:bool=runtime,.value:defer Pick(flag)=true)->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=true,.value:defer Pick(flag)=do {return true;})->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=9,.value:defer Pick(flag)=true)->defer Pick(flag) {value}",
        "fn bad(comptime .flag:bool=true,value:defer Pick(flag),.other:defer Pick(flag)=value)->defer Pick(flag) {other}",
        "fn bad(comptime .flag:bool=true,.count:.u8'builtin=255 + 1,.value:defer Pick(flag)=true)->defer Pick(flag) {value}",
    ] {
        assert!(compile_with_protocols(&format!("{prefix}{bad}"),&Default::default()).is_err(),"unused invalid default: {bad}");
    }
}

#[test]
fn open_deferred_variadics_use_computed_tuple_identity_and_source_order() {
    let source="typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn Args(pair:bool)->Type {if pair {Tuple<bool,.u8'builtin>} else {Tuple<>}}
        fn collect(comptime pair:bool,...values:defer Args(pair),.marker:bool=false)->defer Args(pair) {values}
        fn first()->bool {true}
        fn second()->.u8'builtin {7}
        fn mark()->bool {false}
        fn run()->Tuple<bool,.u8'builtin> {collect(true,first(),.marker=mark(),second())}
        fn empty()->Tuple<> {collect(false)}
        fn named()->Tuple<bool,.u8'builtin> {collect(true,.values=(true,9))}
        fn packet()->Tuple<bool,.u8'builtin> {(first(),second())}
        fn spread()->Tuple<bool,.u8'builtin> {collect(true,...packet(),.marker=mark())}
        fn literal()->Tuple<bool,.u8'builtin> {collect(...(true,true,9))}";
    let selectors=luna::protocols::type_constructors::Selectors {
        meta_list:Some("input::List"),tuple:Some("input::Tuple"),..Default::default()
    };
    let package=compile_with_source_bindings(source,&Default::default(),selectors).unwrap();
    let mut vm=Interpreter::new(&package,Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package,"run"),vec![]).unwrap(),Value::Tuple(vec![Value::Bool(true),Value::UnsignedInteger(7)]));
    assert_eq!(calls(&package,&vm),["run","first","mark","second","collect"]);
    for (entry,result) in [("empty",Value::Tuple(vec![])),("named",Value::Tuple(vec![Value::Bool(true),Value::UnsignedInteger(9)]))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),result);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let mut spread_vm=Interpreter::new(&package,Budget::default()).unwrap();
    assert_eq!(spread_vm.invoke(id(&package,"spread"),vec![]).unwrap(),Value::Tuple(vec![Value::Bool(true),Value::UnsignedInteger(7)]));
    assert_eq!(calls(&package,&spread_vm),["spread","packet","first","second","mark","collect"]);
    assert_eq!(luna::execution::interpret(&package,"literal",100_000).unwrap(),Value::Tuple(vec![Value::Bool(true),Value::UnsignedInteger(9)]));
    for entry in ["spread","literal"] {luna::instantiation::instantiate(&package,entry,100_000).unwrap();}
    let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let collected=graph.functions.iter().find(|instance|instance.function.name=="collect").unwrap();
    let nir::Term::Function {parameters,result}=collected.function.signature.callable_signature() else {panic!("signature")};
    assert_eq!(parameters[0].ty,**result);
    assert!(matches!(result.as_ref(),nir::Term::NormalForm {head,..} if *head==id(&package,"Tuple")));
    for use_site in [
        "fn bad()->Tuple<bool,.u8'builtin> {collect(true,true)}",
        "fn bad()->Tuple<> {collect(false,true)}",
        "fn bad()->Tuple<bool,.u8'builtin> {collect(true,true,7,.values=(true,9))}",
    ] {
        assert!(compile_with_source_bindings(&format!("{source}{use_site}"),&Default::default(),selectors).is_err(),"{use_site}");
    }
}

#[test]
fn open_deferred_variadics_combine_default_format_inputs_with_computed_packets() {
    let source="typealias Type=.Type'builtin;typealias Text=\"\"'type;
        pure comptime fn List<T>->Type {List<T>}
        pure comptime fn Tuple<...Es:List<Type>>->Type {Tuple<...Es>}
        comptime fn Args(format:Text)->Type {format match {\"pair\" => Tuple<bool,.u8'builtin>,\"none\" => Tuple<>,_=>Args(format)}}
        fn pack(comptime .format:Text=\"pair\",...values:defer Args(format))->defer Args(format) {values}
        fn pair()->Tuple<bool,.u8'builtin> {pack(true,7)}
        fn empty()->Tuple<> {pack(.format=\"none\")}";
    let selectors=luna::protocols::type_constructors::Selectors {
        meta_list:Some("input::List"),tuple:Some("input::Tuple"),..Default::default()
    };
    let package=compile_with_source_bindings(source,&Default::default(),selectors).unwrap();
    for (entry,result) in [("pair",Value::Tuple(vec![Value::Bool(true),Value::UnsignedInteger(7)])),("empty",Value::Tuple(vec![]))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),result);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for use_site in [
        "fn bad()->Tuple<> {pack(.format=\"none\",true)}",
        "fn bad()->Tuple<bool,.u8'builtin> {pack(\"wrong\",7)}",
        "fn bad()->Tuple<> {pack(.format=\"unknown\")}",
    ] {
        assert!(compile_with_source_bindings(&format!("{source}{use_site}"),&Default::default(),selectors).is_err(),"{use_site}");
    }
}

#[test]
fn open_deferred_program_contracts_survive_instance_generation_and_call_admission() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        fn step(comptime flag:bool,value:defer Pick(flag),n:Byte)->result:Byte
            where requires n < 255, ensures result == n + 1 {n + 1}
        fn relay(comptime flag:bool,value:defer Pick(flag),n:Byte)->defer Pick(flag)
            where requires n < 10, ensures n == n'old {value}
        fn run()->Byte {step(true,true,254)}
        fn other()->Byte {step(false,7,3)}
        fn forwarded()->bool {relay(true,true,5)}";
    let package=compile(source);
    for (entry,result) in [("run",Value::UnsignedInteger(255)),("other",Value::UnsignedInteger(4)),("forwarded",Value::Bool(true))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),result);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let steps=package.functions.iter().filter(|function|function.name=="step").collect::<Vec<_>>();
    assert_eq!(steps.len(),2);
    assert!(steps.iter().all(|function|function.preconditions.len()==1 && function.postconditions.len()==1 && function.result_binding.is_some()));
    let relay=package.functions.iter().find(|function|function.name=="relay").unwrap();
    assert_eq!(relay.entry_snapshots.len(),1);
    for invalid in [
        format!("{source} fn bad()->Byte {{step(true,true,255)}}"),
        format!("{source} fn bad()->bool {{relay(true,true,10)}}"),
        source.replace("{n + 1}","{n}"),
    ] {
        let error=compile_with_protocols(&invalid,&Default::default()).unwrap_err();
        assert!(!error.contains("not yet implemented"),"{error}");
    }
}

#[test]
fn open_deferred_memory_contracts_export_writes_snapshots_and_results() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}
        fn put(comptime flag:bool,value:defer Pick(flag),p:*Byte,n:Byte)->result:Byte
            where requires n < 255, ensures p.* == n'old and result == p.* and p.* < 255 {
                p.* = n;p.*
            }
        fn run()->Byte {let cell:Byte=0;put(true,true,cell.ref,7);cell + 1}
        fn result()->Byte {let cell:Byte=0;put(false,9,cell.ref,7) + 1}";
    let package=compile(source);
    for entry in ["run","result"] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::UnsignedInteger(8));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    assert!(package.functions.iter().filter(|function|function.name=="put")
        .all(|function|!function.entry_snapshots.is_empty() && !function.postconditions.is_empty()));
    let bad=source.replace("p.* = n;p.*","p.* = 255;p.*");
    assert!(compile_with_protocols(&bad,&Default::default()).is_err());
}

#[test]
fn open_deferred_contracts_require_any_evidence_and_defined_entry_predicates() {
    let prefix="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Pick(flag:bool)->Type {if flag {bool} else {Byte}}";
    for invalid in [
        "fn bad(comptime flag:bool,value:defer Pick(flag))->bool where requires value == true {true}",
        "fn bad(comptime flag:bool,value:defer Pick(flag))->result:defer Pick(flag) where ensures result == value {value}",
        "fn bad(comptime flag:bool,value:defer Pick(flag),n:Byte)->bool where requires n + 1 > 0 {true}",
        "fn noisy()->bool {true} fn bad(comptime flag:bool,value:defer Pick(flag))->bool where requires noisy() {true}",
    ] {
        assert!(compile_with_protocols(&format!("{prefix}{invalid}"),&Default::default()).is_err(),"unused invalid contract: {invalid}");
    }
    let guarded=format!("{prefix}
        fn good(comptime flag:bool,value:defer Pick(flag),n:Byte)->bool
            where requires n < 255, requires n + 1 > 0 {{true}}
        fn run()->bool {{good(true,true,7)}}");
    let package=compile(&guarded);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn open_deferred_type_contracts_admit_before_running_type_computations() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {if T == bool {bool} else {Select(T)}}
        fn pass(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {value}
        fn run()->bool {pass(bool,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let function=package.functions.iter().find(|function|function.name=="pass").unwrap();
    assert_eq!(function.requirements.len(),1);
    for call in ["pass(Byte,7)","pass(Identity<bool>,true)"] {
        let invalid=format!("{source} pure comptime fn Identity<T>->Type {{T}} fn bad()->bool {{{call}}}");
        let failure=compile_with_protocols(&invalid,&Default::default()).unwrap_err();
        assert!(failure.contains("type requirement is not established"),"{failure}");
    }
}

#[test]
fn open_deferred_type_contracts_require_definition_side_forwarding_evidence() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn pass(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {value}
        fn relay(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {pass(T,value)}
        fn run()->bool {relay(bool,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let invalid=source.replace("where requires T == bool {pass(T,value)}","{pass(T,value)}");
    let failure=compile_with_protocols(&invalid,&Default::default()).unwrap_err();
    assert!(failure.contains("type requirement is not established"),"{failure}");
}

#[test]
fn open_deferred_type_contracts_specialize_defaults_and_distinct_instances() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn pass(comptime .T:Type=bool,value:defer Select(T))->defer Select(T)
            where requires T == bool or T == Byte {value}
        fn first()->bool {pass(true)}
        fn second()->Byte {pass(.T=Byte,7)}";
    let package=compile(source);
    for (entry,value) in [("first",Value::Bool(true)),("second",Value::UnsignedInteger(7))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let instances=package.functions.iter().filter(|function|function.name=="pass").collect::<Vec<_>>();
    assert_eq!(instances.len(),2);
    assert_ne!(instances[0].requirements,instances[1].requirements);
    for instance in instances {
        assert!(!instance.requirements.is_empty());
        for predicate in &instance.requirements {
            assert!(matches!(elaboration::ptc::Context::default().prove(
                elaboration::ptc::Formula::from_predicate(predicate),
                elaboration::ptc::Budget::default()), elaboration::ptc::Outcome::Entailed(_)));
        }
    }
}

#[test]
fn open_deferred_type_contracts_forward_dependent_defaults_without_running_overrides() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        comptime fn Loop()->Type {Loop()}
        fn pass(comptime .A:Type=Loop(),comptime .B:Type=A,value:defer Select(B))->defer Select(B)
            where requires B == bool {value}
        fn relay(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T == bool {pass(.A=T,value)}
        fn run()->bool {relay(bool,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    // The caller's Type input sits at a different index from both default
    // dependencies in the callee; lexical indices must be substituted once.
    let shifted=source.replace("fn relay(comptime T:Type,", "fn relay(comptime ignored:bool,comptime T:Type,")
        .replace("relay(bool,true)", "relay(false,bool,true)");
    let shifted_package=compile(&shifted);
    assert_eq!(luna::execution::interpret(&shifted_package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&shifted_package,"run",100_000).unwrap();
    let invalid=source.replace("where requires T == bool {pass(.A=T,value)}","{pass(.A=T,value)}");
    let failure=compile_with_protocols(&invalid,&Default::default()).unwrap_err();
    assert!(failure.contains("type requirement is not established"),"{failure}");
}

#[test]
fn open_deferred_calls_transport_known_domains_and_results_between_binders() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn identity(comptime T:Type,value:T)->T {value}
        fn open_identity(comptime T:Type,value:T,unused:defer Select(T))->T {value}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->U {
            open_identity(U,identity(U,value),unknown)
        }
        fn run()->bool {relay(false,bool,true,false)}
        fn byte()->.u8'builtin {relay(true,.u8'builtin,7,9)}";
    let package=compile(source);
    for (entry,value) in [("run",Value::Bool(true)),("byte",Value::UnsignedInteger(7))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let invalid=source.replace("identity(U,value)","identity(bool,value)");
    assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
}

#[test]
fn open_deferred_known_calls_use_defaults_literal_context_and_callee_requirements() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn identity(comptime .T:Type=Byte,value:T)->T {value}
        fn constrained(comptime T:Type,value:T)->T where requires T == bool {value}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->U
            where requires U == bool {constrained(U,value)}
        fn literal(comptime U:Type,unknown:defer Select(U))->Byte {identity(9)}
        fn run()->bool {relay(false,bool,true,false)}
        fn byte()->Byte {literal(bool,true)}";
    let package=compile(source);
    for (entry,value) in [("run",Value::Bool(true)),("byte",Value::UnsignedInteger(9))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let invalid=source.replace("where requires U == bool {constrained(U,value)}","{constrained(U,value)}");
    let failure=compile_with_protocols(&invalid,&Default::default()).unwrap_err();
    assert!(failure.contains("type requirement is not established"),"{failure}");
    let invalid=source.replace("identity(9)","identity(256)");
    assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
}

#[test]
fn open_deferred_bodies_infer_generic_calls_from_known_inputs_and_result_context() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn identity(value:T)->T where T {value}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->U {identity(value)}
        fn literal(comptime U:Type,unknown:defer Select(U))->Byte {identity(9)}
        fn run()->bool {relay(false,bool,true,false)}
        fn byte()->Byte {literal(bool,true)}";
    let package=compile(source);
    for (entry,value) in [("run",Value::Bool(true)),("byte",Value::UnsignedInteger(9))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let overflow=source.replace("identity(9)","identity(256)");
    assert!(compile_with_protocols(&overflow,&Default::default()).is_err());
}

#[test]
fn open_deferred_generic_calls_preserve_trait_admission_and_definition_evidence() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Allowed {} impl Allowed for bool {}
        comptime fn Select(T:Type)->Type {T}
        fn keep(value:T)->T where T :- Allowed {value}
        fn relay(comptime U:Type,unknown:defer Select(U),value:bool)->bool {keep(value)}
        fn run()->bool {relay(bool,false,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    // Check unused open definitions too: no later concrete instantiation may
    // compensate for a missing implementation or an unknown operand domain.
    for invalid in [
        source.replace("impl Allowed for bool {}", ""),
        source.replace("keep(value)", "keep(unknown)"),
        source.replace("value:bool)->bool {keep(value)}", "value:Byte)->Byte {keep(value)}")
            .replace("fn run()->bool {relay(bool,false,true)}", ""),
    ] {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err(),"{invalid}");
    }
}

#[test]
fn open_deferred_bodies_destructure_known_inputs_with_ordinary_pattern_rules() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn split(comptime T:Type,unknown:defer Select(T),pair:(bool,Byte))->Byte {
            let (flag,n)=pair;
            if flag {n} else {0}
        }
        fn run()->Byte {split(bool,true,(true,7))}
        fn other()->Byte {split(Byte,9,(false,7))}";
    let package=compile(source);
    for (entry,value) in [("run",Value::UnsignedInteger(7)),("other",Value::UnsignedInteger(0))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("let (flag,n)=pair;","let (flag,n)=unknown;"),
        source.replace("let (flag,n)=pair;","let (flag,n)=pair;n = 9;"),
        source.replace("let (flag,n)=pair;","let (flag,n,extra)=pair;"),
    ] {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
    }
}

#[test]
fn open_deferred_known_let_patterns_check_refutability_and_failure_scope() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,unknown:defer Select(T),flag:bool)->bool {
            let value if flag = true else {return false;};value
        }
        fn yes()->bool {choose(bool,false,true)}
        fn no()->bool {choose(bool,true,false)}";
    let package=compile(source);
    for (entry,value) in [("yes",true),("no",false)] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::Bool(value));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("else {return false;}",""),
        source.replace("else {return false;}","else {false}"),
        source.replace("else {return false;}","else {return value;}"),
    ] {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
    }
}

#[test]
fn open_deferred_known_regions_call_callbacks_closures_and_methods() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        trait Inspect {fn inspect(self)->bool;}
        impl Inspect for bool {fn inspect(self)->bool {self}}
        fn relay(comptime T:Type,unknown:defer Select(T),callback:fn(bool)->bool,flag:bool)->bool {
            let closure=|value:bool| callback(value);
            closure(flag).as(Inspect).inspect()
        }
        fn yes()->bool {relay(bool,false,|value| not value,false)}
        fn no()->bool {relay(.u8'builtin,7,|value| value,false)}";
    let package=compile(source);
    for (entry,value) in [("yes",true),("no",false)] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::Bool(value));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("callback(value)","unknown(value)"),
        source.replace("closure(flag).as(Inspect).inspect()","unknown.as(Inspect).inspect()"),
        source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}",""),
    ] {
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
    }
}

#[test]
fn open_deferred_trait_requirements_support_known_operations_and_forwarding() {
    let source="typealias Type=.Type'builtin;
        trait Inspect {fn inspect(self)->bool;}
        impl Inspect for bool {fn inspect(self)->bool {self}}
        comptime fn Select(T:Type)->Type {T}
        fn inspect(comptime T:Type,value:T,unknown:defer Select(T))->bool
            where requires T :- Inspect {value.as(Inspect).inspect()}
        fn relay(comptime ignored:bool,comptime U:Type,value:U,unknown:defer Select(U))->bool
            where requires U :- Inspect {inspect(U,value,unknown)}
        fn run()->bool {relay(false,bool,true,false)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let instances=package.functions.iter().filter(|function|
        (function.name=="inspect" || function.name=="relay") && matches!(function.signature.callable_signature(),
            nir::Term::Function {parameters,..} if parameters.first().is_some_and(|parameter|parameter.sort!=nir::Sort::Runtime))).collect::<Vec<_>>();
    assert_eq!(instances.len(),2);
    for function in instances {
        assert!(!function.capabilities.is_empty());
        for requirement in &function.capabilities {assert_eq!(requirement.subject,nir::Term::Primitive(nir::Primitive::Bool));}
    }
    for invalid in [
        source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", ""),
        source.replace("where requires U :- Inspect {inspect(U,value,unknown)}", "{inspect(U,value,unknown)}"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_trait_requirements_specialize_interface_arguments_and_defaults() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        trait Allowed where Item {} impl Allowed<bool> for bool {}
        comptime fn Select(T:Type)->Type {T}
        fn keep(comptime .A:Type=bool,comptime .B:Type=A,value:defer Select(A))->defer Select(A)
            where requires A :- Allowed<B> {value}
        fn relay(comptime T:Type,value:defer Select(T))->defer Select(T)
            where requires T :- Allowed<T> {keep(.A=T,value)}
        fn run()->bool {relay(bool,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    for function in package.functions.iter().filter(|function|function.name=="keep" || function.name=="relay") {
        assert_eq!(function.capabilities.len(),1);
        assert_eq!(function.capabilities[0].interface_arguments,vec![nir::Term::Primitive(nir::Primitive::Bool)]);
    }
    let invalid=format!("{source} fn bad()->bool {{keep(.A=bool,.B=Byte,true)}}");
    let failure=compile_with_protocols(&invalid,&Default::default()).unwrap_err();
    assert!(failure.contains("type requirement is not established"),"{failure}");
}

#[test]
fn open_deferred_match_arms_forward_unknown_values_with_known_pattern_evidence() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,left:defer Select(T),right:defer Select(T),flag:bool)->defer Select(T) {
            flag match {true=>left,false=>right}
        }
        fn yes()->bool {choose(bool,true,false,true)}
        fn byte()->Byte {choose(Byte,7,9,false)}";
    let package=compile(source);
    for (entry,value) in [("yes",Value::Bool(true)),("byte",Value::UnsignedInteger(9))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("true=>left,false=>right","true=>left"),
        source.replace("flag match", "left match"),
        source.replace("true=>left,false=>right","true=>left,false=>not right"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_match_patterns_bind_guard_and_scope_each_arm() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,left:defer Select(T),right:defer Select(T),pair:(bool,bool))->defer Select(T) {
            pair match {(flag,other) if flag=>do {if other {return left;} right},_=>left}
        }
        fn first()->bool {choose(bool,true,false,(true,true))}
        fn second()->bool {choose(bool,true,false,(true,false))}
        fn fallback()->bool {choose(bool,true,false,(false,false))}";
    let package=compile(source);
    for (entry,value) in [("first",true),("second",false),("fallback",true)] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::Bool(value));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace(",_=>left", ""),
        source.replace("if flag=>", "if left=>"),
        source.replace("(flag,other)", "(flag,other,extra)"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_when_branches_forward_values_and_keep_type_facts_scoped() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,left:defer Select(T),right:defer Select(T),flag:bool)->defer Select(T) {
            when {flag=>left,else=>right}
        }
        fn type_branch(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            when {T == bool=>value,else=>false}
        }
        fn discard(comptime T:Type,value:defer Select(T),flag:bool)->bool {
            when {flag=>value};true
        }
        fn discarded()->bool {discard(Byte,7,false)}
        fn yes()->bool {choose(bool,true,false,true)}
        fn byte()->Byte {choose(Byte,7,9,false)}
        fn typed()->bool {type_branch(bool,true,false)}
        fn fallback()->bool {type_branch(Byte,7,9)}";
    let package=compile(source);
    for (entry,value) in [("yes",Value::Bool(true)),("byte",Value::UnsignedInteger(9)),("typed",Value::Bool(true)),("fallback",Value::Bool(false)),("discarded",Value::Bool(true))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("when {flag=>left,else=>right}","when {else=>right,flag=>left}"),
        source.replace("when {flag=>left,else=>right}","when {left=>left,else=>right}"),
        source.replace("when {T == bool=>value,else=>false}","when {T == bool=>value,else=>value}"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_if_type_facts_and_discard_match_when_semantics() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn narrow(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            if T == bool {value} else {false}
        }
        fn discard(comptime T:Type,unknown:defer Select(T),flag:bool)->bool {
            if flag {unknown};true
        }
        fn first()->bool {narrow(bool,true,false)}
        fn other()->bool {narrow(Byte,7,9)}
        fn ignored()->bool {discard(Byte,7,true)}";
    let package=compile(source);
    for (entry,value) in [("first",true),("other",false),("ignored",true)] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::Bool(value));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for branch in ["if T == bool {value} else {return false;}",
        "when {T == bool=>value,else=>return false}"] {
        let early=compile(&source.replace("if T == bool {value} else {false}",branch));
        for (entry,value) in [("first",true),("other",false)] {
            assert_eq!(luna::execution::interpret(&early,entry,100_000).unwrap(),Value::Bool(value));
            luna::instantiation::instantiate(&early,entry,100_000).unwrap();
        }
    }
    for invalid in [
        source.replace("if T == bool {value} else {false}", "if T == bool {value} else {value}"),
        source.replace("if flag {unknown};true", "if unknown {unknown};true"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_mixed_type_conditions_preserve_short_circuit_evaluation() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn touch(p:*bool)->bool {p.* = true;true}
        fn choose(comptime T:Type,value:T,unknown:defer Select(T),p:*bool)->bool {
            if T == bool and value and touch(p) {value} else {false}
        }
        fn run()->bool {let touched=false;choose(bool,true,false,touched.ref) and touched}
        fn skip_value()->bool {let touched=false;choose(bool,false,true,touched.ref);not touched}
        fn skip_type()->bool {let touched=false;choose(Byte,7,9,touched.ref);not touched}";
    let package=compile(source);
    for entry in ["run","skip_value","skip_type"] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::Bool(true));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let mut vm=Interpreter::new(&package,Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(&package,"run"),vec![]).unwrap(),Value::Bool(true));
    assert_eq!(calls(&package,&vm),["run","choose","touch"]);
    let reversed=compile(&source.replace("T == bool and value and touch(p)","touch(p) and T == bool and value")
        .replace("not touched", "touched"));
    for entry in ["run","skip_value","skip_type"] {
        assert_eq!(luna::execution::interpret(&reversed,entry,100_000).unwrap(),Value::Bool(true));
    }
    let disjunction=compile(&source.replace("if T == bool and value and touch(p) {value} else {false}",
        "if T != bool or value {false} else {value}")
        .replace("choose(bool,true,false,touched.ref) and touched", "not choose(bool,true,false,touched.ref)"));
    for entry in ["run","skip_value","skip_type"] {
        assert_eq!(luna::execution::interpret(&disjunction,entry,100_000).unwrap(),Value::Bool(true));
        luna::instantiation::instantiate(&disjunction,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("T == bool and value", "value and T == bool"),
        source.replace("T == bool and value", "T == bool or value"),
        source.replace("else {false}", "else {value}"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_mutable_locals_preserve_delivery_and_concrete_assignment_types() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn replace(comptime T:Type,first:defer Select(T),second:defer Select(T),flag:bool)->defer Select(T) {
            var result=first;if flag {result = second;} result
        }
        fn yes()->bool {replace(bool,false,true,true)}
        fn byte()->Byte {replace(Byte,7,9,false)}";
    let package=compile(source);
    for (entry,value) in [("yes",Value::Bool(true)),("byte",Value::UnsignedInteger(7))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    let immutable=source.replace("var result=first", "let result=first");
    let error=compile_with_protocols(&immutable,&Default::default()).unwrap_err();
    assert!(error.contains("cannot reassign a let binding"),"{error}");
    let mismatch=source.replace("result = second", "result = true");
    assert!(compile_with_protocols(&mismatch,&Default::default()).is_err());
    let invalid=source.replace("result = second", "result = not second");
    assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
}

#[test]
fn open_deferred_assignments_retain_concrete_move_and_replacement_checks() {
    use elaboration::verification::moves::{Delivery, analyze};
    let prefix="typealias Type=.Type'builtin;struct Ticket {}
        comptime fn Select(T:Type)->Type {T}";
    for (body,valid,cleanup_count) in [
        ("var local=first;local=second;local",true,1),
        ("var local=first;local=local;second;local",true,0),
        ("var local=first;local=second;first",false,0),
        ("var local=first;local=second;second",false,0),
    ] {
        let package=compile(&format!("{prefix}
            fn replace(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {{{body}}}
            fn run()->Ticket {{replace(Ticket,Ticket {{}},Ticket {{}})}}
            fn ordinary(first:Ticket,second:Ticket)->Ticket {{{body}}}"));
        // Runtime resource obligations start after staged parameter removal.
        let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
        let function=&graph.functions.iter().find(|instance|instance.function.name=="replace").unwrap().function;
        let report=analyze(function,|ty|Ok(if matches!(ty,nir::Term::Primitive(_)) {Delivery::Copy} else {Delivery::Move}),100_000);
        assert_eq!(report.is_ok(),valid,"{body}: {report:?}");
        let ordinary=analyze(package.functions.iter().find(|function|function.name=="ordinary").unwrap(),
            |ty|Ok(if matches!(ty,nir::Term::Primitive(_)) {Delivery::Copy} else {Delivery::Move}),100_000);
        assert_eq!(ordinary.is_ok(),valid,"ordinary {body}: {ordinary:?}");
        if valid {
            assert_eq!(report.as_ref().unwrap().cleanup_sites.iter().map(|site|site.reason).collect::<Vec<_>>(),
                ordinary.unwrap().cleanup_sites.iter().map(|site|site.reason).collect::<Vec<_>>(),"{body}");
            let report=report.unwrap();
            assert_eq!(report.cleanup_sites.iter().filter(|site| site.reason==elaboration::verification::moves::CleanupReason::Replacement).count(),cleanup_count,"{body}: {report}");
        }
    }
}

#[test]
fn open_deferred_loops_reassign_values_and_preserve_transfer_targets() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn choose(comptime T:Type,first:defer Select(T),second:defer Select(T),flag:bool)->defer Select(T) {
            var result=first;var running=flag;
            while running {result=second;running=false;continue;}
            result
        }
        fn nested(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {
            var result=first;
            while true {while true {result=second;break;} break;}
            result
        }
        fn returned(comptime T:Type,value:defer Select(T))->defer Select(T) {
            while true {return value;} value
        }
        fn yes()->bool {choose(bool,false,true,true)}
        fn unchanged()->Byte {choose(Byte,7,9,false)}
        fn inner()->Byte {nested(Byte,7,9)}
        fn exit()->bool {returned(bool,true)}";
    let package=compile(source);
    for (entry,value) in [("yes",Value::Bool(true)),("unchanged",Value::UnsignedInteger(7)),
        ("inner",Value::UnsignedInteger(9)),("exit",Value::Bool(true))] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),value);
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("running=false;continue;", "running=false;continue;result=not second;"),
        source.replace("var result=first;var running=flag;", "let result=first;var running=flag;"),
        source.replace("while running", "while first"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_loops_preserve_affine_backedge_checks_after_instantiation() {
    use elaboration::verification::moves::{Delivery, analyze};
    for (body,valid) in [
        ("while flag {local=second;break;}",true),
        ("while flag {local=second;continue;}",false),
        ("while true {local=second;break;}",true),
        ("while flag {local=local;break;}",true),
    ] {
        let package=compile(&format!("typealias Type=.Type'builtin;struct Ticket {{}}
            comptime fn Select(T:Type)->Type {{T}}
            fn cycle(comptime T:Type,first:defer Select(T),second:defer Select(T),flag:bool)->defer Select(T) {{
                var local=first;{body} local
            }}
            fn run()->Ticket {{cycle(Ticket,Ticket {{}},Ticket {{}},false)}}"));
        let graph=luna::instantiation::instantiate(&package,"run",100_000).unwrap();
        let function=&graph.functions.iter().find(|instance|instance.function.name=="cycle").unwrap().function;
        let result=analyze(function,|ty|Ok(if matches!(ty,nir::Term::Primitive(_)) {Delivery::Copy} else {Delivery::Move}),100_000);
        assert_eq!(result.is_ok(),valid,"{body}: {result:?}");
    }
}

#[test]
fn open_deferred_named_loop_transfers_respect_nesting_and_closure_boundaries() {
    let source="typealias Type=.Type'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn select(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {
            var result=first;
            while:outer true {while true {result=second;break outer;} result=first;}
            result
        }
        fn continued(comptime T:Type,first:defer Select(T),second:defer Select(T))->defer Select(T) {
            var result=first;var running=true;
            while:outer running {running=false;while true {result=second;continue outer;} result=first;}
            result
        }
        fn run()->bool {select(bool,false,true)}
        fn again()->bool {continued(bool,false,true)}";
    let package=compile(source);
    for entry in ["run","again"] {
        assert_eq!(luna::execution::interpret(&package,entry,100_000).unwrap(),Value::Bool(true));
        luna::instantiation::instantiate(&package,entry,100_000).unwrap();
    }
    for invalid in [
        source.replace("result=second;break outer;", "let action=|| {break outer;};action();"),
        source.replace("result=second;break outer;", "let outer=true;break outer;"),
        source.replace("result=second;break outer;", "result=second;break absent;"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
}

#[test]
fn open_deferred_type_assumptions_are_checked_before_instance_publication() {
    let source="typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        comptime fn Select(T:Type)->Type {T}
        fn narrowed(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            assumes T == bool;not value
        }
        fn run()->bool {narrowed(bool,false,true)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    for invalid in [
        source.replace("narrowed(bool,false,true)","narrowed(Byte,7,9)"),
        source.replace("unknown:defer Select(T))->bool", "unknown:defer Select(T),flag:bool)->bool")
            .replace("narrowed(bool,false,true)", "narrowed(bool,false,true,false)")
            .replace("assumes T == bool;not value", "if flag {assumes T == bool;};not value"),
        source.replace("assumes T == bool;not value", "not value"),
    ] {assert!(compile_with_protocols(&invalid,&Default::default()).is_err());}
    // Boolean program assumptions keep their runtime check rather than being
    // mistaken for PTC facts just because they occur inside an open definition.
    let checked=compile("typealias Type=.Type'builtin;comptime fn Select(T:Type)->Type {T}
        fn guard(comptime T:Type,value:defer Select(T),flag:bool)->bool {assumes flag;flag}
        fn good()->bool {guard(bool,true,true)} fn bad()->bool {guard(bool,true,false)}");
    assert_eq!(luna::execution::interpret(&checked,"good",100_000).unwrap(),Value::Bool(true));
    assert!(luna::execution::interpret(&checked,"bad",100_000).is_err());
}

#[test]
fn open_deferred_trait_assumptions_require_a_real_concrete_implementation() {
    let source="typealias Type=.Type'builtin;
        trait Inspect {fn inspect(self)->bool;} impl Inspect for bool {fn inspect(self)->bool {self}}
        comptime fn Select(T:Type)->Type {T}
        fn inspect(comptime T:Type,value:T,unknown:defer Select(T))->bool {
            assumes T :- Inspect;value.as(Inspect).inspect()
        }
        fn run()->bool {inspect(bool,true,false)}";
    let package=compile(source);
    assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
    luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    let invalid=source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", "");
    assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
    for assumption in ["assumes (T :- Inspect) or T == .u8'builtin;", "assumes not (T :- Inspect);"] {
        let invalid=source.replace("assumes T :- Inspect;",assumption);
        assert!(compile_with_protocols(&invalid,&Default::default()).is_err());
    }
    let conjunction=compile(&source.replace("assumes T :- Inspect;", "assumes (T :- Inspect) and T == bool;"));
    assert_eq!(luna::execution::interpret(&conjunction,"run",100_000).unwrap(),Value::Bool(true));
}

#[test]
fn open_type_assumptions_export_only_facts_of_completing_paths() {
    let prefix="typealias Type=.Type'builtin;comptime fn Select(T:Type)->Type {T}";
    for statement in [
        "do {do {assumes T == bool;};};",
        "if flag {assumes T == bool;} else {assumes T == bool;}",
        "if flag {return false;} else {assumes T == bool;}",
    ] {
        let source=format!("{prefix} fn narrow(comptime T:Type,value:T,unknown:defer Select(T),flag:bool)->bool {{
            {statement} not value
        }} fn run()->bool {{narrow(bool,false,true,false)}}");
        let package=compile(&source);
        assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
        luna::instantiation::instantiate(&package,"run",100_000).unwrap();
    }
    for statement in [
        "if flag {assumes T == bool;}",
        "while flag {assumes T == bool;break;}",
    ] {
        let source=format!("{prefix} fn invalid(comptime T:Type,value:T,unknown:defer Select(T),flag:bool)->bool {{
            {statement} not value
        }}");
        assert!(compile_with_protocols(&source,&Default::default()).is_err());
    }
}

#[test]
fn open_trait_assumptions_survive_unconditional_and_common_branch_completion() {
    let prefix="typealias Type=.Type'builtin;trait Inspect {fn inspect(self)->bool;}
        impl Inspect for bool {fn inspect(self)->bool {self}}
        comptime fn Select(T:Type)->Type {T}";
    for statement in [
        "do {assumes T :- Inspect;};",
        "if flag {assumes T :- Inspect;} else {assumes T :- Inspect;}",
    ] {
        let source=format!("{prefix} fn inspect(comptime T:Type,value:T,unknown:defer Select(T),flag:bool)->bool {{
            {statement} value.as(Inspect).inspect()
        }} fn run()->bool {{inspect(bool,true,false,false)}}");
        let package=compile(&source);
        assert_eq!(luna::execution::interpret(&package,"run",100_000).unwrap(),Value::Bool(true));
        luna::instantiation::instantiate(&package,"run",100_000).unwrap();
        assert!(compile_with_protocols(&source.replace("impl Inspect for bool {fn inspect(self)->bool {self}}", ""),&Default::default()).is_err());
    }
}
