//! Memory origins and cleanup obligations carried by local closure environments.
#[path = "support/execution.rs"]
mod execution_support;
use comptime_interpretor::Value;
use execution_support::compile;

fn source_closure(package: &nir::program::Package) -> nir::program::Expr {
    let mut found = None;
    nir::visit::expression(
        &package
            .functions
            .iter()
            .find(|f| f.name == "specimen")
            .unwrap()
            .body,
        &mut 10_000,
        |expr, _| {
            if matches!(expr.kind, nir::program::ExprKind::Closure { .. }) {
                found = Some(expr.clone());
            }
            Ok(())
        },
    )
    .unwrap();
    found.unwrap()
}

#[test]
fn formal_code_exit_summary_tracks_assignment_return_and_unknown_paths() {
    use elaboration::verification::memory::{self, CallableInput, CallableResult, PreparedCode};
    use nir::{
        Primitive, Term,
        program::{Expr, ExprKind as K, Place},
    };
    use std::collections::BTreeMap;
    let package = compile(
        "fn first()->bool {false}fn second()->bool {true}
        fn specimen() {let action=|a:fn()->bool,b:fn()->bool,flag:bool| true;}",
    );
    let original = source_closure(&package);
    let key = |name: &str| iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration,
        arguments: vec![].into(),
    };
    let slot = |parameter| CallableInput {
        parameter,
        fields: vec![],
    };
    let inputs = BTreeMap::from([
        (slot(0), CallableResult::Known(key("first"))),
        (slot(1), CallableResult::Known(key("second"))),
    ]);
    for mode in [0, 1, 2, 3] {
        let mut definition = original.clone();
        let signature = definition.ty.callable_signature().clone();
        let Term::Function {
            parameters: types, ..
        } = signature
        else {
            unreachable!()
        };
        let K::Closure {
            parameters, body, ..
        } = &mut definition.kind
        else {
            unreachable!()
        };
        let source = body.source;
        let node = |ty, kind| Expr { ty, kind, source };
        let assign = node(
            Term::Primitive(Primitive::Void),
            K::Assign {
                place: Place {
                    root: parameters[0],
                    fields: vec![],
                },
                value: Box::new(if mode == 3 {
                    node(
                        types[1].ty.clone(),
                        K::Closure {
                            parameters: vec![],
                            captures: vec![],
                            body: body.clone(),
                        },
                    )
                } else {
                    node(types[1].ty.clone(), K::Local(parameters[1]))
                }),
            },
        );
        let branch = if mode == 2 {
            node(
                Term::Primitive(Primitive::Never),
                K::Block {
                    statements: vec![assign],
                    result: Some(Box::new(node(
                        Term::Primitive(Primitive::Never),
                        K::Return(Some(Box::new(body.as_ref().clone()))),
                    ))),
                },
            )
        } else {
            assign
        };
        let statement = if mode == 0 || mode == 3 {
            branch
        } else {
            node(
                Term::Primitive(Primitive::Void),
                K::If {
                    condition: Box::new(node(
                        Term::Primitive(Primitive::Bool),
                        K::Local(parameters[2]),
                    )),
                    then_branch: Box::new(branch),
                    else_branch: None,
                },
            )
        };
        *body = std::sync::Arc::new(node(
            Term::Primitive(Primitive::Bool),
            K::Block {
                statements: vec![statement],
                result: Some(Box::new(body.as_ref().clone())),
            },
        ));
        let infer = |inputs: &BTreeMap<CallableInput, CallableResult>, fuel: &mut usize| {
            memory::infer_closure_with_inputs(&package, &definition, &BTreeMap::new(), inputs, fuel)
        };
        let summary = infer(&inputs, &mut 100_000).unwrap();
        let codes = summary.input_code_exits[&slot(0)].as_ref().unwrap();
        if mode == 3 {
            assert_eq!(codes.len(), 1);
            assert!(
                matches!(&codes[0],PreparedCode::Closure {definition,..} if matches!(&definition.kind,K::Closure {captures,..} if captures.is_empty()))
            );
        } else {
            let actual = codes
                .iter()
                .map(|code| match code {
                    PreparedCode::Named(key) => key.declaration,
                    _ => panic!("unexpected closure"),
                })
                .collect::<std::collections::BTreeSet<_>>();
            let expected = if mode == 0 {
                vec![key("second").declaration]
            } else {
                vec![key("first").declaration, key("second").declaration]
            };
            assert_eq!(actual, expected.into_iter().collect(), "{mode}: {summary}");
        }
        let dump = summary.to_string();
        assert!(dump.contains("(input-code-exits"));
        assert_eq!(dump.matches('(').count(), dump.matches(')').count());
        assert!(infer(&inputs, &mut 1).is_err());
        assert_eq!(infer(&inputs, &mut 100_000).unwrap(), summary);
        let unknown = infer(
            &BTreeMap::from([(slot(0), CallableResult::Known(key("first")))]),
            &mut 100_000,
        )
        .unwrap();
        if mode == 3 {
            assert_eq!(
                unknown.input_code_exits[&slot(0)],
                summary.input_code_exits[&slot(0)]
            );
        } else {
            assert_eq!(unknown.input_code_exits[&slot(0)], None);
            assert!(unknown.to_string().contains("(codes unknown)"));
        }
    }
}

#[test]
fn formal_pointer_exit_summary_tracks_assignment_and_each_return_path() {
    use elaboration::verification::memory::{self, CallableInput};
    use nir::{
        Primitive, Term,
        program::{Expr, ExprKind as K, Place},
    };
    let package = compile("fn specimen() {let action=|p:*bool,q:*bool,flag:bool| p;}");
    let original = source_closure(&package);
    for mode in [0, 1, 2] {
        let mut definition = original.clone();
        let K::Closure {
            parameters, body, ..
        } = &mut definition.kind
        else {
            unreachable!()
        };
        let source = body.source;
        let pointer = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
        let node = |ty, kind| Expr { ty, kind, source };
        // Lifted capture slots use formal input cells in the inference IR.
        // Exercise formal-cell transitions directly, independently of source capture lowering.
        let assign = node(
            Term::Primitive(Primitive::Void),
            K::Assign {
                place: Place {
                    root: parameters[0],
                    fields: vec![],
                },
                value: Box::new(node(pointer.clone(), K::Local(parameters[1]))),
            },
        );
        let branch = if mode == 2 {
            node(
                Term::Primitive(Primitive::Never),
                K::Block {
                    statements: vec![assign],
                    result: Some(Box::new(node(
                        Term::Primitive(Primitive::Never),
                        K::Return(Some(Box::new(node(
                            pointer.clone(),
                            K::Local(parameters[0]),
                        )))),
                    ))),
                },
            )
        } else {
            assign
        };
        let statement = if mode == 0 {
            branch
        } else {
            node(
                Term::Primitive(Primitive::Void),
                K::If {
                    condition: Box::new(node(
                        Term::Primitive(Primitive::Bool),
                        K::Local(parameters[2]),
                    )),
                    then_branch: Box::new(branch),
                    else_branch: None,
                },
            )
        };
        *body = std::sync::Arc::new(node(
            pointer,
            K::Block {
                statements: vec![statement],
                result: Some(Box::new(body.as_ref().clone())),
            },
        ));
        let infer = |fuel: &mut usize| memory::infer_closure(&package, &definition, fuel);
        let summary = infer(&mut 100_000).unwrap();
        let origins = summary
            .input_pointer_exits
            .get(&CallableInput {
                parameter: 0,
                fields: vec![],
            })
            .unwrap()
            .as_ref()
            .unwrap();
        let roots = origins
            .iter()
            .map(|origin| origin.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        let expected = if mode == 0 {
            [1].into_iter().collect()
        } else {
            [0, 1].into_iter().collect()
        };
        assert_eq!(roots, expected, "{mode}: {summary}");
        assert_eq!(
            summary
                .result_targets
                .as_ref()
                .unwrap()
                .iter()
                .map(|origin| origin.parameter)
                .collect::<std::collections::BTreeSet<_>>(),
            roots
        );
        assert!(summary.to_string().contains("(input-pointer-exits"));
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
    }
}

#[test]
fn formal_pointer_exit_summary_never_exports_an_invocation_local_root() {
    use elaboration::verification::memory::{self, CallableInput};
    use nir::{
        Primitive, Term,
        program::{Expr, ExprKind as K, LocalId, Place},
    };
    let package = compile("fn specimen() {let action=|p:*bool| true;}");
    let mut definition = source_closure(&package);
    let K::Closure {
        parameters, body, ..
    } = &mut definition.kind
    else {
        unreachable!()
    };
    let source = body.source;
    let local = LocalId {
        owner: parameters[0].owner,
        index: parameters[0].index + 100,
    };
    let node = |ty, kind| Expr { ty, kind, source };
    let boolean = Term::Primitive(Primitive::Bool);
    let pointer = Term::Pointer(Box::new(boolean.clone()));
    *body = std::sync::Arc::new(node(
        boolean.clone(),
        K::Block {
            statements: vec![
                node(
                    Term::Primitive(Primitive::Void),
                    K::Let {
                        binding: local,
                        value: Box::new(node(
                            boolean.clone(),
                            K::Literal(nir::program::Literal::Bool(true)),
                        )),
                    },
                ),
                node(
                    Term::Primitive(Primitive::Void),
                    K::Assign {
                        place: Place {
                            root: parameters[0],
                            fields: vec![],
                        },
                        value: Box::new(node(
                            pointer,
                            K::AddressOf(Box::new(node(boolean, K::Local(local)))),
                        )),
                    },
                ),
            ],
            result: Some(Box::new(body.as_ref().clone())),
        },
    ));
    let summary = memory::infer_closure(&package, &definition, &mut 100_000).unwrap();
    assert_eq!(
        summary.input_pointer_exits.get(&CallableInput {
            parameter: 0,
            fields: vec![]
        }),
        Some(&None)
    );
    assert!(
        summary
            .to_string()
            .contains("(input-pointer-exits (input (parameter 0) (fields) (targets unknown)))")
    );
}

#[test]
fn lifted_capture_exit_summary_preserves_input_field_paths() {
    use elaboration::verification::memory::{self, CallableInput};
    let package = compile(
        "struct Link {target:*bool}fn specimen(p:*bool) {let link=Link {target:p};let action=|| link.target;}",
    );
    let definition = source_closure(&package);
    let summary = memory::infer_closure_with_borrowed_captures(
        &package,
        &definition,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &mut 100_000,
    )
    .unwrap();
    let origins = summary
        .input_pointer_exits
        .get(&CallableInput {
            parameter: 0,
            fields: vec![0],
        })
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(origins.len(), 1);
    let origin = origins.first().unwrap();
    assert_eq!(origin.parameter, 0);
    assert_eq!(origin.input_fields, vec![0]);
    assert!(origin.fields.is_empty());
    assert_eq!(summary.result_targets.as_ref(), Some(origins));
}

#[test]
fn pointer_capture_rebinding_tracks_sequential_invocations() {
    let prefix="trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Link {target:*bool}
        fn direct(p:*bool,q:*bool)->bool {var current=p;let action=|| {let seen=current.*;current=q;seen};let first=action();let second=action();not first and second}
        fn field(p:*bool,q:*bool)->bool {var link=Link {target:p};let action=|| {let seen=link.target.*;link.target=q;seen};let first=action();let second=action();not first and second}
        fn argument(p:*bool,q:*bool)->bool {var current=p;let action=|next:*bool| {let seen=current.*;current=next;seen};let first=action(q);let second=action(p);not first and second}
        fn looped(p:*bool,q:*bool)->bool {var current=p;let action=|| {let seen=current.*;current=q;seen};var again=true;var seen=false;while true {seen=action();if not again {break;}again=false;}seen}
        fn immediate(p:*bool,q:*bool)->bool {var current=p;let seen=(|| {current=q;current.*})();seen and not current.*}";
    for helper in ["direct", "field", "argument", "immediate", "looped"] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};{helper}(first.flag.ref,second.flag.ref)}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap_or_else(|error| panic!("{helper}: {error}")),
            Value::Bool(true)
        );
        let function = package
            .functions
            .iter()
            .find(|function| function.name == helper)
            .unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 100_000).unwrap();
        let roots = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(roots, [0, 1].into_iter().collect(), "{helper}: {summary}");
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
    }
    let expired=compile(&format!("{prefix} fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};var current=first.flag.ref;
        let action=|next:*bool| {{let seen=current.*;current=next;seen}};
        action(second.flag.ref);let moved=second;action(first.flag.ref)
    }}"));
    assert!(
        luna::execution::interpret_with_protocols(&expired, "run", "Duplicate", "Retire", 300_000)
            .unwrap_err()
            .contains("cannot prepare resource-checked execution")
    );
    let retired_old=compile(&format!("{prefix} fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
        var current=first.flag.ref;let next=second.flag.ref;let action=|| {{let seen=current.*;current=next;seen}};
        action();let retired=first;action()
    }}"));
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &retired_old,
            "run",
            "Duplicate",
            "Retire",
            300_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    let escaped = compile(&format!(
        "{prefix} fn run()->bool {{let cell=false;var current=cell.ref;
        let action=|| {{let seen=current.*;let local=true;current=local.ref;seen}};action();action()
    }}"
    ));
    let error =
        luna::execution::interpret_with_protocols(&escaped, "run", "Duplicate", "Retire", 300_000)
            .unwrap_err();
    assert!(error.contains("memory call target unavailable"), "{error}");
}

#[test]
fn conditional_capture_rebinding_joins_normal_and_early_exits() {
    for body in [
        "if switch {current=q;}current",
        "if switch {current=q;return current;}current",
        "if not switch {return current;}current=q;current",
    ] {
        let source = format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            fn choose(p:*bool,q:*bool,flag:bool)->*bool {{
                var current=p;let action=|switch:bool| {{{body}}};
                action(flag);action(false)
            }}
            fn yes()->bool {{let first=false;let second=true;choose(first.ref,second.ref,true).*}}
            fn no()->bool {{let first=false;let second=true;choose(first.ref,second.ref,false).*}}"
        );
        let package = compile(&source);
        for (entry, expected) in [("yes", true), ("no", false)] {
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    entry,
                    "Duplicate",
                    "Retire",
                    300_000
                )
                .unwrap_or_else(|error| panic!("{body}: {error}")),
                Value::Bool(expected),
                "{body}: {entry}"
            );
        }
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "choose")
            .unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 100_000).unwrap();
        let roots = summary
            .result_targets
            .as_ref()
            .unwrap()
            .iter()
            .map(|origin| origin.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(roots, [0, 1].into_iter().collect(), "{body}: {summary}");
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
    }
}

#[test]
fn wrapped_pointer_capture_rebinding_preserves_private_layout_paths() {
    for (setup, body) in [
        (
            "var current=p.as(Pointer);",
            "let seen=current.as(*bool).*;current=q.as(Pointer);seen",
        ),
        (
            "var current=Outer {inner:Link {target:p.as(Pointer)}};",
            "let seen=current.inner.target.as(*bool).*;current.inner.target=q.as(Pointer);seen",
        ),
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            newtype Pointer=*bool;impl Duplicate for Pointer {{}}
            struct Link {{target:Pointer}}struct Outer {{inner:Link}}
            fn switched(p:*bool,q:*bool)->bool {{
                {setup}let action=|| {{{body}}};let first=action();let second=action();not first and second
            }}
            fn run()->bool {{let first=false;let second=true;switched(first.ref,second.ref)}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap_or_else(|error| panic!("{setup}: {error}")),
            Value::Bool(true)
        );
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "switched")
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            &mut 100_000,
        )
        .unwrap();
        let roots = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(roots, [0, 1].into_iter().collect(), "{setup}: {summary}");
    }
}

#[test]
fn memory_summary_keeps_copied_environment_states_independent() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Callback=%{current:*bool,q:*bool} fn()->*bool;
        fn copied(p:*bool,q:*bool)->*bool {
            var current=p;let action:Callback=|| {let seen=current;current=q;seen};
            let other=action;action();other()
        }";
    // This tests origin isolation only. The independent resource layer must
    // still establish Copy before admitting both uses of the source closure.
    let package = compile(prefix);
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "copied")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        function.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    let roots = summary
        .result_targets
        .unwrap()
        .into_iter()
        .map(|target| target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(roots, [0].into_iter().collect());
}

#[test]
fn capture_pointer_exit_evidence_separates_success_failure_and_consumption() {
    use elaboration::verification::moves::{self, Delivery};
    for (body, success, failure, retained) in [
        ("link.target.*", true, false, true),
        ("assumes flag;link.target.*", true, true, true),
        ("let consumed=link;true", true, false, false),
    ] {
        let package=compile(&format!("struct Link {{target:*bool}}
            fn inspect(flag:bool) {{let cell=true;let link=Link {{target:cell.ref}};let action=|| {{{body}}};action();}}"));
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "inspect")
            .unwrap();
        let analyze = |fuel| {
            moves::analyze_in_package(
                &package,
                function,
                |ty| {
                    Ok(
                        if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                            Delivery::Copy
                        } else {
                            Delivery::Move
                        },
                    )
                },
                fuel,
            )
        };
        let report = analyze(100_000).unwrap();
        let invocation = &report
            .closures
            .iter()
            .find(|closure| closure.invoked)
            .unwrap()
            .invocation;
        assert_eq!(!invocation.exits.is_empty(), success, "{body}: {report}");
        assert_eq!(
            !invocation.failed_exits.is_empty(),
            failure,
            "{body}: {report}"
        );
        assert_eq!(
            invocation.captured_pointer_exits.len(),
            invocation.exits.len()
        );
        assert_eq!(
            invocation.failed_capture_pointer_exits.len(),
            invocation.failed_exits.len()
        );
        assert_eq!(
            invocation.effect_capture_pointer_exits.len(),
            invocation.effect_exits.len()
        );
        for exit in invocation
            .captured_pointer_exits
            .iter()
            .chain(&invocation.failed_capture_pointer_exits)
        {
            assert_eq!(!exit.is_empty(), retained, "{body}: {report}");
            for (slot, targets) in exit {
                assert_eq!(slot.fields, vec![0]);
                assert_eq!(targets.len(), 1);
                for target in targets {
                    assert!(
                        invocation.borrowed_roots.contains_key(&target.root),
                        "{report}"
                    );
                }
            }
        }
        let dump = report.to_string();
        assert!(dump.contains("(captured-pointer-exits"));
        assert!(dump.contains("(failed-capture-pointer-exits"));
        assert_eq!(
            dump.chars().filter(|c| *c == '(').count(),
            dump.chars().filter(|c| *c == ')').count()
        );
        assert!(analyze(1).is_err());
        assert_eq!(analyze(100_000).unwrap().to_string(), dump);
    }
}

#[test]
fn suspended_closure_exit_retains_its_own_capture_pointer_snapshot() {
    use elaboration::verification::moves::{self, Delivery};
    let package=compile("effect poll()->bool;struct Link {target:*bool}
        fn inspect() {let cell=true;let link=Link {target:cell.ref};let action:%{link:Link} fn()->#poll bool=|| {poll()#;link.target.*};}");
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "inspect")
        .unwrap();
    let report = moves::analyze_in_package(
        &package,
        function,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        100_000,
    )
    .unwrap();
    let invocation = &report.closures.first().unwrap().invocation;
    assert!(!invocation.effect_exits.is_empty(), "{report}");
    assert_eq!(
        invocation.effect_capture_pointer_exits.len(),
        invocation.effect_exits.len()
    );
    assert_eq!(
        invocation.captured_pointer_exits.len(),
        invocation.exits.len()
    );
    assert_eq!(
        invocation.failed_capture_pointer_exits.len(),
        invocation.failed_exits.len()
    );
    for snapshot in &invocation.effect_capture_pointer_exits {
        assert_eq!(snapshot.len(), 1, "{report}");
        let (slot, targets) = snapshot.first_key_value().unwrap();
        assert_eq!(slot.fields, vec![0]);
        assert_eq!(targets.len(), 1);
    }
    assert!(
        report
            .to_string()
            .contains("(effect-capture-pointer-exits (captures (slot")
    );
}

#[test]
fn scalar_capture_updates_preserve_pointer_fields_across_invocations() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Cursor {target:*bool,flipped:bool}
        fn twice(p:*bool)->bool {
            var cursor=Cursor {target:p,flipped:false};
            let action=|| {cursor.flipped=not cursor.flipped;cursor.target.* = cursor.flipped;cursor.flipped};
            let first=action();let second=action();first and not second
        }";
    let package = compile(&format!(
        "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};twice(cell.flag.ref) and not cell.flag}}"
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000)
            .unwrap(),
        Value::Bool(true)
    );
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "twice")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        function.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.write && access.target.parameter == 0),
        "{summary}"
    );
    let expired = compile(&format!(
        "{prefix} fn run()->bool {{
        let cell=Cell {{flag:false}};
        var cursor=Cursor {{target:cell.flag.ref,flipped:false}};
        let action=|| {{cursor.flipped=not cursor.flipped;cursor.target.* = cursor.flipped;}};
        action();let moved=cell;action();true
    }}"
    ));
    assert!(
        luna::execution::interpret_with_protocols(&expired, "run", "Duplicate", "Retire", 300_000)
            .unwrap_err()
            .contains("cannot prepare resource-checked execution")
    );
}

#[test]
fn data_capture_fields_can_be_replaced_without_losing_neighboring_pointers() {
    for (state_ty, initial, replacement, read) in [
        (
            "State",
            "State {flag:false}",
            "State {flag:not cursor.state.flag}",
            "cursor.state.flag",
        ),
        (
            "Wrapped",
            "false.as(Wrapped)",
            "(not cursor.state.as(bool)).as(Wrapped)",
            "cursor.state.as(bool)",
        ),
        (
            "Choice",
            "Choice.off",
            "if cursor.state match {.off=>true,.on=>false} {Choice.on} else {Choice.off}",
            "cursor.state match {.off=>false,.on=>true}",
        ),
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct State {{flag:bool}}impl Retire for State {{fn drop(*self) {{}}}}
            newtype Wrapped=bool;enum Choice {{off,on}}
            impl Duplicate for Wrapped {{}}impl Duplicate for Choice {{}}
            struct Cursor {{target:*bool,state:{state_ty}}}
            fn twice(p:*bool)->bool {{
                var cursor=Cursor {{target:p,state:{initial}}};
                let action=|| {{cursor.state={replacement};cursor.target.* = {read};{read}}};
                let first=action();let second=action();first and not second
            }}
            fn run()->bool {{let cell=false;twice(cell.ref) and not cell}}
        "
        ));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300_000,
        );
        assert_eq!(
            result.unwrap_or_else(|error| panic!("{state_ty}: {error}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn aggregate_capture_replacement_updates_origins_and_retires_previous_values() {
    for (setup, replace, read) in [
        (
            "var current=Link {target:p};",
            "current=Link {target:q};",
            "current.target.*",
        ),
        (
            "var current=(Link {target:p}).as(Wrapped);",
            "current=(Link {target:q}).as(Wrapped);",
            "current.as(Link).target.*",
        ),
        (
            "var current=Outer {inner:Link {target:p}};",
            "current.inner=Link {target:q};",
            "current.inner.target.*",
        ),
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Link {{target:*bool}}impl Duplicate for Link {{}}
            newtype Wrapped=Link;impl Duplicate for Wrapped {{}}
            struct Outer {{inner:Link}}
            fn switched(p:*bool,q:*bool)->bool {{
                {setup}let action=|| {{let seen={read};{replace}seen}};
                let first=action();let second=action();not first and second
            }}
            fn run()->bool {{let first=false;let second=true;switched(first.ref,second.ref)}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap_or_else(|error| panic!("{setup}: {error}")),
            Value::Bool(true)
        );
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "switched")
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            &mut 100_000,
        )
        .unwrap();
        let roots = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(roots, [0, 1].into_iter().collect(), "{setup}: {summary}");
    }
    // Replacing an owning aggregate retires the old value immediately. The
    // retained replacement is then retired once when its closure is dropped.
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}impl Retire for Link {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        fn run()->bool {let first=false;let second=false;let p=first.ref;let q=second.ref;
            {var current=Link {target:p};let action=|| {current=Link {target:q};};action();}
            first and second
        }");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn aggregate_capture_replacement_does_not_revive_expired_targets() {
    for suffix in ["let moved=second;action()", "let moved=first;action()"] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            struct Link {{target:*bool}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let p=first.flag.ref;let q=second.flag.ref;var current=Link {{target:p}};
                let action=|| {{let seen=current.target.*;current=Link {{target:q}};seen}};
                action();{suffix}
            }}"
        ));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300_000,
        );
        if suffix.contains("moved=second") {
            let error = result.unwrap_err();
            assert!(
                error.contains("cannot prepare resource-checked execution"),
                "{error}"
            );
        } else {
            assert_eq!(result.unwrap(), Value::Bool(true));
        }
    }
}

#[test]
fn mixed_capture_code_replacement_tracks_later_memory_accesses() {
    for replacement in [
        "cursor.action=write;",
        "cursor=Cursor {target:p,action:write};",
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cursor {{target:*bool,action:fn(*bool)->bool}}
            fn read(p:*bool)->bool {{p.*}}
            fn write(p:*bool)->bool {{p.* = true;true}}
            fn switched(p:*bool)->bool {{var cursor=Cursor {{target:p,action:read}};
                let action=|| {{let seen=cursor.action(cursor.target);{replacement}seen}};
                let first=action();let second=action();not first and second
            }}
            fn run()->bool {{let cell=false;switched(cell.ref) and cell}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap_or_else(|error| panic!("{replacement}: {error}")),
            Value::Bool(true)
        );
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "switched")
            .unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 100_000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.read_value),
            "{summary}"
        );
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.write),
            "{summary}"
        );
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
    }
    let package=compile("struct Cursor {target:*bool,action:fn(*bool)->bool}
        fn read(p:*bool)->bool {p.*}
        fn inspect(p:*bool,next:fn(*bool)->bool)->bool {var cursor=Cursor {target:p,action:read};
            let action=|replacement:fn(*bool)->bool| {let seen=cursor.action(cursor.target);cursor.action=replacement;seen};
            action(next);action(next)
        }");
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "inspect")
        .unwrap();
    let error = elaboration::verification::memory::infer_in_package(
        &package,
        function.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap_err();
    assert!(
        error.contains("closure capture code exit has unknown environment identity"),
        "{error}"
    );
}

#[test]
fn argument_callback_replacement_advances_only_the_called_environment() {
    let source="trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Action=fn(*bool)->bool;impl Duplicate for Action {}
        struct Cursor {target:*bool,action:Action}
        fn read(p:*bool)->bool {p.*}
        fn write(p:*bool)->bool {p.* = true;true}
        fn switched(p:*bool)->bool {var cursor=Cursor {target:p,action:read};
            let action=|next:Action| {let seen=cursor.action(cursor.target);cursor.action=next;seen};
            let first=action(write);let second=action(read);let third=action(read);
            not first and second and third
        }
        fn run()->bool {let cell=false;switched(cell.ref) and cell}";
    let package = compile(source);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000)
            .unwrap(),
        Value::Bool(true)
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "switched")
        .unwrap();
    let infer = |fuel: &mut usize| {
        elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            fuel,
        )
    };
    let summary = infer(&mut 100_000).unwrap();
    assert!(
        summary
            .accesses
            .iter()
            .any(|access| access.target.parameter == 0 && access.write),
        "{summary}"
    );
    assert!(infer(&mut 1).is_err());
    assert_eq!(infer(&mut 100_000).unwrap(), summary);
    // Merely installing a writer must not attribute its future accesses to the
    // call that installed it. Only the currently stored callback executes.
    let package=compile(&format!("{source} fn install(p:*bool)->bool {{var cursor=Cursor {{target:p,action:read}};
        let action=|next:Action| {{let seen=cursor.action(cursor.target);cursor.action=next;seen}};action(write)}}"));
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "install")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        function.declaration,
        &[],
        &mut 100_000,
    )
    .unwrap();
    assert!(
        summary.accesses.iter().all(|access| !access.write),
        "{summary}"
    );
}

#[test]
fn argument_callback_transitions_converge_and_keep_copied_states_separate() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Action=fn(*bool)->bool;impl Duplicate for Action {}
        struct Cursor {target:*bool,action:Action}
        fn read(p:*bool)->bool {p.*}fn write(p:*bool)->bool {p.* = true;true}
        fn looped(p:*bool)->bool {var cursor=Cursor {target:p,action:read};
            let action=|next:Action| {let seen=cursor.action(cursor.target);cursor.action=next;seen};
            var again=true;var result=false;
            while true {result=action(write);if not again {break;}again=false;}result
        }
        fn isolated(p:*bool)->bool {var cursor=Cursor {target:p,action:read};
            let action=|next:Action| {let seen=cursor.action(cursor.target);cursor.action=next;seen};
            let other=action;action(write);other(read)
        }
        fn run()->bool {let cell=false;looped(cell.ref) and cell}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000)
            .unwrap(),
        Value::Bool(true)
    );
    for (name, writes) in [("looped", true), ("isolated", false)] {
        // The isolated case tests inference only. Copy authorization for the
        // closure itself remains a separate resource obligation.
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 100_000).unwrap();
        assert_eq!(
            summary.accesses.iter().any(|access| access.write),
            writes,
            "{name}: {summary}"
        );
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
    }
}

#[test]
fn ordinary_callback_captures_accept_per_invocation_replacement() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Action=fn(*bool)->bool;impl Duplicate for Action {}
        typealias FlagAction=fn()->bool;impl Duplicate for FlagAction {}
        fn off()->bool {false}fn on()->bool {true}
        fn read(p:*bool)->bool {p.*}fn write(p:*bool)->bool {p.* = true;true}
        fn plain()->bool {var current=off;let action=|next:FlagAction| {let seen=current();current=next;seen};
            let first=action(on);let second=action(off);not first and second
        }
        fn mixed(p:*bool)->bool {var current=read;let action=|next:Action| {let seen=current(p);current=next;seen};
            let first=action(write);let second=action(read);not first and second
        }
        fn install(p:*bool)->bool {var current=read;let action=|next:Action| {let seen=current(p);current=next;seen};action(write)}
        fn run()->bool {let cell=false;plain() and mixed(cell.ref) and cell}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000)
            .unwrap(),
        Value::Bool(true)
    );
    for (name, writes) in [("mixed", true), ("install", false), ("plain", false)] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 100_000).unwrap();
        assert_eq!(
            summary.accesses.iter().any(|access| access.write),
            writes,
            "{name}: {summary}"
        );
        assert!(
            summary
                .accesses
                .iter()
                .all(|access| access.target.parameter == 0),
            "{name}: {summary}"
        );
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
    }
}

#[test]
fn returned_nested_enum_callbacks_preserve_absent_branch_constraints() {
    for replacement in ["Outer.direct(second)", "Outer.inside(Inner.right(second))"] {
        for warm in [false, true] {
            let warmup = if warm { "callback(a,b);" } else { "" };
            let source=format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                typealias Action=fn(*bool,*bool)->bool;impl Duplicate for Action {{}}
                enum Inner {{left(Action),right(Action)}}impl Duplicate for Inner {{}}
                enum Outer {{inside(Inner),direct(Action)}}impl Duplicate for Outer {{}}
                typealias Stateful=%{{ops:Outer}} fn(*bool,*bool)->bool;
                fn first(a:*bool,b:*bool)->bool {{a.*}}fn second(a:*bool,b:*bool)->bool {{b.*}}
                fn factory(a:*bool,b:*bool)->Stateful {{var ops=Outer.inside(Inner.left(first));
                    let callback=|a:*bool,b:*bool| {{let seen=ops match {{
                        .inside(inner)=>inner match {{.left(action)=>action(a,b),.right(action)=>action(a,b)}},
                        .direct(action)=>action(a,b)
                    }};ops={replacement};seen}};{warmup}callback
                }}
                fn run()->bool {{let a=true;let b=false;let callback=factory(a.ref,b.ref);callback(a.ref,b.ref)}}");
            let package = compile(&source);
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1200000,
            )
            .unwrap_or_else(|error| panic!("{replacement},warm={warm}: {error}"));
            assert_eq!(result, Value::Bool(!warm), "{replacement},warm={warm}");
            let function = package
                .functions
                .iter()
                .find(|f| f.name == "factory")
                .unwrap();
            let infer = |fuel: &mut usize| {
                elaboration::verification::memory::infer_in_package(
                    &package,
                    function.declaration,
                    &[],
                    fuel,
                )
            };
            let summary = infer(&mut 300000).unwrap();
            assert!(summary.to_string().contains("absent-variant"), "{summary}");
            assert!(infer(&mut 1).is_err());
            assert_eq!(infer(&mut 300000).unwrap(), summary);
            if warm {
                let expired=source.replace(
                    "fn run()->bool {let a=true;let b=false;let callback=factory(a.ref,b.ref);callback(a.ref,b.ref)}",
                    "struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}} fn run()->bool {let a=true;let b=Cell {flag:false};let p=b.flag.ref;let callback=factory(a.ref,p);let moved=b;callback(a.ref,p)}",
                );
                assert_ne!(expired, source);
                let package = compile(&expired);
                let error = luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    1200000,
                )
                .unwrap_err();
                assert!(error.contains("unavailable"), "{replacement}: {error}");
            }
        }
    }
}

#[test]
fn enum_callback_captures_accept_argument_supplied_transitions() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Action=fn(*bool,*bool)->bool;impl Duplicate for Action {}
        enum Ops {left(Action),right(Action)}impl Duplicate for Ops {}
        fn first(a:*bool,b:*bool)->bool {a.*}fn second(a:*bool,b:*bool)->bool {b.*}
        fn inspect(a:*bool,b:*bool)->bool {var ops=Ops.left(first);
            let step=|next:Action| {let seen=ops match {.left(action)=>action(a,b),.right(action)=>action(a,b)};ops=Ops.right(next);seen};
            let left=step(second);let right=step(first);left and not right
        }
        fn run()->bool {let a=true;let b=false;inspect(a.ref,b.ref)}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let summary = elaboration::verification::memory::infer_in_package(
        &package,
        function.declaration,
        &[],
        &mut 300000,
    )
    .unwrap();
    let reads = summary
        .accesses
        .iter()
        .filter(|access| access.read_value)
        .map(|access| access.target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(reads, [0, 1].into_iter().collect(), "{summary}");
}

#[test]
fn returned_enum_state_joins_preserve_both_possible_accesses() {
    for select in [
        "factory(flag,a,b)",
        "if flag {factory(true,a,b)} else {factory(false,a,b)}",
    ] {
        let prefix=format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias Action=fn(*bool,*bool)->bool;impl Duplicate for Action {{}}
            enum Ops {{left(Action),right(Action)}}impl Duplicate for Ops {{}}
            typealias Stateful=%{{ops:Ops}} fn(*bool,*bool)->bool;
            fn first(a:*bool,b:*bool)->bool {{a.*}}fn second(a:*bool,b:*bool)->bool {{b.*}}
            fn factory(flag:bool,a:*bool,b:*bool)->Stateful {{var ops=Ops.left(first);
                let callback=|a:*bool,b:*bool| {{let seen=ops match {{.left(action)=>action(a,b),.right(action)=>action(a,b)}};ops=Ops.right(second);seen}};
                if flag {{callback(a,b);return callback;}}callback
            }}
            fn inspect(a:*bool,b:*bool,flag:bool)->bool {{let callback={select};callback(a,b)}}");
        let package = compile(&format!(
            "{prefix}
            fn cold()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,false)}}
            fn warm()->bool {{let a=true;let b=false;inspect(a.ref,b.ref,true)}}"
        ));
        for (entry, expected) in [("cold", true), ("warm", false)] {
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    entry,
                    "Duplicate",
                    "Retire",
                    1200000
                )
                .unwrap_or_else(|error| panic!("{select},{entry}: {error}")),
                Value::Bool(expected)
            );
        }
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "inspect")
            .unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 500000).unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, [0, 1].into_iter().collect(), "{select}: {summary}");
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 500000).unwrap(), summary);
        let invalid = compile(&format!(
            "{prefix}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            fn run()->bool {{let a=true;let b=Cell {{flag:false}};let p=b.flag.ref;
                let callback=factory(true,a.ref,p);let moved=b;callback(a.ref,p)}}"
        ));
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            1200000,
        )
        .unwrap_err();
        assert!(error.contains("unavailable"), "{select}: {error}");
    }
}

#[test]
fn enum_payload_cleanup_uses_selected_pointer_layouts() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}impl Retire for Link {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        enum Payload {left(Link),right(bool,Link)}
        enum Outer {empty,nested(bool,Payload)}";
    for (body, expected) in [
        ("let payload=Payload.left(Link {target:cell.ref});", true),
        (
            "let payload=Payload.right(false,Link {target:cell.ref});",
            true,
        ),
        (
            "let payload=Outer.nested(false,Payload.right(false,Link {target:cell.ref}));",
            true,
        ),
        (
            "var payload=Payload.left(Link {target:cell.ref});payload=Payload.right(false,Link {target:cell.ref});",
            false,
        ),
        (
            "var payload=Payload.right(false,Link {target:cell.ref});payload=Payload.left(Link {target:cell.ref});",
            false,
        ),
        (
            "var payload=Outer.nested(false,Payload.right(false,Link {target:cell.ref}));payload=Outer.empty;",
            true,
        ),
    ] {
        let package = compile(&format!(
            "{prefix} fn run()->bool {{let cell=false;{{{body}}}cell}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300000
            )
            .unwrap_or_else(|error| panic!("{body}: {error}")),
            Value::Bool(expected)
        );
    }
}

#[test]
fn enum_payload_replacement_preserves_old_and_new_pointer_obligations() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Link {target:*Cell}impl Retire for Link {fn drop(*self) {let p=self.target.*;p.flag.* = true;}}
        enum Payload {left(Link),right(bool,Link)}";
    for invalidation in ["", "let moved=first;", "let moved=second;"] {
        let result = if invalidation.is_empty() {
            "first.flag and second.flag"
        } else {
            "true"
        };
        let package=compile(&format!("{prefix} fn run()->bool {{
            let first=Cell {{flag:false}};let second=Cell {{flag:false}};
            let a=first.ref;let b=second.ref;{invalidation}
            {{var payload=Payload.left(Link {{target:a}});payload=Payload.right(false,Link {{target:b}});}}
            {result}
        }}"));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300000,
        );
        if invalidation.is_empty() {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            let error = actual.unwrap_err();
            assert!(
                error.contains("cannot prepare resource-checked execution"),
                "{invalidation}: {error}"
            );
        }
    }
}

#[test]
fn captured_callback_sources_join_reachable_identities() {
    for replacement in [
        "cursor.action=cursor.next;",
        "cursor.action=if flag {cursor.next} else {cursor.action};",
    ] {
        let source=format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias Action=fn(*bool)->bool;impl Duplicate for Action {{}}
            struct Cursor {{target:*bool,action:Action,next:Action}}
            fn read(p:*bool)->bool {{p.*}}
            fn write(p:*bool)->bool {{p.* = true;true}}
            fn switched(p:*bool,flag:bool)->bool {{var cursor=Cursor {{target:p,action:read,next:write}};
                let action=|| {{let seen=cursor.action(cursor.target);{replacement}seen}};
                let first=action();let second=action();not first and second
            }}
            fn run()->bool {{let cell=false;switched(cell.ref,true) and cell}}");
        let package = compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000
            )
            .unwrap_or_else(|error| panic!("{replacement}: {error}")),
            Value::Bool(true)
        );
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "switched")
            .unwrap();
        let infer = |fuel: &mut usize| {
            elaboration::verification::memory::infer_in_package(
                &package,
                function.declaration,
                &[],
                fuel,
            )
        };
        let summary = infer(&mut 100_000).unwrap();
        assert!(
            summary
                .accesses
                .iter()
                .any(|access| access.target.parameter == 0 && access.write),
            "{summary}"
        );
        assert!(infer(&mut 1).is_err());
        assert_eq!(infer(&mut 100_000).unwrap(), summary);
        let affine = compile(&source.replace("impl Duplicate for Action {}", ""));
        let error = luna::execution::interpret_with_protocols(
            &affine,
            "run",
            "Duplicate",
            "Retire",
            300_000,
        )
        .unwrap_err();
        assert!(
            error.contains("use of unavailable value"),
            "{replacement}: {error}"
        );
    }
}

#[test]
fn capture_replacements_track_enum_fields_without_reading_installed_targets() {
    for (state_ty, initial, replacement) in [("Hidden", "Hidden.none", "Hidden.some(q)")] {
        let package = compile(&format!(
            "struct Link {{target:*bool}}newtype Wrapped=Link;
            enum Hidden {{none,some(*bool)}}struct Operations {{action:fn()->bool}}
            fn first()->bool {{true}}fn second()->bool {{false}}
            struct Cursor {{target:*bool,state:{state_ty}}}
            fn inspect(p:*bool,q:*bool)->bool {{
                var cursor=Cursor {{target:p,state:{initial}}};
                let action=|| {{cursor.state={replacement};cursor.target.*}};action()
            }}"
        ));
        let function = package
            .functions
            .iter()
            .find(|function| function.name == "inspect")
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            &mut 100_000,
        )
        .unwrap();
        let reads = summary
            .accesses
            .iter()
            .filter(|access| access.read_value)
            .map(|access| access.target.parameter)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(reads, std::collections::BTreeSet::from([0]), "{summary}");
    }
}

#[test]
fn alternating_capture_state_keeps_all_possible_result_origins() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cursor {left:*bool,right:*bool,flipped:bool}
        fn select(p:*bool,q:*bool)->*bool {
            var cursor=Cursor {left:p,right:q,flipped:false};
            let action=|| {cursor.flipped=not cursor.flipped;if cursor.flipped {cursor.left} else {cursor.right}};
            action();action()
        }
        fn run()->bool {let first=true;let second=false;not select(first.ref,second.ref).*}");
    let function = package
        .functions
        .iter()
        .find(|function| function.name == "select")
        .unwrap();
    let infer = |fuel: &mut usize| {
        elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            fuel,
        )
    };
    let summary = infer(&mut 100_000).unwrap();
    let roots = summary
        .result_targets
        .as_ref()
        .unwrap()
        .iter()
        .map(|target| target.parameter)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(roots, [0, 1].into_iter().collect(), "{summary}");
    assert!(infer(&mut 1).is_err());
    assert_eq!(infer(&mut 100_000).unwrap(), summary);
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 300_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn moving_a_pointer_bearing_capture_does_not_restore_it_on_the_next_call() {
    for twice in [false, true] {
        let repeat = if twice { "action();" } else { "" };
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Holder {{target:*bool}}impl Retire for Holder {{fn drop(*self) {{}}}}
            fn run()->bool {{let cell=false;let holder=Holder {{target:cell.ref}};
                let action=|| {{let consumed=holder;consumed.target.* = not consumed.target.*;}};
                action();{repeat}cell
            }}"
        ));
        let result = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300_000,
        );
        if twice {
            assert!(
                result
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        } else {
            assert_eq!(result.unwrap(), Value::Bool(true));
        }
    }
}

#[test]
fn conditional_capture_retirement_keeps_guarded_cleanup_origins() {
    let prefix="trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Holder {target:*Cell}impl Retire for Holder {fn drop(*self) {let p=self.target.*;p.flag.* = not p.flag.*;}}
        fn conditional(p:*Cell,consume:bool) {let holder=Holder {target:p};let action=|| {BODY};action();}";
    for body in [
        "if consume {let retired=holder;}",
        "if consume {let retired=holder;return;}",
    ] {
        let prefix = prefix.replace("BODY", body);
        for consume in [false, true] {
            let package = compile(&format!(
                "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};conditional(cell.ref,{consume});cell.flag}}"
            ));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    300_000
                )
                .unwrap(),
                Value::Bool(true)
            );
            let expired = compile(&format!(
                "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};let pointer=cell.ref;let moved=cell;conditional(pointer,{consume});true}}"
            ));
            assert!(
                luna::execution::interpret_with_protocols(
                    &expired,
                    "run",
                    "Duplicate",
                    "Retire",
                    300_000
                )
                .unwrap_err()
                .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn captured_aggregate_cleanup_keeps_the_original_storage_obligation() {
    let prefix = "trait Duplicate {}trait Retire {fn drop(*self);}
        struct Cell {flag:bool}impl Retire for Cell {fn drop(*self) {}}
        struct Link {target:*Cell}
        newtype Wrapped=Link;
        struct Holder {target:*Cell}
        impl Retire for Holder {fn drop(*self) {let p=self.target.*;p.flag.* = true;}}
        fn stored(p:*Cell) {let link=Link {target:p};let action=|| {let holder=Holder {target:link.target};};action();}
        fn immediate(p:*Cell) {let link=Link {target:p};(|| {let holder=Holder {target:link.target};})();}
        fn moved(p:*Cell) {let holder=Holder {target:p};let action=|| {let retired=holder;};action();}
        fn wrapped(p:*Cell) {let link=(Link {target:p}).as(Wrapped);let action=|| {let holder=Holder {target:link.as(Link).target};};action();}";
    for helper in ["stored", "immediate", "moved", "wrapped"] {
        for expired in [false, true] {
            let invalidation = if expired {
                "let retired_cell=cell;"
            } else {
                ""
            };
            let result = if expired { "true" } else { "cell.flag" };
            let package = compile(&format!(
                "{prefix} fn run()->bool {{let cell=Cell {{flag:false}};let pointer=cell.ref;{invalidation}{helper}(pointer);{result}}}"
            ));
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                300_000,
            );
            if expired {
                let error = result.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "{helper}: {error}"
                );
            } else {
                assert_eq!(
                    result.unwrap_or_else(|error| panic!("{helper}: {error}")),
                    Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn enum_payload_flow_join_keeps_absent_pointer_fields_distinct_from_unknown() {
    for (initial, replacement) in [
        (
            "Payload.left(Link {target:cell.ref})",
            "Payload.right(false,Link {target:cell.ref})",
        ),
        (
            "Payload.right(false,Link {target:cell.ref})",
            "Payload.left(Link {target:cell.ref})",
        ),
        (
            "Outer.nested(Payload.left(Link {target:cell.ref}))",
            "Outer.nested(Payload.right(false,Link {target:cell.ref}))",
        ),
    ] {
        for flag in [false, true] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Link {{target:*bool}}impl Retire for Link {{fn drop(*self) {{self.target.*.* = true;}}}}
            enum Payload {{left(Link),right(bool,Link)}}enum Outer {{nested(Payload)}}
            fn inspect(flag:bool)->bool {{let cell=false;{{
                var payload={initial};
                if flag {{payload={replacement};}}
                cell.ref.* = false;
            }}cell}}
            fn run()->bool {{inspect({flag})}}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    300000
                )
                .unwrap_or_else(|error| panic!("{initial}, {flag}: {error}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn enum_payload_flow_join_does_not_hide_expired_targets() {
    for flag in [false, true] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            struct Link {{target:*Cell}}impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = true;}}}}
            enum Payload {{left(Link),right(bool,Link)}}
            fn inspect(flag:bool) {{let cell=Cell {{flag:false}};
                var payload=Payload.left(Link {{target:cell.ref}});
                if flag {{payload=Payload.right(false,Link {{target:cell.ref}});}}
                let moved=cell;
            }}
            fn run() {{inspect({flag})}}"));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            300000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{flag}: {error}"
        );
    }
}

#[test]
fn wrapped_enum_payload_cleanup_survives_control_flow_joins() {
    for declaration in [
        "newtype Wrapped=Payload;",
        "newtype Wrapper<T> = T;typealias Wrapped=Wrapper<Payload>;",
    ] {
        for control in [
            "if flag {payload=(Payload.right(false,Link {target:cell.ref})).as(Wrapped);}",
            "var again=flag;while again {payload=(Payload.right(false,Link {target:cell.ref})).as(Wrapped);again=false;}",
        ] {
            for flag in [false, true] {
                let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Link {{target:*bool}}impl Retire for Link {{fn drop(*self) {{self.target.*.* = true;}}}}
                enum Payload {{left(Link),right(bool,Link)}}{declaration}
                fn inspect(flag:bool)->bool {{let cell=false;{{
                    var payload=(Payload.left(Link {{target:cell.ref}})).as(Wrapped);
                    {control}
                    cell.ref.* = false;
                }}cell}}
                fn run()->bool {{inspect({flag})}}"));
                assert_eq!(
                    luna::execution::interpret_with_protocols(
                        &package,
                        "run",
                        "Duplicate",
                        "Retire",
                        500000
                    )
                    .unwrap_or_else(|error| panic!("{control}, {flag}: {error}")),
                    Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn enum_cleanup_omits_unreachable_payload_variants_after_join() {
    for variants in [
        "left(Link),right(bool,Link),absent(bool,bool,Link)",
        "left(Link),absent(bool,bool,Link),right(bool,Link)",
        "absent(bool,bool,Link),left(Link),right(bool,Link)",
    ] {
        for nested in [false, true] {
            for flag in [false, true] {
                let (initial, replacement) = if nested {
                    (
                        "Outer.nested(Payload.left(Link {target:cell.ref}))",
                        "Outer.nested(Payload.right(false,Link {target:cell.ref}))",
                    )
                } else {
                    (
                        "Payload.left(Link {target:cell.ref})",
                        "Payload.right(false,Link {target:cell.ref})",
                    )
                };
                let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Link {{target:*bool}}impl Retire for Link {{fn drop(*self) {{self.target.*.* = true;}}}}
            enum Payload {{{variants}}}enum Outer {{nested(Payload)}}
            fn inspect(flag:bool)->bool {{let cell=false;{{
                var payload={initial};
                if flag {{payload={replacement};}}
                cell.ref.* = false;
            }}cell}}
            fn run()->bool {{inspect({flag})}}"));
                assert_eq!(
                    luna::execution::interpret_with_protocols(
                        &package,
                        "run",
                        "Duplicate",
                        "Retire",
                        500000
                    )
                    .unwrap_or_else(|error| panic!("{variants}, {nested}, {flag}: {error}")),
                    Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn nested_enum_cleanup_uses_domains_conditional_on_dynamic_outer_tags() {
    for flag in [false, true] {
        for alternate in [false, true] {
            for expired in [false, true] {
                let invalidate = if expired { "let moved=cell;" } else { "" };
                let result = if expired { "true" } else { "cell.flag" };
                let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            struct Link {{target:*Cell}}impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = true;}}}}
            enum Inner {{first(Link),absent(bool,bool,Link),last(bool,Link)}}
            enum Outer {{left(Inner),right(bool,Inner)}}
            fn inspect(flag:bool,alternate:bool)->bool {{let cell=Cell {{flag:false}};{{
                var payload=Outer.left(Inner.first(Link {{target:cell.ref}}));
                if flag {{payload=Outer.right(false,Inner.last(false,Link {{target:cell.ref}}));}}
                if alternate {{payload=Outer.left(Inner.last(false,Link {{target:cell.ref}}));}}
                cell.flag.ref.* = false;
                {invalidate}
            }}{result}}}
            fn run()->bool {{inspect({flag},{alternate})}}"));
                let actual = luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    500000,
                );
                if expired {
                    let error = actual.unwrap_err();
                    assert!(
                        error.contains("cannot prepare resource-checked execution"),
                        "{error}"
                    );
                } else {
                    assert_eq!(
                        actual.unwrap_or_else(|error| panic!("{flag}, {alternate}: {error}")),
                        Value::Bool(true)
                    );
                }
            }
        }
    }
}

#[test]
fn pointer_input_layout_keeps_variant_guards_and_newtype_storage_separate() {
    use elaboration::verification::memory::{input_pointer_fields, pointer_input_layout};
    let package = compile(
        "newtype Pointer=*bool;enum Inner {first(Pointer),last(bool,Pointer)}
        enum Outer {left(Inner),right(bool,Inner)}newtype Wrap<T> = T;
        fn specimen(value:Wrap<Outer>) {}fn plain(value:Pointer) {}",
    );
    let signature = &package
        .functions
        .iter()
        .find(|f| f.name == "specimen")
        .unwrap()
        .signature;
    let nir::Term::Function { parameters, .. } = signature else {
        panic!("function signature")
    };
    let ty = &parameters[0].ty;
    assert!(pointer_input_layout(Some(&package), ty, &mut 1).is_err());
    let layout = pointer_input_layout(Some(&package), ty, &mut 100000).unwrap();
    assert_eq!(layout.len(), 4);
    for (index, leaf) in layout.iter().enumerate() {
        let outer = index / 2;
        let inner = index % 2;
        assert_eq!(
            leaf.input_fields,
            vec![0, usize::MAX, outer, outer, usize::MAX, inner, inner]
        );
        assert_eq!(leaf.storage_fields, vec![0, outer, inner, 0]);
        assert_eq!(leaf.guards, vec![(vec![0], outer), (vec![0, outer], inner)]);
        assert_eq!(leaf.pointee, nir::Term::Primitive(nir::Primitive::Bool));
        let dump = leaf.to_string();
        assert!(dump.contains(&format!("(variant {outer})")), "{dump}");
        assert!(
            dump.contains(&format!("(storage-fields 0 {outer} {inner} 0)")),
            "{dump}"
        );
        assert!(dump.contains("(guards (guard (path 0)"), "{dump}");
        assert_eq!(
            dump.chars().filter(|c| *c == '(').count(),
            dump.chars().filter(|c| *c == ')').count()
        );
    }
    assert!(
        input_pointer_fields(Some(&package), ty, &mut 100000)
            .unwrap_err()
            .contains("richer memory contract")
    );
    let signature = &package
        .functions
        .iter()
        .find(|f| f.name == "plain")
        .unwrap()
        .signature;
    let nir::Term::Function { parameters, .. } = signature else {
        panic!("function signature")
    };
    let plain = &parameters[0].ty;
    let layout = pointer_input_layout(Some(&package), plain, &mut 10000).unwrap();
    assert_eq!(layout.len(), 1);
    assert!(layout[0].input_fields.is_empty());
    assert!(layout[0].guards.is_empty());
    assert_eq!(layout[0].storage_fields, vec![0]);
    assert_eq!(
        input_pointer_fields(Some(&package), plain, &mut 10000)
            .unwrap()
            .get(&vec![]),
        Some(&nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn enum_pointer_arguments_preserve_selected_payload_origins() {
    for constructor in [
        "Choice.empty",
        "Choice.left(cell.ref)",
        "Choice.right(false,cell.ref)",
    ] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool),empty}}
            fn read(value:Choice)->bool {{value match {{.left(p)=>p.*,.right(_,p)=>p.*,.empty=>false}}}}
            fn run()->bool {{let cell=true;read({constructor})}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|error| panic!("{constructor}: {error}")),
            Value::Bool(constructor != "Choice.empty")
        );
    }
}

#[test]
fn enum_pointer_calls_forward_and_reject_expired_targets() {
    for flag in [false, true] {
        for expired in [false, true] {
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            enum Choice {{left(*Cell),right(bool,*Cell),empty}}
            fn read(value:Choice)->bool {{value match {{.left(p)=>p.flag.*,.right(_,p)=>p.flag.*,.empty=>false}}}}
            fn forward(value:Choice)->bool {{read(value)}}
            fn invoke(f:fn(Choice)->bool,value:Choice)->bool {{f(value)}}
            fn inspect(flag:bool)->bool {{let cell=Cell {{flag:true}};let p=cell.ref;{invalidate}
                let value=if flag {{Choice.left(p)}} else {{Choice.right(false,p)}};
                invoke(forward,value)
            }}
            fn run()->bool {{inspect({flag})}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if expired {
                let error = actual.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "{error}"
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{flag}: {e}")),
                    Value::Bool(true)
                );
            }
            let read = package.functions.iter().find(|f| f.name == "read").unwrap();
            let summary = elaboration::verification::memory::infer_in_package(
                &package,
                read.declaration,
                &[],
                &mut 100000,
            )
            .unwrap();
            let inputs = summary
                .accesses
                .iter()
                .map(|a| a.target.input_fields.clone())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                inputs,
                std::collections::BTreeSet::from([vec![usize::MAX, 0, 0], vec![usize::MAX, 1, 1]]),
                "{summary}"
            );
        }
    }
}

#[test]
fn enum_pointer_results_preserve_both_payload_layouts() {
    for flag in [false, true] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool)}}
            fn make(p:*bool,flag:bool)->Choice {{if flag {{Choice.left(p)}} else {{Choice.right(false,p)}}}}
            fn run()->bool {{let cell=true;let value=make(cell.ref,{flag});value match {{.left(p)=>p.*,.right(_,p)=>p.*}}}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{flag}: {e}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn enum_pointer_result_collisions_do_not_erase_target_obligations() {
    for flag in [false, true] {
        for invalidation in ["", "let moved=first;", "let moved=second;"] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            enum Choice {{left(*Cell),right(*Cell),empty}}
            fn make(a:*Cell,b:*Cell,flag:bool)->Choice {{if flag {{Choice.left(a)}} else {{Choice.right(b)}}}}
            fn forward(a:*Cell,b:*Cell,flag:bool)->Choice {{make(a,b,flag)}}
            fn inspect(flag:bool)->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                let a=first.ref;let b=second.ref;{invalidation}
                let value=forward(a,b,flag);value match {{.left(p)=>p.flag.*,.right(p)=>p.flag.*,.empty=>false}}
            }}
            fn run()->bool {{inspect({flag})}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if invalidation.is_empty() {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{flag}: {e}")),
                    Value::Bool(flag)
                );
            } else {
                let error = actual.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "{invalidation}: {error}"
                );
            }
        }
    }
}

#[test]
fn enum_pointer_results_reject_local_address_escape() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        enum Choice {none,some(*bool)}
        fn escape()->Choice {let cell=true;Choice.some(cell.ref)}
        fn run()->bool {escape() match {.none=>false,.some(p)=>p.*}}",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn enum_pointer_results_cross_closures_defaults_and_wrappers() {
    for body in [
        "let factory=|p:*bool| make(p,flag);read(factory(cell.ref))",
        "read_default(cell.ref,flag)",
        "read(wrap(cell.ref,flag).as(Choice))",
    ] {
        for flag in [false, true] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool)}}newtype Wrapped=Choice;
            fn make(p:*bool,flag:bool)->Choice {{if flag {{Choice.left(p)}} else {{Choice.right(false,p)}}}}
            fn wrap(p:*bool,flag:bool)->Wrapped {{make(p,flag).as(Wrapped)}}
            fn read(value:Choice)->bool {{value match {{.left(p)=>p.*,.right(_,p)=>p.*}}}}
            fn read_default(p:*bool,flag:bool,.value:Choice=make(p,flag))->bool {{read(value)}}
            fn inspect(flag:bool)->bool {{let cell=true;{body}}}
            fn run()->bool {{inspect({flag})}}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    500000
                )
                .unwrap_or_else(|e| panic!("{body}, {flag}: {e}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn stored_enum_pointer_captures_keep_selected_origins() {
    for constructor in [
        "Choice.none",
        "Choice.left(cell.ref)",
        "Choice.right(false,cell.ref)",
    ] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{none,left(*bool),right(bool,*bool)}}impl Duplicate for Choice {{}}
            fn run()->bool {{let cell=true;let choice={constructor};
                let read=|| choice match {{.none=>false,.left(p)=>p.*,.right(_,p)=>p.*}};let first=read();let second=read();first and second
            }}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{constructor}: {e}")),
            Value::Bool(constructor != "Choice.none")
        );
    }
}

#[test]
fn enum_pointer_capture_joins_preserve_lifetime_obligations() {
    for flag in [false, true] {
        for invalidation in ["", "let moved=first;", "let moved=second;"] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            enum Choice {{left(*Cell),right(bool,*Cell)}}impl Duplicate for Choice {{}}
            fn inspect(flag:bool)->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                let choice=if flag {{Choice.left(first.ref)}} else {{Choice.right(false,second.ref)}};
                let read=|| choice match {{.left(p)=>p.flag.*,.right(_,p)=>p.flag.*}};
                {invalidation}
                read()
            }}
            fn run()->bool {{inspect({flag})}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if invalidation.is_empty() {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{flag}: {e}")),
                    Value::Bool(flag)
                );
            } else {
                let error = actual.unwrap_err();
                assert!(
                    error.contains("cannot prepare resource-checked execution"),
                    "{invalidation}: {error}"
                );
            }
        }
    }
}

#[test]
fn enum_pointer_capture_updates_change_the_next_invocation() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        enum Choice {none,left(*bool),right(bool,*bool)}impl Duplicate for Choice {}
        fn inspect(a:*bool,b:*bool)->bool {var choice=Choice.left(a);
            let step=|| {let seen=choice match {.none=>false,.left(p)=>p.*,.right(_,p)=>p.*};choice=Choice.right(false,b);seen};
            let first=step();let second=step();first and not second
        }
        fn run()->bool {let a=true;let b=false;inspect(a.ref,b.ref)}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn enum_capture_exit_domains_distinguish_failure_and_normal_return() {
    use elaboration::verification::moves::{self, Delivery};
    let package=compile("enum Choice {none,left(*bool),right(bool,*bool)}
        fn inspect(flag:bool) {let cell=true;let q=cell.ref;var choice=Choice.left(q);
            let action=|| {choice=Choice.right(false,q);assumes flag;choice=Choice.none;};action();}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let report = moves::analyze_in_package(
        &package,
        function,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        200000,
    )
    .unwrap();
    let invocation = &report
        .closures
        .iter()
        .find(|c| c.invoked)
        .unwrap()
        .invocation;
    assert!(!invocation.failed_capture_enum_exits.is_empty(), "{report}");
    assert_eq!(
        invocation.failed_capture_enum_exits.len(),
        invocation.failed_exits.len()
    );
    for (exits, tag) in [
        (&invocation.failed_capture_enum_exits, 2),
        (&invocation.captured_enum_exits, 0),
    ] {
        for exit in exits {
            let tags = exit
                .values()
                .filter_map(|domains| domains.get(&vec![]))
                .collect::<Vec<_>>();
            assert_eq!(
                tags,
                vec![&std::collections::BTreeSet::from([tag])],
                "{report}"
            );
        }
    }
    assert!(
        report
            .to_string()
            .contains("(failed-capture-enum-exits (captures")
    );
}

#[test]
fn enum_capture_effect_exit_keeps_the_suspended_tag() {
    use elaboration::verification::moves::{self, Delivery};
    let package=compile("effect poll()->bool;enum Choice {none,left(*bool),right(bool,*bool)}
        fn inspect() {let cell=true;let q=cell.ref;var choice=Choice.left(q);
            let action:%{choice:Choice,q:*bool} fn()->#poll bool=|| {choice=Choice.right(false,q);poll()#;choice=Choice.none;true};}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let report = moves::analyze_in_package(
        &package,
        function,
        |ty| {
            Ok(
                if matches!(ty, nir::Term::Primitive(_) | nir::Term::Pointer(_)) {
                    Delivery::Copy
                } else {
                    Delivery::Move
                },
            )
        },
        200000,
    )
    .unwrap();
    let invocation = &report.closures.first().unwrap().invocation;
    assert!(!invocation.effect_capture_enum_exits.is_empty(), "{report}");
    assert_eq!(
        invocation.effect_capture_enum_exits.len(),
        invocation.effect_exits.len()
    );
    for exit in &invocation.effect_capture_enum_exits {
        assert_eq!(
            exit.values()
                .filter_map(|domains| domains.get(&vec![]))
                .collect::<Vec<_>>(),
            vec![&std::collections::BTreeSet::from([2])],
            "{report}"
        );
    }
    assert!(
        report
            .to_string()
            .contains("(effect-capture-enum-exits (captures")
    );
}

#[test]
fn enum_capture_replacement_releases_old_target_and_checks_new_target() {
    for retired in ["first", "second"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            enum Choice {{left(*Cell),right(bool,*Cell)}}impl Duplicate for Choice {{}}
            fn run()->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                var choice=Choice.left(first.ref);let q=second.ref;
                let step=|| {{let seen=choice match {{.left(p)=>p.flag.*,.right(_,p)=>p.flag.*}};choice=Choice.right(false,q);seen}};
                let before=step();let moved={retired};let after=step();before and not after
            }}"));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            let error = actual.unwrap_err();
            assert!(
                error.contains("cannot prepare resource-checked execution"),
                "{error}"
            );
        }
    }
}

#[test]
fn uncalled_closure_cleanup_preserves_enum_capture_tags() {
    for delivery in ["", "let stored=(action,false);"] {
        for constructor in [
            "Choice.left(Link {target:cell.ref})",
            "Choice.right(false,Link {target:cell.ref})",
        ] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Link {{target:*bool}}impl Retire for Link {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}
            enum Choice {{left(Link),right(bool,Link)}}
            fn run()->bool {{let cell=false;{{let choice={constructor};let action=|| {{let moved=choice;}};{delivery}}}cell}}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    500000
                )
                .unwrap_or_else(|e| panic!("{constructor}: {e}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn updated_enum_capture_cleanup_uses_the_new_payload() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        struct Link {target:*bool}impl Retire for Link {fn drop(*self) {self.target.*.* = not self.target.*.*;}}
        enum Choice {left(Link),right(bool,Link)}
        fn run()->bool {let first=false;let second=false;{
            var choice=Choice.left(Link {target:first.ref});let q=second.ref;
            let action=|| {choice=Choice.right(false,Link {target:q});};action();
        }first and second}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn selected_enum_pointer_closures_preserve_absent_payload_fields() {
    for flag in [false, true] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool)}}impl Duplicate for Choice {{}}
            typealias Reader=%{{choice:Choice}} fn()->bool;
            fn inspect(flag:bool)->bool {{let first=true;let second=false;
                let reader:Reader=if flag {{let choice=Choice.left(first.ref);|| choice match {{.left(p)=>p.*,.right(_,p)=>p.*}}}}
                    else {{let choice=Choice.right(false,second.ref);|| choice match {{.left(p)=>p.*,.right(_,p)=>p.*}}}};
                reader()
            }}
            fn run()->bool {{inspect({flag})}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{flag}: {e}")),
            Value::Bool(flag)
        );
    }
}

#[test]
fn selected_enum_pointer_closures_reject_expired_targets() {
    for retired in ["first", "second"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            enum Choice {{left(*Cell),right(bool,*Cell)}}impl Duplicate for Choice {{}}
            typealias Reader=%{{choice:Choice}} fn()->bool;
            fn inspect(flag:bool)->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                let reader:Reader=if flag {{let choice=Choice.left(first.ref);|| choice match {{.left(p)=>p.flag.*,.right(_,p)=>p.flag.*}}}}
                    else {{let choice=Choice.right(false,second.ref);|| choice match {{.left(p)=>p.flag.*,.right(_,p)=>p.flag.*}}}};
                let moved={retired};reader()
            }}
            fn run()->bool {{inspect(true)}}"));
        let error = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prepare resource-checked execution"),
            "{retired}: {error}"
        );
    }
}

#[test]
fn reassigned_enum_pointer_closures_survive_flow_joins() {
    for control in [
        "if flag {reader=do {let choice=Choice.right(false,second.ref);|| choice match {.left(p)=>p.*,.right(_,p)=>p.*}};}",
        "var again=flag;while again {reader=do {let choice=Choice.right(false,second.ref);|| choice match {.left(p)=>p.*,.right(_,p)=>p.*}};again=false;}",
    ] {
        for flag in [false, true] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                enum Choice {{left(*bool),right(bool,*bool)}}impl Duplicate for Choice {{}}
                typealias Reader=%{{choice:Choice}} fn()->bool;
                fn inspect(flag:bool)->bool {{let first=true;let second=false;
                    var reader:Reader=do {{let choice=Choice.left(first.ref);|| choice match {{.left(p)=>p.*,.right(_,p)=>p.*}}}};
                    {control}
                    reader()
                }}
                fn run()->bool {{inspect({flag})}}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    500000
                )
                .unwrap_or_else(|e| panic!("{control}, {flag}: {e}")),
                Value::Bool(!flag)
            );
        }
    }
}

#[test]
fn reassigned_enum_closures_clean_each_environment_once() {
    for control in [
        "if flag {action=do {let choice=Choice.right(false,Link {target:second.ref});|| {let moved=choice;}};}",
        "var again=flag;while again {action=do {let choice=Choice.right(false,Link {target:second.ref});|| {let moved=choice;}};again=false;}",
    ] {
        for flag in [false, true] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Link {{target:*bool}}impl Retire for Link {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}
                enum Choice {{left(Link),right(bool,Link)}}
                typealias Action=%{{choice:Choice}} fn();
                fn inspect(flag:bool)->bool {{let first=false;let second=false;
                    {{var action:Action=do {{let choice=Choice.left(Link {{target:first.ref}});|| {{let moved=choice;}}}};
                    {control}}}
                    first and (second == flag)
                }}
                fn run()->bool {{inspect({flag})}}"));
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    500000
                )
                .unwrap_or_else(|e| panic!("{control}, {flag}: {e}")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn returned_pointer_closure_keeps_external_target() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn make(p:*bool)->Reader {|| p.*}
        fn run()->bool {let cell=true;let reader=make(cell.ref);reader()}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn returned_pointer_closures_forward_and_reject_invalid_targets() {
    for delivery in ["make(cell.ref)", "forward(cell.ref)"] {
        for expired in [false, true] {
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                typealias Reader=%{{p:*Cell}} fn()->bool;
                fn make(p:*Cell)->Reader {{|| p.flag.*}}
                fn forward(p:*Cell)->Reader {{make(p)}}
                fn run()->bool {{let cell=Cell {{flag:true}};let reader={delivery};{invalidate}reader()}}"));
            let result = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if expired {
                assert!(
                    result
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    result.unwrap_or_else(|e| panic!("{delivery}: {e}")),
                    Value::Bool(true)
                );
            }
        }
    }
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn make()->Reader {let cell=true;let p=cell.ref;|| p.*}
        fn run()->bool {let reader=make();reader()}",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn returned_pointer_closure_exports_updated_capture_origins() {
    for retired in ["first", "second"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            fn make(p:*Cell,q:*Cell)->Reader {{let reader=|| {{let result=p.flag.*;p=q;result}};reader();reader}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let reader=make(first.ref,second.ref);let moved={retired};reader()}}"));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn returned_aggregate_closure_exports_updated_capture_origins() {
    for retired in ["first", "second"] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            struct Bundle {{reader:Reader}}
            fn make(p:*Cell,q:*Cell)->Bundle {{
                let bundle=Bundle {{reader:|| {{let result=p.flag.*;p=q;result}}}};
                let reader=bundle.reader;reader();Bundle {{reader:reader}}
            }}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let bundle=make(first.ref,second.ref);let moved={retired};bundle.reader()}}"
        ));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn returned_enum_pointer_closure_preserves_conditional_environment() {
    for flag in [false, true] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool)}}impl Duplicate for Choice {{}}
            typealias Reader=%{{choice:Choice}} fn()->bool;
            fn make(p:*bool,q:*bool,flag:bool)->Reader {{
                let choice=if flag {{Choice.left(p)}}else {{Choice.right(false,q)}};
                || choice match {{.left(p)=>p.*,.right(_,p)=>p.*}}
            }}
            fn run()->bool {{let first=true;let second=false;let reader=make(first.ref,second.ref,{flag});reader()}}"));
        let make = package
            .functions
            .iter()
            .find(|function| function.name == "make")
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            make.declaration,
            &[],
            &mut 500000,
        )
        .unwrap();
        assert_eq!(summary.pointer_fields.len(), 2);
        for (path, parameter) in [
            (vec![0, usize::MAX, 0, 0], 0),
            (vec![0, usize::MAX, 1, 1], 1),
        ] {
            let targets = summary.pointer_fields[&path].as_ref().unwrap();
            assert_eq!(targets.len(), 1);
            let target = targets.iter().next().unwrap();
            assert_eq!(target.parameter, parameter);
            assert!(target.input_fields.is_empty());
        }
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{flag}: {e}")),
            Value::Bool(flag)
        );
    }
}

#[test]
fn higher_order_pointer_closure_uses_actual_environment() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn apply(reader:Reader)->bool {reader()}
        fn run()->bool {let cell=true;let p=cell.ref;apply(|| p.*)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn higher_order_pointer_closures_forward_and_check_targets() {
    for entry in ["apply", "forward"] {
        for expired in [false, true] {
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package = compile(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                typealias Reader=%{{p:*Cell}} fn()->bool;
                fn apply(reader:Reader)->bool {{reader()}}
                fn forward(reader:Reader)->bool {{apply(reader)}}
                fn run()->bool {{let cell=Cell {{flag:true}};let p=cell.ref;let reader=|| p.flag.*;
                    {invalidate}{entry}(reader)}}"
            ));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{entry}: {e}")),
                    Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn higher_order_pointer_closure_returns_updated_environment() {
    for retired in ["first", "second"] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            fn advance(reader:Reader)->Reader {{reader();reader}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                var p=first.ref;let q=second.ref;
                let reader=advance(|| {{let result=p.flag.*;p=q;result}});
                let moved={retired};reader()}}"
        ));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn higher_order_pointer_closures_keep_distinct_inputs() {
    for expired in [false, true] {
        let second = if expired {
            "do {let moved=first;right}"
        } else {
            "right"
        };
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell}} fn()->bool;
            fn make(p:*Cell)->Reader {{|| p.flag.*}}
            fn both(left:Reader,right:Reader)->bool {{left() and not right()}}
            fn run()->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                let left=make(first.ref);let right=make(second.ref);both(left,{second})}}"
        ));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if expired {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        } else {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        }
    }
}

#[test]
fn higher_order_enum_pointer_closure_keeps_payload_paths() {
    for flag in [false, true] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool)}}impl Duplicate for Choice {{}}
            typealias Reader=%{{choice:Choice}} fn()->bool;
            fn apply(reader:Reader)->bool {{reader()}}
            fn make(p:*bool,q:*bool,flag:bool)->Reader {{
                let choice=if flag {{Choice.left(p)}}else {{Choice.right(false,q)}};
                || choice match {{.left(p)=>p.*,.right(_,p)=>p.*}}
            }}
            fn run()->bool {{let first=true;let second=false;apply(make(first.ref,second.ref,{flag}))}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{flag}: {e}")),
            Value::Bool(flag)
        );
    }
}

#[test]
fn higher_order_aggregate_pointer_closure_uses_field_prefix() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        struct Bundle {unused:bool,reader:Reader}
        fn apply(bundle:Bundle)->bool {bundle.reader()}
        fn run()->bool {let cell=true;let p=cell.ref;apply(Bundle {unused:false,reader:|| p.*})}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn higher_order_same_code_instances_keep_independent_capture_updates() {
    for retired in ["first", "second"] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            fn make(p:*Cell,q:*Cell)->Reader {{|| {{let result=p.flag.*;p=q;result}}}}
            fn advance(left:Reader,right:Reader)->Reader {{left();right();left}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let left=make(first.ref,second.ref);let right=make(second.ref,first.ref);
                let reader=advance(left,right);let moved={retired};reader()}}"
        ));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn factory_pointer_closure_invocation_has_named_function_summary() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn make(p:*bool)->Reader {|| p.*}
        fn evaluate(p:*bool)->bool {let reader=make(p);reader()}
        fn run()->bool {let cell=true;evaluate(cell.ref)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn factory_instances_in_helper_keep_distinct_origins() {
    for retired in ["first", "second"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            fn make(p:*Cell,q:*Cell)->Reader {{|| {{let result=p.flag.*;p=q;result}}}}
            fn prepare(p:*Cell,q:*Cell)->Reader {{let left=make(p,q);let right=make(q,p);left();right();left}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let reader=prepare(first.ref,second.ref);let moved={retired};reader()}}"));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "prepare")
            .unwrap();
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            &mut 500000,
        )
        .unwrap();
        for path in [vec![0], vec![1]] {
            let targets = summary.pointer_fields[&path].as_ref().unwrap();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets.iter().next().unwrap().parameter, 1);
        }
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn composite_factory_results_restore_pointer_environments() {
    for setup in [
        "let bundle=pack(p);bundle.reader()",
        "let factory=|q:*bool| make(q);let reader=factory(p);reader()",
        "let factory=|| make(p);let reader=factory();reader()",
        "let reader=if flag {make(p)}else {make(q)};reader()",
    ] {
        for flag in [false, true] {
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                typealias Reader=%{{p:*bool}} fn()->bool;
                struct Bundle {{unused:bool,reader:Reader}}
                fn make(p:*bool)->Reader {{|| p.*}}
                fn pack(p:*bool)->Bundle {{Bundle {{unused:false,reader:make(p)}}}}
                fn evaluate(p:*bool,q:*bool,flag:bool)->bool {{{setup}}}
                fn run()->bool {{let first=true;let second=false;evaluate(first.ref,second.ref,{flag})}}"));
            let expected = if setup.starts_with("let reader=if") {
                flag
            } else {
                true
            };
            assert_eq!(
                luna::execution::interpret_with_protocols(
                    &package,
                    "run",
                    "Duplicate",
                    "Retire",
                    500000
                )
                .unwrap_or_else(|e| panic!("{setup}, {flag}: {e}")),
                Value::Bool(expected)
            );
        }
    }
}

#[test]
fn closure_factory_results_keep_lifetime_obligations() {
    for setup in [
        "let factory=|q:*Cell| make(q);factory(p)",
        "let factory=|| make(p);factory()",
    ] {
        for expired in [false, true] {
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                typealias Reader=%{{p:*Cell}} fn()->bool;
                fn make(p:*Cell)->Reader {{|| p.flag.*}}
                fn prepare(p:*Cell)->Reader {{{setup}}}
                fn run()->bool {{let cell=Cell {{flag:true}};let reader=prepare(cell.ref);{invalidate}reader()}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{setup}: {e}")),
                    Value::Bool(true)
                );
            }
        }
    }
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn escape()->Reader {let factory=|| {let cell=true;let p=cell.ref;|| p.*};factory()}
        fn run()->bool {let reader=escape();reader()}",
    );
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn recaptured_pointer_closure_preserves_inner_environment() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn make(p:*bool)->Reader {|| p.*}
        fn run()->bool {let cell=true;let inner=make(cell.ref);let outer=|| inner();outer()}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn recaptured_pointer_closure_returns_and_checks_lifetimes() {
    for construction in ["let outer=|| inner();", "let outer=wrap(inner);"] {
        for expired in [false, true] {
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package = compile(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                typealias Reader=%{{p:*Cell}} fn()->bool;
                typealias Outer=%{{inner:Reader}} fn()->bool;
                fn make(p:*Cell)->Reader {{|| p.flag.*}}
                fn wrap(inner:Reader)->Outer {{|| inner()}}
                fn run()->bool {{let cell=Cell {{flag:true}};let inner=make(cell.ref);
                    {construction}{invalidate}outer() and outer()}}"
            ));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{construction}: {e}")),
                    Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn recaptured_pointer_closure_updates_nested_environment() {
    for retired in ["first", "second"] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            fn make(p:*Cell,q:*Cell)->Reader {{|| {{let result=p.flag.*;p=q;result}}}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let inner=make(first.ref,second.ref);let outer=|| inner();
                let before=outer();let moved={retired};let after=outer();not before and after}}"
        ));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn returned_nested_closure_keeps_post_invocation_pointer_state() {
    for retired in ["first", "second"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            typealias Outer=%{{inner:Reader}} fn()->bool;
            fn make(p:*Cell,q:*Cell)->Reader {{|| {{let result=p.flag.*;p=q;result}}}}
            fn prepare(p:*Cell,q:*Cell)->Outer {{let inner=make(p,q);let outer=|| inner();outer();outer}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let outer=prepare(first.ref,second.ref);let moved={retired};outer()}}"));
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn factory_callback_in_captured_record_preserves_pending_candidates() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        struct Bundle {reader:Reader}
        fn make(p:*bool)->Reader {|| p.*}
        fn evaluate(p:*bool)->bool {let bundle=Bundle {reader:make(p)};let outer=|| bundle.reader();outer()}
        fn run()->bool {let cell=true;evaluate(cell.ref)}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_record_unknown_callback_is_not_pending_bottom() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        struct Bundle {reader:Reader}
        fn make(p:*bool)->Reader {|| p.*}
        fn evaluate(reader:Reader)->bool {let bundle=Bundle {reader:reader};let outer=|| bundle.reader();outer()}
        fn run()->bool {let cell=true;evaluate(make(cell.ref))}");
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "evaluate")
        .unwrap();
    let error = elaboration::verification::memory::infer_in_package(
        &package,
        function.declaration,
        &[],
        &mut 500000,
    )
    .unwrap_err();
    assert!(
        error.contains("dynamic call requires a richer memory contract"),
        "{error}"
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn captured_enum_absence_does_not_make_outer_code_bottom() {
    for flag in [false, true] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            typealias Reader=%{{p:*bool}} fn()->bool;
            enum Choice {{none,some(Reader)}}
            fn make(p:*bool)->Reader {{|| p.*}}
            fn evaluate(p:*bool,flag:bool)->bool {{
                let choice=if flag {{Choice.some(make(p))}}else {{Choice.none}};
                let outer=|| choice match {{.none=>false,.some(reader)=>reader()}};outer()
            }}
            fn run()->bool {{let cell=true;evaluate(cell.ref,{flag})}}"
        ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{flag}: {e}")),
            Value::Bool(flag)
        );
    }
}

#[test]
fn immediate_recapture_preserves_factory_callback_candidates() {
    let package = compile(
        "trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn make(p:*bool)->Reader {|| p.*}
        fn evaluate(p:*bool)->bool {let reader=make(p);(|| reader())()}
        fn run()->bool {let cell=true;evaluate(cell.ref)}",
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn immediate_record_recapture_keeps_identity_and_lifetime_checks() {
    for expired in [false, true] {
        let invalidate = if expired { "let moved=cell;" } else { "" };
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell}} fn()->bool;
            struct Bundle {{reader:Reader}}
            fn make(p:*Cell)->Reader {{|| p.flag.*}}
            fn evaluate(p:*Cell)->bool {{let bundle=Bundle {{reader:make(p)}};(|| bundle.reader())()}}
            fn invoke(reader:Reader)->bool {{let bundle=Bundle {{reader:reader}};(|| bundle.reader())()}}
            fn run()->bool {{let cell=Cell {{flag:true}};let p=cell.ref;{invalidate}evaluate(p)}}"));
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "invoke")
            .unwrap();
        let error = elaboration::verification::memory::infer_in_package(
            &package,
            function.declaration,
            &[],
            &mut 500000,
        )
        .unwrap_err();
        assert!(
            error.contains("dynamic call requires a richer memory contract"),
            "{error}"
        );
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if expired {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        } else {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        }
    }
}

#[test]
fn recursive_pointer_closure_factory_preserves_environment_origins() {
    for flag in [false, true] {
        for expired in [false, true] {
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                typealias Reader=%{{p:*Cell}} fn()->bool;
                fn make(p:*Cell,flag:bool)->Reader {{if flag {{|| p.flag.*}}else {{make(p,true)}}}}
                fn run()->bool {{let cell=Cell {{flag:true}};let reader=make(cell.ref,{flag});{invalidate}reader()}}"));
            let make = package.functions.iter().find(|f| f.name == "make").unwrap();
            let summary = elaboration::verification::memory::infer_in_package(
                &package,
                make.declaration,
                &[],
                &mut 500000,
            )
            .unwrap();
            let origins = summary.pointer_fields[&vec![0]].as_ref().unwrap();
            assert_eq!(origins.len(), 1);
            assert_eq!(origins.iter().next().unwrap().parameter, 0);
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{flag}: {e}")),
                    Value::Bool(true)
                );
            }
        }
    }
}

#[test]
fn mutually_recursive_pointer_factories_keep_contracts_and_reject_local_escape() {
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn first(p:*bool,flag:bool)->Reader {if flag {|| p.*}else {second(p,true)}}
        fn second(p:*bool,flag:bool)->Reader {if flag {|| p.*}else {first(p,true)}}
        fn run()->bool {let cell=true;let a=first(cell.ref,false);let b=second(cell.ref,false);a() and b()}");
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap(),
        Value::Bool(true)
    );
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        typealias Reader=%{p:*bool} fn()->bool;
        fn escape(flag:bool)->Reader {if flag {let cell=true;let p=cell.ref;|| p.*}else {escape(true)}}
        fn run()->bool {let reader=escape(false);reader()}");
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn recursive_pointer_factory_joins_permuted_targets() {
    for flag in [false, true] {
        for retired in ["", "first", "second"] {
            let invalidate = if retired.is_empty() {
                String::new()
            } else {
                format!("let moved={retired};")
            };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                typealias Reader=%{{p:*Cell}} fn()->bool;
                fn make(p:*Cell,q:*Cell,flag:bool)->Reader {{if flag {{|| p.flag.*}}else {{make(q,p,true)}}}}
                fn run()->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                    let reader=make(first.ref,second.ref,{flag});{invalidate}reader()}}"));
            let make = package.functions.iter().find(|f| f.name == "make").unwrap();
            let summary = elaboration::verification::memory::infer_in_package(
                &package,
                make.declaration,
                &[],
                &mut 500000,
            )
            .unwrap();
            let targets = summary.pointer_fields[&vec![0]].as_ref().unwrap();
            assert_eq!(
                targets
                    .iter()
                    .map(|p| p.parameter)
                    .collect::<std::collections::BTreeSet<_>>(),
                std::collections::BTreeSet::from([0, 1])
            );
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if retired.is_empty() {
                assert_eq!(actual.unwrap(), Value::Bool(flag));
            } else {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            }
        }
    }
}

#[test]
fn recursive_factory_returns_updated_pointer_state_after_budget_recovery() {
    for retired in ["first", "second"] {
        let package = compile(&format!(
            "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
            typealias Reader=%{{p:*Cell,q:*Cell}} fn()->bool;
            fn make(p:*Cell,q:*Cell,flag:bool)->Reader {{if flag {{
                let reader=|| {{let before=p.flag.*;p=q;before}};reader();reader
            }}else {{make(p,q,true)}}}}
            fn run()->bool {{let first=Cell {{flag:false}};let second=Cell {{flag:true}};
                let reader=make(first.ref,second.ref,false);let moved={retired};reader()}}"
        ));
        let make = package.functions.iter().find(|f| f.name == "make").unwrap();
        let error = elaboration::verification::memory::infer_in_package(
            &package,
            make.declaration,
            &[],
            &mut 1,
        )
        .unwrap_err();
        assert!(error.contains("budget exhausted"), "{error}");
        let summary = elaboration::verification::memory::infer_in_package(
            &package,
            make.declaration,
            &[],
            &mut 500000,
        )
        .unwrap();
        for path in [vec![0], vec![1]] {
            let targets = summary.pointer_fields[&path].as_ref().unwrap();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets.iter().next().unwrap().parameter, 1);
        }
        let actual = luna::execution::interpret_with_protocols(
            &package,
            "run",
            "Duplicate",
            "Retire",
            500000,
        );
        if retired == "first" {
            assert_eq!(actual.unwrap(), Value::Bool(true));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .contains("cannot prepare resource-checked execution")
            );
        }
    }
}

#[test]
fn recursive_enum_capture_factory_keeps_all_possible_pointer_sources() {
    for tag in [false, true] {
        for retired in ["", "first", "second"] {
            let invalidate = if retired.is_empty() {
                String::new()
            } else {
                format!("let moved={retired};")
            };
            let package = compile(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                enum Choice {{left(*Cell),right(bool,*Cell)}}impl Duplicate for Choice {{}}
                typealias Reader=%{{choice:Choice}} fn()->bool;
                fn make(p:*Cell,q:*Cell,tag:bool,ready:bool)->Reader {{if ready {{
                    let choice=if tag {{Choice.left(p)}}else {{Choice.right(false,q)}};
                    || choice match {{.left(p)=>p.flag.*,.right(_,p)=>p.flag.*}}
                }}else {{make(p,q,tag,true)}}}}
                fn run()->bool {{let first=Cell {{flag:true}};let second=Cell {{flag:false}};
                    let reader=make(first.ref,second.ref,{tag},false);{invalidate}reader()}}"
            ));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if retired.is_empty() {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{tag}: {e}")),
                    Value::Bool(tag)
                );
            } else {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            }
        }
    }
}

#[test]
fn recursive_enum_capture_shared_slots_and_local_escape() {
    for tag in [false, true] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(*bool)}}impl Duplicate for Choice {{}}
            typealias Reader=%{{choice:Choice}} fn()->bool;
            fn make(p:*bool,q:*bool,tag:bool,ready:bool)->Reader {{if ready {{
                let choice=if tag {{Choice.left(p)}}else {{Choice.right(q)}};
                || choice match {{.left(p)=>p.*,.right(p)=>p.*}}
            }}else {{make(p,q,tag,true)}}}}
            fn run()->bool {{let first=true;let second=false;let reader=make(first.ref,second.ref,{tag},false);reader()}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap(),
            Value::Bool(tag)
        );
    }
    let package=compile("trait Duplicate {}trait Retire {fn drop(*self);}
        enum Choice {some(*bool)}impl Duplicate for Choice {}
        typealias Reader=%{choice:Choice} fn()->bool;
        fn escape(ready:bool)->Reader {if ready {let cell=true;let choice=Choice.some(cell.ref);|| choice match {.some(p)=>p.*}}else {escape(true)}}
        fn run()->bool {let reader=escape(false);reader()}");
    let error =
        luna::execution::interpret_with_protocols(&package, "run", "Duplicate", "Retire", 500000)
            .unwrap_err();
    assert!(
        error.contains("cannot prepare resource-checked execution"),
        "{error}"
    );
}

#[test]
fn recursive_enum_capture_factory_excludes_absent_pointer_payloads() {
    for selected in ["Choice.left(p)", "Choice.right(false,p)"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Choice {{left(*bool),right(bool,*bool)}}impl Duplicate for Choice {{}}
            typealias Reader=%{{choice:Choice}} fn()->bool;
            fn make(p:*bool,ready:bool)->Reader {{if ready {{let choice={selected};|| choice match {{.left(p)=>p.*,.right(_,p)=>p.*}}}}else {{make(p,true)}}}}
            fn run()->bool {{let cell=true;let reader=make(cell.ref,false);reader()}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{selected}: {e}")),
            Value::Bool(true)
        );
    }
}

#[test]
fn recursive_absent_capture_payload_does_not_borrow_unused_target() {
    for present in [false, true] {
        for expired in [false, true] {
            let constructor = if present {
                "Choice.some(p)"
            } else {
                "Choice.none"
            };
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                enum Choice {{none,some(*Cell)}}impl Duplicate for Choice {{}}
                typealias Reader=%{{choice:Choice}} fn()->bool;
                fn make(p:*Cell,ready:bool)->Reader {{if ready {{let choice={constructor};|| choice match {{.none=>false,.some(p)=>p.flag.*}}}}else {{make(p,true)}}}}
                fn run()->bool {{let cell=Cell {{flag:true}};let reader=make(cell.ref,false);{invalidate}reader()}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if present && expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{present}, {expired}: {e}")),
                    Value::Bool(present)
                );
            }
        }
    }
}

#[test]
fn recursive_nested_absence_propagates_to_enclosing_capture_tag() {
    for constructor in ["Outer.none", "Outer.some(Inner.left(p))"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            enum Inner {{left(*bool),right(*bool)}}impl Duplicate for Inner {{}}
            enum Outer {{none,some(Inner)}}impl Duplicate for Outer {{}}
            typealias Reader=%{{choice:Outer}} fn()->bool;
            fn make(p:*bool,ready:bool)->Reader {{if ready {{let choice={constructor};|| choice match {{.none=>false,.some(value)=>value match {{.left(p)=>p.*,.right(p)=>p.*}}}}}}else {{make(p,true)}}}}
            fn run()->bool {{let cell=true;let reader=make(cell.ref,false);reader()}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{constructor}: {e}")),
            Value::Bool(constructor != "Outer.none")
        );
    }
}

#[test]
fn recursive_uncalled_closure_cleanup_preserves_capture_tag() {
    for constructor in ["Choice.none", "Choice.some(Link {target:p})"] {
        let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
            struct Link {{target:*bool}}
            impl Retire for Link {{fn drop(*self) {{self.target.*.* = not self.target.*.*;}}}}
            enum Choice {{none,some(Link)}}
            typealias Action=%{{choice:Choice}} fn();
            fn make(p:*bool,ready:bool)->Action {{if ready {{let choice={constructor};|| {{let moved=choice;}}}}else {{make(p,true)}}}}
            fn run()->bool {{let cell=false;{{let action=make(cell.ref,false);}}cell}}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!("{constructor}: {e}")),
            Value::Bool(constructor != "Choice.none")
        );
    }
}

#[test]
fn recursive_uncalled_closure_cleanup_checks_only_present_payload_targets() {
    for present in [false, true] {
        for expired in [false, true] {
            let constructor = if present {
                "Choice.some(Link {target:p})"
            } else {
                "Choice.none"
            };
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let result = if expired { "true" } else { "cell.flag" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                struct Link {{target:*Cell}}impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = not p.flag.*;}}}}
                enum Choice {{none,some(Link)}}
                typealias Action=%{{choice:Choice}} fn();
                fn make(p:*Cell,ready:bool)->Action {{if ready {{let choice={constructor};|| {{let moved=choice;}}}}else {{make(p,true)}}}}
                fn run()->bool {{let cell=Cell {{flag:false}};{{let action=make(cell.ref,false);let stored=(action,false);{invalidate}}}{result}}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if present && expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{present}, {expired}: {e}")),
                    Value::Bool(present || expired)
                );
            }
        }
    }
}

#[test]
fn recursive_uncalled_nested_capture_cleanup_preserves_target_selection() {
    for (constructor, target) in [
        ("Outer.none", 0),
        ("Outer.some(Inner.none)", 0),
        ("Outer.some(Inner.left(Link {target:p}))", 1),
        ("Outer.some(Inner.right(Link {target:q}))", 2),
    ] {
        for expired in 0..3 {
            let invalidate = match expired {
                1 => "let moved=left;",
                2 => "let moved=right;",
                _ => "",
            };
            let observe = match expired {
                1 => "right.flag",
                2 => "left.flag",
                _ => "left.flag or right.flag",
            };
            let package = compile(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                struct Link {{target:*Cell}}
                impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = not p.flag.*;}}}}
                enum Inner {{none,left(Link),right(Link)}}
                enum Outer {{none,some(Inner)}}
                typealias Action=%{{choice:Outer}} fn();
                fn make(p:*Cell,q:*Cell,ready:bool)->Action {{
                    if ready {{let choice={constructor};|| {{let moved=choice;}}}}
                    else {{make(p,q,true)}}
                }}
                fn run()->bool {{
                    let left=Cell {{flag:false}};let right=Cell {{flag:false}};
                    {{let action=make(left.ref,right.ref,false);let stored=(false,(action,true));{invalidate}}}
                    {observe}
                }}"
            ));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if target != 0 && target == expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution"),
                    "{constructor}, expired={expired}"
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{constructor}, expired={expired}: {e}")),
                    Value::Bool(target != 0)
                );
            }
        }
    }
}

#[test]
fn forwarded_recursive_affine_closure_retains_cleanup_evidence() {
    for present in [false, true] {
        for expired in [false, true] {
            let constructor = if present {
                "Choice.some(Link {target:p})"
            } else {
                "Choice.none"
            };
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let result = if expired { "true" } else { "cell.flag" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                struct Link {{target:*Cell}}impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = not p.flag.*;}}}}
                enum Choice {{none,some(Link)}}
                typealias Action=%{{choice:Choice}} fn();
                fn make(p:*Cell,ready:bool)->Action {{if ready {{let choice={constructor};|| {{let moved=choice;}}}}else {{make(p,true)}}}}
                fn forward(action:Action)->Action {{action}}
                fn run()->bool {{let cell=Cell {{flag:false}};{{let action=make(cell.ref,false);let stored=(forward(action),false);{invalidate}}}{result}}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if present && expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{present}, {expired}: {e}")),
                    Value::Bool(present || expired)
                );
            }
        }
    }
}

#[test]
fn callee_cleanup_of_recursive_affine_closure_checks_targets() {
    for present in [false, true] {
        for expired in [false, true] {
            let constructor = if present {
                "Choice.some(Link {target:p})"
            } else {
                "Choice.none"
            };
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let result = if expired { "true" } else { "cell.flag" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                struct Link {{target:*Cell}}impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = not p.flag.*;}}}}
                enum Choice {{none,some(Link)}}
                typealias Action=%{{choice:Choice}} fn();
                fn make(p:*Cell,ready:bool)->Action {{if ready {{let choice={constructor};|| {{let moved=choice;}}}}else {{make(p,true)}}}}
                fn consume(action:Action) {{}}
                fn run()->bool {{let cell=Cell {{flag:false}};{{let action=make(cell.ref,false);{invalidate}consume(action);}}{result}}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if present && expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{present}, {expired}: {e}")),
                    Value::Bool(present || expired)
                );
            }
        }
    }
}

#[test]
fn callee_cleanup_of_aggregate_affine_closure_checks_targets() {
    for present in [false, true] {
        for expired in [false, true] {
            let constructor = if present {
                "Choice.some(Link {target:p})"
            } else {
                "Choice.none"
            };
            let invalidate = if expired { "let moved=cell;" } else { "" };
            let result = if expired { "true" } else { "cell.flag" };
            let package=compile(&format!("trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                struct Link {{target:*Cell}}impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = not p.flag.*;}}}}
                enum Choice {{none,some(Link)}}
                typealias Action=%{{choice:Choice}} fn();
                fn make(p:*Cell,ready:bool)->Action {{if ready {{let choice={constructor};|| {{let moved=choice;}}}}else {{make(p,true)}}}}
                struct Packet {{action:Action}}
                fn retire(packet:Packet) {{}}
                fn consume(action:Action) {{retire(Packet {{action:action}})}}
                fn run()->bool {{let cell=Cell {{flag:false}};{{let action=make(cell.ref,false);{invalidate}consume(action);}}{result}}}"));
            let actual = luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000,
            );
            if present && expired {
                assert!(
                    actual
                        .unwrap_err()
                        .contains("cannot prepare resource-checked execution")
                );
            } else {
                assert_eq!(
                    actual.unwrap_or_else(|e| panic!("{present}, {expired}: {e}")),
                    Value::Bool(present || expired)
                );
            }
        }
    }
}

#[test]
fn shared_callee_cleanup_keeps_closure_instances_separate() {
    for (empty_second, reverse, retire_consumed) in [
        (false, false, false),
        (false, true, false),
        (true, false, false),
        (true, true, false),
        (false, false, true),
        (false, true, true),
    ] {
        let make_second = if empty_second { "empty" } else { "make" };
        let retire_unused = if empty_second { "let moved=right;" } else { "" };
        let calls = if retire_consumed && reverse {
            "consume(second);let moved=right;consume(first);"
        } else if retire_consumed {
            "consume(first);let moved=left;consume(second);"
        } else if reverse {
            "consume(second);consume(first);"
        } else {
            "consume(first);consume(second);"
        };
        let observe = if retire_consumed && !reverse {
            "right.flag"
        } else if empty_second || retire_consumed {
            "left.flag"
        } else {
            "left.flag and right.flag"
        };
        let package = compile(&format!(
                "trait Duplicate {{}}trait Retire {{fn drop(*self);}}
                struct Cell {{flag:bool}}impl Retire for Cell {{fn drop(*self) {{}}}}
                struct Link {{target:*Cell}}
                impl Retire for Link {{fn drop(*self) {{let p=self.target.*;p.flag.* = not p.flag.*;}}}}
                enum Choice {{none,some(Link)}}
                typealias Action=%{{choice:Choice}} fn();
                fn make(p:*Cell,ready:bool)->Action {{
                    if ready {{let choice=Choice.some(Link {{target:p}});|| {{let moved=choice;}}}}
                    else {{make(p,true)}}
                }}
                fn empty(p:*Cell,ready:bool)->Action {{
                    if ready {{let choice=Choice.none;|| {{let moved=choice;}}}}
                    else {{empty(p,true)}}
                }}
                struct Packet {{action:Action}}
                fn retire(packet:Packet) {{}}
                fn consume(action:Action) {{retire(Packet {{action:action}})}}
                fn run()->bool {{
                    let left=Cell {{flag:false}};let right=Cell {{flag:false}};
                    let first=make(left.ref,false);let second={make_second}(right.ref,false);
                    {retire_unused}{calls}{observe}
                }}"
            ));
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                500000
            )
            .unwrap_or_else(|e| panic!(
                "empty={empty_second}, reverse={reverse}, retired={retire_consumed}: {e}"
            )),
            Value::Bool(true)
        );
    }
}
