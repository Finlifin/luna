#[path = "support/execution.rs"]
mod support;
use comptime_interpretor::Value;
use nir::{Parameter, Primitive, Sort, Term};

#[test]
fn template_identity_metadata_admits_tags_but_not_execution() {
    use iir::{InstanceKey, template_storage::TemplateStorage};
    let id = nir::DeclarationId {
        package: 7,
        local: 11,
    };
    let logical = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: Term::Primitive(Primitive::Bool),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let identity = nir::program::RuntimeFunctionIdentity {
        declaration: id,
        type_parameters: vec![],
        signature: logical.clone(),
    };
    let mut package = nir::program::Package {
        runtime_function_identities: vec![identity.clone()],
        ..Default::default()
    };
    let key = InstanceKey {
        declaration: id,
        arguments: vec![].into(),
    };
    let storage =
        TemplateStorage::from_package(logical.clone(), vec![key.clone()], &package, &mut 1000)
            .unwrap();
    assert_eq!(storage.decode(0).unwrap(), &key);
    let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert!(vm.invoke(id, vec![Value::Bool(true)]).is_err());
    package.runtime_function_identities.push(identity);
    assert!(
        TemplateStorage::from_package(logical.clone(), vec![key.clone()], &package, &mut 1000)
            .unwrap_err()
            .contains("ambiguous")
    );
    package.runtime_function_identities.pop();
    package.runtime_function_identities[0].signature = Term::Primitive(Primitive::Bool);
    assert!(
        TemplateStorage::from_package(logical.clone(), vec![key.clone()], &package, &mut 1000)
            .is_err()
    );
    package.runtime_function_identities[0].signature = logical.clone();
    assert!(TemplateStorage::from_package(logical, vec![key], &package, &mut 1000).is_ok());
}

#[test]
fn template_storage_checks_source_declarations_and_recovers_after_rejection() {
    use iir::{InstanceKey, template_storage::TemplateStorage};
    let package = support::compile(
        "typealias Type=.Type'builtin;
         fn keep(comptime T:Type,value:T)->T {value}
         fn generic(comptime T:Type,value:T)->T where X {value}
         fn tagged(comptime T:Type,value:T)->T where tag:bool {value}
         fn wrong(comptime T:Type,value:bool)->bool {value}
         comptime fn staged(comptime T:Type,value:T)->T {value}",
    );
    let key = |name: &str, arguments: Vec<Term>| InstanceKey {
        declaration: support::id(&package, name),
        arguments: arguments.into(),
    };
    let keep = key("keep", vec![]);
    let logical = package
        .functions
        .iter()
        .find(|f| f.declaration == keep.declaration)
        .unwrap()
        .signature
        .clone();
    let targets = vec![
        keep.clone(),
        key("generic", vec![Term::Primitive(Primitive::Bool)]),
        key("tagged", vec![Term::Bool(true)]),
    ];
    let make = |keys, fuel: &mut usize| {
        TemplateStorage::from_package(logical.clone(), keys, &package, fuel)
    };
    let storage = make(targets.clone(), &mut 100_000).unwrap();
    for (index, target) in targets.iter().enumerate() {
        assert_eq!(storage.encode(target, &mut 1000).unwrap(), index as u64);
        assert_eq!(storage.decode(index as u64).unwrap(), target);
    }
    for (invalid, diagnostic) in [
        (vec![key("wrong", vec![])], "signature differs"),
        (vec![key("staged", vec![])], "not a runtime declaration"),
        (vec![keep.clone(), keep.clone()], "duplicate target"),
    ] {
        assert!(
            make(invalid, &mut 100_000)
                .unwrap_err()
                .contains(diagnostic)
        );
    }
    for invalid in [
        key("generic", vec![]),
        key("generic", vec![Term::Bool(true)]),
        key("tagged", vec![Term::Primitive(Primitive::Bool)]),
        key("keep", vec![Term::Primitive(Primitive::Bool)]),
    ] {
        assert!(make(vec![invalid], &mut 100_000).is_err());
    }
    let mut missing = keep.clone();
    missing.declaration.local = u32::MAX;
    assert!(
        make(vec![missing], &mut 100_000)
            .unwrap_err()
            .contains("missing")
    );
    let mut ambiguous = package.clone();
    ambiguous.functions.push(
        package
            .functions
            .iter()
            .find(|f| f.declaration == keep.declaration)
            .unwrap()
            .clone(),
    );
    assert!(
        TemplateStorage::from_package(logical.clone(), vec![keep], &ambiguous, &mut 100_000)
            .unwrap_err()
            .contains("ambiguous")
    );
    assert!(make(targets.clone(), &mut 0).is_err());
    assert_eq!(make(targets, &mut 100_000).unwrap(), storage);
}

#[test]
fn named_type_inputs_support_runtime_and_comptime_function_values() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        typealias Callback=fn(comptime T:Type,value:T)->T;
        fn keep(comptime T:Type,value:T)->T {value}
        fn run()->bool {let callback:Callback=keep;callback(bool,true)}
        typealias Staged=comptime fn(comptime T:Type,value:T)->T;
        comptime fn staged(comptime T:Type,value:T)->T {value}
        comptime fn ready()->bool {let callback:Staged=staged;callback(bool,true)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "ready", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn callable_type_inputs_preserve_slot_indices_and_outer_alias_parameters() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        typealias Callback=fn(flag:bool,comptime T:Type,value:T)->T;
        fn keep(flag:bool,comptime T:Type,value:T)->T {value}
        typealias Fixed<Outer> = fn(comptime Inner:Type,left:Outer,right:Inner)->Outer;
        fn first(comptime U:Type,left:bool,right:U)->bool {left}
        fn run()->bool {
            let callback:Callback=keep;
            let fixed:Fixed<bool> = first;
            callback(false,bool,true) and fixed(.u8'builtin,true,9)
        }",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nested_callable_types_abstract_names_at_their_own_depth() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        comptime fn Shape()->Type {fn(comptime T:Type)->fn(comptime U:Type,first:T,second:U)->T}
        comptime fn Renamed()->Type {fn(comptime A:Type)->fn(comptime B:Type,left:A,right:B)->A}",
    );
    let value = luna::invocation::invoke(&package, "Shape", 100_000).unwrap();
    assert_eq!(
        value,
        luna::invocation::invoke(&package, "Renamed", 100_000).unwrap()
    );
    let p = |sort, ty| Parameter { sort, ty };
    let bound = |depth| Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    };
    assert_eq!(
        value,
        Value::Type(Term::Function {
            parameters: vec![p(Sort::Type, Term::Primitive(Primitive::Type))],
            result: Box::new(Term::Function {
                parameters: vec![
                    p(Sort::Type, Term::Primitive(Primitive::Type)),
                    p(Sort::Runtime, bound(1)),
                    p(Sort::Runtime, bound(0))
                ],
                result: Box::new(bound(1)),
            }),
        })
    );
}

#[test]
fn dependent_callable_results_preserve_tuple_normal_form_heads() {
    let package = support::compile_with_tuple("typealias Type=.Type'builtin;typealias Callback=fn(comptime T:Type,value:T)->Tuple<T,bool>;
        fn pair(comptime T:Type,value:T)->Tuple<T,bool> {(value,true)}
        fn run()->bool {let callback:Callback=pair;let (value,flag)=callback(bool,true);value and flag}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn callable_value_inputs_preserve_opaque_heads_and_domains() {
    let package = support::compile("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;
        pure comptime fn Tagged<flag:bool,number:Byte>->Type {bool}
        typealias Callback=fn(comptime flag:bool,comptime number:Byte,value:Tagged<flag,number>)->Tagged<flag,number>;
        fn keep(comptime flag:bool,comptime number:Byte,value:Tagged<flag,number>)->Tagged<flag,number> {value}
        comptime fn Shape()->Type {Callback}
        fn select()->Callback {keep}
        typealias Simple=fn(comptime number:Byte)->Byte;
        fn echo(comptime number:Byte)->Byte {number}
        fn run()->Byte {let callback:Simple=echo;callback(9)}");
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::UnsignedInteger(9)
    );
    let Value::Type(Term::Function { parameters, result }) =
        luna::invocation::invoke(&package, "Shape", 100_000).unwrap()
    else {
        panic!("expected callable type")
    };
    let Term::NormalForm {
        head, arguments, ..
    } = result.as_ref()
    else {
        panic!("normal-form head was lost")
    };
    assert_eq!(*head, support::id(&package, "Tagged"));
    assert_eq!(
        arguments,
        &vec![
            Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::ComptimeValue
            },
            Term::Bound {
                depth: 0,
                index: 1,
                sort: Sort::ComptimeValue
            }
        ]
    );
    assert_eq!(parameters[2].ty, *result);
    for source in [
        "typealias Bad=fn(number:Byte)->Tagged<true,number>;",
        "typealias Bad=fn(comptime number:bool)->Tagged<true,number>;",
    ] {
        let error = support::compile_with_protocols(&format!("typealias Type=.Type'builtin;typealias Byte=.u8'builtin;pure comptime fn Tagged<flag:bool,number:Byte>->Type {{bool}} {source}"), &Default::default()).unwrap_err();
        assert!(
            error.contains("explicit comptime value input") || error.contains("type mismatch"),
            "{error}"
        );
    }
}

#[test]
fn nested_callable_value_domains_keep_outer_type_dependencies() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Tagged<T,value:T>->Type {T}
        comptime fn Outer()->Type {fn(comptime T:Type,comptime value:T)->fn()->Tagged<T,value>}
        comptime fn Inner()->Type {fn(comptime T:Type)->fn(comptime value:T)->Tagged<T,value>}",
    );
    for (entry, expected_depth, expected_index) in [("Outer", 1, 1), ("Inner", 0, 0)] {
        let Value::Type(Term::Function { result, .. }) =
            luna::invocation::invoke(&package, entry, 100_000).unwrap()
        else {
            panic!("outer callable missing")
        };
        let Term::Function { result, .. } = *result else {
            panic!("inner callable missing")
        };
        let Term::NormalForm { arguments, .. } = *result else {
            panic!("head missing")
        };
        assert_eq!(
            arguments,
            vec![
                Term::Bound {
                    depth: 1,
                    index: 0,
                    sort: Sort::Type
                },
                Term::Bound {
                    depth: expected_depth,
                    index: expected_index,
                    sort: Sort::ComptimeValue
                }
            ]
        );
    }
}

#[test]
fn dependent_callable_environments_and_comptime_captures_keep_input_frames() {
    let package = support::compile("typealias Type=.Type'builtin;
        typealias Callback=fn(comptime T:Type,value:T)->T;
        fn keep(comptime T:Type,value:T)->T {value}
        typealias Staged=comptime fn(comptime T:Type,value:T)->T;
        comptime fn staged(comptime T:Type,value:T)->T {value}
        comptime fn ready()->bool {let callback:Staged=staged;let invoke=||callback(bool,true);invoke()}
        comptime fn Shape()->Type {fn(comptime T:Type)->%{saved:T} fn()->T}
        comptime fn Empty()->Type {%{} comptime fn(comptime T:Type,value:T)->T}");
    assert_eq!(
        luna::invocation::invoke(&package, "ready", 100_000).unwrap(),
        Value::Bool(true)
    );
    let Value::Type(Term::Function { result, .. }) =
        luna::invocation::invoke(&package, "Shape", 100_000).unwrap()
    else {
        panic!("outer function missing")
    };
    let Term::Closure {
        environment,
        signature,
    } = *result
    else {
        panic!("environment missing")
    };
    assert_eq!(
        environment,
        vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type
        }]
    );
    assert_eq!(
        *signature,
        Term::Function {
            parameters: vec![],
            result: Box::new(Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type
            })
        }
    );
    let Value::Type(ty) = luna::invocation::invoke(&package, "Empty", 100_000).unwrap() else {
        panic!("type missing")
    };
    assert!(matches!(ty, Term::ComptimeFunction(_)));
}

#[test]
fn runtime_closures_capture_dependent_callable_templates() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
        typealias Callback=fn(comptime T:Type,value:T)->T;
        fn keep(comptime T:Type,value:T)->T {value}
        fn run()->bool {let callback:Callback=keep;let invoke=||callback(bool,true);invoke()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let target = iir::InstanceKey {
        declaration: support::id(&package, "keep"),
        arguments: vec![].into(),
    };
    assert!(
        graph.function(&target).is_none(),
        "a template is not an executable instance"
    );
    let logical = graph.template_type(&target).unwrap();
    let mut captures = 0;
    for function in &graph.functions {
        iir::visit::expressions(&function.function, &mut 100_000, |expr, _| {
            if let nir::program::ExprKind::Closure {
                captures: fields, ..
            } = &expr.kind
            {
                captures += fields.iter().filter(|field| &field.ty == logical).count();
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(
        captures > 0,
        "the logical capture must survive specialization"
    );
    assert!(
        graph
            .to_string()
            .contains("(template-storage (logical-type")
    );
    let mut missing = graph.clone();
    missing.templates.clear();
    assert!(missing.validate(&mut 100_000).is_err());
    let mut duplicate = graph.clone();
    duplicate.templates.extend(graph.templates.clone());
    assert!(
        duplicate
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("duplicate IIR template")
    );
    let mut staged_call = graph.clone();
    let mut changed = false;
    for function in &mut staged_call.functions {
        iir::visit::expression_mut(&mut function.function.body, &mut 100_000, |expr, _| {
            if !changed {
                if let nir::program::ExprKind::Call { callee, .. } = &mut expr.kind {
                    callee.ty = logical.clone();
                    callee.kind = nir::program::ExprKind::Function(target.declaration);
                    changed = true;
                }
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(changed);
    assert!(
        staged_call
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("retains staged inputs")
    );
}

#[test]
fn nested_runtime_captures_retain_generic_template_identity() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->T;
         fn keep(comptime T:Type,value:T)->T where Marker {value}
         fn run()->bool {
             let callback:Callback=keep<bool>;
             let alias=callback;
             let outer=||{let inner=||alias(bool,true);inner()};
             outer() and outer()
         }",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 200_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn runtime_selected_template_capture_preserves_selection_and_argument_counts() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Count=.u8'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         fn yes(comptime T:Type,value:T)->bool {true}
         fn no(comptime T:Type,value:T)->bool {false}
         fn select(flag:bool,count:*Count)->bool {let before=count.*;assumes before < 255;count.* = before + 1;flag}
         fn argument(count:*Count)->bool {let before=count.*;assumes before < 255;count.* = before + 1;true}
         fn trial(flag:bool)->bool {
             var selections:Count=0;
             var arguments:Count=0;
             let callback:Callback=if select(flag,selections.ref) {yes} else {no};
             let count=arguments.ref;
             let invoke=||callback(bool,argument(count));
             let first=invoke();
             let second=invoke();
             first == flag and second == flag and selections == 1 and arguments == 2
         }
         fn run()->bool {trial(true) and trial(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 500_000).unwrap();
    assert!(graph.to_string().contains("(template-equal "));
    let mut malformed = graph.clone();
    let mut changed = false;
    for function in &mut malformed.functions {
        iir::visit::expression_mut(&mut function.function.body, &mut 100_000, |expr, _| {
            if let nir::program::ExprKind::TemplateEqual(_, right) = &mut expr.kind {
                right.ty = Term::Primitive(Primitive::Bool);
                changed = true;
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(changed);
    assert!(
        malformed
            .validate(&mut 100_000)
            .unwrap_err()
            .contains("template comparison")
    );
    assert!(
        support::compile_with_protocols(
            "typealias Type=.Type'builtin;fn target(comptime T:Type,value:T)->T {value}
         fn run()->bool {let callback=target;callback == callback}",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn runtime_template_dispatch_distinguishes_arguments_of_the_same_declaration() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         fn selected(comptime T:Type,value:T)->bool where answer:bool {answer}
         fn trial(flag:bool)->bool {
             let callback:Callback=flag match {true=>selected<true>,false=>selected<false>,};
             let alias=callback;
             let outer=||{let inner=||alias(bool,true);inner()};
             outer() == flag and outer() == flag
         }
         fn run()->bool {trial(true) and trial(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn mutable_template_captures_keep_snapshots_and_private_persistent_cells() {
    let prefix = "typealias Type=.Type'builtin;
        typealias Callback=fn(comptime T:Type,value:T)->bool;
        fn yes(comptime T:Type,value:T)->bool {true}
        fn no(comptime T:Type,value:T)->bool {false}";
    for body in [
        "var callback:Callback=yes;
         let earlier=||callback(bool,true);
         callback=no;
         let later=||callback(bool,true);
         earlier() and not later() and not callback(bool,true)",
        "var callback:Callback=yes;
         let next=||{let previous=callback(bool,true);callback=no;previous};
         let sibling=||callback(bool,true);
         let first=next();let second=next();
         first and not second and sibling() and callback(bool,true)",
        "var callback:Callback=yes;
         let outer=||{let before=callback(bool,true);callback=no;
             let inner=||{let saved=callback(bool,true);callback=yes;saved};
             let first=inner();let second=inner();before and not first and second};
         outer() and not outer() and callback(bool,true)",
        "var first:Callback=yes;var second:Callback=no;
         let original=||first(bool,true);
         first=second;second=first;
         let copied=||second(bool,true);
         first=yes;
         original() and not copied() and first(bool,true) and not second(bool,true)",
        "var callback:Callback=yes;
         let saved=||callback(bool,true);
         var other:Callback=yes;other=no;
         saved() and not other(bool,true)",
        "var callback:Callback=yes;
         let toggle=||{let before=callback(bool,true);
             if before {callback=no;} else {callback=yes;}
             before};
         let first=toggle();let second=toggle();let third=toggle();
         first and not second and third and callback(bool,true)",
    ] {
        let source = format!("{prefix} fn run()->bool {{{body}}}");
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 500_000).unwrap(),
            Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn unknown_assignment_does_not_leave_a_partial_template_domain() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         fn yes(comptime T:Type,value:T)->bool {true}
         fn no(comptime T:Type,value:T)->bool {false}
         fn trial(input:Callback)->bool {
             var callback:Callback=yes;
             let previous=callback;
             callback=input;
             let alias=callback;
             let invoke=||alias(bool,true);
             invoke()
         }
         fn run()->bool {trial(no)}",
    );
    // A direct caller can specialize its argument, but the open entry cannot
    // treat its known `yes` initializer as the whole domain of an unknown input.
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(false)
    );
    assert!(luna::instantiation::instantiate(&package, "trial", 500_000).is_err());
}

#[test]
fn local_pointer_writes_update_template_domains_without_replaying_stores() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Type=.Type'builtin;
        typealias Callback=fn(comptime T:Type,value:T)->bool;
        impl Duplicate for Callback {}
        fn yes(comptime T:Type,value:T)->bool {true}
        fn no(comptime T:Type,value:T)->bool {false}";
    for body in [
        "var callback:Callback=yes;
         let earlier=||callback(bool,true);
         let pointer=callback.ref;let alias=pointer;
         alias.* = no;
         let later=||callback(bool,true);
         earlier() and not later() and not callback(bool,true)",
        "var callback:Callback=yes;
         let toggle=||{let before=callback(bool,true);let pointer=callback.ref;
             pointer.* = if before {no} else {yes};before};
         let first=toggle();let second=toggle();let third=toggle();
         first and not second and third and callback(bool,true)",
        "var callback:Callback=yes;
         let pointer=callback.ref;
         pointer.* = no;
         let loaded=pointer.*;
         let saved=||callback(bool,true);
         not saved() and not loaded(bool,true)",
    ] {
        let source = format!("{prefix} fn run()->bool {{{body}}}");
        let package = support::compile(&source);
        assert!(
            luna::execution::interpret(&package, "run", 500_000)
                .unwrap_err()
                .contains("Unsupported")
        );
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500_000
            )
            .unwrap(),
            Value::Bool(true),
            "{body}"
        );
    }
}

#[test]
fn template_identity_dispatch_observes_without_requiring_copy() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         fn yes(comptime T:Type,value:T)->bool {true}
         fn no(comptime T:Type,value:T)->bool {false}
         fn trial(flag:bool)->bool {
             let callback:Callback=if flag {yes} else {no};
             let invoke=||callback(bool,true);
             invoke() == flag
         }
         fn run()->bool {trial(true) and trial(false)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn known_template_identity_does_not_authorize_use_after_move() {
    for selected in ["yes", "if flag {yes} else {no}"] {
        let package = support::compile(&format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             typealias Type=.Type'builtin;
             typealias Callback=fn(comptime T:Type,value:T)->bool;
             fn yes(comptime T:Type,value:T)->bool {{true}}
             fn no(comptime T:Type,value:T)->bool {{false}}
             fn trial(flag:bool)->bool {{
                 let callback:Callback={selected};
                 let invoke=||{{let consumed=callback;callback(bool,true)}};
                 invoke()
             }}
             fn run()->bool {{trial(true)}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500_000,
        )
        .unwrap_err();
        assert!(error.contains("unavailable"), "{selected}: {error}");
    }
}

#[test]
fn template_pointer_callee_is_snapshotted_before_argument_writes() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         impl Duplicate for Callback {}
         fn yes(comptime T:Type,value:T)->bool {true}
         fn no(comptime T:Type,value:T)->bool {false}
         fn run()->bool {
             var callback:Callback=yes;
             var calls:.u8'builtin=0;
             let pointer=callback.ref;
             let first=pointer.*(bool,do {let before=calls;assumes before < 255;
                 calls = before + 1;pointer.* = no;true});
             let second=pointer.*(bool,do {let before=calls;assumes before < 255;
                 calls = before + 1;pointer.* = yes;true});
             first and not second and pointer.*(bool,true) and calls == 2
         }",
    );
    assert!(luna::instantiation::instantiate(&package, "run", 1).is_err());
    // Residual range checking now covers the expanded dispatch branches and
    // their argument snapshots in addition to ownership and interpretation.
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            2_000_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_trait_templates_keep_subject_selection_and_mix_with_named_functions() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         trait Picker {fn pick(comptime T:Type,value:T)->bool;}
         impl Picker for bool {fn pick(comptime T:Type,value:T)->bool {true}}
         impl Picker for .u8'builtin {fn pick(comptime T:Type,value:T)->bool {false}}
         fn plain(comptime T:Type,value:T)->bool {false}
         fn trial(flag:bool)->bool {
             let first:Callback=if flag {bool.as(Picker).pick} else {.u8'builtin.as(Picker).pick};
             let second:Callback=if flag {bool.as(Picker).pick} else {plain};
             let invoke=||first(bool,true) == flag and second(bool,false) == flag;
             invoke()
         }
         fn run()->bool {trial(true) and trial(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let mut graph = luna::instantiation::instantiate(&package, "run", 500_000).unwrap();
    // Distinct interface identities must remain distinct even if code sharing
    // maps their stored implementation handles to the same body instance.
    let shared = graph
        .dispatch
        .iter()
        .flat_map(|selected| &selected.methods)
        .next()
        .unwrap()
        .clone();
    for selected in &mut graph.dispatch {
        for method in &mut selected.methods {
            method.body = shared.body;
            method.arguments = shared.arguments.clone();
        }
    }
    graph.validate(&mut 500_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(
        &graph,
        comptime_interpretor::Budget {
            steps: 500_000,
            depth: 128,
        },
    )
    .unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_trait_templates_keep_method_arguments_and_resource_observation() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         trait Picker {fn pick(comptime T:Type,value:T)->bool where answer:bool;}
         impl Picker for bool {fn pick(comptime T:Type,value:T)->bool where answer:bool {answer}}
         fn trial(flag:bool)->bool {
             let callback:Callback=if flag {bool.as(Picker).pick<true>} else {bool.as(Picker).pick<false>};
             let invoke=||callback(bool,true);
             invoke() == flag
         }
         fn run()->bool {trial(true) and trial(false)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap(),
        Value::Bool(true)
    );
    let mut contracted = package.clone();
    let method = &mut contracted
        .traits
        .iter_mut()
        .find(|t| t.name == "Picker")
        .unwrap()
        .methods[0];
    method.program.preconditions.push(nir::program::Expr {
        source: nir::program::SourceRange { start: 0, end: 1 },
        ty: Term::Primitive(Primitive::Bool),
        kind: nir::program::ExprKind::Literal(nir::program::Literal::Bool(false)),
    });
    let graph = luna::instantiation::instantiate(&contracted, "run", 500_000).unwrap();
    assert_eq!(
        graph
            .functions
            .iter()
            .filter(|f| f.function.name == "pick")
            .count(),
        2
    );
    assert!(
        graph
            .functions
            .iter()
            .filter(|f| f.function.name == "pick")
            .all(|f| !f.function.preconditions.is_empty())
    );
    let error = luna::execution::interpret(&contracted, "run", 500_000).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
}

#[test]
fn captured_default_method_templates_instantiate_self_and_method_inputs() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->bool;
         trait Picker {fn pick(comptime T:Type,value:T)->bool where answer:bool {answer}}
         impl Picker for bool {}
         fn trial(flag:bool)->bool {
             let callback:Callback=if flag {bool.as(Picker).pick<true>} else {bool.as(Picker).pick<false>};
             let outer=||{let inner=||callback(bool,true);inner()};
             outer() == flag
         }
         fn run()->bool {trial(true) and trial(false)}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_runtime_method_templates_enforce_interface_preconditions() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         trait Guard {fn check(comptime T:Type,allowed:bool)->bool where requires allowed;}
         impl Guard for bool {fn check(comptime T:Type,allowed:bool)->bool {true}}
         fn good()->bool {let callback=bool.as(Guard).check;let invoke=||callback(bool,true);invoke()}
         fn bad()->bool {let callback=bool.as(Guard).check;let invoke=||callback(bool,false);invoke()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "good", 500_000).unwrap(),
        Value::Bool(true)
    );
    let error = luna::execution::interpret(&package, "bad", 500_000).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
    // Raw NIR execution must not silently replace this identity with the
    // implementation's weaker-domain function value.
    let mut raw = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    assert_eq!(
        raw.invoke(support::id(&package, "good"), vec![])
            .unwrap_err()
            .kind,
        comptime_interpretor::ErrorKind::Unsupported
    );
}

#[test]
fn captured_runtime_method_templates_retain_result_postconditions() {
    let mut package = support::compile(
        "typealias Type=.Type'builtin;
         trait Guard {fn check(comptime T:Type,value:bool)->result:bool where ensures result;}
         impl Guard for bool {fn check(comptime T:Type,value:bool)->bool {true}}
         fn run()->bool {let callback=bool.as(Guard).check;let outer=||{let inner=||callback(bool,false);inner()};outer()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    // Remove inherited implementation contracts to isolate the contract carried
    // by the stored interface method through specialization and nested capture.
    for function in &mut package.functions {
        if function.name == "check" {
            function.postconditions.clear();
            function.result_binding = None;
        }
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let contract = &mut package
        .traits
        .iter_mut()
        .find(|t| t.name == "Guard")
        .unwrap()
        .methods[0]
        .program;
    assert_eq!(contract.postconditions.len(), 1);
    contract.postconditions[0].kind =
        nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let error = luna::execution::interpret(&package, "run", 500_000).unwrap_err();
    assert!(
        error.contains("postcondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
}

#[test]
fn captured_runtime_method_templates_snapshot_entry_values_before_mutation() {
    let mut package = support::compile(
        "typealias Type=.Type'builtin;
         trait Keep {fn keep(comptime T:Type,value:bool)->result:bool where ensures result == value'old;}
         impl Keep for bool {fn keep(comptime T:Type,value:bool)->bool {let before=value;value=not value;before}}
         fn trial(flag:bool)->bool {let callback=bool.as(Keep).keep;let invoke=||callback(bool,flag);invoke()==flag}
         fn run()->bool {trial(true) and trial(false)}",
    );
    // Force specialization to rebuild the entry snapshots from the interface,
    // independently of the implementation's inherited definition contract.
    for function in &mut package.functions {
        if function.name == "keep" {
            function.postconditions.clear();
            function.result_binding = None;
            function.entry_snapshots.clear();
        }
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let implementation = package
        .functions
        .iter_mut()
        .find(|f| f.name == "keep")
        .unwrap();
    implementation.body.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let error = luna::execution::interpret(&package, "run", 500_000).unwrap_err();
    assert!(
        error.contains("postcondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
}

#[test]
fn comptime_factories_transport_contracted_runtime_method_templates() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:bool)->bool;
         trait Guard {fn check(comptime T:Type,value:bool)->result:bool where requires value, ensures result == value'old;}
         impl Guard for bool {fn check(comptime T:Type,value:bool)->bool {value}}
         comptime fn make()->Callback {bool.as(Guard).check}
         trait Source {assoc callback:Callback=make();}
         struct Holder {} impl Source for Holder {}
         fn good()->bool {let callback=Holder.as(Source).callback;let invoke=||callback(bool,true);invoke()}
         fn bad()->bool {let callback=Holder.as(Source).callback;let invoke=||callback(bool,false);invoke()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "good", 500_000).unwrap(),
        Value::Bool(true)
    );
    let error = luna::execution::interpret(&package, "bad", 500_000).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
    let value = luna::invocation::invoke(&package, "make", 500_000).unwrap();
    assert!(matches!(value, Value::RuntimeTraitMethod(_)));
}

#[test]
fn computed_method_templates_keep_selected_interface_domains() {
    let prefix = "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:bool)->bool;
         trait Guard where expected:bool {fn check(comptime T:Type,value:bool)->result:bool where requires value == expected, ensures result == value'old;}
         impl Guard<expected> for bool where expected:bool {fn check(comptime T:Type,value:bool)->bool {value}}
         comptime fn make(expected:bool)->Callback {
             if expected {bool.as(Guard<true>).check} else {bool.as(Guard<false>).check}
         }
         trait Source where expected:bool {assoc callback:Callback=make(expected);}
         struct Holder where expected:bool {}
         impl Source<expected> for Holder<expected> where expected:bool {}";
    let package = support::compile(&format!("{prefix}
         fn trial(flag:bool)->bool {{
             let callback=if flag {{Holder<true>.as(Source<true>).callback}} else {{Holder<false>.as(Source<false>).callback}};
             let invoke=||callback(bool,flag);invoke()==flag
         }}
         fn run()->bool {{trial(true) and trial(false)}}
         fn bad()->bool {{let callback=Holder<false>.as(Source<false>).callback;let invoke=||callback(bool,true);invoke()}}"));
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let error = luna::execution::interpret(&package, "bad", 500_000).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
    let error = support::compile_with_protocols(
        &format!(
            "{prefix}
        comptime fn wrong()->bool {{let callback=make(true);callback(bool,true)}}"
        ),
        &Default::default(),
    )
    .unwrap_err();
    assert!(error.contains("phase"), "{error}");
}

#[test]
fn captured_named_templates_preserve_selected_preconditions_and_entry_snapshots() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         fn checked(comptime T:Type,value:bool)->result:bool
             where expected:bool, requires value == expected, ensures result == value'old {
             let before=value;value=not value;before
         }
         fn trial(flag:bool)->bool {
             let callback=if flag {checked<true>} else {checked<false>};
             let outer=||{let inner=||callback(bool,flag);inner()};outer()==flag
         }
         fn run()->bool {trial(true) and trial(false)}
         fn bad()->bool {let callback=checked<false>;let invoke=||callback(bool,true);invoke()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    let error = luna::execution::interpret(&package, "bad", 500_000).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
    let mut broken = package.clone();
    broken
        .functions
        .iter_mut()
        .find(|f| f.name == "checked")
        .unwrap()
        .body
        .kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let error = luna::execution::interpret(&broken, "run", 500_000).unwrap_err();
    assert!(
        error.contains("postcondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
}

#[test]
fn comptime_factories_transport_contracted_named_templates() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}
         typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:bool)->bool;
         fn checked(comptime T:Type,value:bool)->result:bool
             where requires value, ensures result == value'old {value}
         comptime fn make()->Callback {checked}
         trait Source {assoc callback:Callback=make();}
         struct Holder {} impl Source for Holder {}
         fn good()->bool {let callback=Holder.as(Source).callback;let invoke=||callback(bool,true);invoke()}
         fn bad()->bool {let callback=Holder.as(Source).callback;let invoke=||callback(bool,false);invoke()}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "good", 500_000).unwrap(),
        Value::Bool(true)
    );
    let error = luna::execution::interpret(&package, "bad", 500_000).unwrap_err();
    assert!(
        error.contains("precondition") || error.contains("ProgramContractRejected"),
        "{error}"
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "good", "Duplicate", "Retire", 500_000)
            .unwrap(),
        Value::Bool(true)
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "bad", "Duplicate", "Retire", 500_000)
            .unwrap_err();
    assert!(
        error.contains("residual program contract") && error.contains("precondition"),
        "{error}"
    );
    // A full package has the declaration body; dependency-only snapshots use
    // FunctionInstance. Both must retain exactly the checked declaration.
    let expected = support::id(&package, "checked");
    match luna::invocation::invoke(&package, "make", 500_000).unwrap() {
        Value::Function(declaration) => assert_eq!(declaration, expected),
        Value::FunctionInstance {
            declaration,
            arguments,
        } => {
            assert_eq!(declaration, expected);
            assert!(arguments.is_empty());
        }
        other => panic!("factory lost declaration identity: {other:?}"),
    }
}

#[test]
fn residual_callable_contracts_are_checked_against_exact_instance_keys() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         fn checked(comptime T:Type,value:bool)->result:bool
             where expected:bool, requires value == expected, ensures result == value'old {value}
         fn good()->bool {
             let first=checked<true>;let second=checked<false>;
             let invoke=||first(bool,true) and not second(bool,false);invoke()
         }
         fn bad()->bool {let callback=checked<false>;let invoke=||callback(bool,true);invoke()}",
    );
    for (entry, valid) in [("good", true), ("bad", false)] {
        let graph = luna::instantiation::instantiate(&package, entry, 500_000).unwrap();
        let mut residual = package.clone();
        residual.runtime_instances = graph
            .functions
            .iter()
            .map(|instance| nir::program::RuntimeInstance {
                declaration: instance.key.declaration,
                arguments: instance.key.arguments.to_vec(),
                function: instance.function.clone(),
                cleanup_accesses: vec![],
            })
            .collect();
        let root = graph
            .functions
            .iter()
            .find(|f| f.key == graph.roots[0])
            .unwrap();
        let report = elaboration::verification::flow::analyze_in_package(
            &residual,
            &root.function,
            &mut Default::default(),
        );
        if valid {
            report.unwrap();
            let mut exhausted = elaboration::verification::Limits::default();
            exhausted.arithmetic.steps = 0;
            assert!(
                elaboration::verification::flow::analyze_in_package(
                    &residual,
                    &root.function,
                    &mut exhausted
                )
                .is_err()
            );
            let mut duplicate = residual.clone();
            let target = duplicate
                .runtime_instances
                .iter()
                .find(|f| f.function.name == "checked")
                .unwrap()
                .clone();
            duplicate.runtime_instances.push(target);
            assert!(
                elaboration::verification::flow::analyze_in_package(
                    &duplicate,
                    &root.function,
                    &mut Default::default()
                )
                .is_err()
            );
            elaboration::verification::flow::analyze_in_package(
                &residual,
                &root.function,
                &mut Default::default(),
            )
            .unwrap();
        } else {
            let error = report
                .err()
                .expect("invalid call must fail static contract checking")
                .to_string();
            assert!(error.contains("precondition"), "{error}");
        }
    }
}

#[test]
fn residual_template_preconditions_require_explicit_assumptions() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Type=.Type'builtin;
        fn checked(comptime T:Type,value:bool)->bool where requires value {value}";
    for (proof, accepted) in [("", false), ("assumes flag;", true)] {
        let package = support::compile(&format!(
            "{prefix}
            fn trial(flag:bool)->bool {{
                {proof}
                let callback=checked;let invoke=||callback(bool,flag);invoke()
            }}
            fn run()->bool {{trial(true)}}
            fn failed_check()->bool {{trial(false)}}"
        ));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500_000,
        );
        if accepted {
            assert_eq!(result.unwrap(), Value::Bool(true));
            let error = luna::execution::interpret_with_protocols(
                &package,
                "failed_check",
                "Duplicate",
                "Retire",
                500_000,
            )
            .unwrap_err();
            assert!(error.contains("ProgramContractRejected"), "{error}");
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("residual program contract") && error.contains("precondition"),
                "{error}"
            );
        }
    }
}

#[test]
fn higher_order_cleanup_contexts_do_not_skip_residual_call_contracts() {
    let prefix = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Type=.Type'builtin;
        fn checked(comptime T:Type,value:bool)->bool where requires value {value}
        fn answer()->bool {true}";
    for (contract, valid) in [("", false), ("where requires value", true)] {
        let package = support::compile(&format!(
            "{prefix}
            fn apply(action:fn()->bool,value:bool)->bool {contract} {{
                let target=checked;target(bool,value);action()
            }}
            fn run()->bool {{apply(answer,true)}}"
        ));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500_000,
        );
        if valid {
            assert_eq!(result.unwrap(), Value::Bool(true));
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("residual program contract") && error.contains("precondition"),
                "{error}"
            );
        }
    }
}

#[test]
fn residual_contract_diagnostics_recover_expanded_source_locations() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        typealias Type=.Type'builtin;
        typealias Callback=fn(comptime T:Type,value:bool)->bool;
        fn checked(comptime T:Type,value:bool)->bool where requires value {value}
        inline fn wrapper(callback:Callback,value:bool)->bool {callback(bool,value)}
        fn run()->bool {wrapper(checked,true)}";
    let mut package = support::compile(source);
    package
        .functions
        .iter_mut()
        .find(|f| f.name == "checked")
        .unwrap()
        .preconditions[0]
        .kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(false));
    let graph = luna::instantiation::instantiate(&package, "run", 500_000).unwrap();
    let mut residual = package.clone();
    residual.runtime_instances = graph
        .functions
        .iter()
        .map(|instance| nir::program::RuntimeInstance {
            declaration: instance.key.declaration,
            arguments: instance.key.arguments.to_vec(),
            function: instance.function.clone(),
            cleanup_accesses: vec![],
        })
        .collect();
    let root = graph
        .functions
        .iter()
        .find(|f| f.key == graph.roots[0])
        .unwrap();
    let error = elaboration::verification::flow::analyze_in_package(
        &residual,
        &root.function,
        &mut Default::default(),
    )
    .err()
    .unwrap();
    assert!(
        error.source.expansion_index().is_some(),
        "expected an expanded call: {error}"
    );
    let original = error.source.original(&graph.expansion_sources).unwrap();
    assert!(original.expansion_index().is_none());
    assert!((original.end as usize) <= source.len());
    let failure =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500_000)
            .unwrap_err();
    assert!(
        failure.contains(&format!("(source {} {})", original.start, original.end)),
        "{failure}"
    );
    assert!(!failure.contains("4294967295"), "{failure}");
}

#[test]
fn runtime_type_inputs_do_not_become_static_type_evidence() {
    for declaration in [
        "typealias Bad=fn(T:Type,value:T)->T;",
        "typealias Bad=fn(comptime size:.u8'builtin,value:size)->bool;",
    ] {
        let error = support::compile_with_protocols(
            &format!("typealias Type=.Type'builtin;{declaration}"),
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error.contains("earlier explicit comptime Type parameter"),
            "{error}"
        );
    }
}

#[test]
fn unsafe_callable_types_preserve_dependent_telescope_and_phase_in_source() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Raw=unsafe fn(comptime T:Type,value:T)->T;
         fn retain(value:Raw)->Raw {value}
         comptime fn staged(value:comptime unsafe fn(bool)->bool)->comptime unsafe fn(bool)->bool {value}
         fn nested(value:unsafe fn(bool)->unsafe fn(bool)->bool)->unsafe fn(bool)->unsafe fn(bool)->bool {value}",
    );
    for name in ["retain", "staged", "nested"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let Term::Function { parameters, result } = &function.signature else {
            panic!("{}", function.signature)
        };
        assert_eq!(&parameters[0].ty, result.as_ref());
        assert!(result.is_unsafe_callable());
        assert_eq!(result.is_comptime_callable(), name == "staged");
        result.check_scope(&[]).unwrap();
    }
    let retain = package
        .functions
        .iter()
        .find(|f| f.name == "retain")
        .unwrap();
    let Term::Function { result, .. } = &retain.signature else {
        unreachable!()
    };
    assert_eq!(
        result.callable_signature(),
        &Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::Type,
                    ty: Term::Primitive(Primitive::Type)
                },
                Parameter {
                    sort: Sort::Runtime,
                    ty: Term::Bound {
                        depth: 0,
                        index: 0,
                        sort: Sort::Type
                    }
                }
            ],
            result: Box::new(Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type
            }),
        }
    );
}

#[test]
fn unsafe_callable_source_types_cannot_erase_responsibility_or_phase() {
    for source in [
        "fn erase(value:unsafe fn()->bool)->fn()->bool {value}",
        "fn erase(value:fn()->bool)->unsafe fn()->bool {value}",
        "fn erase(value:unsafe fn()->bool)->comptime unsafe fn()->bool {value}",
        "fn invoke(value:unsafe fn()->bool)->bool {value()}",
    ] {
        let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(!error.contains("panicked"), "{source}: {error}");
        if source.contains("fn invoke") {
            assert!(
                error.contains("unsafe function call requires an unsafe boundary"),
                "{source}: {error}"
            );
        }
    }
}

#[test]
fn unsafe_callable_type_values_and_environment_qualification_keep_identity() {
    let package = support::compile(
        "typealias Raw=%{flag:bool} unsafe fn(bool)->bool;
         fn retain(value:Raw)->Raw {value}
         comptime fn same()->bool {(unsafe fn(bool)->bool) == (unsafe fn(bool)->bool)}
         comptime fn distinct()->bool {(unsafe fn(bool)->bool) == (fn(bool)->bool)}",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "same", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "distinct", 100_000).unwrap(),
        Value::Bool(false)
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "retain")
        .unwrap();
    let Term::Function { result, .. } = &function.signature else {
        unreachable!()
    };
    let Term::Closure {
        environment,
        signature,
    } = result.as_ref()
    else {
        panic!("{result}")
    };
    assert_eq!(environment, &[Term::Primitive(Primitive::Bool)]);
    assert!(signature.is_unsafe_callable());
}

#[test]
fn materialized_template_tables_share_logical_cores_without_sharing_targets() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->T;
         fn first(comptime T:Type,value:T)->T {value}
         fn second(comptime T:Type,value:T)->T {value}
         fn run()->bool {
             let a:Callback=first;
             let b:Callback=second;
             let invoke=||a(bool,b(bool,true));
             invoke()
         }",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let table = |name| {
        let id = support::id(&package, name);
        graph
            .templates
            .iter()
            .find(|table| table.targets().iter().any(|key| key.declaration == id))
            .unwrap()
    };
    let first = table("first");
    let second = table("second");
    assert_ne!(first.targets(), second.targets());
    assert!(std::sync::Arc::ptr_eq(
        first.logical_type_reference().core(),
        second.logical_type_reference().core()
    ));
    assert_ne!(first.decode(0).unwrap(), second.decode(0).unwrap());
    let snapshot = graph.clone();
    for (original, copied) in graph.functions.iter().zip(&snapshot.functions) {
        assert!(std::sync::Arc::ptr_eq(
            &original.key.arguments,
            &copied.key.arguments
        ));
    }
    for (original, copied) in graph.templates.iter().zip(&snapshot.templates) {
        for (left, right) in original.targets().iter().zip(copied.targets()) {
            assert!(std::sync::Arc::ptr_eq(&left.arguments, &right.arguments));
        }
    }
    graph.validate(&mut 100_000).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn template_storage_and_frozen_callable_evidence_share_one_materialization_pool() {
    let package = support::compile(
        "typealias Type=.Type'builtin;
         typealias Callback=fn(comptime T:Type,value:T)->T;
         fn keep(comptime T:Type,value:T)->T {value}
         pure comptime fn Carrier<action:Callback>->Type {bool}
         fn witness()->bool where T {true}
         fn run()->bool {
             let callback:Callback=keep;
             let invoke=||callback(bool,true);
             witness<Carrier<keep>>() and invoke()
         }",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let declaration = support::id(&package, "keep");
    let evidence = graph
        .callable_identities
        .iter()
        .find(|identity| identity.key.declaration == declaration)
        .unwrap();
    let storage = graph
        .templates
        .iter()
        .find(|table| table.targets().contains(&evidence.key))
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(
        evidence.signature.core(),
        storage.logical_type_reference().core()
    ));
    graph.validate(&mut 100_000).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
