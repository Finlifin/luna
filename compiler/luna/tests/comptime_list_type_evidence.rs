//! List observation retains checked element identity and invocation work.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Event, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{{}}} typealias Type=.Type'builtin; typealias Integer=.Integer'builtin; typealias Byte=.u8'builtin; const list_concat=.list_concat'builtin;const list_length=.list_length'builtin; {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn readonly_list_elements_supply_local_type_annotations() {
    for body in [
        "let types:meta.List<Type> = [bool,Byte];let T=types[0];let value:T=true;value",
        "let types:meta.List<Type> = [Byte,bool];let index:Integer=1;let T=types[index];let value:T=true;value",
        "let types:meta.List<Type> = [bool];let alias=types;let T=alias[0];let value:T=true;value",
        "let outer:meta.List<meta.List<Type>> = [[bool]];let types=outer[0];let T=types[0];let value:T=true;value",
        "let types=do {let types:meta.List<Type> = [bool,Byte];types};let T=types[0];let value:T=true;value",
    ] {
        let source = format!("comptime fn run()->bool {{{body}}}");
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
            "{source}"
        );
    }
}

#[test]
fn partial_lists_keep_known_types_without_skipping_other_initializers() {
    for body in [
        "let types:meta.List<Type> = [bool,Other()];let T=types[0];let value:T=true;value",
        "let records:meta.List<Descriptor> = [Descriptor {ty:bool,flag:observe(true)}];let d=records[0];let value:d.ty=d.flag;value",
    ] {
        let source = format!(
            "struct Descriptor {{ty:Type,flag:bool}} comptime fn Other()->Type {{Byte}} comptime fn observe(value:bool)->bool {{value}} comptime fn run()->bool {{{body}}}"
        );
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let callee = support::id(
            &package,
            if body.contains("Other()") {
                "Other"
            } else {
                "observe"
            },
        );
        assert_eq!(
            vm.events()
                .iter()
                .filter(|e| matches!(e,Event::EnterFunction(id) if *id==callee))
                .count(),
            1
        );
    }
    let source = "comptime fn Missing()->Type {let types:meta.List<Type> = [];types[0]} comptime fn run()->bool {let types:meta.List<Type> = [bool,Missing()];let T=types[0];let value:T=true;value}";
    let package = compile(source).unwrap();
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
}

#[test]
fn unknown_mutable_and_out_of_bounds_indices_do_not_invent_type_evidence() {
    for source in [
        "comptime fn run()->bool {var types:meta.List<Type> = [bool];let T=types[0];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [bool];var index:Integer=0;let T=types[index];let value:T=true;value}",
        "comptime fn choose(index:Integer)->bool {let types:meta.List<Type> = [bool,Byte];let T=types[index];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [bool];let T=types[1];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [bool];let T=types[-1];let value:T=true;value}",
        "pure comptime fn Identity<T>->Type {T} comptime fn run()->bool {let types:meta.List<Type> = [Identity<bool>];let T=types[0];let value:T=true;value}",
        "comptime fn run()->bool {let types:meta.List<Type> = [Byte];let T=types[0];let value:T=true;value}",
    ] {
        assert!(compile(source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn list_and_index_initializers_run_once_in_source_order() {
    let source="comptime fn Other()->Type {Byte} comptime fn observe(value:bool)->bool {value}
        comptime fn run()->bool {let types:meta.List<Type> = [bool,Other()];let T=types[do {let ignored=observe(true);0}];let value:T=true;value}";
    let package = compile(source).unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    let ids = [
        support::id(&package, "Other"),
        support::id(&package, "observe"),
    ];
    let calls = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => ids.iter().position(|expected| id == expected),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, [0, 1]);
}

#[test]
fn branches_intersect_known_and_partial_list_elements() {
    for body in [
        "let types:meta.List<Type> = if flag {[bool,Byte]} else {[bool,Integer]};let T=types[0];let result:T=flag;result",
        "let types:meta.List<Type> = if flag {[bool,Byte]} else {[bool,Other()]};let T=types[0];let result:T=flag;result",
        "let types:meta.List<Type> = flag match {true=>[bool,Other()],false=>[bool,Byte]};let T=types[0];let result:T=flag;result",
        "let lists:meta.List<meta.List<Type>> = if flag {[[bool,Byte]]} else {[[bool,Integer]]};let T=lists[0][0];let result:T=flag;result",
        "let records:meta.List<Descriptor> = if flag {[Descriptor {ty:bool,flag:true}]} else {[Descriptor {ty:bool,flag:observe(false)}]};let d=records[0];let result:d.ty=d.flag;result",
    ] {
        let source = format!(
            "struct Descriptor {{ty:Type,flag:bool}} comptime fn Other()->Type {{Byte}} comptime fn observe(flag:bool)->bool {{flag}} comptime fn choose(flag:bool)->bool {{{body}}}"
        );
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        for flag in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
            assert_eq!(
                vm.invoke(support::id(&package, "choose"), vec![Value::Bool(flag)])
                    .unwrap(),
                Value::Bool(flag),
                "{source}"
            );
        }
    }
    for body in [
        "if flag {[bool]} else {[Byte]}",
        "if flag {[bool]} else {[]}",
        "flag match {true=>[bool],false=>[Byte]}",
    ] {
        let source = format!(
            "comptime fn choose(flag:bool)->bool {{let types:meta.List<Type> = {body};let T=types[0];let result:T=true;result}}"
        );
        assert!(compile(&source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn list_branch_observation_preserves_condition_and_selected_initializer() {
    let source = "comptime fn condition(flag:bool)->bool {flag}
        comptime fn Left()->Type {Byte} comptime fn Right()->Type {Integer}
        comptime fn choose(flag:bool)->bool {
            let types:meta.List<Type> = if condition(flag) {[bool,Left()]} else {[bool,Right()]};
            let T=types[0];let result:T=flag;result
        }";
    let package = compile(source).unwrap();
    let ids = [
        support::id(&package, "condition"),
        support::id(&package, "Left"),
        support::id(&package, "Right"),
    ];
    for flag in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "choose"), vec![Value::Bool(flag)])
                .unwrap(),
            Value::Bool(flag)
        );
        let calls = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => ids.iter().position(|expected| id == expected),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls, vec![0, if flag { 1 } else { 2 }]);
    }
}

#[test]
fn different_length_branches_retain_only_guaranteed_prefix_types() {
    for body in [
        "let types:meta.List<Type> = if flag {[bool,Byte]} else {[bool]};let T=types[0];let result:T=flag;result",
        "let types:meta.List<Type> = flag match {true=>[bool],false=>[bool,Byte,Integer]};let T=types[0];let result:T=flag;result",
        "let lists:meta.List<meta.List<Type>> = if flag {[[bool,Byte],[Integer]]} else {[[bool]]};let T=lists[0][0];let result:T=flag;result",
        "let types:meta.List<Type> = if flag {if other {[bool,Byte]} else {[bool]}} else {[bool,Integer,Byte]};let T=types[0];let result:T=flag;result",
    ] {
        let source = format!("comptime fn choose(flag:bool,other:bool)->bool {{{body}}}");
        let package = compile(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        for flag in [false, true] {
            for other in [false, true] {
                let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
                assert_eq!(
                    vm.invoke(
                        support::id(&package, "choose"),
                        vec![Value::Bool(flag), Value::Bool(other)]
                    )
                    .unwrap(),
                    Value::Bool(flag)
                );
            }
        }
    }
    for body in [
        "let types:meta.List<Type> = if flag {[bool,Byte]} else {[bool]};let T=types[1];let result:T=true;result",
        "let types:meta.List<Type> = if flag {[bool]} else {[Byte,Integer]};let T=types[0];let result:T=true;result",
        "let types:meta.List<Type> = if flag {[bool]} else {[]};let T=types[0];let result:T=true;result",
    ] {
        let source = format!("comptime fn choose(flag:bool)->bool {{{body}}}");
        assert!(compile(&source).is_err(), "unexpectedly accepted {source}");
    }
}

#[test]
fn prefix_observation_does_not_truncate_actual_list_values() {
    let source = "comptime fn choose(flag:bool)->Integer {
        let types:meta.List<Type> = if flag {[bool,Byte]} else {[bool]};
        let T=types[0];let value:T=flag;
        types.length()
    }";
    let package = compile(source).unwrap();
    for (flag, length) in [(false, 1), (true, 2)] {
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "choose"), vec![Value::Bool(flag)])
                .unwrap(),
            Value::Integer(comptime_runtime::Integer::from_u128(length))
        );
    }
}

#[test]
fn concatenated_list_evidence_preserves_exact_offsets_and_initializer_calls() {
    for body in [
        "let a:meta.List<Type> = [Other(),bool]; let b:meta.List<Type> = [Byte]; let combined=list_concat(a,b); let T=combined[2]; let value:T=7; value == 7",
        "let a:meta.List<Type> = [Byte]; let b:meta.List<Type> = [Other(),bool]; let combined=list_concat(a,b); let T=combined[2]; let value:T=true; value",
        "let a:meta.List<Type> = [Other(),bool]; let count=list_length(a); let b:meta.List<Type> = [Byte]; let combined=list_concat(a,b); let T=combined[count]; let value:T=7; value == 7",
        "let a:meta.List<Type> = if choose() {[Other(),bool]} else {[Byte,bool]}; let count=list_length(a); let b:meta.List<Type> = [Byte]; let combined=list_concat(a,b); let T=combined[count]; let value:T=7; value == 7",
    ] {
        let package=compile(&format!("comptime fn Other()->Type {{Byte}} comptime fn choose()->bool {{true}} comptime fn run()->bool {{{body}}}")).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let other = support::id(&package, "Other");
        assert_eq!(
            vm.events()
                .iter()
                .filter(|e| matches!(e,Event::EnterFunction(id) if *id==other))
                .count(),
            1
        );
    }
}

#[test]
fn concatenation_never_treats_a_joined_prefix_as_an_exact_length() {
    let prefix = "comptime fn choose()->bool {false}";
    for body in [
        "let a:meta.List<Type> = if choose() {[bool]} else {[bool,Byte]}; let b:meta.List<Type> = [bool]; let combined=list_concat(a,b); let T=combined[1]; let value:T=true; value",
        "let a:meta.List<Type> = if choose() {[bool]} else {[bool,Byte]}; let n=list_length(a); let types:meta.List<Type> = [Byte,bool,Byte]; let T=types[n]; let value:T=true; value",
        "var a:meta.List<Type> = [bool]; let b:meta.List<Type> = [Byte]; let combined=list_concat(a,b); let T=combined[1]; let value:T=7; true",
    ] {
        assert!(
            compile(&format!("{prefix} comptime fn run()->bool {{{body}}}")).is_err(),
            "accepted {body}"
        );
    }
    let package=compile(&format!("{prefix} comptime fn run()->bool {{let a:meta.List<Type> = if choose() {{[bool]}} else {{[bool,Byte]}}; let b:meta.List<Type> = [Byte];let combined=list_concat(a,b);let T=combined[0];let value:T=true;value}}")).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn concatenation_evidence_keeps_failures_and_opaque_type_identity() {
    let package=compile("comptime fn Fail()->Type {let types:meta.List<Type> = [];types[0]} comptime fn run()->bool {let a:meta.List<Type> = [bool];let b:meta.List<Type> = [Fail()];let combined=list_concat(a,b);let T=combined[0];let value:T=true;value}").unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::IndexOutOfBounds
    );
    let source = "pure comptime fn Identity<T>->Type {T} comptime fn run()->bool {let a:meta.List<Type> = [];let b:meta.List<Type> = [Identity<bool>];let combined=list_concat(a,b);let T=combined[0];let value:T=true;value}";
    assert!(compile(source).is_err());
    let package=compile("comptime fn run()->bool {let a:meta.List<Type> = [];let b:meta.List<Type> = [bool];let combined=list_concat(a,b);let T=combined[0];let value:T=true;value}").unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nested_concat_type_observation_is_sound_for_all_branch_choices() {
    // Independent semantic oracle: enumerate actual list values, rather than
    // reproducing the compiler's partial-evidence merge algorithm. Conservative
    // rejection is allowed; every accepted annotation must hold on every path.
    let choices: &[&[bool]] = &[&[], &[true], &[false, true]];
    let spell = |values: &[bool]| {
        format!(
            "[{}]",
            values
                .iter()
                .map(|v| if *v { "bool" } else { "Byte" })
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let mut accepted = 0;
    let mut rejected = 0;
    for a in choices {
        for b in choices {
            for c in choices {
                for d in choices {
                    for index in [0, 1, 2, 4] {
                        let source = format!(
                            "typealias Type=.Type'builtin;typealias Byte=.u8'builtin;const list_concat=.list_concat'builtin;
                            pure comptime fn List<T>->Type {{T}}
                            comptime fn run(first:bool,second:bool)->bool {{
                                let prefix:List<Type> = [bool];
                                let left:List<Type> = if first {{{a}}} else {{{b}}};
                                let right:List<Type> = if second {{{c}}} else {{{d}}};
                                let suffix:List<Type> = [bool];
                                let joined=list_concat(list_concat(list_concat(prefix,left),right),suffix);
                                let Selected=joined[{index}];let value:Selected=true;value
                            }}", a=spell(a),b=spell(b),c=spell(c),d=spell(d)
                        );
                        let result = support::compile_with_source_bindings(
                            &source,
                            &Default::default(),
                            luna::protocols::type_constructors::Selectors {
                                meta_list: Some("input.List"),
                                ..Default::default()
                            },
                        );
                        let Ok(package) = result else {
                            assert_ne!(index, 0, "lost the unconditional bool prefix: {source}");
                            rejected += 1;
                            continue;
                        };
                        accepted += 1;
                        let mut vm =
                            Interpreter::for_comptime(&package, Default::default()).unwrap();
                        for (first, left) in [(true, *a), (false, *b)] {
                            for (second, right) in [(true, *c), (false, *d)] {
                                let actual = std::iter::once(true)
                                    .chain(left.iter().copied())
                                    .chain(right.iter().copied())
                                    .chain(std::iter::once(true))
                                    .collect::<Vec<_>>();
                                assert_eq!(
                                    actual.get(index),
                                    Some(&true),
                                    "unsound annotation: first={first} second={second}: {source}"
                                );
                                assert_eq!(
                                    vm.invoke(
                                        support::id(&package, "run"),
                                        vec![Value::Bool(first), Value::Bool(second)]
                                    )
                                    .unwrap(),
                                    Value::Bool(true)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(accepted > 81, "non-prefix cases must also be usable");
    assert!(
        rejected > 0,
        "invalid or unknown positions must not all be accepted"
    );
}

#[test]
fn ordinary_type_lists_support_methods_spreads_and_canonical_normal_inputs() {
    let package = compile(
        "pure comptime fn Pack<...Ts: meta.List<Type>> -> Type { bool }
        comptime fn inspect_type_list_carrier(types: meta.List<Type>) -> bool {
            var copy = types;
            copy.append(bool);
            let prefix: meta.List<Type> = [Byte];
            let left = prefix.concat(types);
            let right = types.concat(prefix);
            types.length() == 2 and types[0] == bool and types[1] == Byte
                and copy.length() == 3 and copy[2] == bool
                and left[0] == Byte and left[1] == bool
                and right[0] == bool and right[2] == Byte
        }
        comptime fn Form(types: meta.List<Type>) -> Type { Pack<.Ts = types> }
        comptime fn Spread(types: meta.List<Type>) -> Type { Pack<...types> }
        comptime fn identity(types: meta.List<Type>) -> meta.List<Type> { types }
        comptime fn source_values() -> meta.List<Type> { [bool, Byte] }",
    )
    .unwrap();
    let function = package
        .functions
        .iter()
        .find(|f| f.declaration == support::id(&package, "inspect_type_list_carrier"))
        .unwrap();
    let nir::Term::Function { parameters, .. } = function.signature.callable_signature() else {
        panic!()
    };
    let ty = parameters[0].ty.clone();
    let elements = vec![
        nir::Term::Primitive(nir::Primitive::Bool),
        nir::Term::Primitive(nir::Primitive::U8),
    ];
    let ordinary = Value::List {
        ty: ty.clone(),
        values: elements.iter().cloned().map(Value::Type).collect(),
    };
    let mut vm = Interpreter::for_comptime(
        &package,
        comptime_interpretor::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    let produced = vm
        .invoke(support::id(&package, "source_values"), vec![])
        .unwrap();
    assert_eq!(produced, ordinary);
    vm.validate_immutable_input(&ordinary, &ty)
        .unwrap_or_else(|error| {
            panic!(
                "ordinary admission {error:?}: {ordinary:?}, work {:?}",
                ordinary.immutable_copy_work()
            )
        });
    for input in [ordinary.clone(), produced.clone()] {
        assert_eq!(
            vm.invoke(
                support::id(&package, "inspect_type_list_carrier"),
                vec![input]
            )
            .unwrap_or_else(|error| panic!("{error:?}: {:?}", vm.events())),
            Value::Bool(true)
        );
    }
    assert_eq!(
        vm.invoke(support::id(&package, "identity"), vec![ordinary.clone()])
            .unwrap(),
        ordinary
    );
    let mut expected = None;
    for entry in ["Form", "Spread"] {
        for input in [ordinary.clone(), produced.clone()] {
            let value = vm
                .invoke(support::id(&package, entry), vec![input])
                .unwrap();
            if let Some(expected) = &expected {
                assert_eq!(&value, expected);
            } else {
                expected = Some(value);
            }
        }
    }
    // Wrong element domains and forged Type descriptions remain rejected.
    for bad in [Value::Bool(true), Value::Type(nir::Term::Bool(true))] {
        let error = vm
            .invoke(
                support::id(&package, "identity"),
                vec![Value::List {
                    ty: ty.clone(),
                    values: vec![bad].into(),
                }],
            )
            .unwrap_err();
        assert_eq!(error.kind, comptime_interpretor::ErrorKind::Type);
        assert_eq!(
            vm.invoke(
                support::id(&package, "inspect_type_list_carrier"),
                vec![ordinary.clone()]
            )
            .unwrap(),
            Value::Bool(true)
        );
    }
}
